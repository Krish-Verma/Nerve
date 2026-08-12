//! *Can I trust this index right now?* — asked once, answered in one place.
//!
//! Slice 7c-i built this judgement inside `nerve check`, which was correct while a shell was the
//! only place to ask it. It is not correct once HTTP, MCP and the interface ask the same question:
//! four copies of a five-valued verdict is four places for one of them to answer `stale` where
//! another answers `unverified`, and a cross-surface disagreement about *whether the answer can be
//! believed* is worse than any disagreement about the answer itself.
//!
//! So the judgement lives here, beside the two measurements it is made of
//! ([`crate::inspect::index_freshness`] and [`crate::inspect::untracked_files`]), and every surface
//! renders [`TrustReport`] rather than deciding anything. What stays with each surface is what
//! genuinely belongs to it: the CLI owns the mapping from verdict to **exit code**, because an exit
//! code is a shell's vocabulary and HTTP has none.
//!
//! # Nothing here writes, and nothing here repairs
//!
//! The whole value of the answer is that it was not produced by the thing being judged. The
//! connection every caller hands over is opened `PRAGMA query_only`, so SQLite refuses a write by
//! construction; [`trust`] additionally performs no `init`, no migration and no re-index. It reads
//! the database, walks the tree, and returns a judgement.
//!
//! # The two families of evidence, and why they never collapse
//!
//! [`Verdict::Stale`] and [`Verdict::Unverified`] have the same consequence for a caller — do not
//! rely on this index — and **different evidence**, which is why they are two values:
//!
//! - `stale` is a *measurement*. A file's bytes no longer hash to what was extracted, a file the
//!   index describes is gone, or a file exists that no row describes.
//! - `unverified` is the *absence* of a measurement. The sweep hit its cap, or a path could not be
//!   read, so part of the tree was never looked at. Nothing was observed to be wrong.
//!
//! A two-state `fresh`/`stale` payload would have to lie in one direction: either it reports "the
//! index is stale" about a tree nothing observed to have changed, or it reports a clean bill of
//! health issued without looking. The CLI gives them the same exit code because a shell has only
//! one way to say "do not proceed"; that is a property of exit codes, not of the evidence, and no
//! surface with room for a payload may repeat it.

use std::path::Path;

use nerve_store::{Connection, StatusReport};

use crate::error::Result;
use crate::inspect::{index_freshness, untracked_files, IndexFreshness, UntrackedFiles};
use crate::probe::RepositoryProber;

/// How many indexed files a trust sweep re-hashes before it reports a partial sweep.
///
/// A repository can hold a hundred thousand files and this answer is wanted in a pre-commit hook
/// and on an interactive screen. When the cap bites, the sweep is deliberately **not** reported as
/// clean — see [`judge_freshness`].
pub const PROBE_CAP: usize = 5_000;

/// What the trust judgement decided.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verdict {
    /// Every indexed file still hashes to what was extracted, and nothing new is untracked.
    Current,
    /// There is nothing to judge: no database, no schema, or nothing ever indexed.
    NoIndex,
    /// An index exists but cannot be used as it stands: the schema is behind, or a run is open.
    Unusable,
    /// The index is internally sound and describes a tree that has moved on.
    Stale,
    /// The index is internally sound and the sweep could not establish whether it is current.
    ///
    /// Separate from [`Verdict::Stale`] because the evidence is different — nothing was observed
    /// to have changed, some of the tree was simply never looked at — and identical to it in exit
    /// code, because "I could not check" is not a clean bill of health.
    Unverified,
}

impl Verdict {
    /// Every verdict, in the order a reader meets them: the good one, then the four that are not.
    ///
    /// Generated from rather than typed beside the enum wherever a surface offers the set, so a
    /// sixth verdict reaches every surface the day it exists — which is what
    /// `crates/nerve-server/tests/ui_vocabulary.rs` holds the interface to.
    pub const ALL: [Verdict; 5] = [
        Verdict::Current,
        Verdict::NoIndex,
        Verdict::Unusable,
        Verdict::Stale,
        Verdict::Unverified,
    ];

    /// Canonical name, used in rendered output, JSON and MCP alike.
    pub fn as_str(self) -> &'static str {
        match self {
            Verdict::Current => "current",
            Verdict::NoIndex => "no_index",
            Verdict::Unusable => "unusable",
            Verdict::Stale => "stale",
            Verdict::Unverified => "unverified",
        }
    }

    /// What this verdict means, in one sentence, said in exactly one place.
    ///
    /// The generic meaning of the value, as distinct from [`TrustReport::reason`], which is the
    /// measured particulars of one repository at one moment.
    pub fn note(self) -> &'static str {
        match self {
            Verdict::Current => {
                "Every indexed file was re-read and still hashes to what was extracted from it, and \
                 the tree holds no file the index has never seen. Answers drawn from this index \
                 describe the working tree as it is now."
            }
            Verdict::NoIndex => {
                "There is nothing to judge. No database, no migrated schema, or a database that has \
                 never been indexed. This is not a claim that the repository is empty — nothing was \
                 measured at all."
            }
            Verdict::Unusable => {
                "An index exists and cannot be used as it stands: its schema is not the one this \
                 build reads, or an extractor run is still marked running and the graph is a \
                 half-written one. The tree was not swept, because comparing it against a graph \
                 nobody can read would be work in service of an unusable answer."
            }
            Verdict::Stale => {
                "The index is internally sound and it describes a tree that has moved on. This is a \
                 measurement: a file changed, a file the index describes is gone, or a file exists \
                 that no row describes."
            }
            Verdict::Unverified => {
                "The index is internally sound and this run could not establish whether it is \
                 current. Nothing was observed to have changed — part of the tree was never looked \
                 at, because the sweep reached its cap or a path could not be read. This is the \
                 absence of a measurement rather than a finding of staleness."
            }
        }
    }

    /// Whether an answer drawn from this index may be relied on without qualification.
    ///
    /// Exactly one verdict qualifies, and the three that are not [`Verdict::Current`] are not
    /// ranked against each other here: a caller acts on the verdict, not on a grade.
    pub fn is_current(self) -> bool {
        matches!(self, Verdict::Current)
    }
}

impl std::fmt::Display for Verdict {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.as_str())
    }
}

/// What the sweep measured, present only when a sweep was worth running.
#[derive(Debug, Clone)]
pub struct TrustMeasurement {
    /// The re-hash of every file the index has a row for.
    pub freshness: IndexFreshness,
    /// The walk of the tree, which is the only thing that can see an **added** file.
    pub untracked: UntrackedFiles,
}

/// One complete answer to *can I trust this index right now?*
///
/// Every surface renders this and none of them derives anything from it. `measured` is `None`
/// rather than a zeroed report whenever no sweep ran: `0 stale` would be a measurement that was
/// never taken, and a client reading it as one would report an unjudged index as a clean one.
#[derive(Debug, Clone)]
pub struct TrustReport {
    /// The judgement.
    pub verdict: Verdict,
    /// The measured particulars behind this verdict, for this repository at this moment.
    pub reason: String,
    /// Schema version found on disk, absent when the database has never been migrated.
    pub schema_version: Option<i64>,
    /// Extractor runs still marked running, counted once however they were reached.
    pub runs_running: usize,
    /// The cap the sweep was run with, so a `truncated` report can be read against it.
    pub probe_cap: usize,
    /// The sweep, when one ran.
    pub measured: Option<TrustMeasurement>,
}

/// Whether the schema on disk is one this build can read at all.
///
/// Answered on its own, and answered first, because every other question below reads a table:
/// a database written by an older or newer build may not have the tables `status` queries, and
/// reporting that as an internal error would tell a script Nerve broke when in fact the index
/// needs migrating.
pub fn judge_schema(schema_version: Option<i64>) -> Option<(Verdict, String)> {
    let Some(version) = schema_version else {
        return Some((
            Verdict::NoIndex,
            "the database has never been migrated; run `nerve init`".to_string(),
        ));
    };
    if version != nerve_store::SCHEMA_VERSION {
        return Some((
            Verdict::Unusable,
            format!(
                "schema version {version} is not the supported version {}; \
                 run `nerve index` to migrate",
                nerve_store::SCHEMA_VERSION
            ),
        ));
    }
    None
}

/// Whether the index is usable at all, before anything is compared against the disk.
///
/// `None` means there is an index worth measuring the tree against; anything else is a refusal
/// and the freshness sweep is not run, because re-hashing a tree to compare it with a graph that
/// cannot be read would be work in service of an answer nobody can use.
pub fn judge_index(
    schema_version: Option<i64>,
    ever_indexed: bool,
    runs_running: usize,
) -> Option<(Verdict, String)> {
    if let Some(refusal) = judge_schema(schema_version) {
        return Some(refusal);
    }
    if !ever_indexed {
        return Some((
            Verdict::NoIndex,
            "the database is initialized and nothing has been indexed; run `nerve index`"
                .to_string(),
        ));
    }
    if runs_running > 0 {
        return Some((
            Verdict::Unusable,
            format!(
                "{runs_running} extractor run(s) are still marked running; the last index did \
                 not finish, so the graph is a half-written one"
            ),
        ));
    }
    None
}

/// Whether the index still describes the tree, given the sweep and the untracked walk.
///
/// The five freshness counts and the added count fall into two families, and they are kept apart
/// because the evidence behind them is different:
///
/// - **observed divergence** — `stale` (the file changed), `missing` (the indexed file is gone)
///   and `added` (a file exists that no row describes). Each is a measurement, and any of them
///   means the graph and the tree disagree.
/// - **not established** — `refused` (the path-safety check would not read it), `unreadable`
///   (allowed but the bytes would not come) and `truncated` (the cap stopped the sweep). Nothing
///   here says the index is wrong; it says this run did not find out.
///
/// Both are non-`current`. The second family is the reason a truncated sweep can never report a
/// clean result: a partial sweep reported as current would be a clean bill of health issued
/// without looking, which is exactly the failure mode this judgement exists to prevent.
///
/// `cap` is taken as an argument rather than read off [`PROBE_CAP`] so the sentence a truncated
/// sweep produces names the cap that actually bit. A message quoting a constant the caller did not
/// use would be a true-looking number that is not the one the sweep stopped at.
pub fn judge_freshness(freshness: &IndexFreshness, added: usize, cap: usize) -> (Verdict, String) {
    if freshness.stale + freshness.missing + added > 0 {
        return (
            Verdict::Stale,
            format!(
                "{} indexed file(s) changed, {} no longer exist and {} file(s) are not indexed \
                 at all",
                freshness.stale, freshness.missing, added
            ),
        );
    }
    if freshness.truncated {
        return (
            Verdict::Unverified,
            format!(
                "the sweep compared {} of {} indexed file(s) before reaching its {cap}-file \
                 cap; the rest were never looked at",
                freshness.files_probed, freshness.files_total
            ),
        );
    }
    if freshness.refused + freshness.unreadable > 0 {
        return (
            Verdict::Unverified,
            format!(
                "{} indexed file(s) were refused by the path-safety check and {} could not be \
                 read, so they were never compared",
                freshness.refused, freshness.unreadable
            ),
        );
    }
    (
        Verdict::Current,
        format!(
            "{} indexed file(s) still hash to what was extracted, and nothing in the tree is \
             untracked",
            freshness.fresh
        ),
    )
}

/// Runs left open, counted once whether they reach us through `runs` or through `last_run`.
pub fn runs_still_running(report: &StatusReport) -> usize {
    let mut count = report
        .runs
        .iter()
        .filter(|run| run.status == "running")
        .count();
    if let Some(last) = &report.last_run {
        if last.status == "running" && !report.runs.iter().any(|run| run.run_id == last.run_id) {
            count += 1;
        }
    }
    count
}

/// Judge one opened index against the tree it claims to describe.
///
/// `conn` must already be open; opening it is the caller's job because each surface opens it
/// differently and each opens it `query_only`. `root` is the repository root, which is where the
/// sweep reads from — the prober is built here rather than taken as an argument so the order of
/// the checks is fixed in one place: the schema and the index are judged **before** anything
/// touches the repository configuration, so a database from an older build reports `unusable` with
/// the remedy rather than failing on a config file the migration would have rewritten.
pub fn trust(conn: &Connection, root: &Path, cap: usize) -> Result<TrustReport> {
    let unmeasured =
        |verdict: Verdict, reason: String, schema: Option<i64>, running: usize| TrustReport {
            verdict,
            reason,
            schema_version: schema,
            runs_running: running,
            probe_cap: cap,
            measured: None,
        };

    let schema_version = nerve_store::schema_version(conn)?;
    if let Some((verdict, reason)) = judge_schema(schema_version) {
        return Ok(unmeasured(verdict, reason, schema_version, 0));
    }

    let report = nerve_store::status(conn)?;
    let runs_running = runs_still_running(&report);
    let repository = nerve_store::repository(conn)?;
    let ever_indexed = report.last_run.is_some() && repository.is_some();
    if let Some((verdict, reason)) = judge_index(schema_version, ever_indexed, runs_running) {
        return Ok(unmeasured(verdict, reason, schema_version, runs_running));
    }
    let repo_id = repository
        .expect("ever_indexed implies a repository row")
        .repo_id;

    // Freshness is computed by re-reading the repository, so the reader is built from the
    // repository root and enforces the Slice 1 path rules on every path the database supplies.
    let prober = RepositoryProber::new(root)?;
    let freshness = index_freshness(conn, &repo_id, &prober, cap)?;
    // The sweep above walks the cache, so it can only ask about files the index already knows.
    // A file added since the last index has no row to compare and would otherwise be invisible.
    let untracked = untracked_files(root, conn, &repo_id)?;

    let (verdict, reason) = judge_freshness(&freshness, untracked.added.len(), cap);
    Ok(TrustReport {
        verdict,
        reason,
        schema_version,
        runs_running,
        probe_cap: cap,
        measured: Some(TrustMeasurement {
            freshness,
            untracked,
        }),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn swept(fresh: usize) -> IndexFreshness {
        IndexFreshness {
            files_total: fresh,
            files_probed: fresh,
            fresh,
            ..IndexFreshness::default()
        }
    }

    #[test]
    fn an_index_that_matches_the_tree_is_current() {
        let (verdict, _) = judge_freshness(&swept(12), 0, PROBE_CAP);
        assert_eq!(verdict, Verdict::Current);
        assert!(verdict.is_current());
    }

    /// Changed, deleted and added are three different observations and one verdict. Each is
    /// asserted on its own so that dropping any single one from the sum is a test failure.
    #[test]
    fn changed_deleted_and_added_each_make_the_index_stale() {
        let mut changed = swept(12);
        changed.fresh = 11;
        changed.stale = 1;
        assert_eq!(judge_freshness(&changed, 0, PROBE_CAP).0, Verdict::Stale);

        let mut deleted = swept(12);
        deleted.fresh = 11;
        deleted.missing = 1;
        assert_eq!(judge_freshness(&deleted, 0, PROBE_CAP).0, Verdict::Stale);

        assert_eq!(
            judge_freshness(&swept(12), 1, PROBE_CAP).0,
            Verdict::Stale,
            "an added file is not in the cache the sweep walks, so only the untracked walk sees it"
        );
    }

    /// The rule the whole judgement rests on: a sweep that stopped early has not seen the tree, so
    /// it cannot certify it.
    #[test]
    fn a_truncated_sweep_is_never_a_clean_result() {
        let truncated = IndexFreshness {
            files_total: PROBE_CAP * 2,
            files_probed: PROBE_CAP,
            fresh: PROBE_CAP,
            truncated: true,
            ..IndexFreshness::default()
        };
        let (verdict, reason) = judge_freshness(&truncated, 0, PROBE_CAP);
        assert_ne!(verdict, Verdict::Current);
        assert_eq!(verdict, Verdict::Unverified);
        assert!(reason.contains("never looked at"), "{reason}");
        // The cap in the sentence is the cap that bit, not the constant.
        let (_, other) = judge_freshness(&truncated, 0, 7);
        assert!(other.contains("7-file"), "{other}");
    }

    /// A file the sweep was not allowed to read, or could not, is not a fresh file.
    #[test]
    fn a_file_the_sweep_could_not_compare_is_not_counted_as_fresh() {
        let mut refused = swept(12);
        refused.fresh = 11;
        refused.refused = 1;
        assert_eq!(
            judge_freshness(&refused, 0, PROBE_CAP).0,
            Verdict::Unverified
        );

        let mut unreadable = swept(12);
        unreadable.fresh = 11;
        unreadable.unreadable = 1;
        assert_eq!(
            judge_freshness(&unreadable, 0, PROBE_CAP).0,
            Verdict::Unverified
        );
    }

    /// Observed divergence outranks "could not tell": the caller can act on the first.
    #[test]
    fn observed_staleness_outranks_an_incomplete_sweep() {
        let mut both = swept(12);
        both.fresh = 10;
        both.stale = 1;
        both.refused = 1;
        both.truncated = true;
        assert_eq!(judge_freshness(&both, 0, PROBE_CAP).0, Verdict::Stale);
    }

    /// The schema gate is the one question answered before any table is read, so it is asserted
    /// on its own as well as through [`judge_index`].
    #[test]
    fn the_schema_is_judged_before_any_table_is_queried() {
        assert_eq!(judge_schema(Some(nerve_store::SCHEMA_VERSION)), None);
        assert_eq!(judge_schema(None).unwrap().0, Verdict::NoIndex);
        assert_eq!(
            judge_schema(Some(nerve_store::SCHEMA_VERSION + 1))
                .unwrap()
                .0,
            Verdict::Unusable,
            "a database from a newer build is unusable, not merely stale"
        );
    }

    #[test]
    fn an_index_is_judged_before_the_tree_is_swept() {
        assert_eq!(
            judge_index(Some(nerve_store::SCHEMA_VERSION), true, 0),
            None
        );

        let (verdict, _) = judge_index(None, false, 0).expect("no schema is not judgeable");
        assert_eq!(verdict, Verdict::NoIndex);

        let (verdict, reason) = judge_index(Some(nerve_store::SCHEMA_VERSION - 1), true, 0)
            .expect("an old schema is not judgeable");
        assert_eq!(verdict, Verdict::Unusable);
        assert!(reason.contains("migrate"), "{reason}");

        let (verdict, _) = judge_index(Some(nerve_store::SCHEMA_VERSION), false, 0)
            .expect("an empty index is not judgeable");
        assert_eq!(verdict, Verdict::NoIndex);

        let (verdict, reason) = judge_index(Some(nerve_store::SCHEMA_VERSION), true, 1)
            .expect("an open run is not judgeable");
        assert_eq!(verdict, Verdict::Unusable);
        assert!(reason.contains("did not finish"), "{reason}");
    }

    #[test]
    fn verdict_names_are_distinct_and_stable() {
        let names: Vec<&str> = Verdict::ALL
            .iter()
            .map(|verdict| verdict.as_str())
            .collect();
        assert_eq!(
            names,
            ["current", "no_index", "unusable", "stale", "unverified"]
        );
    }

    /// The separation the whole vocabulary exists for, asserted on the prose as well as the names.
    ///
    /// A `note()` shared between `stale` and `unverified` would put every surface back where a
    /// two-state payload was: reporting *I could not check* as *it changed*.
    #[test]
    fn stale_and_unverified_are_never_one_value_or_one_sentence() {
        assert_ne!(Verdict::Stale, Verdict::Unverified);
        assert_ne!(Verdict::Stale.note(), Verdict::Unverified.note());
        assert!(
            Verdict::Unverified
                .note()
                .contains("absence of a measurement"),
            "{}",
            Verdict::Unverified.note()
        );
        assert!(
            Verdict::Stale.note().contains("measurement"),
            "{}",
            Verdict::Stale.note()
        );
        // Exactly one verdict is a clearance, so "not current" can never be reached by accident.
        let current: Vec<Verdict> = Verdict::ALL
            .into_iter()
            .filter(|verdict| verdict.is_current())
            .collect();
        assert_eq!(current, vec![Verdict::Current]);
        // And no two verdicts share a sentence.
        for (index, one) in Verdict::ALL.iter().enumerate() {
            for other in &Verdict::ALL[index + 1..] {
                assert_ne!(one.note(), other.note(), "{one} and {other} share a note");
                assert!(!one.note().is_empty());
            }
        }
    }
}
