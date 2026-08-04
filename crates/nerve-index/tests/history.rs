//! The measured gate for the history ingester, over `fixtures/history-*`.
//!
//! Every per-commit assertion here is read out of the fixture's `inventory.json`, which is **Git's**
//! answer for that fixture's committed bytes — `git cat-file commit`, `git diff-tree --raw -z
//! --no-renames`, `git cat-file -e`, `git ls-tree`. So this suite cannot pass by Nerve's reader
//! agreeing with itself, which is the failure mode a hand-written expectation invites.
//!
//! # What is asserted, and what would make an assertion vacuous
//!
//! A test that asserts an **absence** here also asserts a nonzero tally, because "zero `added` rows
//! at the shallow boundary" is satisfied both by the code being right and by the boundary never being
//! reached. Where a per-case assertion belongs, there is a per-case assertion: an aggregate threshold
//! over a whole fixture set was an entire corrective slice on this project, because "six distinct
//! refusal forms across the set" was satisfied by the working attacks alone while four fixtures
//! attacked nothing.
//!
//! # Three cases no committed fixture can hold
//!
//! The tree-entry bound needs 100 001 entries, the depth bound needs 65 nested trees, and the path
//! guard needs entry names that `parse_tree` lets *through* — and `fixtures/history-hostile` puts its
//! hostile names in the same tree as a subtree called `..`, which the format reader refuses whole, so
//! those names never reach the guard. Those three are built here as real object bytes. They are
//! possible without a hash implementation because the reader deliberately does not verify content
//! against its object id, which is a non-check 12a states on `StoreLimits`.

mod common;

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use nerve_core::vocab::{
    ChangeKind, ChangesEnumerated, ParentCompleteness, RenameAmbiguity, RenameEvidence,
    WalkTermination,
};
use nerve_index::gitobj::{self, ObjectStore, Oid};
use nerve_index::history::{self, ingest_history, HistoryOptions, HistoryOutcome};
use nerve_store::{ChangeRow, CommitRow};

// ---- helpers ----------------------------------------------------------------------------------

fn ingest(root: &Path, options: &HistoryOptions) -> HistoryOutcome {
    ingest_history(root, options).expect("history ingest must succeed")
}

fn repo_id(root: &Path) -> String {
    let conn = common::open_db(root);
    nerve_store::repository(&conn)
        .unwrap()
        .expect("init records the repository row")
        .repo_id
}

/// Every recorded commit, keyed by oid, read back out of the database.
fn recorded_commits(root: &Path) -> BTreeMap<String, CommitRow> {
    let conn = common::open_db(root);
    let repo = nerve_store::repository(&conn).unwrap().unwrap().repo_id;
    nerve_store::commit_log(&conn, &repo, usize::MAX, 0)
        .unwrap()
        .into_iter()
        .map(|row| (row.commit_oid.clone(), row))
        .collect()
}

fn changes_of(root: &Path, commit_oid: &str) -> Vec<ChangeRow> {
    let conn = common::open_db(root);
    let repo = nerve_store::repository(&conn).unwrap().unwrap().repo_id;
    nerve_store::changes_for_commit(&conn, &repo, commit_oid).unwrap()
}

fn totals(root: &Path) -> nerve_store::HistoryTotals {
    let conn = common::open_db(root);
    let repo = nerve_store::repository(&conn).unwrap().unwrap().repo_id;
    nerve_store::history_totals(&conn, &repo).unwrap()
}

fn ingest_row(root: &Path) -> nerve_store::IngestRow {
    let conn = common::open_db(root);
    let repo = nerve_store::repository(&conn).unwrap().unwrap().repo_id;
    nerve_store::history_ingest(&conn, &repo)
        .unwrap()
        .expect("a sync records one ingest row")
}

fn json_strings(value: &serde_json::Value) -> Vec<String> {
    value
        .as_array()
        .expect("a JSON array")
        .iter()
        .map(|item| item.as_str().expect("a JSON string").to_string())
        .collect()
}

fn octal(value: &serde_json::Value) -> Option<i64> {
    value
        .as_str()
        .map(|text| i64::from_str_radix(text, 8).expect("an octal mode"))
}

/// Compare one commit's stored changes against Git's own `diff-tree` answer, field by field.
fn assert_changes_match(commit_oid: &str, expected: &serde_json::Value, actual: &[ChangeRow]) {
    let expected = expected.as_array().expect("an array of changes");
    let mut actual: Vec<&ChangeRow> = actual.iter().collect();
    actual.sort_by(|left, right| left.path.cmp(&right.path));
    let mut expected: Vec<&serde_json::Value> = expected.iter().collect();
    expected.sort_by_key(|change| change["path"].as_str().unwrap_or_default());

    assert_eq!(
        actual.len(),
        expected.len(),
        "commit {commit_oid}: stored {:?} against Git's {:?}",
        actual.iter().map(|row| &row.path).collect::<Vec<_>>(),
        expected
            .iter()
            .map(|change| change["path"].as_str().unwrap_or_default())
            .collect::<Vec<_>>()
    );
    for (row, change) in actual.iter().zip(expected.iter()) {
        let path = change["path"].as_str().expect("a path");
        assert_eq!(row.path, path, "commit {commit_oid}");
        assert_eq!(
            row.change_kind.as_str(),
            change["change_kind"].as_str().expect("a change kind"),
            "commit {commit_oid}, path {path}"
        );
        assert_eq!(
            row.blob_oid.as_deref(),
            change["blob_oid"].as_str(),
            "commit {commit_oid}, path {path}"
        );
        assert_eq!(
            row.prev_blob_oid.as_deref(),
            change["prev_blob_oid"].as_str(),
            "commit {commit_oid}, path {path}"
        );
        assert_eq!(
            row.mode,
            octal(&change["mode_octal"]),
            "commit {commit_oid}, path {path}"
        );
        assert_eq!(
            row.prev_mode,
            octal(&change["prev_mode_octal"]),
            "commit {commit_oid}, path {path}"
        );
    }
}

fn open_store(root: &Path) -> ObjectStore {
    ObjectStore::open(&nerve_index::gitinfo::git_dir(root).expect("a git directory"))
        .expect("the fixture opens")
}

/// Every blob path reachable from a tree, as Git's `ls-tree -r` would list them.
fn tree_paths(store: &ObjectStore, tree: Oid, prefix: &str, out: &mut BTreeSet<String>) {
    let object = store
        .read(&tree)
        .expect("a readable tree")
        .expect("a present tree");
    let gitobj::Object::Tree(bytes) = object else {
        panic!("{tree} is not a tree");
    };
    for entry in gitobj::parse_tree(&bytes).expect("a well-formed tree") {
        let name = String::from_utf8(entry.name.clone()).expect("a UTF-8 name");
        let path = if prefix.is_empty() {
            name
        } else {
            format!("{prefix}/{name}")
        };
        if entry.is_tree() {
            tree_paths(store, entry.oid, &path, out);
        } else {
            out.insert(path);
        }
    }
}

fn object_path(git_dir: &Path, oid: &str) -> PathBuf {
    git_dir.join("objects").join(&oid[..2]).join(&oid[2..])
}

// ---- history-basic ----------------------------------------------------------------------------

/// Criterion 1. Every commit, parent, time, timezone and summary against Git's own answer.
#[test]
fn every_commit_in_history_basic_matches_gits_own_answer() {
    let (_dir, root) = common::history_fixture("history-basic");
    let outcome = ingest(&root, &HistoryOptions::default());
    let inventory = common::history_inventory("history-basic");
    let expected = inventory["commits"].as_array().expect("commits");

    assert_eq!(outcome.commits_recorded, expected.len());
    assert_eq!(outcome.commits_already_present, 0);
    assert_eq!(outcome.head_oid.as_deref(), inventory["head_oid"].as_str());
    assert_eq!(outcome.walk_terminated_by, WalkTermination::Exhausted);
    assert!(!outcome.shallow);
    assert!(outcome.shallow_boundary.is_empty());
    assert!(!outcome.promisor);
    assert!(
        outcome.refused.is_empty(),
        "an ordinary repository refuses nothing: {:?}",
        outcome.refused
    );
    assert_eq!(outcome.reader_version, history::READER_VERSION);

    let stored = recorded_commits(&root);
    assert_eq!(stored.len(), expected.len());

    for commit in expected {
        let oid = commit["oid"].as_str().expect("an oid");
        let row = stored
            .get(oid)
            .unwrap_or_else(|| panic!("{oid} must be recorded"));
        assert_eq!(row.tree_oid, commit["tree_oid"].as_str().unwrap(), "{oid}");
        assert_eq!(
            row.parent_oids,
            json_strings(&commit["parent_oids"]),
            "{oid}"
        );
        assert_eq!(
            row.author_time,
            commit["author_epoch"].as_i64().unwrap(),
            "{oid}"
        );
        assert_eq!(
            row.author_tz,
            commit["author_tz"].as_str().unwrap(),
            "{oid}"
        );
        assert_eq!(
            row.committer_time,
            commit["committer_epoch"].as_i64().unwrap(),
            "{oid}"
        );
        assert_eq!(
            row.committer_tz,
            commit["committer_tz"].as_str().unwrap(),
            "{oid}"
        );
        assert_eq!(row.summary, commit["summary"].as_str().unwrap(), "{oid}");
        assert_eq!(row.is_merge, commit["is_merge"].as_bool().unwrap(), "{oid}");
        // Identity is off by default, so no third-party personal data is in the index.
        assert_eq!(row.author_ident, None, "{oid}");
        assert_eq!(row.committer_ident, None, "{oid}");

        let expected_completeness = if commit["parent_oids"].as_array().unwrap().is_empty() {
            ParentCompleteness::Root
        } else {
            ParentCompleteness::ParentsAvailable
        };
        assert_eq!(row.parent_completeness, expected_completeness, "{oid}");
        assert_eq!(
            row.changes_enumerated,
            ChangesEnumerated::Enumerated,
            "{oid}"
        );
        assert_changes_match(oid, &commit["changes"], &changes_of(&root, oid));
    }

    // The timezone offsets the fixture deliberately contains, so a reader that normalised to UTC and
    // dropped the offset would fail here rather than pass quietly.
    let zones: BTreeSet<&str> = stored.values().map(|row| row.author_tz.as_str()).collect();
    assert!(
        zones.contains("+0530") && zones.contains("-0800"),
        "{zones:?}"
    );
    // And one commit whose author and committer times differ, so both columns are proven distinct.
    assert!(
        stored
            .values()
            .any(|row| row.author_time != row.committer_time),
        "the fixture contains a commit whose author time is six hours before its committer time"
    );

    let counts = totals(&root).changes_by_kind;
    for kind in ChangeKind::ALL {
        assert!(
            counts[&kind] > 0,
            "{kind} has no rows, so its case is untested: {counts:?}"
        );
    }
}

/// A mode change is a change to a file whose bytes did not move. Reporting it as `modified` would
/// claim a content change that did not happen.
#[test]
fn a_mode_change_keeps_the_blob_oid_and_is_not_a_modification() {
    let (_dir, root) = common::history_fixture("history-basic");
    ingest(&root, &HistoryOptions::default());
    let inventory = common::history_inventory("history-basic");

    let mut found = 0;
    for commit in inventory["commits"].as_array().unwrap() {
        for change in commit["changes"].as_array().unwrap() {
            if change["change_kind"] != "mode_changed" {
                continue;
            }
            found += 1;
            let oid = commit["oid"].as_str().unwrap();
            let rows = changes_of(&root, oid);
            let row = rows
                .iter()
                .find(|row| row.path == change["path"].as_str().unwrap())
                .expect("the mode-changed path is recorded");
            assert_eq!(row.change_kind, ChangeKind::ModeChanged);
            assert_eq!(row.blob_oid, row.prev_blob_oid, "the bytes did not move");
            assert_ne!(row.mode, row.prev_mode, "the mode did");
        }
    }
    assert_eq!(found, 1, "the fixture carries exactly one mode change");
}

/// A root commit **is** diffed against the empty tree, so its changes are its whole tree. That is
/// correct here and is the thing that must never happen at a boundary.
#[test]
fn the_root_commits_changes_are_its_whole_tree() {
    let (_dir, root) = common::history_fixture("history-basic");
    ingest(&root, &HistoryOptions::default());
    let inventory = common::history_inventory("history-basic");
    let store = open_store(&root);

    let commit = inventory["commits"]
        .as_array()
        .unwrap()
        .iter()
        .find(|commit| commit["parent_oids"].as_array().unwrap().is_empty())
        .expect("the fixture has a root commit");
    let oid = commit["oid"].as_str().unwrap();

    let mut paths = BTreeSet::new();
    tree_paths(
        &store,
        Oid::from_hex(commit["tree_oid"].as_str().unwrap()).unwrap(),
        "",
        &mut paths,
    );
    assert!(paths.len() >= 6, "the root tree is not empty: {paths:?}");

    let rows = changes_of(&root, oid);
    assert!(rows.iter().all(|row| row.change_kind == ChangeKind::Added));
    assert_eq!(
        rows.iter()
            .map(|row| row.path.clone())
            .collect::<BTreeSet<_>>(),
        paths
    );
}

/// **The property the measured delta cost rests on.**
///
/// The proof is physical rather than statistical: the tree object for `src/lib`, whose oid is equal in
/// every commit after the root, is **deleted from the object store**. A reader that skips an
/// unchanged subtree by comparing the entry's oid never opens it and does not notice; a reader that
/// opens it first, or descends into it anyway, hits an absent object and reports a refusal.
///
/// The control run at the end is what makes this non-vacuous: with the root commit inside the budget
/// the same deleted object *is* needed — a root is diffed against the empty tree, so every path in it
/// is enumerated — and the refusal appears. So the absence is real and it is really load-bearing.
#[test]
fn an_unchanged_subtree_is_skipped_without_being_opened() {
    let (_dir, root) = common::history_fixture("history-basic");
    let git_dir = root.join(".git");
    let inventory = common::history_inventory("history-basic");

    // Find `src/lib`'s tree oid the way a reader would, out of the fixture's own bytes.
    let lib_oid = {
        let store = open_store(&root);
        let head = Oid::from_hex(inventory["head_oid"].as_str().unwrap()).unwrap();
        let gitobj::Object::Commit(bytes) = store.read(&head).unwrap().unwrap() else {
            panic!("HEAD is a commit");
        };
        let commit = gitobj::parse_commit(&bytes).unwrap();
        let mut oid = commit.tree;
        for segment in ["src", "lib"] {
            let gitobj::Object::Tree(bytes) = store.read(&oid).unwrap().unwrap() else {
                panic!("a tree");
            };
            oid = gitobj::parse_tree(&bytes)
                .unwrap()
                .into_iter()
                .find(|entry| entry.name == segment.as_bytes())
                .unwrap_or_else(|| panic!("{segment} is in the tree"))
                .oid;
        }
        oid
    };
    let removed = object_path(&git_dir, &lib_oid.to_hex());
    assert!(removed.is_file(), "the fixture stores src/lib loose");
    std::fs::remove_file(&removed).expect("the copy is writable");
    assert!(!removed.exists());

    // Five commits: every one past the root, each of which must skip `src/lib` by oid equality.
    let outcome = ingest(
        &root,
        &HistoryOptions {
            max_commits: 5,
            ..HistoryOptions::default()
        },
    );
    assert_eq!(outcome.commits_recorded, 5);
    assert_eq!(outcome.walk_terminated_by, WalkTermination::CommitBudget);
    assert_eq!(
        outcome.refusals(history::form::TREE_ABSENT),
        0,
        "an unchanged subtree was opened: {:?}",
        outcome.refused
    );
    assert_eq!(
        outcome.enumeration_of(ChangesEnumerated::Enumerated),
        5,
        "every walked commit still enumerated: {:?}",
        outcome.enumeration
    );

    // Git's own claim about which top-level entries were unchanged, as a floor on the skip count.
    let expected_skips: usize = inventory["commits"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|commit| commit["oid"].as_str() != inventory["commit_order"][0].as_str())
        .filter_map(|commit| commit["equal_top_level_entries"].as_array())
        .flatten()
        .filter(|entry| entry["type"] == "tree")
        .count();
    assert!(expected_skips > 0, "the inventory claims no equal subtrees");
    assert!(
        outcome.subtrees_skipped >= expected_skips,
        "skipped {} but Git names {expected_skips} equal top-level subtrees",
        outcome.subtrees_skipped
    );
    // And every change row still matches Git, so skipping did not lose a change.
    for commit in inventory["commits"].as_array().unwrap() {
        let oid = commit["oid"].as_str().unwrap();
        if recorded_commits(&root).contains_key(oid) {
            assert_changes_match(oid, &commit["changes"], &changes_of(&root, oid));
        }
    }

    // The control. The root commit needs the deleted object, so a full walk must notice it.
    let (_dir, control) = common::history_fixture("history-basic");
    std::fs::remove_file(object_path(&control.join(".git"), &lib_oid.to_hex())).unwrap();
    let full = ingest(&control, &HistoryOptions::default());
    assert!(
        full.refusals(history::form::TREE_ABSENT) > 0,
        "the deleted object is not actually needed by a full walk, so the test above proves nothing"
    );
}

// ---- history-shallow: the headline gate -------------------------------------------------------

/// Criterion 6 and 7. A shallow boundary is **not** a root, and it is never diffed against the empty
/// tree.
#[test]
fn the_shallow_boundary_is_a_boundary_and_never_a_root() {
    let (_dir, root) = common::history_fixture("history-shallow");
    let outcome = ingest(&root, &HistoryOptions::default());
    let inventory = common::history_inventory("history-shallow");

    let boundary_oids = json_strings(&inventory["shallow"]["boundary_oids"]);
    assert_eq!(boundary_oids.len(), 1, "the fixture declares one boundary");
    assert!(outcome.shallow, "Git wrote a shallow file; it must be read");
    assert_eq!(outcome.shallow_boundary, boundary_oids);
    assert_eq!(outcome.walk_terminated_by, WalkTermination::ShallowBoundary);
    assert_eq!(
        outcome.commits_recorded,
        inventory["commits"].as_array().unwrap().len()
    );
    assert_eq!(outcome.refusals(history::form::SHALLOW_UNVERIFIABLE), 0);

    let boundary = &boundary_oids[0];
    let stored = recorded_commits(&root);
    let row = stored.get(boundary).expect("the boundary is recorded");

    assert_eq!(
        row.parent_completeness,
        ParentCompleteness::ShallowBoundary,
        "a declared boundary is neither a root nor a fault"
    );
    assert!(
        !row.parent_completeness.may_claim_history_begins_here(),
        "nothing may say the project's history begins at a boundary"
    );

    // The boundary commit's **own object** still names a parent. Git's revision walk hides it,
    // because the graft cuts it off, and a reader built on that view reports the boundary as
    // parentless — that is, as a root. The fixture records both views so this can be checked.
    let from_object = json_strings(
        &inventory["commits"]
            .as_array()
            .unwrap()
            .iter()
            .find(|commit| commit["oid"].as_str() == Some(boundary.as_str()))
            .unwrap()["parent_oids"],
    );
    let from_walk = json_strings(
        &inventory["commits"]
            .as_array()
            .unwrap()
            .iter()
            .find(|commit| commit["oid"].as_str() == Some(boundary.as_str()))
            .unwrap()["parent_oids_from_revision_walk"],
    );
    assert_eq!(from_object.len(), 1, "the commit object names its parent");
    assert!(from_walk.is_empty(), "and the revision walk does not");
    assert_eq!(
        row.parent_oids, from_object,
        "the commit object is the source of truth for parents"
    );

    // **The gate.** Zero rows, and the count a wrong reader would have produced is stated, so the
    // failure message names the damage rather than reporting a bare inequality.
    let wrongly_addable = inventory["shallow"]["boundary_tree_path_counts"][boundary]
        .as_u64()
        .expect("Git counted the boundary tree's paths");
    assert!(
        wrongly_addable > 0,
        "the boundary tree is empty, so this test could pass vacuously"
    );
    let store = open_store(&root);
    let mut paths = BTreeSet::new();
    tree_paths(
        &store,
        Oid::from_hex(row.tree_oid.as_str()).unwrap(),
        "",
        &mut paths,
    );
    assert_eq!(
        paths.len() as u64,
        wrongly_addable,
        "the reader and Git disagree about the boundary tree's size"
    );

    // **The gate itself, asserted before anything else about the commit**, so that the mutation
    // probe's failure message names the damage rather than reporting a column mismatch.
    let rows = changes_of(&root, boundary);
    let added = rows
        .iter()
        .filter(|row| row.change_kind == ChangeKind::Added)
        .count();
    assert_eq!(
        added, 0,
        "the boundary was diffed against the empty tree: {added} of the {wrongly_addable} paths in \
         the boundary tree were reported as newly added, which states 'the project's history begins \
         here' as data"
    );
    assert!(rows.is_empty(), "and no other kind either: {rows:?}");
    assert_eq!(
        row.changes_enumerated,
        ChangesEnumerated::ParentUnavailable,
        "and the zero rows must say which of the four silences they are"
    );

    // The tip, whose parent *is* present, still enumerates. A reader that abandoned the whole walk
    // because one object was missing would fail here.
    let tip = inventory["head_oid"].as_str().unwrap();
    let tip_row = stored.get(tip).expect("the tip is recorded");
    assert_eq!(tip_row.changes_enumerated, ChangesEnumerated::Enumerated);
    let expected = inventory["commits"]
        .as_array()
        .unwrap()
        .iter()
        .find(|commit| commit["oid"].as_str() == Some(tip))
        .unwrap();
    assert_changes_match(tip, &expected["changes"], &changes_of(&root, tip));
    assert!(
        !changes_of(&root, tip).is_empty(),
        "the tip must contribute rows, or 'zero at the boundary' proves nothing"
    );
}

// ---- history-merge ----------------------------------------------------------------------------

/// Criterion 11. A merge and an empty commit both have zero change rows, and the stored column is
/// what tells them apart.
#[test]
fn a_merge_and_an_empty_commit_both_have_no_rows_and_are_still_distinguishable() {
    let (_dir, root) = common::history_fixture("history-merge");
    let outcome = ingest(&root, &HistoryOptions::default());
    let inventory = common::history_inventory("history-merge");

    assert_eq!(
        outcome.commits_recorded,
        inventory["commits"].as_array().unwrap().len(),
        "both sides of the merge are still walked, so no branch is lost"
    );
    assert_eq!(outcome.walk_terminated_by, WalkTermination::Exhausted);

    let stored = recorded_commits(&root);
    let merge = inventory["commits"]
        .as_array()
        .unwrap()
        .iter()
        .find(|commit| commit["is_merge"] == true)
        .expect("the fixture has a merge");
    let merge_oid = merge["oid"].as_str().unwrap();
    let merge_row = stored.get(merge_oid).expect("the merge is recorded");
    assert!(merge_row.is_merge);
    assert_eq!(merge_row.parent_oids.len(), 2);
    assert_eq!(
        merge_row.changes_enumerated,
        ChangesEnumerated::MergeNotEnumerated
    );
    assert!(changes_of(&root, merge_oid).is_empty());
    assert!(merge["changes"].is_null(), "Git enumerates nothing either");

    // The empty commit: enumerated, and genuinely empty. Same row count, different stored fact.
    let empty = inventory["commits"]
        .as_array()
        .unwrap()
        .iter()
        .find(|commit| commit["changes"].as_array().is_some_and(Vec::is_empty))
        .expect("the fixture has an empty commit");
    let empty_oid = empty["oid"].as_str().unwrap();
    let empty_row = stored.get(empty_oid).expect("the empty commit is recorded");
    assert!(!empty_row.is_merge);
    assert_eq!(empty_row.changes_enumerated, ChangesEnumerated::Enumerated);
    assert!(changes_of(&root, empty_oid).is_empty());

    assert_ne!(
        merge_row.changes_enumerated, empty_row.changes_enumerated,
        "two zero-row commits must not be the same fact"
    );

    // The diamond is walked once: two branch commits and one root, not two roots.
    assert_eq!(outcome.completeness_of(ParentCompleteness::Root), 1);
    let totals = totals(&root);
    assert_eq!(totals.merges, 1);
    assert!(
        totals.changes > 0,
        "the branch commits still contribute rows"
    );
}

// ---- history-worktree -------------------------------------------------------------------------

/// Build the linked checkout the fixture deliberately does not commit.
///
/// The committed `gitdir` pointer was rewritten to a synthetic path when the fixture was generated,
/// so resolving through it would be asserting against a value with no meaning. The worktree's private
/// git directory is opened directly instead, through a pointer this test writes.
fn linked_worktree() -> (tempfile::TempDir, PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    let main = dir.path().join("main");
    std::fs::create_dir_all(&main).unwrap();
    common::copy_tree(
        &common::named_fixture_root("history-worktree").join("gitdir"),
        &main.join(".git"),
    );
    let private = main.join(".git/worktrees/linked");
    assert!(private.is_dir(), "the fixture commits the private git dir");
    assert!(
        !private.join("objects").exists(),
        "a linked worktree has no objects of its own; that is the point"
    );

    let checkout = dir.path().join("linked");
    std::fs::create_dir_all(&checkout).unwrap();
    std::fs::write(
        checkout.join(".git"),
        format!("gitdir: {}\n", private.display()),
    )
    .unwrap();
    nerve_index::init_with_project_id(&checkout, Some(common::TEST_PROJECT_ID)).unwrap();
    (dir, checkout)
}

/// Criterion 5b, first half. A linked worktree has history, and it is the worktree's own.
///
/// `commits_recorded > 0` is the assertion rather than "does not crash": a fixture that passes while
/// producing nothing is the failure shape a corrective slice on this project was written for.
#[test]
fn a_linked_worktree_records_its_own_history() {
    let (_dir, root) = linked_worktree();
    let outcome = ingest(&root, &HistoryOptions::default());
    let inventory = common::history_inventory("history-worktree");

    let tip = inventory["notes"]["linked_worktree_tip"].as_str().unwrap();
    assert!(
        outcome.commits_recorded > 0,
        "the worktree read as a repository with no history"
    );
    assert_eq!(
        outcome.head_oid.as_deref(),
        Some(tip),
        "HEAD must resolve to the worktree's branch, not to main's"
    );
    assert_ne!(
        outcome.head_oid.as_deref(),
        inventory["head_oid"].as_str(),
        "a silent fallback to the main branch is detectable and must not happen"
    );
    assert_eq!(
        outcome.commits_recorded,
        inventory["commits"].as_array().unwrap().len(),
        "every commit in the fixture is reachable from the worktree tip"
    );
    assert_eq!(outcome.walk_terminated_by, WalkTermination::Exhausted);

    for commit in inventory["commits"].as_array().unwrap() {
        let oid = commit["oid"].as_str().unwrap();
        assert_changes_match(oid, &commit["changes"], &changes_of(&root, oid));
    }
}

/// Criterion 5b, second half — a **pre-existing** defect this slice fixes.
///
/// `gitinfo::head_commit` is fed to `repository_state.git_commit` by the indexing pipeline, and before
/// `commondir` was followed it answered `None` for every linked worktree. So indexing a linked
/// worktree recorded no commit for the state, silently. This lives here rather than beside the
/// incremental tests because the defect is a worktree defect, not an incremental one.
#[test]
fn indexing_a_linked_worktree_records_a_commit_for_the_repository_state() {
    let (_dir, root) = linked_worktree();
    std::fs::create_dir_all(root.join("src")).unwrap();
    std::fs::write(
        root.join("src/a.ts"),
        "export function one(): number { return 1; }\n",
    )
    .unwrap();
    let outcome = nerve_index::index_repository(&root).expect("indexing a worktree");
    assert!(
        outcome.files_processed > 0,
        "nothing was indexed, so the state proves nothing"
    );
    assert!(
        outcome.git_commit.is_some(),
        "the run reported no commit for a worktree that has one"
    );

    let conn = common::open_db(&root);
    let state_id = nerve_store::status(&conn)
        .unwrap()
        .state_id
        .expect("indexing records a state");
    let state = nerve_store::repository_state(&conn, &state_id)
        .unwrap()
        .expect("the state row exists");
    let tip = common::history_inventory("history-worktree")["notes"]["linked_worktree_tip"]
        .as_str()
        .unwrap()
        .to_string();
    assert_eq!(
        state.git_commit,
        Some(tip),
        "indexing a linked worktree must record the commit it was indexed at"
    );
}

// ---- history-missing and the undecidable case -------------------------------------------------

/// Criterion 9. An absent parent with no `shallow` file is a fault, and a fault is not a shallow
/// clone.
#[test]
fn an_absent_parent_with_no_shallow_file_is_missing_and_never_shallow() {
    let (_dir, root) = common::history_fixture("history-missing");
    let outcome = ingest(&root, &HistoryOptions::default());
    let inventory = common::history_inventory("history-missing");

    assert!(inventory["shallow"].is_null(), "the fixture has no shallow");
    assert!(!outcome.shallow);
    assert!(outcome.shallow_boundary.is_empty());
    assert_eq!(outcome.walk_terminated_by, WalkTermination::MissingObject);
    assert_eq!(outcome.refusals(history::form::SHALLOW_UNVERIFIABLE), 0);

    let child = inventory["notes"]["child_of_deleted_commit"]
        .as_str()
        .unwrap();
    let stored = recorded_commits(&root);
    let row = stored.get(child).expect("the child is recorded");
    assert_eq!(row.parent_completeness, ParentCompleteness::ParentsMissing);
    assert_eq!(row.changes_enumerated, ChangesEnumerated::ParentUnavailable);
    assert!(changes_of(&root, child).is_empty());
    assert_eq!(
        outcome.completeness_of(ParentCompleteness::ShallowBoundary),
        0
    );
    assert_eq!(
        outcome.completeness_of(ParentCompleteness::ParentsUnverifiable),
        0
    );
    assert_eq!(
        outcome.completeness_of(ParentCompleteness::ParentsMissing),
        1
    );

    // The hole is real, and it is a hole rather than a truncation: the object is absent — which is
    // `Ok(None)`, neither an error nor a refusal — and the commit below it is still readable by oid.
    let store = open_store(&root);
    let deleted =
        Oid::from_hex(inventory["notes"]["deleted_commit_oid"].as_str().unwrap()).unwrap();
    assert!(store.read(&deleted).unwrap().is_none());
    let below = inventory["commits"]
        .as_array()
        .unwrap()
        .iter()
        .find(|commit| commit["parent_oids"].as_array().unwrap().is_empty())
        .unwrap()["oid"]
        .as_str()
        .unwrap();
    assert!(
        store
            .read(&Oid::from_hex(below).unwrap())
            .unwrap()
            .is_some(),
        "the object store is otherwise intact"
    );
    // …and therefore not recorded, because it is unreachable from HEAD across the hole.
    assert!(!stored.contains_key(below));
    assert_eq!(outcome.commits_recorded, 2);
}

/// Criterion 9a. `.git/shallow` present but unreadable by 12a's bound: the absence is
/// **unverifiable**, which is neither shallow nor corrupt.
///
/// Tested in three directions in one test, because "never says shallow" is satisfied by a value
/// frozen at `parents_missing` — the same trap a corrective slice recorded for a count frozen at
/// zero.
#[test]
fn an_unreadable_shallow_file_makes_an_absence_unverifiable_rather_than_a_fault() {
    // 1. Over the pointer-file bound. `read_pointer_file` answers `None` for absent, over-bound and
    //    unreadable alike, and `None` is defined as *not shallow* — so from inside the reader this is
    //    indistinguishable from a complete repository.
    let (_dir, root) = common::history_fixture("history-shallow");
    let shallow = root.join(".git/shallow");
    let real = std::fs::read_to_string(&shallow).unwrap();
    assert_eq!(real.trim().len(), 40, "the fixture's shallow is one oid");
    // The bound is 65 536 ids at 41 bytes plus slack; comfortably past it, and the content is never
    // read so its shape does not matter.
    let mut oversized = real.clone();
    oversized.push_str(&"\n".repeat(3_000_000));
    std::fs::write(&shallow, &oversized).unwrap();

    let outcome = ingest(&root, &HistoryOptions::default());
    assert!(
        !outcome.shallow,
        "the reader cannot see the declaration, and must not pretend it can"
    );
    assert_eq!(
        outcome.refusals(history::form::SHALLOW_UNVERIFIABLE),
        1,
        "the undecidable condition must be counted, not inferred: {:?}",
        outcome.refused
    );
    assert_eq!(
        outcome.completeness_of(ParentCompleteness::ParentsUnverifiable),
        1
    );
    assert_eq!(
        outcome.completeness_of(ParentCompleteness::ParentsMissing),
        0,
        "an absence Nerve could not check must not be called corrupt"
    );
    assert_eq!(
        outcome.completeness_of(ParentCompleteness::ShallowBoundary),
        0,
        "and it must not be called shallow either"
    );

    // 2. The same fixture, unmodified: the declaration is readable and the commit is a boundary.
    let (_dir, readable) = common::history_fixture("history-shallow");
    let outcome = ingest(&readable, &HistoryOptions::default());
    assert_eq!(
        outcome.completeness_of(ParentCompleteness::ShallowBoundary),
        1
    );
    assert_eq!(
        outcome.completeness_of(ParentCompleteness::ParentsUnverifiable),
        0
    );

    // 3. No shallow file at all: the absence is a fault, and the value is not frozen at
    //    `parents_unverifiable`.
    let (_dir, missing) = common::history_fixture("history-missing");
    let outcome = ingest(&missing, &HistoryOptions::default());
    assert_eq!(
        outcome.completeness_of(ParentCompleteness::ParentsMissing),
        1
    );
    assert_eq!(
        outcome.completeness_of(ParentCompleteness::ParentsUnverifiable),
        0
    );
}

// ---- history-hostile --------------------------------------------------------------------------

/// Criterion 5a and 13a. Every hostile path is refused **per case**, and every hostile summary is
/// stored as inert data.
///
/// The refusing guard is 12a's `parse_tree` rather than the new path guard, and that is measured
/// rather than assumed: the fixture puts its hostile names in the *same tree* as a subtree called
/// `..`, and a tree carrying a `..` entry is refused **whole** — a prefix of a tree read as a tree
/// would assert that the remaining paths do not exist. So the names never reach
/// `discover::safe_tree_name`, which is exercised with real bytes further down.
#[test]
fn every_hostile_tree_entry_name_is_refused_and_counted() {
    let (_dir, root) = common::history_fixture("history-hostile");
    let outcome = ingest(&root, &HistoryOptions::default());
    let inventory = common::history_inventory("history-hostile");
    let attacks = inventory["attacks"].as_object().expect("an attack table");

    assert!(
        inventory["attacks_not_achieved"]
            .as_array()
            .unwrap()
            .is_empty(),
        "an attack the fixture could not construct cannot be asserted about"
    );
    assert_eq!(
        outcome.commits_recorded,
        inventory["commits"].as_array().unwrap().len(),
        "a refused tree must not cost the commit its row"
    );

    // Every path stored anywhere in this repository's history.
    let stored_paths: BTreeSet<String> = inventory["commits"]
        .as_array()
        .unwrap()
        .iter()
        .flat_map(|commit| changes_of(&root, commit["oid"].as_str().unwrap()))
        .map(|row| row.path)
        .collect();
    assert!(
        !stored_paths.is_empty(),
        "nothing at all was stored, so 'the hostile path is absent' proves nothing"
    );

    let mut path_attacks = 0;
    for (name, attack) in attacks {
        let Some(path) = attack["path"].as_str() else {
            continue;
        };
        path_attacks += 1;
        assert!(attack["achieved"] == true, "{name}");
        // Per case, by name. An aggregate "the counter moved" would pass over a reader that stored
        // all but one of these.
        assert!(
            !stored_paths.contains(path),
            "{name}: the hostile path was stored"
        );

        // …and the commit that introduced it is recorded with its silence qualified, rather than
        // looking like a commit that changed nothing.
        for commit in inventory["commits"].as_array().unwrap() {
            let introduced = commit["changes"]
                .as_array()
                .is_some_and(|changes| changes.iter().any(|change| change["path"] == path));
            if !introduced {
                continue;
            }
            let oid = commit["oid"].as_str().unwrap();
            let row = recorded_commits(&root)
                .remove(oid)
                .unwrap_or_else(|| panic!("{oid} must be recorded"));
            assert_eq!(
                row.changes_enumerated,
                ChangesEnumerated::Refused,
                "{name}: commit {oid} must say why it has no rows"
            );
            assert!(changes_of(&root, oid).is_empty(), "{name}: commit {oid}");
        }
    }
    assert_eq!(path_attacks, 5, "the fixture claims five path attacks");

    // The refusal is counted by form, and the count is exact rather than "nonzero": three trees in
    // this fixture are unreadable — the two that carry hostile names, and the parent of the commit
    // that removes them again.
    assert_eq!(
        outcome.refusals(gitobj::form::TREE_ENTRY_MALFORMED),
        3,
        "refusals: {:?}",
        outcome.refused
    );
    assert_eq!(outcome.enumeration_of(ChangesEnumerated::Refused), 3);
    assert!(
        outcome.enumeration_of(ChangesEnumerated::Enumerated) > 0,
        "the clean commits must still enumerate"
    );
}

/// Criterion 13a. The summary is stored, bounded, first line only, and never interpreted.
#[test]
fn a_hostile_commit_summary_is_stored_bounded_and_flagged() {
    let (_dir, root) = common::history_fixture("history-hostile");
    let outcome = ingest(&root, &HistoryOptions::default());
    let inventory = common::history_inventory("history-hostile");
    let stored = recorded_commits(&root);

    let mut summary_attacks = 0;
    let mut truncated = 0;
    for (name, attack) in inventory["attacks"].as_object().unwrap() {
        let Some(expected) = attack["summary"].as_str() else {
            continue;
        };
        summary_attacks += 1;
        let oid = attack["commit_oid"]
            .as_str()
            .expect("the attack names a commit");
        let row = stored.get(oid).unwrap_or_else(|| panic!("{oid} recorded"));
        let bytes = attack["summary_bytes"].as_u64().unwrap() as usize;
        assert_eq!(bytes, expected.len(), "{name}: the inventory is consistent");

        if bytes > history::MAX_SUMMARY_BYTES {
            truncated += 1;
            assert_eq!(
                row.summary.len(),
                history::MAX_SUMMARY_BYTES,
                "{name}: truncated to the bound"
            );
            assert_eq!(
                row.summary,
                expected[..history::MAX_SUMMARY_BYTES],
                "{name}"
            );
        } else {
            assert_eq!(
                row.summary, expected,
                "{name}: stored verbatim, not dropped"
            );
        }
    }
    assert_eq!(
        summary_attacks, 3,
        "the fixture claims three summary attacks"
    );
    assert_eq!(truncated, 1, "one of them is over the bound");

    // Truncation is flagged rather than silent: a consumer cannot otherwise tell a short summary
    // from a cut one.
    assert_eq!(outcome.summaries_truncated, 1);
    assert_eq!(outcome.refusals(history::form::SUMMARY_TRUNCATED), 1);
    assert_eq!(
        ingest_row(&root).refusals[history::form::SUMMARY_TRUNCATED],
        1,
        "and the flag survives into the database, not only into the outcome"
    );

    // Stored as data. The script tag and the instruction-shaped sentence come back byte-for-byte,
    // neither sanitized nor obeyed — escaping is the surface's job, and there is nothing to escape if
    // the ingester dropped it.
    let script = stored
        .values()
        .find(|row| row.summary.contains("<script>"))
        .expect("the script-tag summary is stored");
    assert!(script.summary.contains("</script>"));
    assert!(stored
        .values()
        .any(|row| row.summary.contains("IGNORE ALL PREVIOUS INSTRUCTIONS")));
}

// ---- budget, renames, idempotence, repair -----------------------------------------------------

/// Criterion 8. A bounded ingest of a **complete** repository is not a shallow one.
#[test]
fn a_bounded_ingest_reports_the_budget_and_not_a_boundary() {
    let (_dir, root) = common::history_fixture("history-basic");
    let inventory = common::history_inventory("history-basic");
    let all = inventory["commits"].as_array().unwrap().len();
    assert!(all > 2, "the fixture must be longer than the budget");

    let outcome = ingest(
        &root,
        &HistoryOptions {
            max_commits: 2,
            ..HistoryOptions::default()
        },
    );
    assert_eq!(outcome.walk_terminated_by, WalkTermination::CommitBudget);
    assert!(!outcome.shallow, "Nerve's boundary is not the repository's");
    assert!(outcome.shallow_boundary.is_empty());
    assert_eq!(outcome.commits_recorded, 2);
    assert_eq!(outcome.commit_budget, 2);
    assert!(outcome.refusals(history::form::COMMIT_BUDGET) > 0);
    // No commit in a bounded ingest of a complete repository may claim the history begins at it,
    // except a genuine root — and this walk never reaches one.
    assert_eq!(outcome.completeness_of(ParentCompleteness::Root), 0);
    assert_eq!(ingest_row(&root).commit_budget, 2);

    // A request above the hard bound is refused with the clamp stated rather than honoured.
    let (_dir, other) = common::history_fixture("history-basic");
    let outcome = ingest(
        &other,
        &HistoryOptions {
            max_commits: history::MAX_HISTORY_COMMITS + 1,
            ..HistoryOptions::default()
        },
    );
    assert_eq!(outcome.commit_budget, history::MAX_HISTORY_COMMITS);
    assert_eq!(outcome.refusals(history::form::COMMIT_BUDGET), 1);
    assert_eq!(outcome.commits_recorded, all);
}

/// Criterion 5a's second half and criterion 10. Deleted paths survive the guard, and an ambiguous
/// rename records every pairing.
///
/// Both were **structurally impossible** under the guard the plan first named — `canonical_child`
/// canonicalizes, and a deleted path is by definition not on disk — so this is the test that proves
/// the new guard works rather than that the repository has no renames.
#[test]
fn deleted_paths_survive_and_every_rename_pairing_is_recorded() {
    let (_dir, root) = common::history_fixture("history-rename");
    ingest(&root, &HistoryOptions::default());
    let inventory = common::history_inventory("history-rename");

    let totals = totals(&root);
    assert!(
        totals.changes_by_kind[&ChangeKind::Deleted] > 0,
        "no deleted rows: the path guard refused a path that is not on disk"
    );
    assert!(totals.renames > 0, "git_rename_hypothesis is empty");

    let expected = inventory["exact_content_rename_candidates"]
        .as_array()
        .expect("Git's own exact-content pairings");
    assert!(!expected.is_empty(), "the fixture claims no renames");
    assert_eq!(totals.renames, expected.len() as i64);

    let conn = common::open_db(&root);
    let repo = repo_id(&root);
    for candidate in expected {
        let from = candidate["from_path"].as_str().unwrap();
        let to = candidate["to_path"].as_str().unwrap();
        let rows = nerve_store::renames_touching_path(&conn, &repo, from, usize::MAX).unwrap();
        let row = rows
            .iter()
            .find(|row| row.to_path == to)
            .unwrap_or_else(|| panic!("{from} -> {to} must be recorded"));
        assert_eq!(row.commit_oid, candidate["commit_oid"].as_str().unwrap());
        assert_eq!(row.blob_oid, candidate["blob_oid"].as_str().unwrap());
        assert_eq!(row.evidence, RenameEvidence::ExactContent);
        assert_eq!(
            row.ambiguity.as_str(),
            candidate["ambiguity"].as_str().unwrap(),
            "{from} -> {to}"
        );
    }

    // Both shapes are present, so neither case is untested — and the ambiguous one keeps *both*
    // pairings rather than promoting one.
    let shapes: BTreeSet<&str> = expected
        .iter()
        .map(|candidate| candidate["ambiguity"].as_str().unwrap())
        .collect();
    assert!(
        shapes.contains(RenameAmbiguity::Unique.as_str()),
        "{shapes:?}"
    );
    assert!(
        shapes.contains(RenameAmbiguity::ManyTo.as_str()),
        "{shapes:?}"
    );
    let ambiguous: Vec<_> = expected
        .iter()
        .filter(|candidate| candidate["ambiguity"] == "many_to")
        .collect();
    assert_eq!(ambiguous.len(), 2, "one blob, two added paths, two rows");
    let from = ambiguous[0]["from_path"].as_str().unwrap();
    assert_eq!(
        nerve_store::renames_touching_path(&conn, &repo, from, usize::MAX)
            .unwrap()
            .len(),
        2,
        "an ambiguous match promotes nothing"
    );
}

/// A second sync of an unchanged repository writes nothing and says so.
#[test]
fn a_second_sync_writes_nothing_and_reports_the_commits_as_already_present() {
    let (_dir, root) = common::history_fixture("history-basic");
    let first = ingest(&root, &HistoryOptions::default());
    let before = totals(&root);
    assert!(first.commits_recorded > 0 && first.changes_written > 0);

    let second = ingest(&root, &HistoryOptions::default());
    assert_eq!(second.commits_recorded, 0);
    assert_eq!(second.changes_written, 0);
    assert_eq!(second.renames_written, 0);
    assert_eq!(second.commits_repaired, 0);
    assert_eq!(
        second.commits_already_present, first.commits_recorded,
        "every commit must be reported as already present, not merely skipped"
    );
    assert_eq!(second.commits_walked, first.commits_walked);
    assert_eq!(totals(&root), before, "no row moved");
}

/// §8.5.1. A boundary commit's availability is a conclusion from **absence**, so an unshallow must
/// change it — and the repair is what makes that possible.
///
/// The unshallow is simulated by declaring a boundary over a repository whose objects are all
/// present and then withdrawing the declaration, which is the same transition a
/// `git fetch --unshallow` performs: the declaration is what Git grafts on, and removing it is what
/// removing it does.
#[test]
fn an_unshallowed_boundary_commit_is_deleted_and_re_recorded() {
    let (_dir, root) = common::history_fixture("history-basic");
    let inventory = common::history_inventory("history-basic");
    let order = json_strings(&inventory["commit_order"]);
    // Third from the tip, so the walk records a few commits above it and stops there.
    let boundary = order[order.len() - 3].clone();

    std::fs::write(root.join(".git/shallow"), format!("{boundary}\n")).unwrap();
    let shallow = ingest(&root, &HistoryOptions::default());
    assert!(shallow.shallow);
    assert_eq!(shallow.shallow_boundary, vec![boundary.clone()]);
    assert_eq!(shallow.walk_terminated_by, WalkTermination::ShallowBoundary);
    assert_eq!(
        shallow.commits_recorded, 3,
        "the walk stops at the boundary"
    );
    let row = recorded_commits(&root)[&boundary].clone();
    assert_eq!(row.parent_completeness, ParentCompleteness::ShallowBoundary);
    assert_eq!(row.changes_enumerated, ChangesEnumerated::ParentUnavailable);
    assert!(changes_of(&root, &boundary).is_empty());

    // The unshallow.
    std::fs::remove_file(root.join(".git/shallow")).unwrap();
    let repaired = ingest(&root, &HistoryOptions::default());
    assert_eq!(
        repaired.commits_repaired, 1,
        "the former boundary must be deleted, or it keeps availability data that is now false"
    );
    assert!(!repaired.shallow);
    assert_eq!(repaired.walk_terminated_by, WalkTermination::Exhausted);
    assert_eq!(
        repaired.commits_recorded,
        order.len() - 2,
        "the repaired commit plus everything below it"
    );
    assert_eq!(
        repaired.commits_already_present, 2,
        "and the two above it are recognised rather than rewritten"
    );

    let row = recorded_commits(&root)[&boundary].clone();
    assert_eq!(
        row.parent_completeness,
        ParentCompleteness::ParentsAvailable,
        "the boundary is an ordinary commit once the declaration is gone"
    );
    assert_eq!(row.changes_enumerated, ChangesEnumerated::Enumerated);
    let expected = inventory["commits"]
        .as_array()
        .unwrap()
        .iter()
        .find(|commit| commit["oid"].as_str() == Some(boundary.as_str()))
        .unwrap();
    assert_changes_match(
        &boundary,
        &expected["changes"],
        &changes_of(&root, &boundary),
    );
    assert!(
        !changes_of(&root, &boundary).is_empty(),
        "the re-recorded commit must have the rows the boundary could not have"
    );
    assert_eq!(recorded_commits(&root).len(), order.len());
}

/// §8.5.2, end to end. A change insert that fails mid-commit leaves no commit row behind.
///
/// The fault is injected where a crash would land, by a trigger on `git_change` — no production hook,
/// and no reachable input can do it, which is the point: with the transaction in place there is
/// nothing left that can produce a commit claiming `enumerated` with no rows.
#[test]
fn a_failure_between_the_commit_and_its_changes_records_neither() {
    let inventory = common::history_inventory("history-basic");
    let head = inventory["head_oid"].as_str().unwrap();
    let path = inventory["commits"]
        .as_array()
        .unwrap()
        .iter()
        .find(|commit| commit["oid"].as_str() == Some(head))
        .unwrap()["changes"][0]["path"]
        .as_str()
        .unwrap()
        .to_string();

    // The control: without the trigger, this commit and this path are recorded.
    let (_dir, control) = common::history_fixture("history-basic");
    let outcome = ingest(&control, &HistoryOptions::default());
    assert!(outcome.commits_recorded > 0);
    assert!(changes_of(&control, head)
        .iter()
        .any(|row| row.path == path));

    let (_dir, root) = common::history_fixture("history-basic");
    {
        let conn = common::open_db(&root);
        conn.execute_batch(&format!(
            "CREATE TRIGGER injected_failure BEFORE INSERT ON git_change
                 WHEN NEW.path = '{path}'
             BEGIN SELECT RAISE(ABORT, 'injected mid-commit failure'); END;"
        ))
        .unwrap();
    }

    let error = ingest_history(&root, &HistoryOptions::default())
        .expect_err("the failed insert must surface");
    assert!(
        error.to_string().contains("injected"),
        "the failure must be reported, not swallowed: {error}"
    );

    let conn = common::open_db(&root);
    let repo = nerve_store::repository(&conn).unwrap().unwrap().repo_id;
    let recorded = nerve_store::recorded_commit_oids(&conn, &repo).unwrap();
    assert!(
        !recorded.contains(head),
        "the commit was recorded without its changes, so the next sync would skip it"
    );
    assert_eq!(
        nerve_store::history_totals(&conn, &repo).unwrap().commits,
        0,
        "the walk records the tip first, so nothing at all should have survived"
    );
}

// ---- the bounds and the path guard, over synthetic bytes --------------------------------------

/// A one-commit repository whose root tree is `payload`.
fn synthetic_root_commit(tree_payload: &[u8]) -> (tempfile::TempDir, PathBuf) {
    let (dir, root, git_dir) = common::synthetic_repo();
    let tree = "1".repeat(40);
    let commit = "2".repeat(40);
    common::write_loose_object(&git_dir, &tree, "tree", tree_payload);
    common::write_loose_object(
        &git_dir,
        &commit,
        "commit",
        &common::commit_object(&tree, &[], "synthetic root"),
    );
    common::set_main(&git_dir, &commit);
    (dir, root)
}

/// The three refusals `safe_tree_name` can produce for a name the format reader lets through.
///
/// Each with its own nonzero count, per case, because the aggregate form of this assertion is what a
/// corrective slice on this project was written about. `fixtures/history-hostile` cannot exercise
/// these: the tree carrying its hostile names also carries a subtree called `..`, and 12a refuses
/// such a tree **whole**, so the names never arrive here. These bytes are synthetic for that reason
/// and no other.
#[test]
fn each_tree_entry_name_the_format_allows_and_the_guard_refuses_is_counted_by_form() {
    let blob = "3".repeat(40);
    let payload = common::tree_object(&[
        ("100644", b"ok.txt", &blob),
        ("100644", b"back\\slash.txt", &blob),
        ("100644", b"ctl\x01name.txt", &blob),
        ("100644", b"nl\nname.txt", &blob),
        ("100644", &[0xff, 0xfe, 0x80], &blob),
    ]);
    let (_dir, root) = synthetic_root_commit(&payload);
    let outcome = ingest(&root, &HistoryOptions::default());

    assert_eq!(outcome.commits_recorded, 1);
    assert_eq!(outcome.enumeration_of(ChangesEnumerated::Enumerated), 1);

    use nerve_index::discover::TreeNameRefusal;
    assert_eq!(
        outcome.refusals(TreeNameRefusal::Backslash.form()),
        1,
        "{:?}",
        outcome.refused
    );
    assert_eq!(
        outcome.refusals(TreeNameRefusal::ControlCharacter.form()),
        2,
        "0x01 and the newline are both C0: {:?}",
        outcome.refused
    );
    assert_eq!(
        outcome.refusals(TreeNameRefusal::NotUtf8.form()),
        1,
        "{:?}",
        outcome.refused
    );

    // The legitimate path is still recorded, so the guard refuses names rather than trees.
    let stored: Vec<String> = changes_of(&root, &"2".repeat(40))
        .into_iter()
        .map(|row| row.path)
        .collect();
    assert_eq!(stored, vec!["ok.txt".to_string()]);
}

/// The tree-entry bound, reached with real bytes rather than by lowering the constant.
#[test]
fn a_tree_with_more_entries_than_the_bound_is_refused() {
    let blob = "3".repeat(40);
    let mut over = Vec::new();
    for index in 0..=history::MAX_TREE_ENTRIES {
        over.extend_from_slice(&common::tree_object(&[(
            "100644",
            format!("f{index:07}").as_bytes(),
            &blob,
        )]));
    }
    let (_dir, root) = synthetic_root_commit(&over);
    let outcome = ingest(&root, &HistoryOptions::default());
    assert_eq!(outcome.commits_recorded, 1, "the commit is still recorded");
    assert_eq!(outcome.enumeration_of(ChangesEnumerated::Refused), 1);
    assert_eq!(outcome.refusals(history::form::TREE_TOO_LARGE), 1);
    assert!(changes_of(&root, &"2".repeat(40)).is_empty());
}

/// The per-commit change bound, likewise from real bytes: a tree inside the entry bound and past the
/// change bound.
#[test]
fn a_commit_with_more_changes_than_the_bound_is_refused() {
    let blob = "3".repeat(40);
    let mut over = Vec::new();
    for index in 0..=history::MAX_CHANGES_PER_COMMIT {
        over.extend_from_slice(&common::tree_object(&[(
            "100644",
            format!("f{index:07}").as_bytes(),
            &blob,
        )]));
    }
    let (_dir, root) = synthetic_root_commit(&over);
    let outcome = ingest(&root, &HistoryOptions::default());
    assert_eq!(outcome.commits_recorded, 1);
    assert_eq!(outcome.enumeration_of(ChangesEnumerated::Refused), 1);
    assert_eq!(outcome.refusals(history::form::CHANGES_TOO_MANY), 1);
    assert_eq!(outcome.refusals(history::form::TREE_TOO_LARGE), 0);
    assert!(changes_of(&root, &"2".repeat(40)).is_empty());
}

/// The recursion bound, in both directions: one level inside it records the file, one level past it
/// refuses the name and counts it.
#[test]
fn a_tree_nested_past_the_depth_bound_is_refused_at_the_bound() {
    use nerve_index::discover::{TreeNameRefusal, MAX_TREE_PATH_DEPTH};

    for (directories, expect_file) in [
        (MAX_TREE_PATH_DEPTH - 1, true),
        (MAX_TREE_PATH_DEPTH, false),
    ] {
        let (dir, root, git_dir) = common::synthetic_repo();
        let blob = "3".repeat(40);
        // The deepest tree holds the file; each level above it holds one subtree called `d`.
        let mut oid = format!("{:040x}", 0xf00);
        common::write_loose_object(
            &git_dir,
            &oid,
            "tree",
            &common::tree_object(&[("100644", b"file.txt", &blob)]),
        );
        for level in 0..directories {
            let parent = format!("{:040x}", 0x1000 + level);
            common::write_loose_object(
                &git_dir,
                &parent,
                "tree",
                &common::tree_object(&[("40000", b"d", &oid)]),
            );
            oid = parent;
        }
        let commit = "2".repeat(40);
        common::write_loose_object(
            &git_dir,
            &commit,
            "commit",
            &common::commit_object(&oid, &[], "deep"),
        );
        common::set_main(&git_dir, &commit);

        let outcome = ingest(&root, &HistoryOptions::default());
        let paths: Vec<String> = changes_of(&root, &commit)
            .into_iter()
            .map(|row| row.path)
            .collect();
        if expect_file {
            assert_eq!(paths.len(), 1, "{directories} directories: {paths:?}");
            assert_eq!(
                outcome.refusals(TreeNameRefusal::TooDeep.form()),
                0,
                "{directories} directories is inside the bound"
            );
        } else {
            assert!(paths.is_empty(), "{directories} directories: {paths:?}");
            assert_eq!(
                outcome.refusals(TreeNameRefusal::TooDeep.form()),
                1,
                "{directories} directories is past the bound: {:?}",
                outcome.refused
            );
        }
        drop(dir);
    }
}

/// A gitlink is recorded as a change to its own path and the submodule is never opened.
///
/// The commit the gitlink names is not in this object store — that is what a submodule is — so a
/// reader that followed it would report an absence it invented.
#[test]
fn a_gitlink_is_recorded_and_never_followed() {
    let blob = "3".repeat(40);
    let submodule = "4".repeat(40);
    let payload = common::tree_object(&[
        ("100644", b"ok.txt", &blob),
        ("160000", b"vendor", &submodule),
    ]);
    let (_dir, root) = synthetic_root_commit(&payload);
    let outcome = ingest(&root, &HistoryOptions::default());

    assert_eq!(outcome.enumeration_of(ChangesEnumerated::Enumerated), 1);
    assert_eq!(
        outcome.refusals(history::form::TREE_ABSENT),
        0,
        "the submodule commit was looked for: {:?}",
        outcome.refused
    );
    let rows = changes_of(&root, &"2".repeat(40));
    let gitlink = rows
        .iter()
        .find(|row| row.path == "vendor")
        .expect("the gitlink path is recorded");
    assert_eq!(gitlink.change_kind, ChangeKind::Added);
    assert_eq!(gitlink.blob_oid.as_deref(), Some(submodule.as_str()));
    assert_eq!(gitlink.mode, Some(0o160_000));
    assert_eq!(rows.len(), 2, "and nothing under it: {rows:?}");
}

/// A tree naming the same entry twice is a decision, not a crash: the first is kept, the second is
/// refused and counted, and the primary key is never asked to hold both.
#[test]
fn a_repeated_tree_entry_name_is_refused_rather_than_colliding() {
    let first = "3".repeat(40);
    let second = "4".repeat(40);
    let payload = common::tree_object(&[
        ("100644", b"same.txt", &first),
        ("100644", b"same.txt", &second),
    ]);
    let (_dir, root) = synthetic_root_commit(&payload);
    let outcome = ingest(&root, &HistoryOptions::default());

    assert_eq!(outcome.refusals(history::form::DUPLICATE_PATH), 1);
    let rows = changes_of(&root, &"2".repeat(40));
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].blob_oid.as_deref(), Some(first.as_str()));
}

/// Identity is off by default and on by request, and when it is on it is bounded on the same terms as
/// the summary.
///
/// Off by default is a data-protection decision, not a performance one: no question the historical
/// model answers asks *who*. The `false` half of this test is what stops the flag from quietly
/// becoming the default.
#[test]
fn identity_is_absent_by_default_and_bounded_when_it_is_requested() {
    let (_dir, root) = common::history_fixture("history-basic");
    let outcome = ingest(
        &root,
        &HistoryOptions {
            with_identity: true,
            ..HistoryOptions::default()
        },
    );
    assert!(outcome.commits_recorded > 0);
    let inventory = common::history_inventory("history-basic");
    let stored = recorded_commits(&root);
    for commit in inventory["commits"].as_array().unwrap() {
        let oid = commit["oid"].as_str().unwrap();
        let row = &stored[oid];
        assert_eq!(
            row.author_ident.as_deref(),
            commit["author_ident"].as_str(),
            "{oid}"
        );
        assert_eq!(
            row.committer_ident.as_deref(),
            commit["committer_ident"].as_str(),
            "{oid}"
        );
    }
    assert_eq!(outcome.refusals(history::form::IDENT_TRUNCATED), 0);

    // The same repository without the flag stores nothing, so the columns are the flag's doing.
    let (_dir, off) = common::history_fixture("history-basic");
    ingest(&off, &HistoryOptions::default());
    assert!(recorded_commits(&off)
        .values()
        .all(|row| row.author_ident.is_none() && row.committer_ident.is_none()));

    // An over-long identity is truncated and counted, exactly as a summary is. Synthetic, because a
    // committed fixture may hold no developer identity at all, hostile or otherwise.
    let (_dir, long, git_dir) = common::synthetic_repo();
    let tree = "1".repeat(40);
    let commit = "2".repeat(40);
    common::write_loose_object(&git_dir, &tree, "tree", b"");
    let name = "n".repeat(history::MAX_IDENT_BYTES * 2);
    let mut payload = format!("tree {tree}\n").into_bytes();
    payload.extend_from_slice(
        format!(
            "author {name} <a@b> 1767225600 +0000\ncommitter {name} <a@b> 1767225600 +0000\n\nlong\n"
        )
        .as_bytes(),
    );
    common::write_loose_object(&git_dir, &commit, "commit", &payload);
    common::set_main(&git_dir, &commit);

    let outcome = ingest(
        &long,
        &HistoryOptions {
            with_identity: true,
            ..HistoryOptions::default()
        },
    );
    assert_eq!(
        outcome.refusals(history::form::IDENT_TRUNCATED),
        2,
        "author and committer are two strings: {:?}",
        outcome.refused
    );
    let row = &recorded_commits(&long)[&commit];
    assert_eq!(
        row.author_ident.as_deref().map(str::len),
        Some(history::MAX_IDENT_BYTES)
    );
    assert!(row.author_ident.as_deref().unwrap().starts_with("nnn"));
    assert_eq!(outcome.summaries_truncated, 0, "the summary is short");
}

/// An unborn branch is a success with nothing in it, and it is a different fact from a repository
/// that has never been synced.
#[test]
fn an_unborn_branch_is_recorded_as_a_success_with_no_commits() {
    let (_dir, root, _git_dir) = common::synthetic_repo();
    let outcome = ingest(&root, &HistoryOptions::default());

    assert_eq!(outcome.head_oid, None);
    assert!(outcome.walked_from.is_empty());
    assert_eq!(outcome.commits_recorded, 0);
    assert_eq!(outcome.commits_walked, 0);
    assert_eq!(outcome.walk_terminated_by, WalkTermination::Exhausted);
    assert!(outcome.refused.is_empty());

    let row = ingest_row(&root);
    assert_eq!(row.head_oid, None);
    assert_eq!(row.commits_recorded, 0);
    assert_eq!(row.walk_terminated_by, WalkTermination::Exhausted);
}

/// The `gitobj::Error` bridge keeps the closed-vocabulary tag, end to end.
#[test]
fn a_refusal_from_the_object_reader_reaches_the_caller_with_its_form() {
    let (_dir, root, git_dir) = common::synthetic_repo();
    std::fs::write(
        git_dir.join("config"),
        "[core]\n\trepositoryformatversion = 1\n[extensions]\n\tobjectFormat = sha256\n",
    )
    .unwrap();

    let error = ingest_history(&root, &HistoryOptions::default())
        .expect_err("a SHA-256 repository is refused rather than misread");
    assert_eq!(
        error.git_object_form(),
        Some(gitobj::form::UNSUPPORTED_OBJECT_FORMAT),
        "the tag must survive the bridge: {error}"
    );
    assert!(error
        .to_string()
        .contains(gitobj::form::UNSUPPORTED_OBJECT_FORMAT));

    // And the bridge is exhaustive about carrying the tag rather than special-casing one variant.
    for sample in [
        gitobj::Error::LooseUnknownType,
        gitobj::Error::IdxBadMagic,
        gitobj::Error::DeltaDepthExceeded,
    ] {
        let form = sample.form();
        let bridged = nerve_index::IndexError::from(sample);
        assert_eq!(bridged.git_object_form(), Some(form));
    }
    assert_eq!(
        nerve_index::IndexError::NotAFile(PathBuf::from("/x")).git_object_form(),
        None
    );
}
