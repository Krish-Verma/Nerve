//! Read-only questions about what indexing knew and what it could not do.
//!
//! Three of them, and all three are about the *absence* of knowledge, which this product treats
//! as first-class rather than hiding (ARCHITECTURE.md invariant 4):
//!
//! - **Partial parses.** A file whose tree-sitter parse produced an `ERROR` node was still
//!   indexed, but what came out of it is incomplete. The per-file flag lives in the extractor's
//!   own payload in `module_facts`, so reading it back is this crate's job — `nerve-store`
//!   deliberately treats that payload as opaque text and must not learn to parse it.
//! - **Index freshness.** `module_facts` records the content hash every module was extracted at.
//!   Comparing that against the file on disk says whether the graph still describes the
//!   repository. The comparison is a `nerve-store` function; supplying the file bytes is a
//!   path-safety decision and therefore belongs here, next to the root.
//! - **Untracked files.** Freshness walks `module_facts`, so it can only ask about files the
//!   index already has a row for. A file *added* to the repository since the last index has no
//!   row, is never probed, and is therefore invisible to [`index_freshness`] — a repository can
//!   grow a hundred new modules and every recorded hash still match. "Does the graph still
//!   describe this repository" needs the other direction as well, and that direction is a
//!   discovery walk compared against the same cache.
//!
//! None of the three writes anything, and none of them reads a byte it did not have to: the
//! discovery walk is metadata only ([`crate::discover`]), and only a path that is new to the
//! index is opened.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use nerve_store::{Connection, FileProber, Freshness, FreshnessCache};

use crate::config::Config;
use crate::discover::discover;
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

/// Files the repository has now and the index has no row for.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct UntrackedFiles {
    /// Repository-relative paths a re-index would add, sorted.
    pub added: Vec<String>,
    /// Discovered files with no row that the indexer could not have read either.
    ///
    /// Too large for `index.max_file_bytes`, unreadable, or not UTF-8 — the same three refusals
    /// the pipeline makes when it loads the tree, which is why they have no row. They are counted
    /// rather than reported as additions: re-indexing would not produce a row for them, so
    /// calling them additions would make "is this index current?" permanently answer *no* with
    /// nothing the answer's reader could do about it. `nerve index` already reports them, as the
    /// failed-file count that makes a run partial.
    pub unindexable: usize,
}

/// Walk the repository and report the files the extraction cache has never heard of.
///
/// The set difference is `discover(root) − module_facts(repo_id)`. Both sides are the same
/// population by construction: the pipeline writes one `module_facts` row per file it loaded out
/// of the very same walk, documents included, so a path present in one and absent from the other
/// is a real difference and not a units mismatch.
///
/// The walk itself reads no file contents. The only bytes this reads are those of a path that is
/// new to the index, and only to decide whether the indexer could have read it — which is
/// bounded by the size of the change, not by the size of the repository.
pub fn untracked_files(root: &Path, conn: &Connection, repo_id: &str) -> Result<UntrackedFiles> {
    let config = Config::load(root)?;
    let discovered = discover(root, &config)?;
    let indexed: BTreeSet<String> = nerve_store::load_module_facts(conn, repo_id)?
        .into_keys()
        .collect();

    let mut report = UntrackedFiles::default();
    for file in &discovered.files {
        if indexed.contains(&file.rel_path) {
            continue;
        }
        if indexable(&file.abs_path, config.index.max_file_bytes) {
            report.added.push(file.rel_path.clone());
        } else {
            report.unindexable += 1;
        }
    }
    Ok(report)
}

/// Whether the pipeline's loader would have accepted this file.
///
/// Deliberately the same three conditions `crate::pipeline` applies when it reads the tree —
/// within the size ceiling, readable, valid UTF-8. If those ever diverge, this reports a file as
/// an addition that indexing will not add, which is why the rule is stated in one shape here and
/// not spread across several.
fn indexable(abs_path: &Path, max_file_bytes: u64) -> bool {
    let Ok(metadata) = std::fs::metadata(abs_path) else {
        return false;
    };
    if metadata.len() > max_file_bytes {
        return false;
    }
    let Ok(bytes) = std::fs::read(abs_path) else {
        return false;
    };
    std::str::from_utf8(&bytes).is_ok()
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

    /// The gap freshness cannot see: a file the index has no row for is never probed, so every
    /// recorded hash still matches while the repository has grown a module.
    #[test]
    fn an_added_file_is_invisible_to_freshness_and_visible_to_untracked_files() {
        let (_dir, root, repo_id) = indexed(&[("src/a.ts", "export const a = 1;\n")]);
        let conn = nerve_store::open(&crate::config::db_path(&root)).unwrap();
        let prober = RepositoryProber::new(&root).unwrap();

        assert_eq!(
            untracked_files(&root, &conn, &repo_id).unwrap(),
            UntrackedFiles::default()
        );

        std::fs::write(root.join("src/b.ts"), "export const b = 1;\n").unwrap();

        let freshness = index_freshness(&conn, &repo_id, &prober, 100).unwrap();
        assert_eq!(
            (freshness.fresh, freshness.stale, freshness.missing),
            (1, 0, 0),
            "freshness has no row for the new file, so it reports a wholly fresh index"
        );

        let untracked = untracked_files(&root, &conn, &repo_id).unwrap();
        assert_eq!(untracked.added, vec!["src/b.ts".to_string()]);
        assert_eq!(untracked.unindexable, 0);
    }

    /// Documents are in the same cache as source, so a new `.md` is an addition too.
    #[test]
    fn an_added_document_counts_as_an_addition() {
        let (_dir, root, repo_id) = indexed(&[("src/a.ts", "export const a = 1;\n")]);
        let conn = nerve_store::open(&crate::config::db_path(&root)).unwrap();
        std::fs::write(root.join("README.md"), "# Title\n").unwrap();
        let untracked = untracked_files(&root, &conn, &repo_id).unwrap();
        assert_eq!(untracked.added, vec!["README.md".to_string()]);
    }

    /// A file the loader would refuse is not an addition: re-indexing would not add it, so
    /// reporting it as one would make the question permanently unanswerable in the negative.
    #[test]
    fn a_file_the_indexer_could_not_read_is_counted_not_called_an_addition() {
        let (_dir, root, repo_id) = indexed(&[("src/a.ts", "export const a = 1;\n")]);
        let conn = nerve_store::open(&crate::config::db_path(&root)).unwrap();
        std::fs::write(root.join("src/binary.ts"), [0xff, 0xfe, 0x41]).unwrap();
        let untracked = untracked_files(&root, &conn, &repo_id).unwrap();
        assert_eq!(untracked.added, Vec::<String>::new());
        assert_eq!(untracked.unindexable, 1);
    }
}
