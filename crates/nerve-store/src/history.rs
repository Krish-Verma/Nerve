//! The historical model: `git_commit`, `git_change`, `git_rename_hypothesis` and
//! `git_history_ingest` (schema v6, Slice 12b).
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
//!    (`crates/nerve-index/src/pipeline.rs:654-666`). [`insert_changes`] and [`insert_renames`]
//!    use a plain `INSERT`, so a constraint or foreign-key violation is an error.
//! 3. **Every read has a total order.** History fixtures use fixed synthetic dates, so ties on
//!    `committer_time` are guaranteed rather than unlikely; an order that stopped at the timestamp
//!    would be a flaky test waiting for a fixture to grow a second commit in the same second.

use std::collections::{BTreeMap, BTreeSet};

use rusqlite::{params, Connection, Row};

use nerve_core::vocab::{
    ChangeKind, ChangesEnumerated, ParentCompleteness, RenameAmbiguity, RenameEvidence,
    WalkTermination,
};

use crate::error::{Result, StoreError};

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
    /// The blob both paths name.
    pub blob_oid: String,
    /// How many ways the pairing could have been drawn.
    pub ambiguity: RenameAmbiguity,
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
     c.author_ident, c.committer_ident, c.summary, c.is_merge";

/// How many columns [`COMMIT_COLUMNS`] names, so a join can offset past them.
const COMMIT_COLUMN_COUNT: usize = 13;

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
        is_merge: row.get(base + 12)?,
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
    blob_oid: String,
    ambiguity: String,
}

fn rename_from_raw(raw: RawRename) -> Result<RenameRow> {
    Ok(RenameRow {
        commit_oid: raw.commit_oid,
        from_path: raw.from_path,
        to_path: raw.to_path,
        evidence: raw.evidence.parse()?,
        blob_oid: raw.blob_oid,
        ambiguity: raw.ambiguity.parse()?,
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
pub fn insert_commit(conn: &Connection, repo_id: &str, row: &CommitRow) -> Result<bool> {
    let parent_oids = encode_json("git_commit.parent_oids", &row.parent_oids)?;
    let written = conn.execute(
        "INSERT OR IGNORE INTO git_commit
             (repo_id, commit_oid, tree_oid, parent_oids, parent_completeness,
              changes_enumerated, author_time, author_tz, committer_time, committer_tz,
              author_ident, committer_ident, summary, is_merge)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14)",
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
pub fn insert_renames(conn: &Connection, repo_id: &str, rows: &[RenameRow]) -> Result<usize> {
    let mut stmt = conn.prepare(
        "INSERT INTO git_rename_hypothesis
             (repo_id, commit_oid, from_path, to_path, evidence, blob_oid, ambiguity)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
    )?;
    let mut written = 0;
    for row in rows {
        written += stmt.execute(params![
            repo_id,
            row.commit_oid,
            row.from_path,
            row.to_path,
            row.evidence.as_str(),
            row.blob_oid,
            row.ambiguity.as_str(),
        ])?;
    }
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
    let mut stmt = conn.prepare(
        "SELECT r.commit_oid, r.from_path, r.to_path, r.evidence, r.blob_oid, r.ambiguity
           FROM git_rename_hypothesis r
           JOIN git_commit c ON c.repo_id = r.repo_id AND c.commit_oid = r.commit_oid
          WHERE r.repo_id = ?1 AND (r.from_path = ?2 OR r.to_path = ?2)
          ORDER BY c.committer_time DESC, r.commit_oid ASC, r.from_path ASC, r.to_path ASC
          LIMIT ?3",
    )?;
    let rows = stmt.query_map(params![repo_id, path, as_bound(limit)], |row| {
        Ok(RawRename {
            commit_oid: row.get(0)?,
            from_path: row.get(1)?,
            to_path: row.get(2)?,
            evidence: row.get(3)?,
            blob_oid: row.get(4)?,
            ambiguity: row.get(5)?,
        })
    })?;
    let mut out = Vec::new();
    for row in rows {
        out.push(rename_from_raw(row?)?);
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
