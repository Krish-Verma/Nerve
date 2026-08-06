//! The historical model: `git_commit`, `git_change`, `git_rename_hypothesis`,
//! `git_rename_analysis` and `git_history_ingest` (schema v6, Slice 12b; v7, Slice 12c-ii).
//!
//! These four tables are **not** part of the evidence graph and are deliberately absent from the
//! canonical dump. They record what the repository *was*, read out of the Git object store as
//! primary-source fact. `docs/plans/slice-12b-historical-model.md` §3 states why a tree diff is
//! not routed through Assertion / Observation / AssertionState, and [`crate::schema`]'s `V6` doc
//! comment restates it beside the DDL.
//!
//! Three properties of this module are load-bearing and are the reason it is not a thin wrapper
//! over `execute`:
//!
//! 1. **An absence is never inferred.** A commit with zero [`ChangeRow`]s is qualified by
//!    [`CommitRow::changes_enumerated`], so "the parent tree was unreadable", "this is a merge",
//!    "a bound was hit" and "nothing changed" are four stored facts rather than one row count.
//!    Every read that can return an empty vector returns the qualifying commit alongside it.
//! 2. **Only [`insert_commit`] tolerates a duplicate.** A commit oid names an immutable object, so
//!    the same oid is the same commit and `INSERT OR IGNORE` is exactly right there. It is wrong
//!    everywhere else, and the reason is measured rather than stylistic: Slice 3b lost graph rows
//!    to an `INSERT OR IGNORE` that swallowed `NOT NULL` violations and exited zero
//!    (`crates/nerve-index/src/pipeline.rs:654-666`). [`insert_changes`], [`insert_renames`] and
//!    [`insert_rename_analysis`] use a plain `INSERT`, so a constraint or foreign-key violation is
//!    an error.
//! 3. **Every read has a total order.** History fixtures use fixed synthetic dates, so ties on
//!    `committer_time` are guaranteed rather than unlikely; an order that stopped at the timestamp
//!    would be a flaky test waiting for a fixture to grow a second commit in the same second.

use std::collections::{BTreeMap, BTreeSet, VecDeque};

use rusqlite::{params, Connection, Row};

use nerve_core::vocab::{
    ChangeKind, ChangesEnumerated, FirstObservedKind, HistoryFreshness, ParentCompleteness,
    PathRole, RenameAmbiguity, RenameAnalysisCompleteness, RenameEvidence, SimilarityUnmeasured,
    SummaryTruncation, WalkTermination,
};

use crate::error::{Result, StoreError};
use crate::select::path_kinds_sql;

/// One recorded commit.
///
/// The times are epoch seconds exactly as the commit object records them — signed, because a
/// repository can carry a commit dated before 1970 — and the timezone is kept as the text offset
/// the object carries rather than normalised away, so a local commit time is still recoverable.
///
/// `author_ident` and `committer_ident` are `None` unless identity capture was explicitly
/// requested. Not one question the historical model answers asks *who*, so contributor names and
/// addresses are third-party personal data with no query behind them; the columns exist so that
/// enabling them later needs no migration.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommitRow {
    /// 40 lowercase hex characters naming this commit object.
    pub commit_oid: String,
    /// The tree this commit points at.
    pub tree_oid: String,
    /// Parent oids in the order the commit object lists them. Empty for a root commit.
    pub parent_oids: Vec<String>,
    /// Which of the five parent situations this commit is in.
    pub parent_completeness: ParentCompleteness,
    /// Which of the four silences a commit with no [`ChangeRow`]s is.
    pub changes_enumerated: ChangesEnumerated,
    /// Author time, epoch seconds, signed.
    pub author_time: i64,
    /// Author timezone offset, as the object records it (for example `+0000`).
    pub author_tz: String,
    /// Committer time, epoch seconds, signed. This is what [`commit_log`] orders by.
    pub committer_time: i64,
    /// Committer timezone offset, as the object records it.
    pub committer_tz: String,
    /// Author identity, `None` unless identity capture was requested.
    pub author_ident: Option<String>,
    /// Committer identity, `None` unless identity capture was requested.
    pub committer_ident: Option<String>,
    /// First line of the commit message, bounded and lossily converted. Untrusted text on every
    /// surface: it is repository prose, it is attacker-influencable wherever contributions are
    /// accepted, and it is never interpreted.
    pub summary: String,
    /// Whether [`CommitRow::summary`] is the whole first line or a cut one.
    ///
    /// Per record, because the per-repository tally
    /// (`git_history_ingest.refusals["history-summary-truncated"]`) cannot say which summary was
    /// cut. [`SummaryTruncation::Unknown`] is what the v6→v7 migration wrote and must never be
    /// written by a fresh ingest — a writer that knows the bound knows the answer.
    pub summary_truncation: SummaryTruncation,
    /// Whether the commit has more than one parent. Denormalised from `parent_oids` so that
    /// counting merges does not require decoding a JSON array per row.
    pub is_merge: bool,
}

/// What one commit did to one path.
///
/// A rename is **not** here. Git records no rename, so a rename is a hypothesis
/// ([`RenameRow`]) and never a change kind.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChangeRow {
    /// The commit this change belongs to. Must already be recorded — see [`insert_changes`].
    pub commit_oid: String,
    /// Repository-relative path, as recorded in the tree.
    pub path: String,
    /// What happened to the path.
    pub change_kind: ChangeKind,
    /// Content at this commit. `None` for a deletion.
    pub blob_oid: Option<String>,
    /// Content at the parent. `None` for an addition.
    pub prev_blob_oid: Option<String>,
    /// File mode at this commit, as the tree entry records it.
    pub mode: Option<i64>,
    /// File mode at the parent.
    pub prev_mode: Option<i64>,
}

/// A proposal that one path became another, with what it rests on and how ambiguous it is kept
/// apart.
///
/// There is no score. `evidence` and `ambiguity` are separate columns because they are separate
/// facts, and when one blob matches several paths every pairing is recorded and none is promoted.
///
/// **Two blob oids, not one.** An exact-content hypothesis has
/// `from_blob_oid == to_blob_oid` — that identity *is* the evidence. A similarity hypothesis has
/// two different blobs and a measurement of how much content they share. The schema's `CHECK`
/// enforces which combination goes with which evidence, so the two kinds cannot be blended by a
/// writer that forgot the rule.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RenameRow {
    /// The commit in which the deletion and the addition both appear.
    pub commit_oid: String,
    /// The deleted path. Usually not present in the current tree at all.
    pub from_path: String,
    /// The added path.
    pub to_path: String,
    /// What the hypothesis rests on.
    pub evidence: RenameEvidence,
    /// The blob the deleted path named at the parent.
    pub from_blob_oid: String,
    /// The blob the added path names at this commit. Equal to
    /// [`RenameRow::from_blob_oid`] exactly when the evidence is
    /// [`RenameEvidence::ExactContent`].
    pub to_blob_oid: String,
    /// Which method produced this row. On the row rather than reached by a join, so a hypothesis
    /// names its own producer instead of being attributed by a caller that might forget.
    pub matcher_id: String,
    /// The producing method's version. Changing the method is a version bump, never a silent
    /// redefinition of what the measurement means.
    pub matcher_version: String,
    /// Numerator of the match measurement. `None` for [`RenameEvidence::ExactContent`], which
    /// carries no measurement at all rather than a perfect one.
    pub match_numerator: Option<i64>,
    /// Denominator of the match measurement. Two integers rather than a float: an exact rational
    /// says *what was counted*, where a float is comparable against anything and rounds.
    pub match_denominator: Option<i64>,
    /// How many ways the pairing could have been drawn.
    pub ambiguity: RenameAmbiguity,
}

/// What one matcher's candidate set for one commit was, and how much of it was measured.
///
/// **Per commit rather than per row, because the decisive case has no row.** When a bound refuses
/// the candidate set the commit records no similarity hypothesis at all, and a per-row flag cannot
/// state that — an absence would have to be interpreted, which is the failure
/// [`CommitRow::changes_enumerated`] exists to prevent one table over.
///
/// Exact-content renames get no [`AnalysisRow`], and that is a claim rather than an omission: the
/// exact matcher reads no blob content, so it is complete exactly when the diff was enumerated,
/// which [`CommitRow::changes_enumerated`] already records. Giving it a row with a meaningless
/// threshold would be inventing a measurement to fill a column.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AnalysisRow {
    /// The commit whose candidate set this describes. Must already be recorded.
    pub commit_oid: String,
    /// Which method analysed it. Part of the primary key, so a second matcher can analyse the same
    /// commit later without a migration.
    pub matcher_id: String,
    /// The analysing method's version.
    pub matcher_version: String,
    /// Numerator of the admission threshold in force for this run.
    pub threshold_numerator: i64,
    /// Denominator of the admission threshold. The threshold is stored rather than assumed,
    /// because a measurement rendered without it is a percentage from nowhere.
    pub threshold_denominator: i64,
    /// Deletions in the commit that were considered as candidates.
    pub deletions_considered: i64,
    /// Additions in the commit that were considered as candidates.
    pub additions_considered: i64,
    /// Candidate pairs the run set out to measure.
    pub pairs_considered: i64,
    /// Candidate pairs it actually measured. Never more than
    /// [`AnalysisRow::pairs_considered`] — the schema refuses it.
    pub pairs_measured: i64,
    /// Whether the rows present are the whole answer for this commit.
    pub completeness: RenameAnalysisCompleteness,
    /// Why the unmeasured pairs went unmeasured, by reason, with counts. Empty is a claim, so it
    /// is stored as `{}` rather than as a `NULL` — the same discipline
    /// [`IngestRow::refusals`] follows.
    pub unmeasured: BTreeMap<SimilarityUnmeasured, i64>,
}

/// What one history ingest read, and what it could not.
///
/// One row per repository: the latest ingest replaces the previous one, because the questions it
/// answers — *is this repository shallow, where is the boundary, did Nerve stop early, what did it
/// refuse* — are all about the current state of the object store rather than about history.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IngestRow {
    /// HEAD at ingest time. `None` on an unborn branch.
    pub head_oid: Option<String>,
    /// The tips the walk started from. A JSON array on disk, so adding ref enumeration later needs
    /// no migration.
    pub walked_from: Vec<String>,
    /// How many commits were recorded.
    pub commits_recorded: i64,
    /// The budget in force. With [`WalkTermination::CommitBudget`] this is where the walk stopped.
    pub commit_budget: i64,
    /// Why the walk stopped. [`WalkTermination::CommitBudget`] is Nerve's own boundary and must
    /// never be read as the repository being unable to go further.
    pub walk_terminated_by: WalkTermination,
    /// Whether the repository declares a shallow boundary.
    pub shallow: bool,
    /// The declared boundary oids. Empty when not shallow.
    pub shallow_boundary: Vec<String>,
    /// Whether the repository is a promisor (partial) clone.
    pub promisor: bool,
    /// Refusals by closed-vocabulary form, with counts. Empty when nothing was refused — and
    /// empty is a claim, so it is stored as `{}` rather than as a `NULL`.
    pub refusals: BTreeMap<String, usize>,
    /// Version of the object reader that produced this ingest.
    pub reader_version: String,
}

/// Whole-repository history tallies.
///
/// [`HistoryTotals::changes_by_kind`] holds **every** [`ChangeKind`], including the ones with a
/// count of zero. An absent key and a zero are the same fact here and a caller should not have to
/// know which it got; carrying all of them also means a kind added to [`ChangeKind::ALL`] appears
/// without every consumer being edited.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HistoryTotals {
    /// Commits recorded for this repository.
    pub commits: i64,
    /// Change rows recorded for this repository.
    pub changes: i64,
    /// Rename hypotheses recorded for this repository.
    pub renames: i64,
    /// Commits with more than one parent. Their changes are not enumerated by decision, so
    /// `merges > 0` with fewer changes than commits is expected rather than suspicious.
    pub merges: i64,
    /// Change rows per kind, with a zero for every kind that has none.
    pub changes_by_kind: BTreeMap<ChangeKind, i64>,
}

/// Every `git_commit` column a [`CommitRow`] needs, in the order [`read_commit`] expects.
///
/// Table-qualified, and every query below aliases `git_commit` as `c` to match. `commit_oid` is a
/// column of both `git_commit` and `git_change`, so an unqualified list is an *ambiguous column
/// name* error the moment the two are joined — and it is better for the two reads to share one
/// list and one alias than for a join to carry its own copy that can drift out of step with
/// [`read_commit`]'s indices.
const COMMIT_COLUMNS: &str = "c.commit_oid, c.tree_oid, c.parent_oids, c.parent_completeness, \
     c.changes_enumerated, c.author_time, c.author_tz, c.committer_time, c.committer_tz, \
     c.author_ident, c.committer_ident, c.summary, c.summary_truncation, c.is_merge";

/// How many columns [`COMMIT_COLUMNS`] names, so a join can offset past them.
const COMMIT_COLUMN_COUNT: usize = 14;

/// Every `git_rename_hypothesis` column a [`RenameRow`] needs, in the order [`read_rename`]
/// expects. Qualified with the `r` alias every query below gives `git_rename_hypothesis`.
const RENAME_COLUMNS: &str = "r.commit_oid, r.from_path, r.to_path, r.evidence, r.from_blob_oid, \
     r.to_blob_oid, r.matcher_id, r.matcher_version, r.match_numerator, r.match_denominator, \
     r.ambiguity";

/// Every `git_change` column a [`ChangeRow`] needs, in the order [`read_change`] expects.
/// Qualified with the `ch` alias every query below gives `git_change`.
const CHANGE_COLUMNS: &str = "ch.commit_oid, ch.path, ch.change_kind, ch.blob_oid, \
     ch.prev_blob_oid, ch.mode, ch.prev_mode";

fn encode_json<T: serde::Serialize>(column: &'static str, value: &T) -> Result<String> {
    serde_json::to_string(value).map_err(|source| StoreError::Json { column, source })
}

fn decode_json<T: serde::de::DeserializeOwned>(column: &'static str, raw: &str) -> Result<T> {
    serde_json::from_str(raw).map_err(|source| StoreError::Json { column, source })
}

/// A `git_commit` row as SQLite hands it over: text where a vocabulary belongs.
///
/// The two halves are separate because a `query_map` closure must return a `rusqlite::Result`,
/// which cannot carry a vocabulary refusal. Reading and parsing in one step would force an unknown
/// stored value to be mapped into a SQLite error and lose which vocabulary rejected it.
struct RawCommit {
    commit_oid: String,
    tree_oid: String,
    parent_oids: String,
    parent_completeness: String,
    changes_enumerated: String,
    author_time: i64,
    author_tz: String,
    committer_time: i64,
    committer_tz: String,
    author_ident: Option<String>,
    committer_ident: Option<String>,
    summary: String,
    summary_truncation: String,
    is_merge: bool,
}

fn read_commit(row: &Row<'_>, base: usize) -> rusqlite::Result<RawCommit> {
    Ok(RawCommit {
        commit_oid: row.get(base)?,
        tree_oid: row.get(base + 1)?,
        parent_oids: row.get(base + 2)?,
        parent_completeness: row.get(base + 3)?,
        changes_enumerated: row.get(base + 4)?,
        author_time: row.get(base + 5)?,
        author_tz: row.get(base + 6)?,
        committer_time: row.get(base + 7)?,
        committer_tz: row.get(base + 8)?,
        author_ident: row.get(base + 9)?,
        committer_ident: row.get(base + 10)?,
        summary: row.get(base + 11)?,
        summary_truncation: row.get(base + 12)?,
        is_merge: row.get(base + 13)?,
    })
}

fn commit_from_raw(raw: RawCommit) -> Result<CommitRow> {
    Ok(CommitRow {
        commit_oid: raw.commit_oid,
        tree_oid: raw.tree_oid,
        parent_oids: decode_json("git_commit.parent_oids", &raw.parent_oids)?,
        parent_completeness: raw.parent_completeness.parse()?,
        changes_enumerated: raw.changes_enumerated.parse()?,
        author_time: raw.author_time,
        author_tz: raw.author_tz,
        committer_time: raw.committer_time,
        committer_tz: raw.committer_tz,
        author_ident: raw.author_ident,
        committer_ident: raw.committer_ident,
        summary: raw.summary,
        summary_truncation: raw.summary_truncation.parse()?,
        is_merge: raw.is_merge,
    })
}

/// A `git_change` row as SQLite hands it over. Same split, same reason, as [`RawCommit`].
struct RawChange {
    commit_oid: String,
    path: String,
    change_kind: String,
    blob_oid: Option<String>,
    prev_blob_oid: Option<String>,
    mode: Option<i64>,
    prev_mode: Option<i64>,
}

fn read_change(row: &Row<'_>, base: usize) -> rusqlite::Result<RawChange> {
    Ok(RawChange {
        commit_oid: row.get(base)?,
        path: row.get(base + 1)?,
        change_kind: row.get(base + 2)?,
        blob_oid: row.get(base + 3)?,
        prev_blob_oid: row.get(base + 4)?,
        mode: row.get(base + 5)?,
        prev_mode: row.get(base + 6)?,
    })
}

fn change_from_raw(raw: RawChange) -> Result<ChangeRow> {
    Ok(ChangeRow {
        commit_oid: raw.commit_oid,
        path: raw.path,
        change_kind: raw.change_kind.parse()?,
        blob_oid: raw.blob_oid,
        prev_blob_oid: raw.prev_blob_oid,
        mode: raw.mode,
        prev_mode: raw.prev_mode,
    })
}

/// A `git_rename_hypothesis` row as SQLite hands it over.
struct RawRename {
    commit_oid: String,
    from_path: String,
    to_path: String,
    evidence: String,
    from_blob_oid: String,
    to_blob_oid: String,
    matcher_id: String,
    matcher_version: String,
    match_numerator: Option<i64>,
    match_denominator: Option<i64>,
    ambiguity: String,
}

fn read_rename(row: &Row<'_>, base: usize) -> rusqlite::Result<RawRename> {
    Ok(RawRename {
        commit_oid: row.get(base)?,
        from_path: row.get(base + 1)?,
        to_path: row.get(base + 2)?,
        evidence: row.get(base + 3)?,
        from_blob_oid: row.get(base + 4)?,
        to_blob_oid: row.get(base + 5)?,
        matcher_id: row.get(base + 6)?,
        matcher_version: row.get(base + 7)?,
        match_numerator: row.get(base + 8)?,
        match_denominator: row.get(base + 9)?,
        ambiguity: row.get(base + 10)?,
    })
}

fn rename_from_raw(raw: RawRename) -> Result<RenameRow> {
    Ok(RenameRow {
        commit_oid: raw.commit_oid,
        from_path: raw.from_path,
        to_path: raw.to_path,
        evidence: raw.evidence.parse()?,
        from_blob_oid: raw.from_blob_oid,
        to_blob_oid: raw.to_blob_oid,
        matcher_id: raw.matcher_id,
        matcher_version: raw.matcher_version,
        match_numerator: raw.match_numerator,
        match_denominator: raw.match_denominator,
        ambiguity: raw.ambiguity.parse()?,
    })
}

/// A `git_rename_analysis` row as SQLite hands it over. Same split, same reason, as [`RawCommit`].
struct RawAnalysis {
    commit_oid: String,
    matcher_id: String,
    matcher_version: String,
    threshold_numerator: i64,
    threshold_denominator: i64,
    deletions_considered: i64,
    additions_considered: i64,
    pairs_considered: i64,
    pairs_measured: i64,
    completeness: String,
    unmeasured: String,
}

fn analysis_from_raw(raw: RawAnalysis) -> Result<AnalysisRow> {
    // Decoded key by key rather than straight into a `BTreeMap<SimilarityUnmeasured, i64>`, because
    // an unrecognised key must be *refused* by the vocabulary that owns it. A serde map key error
    // would say only that a string did not deserialise, losing which vocabulary rejected it —
    // the same reason `RawCommit` splits reading from parsing.
    let raw_counts: BTreeMap<String, i64> =
        decode_json("git_rename_analysis.unmeasured", &raw.unmeasured)?;
    let mut unmeasured = BTreeMap::new();
    for (reason, count) in raw_counts {
        unmeasured.insert(reason.parse::<SimilarityUnmeasured>()?, count);
    }
    Ok(AnalysisRow {
        commit_oid: raw.commit_oid,
        matcher_id: raw.matcher_id,
        matcher_version: raw.matcher_version,
        threshold_numerator: raw.threshold_numerator,
        threshold_denominator: raw.threshold_denominator,
        deletions_considered: raw.deletions_considered,
        additions_considered: raw.additions_considered,
        pairs_considered: raw.pairs_considered,
        pairs_measured: raw.pairs_measured,
        completeness: raw.completeness.parse()?,
        unmeasured,
    })
}

/// A `usize` bound as SQLite can take it.
///
/// `LIMIT` is signed and a negative value means *no limit* in SQLite, so a `usize as i64` cast
/// that wrapped would quietly turn a bound into its absence. Saturating is the safe direction:
/// a caller asking for more rows than `i64::MAX` gets all of them, which is what it asked for.
fn as_bound(value: usize) -> i64 {
    i64::try_from(value).unwrap_or(i64::MAX)
}

/// Record one commit. `Ok(true)` if a row was written, `Ok(false)` if it was already recorded.
///
/// **This is the only writer here that tolerates a duplicate, and the licence is narrow.** A
/// commit oid is the hash of the commit object, so a second insert of the same oid is the same
/// commit and ignoring it is exactly what incremental re-ingest needs. The return value is the
/// statement's `conn.changes()`, which is how the caller learns which happened rather than having
/// to query first.
///
/// **A caller must write a commit and its changes in one transaction.** Nothing here can enforce
/// that, and the consequence of not doing it is precisely the ambiguity
/// [`CommitRow::changes_enumerated`] exists to remove: a crash between [`insert_commit`] and
/// [`insert_changes`] leaves a commit claiming [`ChangesEnumerated::Enumerated`] with no change
/// rows, and the next ingest will skip it because this function now returns `false` for it.
///
/// **`parent_completeness` and `changes_enumerated` are not properties of the commit object.**
/// They describe what *this* repository could see when it was read, and that can improve — a
/// `git fetch --unshallow` turns a [`ParentCompleteness::ShallowBoundary`] commit into an ordinary
/// one. Because this function ignores the second insert, such a commit keeps its old availability
/// values until something deletes and re-records it. That is a real limit of the write path, and it
/// is stated here rather than left to be discovered.
///
/// [`CommitRow::summary_truncation`] inherits that limitation and is stable in practice, because
/// the bound it is computed against is a compile-time constant.
pub fn insert_commit(conn: &Connection, repo_id: &str, row: &CommitRow) -> Result<bool> {
    let parent_oids = encode_json("git_commit.parent_oids", &row.parent_oids)?;
    let written = conn.execute(
        "INSERT OR IGNORE INTO git_commit
             (repo_id, commit_oid, tree_oid, parent_oids, parent_completeness,
              changes_enumerated, author_time, author_tz, committer_time, committer_tz,
              author_ident, committer_ident, summary, summary_truncation, is_merge)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15)",
        params![
            repo_id,
            row.commit_oid,
            row.tree_oid,
            parent_oids,
            row.parent_completeness.as_str(),
            row.changes_enumerated.as_str(),
            row.author_time,
            row.author_tz,
            row.committer_time,
            row.committer_tz,
            row.author_ident,
            row.committer_ident,
            row.summary,
            row.summary_truncation.as_str(),
            row.is_merge,
        ],
    )?;
    Ok(written > 0)
}

/// Record a commit's changes. Returns the number of rows written.
///
/// **Plain `INSERT`, never `INSERT OR IGNORE`.** Slice 3b shipped a silent data-destruction bug in
/// which `INSERT OR IGNORE` swallowed `NOT NULL` violations, the graph shrank, and the process
/// exited zero — documented at `crates/nerve-index/src/pipeline.rs:654-666`. A dropped change row
/// here would be indistinguishable from a commit that did not touch the path, which is the failure
/// this whole table is shaped to prevent, so a constraint or foreign-key violation is an error and
/// the transaction the caller is holding rolls back.
///
/// **Contract: call this only for a commit for which [`insert_commit`] returned `true`.** The
/// foreign key on `(repo_id, commit_oid)` is enforced — `PRAGMA foreign_keys=ON` is set in
/// `db.rs` — so a change for an unrecorded commit is refused rather than orphaned, and the primary
/// key on `(repo_id, commit_oid, path)` means re-supplying a recorded commit's changes is a
/// conflict rather than a duplicate.
pub fn insert_changes(conn: &Connection, repo_id: &str, rows: &[ChangeRow]) -> Result<usize> {
    let mut stmt = conn.prepare(
        "INSERT INTO git_change
             (repo_id, commit_oid, path, change_kind, blob_oid, prev_blob_oid, mode, prev_mode)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
    )?;
    let mut written = 0;
    for row in rows {
        written += stmt.execute(params![
            repo_id,
            row.commit_oid,
            row.path,
            row.change_kind.as_str(),
            row.blob_oid,
            row.prev_blob_oid,
            row.mode,
            row.prev_mode,
        ])?;
    }
    Ok(written)
}

/// Record a commit's rename hypotheses. Returns the number of rows written.
///
/// **Plain `INSERT`, never `INSERT OR IGNORE`**, for the same measured reason as
/// [`insert_changes`]: a silently dropped hypothesis looks exactly like a repository with no
/// renames, which is how a broken guard hides behind a green test.
///
/// **Contract: call this only for a commit for which [`insert_commit`] returned `true`.** The
/// foreign key on `(repo_id, commit_oid)` is enforced.
///
/// Every pairing an ambiguous match produces is a row of its own, carrying
/// [`RenameAmbiguity::ManyTo`] or its siblings. None is promoted and none is scored, so passing
/// several rows for one blob is the normal case rather than a caller error.
///
/// The schema's `CHECK` decides whether the evidence and the measurement agree — an exact-content
/// row with a measurement, or a similar-content row without one, is refused here rather than
/// reviewed later. The plain `INSERT` is what makes that refusal visible.
pub fn insert_renames(conn: &Connection, repo_id: &str, rows: &[RenameRow]) -> Result<usize> {
    let mut stmt = conn.prepare(
        "INSERT INTO git_rename_hypothesis
             (repo_id, commit_oid, from_path, to_path, evidence, from_blob_oid, to_blob_oid,
              matcher_id, matcher_version, match_numerator, match_denominator, ambiguity)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
    )?;
    let mut written = 0;
    for row in rows {
        written += stmt.execute(params![
            repo_id,
            row.commit_oid,
            row.from_path,
            row.to_path,
            row.evidence.as_str(),
            row.from_blob_oid,
            row.to_blob_oid,
            row.matcher_id,
            row.matcher_version,
            row.match_numerator,
            row.match_denominator,
            row.ambiguity.as_str(),
        ])?;
    }
    Ok(written)
}

/// Record what one matcher's candidate set for one commit was. Returns the number of rows written.
///
/// **Plain `INSERT`, never `INSERT OR IGNORE`**, for the same measured reason as
/// [`insert_changes`] and [`insert_renames`]: Slice 3b lost graph rows to an `INSERT OR IGNORE`
/// that swallowed `NOT NULL` violations and exited zero
/// (`crates/nerve-index/src/pipeline.rs:654-666`). A silently dropped analysis row is worse here
/// than anywhere else in this module, because its absence is exactly what a
/// [`RenameAnalysisCompleteness::RefusedBound`] commit looks like from the hypothesis table — a
/// commit with no similarity rows. Losing the row would turn *"a bound refused this"* into
/// *"nothing was renamed"*, which is the one confusion this table exists to remove.
///
/// **Contract: call this only for a commit that is already recorded.** The foreign key on
/// `(repo_id, commit_oid)` is enforced, and the primary key on `(repo_id, commit_oid, matcher_id)`
/// means re-supplying one matcher's analysis of a recorded commit is a conflict rather than a
/// duplicate.
pub fn insert_rename_analysis(
    conn: &Connection,
    repo_id: &str,
    row: &AnalysisRow,
) -> Result<usize> {
    // Keyed by the vocabulary's canonical name. A JSON object's keys are strings, so the map is
    // restated over `&'static str` on the way out rather than deriving `Serialize` for a
    // vocabulary that has exactly one wire form already.
    let unmeasured: BTreeMap<&'static str, i64> = row
        .unmeasured
        .iter()
        .map(|(reason, count)| (reason.as_str(), *count))
        .collect();
    let unmeasured = encode_json("git_rename_analysis.unmeasured", &unmeasured)?;
    let written = conn.execute(
        "INSERT INTO git_rename_analysis
             (repo_id, commit_oid, matcher_id, matcher_version, threshold_numerator,
              threshold_denominator, deletions_considered, additions_considered,
              pairs_considered, pairs_measured, completeness, unmeasured)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
        params![
            repo_id,
            row.commit_oid,
            row.matcher_id,
            row.matcher_version,
            row.threshold_numerator,
            row.threshold_denominator,
            row.deletions_considered,
            row.additions_considered,
            row.pairs_considered,
            row.pairs_measured,
            row.completeness.as_str(),
            unmeasured,
        ],
    )?;
    Ok(written)
}

/// Record what this ingest read, replacing any previous record for the repository.
///
/// One row per repository on purpose. Every field describes the current state of the object store
/// — shallow or not, where the boundary is, whether Nerve stopped early, what it refused — so
/// keeping a history of ingests would be keeping stale answers to questions that only have a
/// current one. `ingested_at` is stamped here rather than supplied, in the manner of every other
/// `created_at` in this schema.
pub fn upsert_history_ingest(conn: &Connection, repo_id: &str, row: &IngestRow) -> Result<()> {
    let walked_from = encode_json("git_history_ingest.walked_from", &row.walked_from)?;
    let shallow_boundary =
        encode_json("git_history_ingest.shallow_boundary", &row.shallow_boundary)?;
    let refusals = encode_json("git_history_ingest.refusals", &row.refusals)?;
    conn.execute(
        "INSERT INTO git_history_ingest
             (repo_id, head_oid, walked_from, commits_recorded, commit_budget,
              walk_terminated_by, shallow, shallow_boundary, promisor, refusals,
              reader_version, ingested_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11,
                 strftime('%Y-%m-%dT%H:%M:%fZ','now'))
         ON CONFLICT(repo_id) DO UPDATE SET
             head_oid           = excluded.head_oid,
             walked_from        = excluded.walked_from,
             commits_recorded   = excluded.commits_recorded,
             commit_budget      = excluded.commit_budget,
             walk_terminated_by = excluded.walk_terminated_by,
             shallow            = excluded.shallow,
             shallow_boundary   = excluded.shallow_boundary,
             promisor           = excluded.promisor,
             refusals           = excluded.refusals,
             reader_version     = excluded.reader_version,
             ingested_at        = excluded.ingested_at",
        params![
            repo_id,
            row.head_oid,
            walked_from,
            row.commits_recorded,
            row.commit_budget,
            row.walk_terminated_by.as_str(),
            row.shallow,
            shallow_boundary,
            row.promisor,
            refusals,
            row.reader_version,
        ],
    )?;
    Ok(())
}

/// Every commit oid already recorded for a repository.
///
/// A set rather than a list, because its one use is the walk's *stop* condition: re-ingest halts as
/// soon as it reaches a commit already present, and the whole point of the composite primary key
/// is that this question is cheap. Ordered by construction — [`BTreeSet`] has no other order.
pub fn recorded_commit_oids(conn: &Connection, repo_id: &str) -> Result<BTreeSet<String>> {
    let mut stmt =
        conn.prepare("SELECT commit_oid FROM git_commit WHERE repo_id = ?1 ORDER BY commit_oid")?;
    let rows = stmt.query_map(params![repo_id], |row| row.get::<_, String>(0))?;
    let mut out = BTreeSet::new();
    for row in rows {
        out.insert(row?);
    }
    Ok(out)
}

/// What the last ingest recorded, or `None` if history has never been ingested.
///
/// `None` here means *no ingest has run*, which is a different fact from an ingest that found
/// nothing: the latter is a row with `commits_recorded = 0` and a
/// [`WalkTermination`] saying why. A caller that treated them alike would report an
/// un-ingested repository as one with no history.
pub fn history_ingest(conn: &Connection, repo_id: &str) -> Result<Option<IngestRow>> {
    let mut stmt = conn.prepare(
        "SELECT head_oid, walked_from, commits_recorded, commit_budget, walk_terminated_by,
                shallow, shallow_boundary, promisor, refusals, reader_version
           FROM git_history_ingest WHERE repo_id = ?1",
    )?;
    let mut rows = stmt.query(params![repo_id])?;
    let Some(row) = rows.next()? else {
        return Ok(None);
    };
    let head_oid: Option<String> = row.get(0)?;
    let walked_from: String = row.get(1)?;
    let commits_recorded: i64 = row.get(2)?;
    let commit_budget: i64 = row.get(3)?;
    let walk_terminated_by: String = row.get(4)?;
    let shallow: bool = row.get(5)?;
    let shallow_boundary: String = row.get(6)?;
    let promisor: bool = row.get(7)?;
    let refusals: String = row.get(8)?;
    let reader_version: String = row.get(9)?;
    Ok(Some(IngestRow {
        head_oid,
        walked_from: decode_json("git_history_ingest.walked_from", &walked_from)?,
        commits_recorded,
        commit_budget,
        walk_terminated_by: walk_terminated_by.parse()?,
        shallow,
        shallow_boundary: decode_json("git_history_ingest.shallow_boundary", &shallow_boundary)?,
        promisor,
        refusals: decode_json("git_history_ingest.refusals", &refusals)?,
        reader_version,
    }))
}

/// Recorded commits, newest committer time first.
///
/// **`commit_oid` is not decoration in the `ORDER BY`.** Fixtures commit with fixed synthetic
/// dates, so equal `committer_time` values are guaranteed, and `ORDER BY committer_time DESC`
/// alone leaves SQLite free to return tied rows in any order it likes — including a different one
/// after an unrelated insert changes the query plan. The oid tiebreak makes the order total, which
/// is what lets a fixture test assert a sequence instead of a set.
pub fn commit_log(
    conn: &Connection,
    repo_id: &str,
    limit: usize,
    offset: usize,
) -> Result<Vec<CommitRow>> {
    let mut stmt = conn.prepare(&format!(
        "SELECT {COMMIT_COLUMNS}
           FROM git_commit c WHERE c.repo_id = ?1
          ORDER BY c.committer_time DESC, c.commit_oid ASC
          LIMIT ?2 OFFSET ?3"
    ))?;
    let rows = stmt.query_map(params![repo_id, as_bound(limit), as_bound(offset)], |row| {
        read_commit(row, 0)
    })?;
    let mut out = Vec::new();
    for row in rows {
        out.push(commit_from_raw(row?)?);
    }
    Ok(out)
}

/// Every change recorded for one commit, ordered by path.
///
/// **An empty vector is not "this commit changed nothing".** It is one of four facts, and which one
/// is on the commit: read [`CommitRow::changes_enumerated`] before drawing any conclusion from a
/// zero length here. A merge has zero rows by decision and a shallow boundary has zero because its
/// parent tree could not be read.
///
/// `path` is unique per commit by primary key, so ordering by it is total.
pub fn changes_for_commit(
    conn: &Connection,
    repo_id: &str,
    commit_oid: &str,
) -> Result<Vec<ChangeRow>> {
    let mut stmt = conn.prepare(&format!(
        "SELECT {CHANGE_COLUMNS}
           FROM git_change ch WHERE ch.repo_id = ?1 AND ch.commit_oid = ?2
          ORDER BY ch.path ASC"
    ))?;
    let rows = stmt.query_map(params![repo_id, commit_oid], |row| read_change(row, 0))?;
    let mut out = Vec::new();
    for row in rows {
        out.push(change_from_raw(row?)?);
    }
    Ok(out)
}

/// Commits that touched one path, newest first, each paired with what it did to that path.
///
/// The commit is returned alongside the change rather than after a second query, because the change
/// on its own cannot be dated and the commit on its own cannot say what happened. Ordered
/// `committer_time DESC, commit_oid ASC`, which is total: a commit records at most one change per
/// path.
pub fn commits_touching_path(
    conn: &Connection,
    repo_id: &str,
    path: &str,
    limit: usize,
) -> Result<Vec<(CommitRow, ChangeRow)>> {
    let mut stmt = conn.prepare(&format!(
        "SELECT {COMMIT_COLUMNS}, {CHANGE_COLUMNS}
           FROM git_change ch
           JOIN git_commit c ON c.repo_id = ch.repo_id AND c.commit_oid = ch.commit_oid
          WHERE ch.repo_id = ?1 AND ch.path = ?2
          ORDER BY c.committer_time DESC, c.commit_oid ASC
          LIMIT ?3"
    ))?;
    let rows = stmt.query_map(params![repo_id, path, as_bound(limit)], |row| {
        Ok((read_commit(row, 0)?, read_change(row, COMMIT_COLUMN_COUNT)?))
    })?;
    let mut out = Vec::new();
    for row in rows {
        let (commit, change) = row?;
        out.push((commit_from_raw(commit)?, change_from_raw(change)?));
    }
    Ok(out)
}

/// Rename hypotheses naming one path on **either** side, newest commit first.
///
/// Both sides are searched because the question "what happened to this path" is asked of a path
/// that vanished as often as of one that appeared, and answering only for `to_path` would silently
/// drop exactly the deleted paths §7 exists to trace.
///
/// Ordered `committer_time DESC` then by the hypothesis's own primary key, which is total. The
/// order needs the join even though no commit field is returned: without it, several hypotheses
/// with the same `commit_oid` — the ambiguous case, which is the interesting one — would come back
/// in whatever order the query plan produced.
pub fn renames_touching_path(
    conn: &Connection,
    repo_id: &str,
    path: &str,
    limit: usize,
) -> Result<Vec<RenameRow>> {
    let mut stmt = conn.prepare(&format!(
        "SELECT {RENAME_COLUMNS}
           FROM git_rename_hypothesis r
           JOIN git_commit c ON c.repo_id = r.repo_id AND c.commit_oid = r.commit_oid
          WHERE r.repo_id = ?1 AND (r.from_path = ?2 OR r.to_path = ?2)
          ORDER BY c.committer_time DESC, r.commit_oid ASC, r.from_path ASC, r.to_path ASC
          LIMIT ?3"
    ))?;
    let rows = stmt.query_map(params![repo_id, path, as_bound(limit)], |row| {
        read_rename(row, 0)
    })?;
    let mut out = Vec::new();
    for row in rows {
        out.push(rename_from_raw(row?)?);
    }
    Ok(out)
}

/// One matcher's analysis of each of the given commits, keyed by commit oid.
///
/// **A missing key is the answer to a different question from a
/// [`RenameAnalysisCompleteness::RefusedBound`] row**, and that is the whole reason this read
/// exists as a join rather than as an inference. A commit absent from the returned map was never
/// analysed by this matcher; a commit present with `refused_bound` was analysed and refused. Both
/// have zero similarity hypotheses, and only the stored row tells them apart.
///
/// `matcher_id` is a parameter rather than implicit, because the primary key admits several
/// matchers per commit and merging their analyses would produce a completeness that describes no
/// run that ever happened.
///
/// A `BTreeMap` rather than a `Vec`, because the one use is joining completeness onto hypotheses a
/// surface already holds, and that is a lookup. Ordered by construction, which keeps a caller that
/// iterates it deterministic.
pub fn rename_analysis_for_commits(
    conn: &Connection,
    repo_id: &str,
    commit_oids: &[&str],
    matcher_id: &str,
) -> Result<BTreeMap<String, AnalysisRow>> {
    let mut out = BTreeMap::new();
    if commit_oids.is_empty() {
        return Ok(out);
    }
    // One prepared statement executed per oid rather than an `IN (…)` list built by string
    // concatenation: the oids come from stored rows, but building SQL out of values is how a
    // query stops being parameterised, and the statement is prepared once either way.
    let mut stmt = conn.prepare(
        "SELECT commit_oid, matcher_id, matcher_version, threshold_numerator,
                threshold_denominator, deletions_considered, additions_considered,
                pairs_considered, pairs_measured, completeness, unmeasured
           FROM git_rename_analysis
          WHERE repo_id = ?1 AND commit_oid = ?2 AND matcher_id = ?3",
    )?;
    for commit_oid in commit_oids {
        let mut rows = stmt.query(params![repo_id, commit_oid, matcher_id])?;
        let Some(row) = rows.next()? else {
            continue;
        };
        let raw = RawAnalysis {
            commit_oid: row.get(0)?,
            matcher_id: row.get(1)?,
            matcher_version: row.get(2)?,
            threshold_numerator: row.get(3)?,
            threshold_denominator: row.get(4)?,
            deletions_considered: row.get(5)?,
            additions_considered: row.get(6)?,
            pairs_considered: row.get(7)?,
            pairs_measured: row.get(8)?,
            completeness: row.get(9)?,
            unmeasured: row.get(10)?,
        };
        let analysis = analysis_from_raw(raw)?;
        out.insert(analysis.commit_oid.clone(), analysis);
    }
    Ok(out)
}

/// Whole-repository history tallies.
///
/// `changes` being lower than `commits` is normal rather than a sign of loss: merges contribute
/// none by decision and a shallow boundary contributes none because its parent was unreadable.
/// That is why [`HistoryTotals::merges`] is counted here — the two numbers are only readable
/// together.
pub fn history_totals(conn: &Connection, repo_id: &str) -> Result<HistoryTotals> {
    let commits: i64 = conn.query_row(
        "SELECT count(*) FROM git_commit WHERE repo_id = ?1",
        params![repo_id],
        |row| row.get(0),
    )?;
    let merges: i64 = conn.query_row(
        "SELECT count(*) FROM git_commit WHERE repo_id = ?1 AND is_merge = 1",
        params![repo_id],
        |row| row.get(0),
    )?;
    let changes: i64 = conn.query_row(
        "SELECT count(*) FROM git_change WHERE repo_id = ?1",
        params![repo_id],
        |row| row.get(0),
    )?;
    let renames: i64 = conn.query_row(
        "SELECT count(*) FROM git_rename_hypothesis WHERE repo_id = ?1",
        params![repo_id],
        |row| row.get(0),
    )?;

    // Every kind, including the ones at zero: a caller must not have to tell an absent key from a
    // zero, and an unknown stored value is refused here rather than silently bucketed.
    let mut changes_by_kind: BTreeMap<ChangeKind, i64> =
        ChangeKind::ALL.into_iter().map(|kind| (kind, 0)).collect();
    let mut stmt = conn.prepare(
        "SELECT change_kind, count(*) FROM git_change
          WHERE repo_id = ?1 GROUP BY change_kind ORDER BY change_kind",
    )?;
    let rows = stmt.query_map(params![repo_id], |row| {
        Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?))
    })?;
    for row in rows {
        let (kind, count) = row?;
        changes_by_kind.insert(kind.parse()?, count);
    }

    Ok(HistoryTotals {
        commits,
        changes,
        renames,
        merges,
        changes_by_kind,
    })
}

// ---- Slice 12c-i: the derived questions --------------------------------------------------------
//
// Everything below is a `SELECT` or a walk over the four tables above. No table, column, index or
// `SCHEMA_VERSION` bump: §7's grouping is served by `idx_git_change_path`, §8's self-join by the
// `(repo_id, commit_oid, path)` primary key, and §5's ancestry walk by `(repo_id, commit_oid)`.
//
// **Nothing here touches the filesystem.** A historical path is matched as the bytes a tree
// recorded, literally, and `nerve-store` has no path guard because it opens no path. That is the
// structural answer to the trap recorded three times on this project — routing a historical path
// through `discover::canonical_child` refuses every deleted path, because it ends in
// `std::fs::canonicalize` and so requires existence, and it counts each refusal as path-safety
// coverage. The guard that *is* needed lives at ingest time, in `nerve-index`, where the bytes
// arrive.

/// Whether this ingest could not see the whole reachable history, so an earlier change may exist
/// and not be recorded.
///
/// **One derived boolean, in one place.** Slice 12b had this inside the CLI binary, where three
/// further surfaces were each about to re-derive it — and it is not wording but a *judgment*, the
/// single question every history surface has to agree on. It lives here rather than in `nerve-core`
/// because it takes an [`IngestRow`] and `nerve-core` does not depend on this crate.
///
/// `shallow` is included even when the walk ended [`WalkTermination::Exhausted`]: the walk exhausted
/// what it could *see*.
pub fn earlier_changes_may_exist(ingest: &IngestRow) -> bool {
    ingest.shallow || ingest.walk_terminated_by != WalkTermination::Exhausted
}

/// What the current tree was read from, reported as a field rather than assumed.
///
/// There is exactly one basis and it is the `entity` table. A `stat` under the repository root would
/// need its own path guard for a path that may not exist, which is the one thing
/// `discover::canonical_child` cannot do; and a repository can have history without ever having been
/// indexed, so the basis has to be reported rather than presumed available.
pub const CURRENT_TREE_BASIS: &str = "entity_table";

/// What the `entity` table says about one path, and whether it was in a position to say anything.
///
/// [`CurrentTree::index_exists`] is established from the database — `entity` rows for this
/// repository — and never guessed. An indexed repository always has at least the repository entity,
/// so zero rows means no index has run here, which is
/// [`FirstObservedKind::CurrentTreeUnknown`] rather than an absent path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CurrentTree {
    /// Always [`CURRENT_TREE_BASIS`]. Carried so a consumer knows which basis it got.
    pub basis: &'static str,
    /// Whether any `entity` row exists for this repository at all.
    pub index_exists: bool,
    /// How many `entity` rows name this path, by [`nerve_core::vocab::EntityKind::path_role`].
    pub entities_at_path: i64,
}

/// Why visible history stops above the earliest recorded change to a path.
///
/// The four reasons of `docs/plans/slice-12c-historical-questions.md` §4.1. It is **not** a new
/// closed vocabulary and deliberately does not live in `nerve-core`: three of its values are
/// [`ParentCompleteness`] values and the fourth is a [`WalkTermination`] value, so it is a
/// composition of two existing vocabularies rather than a fifth axis. Adding it to `nerve-core`
/// would create a second place where "a shallow boundary hides history" is defined.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum EarlierHistoryUnavailable {
    /// A declared shallow boundary. Expected, and not a fault.
    ShallowBoundary,
    /// A parent object is absent and was not declared absent. A fault, never called shallow.
    ParentsMissing,
    /// A parent is absent and Nerve could not establish whether the absence was declared.
    ParentsUnverifiable,
    /// Nerve's own commit budget. The history did not stop; the read did.
    CommitBudget,
    /// A bound refused an object the walk needed. Nerve's own doing, like
    /// [`EarlierHistoryUnavailable::CommitBudget`], and never a property of the repository.
    ///
    /// This value exists because omitting it produced a response that contradicted itself.
    /// [`WalkTermination::Refused`] was mapped to "no reason applies", while
    /// [`earlier_changes_may_exist`] — which is `walk_terminated_by != Exhausted` — said earlier
    /// changes *may* exist. Two derivations of one question, disagreeing: the duplication this slice
    /// exists to remove, inside the slice itself.
    WalkRefused,
}

impl EarlierHistoryUnavailable {
    /// Every value, in declaration order.
    pub const ALL: [EarlierHistoryUnavailable; 5] = [
        EarlierHistoryUnavailable::ShallowBoundary,
        EarlierHistoryUnavailable::ParentsMissing,
        EarlierHistoryUnavailable::ParentsUnverifiable,
        EarlierHistoryUnavailable::CommitBudget,
        EarlierHistoryUnavailable::WalkRefused,
    ];

    /// Canonical lower-case name, carried in responses.
    ///
    /// The names match the [`ParentCompleteness`] and [`WalkTermination`] members they are drawn
    /// from, so a consumer that already renders those needs no second table.
    pub fn as_str(self) -> &'static str {
        match self {
            EarlierHistoryUnavailable::ShallowBoundary => "shallow_boundary",
            EarlierHistoryUnavailable::ParentsMissing => "parents_missing",
            EarlierHistoryUnavailable::ParentsUnverifiable => "parents_unverifiable",
            EarlierHistoryUnavailable::CommitBudget => "commit_budget",
            EarlierHistoryUnavailable::WalkRefused => "walk_refused",
        }
    }

    /// Why visible history stops above a path's earliest recorded change, in words.
    ///
    /// This note lives here rather than in `nerve-core` for the reason the enum does: it composes
    /// [`ParentCompleteness`] and [`WalkTermination`] rather than being a third axis, and putting it
    /// in `nerve-core` would create a second place where "a shallow boundary hides history" is
    /// defined. `crates/nerve-cli/tests/history_wording.rs` enforces the single copy per note *by
    /// crate*, which is why the guard records an owner for each note instead of one global answer.
    ///
    /// Hoisted out of the CLI binary in Slice 12c-iii-a. The
    /// [`EarlierHistoryUnavailable::WalkRefused`] arm is deliberately **not** a restatement of
    /// [`WalkTermination::Refused`]'s note — a first draft of it reproduced that sentence verbatim
    /// and the wording guard failed it, which is the whole reason the guard exists.
    pub fn note(self) -> &'static str {
        match self {
            EarlierHistoryUnavailable::ShallowBoundary => {
                "a declared shallow boundary sits above what Nerve read of this path, so an earlier \
                 change to it may exist and not be recorded; expected, and not a fault"
            }
            EarlierHistoryUnavailable::ParentsMissing => {
                "a parent object is absent and was not declared absent — a fault in this \
                 repository, never called shallow"
            }
            EarlierHistoryUnavailable::ParentsUnverifiable => {
                "a parent object is absent and Nerve could not establish whether the absence was \
                 declared, so neither answer may be asserted"
            }
            EarlierHistoryUnavailable::CommitBudget => {
                "Nerve's own commit budget stopped the read; the history did not stop, the read did"
            }
            EarlierHistoryUnavailable::WalkRefused => {
                "Nerve declined an object the walk required and stopped there — Nerve's own doing, \
                 never a property of the repository"
            }
        }
    }
}

/// One change to one path, with the commit that made it.
///
/// Paired rather than returned separately for the reason [`commits_touching_path`] states: the change
/// cannot be dated on its own and the commit cannot say what happened on its own.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PathChange {
    /// The commit the change belongs to.
    pub commit: CommitRow,
    /// What that commit did to the path.
    pub change: ChangeRow,
}

/// When one path was first and last *observed* changing — and which of six answers that is.
///
/// The earliest [`ChangeRow`] for a path is not when the path was created. Read
/// [`FirstLastObserved::kind`] and [`FirstLastObserved::may_claim_created`] before rendering
/// anything; the permission has exactly one source,
/// [`FirstObservedKind::may_claim_created`], and it is carried here so no surface re-derives it.
///
/// **`first` and `last` are ordered by `committer_time`, which is not an ancestry order.** A rebase
/// or a fabricated date can reorder commits freely, so these are the earliest and latest *dated*
/// changes rather than the topologically first and last. The one claim that must not rest on a date
/// does not: [`FirstObservedKind::CreatedInVisibleHistory`] additionally requires the commit to have
/// no parents at all, which no clock can fake.
///
/// [`FirstLastObserved::last`] has the opposite trap to `first` and it is staleness rather than
/// availability: the latest visible change is the latest change only if the ingest's `head_oid` is
/// still the repository's current commit. See [`history_freshness`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FirstLastObserved {
    /// The path as asked for, matched against the bytes a tree recorded.
    pub path: String,
    /// Which of the six answers this is.
    pub kind: FirstObservedKind,
    /// [`FirstObservedKind::may_claim_created`] for [`FirstLastObserved::kind`], carried rather than
    /// left for a consumer to re-derive.
    pub may_claim_created: bool,
    /// The earliest dated change Nerve can see, or `None` when there are no change rows.
    pub first: Option<PathChange>,
    /// The latest dated change Nerve can see, or `None` when there are no change rows.
    pub last: Option<PathChange>,
    /// How many commits in visible history touched this path. Zero is qualified by
    /// [`FirstLastObserved::kind`], never read alone.
    pub changes_in_visible_history: i64,
    /// Which of the five reasons hides history above **this path's** earliest change, when one does.
    ///
    /// **`None` means nothing is hidden above this path, and it is a measured claim rather than a
    /// default.** It is reachable two ways: the earliest change sits at a parentless commit, where
    /// nothing precedes it by definition; or the walk terminated `Exhausted`, which `ingest_history`
    /// assigns only after a budget stop, a refusal, a missing object and a reached boundary have each
    /// had their chance (`crates/nerve-index/src/history.rs:419-433`).
    ///
    /// **This is a path-level answer and [`FirstLastObserved::earlier_changes_may_exist`] is a
    /// repository-level one. They are not the same question and must not be collapsed.** A shallow
    /// clone can contain a genuine root commit — one branch fetched whole, another truncated — so a
    /// path created at that root has `None` here while the repository still reports that earlier
    /// commits may exist. Where the earliest change sits at a commit with *available parents*, the two
    /// do agree, because then the path's answer rests entirely on whether the walk was complete; that
    /// narrower equivalence is asserted, and an earlier draft of this module violated it on
    /// [`WalkTermination::Refused`] by naming no reason beside a `true` boolean.
    pub earlier_history_unavailable: Option<EarlierHistoryUnavailable>,
    /// How many of this path's recorded changes are additions.
    ///
    /// Load-bearing for [`FirstObservedKind::CreatedInVisibleHistory`], and the reason that claim
    /// does not rest on a date. `first` and `last` are ordered by `committer_time`, which a rebase or
    /// a fabricated clock can reorder freely — so "the earliest *dated* change is an addition" does
    /// not establish that it is the topologically first change.
    ///
    /// Exactly **one** recorded addition does establish it, and without consulting any clock: a path
    /// created, deleted and re-created records two additions, so one addition in a history where
    /// nothing is hidden means the path was created once, at that commit.
    pub additions_recorded: i64,
    /// Merge commits recorded for this repository, which enumerate no changes by 12b's decision.
    ///
    /// Carried as **data rather than prose** because it is the one residual way
    /// [`FirstObservedKind::CreatedInVisibleHistory`] can still be wrong: a path created inside one
    /// merge and deleted inside another has both events unrecorded, so a later addition can look
    /// like a first one. Zero here removes that possibility entirely.
    pub merges_in_repository: i64,
    /// [`earlier_changes_may_exist`] for this repository's ingest. `false` when there is no ingest,
    /// where the answer is [`FirstObservedKind::NoHistoryIngested`] instead.
    pub earlier_changes_may_exist: bool,
    /// Why the ingest's walk stopped, or `None` when history has never been ingested.
    pub walk_terminated_by: Option<WalkTermination>,
    /// Whether the repository declares a shallow boundary.
    pub shallow: bool,
    /// What the entity table said, and whether it was in a position to say it.
    pub current_tree: CurrentTree,
}

/// Read one commit by oid. `None` when it is not recorded for this repository.
///
/// A primary-key lookup on `(repo_id, commit_oid)`, which is what makes the ancestry walk in
/// [`state_diff`] affordable without a second index.
pub fn commit_by_oid(
    conn: &Connection,
    repo_id: &str,
    commit_oid: &str,
) -> Result<Option<CommitRow>> {
    let mut stmt = conn.prepare(&format!(
        "SELECT {COMMIT_COLUMNS} FROM git_commit c
          WHERE c.repo_id = ?1 AND c.commit_oid = ?2"
    ))?;
    let mut rows = stmt.query_map(params![repo_id, commit_oid], |row| read_commit(row, 0))?;
    match rows.next() {
        Some(row) => Ok(Some(commit_from_raw(row?)?)),
        None => Ok(None),
    }
}

/// The earliest or latest dated change to one path, with its commit.
///
/// The `commit_oid` tiebreak is the same load-bearing one [`commit_log`] documents: fixture clocks
/// are synthetic, so ties are guaranteed, and `LIMIT 1` over a partial order would return whichever
/// row the query plan reached first.
fn path_change_at(
    conn: &Connection,
    repo_id: &str,
    path: &str,
    newest: bool,
) -> Result<Option<PathChange>> {
    let direction = if newest { "DESC" } else { "ASC" };
    let mut stmt = conn.prepare(&format!(
        "SELECT {COMMIT_COLUMNS}, {CHANGE_COLUMNS}
           FROM git_change ch
           JOIN git_commit c ON c.repo_id = ch.repo_id AND c.commit_oid = ch.commit_oid
          WHERE ch.repo_id = ?1 AND ch.path = ?2
          ORDER BY c.committer_time {direction}, c.commit_oid ASC
          LIMIT 1"
    ))?;
    let mut rows = stmt.query_map(params![repo_id, path], |row| {
        Ok((read_commit(row, 0)?, read_change(row, COMMIT_COLUMN_COUNT)?))
    })?;
    match rows.next() {
        Some(row) => {
            let (commit, change) = row?;
            Ok(Some(PathChange {
                commit: commit_from_raw(commit)?,
                change: change_from_raw(change)?,
            }))
        }
        None => Ok(None),
    }
}

/// What the `entity` table says about one path.
///
/// The two addressable [`PathRole`]s store the path in different columns, so each needs its own
/// predicate — the same shape `crate::select` uses for a `<rel_path>` selector, generated from the
/// vocabulary rather than from "whatever has a path-shaped scope".
fn current_tree(conn: &Connection, repo_id: &str, path: &str) -> Result<CurrentTree> {
    let indexed: i64 = conn.query_row(
        "SELECT EXISTS (SELECT 1 FROM entity WHERE repo_id = ?1)",
        params![repo_id],
        |row| row.get(0),
    )?;
    let entities_at_path: i64 = conn.query_row(
        &format!(
            "SELECT count(*) FROM entity e
              WHERE e.repo_id = ?1
                AND ((e.kind IN ({content}) AND e.scope_path = ?2)
                  OR (e.kind IN ({container})
                      AND CASE WHEN e.scope_path = '' THEN e.name
                               ELSE e.scope_path || '/' || e.name END = ?2))",
            content = path_kinds_sql(PathRole::Content),
            container = path_kinds_sql(PathRole::Container),
        ),
        params![repo_id, path],
        |row| row.get(0),
    )?;
    Ok(CurrentTree {
        basis: CURRENT_TREE_BASIS,
        index_exists: indexed != 0,
        entities_at_path,
    })
}

/// Which of §4.1's four reasons hides history above the earliest visible change.
///
/// Precedence, and each step is evidence rather than a guess:
///
/// 1. the earliest change's own commit says its parents are unavailable — the most local fact there
///    is, and the only one that names *this* path's boundary;
/// 2. the repository declares a shallow boundary somewhere above;
/// 3. Nerve's own commit budget stopped the walk;
/// 4. an object the walk needed was absent, which is a missing parent.
///
/// [`WalkTermination::Refused`] is deliberately **not** mapped onto one of the four. A refusal is
/// neither a parent situation nor a budget, and relabelling it as either would be the invention this
/// module exists to prevent; [`earlier_changes_may_exist`] is `true` for it, which is the honest
/// warning.
fn earlier_history_unavailable(
    earliest: Option<&CommitRow>,
    ingest: &IngestRow,
) -> Option<EarlierHistoryUnavailable> {
    if let Some(commit) = earliest {
        match commit.parent_completeness {
            ParentCompleteness::ShallowBoundary => {
                return Some(EarlierHistoryUnavailable::ShallowBoundary)
            }
            ParentCompleteness::ParentsMissing => {
                return Some(EarlierHistoryUnavailable::ParentsMissing)
            }
            ParentCompleteness::ParentsUnverifiable => {
                return Some(EarlierHistoryUnavailable::ParentsUnverifiable)
            }
            // Nothing precedes a parentless commit, so nothing above this path's earliest change can
            // be hidden — whatever is true of the repository elsewhere. This returns rather than
            // falling through, and the difference is measurable: a shallow clone can still contain a
            // genuine root (one branch fetched whole, another truncated), and falling through would
            // let the repository-wide shallow flag deny a creation the object graph proves. This is
            // exactly the licence `may_claim_history_begins_here` names.
            ParentCompleteness::Root => return None,
            // A visible parent settles the immediate question and no more: the parent's own ancestry
            // can still be truncated, so the repository-wide checks below apply.
            ParentCompleteness::ParentsAvailable => {}
        }
    }
    if ingest.shallow {
        return Some(EarlierHistoryUnavailable::ShallowBoundary);
    }
    match ingest.walk_terminated_by {
        WalkTermination::CommitBudget => Some(EarlierHistoryUnavailable::CommitBudget),
        WalkTermination::MissingObject => Some(EarlierHistoryUnavailable::ParentsMissing),
        WalkTermination::ShallowBoundary => Some(EarlierHistoryUnavailable::ShallowBoundary),
        WalkTermination::Refused => Some(EarlierHistoryUnavailable::WalkRefused),
        // The only terminal that establishes nothing was hidden. `ingest_history` sets each of the
        // other four from its own cause (`crates/nerve-index/src/history.rs:419-433`): a budget
        // stop, a refusal, a missing object and a reached boundary each win over this arm, so
        // reaching it means the walk read every commit it could name and stopped because there were
        // no more. That is what makes `None` here a *measured* completeness rather than a default.
        WalkTermination::Exhausted => None,
    }
}

/// When one path was first and last observed changing, and which of six answers that is.
///
/// See [`FirstLastObserved`] for the traps. The four states §11 requires to stay distinct all come
/// out of here as different [`FirstObservedKind`] values: no history ingested, history ingested with
/// the path unknown, the path known with zero changes in visible history, and the current tree not
/// knowable at all.
///
/// The path is matched literally against `git_change.path`. No filesystem call is made and none may
/// be added: a historical path is frequently one that no longer exists.
pub fn first_last_observed(
    conn: &Connection,
    repo_id: &str,
    path: &str,
) -> Result<FirstLastObserved> {
    let tree = current_tree(conn, repo_id, path)?;
    let Some(ingest) = history_ingest(conn, repo_id)? else {
        // Absence of an ingest is not absence of history, and it is not a failure (§11).
        return Ok(FirstLastObserved {
            path: path.to_string(),
            kind: FirstObservedKind::NoHistoryIngested,
            may_claim_created: FirstObservedKind::NoHistoryIngested.may_claim_created(),
            first: None,
            last: None,
            changes_in_visible_history: 0,
            earlier_history_unavailable: None,
            earlier_changes_may_exist: false,
            additions_recorded: 0,
            merges_in_repository: 0,
            walk_terminated_by: None,
            shallow: false,
            current_tree: tree,
        });
    };

    let first = path_change_at(conn, repo_id, path, false)?;
    let last = path_change_at(conn, repo_id, path, true)?;
    let changes_in_visible_history: i64 = conn.query_row(
        "SELECT count(DISTINCT commit_oid) FROM git_change WHERE repo_id = ?1 AND path = ?2",
        params![repo_id, path],
        |row| row.get(0),
    )?;
    // Counted rather than derived from `first`, because the claim it licences must not depend on
    // which change happens to be earliest by `committer_time`. See `additions_recorded`.
    let additions_recorded: i64 = conn.query_row(
        "SELECT count(*) FROM git_change
          WHERE repo_id = ?1 AND path = ?2 AND change_kind = ?3",
        params![repo_id, path, ChangeKind::Added.as_str()],
        |row| row.get(0),
    )?;
    let merges_in_repository: i64 = conn.query_row(
        "SELECT count(*) FROM git_commit WHERE repo_id = ?1 AND is_merge = 1",
        params![repo_id],
        |row| row.get(0),
    )?;
    // Derived once, here, and then both consumed by the classification below and reported on the
    // response. The two must not be computed twice: an earlier draft evaluated the reason inside the
    // struct literal while the classification used a different rule, which is how the two disagreed.
    let earlier_unavailable =
        earlier_history_unavailable(first.as_ref().map(|observed| &observed.commit), &ingest);

    let kind = match &first {
        // The one claim that may be rendered as creation, and it rests on three facts, none of them
        // a date.
        //
        // 1. The earliest recorded change is an **addition**, so the path was absent from the parent
        //    tree `ingest_history` diffed against.
        // 2. **Nothing is hidden above it** — `earlier_unavailable` is `None`, reachable only from
        //    `WalkTermination::Exhausted`, which is assigned last after a budget stop, a refusal, a
        //    missing object and a reached boundary have each had their chance.
        // 3. **Exactly one addition is recorded** for the path. This is what makes the claim
        //    clock-independent, and it is the correction to an earlier draft: requiring only (1) and
        //    (2) would rest on `first` being the topologically first change, when `first` is ordered
        //    by `committer_time` and a rebase can reorder that freely. A path created, deleted and
        //    re-created records two additions, so one addition in a complete history means one
        //    creation — whatever the timestamps say.
        //
        // An earlier draft required the commit to be parentless instead of (2) and (3). That is also
        // clock-proof, but it made the value unreachable for every file not in a root commit — on a
        // complete clone of this repository, 6 of ~420 — while the response simultaneously reported
        // `earlier_history_unavailable: None` and `earlier_changes_may_exist: false`. A kind meaning
        // "history above may be hidden" beside two fields saying nothing is hidden is not caution;
        // it is a third statement that contradicts both.
        Some(observed)
            if observed.change.change_kind == ChangeKind::Added
                && earlier_unavailable.is_none()
                && additions_recorded == 1 =>
        {
            FirstObservedKind::CreatedInVisibleHistory
        }
        Some(_) => FirstObservedKind::EarliestVisibleChange,
        // Zero change rows. Which of the three silences it is depends on the current tree, and the
        // current tree is only knowable when an index exists.
        None if !tree.index_exists => FirstObservedKind::CurrentTreeUnknown,
        None if tree.entities_at_path > 0 => FirstObservedKind::PresentBeforeVisibleHistory,
        None => FirstObservedKind::AbsentFromVisibleHistory,
    };

    Ok(FirstLastObserved {
        path: path.to_string(),
        kind,
        may_claim_created: kind.may_claim_created(),
        earlier_history_unavailable: earlier_unavailable,
        earlier_changes_may_exist: earlier_changes_may_exist(&ingest),
        additions_recorded,
        merges_in_repository,
        walk_terminated_by: Some(ingest.walk_terminated_by),
        shallow: ingest.shallow,
        first,
        last,
        changes_in_visible_history,
        current_tree: tree,
    })
}

/// Bounds for [`state_diff`], every one of them explicit.
///
/// Three separate numbers because they bound three different things and collapsing them would make
/// one of the three unbounded. A caller that wants everything asks for [`usize::MAX`], which is what
/// [`as_bound`] saturates for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StateDiffLimits {
    /// Commits the ancestry walk may visit, in each of its two passes.
    pub commits_walked: usize,
    /// Commits the returned range may carry.
    pub commits: usize,
    /// Change rows the returned diff may carry.
    pub changes: usize,
}

impl StateDiffLimits {
    /// The default bounds, matching the ingester's own commit budget so a diff can span a whole
    /// recorded history without the walk bound being the thing that stops it.
    pub const DEFAULT: StateDiffLimits = StateDiffLimits {
        commits_walked: 5_000,
        commits: 500,
        changes: 5_000,
    };
}

impl Default for StateDiffLimits {
    fn default() -> Self {
        StateDiffLimits::DEFAULT
    }
}

/// What changed between two visible states — or why that question has no diff as its answer.
///
/// **Four outcomes, and three of them are refusals that must never be returned as an empty diff.**
/// An empty [`StateDiff::Diff`] means one thing only: `from` and `to` name the same tree of history
/// and nothing lies between them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StateDiff {
    /// One or both endpoints are not recorded commits. Which is named, because "we never read that
    /// commit" and "nothing changed" are different answers.
    StateNotRecorded {
        /// The `from` oid as asked for.
        from: String,
        /// The `to` oid as asked for.
        to: String,
        /// Whether `from` is recorded.
        from_recorded: bool,
        /// Whether `to` is recorded.
        to_recorded: bool,
    },
    /// The walk from `to` exhausted the visible ancestry without reaching `from`, and nothing
    /// stopped it early. `from` is genuinely not an ancestor of `to`.
    NotAnAncestor {
        /// The `from` oid as asked for.
        from: String,
        /// The `to` oid as asked for.
        to: String,
        /// Commits visited before the walk ran out of ancestry.
        commits_walked: usize,
    },
    /// The walk reached a commit whose parents are unavailable before reaching `from`, so whether
    /// `from` is an ancestor cannot be decided.
    AncestryIncomplete {
        /// The `from` oid as asked for.
        from: String,
        /// The `to` oid as asked for.
        to: String,
        /// The commit whose parents could not be followed.
        stopped_at: String,
        /// Why they could not be followed.
        parent_completeness: ParentCompleteness,
        /// Commits visited before stopping.
        commits_walked: usize,
    },
    /// The walk hit **Nerve's own** bound before reaching `from`.
    ///
    /// A fourth outcome, and it exists for the reason [`WalkTermination::CommitBudget`] exists:
    /// returning [`StateDiff::NotAnAncestor`] here would state a property of the repository that was
    /// never established. `docs/plans/slice-12c-historical-questions.md` §5 lists three refusals; a
    /// bounded walk that has not reached `from` is a fourth, and it is Nerve's doing rather than the
    /// repository's.
    WalkBudgetExhausted {
        /// The `from` oid as asked for.
        from: String,
        /// The `to` oid as asked for.
        to: String,
        /// Commits visited before the bound stopped the walk.
        commits_walked: usize,
        /// The bound that stopped it.
        limit: usize,
    },
    /// `from` is an ancestor of `to`, or equal to it, and here is what lies between them.
    Diff(Box<StateDiffReport>),
}

/// The commits and changes between two recorded states, `from` exclusive and `to` inclusive.
///
/// **Merges contribute zero change rows by Slice 12b's decision**, so a merge-heavy range reporting
/// few changes is expected. [`StateDiffReport::merges_in_range`] and
/// [`StateDiffReport::changes_enumerated`] are what stop that reading as "little changed"; neither
/// is optional decoration.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StateDiffReport {
    /// The `from` oid, excluded from the range.
    pub from: String,
    /// The `to` oid, included in the range.
    pub to: String,
    /// The range, newest committer time first, `commit_oid` breaking ties.
    pub commits: Vec<CommitRow>,
    /// How many commits the range holds, before [`StateDiffLimits::commits`] cut it.
    pub commits_in_range: usize,
    /// Whether [`StateDiffLimits::commits`] cut the range. A fact, not `len() == limit`.
    pub commits_truncated: bool,
    /// Commits the walk visited, including the pruned boundary.
    pub commits_walked: usize,
    /// The changes of the returned commits, in the same order, then by path.
    pub changes: Vec<ChangeRow>,
    /// Whether these are **not** all the changes in the range.
    ///
    /// True when [`StateDiffLimits::changes`] cut the list, **and also whenever
    /// [`StateDiffReport::commits_truncated`] is true**: the changes are the changes of the returned
    /// commits, so a cut commit list necessarily means a cut diff. Two independent flags would let a
    /// consumer read `changes_truncated: false` beside a paged commit list as "this is everything
    /// that changed", which is the shape of mistake this module exists to make impossible.
    pub changes_truncated: bool,
    /// Merges inside the range. Their changes are not enumerated, by decision.
    pub merges_in_range: usize,
    /// How many commits in the range are in each enumeration state, every state present with a zero.
    pub changes_enumerated: BTreeMap<ChangesEnumerated, usize>,
    /// A commit inside the range whose parents could not be followed, if there is one.
    ///
    /// Present even in a successful diff: `from` may have been reached down one branch while another
    /// branch of the range was cut off, in which case the range is a floor rather than the whole of
    /// it. Reporting the diff without this would be the "an absence is never inferred" rule broken
    /// one level up from where 12b enforced it.
    pub ancestry_incomplete_at: Option<(String, ParentCompleteness)>,
}

/// Ancestors of one recorded commit, including itself.
///
/// Bounded, and `truncated` is **not** advisory. The set is used to *prune* the second walk, so a
/// floor prunes too little and the range would silently gain commits that are ancestors of `from` —
/// a wrong answer rather than a wide one. [`state_diff`] therefore refuses with
/// [`StateDiff::WalkBudgetExhausted`] when this is `true`, which is Nerve's own bound admitting it
/// could not compute the answer, rather than reporting an answer it could not compute.
fn ancestor_set(
    conn: &Connection,
    repo_id: &str,
    start: &str,
    recorded: &BTreeSet<String>,
    limit: usize,
) -> Result<(BTreeSet<String>, bool)> {
    let mut seen = BTreeSet::new();
    let mut queue = VecDeque::new();
    seen.insert(start.to_string());
    queue.push_back(start.to_string());
    let mut visited = 0usize;
    while let Some(oid) = queue.pop_front() {
        if visited >= limit {
            return Ok((seen, true));
        }
        visited += 1;
        let Some(commit) = commit_by_oid(conn, repo_id, &oid)? else {
            continue;
        };
        for parent in &commit.parent_oids {
            if !recorded.contains(parent) {
                continue;
            }
            if seen.insert(parent.clone()) {
                queue.push_back(parent.clone());
            }
        }
    }
    Ok((seen, false))
}

/// What changed between two visible states, by **ancestry** and never by a `committer_time` range.
///
/// A time range is not an ancestry range and answering one for the other fails silently: a merge
/// brings in commits whose committer time precedes the merge, and a rebase or a fabricated date
/// reorders them freely. `commit_log` already orders by `committer_time`, which is exactly what makes
/// the wrong implementation the convenient one.
///
/// The walk is `to` toward `from`, over `parent_oids`, pruned at the ancestors of `from` so that a
/// side branch merged in below `from` does not drag `from`'s own history into the range — which is
/// what `git log from..to` means and what a single unpruned walk would get wrong.
pub fn state_diff(
    conn: &Connection,
    repo_id: &str,
    from: &str,
    to: &str,
    limits: StateDiffLimits,
) -> Result<StateDiff> {
    let from_row = commit_by_oid(conn, repo_id, from)?;
    let to_row = commit_by_oid(conn, repo_id, to)?;
    if from_row.is_none() || to_row.is_none() {
        return Ok(StateDiff::StateNotRecorded {
            from: from.to_string(),
            to: to.to_string(),
            from_recorded: from_row.is_some(),
            to_recorded: to_row.is_some(),
        });
    }

    let recorded = recorded_commit_oids(conn, repo_id)?;
    let (below, below_truncated) =
        ancestor_set(conn, repo_id, from, &recorded, limits.commits_walked)?;
    if below_truncated {
        // The prune set is a floor, so the range below would gain `from`'s own ancestors. Refusing is
        // the only honest answer: the bound is Nerve's, and a wrong range is worse than no range.
        return Ok(StateDiff::WalkBudgetExhausted {
            from: from.to_string(),
            to: to.to_string(),
            commits_walked: limits.commits_walked,
            limit: limits.commits_walked,
        });
    }

    let mut range: Vec<CommitRow> = Vec::new();
    let mut seen = BTreeSet::new();
    let mut queue = VecDeque::new();
    seen.insert(to.to_string());
    queue.push_back(to.to_string());
    let mut walked = 0usize;
    let mut reached = false;
    let mut incomplete: Option<(String, ParentCompleteness)> = None;

    while let Some(oid) = queue.pop_front() {
        if walked >= limits.commits_walked {
            return Ok(StateDiff::WalkBudgetExhausted {
                from: from.to_string(),
                to: to.to_string(),
                commits_walked: walked,
                limit: limits.commits_walked,
            });
        }
        walked += 1;
        if below.contains(&oid) {
            // The boundary. Not expanded, so `from`'s own history stays out of the range.
            if oid == from {
                reached = true;
            }
            continue;
        }
        let Some(commit) = commit_by_oid(conn, repo_id, &oid)? else {
            continue;
        };
        // A parent absent from the object store, and a parent present but never recorded, are both
        // "the range may be larger than this". The first is a property of the repository and is
        // named by `parent_completeness`; the second is a property of the ingest.
        if !commit.parent_completeness.may_claim_history_begins_here()
            && commit.parent_completeness != ParentCompleteness::ParentsAvailable
        {
            incomplete = Some((commit.commit_oid.clone(), commit.parent_completeness));
        }
        for parent in &commit.parent_oids {
            if !recorded.contains(parent) {
                incomplete = Some((commit.commit_oid.clone(), commit.parent_completeness));
                continue;
            }
            if seen.insert(parent.clone()) {
                queue.push_back(parent.clone());
            }
        }
        range.push(commit);
    }

    if !reached {
        return Ok(match incomplete {
            Some((stopped_at, parent_completeness)) => StateDiff::AncestryIncomplete {
                from: from.to_string(),
                to: to.to_string(),
                stopped_at,
                parent_completeness,
                commits_walked: walked,
            },
            None => StateDiff::NotAnAncestor {
                from: from.to_string(),
                to: to.to_string(),
                commits_walked: walked,
            },
        });
    }

    // Total order, and the `commit_oid` tiebreak is the same load-bearing one `commit_log` needs:
    // synthetic fixture clocks tie constantly.
    range.sort_by(|left, right| {
        right
            .committer_time
            .cmp(&left.committer_time)
            .then_with(|| left.commit_oid.cmp(&right.commit_oid))
    });
    let commits_in_range = range.len();
    let merges_in_range = range.iter().filter(|commit| commit.is_merge).count();
    let mut changes_enumerated: BTreeMap<ChangesEnumerated, usize> = ChangesEnumerated::ALL
        .into_iter()
        .map(|value| (value, 0))
        .collect();
    for commit in &range {
        *changes_enumerated
            .entry(commit.changes_enumerated)
            .or_insert(0) += 1;
    }

    let commits_truncated = commits_in_range > limits.commits;
    range.truncate(limits.commits);

    let mut changes = Vec::new();
    // A cut commit list is a cut diff: the changes below are the changes of the *returned* commits.
    let mut changes_truncated = commits_truncated;
    for commit in &range {
        if changes.len() >= limits.changes {
            changes_truncated = true;
            break;
        }
        let mut rows = changes_for_commit(conn, repo_id, &commit.commit_oid)?;
        let room = limits.changes - changes.len();
        if rows.len() > room {
            changes_truncated = true;
            rows.truncate(room);
        }
        changes.append(&mut rows);
    }

    Ok(StateDiff::Diff(Box::new(StateDiffReport {
        from: from.to_string(),
        to: to.to_string(),
        commits: range,
        commits_in_range,
        commits_truncated,
        commits_walked: walked,
        changes,
        changes_truncated,
        merges_in_range,
        changes_enumerated,
        ancestry_incomplete_at: incomplete,
    })))
}

/// One path and how many recorded commits touched it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChangeFrequencyRow {
    /// The path, as a tree recorded it.
    pub path: String,
    /// `count(DISTINCT commit_oid)` for that path, **within visible history**.
    pub commits: i64,
}

/// Which paths changed most often in visible history.
///
/// Two honesty requirements, both carried as fields rather than left to documentation:
/// [`ChangeFrequency::merges`] is nonzero exactly when the repository undercounts against its own
/// log, because merges contribute no change rows by decision; and every count is a **floor** on a
/// shallow or bounded ingest, which is why the caller must render this beside the availability block.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChangeFrequency {
    /// The rows, count descending then path ascending.
    pub rows: Vec<ChangeFrequencyRow>,
    /// Distinct paths with at least one change row in visible history.
    pub paths_total: i64,
    /// Whether [`ChangeFrequency::rows`] is shorter than [`ChangeFrequency::paths_total`]. A
    /// comparison against a counted total, never `len() == limit`.
    pub truncated: bool,
    /// The bound in force.
    pub limit: usize,
    /// Merges recorded for this repository. Their changes are not enumerated, so a merge-heavy
    /// workflow undercounts against its own log.
    pub merges: i64,
}

/// Which paths changed most frequently, bounded and totally ordered.
///
/// A count tie is the *normal* case here — most paths change once — so the order needs an explicit
/// second key or it is whatever the query plan produced.
///
/// **The `path ASC` tiebreak cannot be falsified by a test on this schema, and that is measured
/// rather than assumed.** `EXPLAIN QUERY PLAN` for this statement is
/// `SEARCH git_change USING INDEX idx_git_change_path (repo_id=?)` with no temp b-tree for the
/// `GROUP BY`, so groups are emitted in `path` order and the `ORDER BY touches DESC` sort preserves
/// that order among ties — with or without the tiebreak. Deleting it was probed on this suite and
/// **every ordering assertion still passed**, which is 12b's fifth vacuity trap in a new place.
///
/// The tiebreak stays because a query plan is not part of the contract: `ANALYZE`, a schema change,
/// or a different SQLite build may group through a temp b-tree instead, and then tied rows come back
/// in sorter order. What the suite can check is that the clause is present, which
/// `the_derived_orderings_state_their_tiebreaks` does on the source — recorded as a source-level
/// check rather than dressed up as a behavioural one.
pub fn change_frequency(conn: &Connection, repo_id: &str, limit: usize) -> Result<ChangeFrequency> {
    let mut stmt = conn.prepare(
        "SELECT path, count(DISTINCT commit_oid) AS touches
           FROM git_change WHERE repo_id = ?1
          GROUP BY path
          ORDER BY touches DESC, path ASC
          LIMIT ?2",
    )?;
    let rows = stmt.query_map(params![repo_id, as_bound(limit)], |row| {
        Ok(ChangeFrequencyRow {
            path: row.get(0)?,
            commits: row.get(1)?,
        })
    })?;
    let mut out = Vec::new();
    for row in rows {
        out.push(row?);
    }
    let paths_total: i64 = conn.query_row(
        "SELECT count(DISTINCT path) FROM git_change WHERE repo_id = ?1",
        params![repo_id],
        |row| row.get(0),
    )?;
    let merges: i64 = conn.query_row(
        "SELECT count(*) FROM git_commit WHERE repo_id = ?1 AND is_merge = 1",
        params![repo_id],
        |row| row.get(0),
    )?;
    Ok(ChangeFrequency {
        truncated: i64::try_from(out.len()).unwrap_or(i64::MAX) < paths_total,
        rows: out,
        paths_total,
        limit,
        merges,
    })
}

/// Two paths and how many recorded commits changed both of them.
///
/// **This is not a dependency, a coupling or an affinity.** Two files changing together is equally
/// consistent with coupling, with a formatting sweep, with a release-version bump, and with one
/// commit that did two unrelated things. The field is named
/// [`CochangeRow::cochange_observations`] for that reason, and the count is a **raw shared-commit
/// count** rather than a normalised figure — a normalised number invites exactly the comparison the
/// label forbids. No `Relation` is emitted and no assertion is written: co-change exists only in a
/// response.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CochangeRow {
    /// The lexicographically smaller path of the pair.
    pub path_a: String,
    /// The lexicographically larger path of the pair.
    pub path_b: String,
    /// How many recorded commits changed both paths. An observation, not an inference.
    pub cochange_observations: i64,
}

/// The sentence a co-change response must carry, in one place so no surface writes its own.
///
/// Nerve has refused a weaker version of this inference twice already: `ADR_DESCRIBES_COMPONENT` was
/// refused because no deterministic rule separates "describes" from "mentions", and identity is
/// never established by fuzzy name matching alone.
pub const COCHANGE_IS_NOT_A_DEPENDENCY: &str =
    "a shared-commit count is an observation, not a dependency: two paths changing together is \
     equally consistent with coupling, a formatting sweep, a version bump, and one commit that did \
     two unrelated things";

/// Which paths changed together, bounded and totally ordered.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Cochange {
    /// The pairs, shared-commit count descending then both paths ascending.
    pub rows: Vec<CochangeRow>,
    /// Distinct pairs that share at least one commit, under the same filter.
    pub pairs_total: i64,
    /// Whether [`Cochange::rows`] is shorter than [`Cochange::pairs_total`].
    pub truncated: bool,
    /// The bound in force.
    pub limit: usize,
    /// The path the query was restricted to, if any.
    pub path: Option<String>,
    /// [`COCHANGE_IS_NOT_A_DEPENDENCY`], carried on the response rather than left to a footnote a
    /// consumer can drop.
    pub disclaimer: &'static str,
    /// Merges recorded for this repository, which contribute no pairs at all.
    pub merges: i64,
}

/// Which paths changed together, as a **raw shared-commit count**.
///
/// `git_change` self-joined on `commit_oid`, with `b.path > a.path` so each unordered pair appears
/// once. The join uses a prefix of the `(repo_id, commit_oid, path)` primary key, which is why §7 and
/// §8 need no new index.
///
/// `path`, when given, restricts the answer to pairs naming it — the "what changed with this file"
/// question — and [`Cochange::pairs_total`] is counted under the same restriction, so truncation
/// stays a fact.
///
/// The two path keys in the `ORDER BY` carry the same caveat as [`change_frequency`]'s: deleting them
/// was probed and no ordering assertion failed, because this plan's `USE TEMP B-TREE FOR GROUP BY`
/// happens to emit pairs in path order and the outer sort preserves it among ties. They stay because
/// that is a property of one query plan rather than of the answer.
pub fn cochange(
    conn: &Connection,
    repo_id: &str,
    path: Option<&str>,
    limit: usize,
) -> Result<Cochange> {
    const PAIRS: &str = "SELECT a.path AS left_path, b.path AS right_path,
                                count(DISTINCT a.commit_oid) AS shared
                           FROM git_change a
                           JOIN git_change b
                             ON b.repo_id = a.repo_id AND b.commit_oid = a.commit_oid
                                AND b.path > a.path
                          WHERE a.repo_id = ?1
                            AND (?2 IS NULL OR a.path = ?2 OR b.path = ?2)
                          GROUP BY a.path, b.path";
    let mut stmt = conn.prepare(&format!(
        "{PAIRS} ORDER BY shared DESC, left_path ASC, right_path ASC LIMIT ?3"
    ))?;
    let rows = stmt.query_map(params![repo_id, path, as_bound(limit)], |row| {
        Ok(CochangeRow {
            path_a: row.get(0)?,
            path_b: row.get(1)?,
            cochange_observations: row.get(2)?,
        })
    })?;
    let mut out = Vec::new();
    for row in rows {
        out.push(row?);
    }
    let pairs_total: i64 = conn.query_row(
        &format!("SELECT count(*) FROM ({PAIRS})"),
        params![repo_id, path],
        |row| row.get(0),
    )?;
    let merges: i64 = conn.query_row(
        "SELECT count(*) FROM git_commit WHERE repo_id = ?1 AND is_merge = 1",
        params![repo_id],
        |row| row.get(0),
    )?;
    Ok(Cochange {
        truncated: i64::try_from(out.len()).unwrap_or(i64::MAX) < pairs_total,
        rows: out,
        pairs_total,
        limit,
        path: path.map(str::to_string),
        disclaimer: COCHANGE_IS_NOT_A_DEPENDENCY,
        merges,
    })
}

/// Whether the recorded history still describes the repository's current commit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HistoryFreshnessReport {
    /// Which of the four verdicts this is.
    pub verdict: HistoryFreshness,
    /// HEAD as the last ingest read it. `None` on an unborn branch, or with no ingest at all.
    pub ingest_head_oid: Option<String>,
    /// `repository_state.git_commit` for the newest recorded extractor run. `None` when no index has
    /// run here, or when the tree had no readable `.git/HEAD` at index time.
    pub current_git_commit: Option<String>,
    /// The state the current commit was read from, so the comparison is traceable.
    pub current_state_id: Option<String>,
}

/// Compare the ingest's HEAD against the repository's current commit.
///
/// Four verdicts, and [`HistoryFreshness::Unverifiable`] is the one that must not collapse into
/// [`HistoryFreshness::Current`]: a repository state with no recorded commit cannot be compared, and
/// reporting "unknown" as "current" is how a truncated sweep becomes a clean bill of health. That is
/// the distinction Slice 7c-i drew between `Freshness::Stale` and `Freshness::Unverified`.
///
/// "Current" is the state named by the newest `extractor_run`, which is how `status` already decides
/// it — one definition of *now*, not two.
pub fn history_freshness(conn: &Connection, repo_id: &str) -> Result<HistoryFreshnessReport> {
    let current: Option<(String, Option<String>)> = conn
        .query_row(
            "SELECT s.state_id, s.git_commit
               FROM extractor_run r
               JOIN repository_state s ON s.state_id = r.state_id
              WHERE r.repo_id = ?1
              ORDER BY r.run_id DESC LIMIT 1",
            params![repo_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .ok();
    let current_state_id = current.as_ref().map(|(state_id, _)| state_id.clone());
    let current_git_commit = current.and_then(|(_, git_commit)| git_commit);

    let ingest = history_ingest(conn, repo_id)?;
    let ingest_head_oid = ingest.as_ref().and_then(|row| row.head_oid.clone());

    let verdict = match (&ingest, &current_git_commit) {
        (None, _) => HistoryFreshness::NoHistoryIngested,
        // No commit to compare against. Not `current`.
        (Some(_), None) => HistoryFreshness::Unverifiable,
        (Some(row), Some(commit)) => {
            if row.head_oid.as_deref() == Some(commit.as_str()) {
                HistoryFreshness::Current
            } else {
                HistoryFreshness::Stale
            }
        }
    };

    Ok(HistoryFreshnessReport {
        verdict,
        ingest_head_oid,
        current_git_commit,
        current_state_id,
    })
}
