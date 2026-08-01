//! Read-only questions about what indexing knew and what it could not do.
//!
//! Two of them, and both are about the *absence* of knowledge, which this product treats as
//! first-class rather than hiding (ARCHITECTURE.md invariant 4):
//!
//! - **Partial parses.** A file whose tree-sitter parse produced an `ERROR` node was still
//!   indexed, but what came out of it is incomplete. The per-file flag lives in the extractor's
//!   own payload in `module_facts`, so reading it back is this crate's job — `nerve-store`
//!   deliberately treats that payload as opaque text and must not learn to parse it.
//! - **Index freshness.** `module_facts` records the content hash every module was extracted at.
//!   Comparing that against the file on disk says whether the graph still describes the
//!   repository. The comparison is a `nerve-store` function; supplying the file bytes is a
//!   path-safety decision and therefore belongs here, next to the root.
//!
//! Neither function writes anything.

use std::collections::BTreeMap;

use nerve_store::{Connection, FileProber, Freshness, FreshnessCache};

use crate::error::Result;
use crate::facts::ModuleFacts;

/// One file that parsed with errors, with the tallies recorded alongside it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PartialParse {
    /// Repository-relative path.
    pub rel_path: String,
    /// Language the file was parsed as.
    pub language: String,
    /// Content hash the file had when it was extracted.
    pub content_hash: String,
    /// `import(expr)` calls that named no specifier.
    pub dynamic_imports_without_specifier: usize,
    /// Call and heritage sites whose form Nerve does not model.
    pub unmodelled_call_sites: usize,
    /// Breakdown of the above by form tag.
    pub unmodelled_by_form: BTreeMap<String, usize>,
}

/// Every indexed file whose parse produced a syntax error, sorted by path.
///
/// A module whose cached payload this build cannot read is skipped rather than guessed at: the
/// cache is allowed to miss, and reporting "no syntax errors" from an unreadable cache would be
/// a confident wrong answer.
pub fn partial_parses(conn: &Connection, repo_id: &str) -> Result<Vec<PartialParse>> {
    let rows = nerve_store::load_module_facts(conn, repo_id)?;
    let mut out = Vec::new();
    for (rel_path, row) in rows {
        let Some(facts) = ModuleFacts::from_json(&row.facts) else {
            continue;
        };
        if !facts.counters.has_syntax_error {
            continue;
        }
        out.push(PartialParse {
            rel_path,
            language: row.language,
            content_hash: row.content_hash,
            dynamic_imports_without_specifier: facts.counters.dynamic_imports_without_specifier,
            unmodelled_call_sites: facts.counters.unmodelled_call_sites,
            unmodelled_by_form: facts.counters.unmodelled_by_form,
        });
    }
    out.sort_by(|a, b| a.rel_path.cmp(&b.rel_path));
    Ok(out)
}

/// Whether the graph still describes the files it was built from.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct IndexFreshness {
    /// Indexed modules recorded in the cache.
    pub files_total: usize,
    /// Modules actually re-hashed. Less than `files_total` when the cap bit.
    pub files_probed: usize,
    /// Files whose bytes still hash to what was extracted.
    pub fresh: usize,
    /// Files that exist and have changed.
    pub stale: usize,
    /// Files that no longer exist.
    pub missing: usize,
    /// Files refused by the path-safety check. Nothing was read.
    pub refused: usize,
    /// Files that were allowed but could not be read.
    pub unreadable: usize,
    /// True when the cap stopped the sweep before every file was checked.
    pub truncated: bool,
}

/// Re-hash up to `cap` indexed files and summarise how much of the graph is still current.
///
/// Bounded on purpose: this is a status question asked by an interactive surface, and a
/// repository can hold a hundred thousand files. When the cap bites, the report says so rather
/// than presenting a partial sweep as a whole one.
pub fn index_freshness(
    conn: &Connection,
    repo_id: &str,
    prober: &dyn FileProber,
    cap: usize,
) -> Result<IndexFreshness> {
    let rows = nerve_store::load_module_facts(conn, repo_id)?;
    let mut report = IndexFreshness {
        files_total: rows.len(),
        ..IndexFreshness::default()
    };
    let mut cache = FreshnessCache::new(prober);
    for (rel_path, row) in rows {
        if report.files_probed >= cap {
            report.truncated = true;
            break;
        }
        report.files_probed += 1;
        match cache.evaluate(&rel_path, &row.content_hash) {
            Freshness::Fresh => report.fresh += 1,
            Freshness::Stale => report.stale += 1,
            Freshness::FileMissing => report.missing += 1,
            Freshness::Refused => report.refused += 1,
            Freshness::Unreadable => report.unreadable += 1,
        }
    }
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::probe::RepositoryProber;

    fn indexed(files: &[(&str, &str)]) -> (tempfile::TempDir, std::path::PathBuf, String) {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().to_path_buf();
        for (path, source) in files {
            let full = root.join(path);
            std::fs::create_dir_all(full.parent().unwrap()).unwrap();
            std::fs::write(full, source).unwrap();
        }
        let outcome =
            crate::init_with_project_id(&root, Some("00000000000000000000000000000001")).unwrap();
        crate::index_repository(&root).unwrap();
        let repo_id = nerve_core::ids::repository_id(&outcome.project_id);
        (dir, root, repo_id)
    }

    #[test]
    fn a_file_that_parses_cleanly_is_not_reported_as_partial() {
        let (_dir, root, repo_id) = indexed(&[("src/a.ts", "export const a = 1;\n")]);
        let conn = nerve_store::open(&crate::config::db_path(&root)).unwrap();
        assert_eq!(partial_parses(&conn, &repo_id).unwrap(), Vec::new());
    }

    #[test]
    fn a_file_with_a_syntax_error_is_listed() {
        let (_dir, root, repo_id) = indexed(&[
            ("src/a.ts", "export const a = 1;\n"),
            ("src/broken.ts", "export function oops( {\n"),
        ]);
        let conn = nerve_store::open(&crate::config::db_path(&root)).unwrap();
        let partial = partial_parses(&conn, &repo_id).unwrap();
        let paths: Vec<&str> = partial.iter().map(|p| p.rel_path.as_str()).collect();
        assert_eq!(paths, vec!["src/broken.ts"]);
        assert_eq!(partial[0].language, "typescript");
    }

    #[test]
    fn freshness_tracks_the_files_and_reports_its_own_cap() {
        let (_dir, root, repo_id) = indexed(&[
            ("src/a.ts", "export const a = 1;\n"),
            ("src/b.ts", "export const b = 1;\n"),
        ]);
        let conn = nerve_store::open(&crate::config::db_path(&root)).unwrap();
        let prober = RepositoryProber::new(&root).unwrap();

        let all = index_freshness(&conn, &repo_id, &prober, 100).unwrap();
        assert_eq!((all.files_total, all.files_probed, all.fresh), (2, 2, 2));
        assert!(!all.truncated);

        std::fs::write(root.join("src/b.ts"), "export const b = 2;\n").unwrap();
        let changed = index_freshness(&conn, &repo_id, &prober, 100).unwrap();
        assert_eq!((changed.fresh, changed.stale), (1, 1));

        std::fs::remove_file(root.join("src/a.ts")).unwrap();
        let gone = index_freshness(&conn, &repo_id, &prober, 100).unwrap();
        assert_eq!(gone.missing, 1);

        let capped = index_freshness(&conn, &repo_id, &prober, 1).unwrap();
        assert_eq!((capped.files_total, capped.files_probed), (2, 1));
        assert!(capped.truncated);
    }
}
