//! `nerve history sync`'s engine: read the commit graph out of `.git` and record what it says.
//!
//! Slice 12a is a reader that writes nothing. This is the half that writes, and it is the first code
//! in Nerve to persist a claim about what the repository *was*. `docs/plans/slice-12b-historical-model.md`
//! is the design; three things in it are load-bearing enough to restate beside the code.
//!
//! # 1. A shallow boundary is not a root commit
//!
//! A root commit is diffed against the **empty tree**, so every path in it is `added`. That is
//! correct for a root and catastrophic for anything else: doing it to a shallow boundary reports
//! every file in the boundary tree as newly added at the boundary, which is *"the project's history
//! begins here"* stated as data rather than as prose. So
//! [`ParentCompleteness::ShallowBoundary`], [`ParentCompleteness::ParentsMissing`] and
//! [`ParentCompleteness::ParentsUnverifiable`] all get
//! [`ChangesEnumerated::ParentUnavailable`] and **zero** change rows — zero because the parent tree
//! was unreadable, not because nothing changed, which is why the column is stored rather than
//! inferred from a row count.
//!
//! # 2. Five reasons a commit has no visible parent, and the fifth is the undecidable one
//!
//! [`ParentCompleteness`] has five values and this module derives them. The one that needs code
//! rather than a lookup is [`ParentCompleteness::ParentsUnverifiable`]: 12a's
//! `StoreLimits::shallow` is `None` for *absent*, *over the pointer-file bound* and *unreadable*
//! alike, and `None` is defined as "not shallow". So a repository that is shallow and whose
//! `shallow` file Nerve could not read would report as complete, and an absent parent would be
//! called corrupt. This module therefore re-stats `<common_dir>/shallow` itself: **a file that
//! exists on disk while `StoreLimits::shallow` is `None` is exactly the undecidable case.** Two
//! shallow-related refusal forms from 12a — a dropped line, an exceeded entry bound — mean the same
//! thing.
//!
//! # 3. An unchanged directory costs one oid comparison
//!
//! The delta storage strategy was measured at 30.1× fewer rows than per-commit snapshots on this
//! repository and 177× on a 1,214-commit one, and the measurement is only achievable because a
//! subtree whose oid is unchanged is **skipped without being read**. That is not an optimisation
//! bolted on afterwards; it is what makes the cost `O(churn)` instead of `O(commits × tree)`.
//! [`HistoryOutcome::subtrees_skipped`] and [`HistoryOutcome::trees_read`] exist so a test can prove
//! the walk did not descend, rather than inferring it from change rows that look the same either
//! way.
//!
//! # What this module does not do
//!
//! No subprocess, no `git` binary, no network, no repository code. It reads `.git` through
//! [`crate::gitobj`] and nothing else. Tree entry names are bytes and are **not** trusted:
//! [`crate::discover::safe_tree_name`] is the filesystem-free choke point they pass, because
//! [`crate::discover::canonical_child`] canonicalizes and would therefore refuse every `deleted`
//! path. Commit summaries are the first free-form repository prose Nerve stores: bounded, first line
//! only, lossy UTF-8, never interpreted.

use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::path::{Path, PathBuf};
use std::time::Instant;

use nerve_core::vocab::{
    ChangeKind, ChangesEnumerated, ParentCompleteness, RenameAmbiguity, RenameEvidence,
    SummaryTruncation, WalkTermination,
};
use nerve_store::{ChangeRow, CommitRow, IngestRow, RenameRow};

use crate::config;
use crate::discover::{canonical_root, safe_tree_name};
use crate::error::{IndexError, Result};
use crate::gitinfo;
use crate::gitobj::{self, Commit, Identity, Object, ObjectStore, Oid, TreeEntry};
use crate::pipeline::RunStatus;

/// Commits one sync will walk, however large the request.
///
/// 5000 is the clamp, not a default preference: a repository with more history is ordinary, and the
/// bound is what stops one command from turning into an unbounded read of an attacker-shaped commit
/// graph. A request above it is **refused with the clamp stated** rather than silently honoured, so
/// a caller learns that its number was not the one used.
pub const MAX_HISTORY_COMMITS: usize = 5_000;

/// Entries one tree object may name.
///
/// A tree is parsed whole, so its entry count is an allocation the repository chooses. 100 000 is
/// far past the largest single directory in any real tree and bounds what one hostile object costs.
pub const MAX_TREE_ENTRIES: usize = 100_000;

/// Change rows one commit may contribute.
///
/// A commit that rewrites 10 000 paths exists (a vendored-directory import, a licence sweep); one
/// that rewrites more is either generated or built to make this table grow. The commit is still
/// recorded — with [`ChangesEnumerated::Refused`], so the zero rows are qualified.
pub const MAX_CHANGES_PER_COMMIT: usize = 10_000;

/// Bytes of a commit summary that are stored.
///
/// The summary is the first repository *prose* Nerve keeps, and it is attacker-influencable in any
/// repository that accepts contributions. 512 bytes is more than a conventional subject line and
/// bounds what one commit can put in the database. Truncation is **counted**, never silent.
pub const MAX_SUMMARY_BYTES: usize = 512;

/// The producer named on every exact-content rename hypothesis.
///
/// On the row rather than left implicit, because schema v7 admits more than one matcher and a row
/// that did not name its own producer would have to be attributed by a join a caller might forget.
/// This one reads no blob content at all — the oids are already in the tree diff — which is exactly
/// why it records no measurement and no analysis row.
pub const EXACT_MATCHER_ID: &str = "git-blob-oid";

/// Version of [`EXACT_MATCHER_ID`]. Changing what the matcher means is a version bump here, never a
/// silent redefinition of rows already on disk.
pub const EXACT_MATCHER_VERSION: &str = "1";

/// Bytes of an author or committer identity that are stored when identity capture is requested.
///
/// Identity is off by default (see [`HistoryOptions::with_identity`]), and when it is on, a name and
/// an email are untrusted strings on the same terms as the summary. Same reason, smaller bound: an
/// identity has no legitimate reason to be longer than a subject line.
pub const MAX_IDENT_BYTES: usize = 256;

/// Version of this ingester, recorded in `git_history_ingest.reader_version`.
///
/// The reader's version rather than an extractor version, because nothing here emits an
/// observation: history is a primary-source fact in plain tables, not an assertion.
pub const READER_VERSION: &str = "1.0.0";

/// Refusal tags this module adds. **Closed**, in the manner of [`crate::gitobj::form`],
/// [`crate::coverage::form`] and [`crate::trace::form`].
///
/// [`crate::discover::TreeNameRefusal`] carries the path-guard tags, and every tag 12a can produce
/// arrives through [`crate::gitobj::Error::form`]. All three vocabularies share one counter map, so
/// one reading of a sync shows what the format reader refused, what the path guard refused, and what
/// this module's own bounds refused.
pub mod form {
    /// The commit budget was enforced: either the walk stopped at it, or a larger request was
    /// clamped to [`super::MAX_HISTORY_COMMITS`].
    pub const COMMIT_BUDGET: &str = "history-commit-budget";
    /// A tree named more entries than [`super::MAX_TREE_ENTRIES`].
    pub const TREE_TOO_LARGE: &str = "history-tree-too-large";
    /// A commit's diff produced more rows than [`super::MAX_CHANGES_PER_COMMIT`].
    pub const CHANGES_TOO_MANY: &str = "history-changes-too-many";
    /// A commit summary was longer than [`super::MAX_SUMMARY_BYTES`] and was truncated.
    pub const SUMMARY_TRUNCATED: &str = "history-summary-truncated";
    /// An identity string was longer than [`super::MAX_IDENT_BYTES`] and was truncated.
    pub const IDENT_TRUNCATED: &str = "history-ident-truncated";
    /// One tree named the same entry twice. The first is kept; the second is refused, because
    /// silently overwriting it would drop a change and silently keeping both would collide on
    /// `git_change`'s primary key.
    pub const DUPLICATE_PATH: &str = "history-duplicate-path";
    /// A tree object a diff needed was **absent** from the store rather than refused by it.
    ///
    /// Distinct from a refusal in what it means and identical in what it costs: the diff cannot run.
    pub const TREE_ABSENT: &str = "history-tree-absent";
    /// `<common_dir>/shallow` exists while the reader reports the repository as not shallow, so
    /// whether an absent parent was *declared* absent cannot be established.
    ///
    /// The one condition that produces [`nerve_core::vocab::ParentCompleteness::ParentsUnverifiable`]
    /// from a file rather than from a counter.
    pub const SHALLOW_UNVERIFIABLE: &str = "history-shallow-unverifiable";

    /// Every tag in this module, in declaration order.
    pub const ALL: [&str; 8] = [
        COMMIT_BUDGET,
        TREE_TOO_LARGE,
        CHANGES_TOO_MANY,
        SUMMARY_TRUNCATED,
        IDENT_TRUNCATED,
        DUPLICATE_PATH,
        TREE_ABSENT,
        SHALLOW_UNVERIFIABLE,
    ];
}

/// What one sync is allowed to read.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HistoryOptions {
    /// Commits to walk. Clamped to [`MAX_HISTORY_COMMITS`], with the clamp counted under
    /// [`form::COMMIT_BUDGET`] rather than applied quietly.
    pub max_commits: usize,
    /// Whether to store author and committer identities.
    ///
    /// Off by default, and that is a data-protection decision rather than a performance one: not one
    /// question the historical model answers asks *who*, so storing contributor names and email
    /// addresses would put third-party personal data in the index with no query behind it. The
    /// columns exist so that enabling it later needs no migration.
    pub with_identity: bool,
}

impl Default for HistoryOptions {
    fn default() -> Self {
        Self {
            max_commits: MAX_HISTORY_COMMITS,
            with_identity: false,
        }
    }
}

/// What one sync read, wrote, repaired and refused.
///
/// Every field a caller would otherwise have to infer is here, and the inferences this type exists
/// to prevent are named on the fields themselves: a zero is a different fact from an absence, and
/// [`HistoryOutcome::walk_terminated_by`] is Nerve's boundary rather than the repository's.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HistoryOutcome {
    /// Canonical repository root.
    pub root: PathBuf,
    /// The git directory that was opened. Differs from `<root>/.git` for a linked worktree.
    pub git_dir: PathBuf,
    /// `HEAD` at sync time. `None` on an unborn branch, which is a success and not an error.
    pub head_oid: Option<String>,
    /// Tips the walk started from.
    pub walked_from: Vec<String>,
    /// The budget actually in force, after clamping.
    pub commit_budget: usize,
    /// Commits dequeued and examined, whether or not they were newly recorded.
    pub commits_walked: usize,
    /// Commits newly written to `git_commit`.
    pub commits_recorded: usize,
    /// Commits the walk reached that were already recorded, so their changes were not re-derived.
    pub commits_already_present: usize,
    /// Commits the repair step deleted before walking, because their availability was a conclusion
    /// from absence and absence can improve.
    pub commits_repaired: usize,
    /// `git_change` rows written.
    pub changes_written: usize,
    /// `git_rename_hypothesis` rows written.
    pub renames_written: usize,
    /// Why the walk stopped. [`WalkTermination::CommitBudget`] is **Nerve's** boundary and must
    /// never be read as the repository being unable to go further.
    pub walk_terminated_by: WalkTermination,
    /// Whether the repository declares a shallow boundary.
    pub shallow: bool,
    /// The declared boundary oids. Empty when not shallow.
    pub shallow_boundary: Vec<String>,
    /// Whether the repository is a partial clone, so a missing object may exist on a remote Nerve is
    /// forbidden to call.
    pub promisor: bool,
    /// Commits per [`ParentCompleteness`], with a zero for every value that has none.
    pub completeness: BTreeMap<ParentCompleteness, usize>,
    /// Commits per [`ChangesEnumerated`], with a zero for every value that has none.
    pub enumeration: BTreeMap<ChangesEnumerated, usize>,
    /// Tree objects read. Paired with [`HistoryOutcome::subtrees_skipped`], this is what makes the
    /// equal-oid shortcut measurable instead of asserted.
    pub trees_read: usize,
    /// Subtrees whose oid was unchanged and which were therefore **not** read.
    pub subtrees_skipped: usize,
    /// Commit summaries that were truncated at [`MAX_SUMMARY_BYTES`].
    pub summaries_truncated: usize,
    /// Refusals by closed-vocabulary form, across 12a's reader, the path guard and this module.
    pub refused: BTreeMap<String, usize>,
    /// [`READER_VERSION`].
    pub reader_version: String,
    /// Wall time.
    pub duration_ms: u128,
    /// Whether anything was refused.
    pub status: RunStatus,
}

impl HistoryOutcome {
    /// How many times `tag` was refused.
    pub fn refusals(&self, tag: &str) -> usize {
        self.refused.get(tag).copied().unwrap_or(0)
    }

    /// Commits recorded with `value`.
    pub fn completeness_of(&self, value: ParentCompleteness) -> usize {
        self.completeness.get(&value).copied().unwrap_or(0)
    }

    /// Commits recorded with `value`.
    pub fn enumeration_of(&self, value: ChangesEnumerated) -> usize {
        self.enumeration.get(&value).copied().unwrap_or(0)
    }
}

/// Read the commit graph reachable from `HEAD` and record it.
///
/// Requires `nerve init`, and nothing more: history resolves nothing against the graph, so a
/// repository that has never been indexed still has a history to read. That is the opposite of
/// `nerve coverage`, which refuses without an index because every path in a report is resolved
/// against what was indexed.
///
/// An **unborn branch** — `HEAD` naming a ref that does not exist yet — is a success:
/// `head_oid = NULL`, no commits, [`WalkTermination::Exhausted`]. A repository with no readable git
/// directory at all is a different fact and is an error.
pub fn ingest_history(root: &Path, options: &HistoryOptions) -> Result<HistoryOutcome> {
    let started = Instant::now();
    let root = canonical_root(root)?;
    let db_path = config::db_path(&root);
    if !db_path.exists() {
        return Err(IndexError::NotInitialized(root));
    }

    // The pairing 12a's doc comment specifies, and the reason a linked worktree works at all:
    // `git_dir` resolves the `.git`-is-a-file case and `open` follows `commondir`, without which a
    // worktree reads as a repository with no history.
    let Some(git_dir) = gitinfo::git_dir(&root) else {
        return Err(IndexError::NotADirectory(root.join(".git")));
    };
    let store = ObjectStore::open(&git_dir)?;

    let mut conn = nerve_store::open(&db_path)?;
    // Before a single row is written, and unconditionally. `nerve init` migrates, but a database
    // written by an older build and then synced by a newer one never passed through `init` again —
    // and the Slice 3b failure this prevents was not loud: `INSERT OR IGNORE` swallowed the
    // resulting `NOT NULL` violations *after* the old rows had been deleted, so the graph shrank and
    // the process exited zero.
    nerve_store::migrate(&conn)?;
    let Some(repository) = nerve_store::repository(&conn)? else {
        return Err(IndexError::NotInitialized(root));
    };
    let repo_id = repository.repo_id;

    let mut walker = Walker::new(&store);
    let budget = walker.clamp_budget(options.max_commits);
    let (shallow_read_cleanly, boundary) = walker.shallow_state();

    let commits_repaired = delete_commits_with_unavailable_parents(&mut conn, &repo_id)?;

    // ---- the walk ---------------------------------------------------------------------------
    let head = gitinfo::head_commit(&root).and_then(|hex| Oid::from_hex(&hex));
    let mut outcome = HistoryOutcome {
        root: root.clone(),
        git_dir: store.git_dir().to_path_buf(),
        head_oid: head.map(|oid| oid.to_hex()),
        walked_from: head.iter().map(Oid::to_hex).collect(),
        commit_budget: budget,
        commits_walked: 0,
        commits_recorded: 0,
        commits_already_present: 0,
        commits_repaired,
        changes_written: 0,
        renames_written: 0,
        walk_terminated_by: WalkTermination::Exhausted,
        shallow: store.limits().shallow.is_some(),
        shallow_boundary: boundary.iter().map(Oid::to_hex).collect(),
        promisor: store.limits().promisor,
        completeness: ParentCompleteness::ALL
            .into_iter()
            .map(|value| (value, 0))
            .collect(),
        enumeration: ChangesEnumerated::ALL
            .into_iter()
            .map(|value| (value, 0))
            .collect(),
        trees_read: 0,
        subtrees_skipped: 0,
        summaries_truncated: 0,
        refused: BTreeMap::new(),
        reader_version: READER_VERSION.to_string(),
        duration_ms: 0,
        status: RunStatus::Complete,
    };

    // Already recorded, read **after** the repair so a repaired commit is walked again. This is
    // also why the walk does not stop at the first recorded commit: a repaired one is reached
    // *through* recorded ones, so stopping there would leave it deleted and never re-recorded. What
    // is skipped for a recorded commit is the tree diff, which is the cost that matters.
    let recorded = nerve_store::recorded_commit_oids(&conn, &repo_id)?;

    let mut queue: VecDeque<Oid> = VecDeque::new();
    let mut visited: BTreeSet<Oid> = BTreeSet::new();
    if let Some(head) = head {
        queue.push_back(head);
        visited.insert(head);
    }

    let mut stopped_at_budget = false;
    let mut boundary_reached = false;
    let mut object_missing = false;
    let mut walk_refused = false;

    while let Some(oid) = queue.pop_front() {
        if outcome.commits_walked >= budget {
            stopped_at_budget = true;
            walker.count(form::COMMIT_BUDGET);
            break;
        }
        outcome.commits_walked += 1;

        let commit = match walker.read_commit(&oid) {
            CommitRead::Found(commit) => *commit,
            CommitRead::Absent => {
                // The tip itself, or a parent `contains` said was present and a delta chain then
                // could not reconstruct. Absent is not refused, and it is not a root either.
                object_missing = true;
                continue;
            }
            CommitRead::Refused => {
                walk_refused = true;
                continue;
            }
        };

        let (completeness, present_parents) =
            walker.classify(&oid, &commit, &boundary, shallow_read_cleanly);
        match completeness {
            ParentCompleteness::ShallowBoundary => boundary_reached = true,
            ParentCompleteness::ParentsMissing | ParentCompleteness::ParentsUnverifiable => {
                object_missing = true;
            }
            ParentCompleteness::Root | ParentCompleteness::ParentsAvailable => {}
        }

        // A boundary commit's parents are absent **by declaration**, so they are not queued and not
        // reported missing. Every other commit walks the parents that are actually there; a merge
        // with one present and one absent parent still walks the present side.
        if completeness != ParentCompleteness::ShallowBoundary {
            for parent in present_parents {
                if visited.insert(parent) {
                    queue.push_back(parent);
                }
            }
        }

        let commit_oid = oid.to_hex();
        if recorded.contains(&commit_oid) {
            outcome.commits_already_present += 1;
            continue;
        }

        let (enumeration, changes) = walker.enumerate(&commit_oid, &commit, completeness);
        let renames = rename_hypotheses(&commit_oid, &changes);
        let row = walker.commit_row(commit_oid, &commit, completeness, enumeration, options);

        match write_commit(&mut conn, &repo_id, &row, &changes, &renames)? {
            Some(written) => {
                outcome.commits_recorded += 1;
                outcome.changes_written += written.changes;
                outcome.renames_written += written.renames;
                *outcome.completeness.entry(completeness).or_insert(0) += 1;
                *outcome.enumeration.entry(enumeration).or_insert(0) += 1;
            }
            None => outcome.commits_already_present += 1,
        }
    }

    outcome.walk_terminated_by = if stopped_at_budget {
        // Nerve's own boundary wins, because it is the reason unvisited work remains. Whether the
        // repository is also shallow is a separate field rather than a competing value.
        WalkTermination::CommitBudget
    } else if walk_refused {
        WalkTermination::Refused
    } else if object_missing {
        // An undeclared absence is a fault and outranks a declared one: a repository that is both
        // shallow and holed must report the hole.
        WalkTermination::MissingObject
    } else if boundary_reached {
        WalkTermination::ShallowBoundary
    } else {
        WalkTermination::Exhausted
    };

    outcome.trees_read = walker.trees_read;
    outcome.subtrees_skipped = walker.subtrees_skipped;
    outcome.summaries_truncated = walker.summaries_truncated;
    outcome.refused = walker.into_refusals();
    if !outcome.refused.is_empty() {
        outcome.status = RunStatus::Partial;
    }

    nerve_store::upsert_history_ingest(
        &conn,
        &repo_id,
        &IngestRow {
            head_oid: outcome.head_oid.clone(),
            walked_from: outcome.walked_from.clone(),
            commits_recorded: nerve_store::history_totals(&conn, &repo_id)?.commits,
            commit_budget: i64::try_from(budget).unwrap_or(i64::MAX),
            walk_terminated_by: outcome.walk_terminated_by,
            shallow: outcome.shallow,
            shallow_boundary: outcome.shallow_boundary.clone(),
            promisor: outcome.promisor,
            refusals: outcome.refused.clone(),
            reader_version: outcome.reader_version.clone(),
        },
    )?;

    outcome.duration_ms = started.elapsed().as_millis();
    Ok(outcome)
}

/// Rows one commit contributed, so the caller does not have to add up two return values.
struct Written {
    changes: usize,
    renames: usize,
}

/// Delete every commit whose availability was a conclusion from **absence**, with its dependent rows.
///
/// `parent_completeness` and `changes_enumerated` are the two columns in `git_commit` that are *not*
/// properties of the commit object. They record what this repository could see when it was read, and
/// a `git fetch --unshallow` — or a fetch that fills a promisor hole — changes them. Because
/// [`nerve_store::insert_commit`] ignores a second insert, a former boundary commit would otherwise
/// keep `shallow_boundary` / `parent_unavailable` with **zero change rows forever**: availability
/// data that is now false, at exactly the boundary this slice exists to get right.
///
/// The rule is provable rather than heuristic. A commit classified by what was *missing* must be
/// re-examined; one classified by what was *present* need not be — `root` and `parents_available` are
/// conclusions from presence and cannot be improved by fetching, while the other three are
/// conclusions from absence and can. On a complete repository the set is empty, so an ordinary
/// re-sync pays one indexed count.
///
/// **These three statements belong in `nerve_store::history` beside the rest of the SQL**, as
/// `delete_commits_with_unavailable_parents`, and a future tidy should move them there. They are here
/// because this slice may not edit that crate. They are parameterised, and the two vocabulary values
/// are bound rather than interpolated so the set is defined by
/// [`ParentCompleteness`] rather than by a string in this file.
fn delete_commits_with_unavailable_parents(
    conn: &mut nerve_store::Connection,
    repo_id: &str,
) -> Result<usize> {
    let tx = conn.transaction().map_err(nerve_store::StoreError::from)?;
    let selection = "SELECT commit_oid FROM git_commit
                       WHERE repo_id = ?1 AND parent_completeness NOT IN (?2, ?3)";
    let keep = [
        repo_id,
        ParentCompleteness::Root.as_str(),
        ParentCompleteness::ParentsAvailable.as_str(),
    ];
    // Foreign-key order: the two dependent tables first, then the commits they point at.
    for table in ["git_rename_hypothesis", "git_change"] {
        tx.execute(
            &format!("DELETE FROM {table} WHERE repo_id = ?1 AND commit_oid IN ({selection})"),
            keep,
        )
        .map_err(nerve_store::StoreError::from)?;
    }
    let deleted = tx
        .execute(
            "DELETE FROM git_commit
               WHERE repo_id = ?1 AND parent_completeness NOT IN (?2, ?3)",
            keep,
        )
        .map_err(nerve_store::StoreError::from)?;
    tx.commit().map_err(nerve_store::StoreError::from)?;
    Ok(deleted)
}

/// Write one commit and its dependent rows in **one transaction**. `None` when it was already
/// recorded.
///
/// Nothing in the store layer can enforce the transaction, and the consequence of not having it is
/// exactly the ambiguity [`ChangesEnumerated`] exists to remove: a failure between the commit insert
/// and the change inserts would leave a commit claiming [`ChangesEnumerated::Enumerated`] with no
/// change rows, and the next sync **skips it**, because [`nerve_store::insert_commit`] now answers
/// `false`. The ambiguity would then be permanent and indistinguishable from a legitimately empty
/// commit.
///
/// So an error here rolls the commit row back with it, and the commit is re-read on the next sync.
fn write_commit(
    conn: &mut nerve_store::Connection,
    repo_id: &str,
    commit: &CommitRow,
    changes: &[ChangeRow],
    renames: &[RenameRow],
) -> Result<Option<Written>> {
    let tx = conn.transaction().map_err(nerve_store::StoreError::from)?;
    if !nerve_store::insert_commit(&tx, repo_id, commit)? {
        return Ok(None);
    }
    let written = Written {
        changes: nerve_store::insert_changes(&tx, repo_id, changes)?,
        renames: nerve_store::insert_renames(&tx, repo_id, renames)?,
    };
    tx.commit().map_err(nerve_store::StoreError::from)?;
    Ok(Some(written))
}

/// Three answers when reading a commit object, kept apart for the same reason
/// [`ObjectStore::read`] has three: *absent* is not *refused*, and neither is *here it is*.
enum CommitRead {
    /// The commit object was read and parsed.
    ///
    /// Boxed because a [`Commit`] carries five owned buffers and the two other answers carry
    /// nothing, so an unboxed payload would make every `Absent` cost a commit's worth of stack.
    Found(Box<Commit>),
    /// Not present in this store. Recorded as an absence on the child, never as a root.
    Absent,
    /// The store found something and refused it. Counted by form.
    Refused,
}

/// What a tree read produced. `Unreadable` covers absent and refused alike, because both have the
/// same consequence for a diff — it cannot run — while the *reason* has already been counted under
/// its own form.
enum TreeRead {
    /// The entries, in the order the tree lists them.
    Entries(Vec<TreeEntry>),
    /// The tree could not be read. Already counted.
    Unreadable,
}

/// The walk's mutable state: what it has read, skipped and refused.
struct Walker<'a> {
    store: &'a ObjectStore,
    refused: BTreeMap<String, usize>,
    trees_read: usize,
    subtrees_skipped: usize,
    summaries_truncated: usize,
}

impl<'a> Walker<'a> {
    fn new(store: &'a ObjectStore) -> Self {
        Self {
            store,
            refused: BTreeMap::new(),
            trees_read: 0,
            subtrees_skipped: 0,
            summaries_truncated: 0,
        }
    }

    fn count(&mut self, tag: &str) {
        *self.refused.entry(tag.to_string()).or_insert(0) += 1;
    }

    /// This module's refusals, merged with everything the object store refused.
    ///
    /// Merged rather than reported separately: a caller asking "what did this sync decline to
    /// believe" wants one answer, and the tags are disjoint by construction.
    fn into_refusals(mut self) -> BTreeMap<String, usize> {
        for (tag, hits) in self.store.counters().refused {
            *self.refused.entry(tag).or_insert(0) += hits;
        }
        self.refused
    }

    /// Apply [`MAX_HISTORY_COMMITS`], counting the clamp.
    fn clamp_budget(&mut self, requested: usize) -> usize {
        if requested > MAX_HISTORY_COMMITS {
            self.count(form::COMMIT_BUDGET);
            return MAX_HISTORY_COMMITS;
        }
        // Zero is a real answer — "walk nothing" — and is not clamped upwards into a surprise.
        requested
    }

    /// Whether the shallow declaration was read cleanly, and the boundary set.
    ///
    /// "Cleanly" is the negation of three conditions, all of which 12a reports and none of which it
    /// can resolve: a `shallow` line it could not parse, an entry bound it exceeded, and — the one
    /// that needs a filesystem call rather than a counter — a `shallow` file that exists while
    /// `StoreLimits::shallow` is `None`, which is what an unreadable or over-bound pointer file looks
    /// like from the outside. The last case is the only place this module looks at a `.git` path 12a
    /// has already looked at, and it is three lines rather than a change to the reader.
    fn shallow_state(&mut self) -> (bool, BTreeSet<Oid>) {
        let counters = self.store.counters();
        let dropped = counters.get(gitobj::form::SHALLOW_ENTRY_UNPARSED)
            + counters.get(gitobj::form::SHALLOW_ENTRIES_EXCEEDED);
        let declared = self.store.limits().shallow.clone();
        let file_exists = std::fs::metadata(self.store.common_dir().join("shallow"))
            .is_ok_and(|metadata| metadata.is_file());
        let undecidable = dropped > 0 || (declared.is_none() && file_exists);
        if undecidable {
            self.count(form::SHALLOW_UNVERIFIABLE);
        }
        (
            !undecidable,
            declared.unwrap_or_default().into_iter().collect(),
        )
    }

    fn read_commit(&mut self, oid: &Oid) -> CommitRead {
        let object = match self.store.read(oid) {
            Ok(Some(object)) => object,
            Ok(None) => return CommitRead::Absent,
            Err(error) => {
                self.count(error.form());
                return CommitRead::Refused;
            }
        };
        let Object::Commit(bytes) = object else {
            // The oid named something that is not a commit. Refused rather than reinterpreted: a
            // tree read as a commit would produce headers out of thin air.
            self.count(gitobj::form::COMMIT_HEADER_MALFORMED);
            return CommitRead::Refused;
        };
        match gitobj::parse_commit(&bytes) {
            Ok(commit) => CommitRead::Found(Box::new(commit)),
            Err(error) => {
                self.count(error.form());
                CommitRead::Refused
            }
        }
    }

    /// Which of the five parent situations this commit is in, and which parents may be walked.
    ///
    /// The order of the tests is the invariant. A declared boundary is decided **before** any parent
    /// is looked for, because Git grafts a boundary commit to have no parents: looking first would
    /// count its declared-absent parents as a fault, and finding them present — which a hand-written
    /// `shallow` file over a complete store does — would make the boundary look ordinary.
    fn classify(
        &mut self,
        oid: &Oid,
        commit: &Commit,
        boundary: &BTreeSet<Oid>,
        shallow_read_cleanly: bool,
    ) -> (ParentCompleteness, Vec<Oid>) {
        if boundary.contains(oid) {
            return (ParentCompleteness::ShallowBoundary, Vec::new());
        }
        if commit.parents.is_empty() {
            return (ParentCompleteness::Root, Vec::new());
        }

        let mut present = Vec::new();
        let mut absent = false;
        let mut undecidable = false;
        for parent in &commit.parents {
            match self.store.contains(parent) {
                Ok(true) => present.push(*parent),
                Ok(false) => absent = true,
                Err(error) => {
                    // Presence could not be established at all, which is weaker than "absent" and
                    // must not be reported as a fault.
                    self.count(error.form());
                    undecidable = true;
                }
            }
        }

        let completeness = if !absent && !undecidable {
            ParentCompleteness::ParentsAvailable
        } else if undecidable || !shallow_read_cleanly {
            ParentCompleteness::ParentsUnverifiable
        } else {
            ParentCompleteness::ParentsMissing
        };
        (completeness, present)
    }

    /// Whether this commit's changes can be enumerated, and them if so.
    ///
    /// The headline gate of the slice lives in the first arm. A `Root` is diffed against the empty
    /// tree and every path in it is `added`, which is correct **for a root**; the three availability
    /// values that also have no readable parent get zero rows and
    /// [`ChangesEnumerated::ParentUnavailable`] instead, because the parent tree was unreadable
    /// rather than empty.
    fn enumerate(
        &mut self,
        commit_oid: &str,
        commit: &Commit,
        completeness: ParentCompleteness,
    ) -> (ChangesEnumerated, Vec<ChangeRow>) {
        match completeness {
            ParentCompleteness::ShallowBoundary
            | ParentCompleteness::ParentsMissing
            | ParentCompleteness::ParentsUnverifiable => {
                // Availability is tested before the merge rule, so a boundary that is also a merge
                // reports the boundary. Nothing is lost: `is_merge` records the other fact.
                (ChangesEnumerated::ParentUnavailable, Vec::new())
            }
            ParentCompleteness::Root | ParentCompleteness::ParentsAvailable => {
                if commit.parents.len() > 1 {
                    // Enumeration is defined against a *single* parent. Diffing against parent 0
                    // double-counts every change already recorded on the branch, and diffing against
                    // every parent produces several conflicting kinds for one path that the primary
                    // key cannot hold. Enumerate nothing, and say so.
                    return (ChangesEnumerated::MergeNotEnumerated, Vec::new());
                }
                let parent_tree = match commit.parents.first() {
                    Some(parent) => match self.read_commit(parent) {
                        CommitRead::Found(parent) => Some(parent.tree),
                        // `contains` said the parent was there and reconstruction disagreed — a
                        // delta whose base is missing. The parent tree is unreadable, which is the
                        // same conclusion as an absent parent.
                        CommitRead::Absent => {
                            self.count(form::TREE_ABSENT);
                            return (ChangesEnumerated::ParentUnavailable, Vec::new());
                        }
                        // Already counted by form inside `read_commit`; counting an absence here as
                        // well would report one event twice.
                        CommitRead::Refused => {
                            return (ChangesEnumerated::ParentUnavailable, Vec::new())
                        }
                    },
                    None => None,
                };
                match self.diff(commit_oid, parent_tree, commit.tree) {
                    Some(changes) => (ChangesEnumerated::Enumerated, changes),
                    None => (ChangesEnumerated::Refused, Vec::new()),
                }
            }
        }
    }

    /// Diff two trees. `None` when a bound or a refusal stopped it — already counted.
    fn diff(
        &mut self,
        commit_oid: &str,
        old_tree: Option<Oid>,
        new_tree: Oid,
    ) -> Option<Vec<ChangeRow>> {
        let mut changes = Vec::new();
        let complete = self.walk_pair(commit_oid, old_tree, Some(new_tree), "", 1, &mut changes);
        complete.then_some(changes)
    }

    fn entries(&mut self, tree: Option<Oid>) -> TreeRead {
        let Some(oid) = tree else {
            // No tree on this side. The empty tree, which is what a root commit is diffed against
            // and what a deleted directory becomes.
            return TreeRead::Entries(Vec::new());
        };
        self.trees_read += 1;
        let object = match self.store.read(&oid) {
            Ok(Some(object)) => object,
            Ok(None) => {
                self.count(form::TREE_ABSENT);
                return TreeRead::Unreadable;
            }
            Err(error) => {
                self.count(error.form());
                return TreeRead::Unreadable;
            }
        };
        let Object::Tree(bytes) = object else {
            self.count(gitobj::form::TREE_ENTRY_MALFORMED);
            return TreeRead::Unreadable;
        };
        match gitobj::parse_tree(&bytes) {
            Ok(entries) if entries.len() > MAX_TREE_ENTRIES => {
                self.count(form::TREE_TOO_LARGE);
                TreeRead::Unreadable
            }
            Ok(entries) => TreeRead::Entries(entries),
            Err(error) => {
                self.count(error.form());
                TreeRead::Unreadable
            }
        }
    }

    /// One directory level of the diff. `false` on refusal.
    ///
    /// `depth` is the number of path segments the entries at this level will have, so the top level
    /// is `1`; it is what [`safe_tree_name`] bounds.
    fn walk_pair(
        &mut self,
        commit_oid: &str,
        old_tree: Option<Oid>,
        new_tree: Option<Oid>,
        prefix: &str,
        depth: usize,
        out: &mut Vec<ChangeRow>,
    ) -> bool {
        // Sequential rather than a tuple, so an unreadable old tree is not paid for twice: the new
        // side is never read, and the refusal count stays one per unreadable tree.
        let TreeRead::Entries(old_entries) = self.entries(old_tree) else {
            return false;
        };
        let TreeRead::Entries(new_entries) = self.entries(new_tree) else {
            return false;
        };
        let old_by_name = self.index_by_name(&old_entries);
        let new_by_name = self.index_by_name(&new_entries);

        let names: BTreeSet<&[u8]> = old_by_name
            .keys()
            .chain(new_by_name.keys())
            .copied()
            .collect();

        for name in names {
            let text = match safe_tree_name(name, depth) {
                Ok(text) => text,
                Err(refusal) => {
                    // Counted, never dropped silently, and never echoed: the name is hostile text
                    // by assumption.
                    self.count(refusal.form());
                    continue;
                }
            };
            let path = if prefix.is_empty() {
                text.to_string()
            } else {
                format!("{prefix}/{text}")
            };
            let old = old_by_name.get(name).copied();
            let new = new_by_name.get(name).copied();
            if !self.walk_entry(commit_oid, old, new, &path, depth, out) {
                return false;
            }
            if out.len() > MAX_CHANGES_PER_COMMIT {
                self.count(form::CHANGES_TOO_MANY);
                return false;
            }
        }
        true
    }

    /// One name, present on either or both sides.
    fn walk_entry(
        &mut self,
        commit_oid: &str,
        old: Option<&TreeEntry>,
        new: Option<&TreeEntry>,
        path: &str,
        depth: usize,
        out: &mut Vec<ChangeRow>,
    ) -> bool {
        let old_is_tree = old.is_some_and(TreeEntry::is_tree);
        let new_is_tree = new.is_some_and(TreeEntry::is_tree);

        if old_is_tree && new_is_tree {
            let (old, new) = (old.expect("a tree"), new.expect("a tree"));
            if old.oid == new.oid {
                // **The property the measured delta cost rests on.** An unchanged directory costs
                // one oid comparison regardless of how many paths are under it, and neither tree
                // object is read.
                self.subtrees_skipped += 1;
                return true;
            }
            return self.walk_pair(
                commit_oid,
                Some(old.oid),
                Some(new.oid),
                path,
                depth + 1,
                out,
            );
        }

        // A directory replaced by a file, or the reverse. Everything under the directory side is
        // enumerated, and the file side is an ordinary addition or deletion at the same path — which
        // cannot collide, because nothing was at exactly that path on the directory side.
        if old_is_tree {
            let old = old.expect("a tree");
            if !self.walk_pair(commit_oid, Some(old.oid), None, path, depth + 1, out) {
                return false;
            }
            if let Some(new) = new {
                out.push(added(commit_oid, path, new));
            }
            return true;
        }
        if new_is_tree {
            let new = new.expect("a tree");
            if let Some(old) = old {
                out.push(deleted(commit_oid, path, old));
            }
            return self.walk_pair(commit_oid, None, Some(new.oid), path, depth + 1, out);
        }

        // Two leaves. A gitlink (`160000`) is one of them and is **never followed**: the commit it
        // names belongs to another repository, so the change is recorded against the gitlink path and
        // nothing is read.
        match (old, new) {
            (None, None) => true,
            (None, Some(new)) => {
                out.push(added(commit_oid, path, new));
                true
            }
            (Some(old), None) => {
                out.push(deleted(commit_oid, path, old));
                true
            }
            (Some(old), Some(new)) => {
                if old.oid != new.oid {
                    out.push(ChangeRow {
                        commit_oid: commit_oid.to_string(),
                        path: path.to_string(),
                        change_kind: ChangeKind::Modified,
                        blob_oid: Some(new.oid.to_hex()),
                        prev_blob_oid: Some(old.oid.to_hex()),
                        mode: Some(i64::from(new.mode)),
                        prev_mode: Some(i64::from(old.mode)),
                    });
                } else if old.mode != new.mode {
                    // Identical bytes, different mode. Reporting this as `modified` would claim a
                    // content change that did not happen; reporting nothing would say the commit
                    // touched no path.
                    out.push(ChangeRow {
                        commit_oid: commit_oid.to_string(),
                        path: path.to_string(),
                        change_kind: ChangeKind::ModeChanged,
                        blob_oid: Some(new.oid.to_hex()),
                        prev_blob_oid: Some(old.oid.to_hex()),
                        mode: Some(i64::from(new.mode)),
                        prev_mode: Some(i64::from(old.mode)),
                    });
                }
                true
            }
        }
    }

    /// Index one tree's entries by name, refusing a repeated name.
    ///
    /// Git refuses a tree with duplicate entries and this reader does not verify object content, so a
    /// duplicate is possible and has to be decided. The first entry is kept and the second refused:
    /// overwriting would drop a change silently, and keeping both would collide on
    /// `git_change`'s primary key mid-transaction.
    fn index_by_name<'e>(&mut self, entries: &'e [TreeEntry]) -> BTreeMap<&'e [u8], &'e TreeEntry> {
        let mut out: BTreeMap<&[u8], &TreeEntry> = BTreeMap::new();
        for entry in entries {
            if out.contains_key(entry.name.as_slice()) {
                self.count(form::DUPLICATE_PATH);
                continue;
            }
            out.insert(entry.name.as_slice(), entry);
        }
        out
    }

    fn commit_row(
        &mut self,
        commit_oid: String,
        commit: &Commit,
        completeness: ParentCompleteness,
        enumeration: ChangesEnumerated,
        options: &HistoryOptions,
    ) -> CommitRow {
        let (summary, summary_truncation) = self.summary(&commit.message);
        CommitRow {
            commit_oid,
            tree_oid: commit.tree.to_hex(),
            parent_oids: commit.parents.iter().map(Oid::to_hex).collect(),
            parent_completeness: completeness,
            changes_enumerated: enumeration,
            author_time: commit.author.timestamp,
            author_tz: commit.author.timezone.clone(),
            committer_time: commit.committer.timestamp,
            committer_tz: commit.committer.timezone.clone(),
            author_ident: options.with_identity.then(|| self.identity(&commit.author)),
            committer_ident: options
                .with_identity
                .then(|| self.identity(&commit.committer)),
            summary,
            summary_truncation,
            is_merge: commit.parents.len() > 1,
        }
    }

    /// The commit summary: first line, bounded, lossy, never interpreted — and whether it was cut.
    ///
    /// The order matters. Lossy conversion happens **before** the bound, because a replacement
    /// character is three bytes where the invalid one was one, so bounding first would let a summary
    /// grow past the bound on its way into the database. Truncation is counted, because a consumer
    /// cannot otherwise tell a short summary from a cut one.
    ///
    /// **The per-record verdict is decided here, where the untruncated length is still in hand, and
    /// is never [`SummaryTruncation::Unknown`].** `Unknown` is what the v6→v7 migration wrote for
    /// commits recorded before the column existed; a fresh write knows the answer, and returning
    /// `Unknown` from a writer that measured the length would be manufacturing an absence. The
    /// comparison is `>` rather than `>=`, so a first line of exactly [`MAX_SUMMARY_BYTES`] is
    /// [`SummaryTruncation::Complete`] — the boundary case a length-based reconstruction would get
    /// wrong, which is why the column exists at all.
    fn summary(&mut self, message: &[u8]) -> (String, SummaryTruncation) {
        let first_line = message
            .split(|byte| *byte == b'\n')
            .next()
            .unwrap_or_default();
        let text = String::from_utf8_lossy(first_line).into_owned();
        let truncation = if text.len() > MAX_SUMMARY_BYTES {
            self.summaries_truncated += 1;
            SummaryTruncation::Truncated
        } else {
            SummaryTruncation::Complete
        };
        (
            self.bound(text, MAX_SUMMARY_BYTES, form::SUMMARY_TRUNCATED),
            truncation,
        )
    }

    fn identity(&mut self, identity: &Identity) -> String {
        let text = format!(
            "{} <{}>",
            String::from_utf8_lossy(&identity.name),
            String::from_utf8_lossy(&identity.email)
        );
        self.bound(text, MAX_IDENT_BYTES, form::IDENT_TRUNCATED)
    }

    /// Truncate `text` to at most `limit` bytes on a character boundary, counting the truncation.
    fn bound(&mut self, text: String, limit: usize, tag: &'static str) -> String {
        if text.len() <= limit {
            return text;
        }
        self.count(tag);
        let mut end = limit;
        while end > 0 && !text.is_char_boundary(end) {
            end -= 1;
        }
        text[..end].to_string()
    }
}

fn added(commit_oid: &str, path: &str, entry: &TreeEntry) -> ChangeRow {
    ChangeRow {
        commit_oid: commit_oid.to_string(),
        path: path.to_string(),
        change_kind: ChangeKind::Added,
        blob_oid: Some(entry.oid.to_hex()),
        prev_blob_oid: None,
        mode: Some(i64::from(entry.mode)),
        prev_mode: None,
    }
}

fn deleted(commit_oid: &str, path: &str, entry: &TreeEntry) -> ChangeRow {
    ChangeRow {
        commit_oid: commit_oid.to_string(),
        path: path.to_string(),
        change_kind: ChangeKind::Deleted,
        blob_oid: None,
        prev_blob_oid: Some(entry.oid.to_hex()),
        mode: None,
        prev_mode: Some(i64::from(entry.mode)),
    }
}

/// Rename hypotheses for one commit, from exact blob identity and nothing else.
///
/// Within one commit only: a path deleted here and added there with **the same blob oid** is a
/// hypothesis with evidence `exact_content`. The oids are already in hand from the diff, so there is
/// no similarity computation, no threshold, no tie-break and no score.
///
/// **Every pairing is recorded and none is promoted.** Files with identical content — an empty file,
/// a copied licence header, a re-exported barrel — split and merge constantly, so when one deleted
/// blob matches several added paths the answer is several rows carrying
/// [`RenameAmbiguity::ManyTo`], not a winner. Ambiguous identity stays ambiguous.
fn rename_hypotheses(commit_oid: &str, changes: &[ChangeRow]) -> Vec<RenameRow> {
    let mut deleted_by_blob: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
    let mut added_by_blob: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
    for change in changes {
        match change.change_kind {
            ChangeKind::Deleted => {
                if let Some(blob) = change.prev_blob_oid.as_deref() {
                    deleted_by_blob.entry(blob).or_default().push(&change.path);
                }
            }
            ChangeKind::Added => {
                if let Some(blob) = change.blob_oid.as_deref() {
                    added_by_blob.entry(blob).or_default().push(&change.path);
                }
            }
            ChangeKind::Modified | ChangeKind::ModeChanged => {}
        }
    }

    let mut out = Vec::new();
    for (blob, from_paths) in &deleted_by_blob {
        let Some(to_paths) = added_by_blob.get(blob) else {
            continue;
        };
        let ambiguity = match (from_paths.len(), to_paths.len()) {
            (1, 1) => RenameAmbiguity::Unique,
            (1, _) => RenameAmbiguity::ManyTo,
            (_, 1) => RenameAmbiguity::ManyFrom,
            _ => RenameAmbiguity::ManyBoth,
        };
        for from_path in from_paths {
            for to_path in to_paths {
                out.push(RenameRow {
                    commit_oid: commit_oid.to_string(),
                    from_path: (*from_path).to_string(),
                    to_path: (*to_path).to_string(),
                    evidence: RenameEvidence::ExactContent,
                    // One blob, named twice. The identity of the two oids *is* the evidence, and
                    // the schema's `CHECK` requires it of an `exact_content` row.
                    from_blob_oid: (*blob).to_string(),
                    to_blob_oid: (*blob).to_string(),
                    matcher_id: EXACT_MATCHER_ID.to_string(),
                    matcher_version: EXACT_MATCHER_VERSION.to_string(),
                    // No measurement, rather than a perfect one. An exact match computes no
                    // similarity, so a `1/1` here would be a number this matcher never counted.
                    match_numerator: None,
                    match_denominator: None,
                    ambiguity,
                });
            }
        }
    }
    out.sort_by(|left, right| {
        (&left.from_path, &left.to_path).cmp(&(&right.from_path, &right.to_path))
    });
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn change(kind: ChangeKind, path: &str, blob: &str) -> ChangeRow {
        let (blob_oid, prev_blob_oid) = match kind {
            ChangeKind::Deleted => (None, Some(blob.to_string())),
            _ => (Some(blob.to_string()), None),
        };
        ChangeRow {
            commit_oid: "c".to_string(),
            path: path.to_string(),
            change_kind: kind,
            blob_oid,
            prev_blob_oid,
            mode: None,
            prev_mode: None,
        }
    }

    #[test]
    fn every_form_tag_is_distinct_and_non_empty() {
        let mut tags = form::ALL.to_vec();
        tags.sort_unstable();
        tags.dedup();
        assert_eq!(tags.len(), form::ALL.len(), "two form tags collide");
        assert!(tags.iter().all(|tag| tag.starts_with("history-")));
    }

    /// The four ambiguity shapes, and the rule that none of them promotes a pairing.
    #[test]
    fn every_pairing_is_recorded_and_none_is_promoted() {
        let unique = rename_hypotheses(
            "c",
            &[
                change(ChangeKind::Deleted, "old.txt", "b1"),
                change(ChangeKind::Added, "new.txt", "b1"),
            ],
        );
        assert_eq!(unique.len(), 1);
        assert_eq!(unique[0].ambiguity, RenameAmbiguity::Unique);
        assert_eq!(unique[0].evidence, RenameEvidence::ExactContent);

        let many_to = rename_hypotheses(
            "c",
            &[
                change(ChangeKind::Deleted, "src.txt", "b1"),
                change(ChangeKind::Added, "a.txt", "b1"),
                change(ChangeKind::Added, "b.txt", "b1"),
            ],
        );
        assert_eq!(many_to.len(), 2);
        assert!(many_to
            .iter()
            .all(|row| row.ambiguity == RenameAmbiguity::ManyTo));

        let many_from = rename_hypotheses(
            "c",
            &[
                change(ChangeKind::Deleted, "a.txt", "b1"),
                change(ChangeKind::Deleted, "b.txt", "b1"),
                change(ChangeKind::Added, "one.txt", "b1"),
            ],
        );
        assert_eq!(many_from.len(), 2);
        assert!(many_from
            .iter()
            .all(|row| row.ambiguity == RenameAmbiguity::ManyFrom));

        let many_both = rename_hypotheses(
            "c",
            &[
                change(ChangeKind::Deleted, "a.txt", "b1"),
                change(ChangeKind::Deleted, "b.txt", "b1"),
                change(ChangeKind::Added, "c.txt", "b1"),
                change(ChangeKind::Added, "d.txt", "b1"),
            ],
        );
        assert_eq!(many_both.len(), 4, "2x2 is four pairings, not one winner");
        assert!(many_both
            .iter()
            .all(|row| row.ambiguity == RenameAmbiguity::ManyBoth));
    }

    /// A modified path is not a rename candidate, and a blob on only one side pairs with nothing.
    #[test]
    fn only_a_deletion_paired_with_an_addition_is_a_hypothesis() {
        assert!(rename_hypotheses(
            "c",
            &[
                change(ChangeKind::Modified, "a.txt", "b1"),
                change(ChangeKind::Added, "b.txt", "b1"),
            ]
        )
        .is_empty());
        assert!(rename_hypotheses("c", &[change(ChangeKind::Deleted, "a.txt", "b1")]).is_empty());
        assert!(rename_hypotheses(
            "c",
            &[
                change(ChangeKind::Deleted, "a.txt", "b1"),
                change(ChangeKind::Added, "b.txt", "b2"),
            ]
        )
        .is_empty());
    }

    /// The bound is on the stored string, and truncation lands on a character boundary rather than
    /// mid-codepoint.
    #[test]
    fn an_over_long_summary_is_truncated_on_a_character_boundary_and_counted() {
        let store_dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(store_dir.path().join("objects")).unwrap();
        let store = ObjectStore::open(store_dir.path()).unwrap();
        let mut walker = Walker::new(&store);

        let (short, truncation) = walker.summary(b"first line\nsecond line\n");
        assert_eq!(short, "first line");
        assert_eq!(truncation, SummaryTruncation::Complete);
        assert_eq!(walker.summaries_truncated, 0);

        // Three-byte characters, so a naive slice at 512 would land inside one.
        let wide = "\u{4e00}".repeat(400);
        let (bounded, truncation) = walker.summary(wide.as_bytes());
        assert!(bounded.len() <= MAX_SUMMARY_BYTES);
        assert_eq!(bounded.len(), 510, "512 is not a multiple of three");
        assert!(wide.starts_with(&bounded));
        assert_eq!(truncation, SummaryTruncation::Truncated);
        assert_eq!(walker.summaries_truncated, 1);
        assert_eq!(walker.refused.get(form::SUMMARY_TRUNCATED), Some(&1));
    }

    /// **The boundary case the column exists for.** Exactly `MAX_SUMMARY_BYTES` is *not* truncated.
    ///
    /// This is why `summary_truncation` is stored rather than reconstructed from the stored length:
    /// `length(summary) = MAX_SUMMARY_BYTES ⟹ truncated` would call this summary cut when nothing
    /// was cut from it, and one byte more is where the answer genuinely changes. Neither answer is
    /// [`SummaryTruncation::Unknown`] — a writer that measured the length knows.
    #[test]
    fn a_summary_of_exactly_the_bound_is_complete_and_one_byte_more_is_truncated() {
        let store_dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(store_dir.path().join("objects")).unwrap();
        let store = ObjectStore::open(store_dir.path()).unwrap();
        let mut walker = Walker::new(&store);

        let exact = "a".repeat(MAX_SUMMARY_BYTES);
        let (stored, truncation) = walker.summary(exact.as_bytes());
        assert_eq!(stored.len(), MAX_SUMMARY_BYTES);
        assert_eq!(stored, exact, "nothing was cut, so nothing may be dropped");
        assert_eq!(truncation, SummaryTruncation::Complete);
        assert_eq!(walker.summaries_truncated, 0);

        let over = "a".repeat(MAX_SUMMARY_BYTES + 1);
        let (stored, truncation) = walker.summary(over.as_bytes());
        assert_eq!(stored.len(), MAX_SUMMARY_BYTES);
        assert_eq!(truncation, SummaryTruncation::Truncated);
        assert_eq!(walker.summaries_truncated, 1);

        // And a short one, so the test is a measurement rather than a demonstration that the
        // function returns `Truncated` for everything at or above the bound.
        let (stored, truncation) = walker.summary(b"short");
        assert_eq!(stored, "short");
        assert_eq!(truncation, SummaryTruncation::Complete);
        assert_eq!(
            walker.summaries_truncated, 1,
            "unchanged by a short summary"
        );
    }

    /// Invalid UTF-8 is replaced before the bound is applied, because a replacement character is
    /// three bytes where the invalid one was one.
    #[test]
    fn a_summary_that_is_not_utf8_is_converted_before_it_is_bounded() {
        let store_dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(store_dir.path().join("objects")).unwrap();
        let store = ObjectStore::open(store_dir.path()).unwrap();
        let mut walker = Walker::new(&store);

        let (bounded, truncation) = walker.summary(&[0xff; 200]);
        assert!(bounded.len() <= MAX_SUMMARY_BYTES);
        assert_eq!(truncation, SummaryTruncation::Truncated);
        assert_eq!(walker.summaries_truncated, 1, "200 invalid bytes is 600");
    }

    fn migrated_database() -> (nerve_store::Connection, String) {
        let conn = nerve_store::open_in_memory().unwrap();
        nerve_store::migrate(&conn).unwrap();
        let repo_id = nerve_core::ids::repository_id("0123456789abcdef0123456789abcdef");
        nerve_store::upsert_repository(
            &conn,
            &nerve_store::RepositoryRow {
                repo_id: repo_id.clone(),
                project_id: "0123456789abcdef0123456789abcdef".to_string(),
                root_path: "/nowhere".to_string(),
            },
        )
        .unwrap();
        (conn, repo_id)
    }

    fn commit_row(oid: &str, enumeration: ChangesEnumerated) -> CommitRow {
        CommitRow {
            commit_oid: oid.to_string(),
            tree_oid: "1".repeat(40),
            parent_oids: Vec::new(),
            parent_completeness: ParentCompleteness::Root,
            changes_enumerated: enumeration,
            author_time: 1,
            author_tz: "+0000".to_string(),
            committer_time: 1,
            committer_tz: "+0000".to_string(),
            author_ident: None,
            committer_ident: None,
            summary: "s".to_string(),
            summary_truncation: SummaryTruncation::Complete,
            is_merge: false,
        }
    }

    /// **The §8.5.2 property, tested directly.** A change insert that fails mid-commit must take the
    /// commit row with it, or the next sync skips a commit that claims `enumerated` and has no rows.
    ///
    /// The injected fault is a foreign-key violation: the second change row names a commit that is
    /// not recorded. `PRAGMA foreign_keys=ON` is set by `nerve_store::open`, so the row is refused
    /// rather than orphaned — and because `insert_changes` is a plain `INSERT` rather than
    /// `INSERT OR IGNORE`, the refusal is an error rather than a silently dropped row.
    #[test]
    fn a_change_insert_that_fails_mid_commit_leaves_no_commit_row() {
        let (mut conn, repo_id) = migrated_database();
        let oid = "a".repeat(40);
        let good = ChangeRow {
            commit_oid: oid.clone(),
            path: "kept.txt".to_string(),
            change_kind: ChangeKind::Added,
            blob_oid: Some("b".repeat(40)),
            prev_blob_oid: None,
            mode: Some(0o100_644),
            prev_mode: None,
        };
        let orphan = ChangeRow {
            commit_oid: "c".repeat(40),
            ..good.clone()
        };

        // First, the same call with sound rows must succeed — otherwise the failure below could be
        // anything at all.
        let written = write_commit(
            &mut conn,
            &repo_id,
            &commit_row(&oid, ChangesEnumerated::Enumerated),
            std::slice::from_ref(&good),
            &[],
        )
        .unwrap()
        .expect("a fresh commit is written");
        assert_eq!(written.changes, 1);
        assert_eq!(
            nerve_store::history_totals(&conn, &repo_id)
                .unwrap()
                .commits,
            1
        );

        let second = "d".repeat(40);
        let result = write_commit(
            &mut conn,
            &repo_id,
            &commit_row(&second, ChangesEnumerated::Enumerated),
            &[
                ChangeRow {
                    commit_oid: second.clone(),
                    ..good.clone()
                },
                orphan,
            ],
            &[],
        );
        assert!(result.is_err(), "an orphan change row must be refused");

        let totals = nerve_store::history_totals(&conn, &repo_id).unwrap();
        assert_eq!(
            totals.commits, 1,
            "the failed commit must not be recorded, or the next sync skips it"
        );
        assert_eq!(totals.changes, 1, "and its changes must be gone with it");
        assert!(!nerve_store::recorded_commit_oids(&conn, &repo_id)
            .unwrap()
            .contains(&second));
    }

    /// The repair deletes exactly the three availability values that are conclusions from absence,
    /// with their dependent rows, and leaves the two that are conclusions from presence.
    #[test]
    fn the_repair_deletes_only_commits_classified_by_an_absence() {
        let (mut conn, repo_id) = migrated_database();
        let oids: BTreeMap<ParentCompleteness, String> = ParentCompleteness::ALL
            .into_iter()
            .enumerate()
            .map(|(index, value)| (value, format!("{index:040x}")))
            .collect();
        for (value, oid) in &oids {
            let mut row = commit_row(oid, ChangesEnumerated::Enumerated);
            row.parent_completeness = *value;
            write_commit(
                &mut conn,
                &repo_id,
                &row,
                &[ChangeRow {
                    commit_oid: oid.clone(),
                    path: "p.txt".to_string(),
                    change_kind: ChangeKind::Added,
                    blob_oid: Some("b".repeat(40)),
                    prev_blob_oid: None,
                    mode: Some(0o100_644),
                    prev_mode: None,
                }],
                &[RenameRow {
                    commit_oid: oid.clone(),
                    from_path: "old.txt".to_string(),
                    to_path: "p.txt".to_string(),
                    evidence: RenameEvidence::ExactContent,
                    from_blob_oid: "b".repeat(40),
                    to_blob_oid: "b".repeat(40),
                    matcher_id: EXACT_MATCHER_ID.to_string(),
                    matcher_version: EXACT_MATCHER_VERSION.to_string(),
                    match_numerator: None,
                    match_denominator: None,
                    ambiguity: RenameAmbiguity::Unique,
                }],
            )
            .unwrap()
            .expect("written");
        }
        let before = nerve_store::history_totals(&conn, &repo_id).unwrap();
        assert_eq!(before.commits, 5, "five values, five commits");
        assert_eq!(before.changes, 5);
        assert_eq!(before.renames, 5);

        let deleted = delete_commits_with_unavailable_parents(&mut conn, &repo_id).unwrap();
        assert_eq!(
            deleted, 3,
            "shallow_boundary, parents_missing, unverifiable"
        );

        let after = nerve_store::history_totals(&conn, &repo_id).unwrap();
        assert_eq!(after.commits, 2);
        assert_eq!(after.changes, 2, "dependent rows go with the commit");
        assert_eq!(after.renames, 2);
        let surviving = nerve_store::recorded_commit_oids(&conn, &repo_id).unwrap();
        assert!(surviving.contains(&oids[&ParentCompleteness::Root]));
        assert!(surviving.contains(&oids[&ParentCompleteness::ParentsAvailable]));
        assert!(!surviving.contains(&oids[&ParentCompleteness::ShallowBoundary]));

        // Idempotent: a second repair on a repaired database deletes nothing.
        assert_eq!(
            delete_commits_with_unavailable_parents(&mut conn, &repo_id).unwrap(),
            0
        );
    }

    #[test]
    fn a_budget_over_the_bound_is_clamped_and_counted() {
        let store_dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(store_dir.path().join("objects")).unwrap();
        let store = ObjectStore::open(store_dir.path()).unwrap();
        let mut walker = Walker::new(&store);

        assert_eq!(walker.clamp_budget(10), 10);
        assert_eq!(walker.refused.get(form::COMMIT_BUDGET), None);
        assert_eq!(
            walker.clamp_budget(MAX_HISTORY_COMMITS + 1),
            MAX_HISTORY_COMMITS
        );
        assert_eq!(walker.refused.get(form::COMMIT_BUDGET), Some(&1));
        // Exactly at the bound is inside it.
        assert_eq!(
            walker.clamp_budget(MAX_HISTORY_COMMITS),
            MAX_HISTORY_COMMITS
        );
        assert_eq!(walker.refused.get(form::COMMIT_BUDGET), Some(&1));
    }
}
