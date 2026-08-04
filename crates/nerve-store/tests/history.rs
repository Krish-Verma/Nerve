//! The historical model at the storage boundary (schema v6, Slice 12b).
//!
//! Two properties are asserted over and over here because the slice is about them:
//!
//! - **An empty result is never allowed to be ambiguous.** Every test that asserts zero rows also
//!   asserts the stored qualifier that says *which* zero it is, and a nonzero tally somewhere in
//!   the same database, so no assertion of absence can pass because the write path was never
//!   reached.
//! - **Every order is total.** Ties on `committer_time` are guaranteed by fixtures with fixed
//!   synthetic dates, so a read whose order stopped at the timestamp would be a test waiting to
//!   flake.

use std::collections::BTreeMap;

use nerve_core::vocab::{
    ChangeKind, ChangesEnumerated, ParentCompleteness, RenameAmbiguity, RenameEvidence,
    WalkTermination,
};
use nerve_store::history::{
    changes_for_commit, commit_log, commits_touching_path, history_ingest, history_totals,
    insert_changes, insert_commit, insert_renames, recorded_commit_oids, renames_touching_path,
    ChangeRow, CommitRow, IngestRow, RenameRow,
};
use nerve_store::{migrate, open_in_memory, Connection};

/// A 40-character lowercase hex oid derived from a label.
///
/// Placeholder values satisfy their own field contracts. `git_commit.commit_oid` is documented as
/// 40 lowercase hex, so a fixture using `"c1"` would be exercising a shape no Git object has —
/// which is the `__GIT_COMMIT__` lesson from Slice 11b, where a placeholder that failed its own
/// field contract made a test assert nothing.
fn oid(label: &str) -> String {
    let mut hex: String = label.bytes().map(|byte| format!("{byte:02x}")).collect();
    hex.truncate(40);
    while hex.len() < 40 {
        hex.push('0');
    }
    hex
}

/// A migrated database with two repositories in it.
///
/// Two, always, because `repo_id` is on every one of the four tables and a read that forgot to
/// scope by it would still pass against a single-repository fixture.
fn store() -> Connection {
    let conn = open_in_memory().unwrap();
    migrate(&conn).unwrap();
    conn.execute("INSERT INTO repository VALUES ('r','p','/tmp/a','t')", [])
        .unwrap();
    conn.execute("INSERT INTO repository VALUES ('r2','p','/tmp/b','t')", [])
        .unwrap();
    conn
}

/// An ordinary non-merge commit with one available parent.
fn commit(label: &str, committer_time: i64) -> CommitRow {
    CommitRow {
        commit_oid: oid(label),
        tree_oid: oid(&format!("t{label}")),
        parent_oids: vec![oid(&format!("p{label}"))],
        parent_completeness: ParentCompleteness::ParentsAvailable,
        changes_enumerated: ChangesEnumerated::Enumerated,
        author_time: committer_time,
        author_tz: "+0000".to_string(),
        committer_time,
        committer_tz: "+0000".to_string(),
        author_ident: None,
        committer_ident: None,
        summary: format!("commit {label}"),
        is_merge: false,
    }
}

fn added(commit_oid: &str, path: &str) -> ChangeRow {
    ChangeRow {
        commit_oid: commit_oid.to_string(),
        path: path.to_string(),
        change_kind: ChangeKind::Added,
        blob_oid: Some(oid(&format!("b{path}"))),
        prev_blob_oid: None,
        mode: Some(0o100644),
        prev_mode: None,
    }
}

fn scalar(conn: &Connection, sql: &str) -> i64 {
    conn.query_row(sql, [], |row| row.get(0)).unwrap()
}

/// Every field of every struct survives storage, including the ones with no query behind them yet.
///
/// A round trip through `assert_eq!` on the whole struct rather than field by field: a column
/// dropped from an `INSERT` or read at the wrong index fails here, and a field added later without
/// being persisted fails here too.
#[test]
fn every_field_of_every_row_survives_the_round_trip() {
    let conn = store();

    // Deliberately awkward: several parents, a negative author time (a repository can carry a
    // commit dated before 1970), a non-UTC offset, identity present, and a summary that is
    // repository prose rather than an identifier.
    let written = CommitRow {
        commit_oid: oid("merge"),
        tree_oid: oid("tree"),
        parent_oids: vec![oid("pa"), oid("pb"), oid("pc")],
        parent_completeness: ParentCompleteness::ParentsAvailable,
        changes_enumerated: ChangesEnumerated::MergeNotEnumerated,
        author_time: -86_400,
        author_tz: "-0730".to_string(),
        committer_time: 1_700_000_000,
        committer_tz: "+0530".to_string(),
        author_ident: Some("A U Thor <a@example.invalid>".to_string()),
        committer_ident: Some("C O Mitter <c@example.invalid>".to_string()),
        summary: "fix: <script>alert(1)</script> and a trailing space ".to_string(),
        is_merge: true,
    };
    assert!(insert_commit(&conn, "r", &written).unwrap());
    let read = commit_log(&conn, "r", 10, 0).unwrap();
    assert_eq!(read, vec![written.clone()]);

    // A commit whose changes can be recorded, so every `ChangeKind` gets a row with the null
    // pattern its kind requires: no blob for a deletion, no previous blob for an addition.
    let host = commit("host", 1_700_000_100);
    assert!(insert_commit(&conn, "r", &host).unwrap());
    let changes = vec![
        ChangeRow {
            commit_oid: host.commit_oid.clone(),
            path: "src/added.ts".to_string(),
            change_kind: ChangeKind::Added,
            blob_oid: Some(oid("b1")),
            prev_blob_oid: None,
            mode: Some(0o100644),
            prev_mode: None,
        },
        ChangeRow {
            commit_oid: host.commit_oid.clone(),
            path: "src/deleted.ts".to_string(),
            change_kind: ChangeKind::Deleted,
            blob_oid: None,
            prev_blob_oid: Some(oid("b2")),
            mode: None,
            prev_mode: Some(0o100644),
        },
        ChangeRow {
            commit_oid: host.commit_oid.clone(),
            path: "src/mode.sh".to_string(),
            change_kind: ChangeKind::ModeChanged,
            blob_oid: Some(oid("b3")),
            prev_blob_oid: Some(oid("b3")),
            mode: Some(0o100755),
            prev_mode: Some(0o100644),
        },
        ChangeRow {
            commit_oid: host.commit_oid.clone(),
            path: "src/modified.ts".to_string(),
            change_kind: ChangeKind::Modified,
            blob_oid: Some(oid("b4")),
            prev_blob_oid: Some(oid("b5")),
            mode: Some(0o100644),
            prev_mode: Some(0o100644),
        },
    ];
    assert_eq!(insert_changes(&conn, "r", &changes).unwrap(), 4);
    // `changes` is already in path order, which is the order the read promises.
    assert_eq!(
        changes_for_commit(&conn, "r", &host.commit_oid).unwrap(),
        changes
    );

    let rename = RenameRow {
        commit_oid: host.commit_oid.clone(),
        from_path: "src/deleted.ts".to_string(),
        to_path: "src/added.ts".to_string(),
        evidence: RenameEvidence::ExactContent,
        blob_oid: oid("b1"),
        ambiguity: RenameAmbiguity::Unique,
    };
    assert_eq!(
        insert_renames(&conn, "r", std::slice::from_ref(&rename)).unwrap(),
        1
    );
    assert_eq!(
        renames_touching_path(&conn, "r", "src/deleted.ts", 10).unwrap(),
        vec![rename.clone()]
    );
    // The added side finds it too: a rename is a claim about both paths.
    assert_eq!(
        renames_touching_path(&conn, "r", "src/added.ts", 10).unwrap(),
        vec![rename]
    );

    let ingest = IngestRow {
        head_oid: Some(oid("head")),
        walked_from: vec![oid("head"), oid("tag")],
        commits_recorded: 2,
        commit_budget: 5_000,
        walk_terminated_by: WalkTermination::CommitBudget,
        shallow: true,
        shallow_boundary: vec![oid("boundary")],
        promisor: true,
        refusals: BTreeMap::from([
            ("history-commit-budget".to_string(), 1usize),
            ("history-tree-too-large".to_string(), 3usize),
        ]),
        reader_version: "gitobj-1.0.0".to_string(),
    };
    nerve_store::upsert_history_ingest(&conn, "r", &ingest).unwrap();
    assert_eq!(history_ingest(&conn, "r").unwrap(), Some(ingest));

    // And none of it leaked into the other repository.
    assert!(commit_log(&conn, "r2", 10, 0).unwrap().is_empty());
    assert!(history_ingest(&conn, "r2").unwrap().is_none());
    assert_eq!(scalar(&conn, "SELECT count(*) FROM git_commit"), 2);
}

/// The JSON columns round-trip in their **empty** forms, and empty is stored as a value.
///
/// `[]` and `{}` rather than `NULL`, because "this commit has no parents" and "nothing was refused"
/// are claims. A `NULL` would make them indistinguishable from "not recorded", which is the class
/// of ambiguity this schema exists to remove. The non-empty controls are in the same test so that
/// the empty assertions cannot pass against a write path that stores nothing at all.
#[test]
fn the_json_columns_round_trip_including_their_empty_forms() {
    let conn = store();

    let mut root = commit("root", 1_000);
    root.parent_oids = Vec::new();
    root.parent_completeness = ParentCompleteness::Root;
    assert!(insert_commit(&conn, "r", &root).unwrap());

    let child = commit("child", 2_000);
    assert!(insert_commit(&conn, "r", &child).unwrap());

    let read = commit_log(&conn, "r", 10, 0).unwrap();
    assert_eq!(read.len(), 2);
    // Newest first, so the child comes back before the root.
    assert!(read[0].parent_oids.len() == 1, "{:?}", read[0]);
    assert!(read[1].parent_oids.is_empty(), "{:?}", read[1]);

    let stored: String = conn
        .query_row(
            "SELECT parent_oids FROM git_commit WHERE commit_oid = ?1",
            [&root.commit_oid],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(
        stored, "[]",
        "an empty parent list must be a value, not NULL"
    );

    let empty = IngestRow {
        head_oid: None,
        walked_from: Vec::new(),
        commits_recorded: 0,
        commit_budget: 5_000,
        walk_terminated_by: WalkTermination::Exhausted,
        shallow: false,
        shallow_boundary: Vec::new(),
        promisor: false,
        refusals: BTreeMap::new(),
        reader_version: "gitobj-1.0.0".to_string(),
    };
    nerve_store::upsert_history_ingest(&conn, "r", &empty).unwrap();
    assert_eq!(history_ingest(&conn, "r").unwrap(), Some(empty));
    let (walked, boundary, refusals): (String, String, String) = conn
        .query_row(
            "SELECT walked_from, shallow_boundary, refusals FROM git_history_ingest
              WHERE repo_id = 'r'",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .unwrap();
    assert_eq!(
        (walked.as_str(), boundary.as_str(), refusals.as_str()),
        ("[]", "[]", "{}")
    );

    // The tally that stops the three assertions above from passing vacuously.
    assert_eq!(scalar(&conn, "SELECT count(*) FROM git_commit"), 2);
    assert_eq!(scalar(&conn, "SELECT count(*) FROM git_history_ingest"), 1);
}

/// Recording the same commit twice writes one row, and the second call says so.
///
/// The licence for `INSERT OR IGNORE` here is that a commit oid is the hash of an immutable object,
/// so the same oid is the same commit. The return value is what the walk uses to decide whether to
/// enumerate the commit's changes, so it has to be right in both directions.
#[test]
fn recording_the_same_commit_twice_writes_one_row_and_reports_which() {
    let conn = store();
    let first = commit("one", 1_000);

    assert!(
        insert_commit(&conn, "r", &first).unwrap(),
        "the first insert must report that it wrote"
    );
    assert!(
        !insert_commit(&conn, "r", &first).unwrap(),
        "the second insert must report that it did not"
    );
    assert!(!insert_commit(&conn, "r", &first).unwrap());
    assert_eq!(scalar(&conn, "SELECT count(*) FROM git_commit"), 1);

    // The recorded row is the first one. A silent `REPLACE` would have passed the count above.
    assert_eq!(commit_log(&conn, "r", 10, 0).unwrap(), vec![first.clone()]);

    // A commit that differs only in oid is a different commit, so the ignore is keyed on identity
    // rather than being an unconditional swallow.
    let second = commit("two", 1_000);
    assert!(insert_commit(&conn, "r", &second).unwrap());
    assert_eq!(scalar(&conn, "SELECT count(*) FROM git_commit"), 2);

    // The same oid in a different repository is a different row: the key is composite.
    assert!(insert_commit(&conn, "r2", &first).unwrap());
    assert_eq!(scalar(&conn, "SELECT count(*) FROM git_commit"), 3);
    assert_eq!(recorded_commit_oids(&conn, "r2").unwrap().len(), 1);
    assert_eq!(recorded_commit_oids(&conn, "r").unwrap().len(), 2);
}

/// **A commit with no change rows always says which silence it is.**
///
/// Four commits, three of them with zero changes for three different reasons, and one control with
/// changes so the fixture is demonstrably capable of producing them. `changes_for_commit` returning
/// an empty vector is asserted *together with* the qualifier every time, because on its own the
/// empty vector is the one thing this schema refuses to let a consumer interpret.
#[test]
fn a_commit_with_no_changes_states_which_of_the_four_silences_it_is() {
    let conn = store();

    let mut with_changes = commit("full", 4_000);
    with_changes.changes_enumerated = ChangesEnumerated::Enumerated;
    assert!(insert_commit(&conn, "r", &with_changes).unwrap());
    assert_eq!(
        insert_changes(&conn, "r", &[added(&with_changes.commit_oid, "src/a.ts")]).unwrap(),
        1
    );

    let mut empty = commit("empty", 3_000);
    empty.changes_enumerated = ChangesEnumerated::Enumerated;
    assert!(insert_commit(&conn, "r", &empty).unwrap());

    let mut boundary = commit("bound", 2_000);
    boundary.parent_completeness = ParentCompleteness::ShallowBoundary;
    boundary.changes_enumerated = ChangesEnumerated::ParentUnavailable;
    assert!(insert_commit(&conn, "r", &boundary).unwrap());

    let mut refused = commit("refus", 1_000);
    refused.changes_enumerated = ChangesEnumerated::Refused;
    assert!(insert_commit(&conn, "r", &refused).unwrap());

    // The control: this fixture can write change rows, so the three empties below mean something.
    assert_eq!(
        changes_for_commit(&conn, "r", &with_changes.commit_oid)
            .unwrap()
            .len(),
        1
    );

    let by_oid: BTreeMap<String, CommitRow> = commit_log(&conn, "r", 10, 0)
        .unwrap()
        .into_iter()
        .map(|row| (row.commit_oid.clone(), row))
        .collect();

    for (row, expected) in [
        (&empty, ChangesEnumerated::Enumerated),
        (&boundary, ChangesEnumerated::ParentUnavailable),
        (&refused, ChangesEnumerated::Refused),
    ] {
        let changes = changes_for_commit(&conn, "r", &row.commit_oid).unwrap();
        assert!(
            changes.is_empty(),
            "{:?} unexpectedly had changes",
            row.summary
        );
        assert_eq!(
            by_oid[&row.commit_oid].changes_enumerated, expected,
            "zero change rows without the qualifier that explains them"
        );
    }

    // Only `enumerated` permits reading zero rows as "nothing changed", and only one of these
    // three is that.
    assert_eq!(
        [&empty, &boundary, &refused]
            .iter()
            .filter(
                |row| by_oid[&row.commit_oid].changes_enumerated == ChangesEnumerated::Enumerated
            )
            .count(),
        1
    );
    // A shallow boundary is not a root, so nothing here may claim history begins at it.
    assert!(!by_oid[&boundary.commit_oid]
        .parent_completeness
        .may_claim_history_begins_here());

    let totals = history_totals(&conn, "r").unwrap();
    assert_eq!(totals.commits, 4);
    assert_eq!(
        totals.changes, 1,
        "the tally that makes the empties legible"
    );
}

/// A merge with no changes and an empty commit with no changes are different facts.
///
/// Both have zero `git_change` rows. Everything that distinguishes them is stored: the qualifier,
/// the merge flag, and the parent count. If any one of the three were derived from the row count
/// instead, this test would collapse.
#[test]
fn a_merge_with_no_changes_is_distinguishable_from_an_empty_commit() {
    let conn = store();

    let mut merge = commit("merge", 2_000);
    merge.parent_oids = vec![oid("pa"), oid("pb")];
    merge.is_merge = true;
    merge.changes_enumerated = ChangesEnumerated::MergeNotEnumerated;
    assert!(insert_commit(&conn, "r", &merge).unwrap());

    let mut empty = commit("empty", 1_000);
    empty.changes_enumerated = ChangesEnumerated::Enumerated;
    assert!(insert_commit(&conn, "r", &empty).unwrap());

    // A third commit with changes, so "zero rows" is a property of these two rather than of the
    // fixture.
    let full = commit("full", 3_000);
    assert!(insert_commit(&conn, "r", &full).unwrap());
    assert_eq!(
        insert_changes(&conn, "r", &[added(&full.commit_oid, "src/a.ts")]).unwrap(),
        1
    );

    assert!(changes_for_commit(&conn, "r", &merge.commit_oid)
        .unwrap()
        .is_empty());
    assert!(changes_for_commit(&conn, "r", &empty.commit_oid)
        .unwrap()
        .is_empty());

    let log = commit_log(&conn, "r", 10, 0).unwrap();
    let read_merge = log
        .iter()
        .find(|c| c.commit_oid == merge.commit_oid)
        .unwrap();
    let read_empty = log
        .iter()
        .find(|c| c.commit_oid == empty.commit_oid)
        .unwrap();

    assert_ne!(read_merge.changes_enumerated, read_empty.changes_enumerated);
    assert_eq!(
        read_merge.changes_enumerated,
        ChangesEnumerated::MergeNotEnumerated
    );
    assert_eq!(read_empty.changes_enumerated, ChangesEnumerated::Enumerated);
    assert!(read_merge.is_merge);
    assert!(!read_empty.is_merge);
    assert_eq!(read_merge.parent_oids.len(), 2);
    assert_eq!(read_empty.parent_oids.len(), 1);

    let totals = history_totals(&conn, "r").unwrap();
    assert_eq!(totals.commits, 3);
    assert_eq!(totals.merges, 1, "the merge must be counted, not skipped");
    assert_eq!(totals.changes, 1);
}

/// All five parent-completeness values round-trip, and exactly one of them means the beginning.
///
/// Tested over `ParentCompleteness::ALL` rather than over a written-out list, so a sixth value
/// added to the vocabulary fails here until it can be stored and read back — the storage half of
/// the exhaustiveness check `nerve-core` does on the vocabulary itself.
#[test]
fn every_parent_completeness_value_round_trips_through_the_database() {
    let conn = store();

    for (index, value) in ParentCompleteness::ALL.into_iter().enumerate() {
        let mut row = commit(&format!("pc{index}"), 1_000 + index as i64);
        row.parent_completeness = value;
        if value == ParentCompleteness::Root {
            row.parent_oids = Vec::new();
        }
        assert!(insert_commit(&conn, "r", &row).unwrap());
    }

    let log = commit_log(&conn, "r", 100, 0).unwrap();
    assert_eq!(log.len(), ParentCompleteness::ALL.len());

    let mut read: Vec<ParentCompleteness> = log.iter().map(|row| row.parent_completeness).collect();
    read.sort_unstable();
    let mut all = ParentCompleteness::ALL.to_vec();
    all.sort_unstable();
    assert_eq!(read, all, "a value did not survive storage");

    assert_eq!(
        log.iter()
            .filter(|row| row.parent_completeness.may_claim_history_begins_here())
            .count(),
        1,
        "exactly one recorded commit may be called the beginning of the history"
    );
    // The two that mean "cannot see further" are stored apart, so a fault is never reported as a
    // shallow boundary.
    assert_eq!(
        scalar(
            &conn,
            "SELECT count(DISTINCT parent_completeness) FROM git_commit
              WHERE parent_completeness IN
                    ('shallow_boundary','parents_missing','parents_unverifiable')"
        ),
        3
    );
}

/// An ambiguous exact-content match records **every** pairing and promotes none.
///
/// One deleted path whose blob turns up at three added paths. There is no tie-break to apply and no
/// score to compute, so all three pairings are stored with `many_to` and the read returns all three.
/// The unique case is written in the same database as a control: without it, "no pairing is
/// `unique`" would also be satisfied by a write path that could not produce `unique` at all.
#[test]
fn an_ambiguous_rename_records_every_pairing_and_promotes_none() {
    let conn = store();
    let ambiguous = commit("amb", 2_000);
    assert!(insert_commit(&conn, "r", &ambiguous).unwrap());
    let unique = commit("uniq", 1_000);
    assert!(insert_commit(&conn, "r", &unique).unwrap());

    let shared = oid("shared");
    let pairings: Vec<RenameRow> = ["src/one.ts", "src/two.ts", "src/three.ts"]
        .into_iter()
        .map(|to_path| RenameRow {
            commit_oid: ambiguous.commit_oid.clone(),
            from_path: "src/gone.ts".to_string(),
            to_path: to_path.to_string(),
            evidence: RenameEvidence::ExactContent,
            blob_oid: shared.clone(),
            ambiguity: RenameAmbiguity::ManyTo,
        })
        .collect();
    assert_eq!(insert_renames(&conn, "r", &pairings).unwrap(), 3);

    let control = RenameRow {
        commit_oid: unique.commit_oid.clone(),
        from_path: "docs/old.md".to_string(),
        to_path: "docs/new.md".to_string(),
        evidence: RenameEvidence::ExactContent,
        blob_oid: oid("solo"),
        ambiguity: RenameAmbiguity::Unique,
    };
    assert_eq!(
        insert_renames(&conn, "r", std::slice::from_ref(&control)).unwrap(),
        1
    );

    let found = renames_touching_path(&conn, "r", "src/gone.ts", 10).unwrap();
    assert_eq!(found.len(), 3, "a pairing was dropped: {found:?}");
    for row in &found {
        assert_eq!(
            row.ambiguity,
            RenameAmbiguity::ManyTo,
            "a pairing was promoted out of ambiguity"
        );
        assert_eq!(row.evidence, RenameEvidence::ExactContent);
        assert_eq!(row.blob_oid, shared);
    }
    // Every added path is reachable from its own side too, and none of them was chosen.
    for to_path in ["src/one.ts", "src/two.ts", "src/three.ts"] {
        let side = renames_touching_path(&conn, "r", to_path, 10).unwrap();
        assert_eq!(side.len(), 1);
        assert_eq!(side[0].ambiguity, RenameAmbiguity::ManyTo);
    }
    // The order within the ambiguous commit is total, so the three come back the same way twice.
    let again = renames_touching_path(&conn, "r", "src/gone.ts", 10).unwrap();
    assert_eq!(found, again);
    let to_paths: Vec<&str> = found.iter().map(|row| row.to_path.as_str()).collect();
    assert_eq!(to_paths, vec!["src/one.ts", "src/three.ts", "src/two.ts"]);

    // The control, which proves `unique` is reachable and is what makes the assertions above
    // about `many_to` a measurement rather than a tautology.
    let unique_found = renames_touching_path(&conn, "r", "docs/old.md", 10).unwrap();
    assert_eq!(unique_found, vec![control]);

    let totals = history_totals(&conn, "r").unwrap();
    assert_eq!(totals.renames, 4);
    assert_eq!(
        scalar(
            &conn,
            "SELECT count(*) FROM git_rename_hypothesis WHERE ambiguity = 'unique'"
        ),
        1,
        "exactly one pairing in this database is unambiguous"
    );
}

/// **The `commit_oid` tiebreak is load-bearing.**
///
/// Four commits share a `committer_time`, which is the normal case for a fixture built with fixed
/// synthetic dates. Their insertion order is **neither ascending nor descending** by oid, and that
/// detail is the whole test: `idx_git_commit_time(repo_id, committer_time)` lets SQLite satisfy
/// `ORDER BY committer_time DESC` by scanning the index backwards, which returns tied rows in
/// *reverse rowid* order. A fixture inserted in descending oid order would therefore come back in
/// ascending oid order by accident, and the test would pass with the tiebreak deleted — which is
/// what a first attempt at this test did, verified by removing it.
#[test]
fn the_commit_log_order_is_total_when_committer_times_tie() {
    let conn = store();
    let tied = 1_700_000_000;

    // Insertion order bb, dd, aa, cc: rowid order is bb,dd,aa,cc and reverse rowid order is
    // cc,aa,dd,bb. Neither is the promised aa,bb,cc,dd, so no scan direction can fake it.
    for label in ["bb", "dd", "aa", "cc"] {
        assert!(insert_commit(&conn, "r", &commit(label, tied)).unwrap());
    }
    // One commit at a different time, so the primary sort key is exercised as well as the tiebreak.
    assert!(insert_commit(&conn, "r", &commit("newest", tied + 1)).unwrap());

    let expected = vec![oid("newest"), oid("aa"), oid("bb"), oid("cc"), oid("dd")];
    for attempt in 0..5 {
        let order: Vec<String> = commit_log(&conn, "r", 10, 0)
            .unwrap()
            .into_iter()
            .map(|row| row.commit_oid)
            .collect();
        assert_eq!(order, expected, "order changed on attempt {attempt}");
    }

    // The tie is real: four of the five rows share a committer_time.
    assert_eq!(
        scalar(
            &conn,
            &format!("SELECT count(*) FROM git_commit WHERE committer_time = {tied}")
        ),
        4,
        "no tie to break; the test would prove nothing"
    );

    // Paging is stable against the same total order, so page 2 does not repeat page 1.
    assert_eq!(
        commit_log(&conn, "r", 2, 0)
            .unwrap()
            .into_iter()
            .map(|row| row.commit_oid)
            .collect::<Vec<_>>(),
        expected[..2].to_vec()
    );
    assert_eq!(
        commit_log(&conn, "r", 3, 2)
            .unwrap()
            .into_iter()
            .map(|row| row.commit_oid)
            .collect::<Vec<_>>(),
        expected[2..].to_vec()
    );
    assert!(commit_log(&conn, "r", 10, 5).unwrap().is_empty());
}

/// Commits that touched a path come back newest first, each with what it did to that path.
///
/// The order is total under a tie for the same reason `commit_log`'s is, and the pairing is the
/// point: the change cannot be dated on its own and the commit cannot say what happened on its own.
#[test]
fn commits_touching_a_path_are_paired_with_what_they_did_in_a_total_order() {
    let conn = store();
    let tied = 1_700_000_000;
    let path = "src/app.ts";

    // Three commits at the same time touching the same path, plus an older one. Insertion order is
    // neither ascending nor descending by oid, for the reason spelled out in
    // `the_commit_log_order_is_total_when_committer_times_tie`: a reverse index scan would
    // otherwise reproduce the promised order by accident.
    for (label, kind) in [
        ("bb", ChangeKind::Deleted),
        ("dd", ChangeKind::ModeChanged),
        ("aa", ChangeKind::Modified),
        ("zz", ChangeKind::Added),
    ] {
        let time = if label == "zz" { tied - 1 } else { tied };
        let row = commit(label, time);
        assert!(insert_commit(&conn, "r", &row).unwrap());
        let change = ChangeRow {
            commit_oid: row.commit_oid.clone(),
            path: path.to_string(),
            change_kind: kind,
            blob_oid: match kind {
                ChangeKind::Deleted => None,
                _ => Some(oid("blob")),
            },
            prev_blob_oid: match kind {
                ChangeKind::Added => None,
                _ => Some(oid("prev")),
            },
            mode: Some(0o100644),
            prev_mode: Some(0o100644),
        };
        assert_eq!(insert_changes(&conn, "r", &[change]).unwrap(), 1);
    }

    let found = commits_touching_path(&conn, "r", path, 10).unwrap();
    assert_eq!(
        found
            .iter()
            .map(|(c, ch)| (c.commit_oid.clone(), ch.change_kind))
            .collect::<Vec<_>>(),
        vec![
            (oid("aa"), ChangeKind::Modified),
            (oid("bb"), ChangeKind::Deleted),
            (oid("dd"), ChangeKind::ModeChanged),
            (oid("zz"), ChangeKind::Added),
        ]
    );
    // Repeated because a total order has to be stable, not merely correct once.
    assert_eq!(commits_touching_path(&conn, "r", path, 10).unwrap(), found);
    for (commit_row, change) in &found {
        assert_eq!(&change.commit_oid, &commit_row.commit_oid);
        assert_eq!(change.path, path);
    }
    assert_eq!(commits_touching_path(&conn, "r", path, 2).unwrap().len(), 2);

    // A path nothing touched returns nothing — asserted beside a nonzero tally, so it cannot pass
    // because the write path was never reached.
    assert!(commits_touching_path(&conn, "r", "src/never.ts", 10)
        .unwrap()
        .is_empty());
    assert_eq!(history_totals(&conn, "r").unwrap().changes, 4);
    // And a path in the other repository is not this repository's path.
    assert!(commits_touching_path(&conn, "r2", path, 10)
        .unwrap()
        .is_empty());
}

/// Totals carry every change kind, including the ones with no rows.
///
/// An absent key and a zero are the same fact, and a caller should not have to know which it got.
/// Carrying all of them also means a kind added to `ChangeKind::ALL` appears in the map without
/// every consumer being edited — and the length check is what fails if one is dropped.
#[test]
fn totals_carry_every_change_kind_including_the_absent_ones() {
    let conn = store();
    let host = commit("host", 1_000);
    assert!(insert_commit(&conn, "r", &host).unwrap());
    assert_eq!(
        insert_changes(
            &conn,
            "r",
            &[
                added(&host.commit_oid, "src/a.ts"),
                added(&host.commit_oid, "src/b.ts"),
                ChangeRow {
                    commit_oid: host.commit_oid.clone(),
                    path: "src/c.ts".to_string(),
                    change_kind: ChangeKind::Deleted,
                    blob_oid: None,
                    prev_blob_oid: Some(oid("b")),
                    mode: None,
                    prev_mode: Some(0o100644),
                },
            ]
        )
        .unwrap(),
        3
    );

    let totals = history_totals(&conn, "r").unwrap();
    assert_eq!(totals.changes_by_kind.len(), ChangeKind::ALL.len());
    assert_eq!(totals.changes_by_kind[&ChangeKind::Added], 2);
    assert_eq!(totals.changes_by_kind[&ChangeKind::Deleted], 1);
    assert_eq!(
        totals.changes_by_kind[&ChangeKind::Modified],
        0,
        "an absent kind must be present with a zero, not missing"
    );
    assert_eq!(totals.changes_by_kind[&ChangeKind::ModeChanged], 0);
    assert_eq!(
        totals.changes_by_kind.values().sum::<i64>(),
        totals.changes,
        "the per-kind counts must account for every change row"
    );
    assert_eq!(totals.commits, 1);
    assert_eq!(totals.merges, 0);
    assert_eq!(totals.renames, 0);

    // The other repository's totals are its own, all zero, with every key still present.
    let other = history_totals(&conn, "r2").unwrap();
    assert_eq!(other.changes, 0);
    assert_eq!(other.changes_by_kind.len(), ChangeKind::ALL.len());
}

/// "No ingest has run" and "an ingest ran and found nothing" are different answers.
///
/// `None` is the first; a row with `commits_recorded = 0` and a `WalkTermination` saying why is the
/// second. A caller that folded them together would report an un-ingested repository as one with no
/// history, which is the same error class as reading a shallow boundary as a root.
#[test]
fn no_ingest_and_an_ingest_that_found_nothing_are_different_answers() {
    let conn = store();
    assert!(history_ingest(&conn, "r").unwrap().is_none());

    // An unborn branch: no HEAD, nothing walked, and the walk ran to completion doing so.
    let unborn = IngestRow {
        head_oid: None,
        walked_from: Vec::new(),
        commits_recorded: 0,
        commit_budget: 5_000,
        walk_terminated_by: WalkTermination::Exhausted,
        shallow: false,
        shallow_boundary: Vec::new(),
        promisor: false,
        refusals: BTreeMap::new(),
        reader_version: "gitobj-1.0.0".to_string(),
    };
    nerve_store::upsert_history_ingest(&conn, "r", &unborn).unwrap();
    let read = history_ingest(&conn, "r").unwrap().expect("recorded");
    assert_eq!(read, unborn);
    assert_eq!(read.commits_recorded, 0);
    assert_eq!(read.walk_terminated_by, WalkTermination::Exhausted);
    // The distinguishing fact: `Some(row with 0)` rather than `None`.
    assert!(history_ingest(&conn, "r").unwrap().is_some());
    assert!(history_ingest(&conn, "r2").unwrap().is_none());

    // A bounded ingest of a complete repository is not a shallow one, and the replacement keeps one
    // row per repository.
    let bounded = IngestRow {
        head_oid: Some(oid("head")),
        walked_from: vec![oid("head")],
        commits_recorded: 50,
        commit_budget: 50,
        walk_terminated_by: WalkTermination::CommitBudget,
        shallow: false,
        shallow_boundary: Vec::new(),
        promisor: false,
        refusals: BTreeMap::from([("history-commit-budget".to_string(), 1usize)]),
        reader_version: "gitobj-1.0.0".to_string(),
    };
    nerve_store::upsert_history_ingest(&conn, "r", &bounded).unwrap();
    assert_eq!(history_ingest(&conn, "r").unwrap(), Some(bounded));
    assert_eq!(scalar(&conn, "SELECT count(*) FROM git_history_ingest"), 1);
    let replaced = history_ingest(&conn, "r").unwrap().unwrap();
    assert!(!replaced.shallow, "a budget is not a shallow boundary");
    assert_eq!(replaced.walk_terminated_by, WalkTermination::CommitBudget);
    // `ingested_at` is stamped by the store, so the row is dated even though `IngestRow` has no
    // field for it. That the column is unreadable through this API is recorded in the slice report.
    let stamped: String = conn
        .query_row(
            "SELECT ingested_at FROM git_history_ingest WHERE repo_id = 'r'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert!(stamped.ends_with('Z') && stamped.len() >= 20, "{stamped}");
}

/// A change for a commit that was never recorded is refused, through the public write path.
///
/// The foreign key is what makes `insert_changes`' plain `INSERT` worth having. Under
/// `INSERT OR IGNORE` — the shape that cost Slice 3b a silent data loss — this row would vanish and
/// the commit would read as one that did not touch the path.
#[test]
fn insert_changes_refuses_a_change_for_an_unrecorded_commit() {
    let conn = store();
    let recorded = commit("rec", 1_000);
    assert!(insert_commit(&conn, "r", &recorded).unwrap());

    // The control: the same shape against the recorded commit is accepted.
    assert_eq!(
        insert_changes(&conn, "r", &[added(&recorded.commit_oid, "src/a.ts")]).unwrap(),
        1
    );

    let orphan = added(&oid("absent"), "src/b.ts");
    let err = insert_changes(&conn, "r", &[orphan]).unwrap_err();
    assert!(
        err.to_string().to_lowercase().contains("foreign key"),
        "expected a foreign-key refusal, got {err}"
    );
    assert_eq!(
        scalar(&conn, "SELECT count(*) FROM git_change"),
        1,
        "the refused row must not have landed"
    );

    // The commit exists in `r` but not in `r2`, so the key really is composite.
    let err = insert_changes(&conn, "r2", &[added(&recorded.commit_oid, "src/a.ts")]).unwrap_err();
    assert!(
        err.to_string().to_lowercase().contains("foreign key"),
        "{err}"
    );

    // And the same for a hypothesis.
    let orphan = RenameRow {
        commit_oid: oid("absent"),
        from_path: "a".to_string(),
        to_path: "b".to_string(),
        evidence: RenameEvidence::ExactContent,
        blob_oid: oid("blob"),
        ambiguity: RenameAmbiguity::Unique,
    };
    let err = insert_renames(&conn, "r", &[orphan]).unwrap_err();
    assert!(
        err.to_string().to_lowercase().contains("foreign key"),
        "expected a foreign-key refusal, got {err}"
    );
    assert_eq!(
        scalar(&conn, "SELECT count(*) FROM git_rename_hypothesis"),
        0
    );
}

/// Re-supplying a recorded commit's changes is an **error**, not a silent no-op.
///
/// This is the direct assertion that `insert_changes` is not `INSERT OR IGNORE`. The caller's
/// contract — only enumerate changes for a commit `insert_commit` said was new — is what makes a
/// conflict here impossible in normal operation, so a conflict means the contract was broken and
/// silence would hide it.
#[test]
fn a_repeated_change_row_is_an_error_rather_than_a_silent_drop() {
    let conn = store();
    let host = commit("host", 1_000);
    assert!(insert_commit(&conn, "r", &host).unwrap());
    let change = added(&host.commit_oid, "src/a.ts");
    assert_eq!(
        insert_changes(&conn, "r", std::slice::from_ref(&change)).unwrap(),
        1
    );

    let err = insert_changes(&conn, "r", std::slice::from_ref(&change)).unwrap_err();
    assert!(
        err.to_string().to_lowercase().contains("unique")
            || err.to_string().to_lowercase().contains("constraint"),
        "expected a primary-key violation, got {err}"
    );
    assert_eq!(scalar(&conn, "SELECT count(*) FROM git_change"), 1);

    // A batch that conflicts part way through reports the error rather than the rows it managed.
    let err =
        insert_changes(&conn, "r", &[added(&host.commit_oid, "src/new.ts"), change]).unwrap_err();
    assert!(
        err.to_string().to_lowercase().contains("constraint"),
        "{err}"
    );

    // The same for a hypothesis.
    let rename = RenameRow {
        commit_oid: host.commit_oid.clone(),
        from_path: "src/old.ts".to_string(),
        to_path: "src/a.ts".to_string(),
        evidence: RenameEvidence::ExactContent,
        blob_oid: oid("blob"),
        ambiguity: RenameAmbiguity::Unique,
    };
    assert_eq!(
        insert_renames(&conn, "r", std::slice::from_ref(&rename)).unwrap(),
        1
    );
    assert!(insert_renames(&conn, "r", &[rename]).is_err());
    assert_eq!(
        scalar(&conn, "SELECT count(*) FROM git_rename_hypothesis"),
        1
    );
}

/// A stored value outside a closed vocabulary is **refused on read**, never defaulted.
///
/// The columns are `TEXT` with no `CHECK`, because the vocabulary is closed in Rust rather than in
/// SQL. That makes the read the enforcement point, and the failure mode a silent default would
/// produce is exactly the one this schema exists to prevent: an unrecognised `parent_completeness`
/// quietly becoming `root` would state that the project's history begins at a commit nobody
/// classified.
#[test]
fn a_malformed_historical_row_is_refused_rather_than_defaulted() {
    let conn = store();
    let good = commit("good", 2_000);
    assert!(insert_commit(&conn, "r", &good).unwrap());
    // The control: the well-formed row reads back.
    assert_eq!(commit_log(&conn, "r", 10, 0).unwrap().len(), 1);

    conn.execute(
        "INSERT INTO git_commit VALUES
             ('r', ?1, 'aa', '[]', 'not_a_vocabulary_value', 'enumerated',
              1, '+0000', 1, '+0000', NULL, NULL, 'bad', 0)",
        [&oid("badvocab")],
    )
    .unwrap();
    let err = commit_log(&conn, "r", 10, 0).unwrap_err();
    assert!(
        matches!(err, nerve_store::StoreError::Core(_)),
        "expected a vocabulary refusal, got {err}"
    );

    conn.execute(
        "DELETE FROM git_commit WHERE commit_oid = ?1",
        [&oid("badvocab")],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO git_commit VALUES
             ('r', ?1, 'aa', 'not json', 'root', 'enumerated',
              1, '+0000', 1, '+0000', NULL, NULL, 'bad', 0)",
        [&oid("badjson")],
    )
    .unwrap();
    let err = commit_log(&conn, "r", 10, 0).unwrap_err();
    assert!(
        matches!(err, nerve_store::StoreError::Json { .. }),
        "expected a JSON refusal, got {err}"
    );

    // A malformed change kind is refused by the totals query too, not only by the row read.
    conn.execute(
        "DELETE FROM git_commit WHERE commit_oid = ?1",
        [&oid("badjson")],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO git_change VALUES ('r', ?1, 'src/a.ts', 'renamed', 'bb', NULL, 33188, NULL)",
        [&good.commit_oid],
    )
    .unwrap();
    assert!(matches!(
        changes_for_commit(&conn, "r", &good.commit_oid).unwrap_err(),
        nerve_store::StoreError::Core(_)
    ));
    assert!(matches!(
        history_totals(&conn, "r").unwrap_err(),
        nerve_store::StoreError::Core(_)
    ));
}
