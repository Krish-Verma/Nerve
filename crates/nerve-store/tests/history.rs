//! The historical model at the storage boundary (schema v6, Slice 12b; v7, Slice 12c-ii).
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

use std::collections::{BTreeMap, BTreeSet};

use nerve_core::vocab::{
    ChangeKind, ChangesEnumerated, FirstObservedKind, HistoryFreshness, ParentCompleteness,
    RenameAmbiguity, RenameAnalysisCompleteness, RenameEvidence, SimilarityUnmeasured,
    SummaryTruncation, WalkTermination,
};
use nerve_store::history::{
    change_frequency, changes_for_commit, cochange, commit_by_oid, commit_log,
    commits_touching_path, earlier_changes_may_exist, first_last_observed, history_freshness,
    history_ingest, history_totals, insert_changes, insert_commit, insert_rename_analysis,
    insert_renames, recorded_commit_oids, rename_analysis_for_commits, renames_touching_path,
    state_diff, AnalysisRow, ChangeRow, CommitRow, EarlierHistoryUnavailable, IngestRow, RenameRow,
    StateDiff, StateDiffLimits, COCHANGE_IS_NOT_A_DEPENDENCY, CURRENT_TREE_BASIS,
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
        summary_truncation: SummaryTruncation::Complete,
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
    //
    // `summary_truncation` is the non-default value on purpose. It is decided by the writer that
    // still holds the untruncated first line — `nerve-index`, not this layer — so the storage
    // boundary's obligation is to carry back whatever it was given rather than to re-derive it
    // from the stored length, which is exactly what it cannot do.
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
        summary_truncation: SummaryTruncation::Truncated,
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
        from_blob_oid: oid("b1"),
        to_blob_oid: oid("b1"),
        matcher_id: "git-blob-oid".to_string(),
        matcher_version: "1".to_string(),
        match_numerator: None,
        match_denominator: None,
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
            from_blob_oid: shared.clone(),
            to_blob_oid: shared.clone(),
            matcher_id: "git-blob-oid".to_string(),
            matcher_version: "1".to_string(),
            match_numerator: None,
            match_denominator: None,
            ambiguity: RenameAmbiguity::ManyTo,
        })
        .collect();
    assert_eq!(insert_renames(&conn, "r", &pairings).unwrap(), 3);

    let control = RenameRow {
        commit_oid: unique.commit_oid.clone(),
        from_path: "docs/old.md".to_string(),
        to_path: "docs/new.md".to_string(),
        evidence: RenameEvidence::ExactContent,
        from_blob_oid: oid("solo"),
        to_blob_oid: oid("solo"),
        matcher_id: "git-blob-oid".to_string(),
        matcher_version: "1".to_string(),
        match_numerator: None,
        match_denominator: None,
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
        assert_eq!(row.from_blob_oid, shared);
        assert_eq!(row.to_blob_oid, shared);
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

/// **A commit that recorded no similarity hypothesis says which kind of nothing that is.**
///
/// The reason `git_rename_analysis` is a table rather than a column: a `refused_bound` commit has no
/// hypothesis row to carry the qualifier, so the only thing that distinguishes *"a bound refused
/// this"* from *"nothing was renamed here"* is a row of its own. Both commits below have zero
/// similarity hypotheses and different stored answers, and a third commit is analysed by a second
/// matcher so the read is shown to be per matcher rather than per commit.
///
/// The `unmeasured` object is asserted non-empty on the `partial` row and empty on the `complete`
/// one, because `{}` is a claim — the same discipline `git_history_ingest.refusals` follows — and a
/// writer that dropped the reasons would otherwise pass.
#[test]
fn a_commit_with_no_similarity_row_says_which_kind_of_nothing_that_is() {
    let conn = store();
    let refused = commit("refused", 3_000);
    let measured = commit("measured", 2_000);
    let two_matchers = commit("twomatch", 1_000);
    for row in [&refused, &measured, &two_matchers] {
        assert!(insert_commit(&conn, "r", row).unwrap());
    }

    let bounded = AnalysisRow {
        commit_oid: refused.commit_oid.clone(),
        matcher_id: "nerve-line-multiset".to_string(),
        matcher_version: "1".to_string(),
        threshold_numerator: 6,
        threshold_denominator: 10,
        deletions_considered: 900,
        additions_considered: 900,
        pairs_considered: 810_000,
        // Zero measured, and the completeness is what says why. Without the row this commit and a
        // commit that genuinely renamed nothing are the same empty result.
        pairs_measured: 0,
        completeness: RenameAnalysisCompleteness::RefusedBound,
        unmeasured: BTreeMap::new(),
    };
    let partial = AnalysisRow {
        commit_oid: measured.commit_oid.clone(),
        matcher_id: "nerve-line-multiset".to_string(),
        matcher_version: "1".to_string(),
        threshold_numerator: 6,
        threshold_denominator: 10,
        deletions_considered: 2,
        additions_considered: 3,
        pairs_considered: 6,
        pairs_measured: 4,
        completeness: RenameAnalysisCompleteness::Partial,
        unmeasured: BTreeMap::from([
            (SimilarityUnmeasured::BlobBinary, 1),
            (SimilarityUnmeasured::BlobTooLarge, 1),
        ]),
    };
    let complete = AnalysisRow {
        commit_oid: two_matchers.commit_oid.clone(),
        matcher_id: "nerve-line-multiset".to_string(),
        matcher_version: "1".to_string(),
        threshold_numerator: 6,
        threshold_denominator: 10,
        deletions_considered: 1,
        additions_considered: 1,
        pairs_considered: 1,
        pairs_measured: 1,
        completeness: RenameAnalysisCompleteness::Complete,
        unmeasured: BTreeMap::new(),
    };
    // The same commit, a second matcher. `matcher_id` is in the primary key so this is a new row
    // rather than a conflict, and the read below must not merge the two into one verdict.
    let other_matcher = AnalysisRow {
        matcher_id: "some-other-matcher".to_string(),
        completeness: RenameAnalysisCompleteness::NotAttempted,
        pairs_considered: 0,
        pairs_measured: 0,
        ..complete.clone()
    };
    for row in [&bounded, &partial, &complete, &other_matcher] {
        assert_eq!(insert_rename_analysis(&conn, "r", row).unwrap(), 1);
    }

    let oids: Vec<&str> = vec![
        refused.commit_oid.as_str(),
        measured.commit_oid.as_str(),
        two_matchers.commit_oid.as_str(),
    ];
    let read = rename_analysis_for_commits(&conn, "r", &oids, "nerve-line-multiset").unwrap();
    assert_eq!(read.len(), 3);
    assert_eq!(read[&refused.commit_oid], bounded);
    assert_eq!(read[&measured.commit_oid], partial);
    assert_eq!(read[&two_matchers.commit_oid], complete);
    // Every field came back, including the reasons, which are the part a JSON round trip loses
    // first.
    assert_eq!(read[&measured.commit_oid].unmeasured.len(), 2);
    assert_eq!(
        read[&measured.commit_oid].unmeasured[&SimilarityUnmeasured::BlobBinary],
        1
    );
    assert!(
        read[&refused.commit_oid].unmeasured.is_empty(),
        "empty is a claim and must survive as one"
    );

    // The second matcher's answer is its own, and asking for it returns a different verdict for the
    // same commit. Merging them would produce a completeness that describes no run that happened.
    let other = rename_analysis_for_commits(&conn, "r", &oids, "some-other-matcher").unwrap();
    assert_eq!(other.len(), 1);
    assert_eq!(
        other[&two_matchers.commit_oid].completeness,
        RenameAnalysisCompleteness::NotAttempted
    );

    // A matcher nobody ran is an empty map, which is the third fact: never analysed, as distinct
    // from analysed-and-refused and from analysed-and-complete.
    assert!(rename_analysis_for_commits(&conn, "r", &oids, "never-ran")
        .unwrap()
        .is_empty());
    // And the read is scoped by repository like every other, so the second repository sees none of
    // it even though the oids exist.
    assert!(
        rename_analysis_for_commits(&conn, "r2", &oids, "nerve-line-multiset")
            .unwrap()
            .is_empty()
    );
    assert!(
        rename_analysis_for_commits(&conn, "r", &[], "nerve-line-multiset")
            .unwrap()
            .is_empty()
    );
}

/// An analysis row is a plain `INSERT`: a repeat is an error and an orphan is refused.
///
/// The same measured reason as `insert_changes` and `insert_renames`, and it bites harder here.
/// A silently dropped analysis row leaves a commit with no similarity hypotheses and nothing saying
/// why — which is exactly what a `refused_bound` commit looks like from the hypothesis table, so the
/// loss would turn a refusal into "nothing was renamed".
#[test]
fn a_repeated_or_orphaned_analysis_row_is_an_error_rather_than_a_silent_drop() {
    let conn = store();
    let host = commit("host", 1_000);
    assert!(insert_commit(&conn, "r", &host).unwrap());
    let row = AnalysisRow {
        commit_oid: host.commit_oid.clone(),
        matcher_id: "nerve-line-multiset".to_string(),
        matcher_version: "1".to_string(),
        threshold_numerator: 6,
        threshold_denominator: 10,
        deletions_considered: 1,
        additions_considered: 1,
        pairs_considered: 1,
        pairs_measured: 1,
        completeness: RenameAnalysisCompleteness::Complete,
        unmeasured: BTreeMap::new(),
    };
    assert_eq!(insert_rename_analysis(&conn, "r", &row).unwrap(), 1);

    let err = insert_rename_analysis(&conn, "r", &row).unwrap_err();
    assert!(
        err.to_string().to_lowercase().contains("constraint")
            || err.to_string().to_lowercase().contains("unique"),
        "expected a primary-key violation, got {err}"
    );

    let orphan = AnalysisRow {
        commit_oid: oid("absent"),
        ..row.clone()
    };
    let err = insert_rename_analysis(&conn, "r", &orphan).unwrap_err();
    assert!(
        err.to_string().to_lowercase().contains("foreign key"),
        "expected a foreign-key refusal, got {err}"
    );

    // And the schema's own `CHECK`: more pairs measured than considered is not a tally.
    let impossible = AnalysisRow {
        matcher_id: "second".to_string(),
        pairs_considered: 1,
        pairs_measured: 2,
        ..row.clone()
    };
    assert!(insert_rename_analysis(&conn, "r", &impossible).is_err());

    assert_eq!(scalar(&conn, "SELECT count(*) FROM git_rename_analysis"), 1);
}

/// A stored `unmeasured` key outside the closed vocabulary is refused on read, never defaulted.
///
/// The column is `TEXT` holding JSON, so nothing in SQL constrains its keys. That makes the read
/// the enforcement point, exactly as it is for `parent_completeness`: a key nobody defined must not
/// become a count nobody can explain.
#[test]
fn an_unknown_unmeasured_reason_is_refused_rather_than_dropped() {
    let conn = store();
    let host = commit("host", 1_000);
    assert!(insert_commit(&conn, "r", &host).unwrap());
    let good = AnalysisRow {
        commit_oid: host.commit_oid.clone(),
        matcher_id: "nerve-line-multiset".to_string(),
        matcher_version: "1".to_string(),
        threshold_numerator: 6,
        threshold_denominator: 10,
        deletions_considered: 1,
        additions_considered: 1,
        pairs_considered: 1,
        pairs_measured: 0,
        completeness: RenameAnalysisCompleteness::Partial,
        unmeasured: BTreeMap::from([(SimilarityUnmeasured::BlobAbsent, 1)]),
    };
    assert_eq!(insert_rename_analysis(&conn, "r", &good).unwrap(), 1);
    // The control: the well-formed row reads back.
    let oids = [host.commit_oid.as_str()];
    assert_eq!(
        rename_analysis_for_commits(&conn, "r", &oids, "nerve-line-multiset")
            .unwrap()
            .len(),
        1
    );

    conn.execute(
        "UPDATE git_rename_analysis SET unmeasured = '{\"blob_absent\":1}' WHERE repo_id = 'r'",
        [],
    )
    .unwrap();
    let err = rename_analysis_for_commits(&conn, "r", &oids, "nerve-line-multiset").unwrap_err();
    assert!(
        matches!(err, nerve_store::StoreError::Core(_)),
        "expected a vocabulary refusal, got {err}"
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
        from_blob_oid: oid("blob"),
        to_blob_oid: oid("blob"),
        matcher_id: "git-blob-oid".to_string(),
        matcher_version: "1".to_string(),
        match_numerator: None,
        match_denominator: None,
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
        from_blob_oid: oid("blob"),
        to_blob_oid: oid("blob"),
        matcher_id: "git-blob-oid".to_string(),
        matcher_version: "1".to_string(),
        match_numerator: None,
        match_denominator: None,
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

    // Columns named rather than positional: v7 appends `summary_truncation` after `is_merge`, so a
    // positional insert written against v6 would put the merge flag in a vocabulary column and this
    // test would refuse the wrong thing.
    conn.execute(
        "INSERT INTO git_commit
             (repo_id, commit_oid, tree_oid, parent_oids, parent_completeness, changes_enumerated,
              author_time, author_tz, committer_time, committer_tz, author_ident, committer_ident,
              summary, summary_truncation, is_merge)
         VALUES ('r', ?1, 'aa', '[]', 'not_a_vocabulary_value', 'enumerated',
                 1, '+0000', 1, '+0000', NULL, NULL, 'bad', 'complete', 0)",
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
        "INSERT INTO git_commit
             (repo_id, commit_oid, tree_oid, parent_oids, parent_completeness, changes_enumerated,
              author_time, author_tz, committer_time, committer_tz, author_ident, committer_ident,
              summary, summary_truncation, is_merge)
         VALUES ('r', ?1, 'aa', 'not json', 'root', 'enumerated',
                 1, '+0000', 1, '+0000', NULL, NULL, 'bad', 'complete', 0)",
        [&oid("badjson")],
    )
    .unwrap();

    let err = commit_log(&conn, "r", 10, 0).unwrap_err();
    assert!(
        matches!(err, nerve_store::StoreError::Json { .. }),
        "expected a JSON refusal, got {err}"
    );

    // And `summary_truncation` is a closed vocabulary on the same terms as the two above: a stored
    // value nobody defined is refused rather than read as `complete`, which would report a cut
    // summary as whole — the exact substitution the third vocabulary value exists to prevent.
    conn.execute(
        "DELETE FROM git_commit WHERE commit_oid = ?1",
        [&oid("badjson")],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO git_commit
             (repo_id, commit_oid, tree_oid, parent_oids, parent_completeness, changes_enumerated,
              author_time, author_tz, committer_time, committer_tz, author_ident, committer_ident,
              summary, summary_truncation, is_merge)
         VALUES ('r', ?1, 'aa', '[]', 'root', 'enumerated',
                 1, '+0000', 1, '+0000', NULL, NULL, 'bad', 'maybe', 0)",
        [&oid("badtrunc")],
    )
    .unwrap();
    let err = commit_log(&conn, "r", 10, 0).unwrap_err();
    assert!(
        matches!(err, nerve_store::StoreError::Core(_)),
        "expected a vocabulary refusal, got {err}"
    );
    conn.execute(
        "DELETE FROM git_commit WHERE commit_oid = ?1",
        [&oid("badtrunc")],
    )
    .unwrap();

    // A malformed change kind is refused by the totals query too, not only by the row read.
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

// ---- Slice 12c-i: the derived questions --------------------------------------------------------

/// An ingest record that saw everything it could: not shallow, walk exhausted.
fn complete_ingest() -> IngestRow {
    IngestRow {
        head_oid: Some(oid("head")),
        walked_from: vec![oid("head")],
        commits_recorded: 0,
        commit_budget: 5_000,
        walk_terminated_by: WalkTermination::Exhausted,
        shallow: false,
        shallow_boundary: Vec::new(),
        promisor: false,
        refusals: BTreeMap::new(),
        reader_version: "gitobj-1.0.0".to_string(),
    }
}

/// A root commit: no parents in the object, and not a boundary.
fn root_commit(label: &str, committer_time: i64) -> CommitRow {
    let mut row = commit(label, committer_time);
    row.parent_oids = Vec::new();
    row.parent_completeness = ParentCompleteness::Root;
    row
}

/// Record an `entity` row that a `<rel_path>` selector would find at `path`.
///
/// Written as a `file` entity, whose [`nerve_core::vocab::PathRole`] is `Container`, so the path is
/// `scope_path` joined to `name` — the harder of the two shapes and the one a query that only
/// handled `Content` would miss.
fn entity_at(conn: &Connection, repo_id: &str, path: &str) {
    let (scope_path, name) = match path.rfind('/') {
        Some(index) => (&path[..index], &path[index + 1..]),
        None => ("", path),
    };
    conn.execute(
        "INSERT INTO entity (entity_id, repo_id, kind, name, scope_path, language, meta)
         VALUES (?1, ?2, 'file', ?3, ?4, NULL, NULL)",
        rusqlite::params![format!("file:{repo_id}:{path}"), repo_id, name, scope_path],
    )
    .unwrap();
}

/// Record the state and run that make `git_commit` the repository's *current* commit.
fn indexed_at(conn: &Connection, repo_id: &str, state_id: &str, git_commit: Option<&str>) {
    nerve_store::upsert_repository_state(
        conn,
        &nerve_store::RepositoryStateRow {
            state_id: state_id.to_string(),
            repo_id: repo_id.to_string(),
            kind: "content".to_string(),
            git_commit: git_commit.map(str::to_string),
            content_merkle: state_id.to_string(),
        },
    )
    .unwrap();
    nerve_store::begin_extractor_run(conn, repo_id, state_id, "fs-structural", "1.0.0").unwrap();
}

/// **All six `FirstObservedKind` values, each produced by real rows through the public write path.**
///
/// Six values asserted only against the enum would be a vacuity trap of exactly the shape this
/// project has caught five times. Every one here comes out of `first_last_observed` reading rows a
/// writer put in, and every zero-change answer is asserted beside a nonzero tally in the same
/// database, so no absence can pass because the write path was never reached.
///
/// The three zero-change values are the interesting ones: they differ only in what the **entity
/// table** says, and `history sync` needs no index, so "exists now, never changed", "does not exist
/// now" and "we could not look" are three answers rather than one empty result.
#[test]
fn every_first_observed_kind_is_produced_by_real_rows() {
    let conn = store();

    // `r2` has no ingest record yet. Absence of an ingest is not absence of history.
    let none_yet = first_last_observed(&conn, "r2", "src/anything.ts").unwrap();
    assert_eq!(none_yet.kind, FirstObservedKind::NoHistoryIngested);
    assert!(!none_yet.may_claim_created);
    assert!(none_yet.first.is_none() && none_yet.last.is_none());
    assert_eq!(none_yet.walk_terminated_by, None);
    assert!(
        !none_yet.earlier_changes_may_exist,
        "there is no ingest to judge"
    );

    // `r`: a complete, non-shallow, exhausted ingest.
    nerve_store::upsert_history_ingest(&conn, "r", &complete_ingest()).unwrap();

    let root = root_commit("root", 1_000);
    assert!(insert_commit(&conn, "r", &root).unwrap());
    assert_eq!(
        insert_changes(&conn, "r", &[added(&root.commit_oid, "src/created.ts")]).unwrap(),
        1
    );

    // A path added at a commit whose parents are available, in a complete history, and added
    // **once**. Nerve diffed against a present parent tree and the path was not in it, and nothing
    // above is hidden, so this is a creation — established without consulting a clock.
    let later = commit("later", 2_000);
    assert!(insert_commit(&conn, "r", &later).unwrap());
    assert_eq!(
        insert_changes(&conn, "r", &[added(&later.commit_oid, "src/later.ts")]).unwrap(),
        1
    );

    // A path added **twice** in the same complete history: created, deleted, re-created. Which
    // addition came first is a question about ancestry, and change order here is `committer_time`
    // order, which a rebase can reorder freely — so this is the earliest *dated* change and not a
    // creation claim. The two additions are what make it so; both commits have available parents.
    let revived_a = commit("revived-a", 3_000);
    let revived_b = commit("revived-b", 4_000);
    assert!(insert_commit(&conn, "r", &revived_a).unwrap());
    assert!(insert_commit(&conn, "r", &revived_b).unwrap());
    assert_eq!(
        insert_changes(
            &conn,
            "r",
            &[
                added(&revived_a.commit_oid, "src/revived.ts"),
                added(&revived_b.commit_oid, "src/revived.ts"),
            ]
        )
        .unwrap(),
        2
    );

    // The current tree, from the entity table and nowhere else. Two entities, so an index exists.
    entity_at(&conn, "r", "src/created.ts");
    entity_at(&conn, "r", "src/untouched.ts");

    // 1. Added at a commit with no parents at all: the one answer that may say "created".
    let created = first_last_observed(&conn, "r", "src/created.ts").unwrap();
    assert_eq!(created.kind, FirstObservedKind::CreatedInVisibleHistory);
    assert!(created.may_claim_created, "the only value that may");
    assert_eq!(created.changes_in_visible_history, 1);
    assert_eq!(
        created.first.as_ref().unwrap().commit.commit_oid,
        root.commit_oid
    );
    assert_eq!(
        created.first.as_ref().unwrap().change.change_kind,
        ChangeKind::Added
    );
    assert_eq!(created.earlier_history_unavailable, None);
    assert!(!created.earlier_changes_may_exist);

    // 1b. Added once, below a visible parent, in a complete history. Also a creation, and the claim
    //     rests on no date: the path was absent from a parent tree Nerve actually read, and exactly
    //     one addition exists. Requiring the commit to be parentless instead would make this value
    //     unreachable for every file outside a root commit while the response still reported nothing
    //     hidden — a kind contradicting the two fields beside it.
    let later_seen = first_last_observed(&conn, "r", "src/later.ts").unwrap();
    assert_eq!(later_seen.kind, FirstObservedKind::CreatedInVisibleHistory);
    assert!(later_seen.may_claim_created);
    assert_eq!(
        later_seen.additions_recorded, 1,
        "the licence for the claim"
    );
    assert_eq!(later_seen.earlier_history_unavailable, None);
    assert!(!later_seen.earlier_changes_may_exist);

    // 2. Added **twice** in the same complete history. Nothing is hidden, yet this is not a creation
    //    claim, because `committer_time` order cannot say which addition was topologically first.
    //    This is the case that distinguishes the rule from "the earliest dated change is an add".
    let revived = first_last_observed(&conn, "r", "src/revived.ts").unwrap();
    assert_eq!(revived.kind, FirstObservedKind::EarliestVisibleChange);
    assert!(
        !revived.may_claim_created,
        "two additions mean two creations, and dates cannot order them"
    );
    assert_eq!(revived.additions_recorded, 2);
    assert_eq!(
        revived.earlier_history_unavailable, None,
        "nothing is hidden — the refusal here is about ordering, not availability"
    );
    assert!(!revived.earlier_changes_may_exist);
    assert_eq!(
        revived.first.as_ref().unwrap().change.change_kind,
        ChangeKind::Added,
        "the earliest dated change IS an addition, which is exactly why the count is needed"
    );

    // 3. In the tree now, never touched in visible history. The common case on a shallow clone, and
    //    the value whose absence would report an unchanged file as having no history.
    let present = first_last_observed(&conn, "r", "src/untouched.ts").unwrap();
    assert_eq!(present.kind, FirstObservedKind::PresentBeforeVisibleHistory);
    assert!(!present.may_claim_created);
    assert_eq!(present.changes_in_visible_history, 0);
    assert!(present.first.is_none() && present.last.is_none());
    assert!(present.current_tree.index_exists);
    assert_eq!(present.current_tree.entities_at_path, 1);
    assert_eq!(present.current_tree.basis, CURRENT_TREE_BASIS);

    // 4. Not in the tree now, and the tree was genuinely consulted.
    let absent = first_last_observed(&conn, "r", "src/never-existed.ts").unwrap();
    assert_eq!(absent.kind, FirstObservedKind::AbsentFromVisibleHistory);
    assert!(absent.current_tree.index_exists);
    assert_eq!(absent.current_tree.entities_at_path, 0);

    // 5. History without an index. Nerve cannot tell 3 from 4, and says so instead of choosing.
    nerve_store::upsert_history_ingest(&conn, "r2", &complete_ingest()).unwrap();
    let unknown = first_last_observed(&conn, "r2", "src/created.ts").unwrap();
    assert_eq!(unknown.kind, FirstObservedKind::CurrentTreeUnknown);
    assert!(!unknown.current_tree.index_exists);
    assert_eq!(
        unknown.current_tree.entities_at_path, 0,
        "the entity table is scoped by repository, so `r`'s rows are not `r2`'s"
    );

    // 6. Already asserted at the top, before `r2` had an ingest row.

    // Exactly one of the six answers permits the claim, measured over the answers rather than over
    // the enum — the enum is pinned separately in `nerve-core`.
    let observed = [
        created.kind,
        later_seen.kind,
        revived.kind,
        present.kind,
        absent.kind,
        unknown.kind,
        none_yet.kind,
    ];
    let mut distinct = observed.to_vec();
    distinct.sort_unstable();
    distinct.dedup();
    assert_eq!(
        distinct.len(),
        6,
        "seven answers over six values, two of them creations: {observed:?}"
    );
    let mut all = FirstObservedKind::ALL.to_vec();
    all.sort_unstable();
    assert_eq!(
        distinct, all,
        "a value of the vocabulary was never produced by a row"
    );
    assert_eq!(
        distinct
            .iter()
            .filter(|kind| kind.may_claim_created())
            .count(),
        1,
        "exactly one of the six values may claim creation"
    );

    // The tallies that stop every absence above from being satisfied by an empty database.
    assert_eq!(history_totals(&conn, "r").unwrap().changes, 4);
    assert_eq!(scalar(&conn, "SELECT count(*) FROM entity"), 2);
    assert_eq!(scalar(&conn, "SELECT count(*) FROM git_history_ingest"), 2);
}

/// **A path-level reason and a repository-level boolean, and exactly where they must agree.**
///
/// `earlier_history_unavailable` answers *"is history hidden above this path's earliest change?"*.
/// `earlier_changes_may_exist` answers *"might this ingest be missing earlier commits?"*. Two
/// questions at two scopes, and conflating them fails in both directions:
///
/// - Demanding they always agree denies a creation the object graph proves. A parentless commit has
///   nothing above it whatever is true elsewhere, and a shallow clone can contain a genuine root —
///   one branch fetched whole, another truncated.
/// - Letting them disagree freely is how the real defect got in: [`WalkTermination::Refused`]
///   produced *no reason* beside *may exist: true* for a path below a **visible parent**, where the
///   path's answer rests entirely on whether the walk was complete.
///
/// So the equivalence is asserted for the `ParentsAvailable` anchor and the independence is asserted
/// for the `Root` anchor, over **every** [`WalkTermination`] — the defect was in one arm of five, and
/// a spot check is how it survived a review.
#[test]
fn a_path_level_reason_and_a_repository_level_boolean_agree_only_where_they_should() {
    let conn = store();

    // Anchored at a parentless commit: nothing precedes it, ever.
    let root = root_commit("root", 1_000);
    assert!(insert_commit(&conn, "r", &root).unwrap());
    assert_eq!(
        insert_changes(&conn, "r", &[added(&root.commit_oid, "src/at-root.ts")]).unwrap(),
        1
    );
    // Anchored below a visible parent: the answer depends on the walk.
    let below = commit("below", 2_000);
    assert!(insert_commit(&conn, "r", &below).unwrap());
    assert_eq!(
        insert_changes(&conn, "r", &[added(&below.commit_oid, "src/below.ts")]).unwrap(),
        1
    );
    assert_eq!(
        below.parent_completeness,
        ParentCompleteness::ParentsAvailable,
        "the fixture for this test must anchor below a visible parent"
    );

    let mut agreed = 0_usize;
    let mut root_stayed_none = 0_usize;
    let mut boolean_was_true = 0_usize;
    for termination in WalkTermination::ALL {
        let mut ingest = complete_ingest();
        ingest.walk_terminated_by = termination;
        nerve_store::upsert_history_ingest(&conn, "r", &ingest).unwrap();

        let below_parent = first_last_observed(&conn, "r", "src/below.ts").unwrap();
        assert_eq!(
            below_parent.earlier_history_unavailable.is_some(),
            below_parent.earlier_changes_may_exist,
            "{termination:?}: below a visible parent, a named reason and the boolean must agree"
        );
        // One addition, so the creation claim is exactly the inverse of "something may be hidden".
        assert_eq!(
            below_parent.additions_recorded, 1,
            "{termination:?}: fixture invariant"
        );
        assert_eq!(
            below_parent.may_claim_created, !below_parent.earlier_changes_may_exist,
            "{termination:?}: the claim must track the reason, not merely coexist with it"
        );
        if below_parent.earlier_history_unavailable.is_some() {
            agreed += 1;
        }
        if below_parent.earlier_changes_may_exist {
            boolean_was_true += 1;
        }

        let at_root = first_last_observed(&conn, "r", "src/at-root.ts").unwrap();
        assert_eq!(
            at_root.earlier_history_unavailable, None,
            "{termination:?}: a parentless commit hides nothing above it"
        );
        assert!(
            at_root.may_claim_created,
            "{termination:?}: a creation the object graph proves was denied by a repository flag"
        );
        root_stayed_none += 1;
    }

    // Anti-vacuity: all-`Some` or all-`None` would satisfy the equality above while testing one
    // branch, so both outcomes must have occurred.
    assert_eq!(
        boolean_was_true,
        WalkTermination::ALL.len() - 1,
        "only `Exhausted` establishes that the walk missed nothing"
    );
    assert_eq!(agreed, boolean_was_true, "the agreeing arm never fired");
    assert_eq!(root_stayed_none, WalkTermination::ALL.len());

    // And the independence is real, not an artefact of every terminal behaving alike: with the
    // repository shallow, the root-anchored path still reports a creation.
    let mut shallow = complete_ingest();
    shallow.shallow = true;
    shallow.shallow_boundary = vec![oid("boundary")];
    nerve_store::upsert_history_ingest(&conn, "r", &shallow).unwrap();
    let at_root = first_last_observed(&conn, "r", "src/at-root.ts").unwrap();
    assert!(
        at_root.earlier_changes_may_exist,
        "the repository is shallow"
    );
    assert_eq!(at_root.earlier_history_unavailable, None);
    assert_eq!(at_root.kind, FirstObservedKind::CreatedInVisibleHistory);
    let below_parent = first_last_observed(&conn, "r", "src/below.ts").unwrap();
    assert_eq!(
        below_parent.earlier_history_unavailable,
        Some(EarlierHistoryUnavailable::ShallowBoundary),
        "below a visible parent, a shallow repository does hide history"
    );
}

/// **An addition at a shallow boundary is never a creation, and the reason is named.**
///
/// The boundary tree diffed against nothing would report every path in it as added; that is "the
/// project's history begins here" stated as data. The control in the same database is a genuine root
/// commit, so `created_in_visible_history` is demonstrably reachable and the negative below is a
/// measurement rather than a write path that cannot produce the positive.
#[test]
fn an_addition_at_a_shallow_boundary_is_not_a_creation() {
    let conn = store();
    let mut ingest = complete_ingest();
    ingest.shallow = true;
    ingest.shallow_boundary = vec![oid("bound")];
    ingest.walk_terminated_by = WalkTermination::ShallowBoundary;
    nerve_store::upsert_history_ingest(&conn, "r", &ingest).unwrap();

    let mut boundary = commit("bound", 2_000);
    boundary.parent_completeness = ParentCompleteness::ShallowBoundary;
    assert!(insert_commit(&conn, "r", &boundary).unwrap());
    assert_eq!(
        insert_changes(
            &conn,
            "r",
            &[added(&boundary.commit_oid, "src/at-boundary.ts")]
        )
        .unwrap(),
        1
    );

    // The control: a real root in the same database.
    let root = root_commit("root", 1_000);
    assert!(insert_commit(&conn, "r", &root).unwrap());
    assert_eq!(
        insert_changes(&conn, "r", &[added(&root.commit_oid, "src/real-root.ts")]).unwrap(),
        1
    );

    let at_boundary = first_last_observed(&conn, "r", "src/at-boundary.ts").unwrap();
    assert_eq!(
        at_boundary.first.as_ref().unwrap().change.change_kind,
        ChangeKind::Added,
        "the change really is an addition, so the refusal below is about the boundary"
    );
    assert_eq!(at_boundary.kind, FirstObservedKind::EarliestVisibleChange);
    assert!(
        !at_boundary.may_claim_created,
        "an addition at a shallow boundary claimed to be a creation"
    );
    assert_eq!(
        at_boundary.earlier_history_unavailable,
        Some(EarlierHistoryUnavailable::ShallowBoundary),
        "the reason history stops above it must be named"
    );
    assert!(at_boundary.earlier_changes_may_exist);
    assert!(at_boundary.shallow);

    let real_root = first_last_observed(&conn, "r", "src/real-root.ts").unwrap();
    assert_eq!(real_root.kind, FirstObservedKind::CreatedInVisibleHistory);
    assert!(
        real_root.may_claim_created,
        "the control proves creation is reachable in this database"
    );
}

/// **A budget-bounded ingest names Nerve's own boundary, and never the repository's.**
///
/// `commit_budget` is the fourth of §4.1's reasons and the one that is Nerve's doing. A bounded read
/// that reported no reason at all would let an earliest visible change read as an origin; one that
/// reported `shallow_boundary` would blame the repository for Nerve's decision.
#[test]
fn a_budget_bounded_ingest_names_nerves_own_boundary() {
    let conn = store();
    let mut ingest = complete_ingest();
    ingest.walk_terminated_by = WalkTermination::CommitBudget;
    ingest.commit_budget = 1;
    ingest.commits_recorded = 1;
    ingest
        .refusals
        .insert("history-commit-budget".to_string(), 1);
    nerve_store::upsert_history_ingest(&conn, "r", &ingest).unwrap();

    // Parents are present in the object store — they were simply never walked. So the commit itself
    // says `parents_available` and only the ingest knows the read was cut short.
    let tip = commit("tip", 3_000);
    assert!(insert_commit(&conn, "r", &tip).unwrap());
    assert_eq!(
        insert_changes(&conn, "r", &[added(&tip.commit_oid, "src/app.ts")]).unwrap(),
        1
    );

    let observed = first_last_observed(&conn, "r", "src/app.ts").unwrap();
    assert_eq!(
        observed.first.as_ref().unwrap().commit.parent_completeness,
        ParentCompleteness::ParentsAvailable,
        "the commit's own parents are available; only the walk stopped"
    );
    assert_eq!(observed.kind, FirstObservedKind::EarliestVisibleChange);
    assert!(!observed.may_claim_created);
    assert_eq!(
        observed.earlier_history_unavailable,
        Some(EarlierHistoryUnavailable::CommitBudget)
    );
    assert!(
        !observed.shallow,
        "Nerve stopping is not the repository being shallow"
    );
    assert!(observed.earlier_changes_may_exist);
    assert_eq!(
        observed.walk_terminated_by,
        Some(WalkTermination::CommitBudget)
    );

    // Every reason is reachable, and each is a different answer. `ParentsUnverifiable` and
    // `ParentsMissing` come from the commit rather than from the ingest, which is why they are
    // exercised here through a second and third commit rather than through a second ingest.
    for (label, completeness, expected) in [
        (
            "miss",
            ParentCompleteness::ParentsMissing,
            EarlierHistoryUnavailable::ParentsMissing,
        ),
        (
            "unver",
            ParentCompleteness::ParentsUnverifiable,
            EarlierHistoryUnavailable::ParentsUnverifiable,
        ),
    ] {
        let mut row = commit(label, 4_000);
        row.parent_completeness = completeness;
        assert!(insert_commit(&conn, "r", &row).unwrap());
        let path = format!("src/{label}.ts");
        assert_eq!(
            insert_changes(&conn, "r", &[added(&row.commit_oid, &path)]).unwrap(),
            1
        );
        let seen = first_last_observed(&conn, "r", &path).unwrap();
        assert_eq!(
            seen.earlier_history_unavailable,
            Some(expected),
            "{completeness} must name itself above the earliest change"
        );
        assert!(!seen.may_claim_created);
    }

    // All the reasons above are demonstrably distinct values of a five-valued set. The fifth,
    // `walk_refused`, is Nerve's own doing like `commit_budget` and is exercised by
    // `a_path_level_reason_and_a_repository_level_boolean_agree_only_where_they_should`.
    assert_eq!(EarlierHistoryUnavailable::ALL.len(), 5);
    let mut names: Vec<&str> = EarlierHistoryUnavailable::ALL
        .iter()
        .map(|reason| reason.as_str())
        .collect();
    names.sort_unstable();
    assert_eq!(
        names,
        vec![
            "commit_budget",
            "parents_missing",
            "parents_unverifiable",
            "shallow_boundary",
            "walk_refused"
        ]
    );
}

/// The first and last visible change are two ends of one answer, and `last` is bounded by the ingest.
#[test]
fn first_and_last_observed_are_the_two_ends_of_the_visible_range() {
    let conn = store();
    nerve_store::upsert_history_ingest(&conn, "r", &complete_ingest()).unwrap();
    let path = "src/app.ts";

    // Deliberately inserted newest first, so neither scan direction reproduces the answer by luck.
    for (label, time, kind) in [
        ("third", 3_000, ChangeKind::Deleted),
        ("first", 1_000, ChangeKind::Added),
        ("second", 2_000, ChangeKind::Modified),
    ] {
        let row = commit(label, time);
        assert!(insert_commit(&conn, "r", &row).unwrap());
        assert_eq!(
            insert_changes(
                &conn,
                "r",
                &[ChangeRow {
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
                }]
            )
            .unwrap(),
            1
        );
    }

    let observed = first_last_observed(&conn, "r", path).unwrap();
    assert_eq!(observed.changes_in_visible_history, 3);
    let first = observed.first.as_ref().unwrap();
    let last = observed.last.as_ref().unwrap();
    assert_eq!(first.commit.commit_oid, oid("first"));
    assert_eq!(first.change.change_kind, ChangeKind::Added);
    assert_eq!(last.commit.commit_oid, oid("third"));
    assert_eq!(last.change.change_kind, ChangeKind::Deleted);
    assert_ne!(first.commit.commit_oid, last.commit.commit_oid);
    // Stable, not merely right once.
    assert_eq!(first_last_observed(&conn, "r", path).unwrap(), observed);

    // The other repository's path is not this one's.
    nerve_store::upsert_history_ingest(&conn, "r2", &complete_ingest()).unwrap();
    let other = first_last_observed(&conn, "r2", path).unwrap();
    assert_eq!(other.changes_in_visible_history, 0);
    assert!(other.first.is_none());
}

/// A five-commit graph in which an ancestry range and a `committer_time` range differ by count.
///
/// ```text
/// root(1000) ─┬─ main1(2000) ── main2(3000) ─┬─ merge(4000)
///             └─ side(2500) ─────────────────┘
/// ```
///
/// `side` was committed *between* `main1` and `main2` by the clock and is an ancestor of neither.
fn merge_graph(conn: &Connection) -> BTreeMap<&'static str, CommitRow> {
    nerve_store::upsert_history_ingest(conn, "r", &complete_ingest()).unwrap();
    let root = root_commit("root", 1_000);
    let mut main1 = commit("main1", 2_000);
    main1.parent_oids = vec![root.commit_oid.clone()];
    let mut side = commit("side", 2_500);
    side.parent_oids = vec![root.commit_oid.clone()];
    let mut main2 = commit("main2", 3_000);
    main2.parent_oids = vec![main1.commit_oid.clone()];
    let mut merge = commit("merge", 4_000);
    merge.parent_oids = vec![main2.commit_oid.clone(), side.commit_oid.clone()];
    merge.is_merge = true;
    merge.changes_enumerated = ChangesEnumerated::MergeNotEnumerated;

    let mut out = BTreeMap::new();
    for (label, row) in [
        ("root", &root),
        ("main1", &main1),
        ("side", &side),
        ("main2", &main2),
        ("merge", &merge),
    ] {
        assert!(insert_commit(conn, "r", row).unwrap());
        if !row.is_merge {
            let path = format!("src/{label}.ts");
            assert_eq!(
                insert_changes(conn, "r", &[added(&row.commit_oid, &path)]).unwrap(),
                1
            );
        }
        out.insert(label, row.clone());
    }
    out
}

/// **The state diff walks ancestry, and a `committer_time` range would answer differently.**
///
/// This is the assertion that makes §5 a property rather than a paragraph. `main1..main2` holds one
/// commit by ancestry and two by the clock, because `side` was committed between them and descends
/// from neither. A diff implemented as a time range — the convenient one, since `commit_log` already
/// orders that way — passes every count test that does not look like this.
#[test]
fn the_state_diff_walks_ancestry_rather_than_a_committer_time_range() {
    let conn = store();
    let graph = merge_graph(&conn);
    let limits = StateDiffLimits::DEFAULT;

    let by_time: i64 = conn
        .query_row(
            "SELECT count(*) FROM git_commit
              WHERE repo_id = 'r' AND committer_time > 2000 AND committer_time <= 3000",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(
        by_time, 2,
        "the fixture must make the two answers differ, or this test proves nothing"
    );

    let StateDiff::Diff(report) = state_diff(
        &conn,
        "r",
        &graph["main1"].commit_oid,
        &graph["main2"].commit_oid,
        limits,
    )
    .unwrap() else {
        panic!("main1 is an ancestor of main2");
    };
    assert_eq!(
        report.commits_in_range,
        1,
        "ancestry holds one commit where the clock holds two: {:?}",
        report
            .commits
            .iter()
            .map(|c| &c.summary)
            .collect::<Vec<_>>()
    );
    assert_eq!(report.commits[0].commit_oid, graph["main2"].commit_oid);
    assert!(
        !report
            .commits
            .iter()
            .any(|c| c.commit_oid == graph["side"].commit_oid),
        "a commit that descends from neither endpoint entered the range"
    );

    // `from` is excluded and `to` is included, which is what `from..to` means.
    assert!(!report
        .commits
        .iter()
        .any(|c| c.commit_oid == graph["main1"].commit_oid));

    // The prune: `main1..merge` must carry `side`, because `side` is not an ancestor of `main1`.
    let StateDiff::Diff(wide) = state_diff(
        &conn,
        "r",
        &graph["main1"].commit_oid,
        &graph["merge"].commit_oid,
        limits,
    )
    .unwrap() else {
        panic!("main1 is an ancestor of merge");
    };
    let oids: BTreeSet<String> = wide.commits.iter().map(|c| c.commit_oid.clone()).collect();
    assert_eq!(
        oids,
        BTreeSet::from([
            graph["merge"].commit_oid.clone(),
            graph["main2"].commit_oid.clone(),
            graph["side"].commit_oid.clone(),
        ])
    );
    // And `root` — an ancestor of `main1` — stays out, which a single unpruned walk would not manage.
    assert!(!oids.contains(&graph["root"].commit_oid));

    // Newest first with the oid tiebreak, and stable across calls.
    assert_eq!(
        wide.commits
            .iter()
            .map(|c| c.committer_time)
            .collect::<Vec<_>>(),
        vec![4_000, 3_000, 2_500]
    );

    // Identical endpoints are the one legitimate empty diff: the same state, nothing between.
    let StateDiff::Diff(same) = state_diff(
        &conn,
        "r",
        &graph["merge"].commit_oid,
        &graph["merge"].commit_oid,
        limits,
    )
    .unwrap() else {
        panic!("a commit is its own ancestor for this purpose");
    };
    assert_eq!(same.commits_in_range, 0);
    assert!(!same.commits_truncated);
}

/// **Three refusals, three different answers, and not one of them is an empty diff.**
///
/// Plus a fourth that §5 did not list: a walk stopped by Nerve's **own** bound has not established
/// that `from` is not an ancestor, and returning `not_an_ancestor` there would state a property of
/// the repository that was never measured.
#[test]
fn a_state_diff_refuses_four_ways_and_never_as_an_empty_diff() {
    let conn = store();
    let graph = merge_graph(&conn);
    let limits = StateDiffLimits::DEFAULT;

    // 1. An endpoint that was never recorded, naming which.
    let absent = oid("absent");
    let refusal = state_diff(&conn, "r", &absent, &graph["merge"].commit_oid, limits).unwrap();
    match refusal {
        StateDiff::StateNotRecorded {
            from_recorded,
            to_recorded,
            ..
        } => {
            assert!(!from_recorded, "the missing endpoint must be named");
            assert!(to_recorded);
        }
        other => panic!("expected state_not_recorded, got {other:?}"),
    }
    let refusal = state_diff(&conn, "r", &graph["root"].commit_oid, &absent, limits).unwrap();
    match refusal {
        StateDiff::StateNotRecorded {
            from_recorded,
            to_recorded,
            ..
        } => {
            assert!(from_recorded);
            assert!(!to_recorded);
        }
        other => panic!("expected state_not_recorded, got {other:?}"),
    }
    // A recorded commit in another repository is not recorded in this one.
    assert!(matches!(
        state_diff(
            &conn,
            "r2",
            &graph["root"].commit_oid,
            &graph["merge"].commit_oid,
            limits
        )
        .unwrap(),
        StateDiff::StateNotRecorded { .. }
    ));

    // 2. Two recorded commits, neither an ancestor of the other. Not an empty diff.
    let refusal = state_diff(
        &conn,
        "r",
        &graph["side"].commit_oid,
        &graph["main2"].commit_oid,
        limits,
    )
    .unwrap();
    match refusal {
        StateDiff::NotAnAncestor { commits_walked, .. } => assert!(
            commits_walked > 0,
            "the walk must have run for the verdict to mean anything"
        ),
        other => panic!("expected not_an_ancestor, got {other:?}"),
    }
    // The control: reversing it is not an ancestor either, and the same call in the other direction
    // for a real ancestor pair does return a diff — so the refusal is deciding, not defaulting.
    assert!(matches!(
        state_diff(
            &conn,
            "r",
            &graph["main2"].commit_oid,
            &graph["side"].commit_oid,
            limits
        )
        .unwrap(),
        StateDiff::NotAnAncestor { .. }
    ));
    assert!(matches!(
        state_diff(
            &conn,
            "r",
            &graph["root"].commit_oid,
            &graph["merge"].commit_oid,
            limits
        )
        .unwrap(),
        StateDiff::Diff(_)
    ));

    // 3. A shallow boundary between the endpoints: whether `from` is an ancestor is undecidable, and
    //    the `ParentCompleteness` that stopped the walk is carried.
    let mut boundary = commit("bound", 5_000);
    boundary.parent_completeness = ParentCompleteness::ShallowBoundary;
    boundary.parent_oids = vec![oid("hidden")];
    assert!(insert_commit(&conn, "r", &boundary).unwrap());
    let mut tip = commit("tip", 6_000);
    tip.parent_oids = vec![boundary.commit_oid.clone()];
    assert!(insert_commit(&conn, "r", &tip).unwrap());

    let refusal = state_diff(
        &conn,
        "r",
        &graph["root"].commit_oid,
        &tip.commit_oid,
        limits,
    )
    .unwrap();
    match refusal {
        StateDiff::AncestryIncomplete {
            stopped_at,
            parent_completeness,
            ..
        } => {
            assert_eq!(stopped_at, boundary.commit_oid);
            assert_eq!(parent_completeness, ParentCompleteness::ShallowBoundary);
            assert!(!parent_completeness.may_claim_history_begins_here());
        }
        other => panic!("expected ancestry_incomplete, got {other:?}"),
    }

    // 4. Nerve's own bound. Not `not_an_ancestor`: nothing about the repository was established.
    let refusal = state_diff(
        &conn,
        "r",
        &graph["root"].commit_oid,
        &graph["merge"].commit_oid,
        StateDiffLimits {
            commits_walked: 1,
            ..limits
        },
    )
    .unwrap();
    match refusal {
        StateDiff::WalkBudgetExhausted { limit, .. } => assert_eq!(limit, 1),
        other => panic!("expected walk_budget_exhausted, got {other:?}"),
    }

    // The same bound applied to the *prune* set, which is the subtler half. `merge` has four
    // ancestors, so a budget of one cannot enumerate them, and a partial prune set would let the
    // range gain commits that are ancestors of `from` — a wrong range rather than a short one. Even
    // identical endpoints refuse here, where they are an empty diff when the bound is adequate.
    let refusal = state_diff(
        &conn,
        "r",
        &graph["merge"].commit_oid,
        &graph["merge"].commit_oid,
        StateDiffLimits {
            commits_walked: 1,
            ..limits
        },
    )
    .unwrap();
    assert!(
        matches!(refusal, StateDiff::WalkBudgetExhausted { .. }),
        "a prune set Nerve could not compute must not be reported as an empty diff: {refusal:?}"
    );
    // The control: with an adequate bound the same call is the one legitimate empty diff.
    assert!(matches!(
        state_diff(
            &conn,
            "r",
            &graph["merge"].commit_oid,
            &graph["merge"].commit_oid,
            limits
        )
        .unwrap(),
        StateDiff::Diff(_)
    ));

    // The tally: this database can produce a diff, so the four refusals above are refusals rather
    // than a store that answers nothing.
    assert_eq!(history_totals(&conn, "r").unwrap().commits, 7);
}

/// A merge-heavy range reporting few changes is expected, and the response says why.
///
/// Merges contribute **zero** change rows by Slice 12b's decision, so `changes.len()` alone would
/// read as "little changed". `merges_in_range` and the per-commit `changes_enumerated` tally are what
/// make the number legible, and both are asserted here against the same range.
#[test]
fn the_state_diff_says_why_a_merge_heavy_range_carries_few_changes() {
    let conn = store();
    let graph = merge_graph(&conn);

    let StateDiff::Diff(report) = state_diff(
        &conn,
        "r",
        &graph["root"].commit_oid,
        &graph["merge"].commit_oid,
        StateDiffLimits::DEFAULT,
    )
    .unwrap() else {
        panic!("root is an ancestor of merge");
    };
    assert_eq!(report.commits_in_range, 4);
    assert_eq!(report.merges_in_range, 1, "the merge must be counted");
    assert_eq!(
        report.changes.len(),
        3,
        "four commits, three of them with a change row"
    );
    assert_eq!(
        report.changes_enumerated.len(),
        ChangesEnumerated::ALL.len(),
        "every enumeration state present, including the ones at zero"
    );
    assert_eq!(
        report.changes_enumerated[&ChangesEnumerated::MergeNotEnumerated],
        1
    );
    assert_eq!(report.changes_enumerated[&ChangesEnumerated::Enumerated], 3);
    assert_eq!(
        report.changes_enumerated[&ChangesEnumerated::ParentUnavailable],
        0,
        "an absent state must be present with a zero, not missing"
    );
    assert_eq!(
        report.changes_enumerated.values().sum::<usize>(),
        report.commits_in_range
    );
    assert!(report.ancestry_incomplete_at.is_none());

    // Both bounds are honoured and both report truncation as a fact rather than as `len() == limit`.
    let StateDiff::Diff(cut) = state_diff(
        &conn,
        "r",
        &graph["root"].commit_oid,
        &graph["merge"].commit_oid,
        StateDiffLimits {
            commits: 2,
            changes: 1,
            ..StateDiffLimits::DEFAULT
        },
    )
    .unwrap() else {
        panic!("root is an ancestor of merge");
    };
    assert_eq!(cut.commits.len(), 2);
    assert_eq!(
        cut.commits_in_range, 4,
        "the true size must survive the cut, or a page cannot say what it is a page of"
    );
    assert!(cut.commits_truncated);
    assert_eq!(cut.changes.len(), 1);
    assert!(cut.changes_truncated);

    // The change bound on its own, with the commit list whole — so `changes_truncated` is not merely
    // an echo of `commits_truncated`.
    let StateDiff::Diff(few) = state_diff(
        &conn,
        "r",
        &graph["root"].commit_oid,
        &graph["merge"].commit_oid,
        StateDiffLimits {
            changes: 1,
            ..StateDiffLimits::DEFAULT
        },
    )
    .unwrap() else {
        panic!("root is an ancestor of merge");
    };
    assert_eq!(few.commits.len(), 4);
    assert!(!few.commits_truncated);
    assert_eq!(few.changes.len(), 1);
    assert!(few.changes_truncated);

    // A cut commit list is a cut diff even when the change bound was never reached: the changes are
    // the changes of the *returned* commits, so reporting otherwise would read as "this is
    // everything that changed".
    let StateDiff::Diff(paged) = state_diff(
        &conn,
        "r",
        &graph["root"].commit_oid,
        &graph["merge"].commit_oid,
        StateDiffLimits {
            commits: 2,
            ..StateDiffLimits::DEFAULT
        },
    )
    .unwrap() else {
        panic!("root is an ancestor of merge");
    };
    assert!(paged.commits_truncated);
    assert!(paged.changes.len() < report.changes.len());
    assert!(paged.changes_truncated);

    // And an uncut answer says so, so neither flag can be hardcoded.
    assert!(!report.commits_truncated);
    assert!(!report.changes_truncated);

    // `commit_by_oid` is the primary-key lookup the walk rests on, and it is scoped by repository.
    assert!(commit_by_oid(&conn, "r", &graph["merge"].commit_oid)
        .unwrap()
        .is_some());
    assert!(commit_by_oid(&conn, "r2", &graph["merge"].commit_oid)
        .unwrap()
        .is_none());
    assert!(commit_by_oid(&conn, "r", &oid("nope")).unwrap().is_none());
}

/// **Change frequency is bounded and totally ordered — and what this test cannot prove is stated.**
///
/// Four paths tie at one commit each and their insertion order is neither ascending nor descending,
/// following `the_commit_log_order_is_total_when_committer_times_tie`.
///
/// **The ordering assertion below does not falsify the `path ASC` tiebreak.** Deleting it from
/// `change_frequency`'s `ORDER BY` was probed against this whole suite and nothing failed, because
/// `EXPLAIN QUERY PLAN` shows `GROUP BY path` satisfied by `idx_git_change_path` with no temp b-tree,
/// so groups already arrive in path order. That is 12b's fifth vacuity trap in a new place, and it is
/// written here rather than left for a later slice to discover: this test asserts the *order*, and
/// `the_derived_orderings_state_their_tiebreaks` asserts the *clause*, because a query plan is not
/// part of the contract and no behavioural test on this schema can stand in for the clause.
#[test]
fn change_frequency_is_bounded_and_totally_ordered() {
    let conn = store();

    // "hot.ts" in three commits; four other paths in one each, inserted d, b, a, c.
    for (index, label) in ["one", "two", "three"].into_iter().enumerate() {
        let row = commit(label, 1_000 + index as i64);
        assert!(insert_commit(&conn, "r", &row).unwrap());
        assert_eq!(
            insert_changes(&conn, "r", &[added(&row.commit_oid, "src/hot.ts")]).unwrap(),
            1
        );
    }
    let host = commit("host", 5_000);
    assert!(insert_commit(&conn, "r", &host).unwrap());
    for path in ["src/d.ts", "src/b.ts", "src/a.ts", "src/c.ts"] {
        assert_eq!(
            insert_changes(&conn, "r", &[added(&host.commit_oid, path)]).unwrap(),
            1
        );
    }

    let report = change_frequency(&conn, "r", 10).unwrap();
    assert_eq!(
        report
            .rows
            .iter()
            .map(|row| (row.path.as_str(), row.commits))
            .collect::<Vec<_>>(),
        vec![
            ("src/hot.ts", 3),
            ("src/a.ts", 1),
            ("src/b.ts", 1),
            ("src/c.ts", 1),
            ("src/d.ts", 1),
        ],
        "count desc then path asc, with the tie broken explicitly"
    );
    // The tie is real: four of the five rows share a count, so there is something to break.
    assert_eq!(
        report.rows.iter().filter(|row| row.commits == 1).count(),
        4,
        "no tie to break; the ordering assertion would prove nothing"
    );
    assert_eq!(report.paths_total, 5);
    assert!(
        !report.truncated,
        "nothing was cut, so nothing may say it was"
    );
    assert_eq!(report.merges, 0);
    // Stable across calls, not merely right once.
    assert_eq!(change_frequency(&conn, "r", 10).unwrap(), report);

    // The bound, and truncation as a comparison against a counted total.
    let cut = change_frequency(&conn, "r", 2).unwrap();
    assert_eq!(cut.rows.len(), 2);
    assert_eq!(cut.limit, 2);
    assert!(cut.truncated);
    assert_eq!(cut.paths_total, 5, "the true total survives the bound");
    assert_eq!(cut.rows[0].path, "src/hot.ts");

    // A merge in the database is reported, because its changes are not enumerated and the count
    // therefore undercounts against the repository's own log.
    let mut merge = commit("merge", 6_000);
    merge.is_merge = true;
    merge.changes_enumerated = ChangesEnumerated::MergeNotEnumerated;
    assert!(insert_commit(&conn, "r", &merge).unwrap());
    assert_eq!(change_frequency(&conn, "r", 10).unwrap().merges, 1);

    // The other repository's frequencies are its own, and empty.
    let other = change_frequency(&conn, "r2", 10).unwrap();
    assert!(other.rows.is_empty());
    assert_eq!(other.paths_total, 0);
    assert!(!other.truncated);
    assert_eq!(
        history_totals(&conn, "r").unwrap().changes,
        7,
        "the tally that makes the empty answer above legible"
    );
}

/// **Co-change is a shared-commit count and the response says what it is not.**
///
/// The field is `cochange_observations`, so renaming it to `related`, `coupled` or `depends` is a
/// compile error in this test rather than a review comment. No `Relation` is emitted and no
/// assertion is written — asserted by a count over `assertion`, not by inspection — and the count is
/// raw rather than normalised, because a normalised figure invites exactly the comparison the label
/// forbids.
#[test]
fn cochange_counts_shared_commits_and_states_what_it_is_not() {
    let conn = store();

    let sweep = commit("sweep", 1_000);
    assert!(insert_commit(&conn, "r", &sweep).unwrap());
    assert_eq!(
        insert_changes(
            &conn,
            "r",
            &[
                added(&sweep.commit_oid, "src/a.ts"),
                added(&sweep.commit_oid, "src/b.ts"),
                added(&sweep.commit_oid, "src/c.ts"),
            ]
        )
        .unwrap(),
        3
    );
    let pair = commit("pair", 2_000);
    assert!(insert_commit(&conn, "r", &pair).unwrap());
    assert_eq!(
        insert_changes(
            &conn,
            "r",
            &[
                added(&pair.commit_oid, "src/a.ts"),
                added(&pair.commit_oid, "src/b.ts"),
            ]
        )
        .unwrap(),
        2
    );
    let alone = commit("alone", 3_000);
    assert!(insert_commit(&conn, "r", &alone).unwrap());
    assert_eq!(
        insert_changes(&conn, "r", &[added(&alone.commit_oid, "src/lonely.ts")]).unwrap(),
        1
    );

    let report = cochange(&conn, "r", None, 10).unwrap();
    assert_eq!(
        report
            .rows
            .iter()
            .map(|row| (
                row.path_a.as_str(),
                row.path_b.as_str(),
                row.cochange_observations
            ))
            .collect::<Vec<_>>(),
        vec![
            ("src/a.ts", "src/b.ts", 2),
            ("src/a.ts", "src/c.ts", 1),
            ("src/b.ts", "src/c.ts", 1),
        ],
        "shared commits desc, then both paths asc"
    );
    // Each unordered pair appears once, never twice with the sides swapped.
    for row in &report.rows {
        assert!(row.path_a < row.path_b, "{row:?} is not an ordered pair");
    }
    // A path that changed alone is in no pair. Asserted beside a nonzero pair count.
    assert!(!report
        .rows
        .iter()
        .any(|row| row.path_a == "src/lonely.ts" || row.path_b == "src/lonely.ts"));
    assert_eq!(report.pairs_total, 3);
    assert!(!report.truncated);
    assert_eq!(cochange(&conn, "r", None, 10).unwrap(), report);

    // The count is raw. Normalising by either path's own commit count would give 2/2 = 1.0 here and
    // make the strongest pair indistinguishable from a pair that changed together once out of once.
    assert_eq!(report.rows[0].cochange_observations, 2);

    // The sentence, in one place, carried on the response.
    assert_eq!(report.disclaimer, COCHANGE_IS_NOT_A_DEPENDENCY);
    assert!(report.disclaimer.contains("not a dependency"));
    assert!(report.disclaimer.contains("observation"));
    for forbidden in ["coupled", "affinity", "depends on", "related to"] {
        assert!(
            !report.disclaimer.contains(forbidden),
            "the disclaimer must not use the vocabulary it refuses: {forbidden:?}"
        );
    }

    // No `Relation`, no `assertion`, no `observation`. Co-change exists only in the response.
    assert_eq!(scalar(&conn, "SELECT count(*) FROM assertion"), 0);
    assert_eq!(scalar(&conn, "SELECT count(*) FROM assertion_state"), 0);
    assert_eq!(scalar(&conn, "SELECT count(*) FROM observation"), 0);
    assert_eq!(scalar(&conn, "SELECT count(*) FROM identity_link"), 0);

    // The bound, and truncation against a counted total.
    let cut = cochange(&conn, "r", None, 1).unwrap();
    assert_eq!(cut.rows.len(), 1);
    assert!(cut.truncated);
    assert_eq!(cut.pairs_total, 3);

    // Restricted to one path, with the total counted under the same restriction.
    let focused = cochange(&conn, "r", Some("src/c.ts"), 10).unwrap();
    assert_eq!(focused.pairs_total, 2);
    assert!(!focused.truncated);
    assert_eq!(focused.path.as_deref(), Some("src/c.ts"));
    for row in &focused.rows {
        assert!(row.path_a == "src/c.ts" || row.path_b == "src/c.ts");
    }
    let none = cochange(&conn, "r", Some("src/lonely.ts"), 10).unwrap();
    assert!(none.rows.is_empty());
    assert_eq!(none.pairs_total, 0);
    assert!(!none.truncated);

    // The other repository shares no commits with this one.
    assert!(cochange(&conn, "r2", None, 10).unwrap().rows.is_empty());
}

/// Every derived ordering carries an explicit tiebreak, checked on the source.
///
/// This is a **source-level** guard and is labelled as one. Both aggregate queries group by their
/// ordering key, so SQLite's current plan emits ties in that key's order anyway — deleting either
/// tiebreak was probed and no behavioural assertion failed. The clause is still required, because a
/// plan chosen by `ANALYZE`, a schema change or another SQLite build may group through a temp b-tree
/// and return ties in sorter order. Asserting the clause is the strongest check available, and saying
/// so is better than a behavioural test that would look stronger and prove nothing.
#[test]
fn the_derived_orderings_state_their_tiebreaks() {
    let source = include_str!("../src/history.rs");
    for required in [
        // `commit_log`'s, whose tiebreak *is* falsifiable — 12b measured that.
        "ORDER BY c.committer_time DESC, c.commit_oid ASC",
        // The two aggregates', whose tiebreaks are not.
        "ORDER BY touches DESC, path ASC",
        "ORDER BY shared DESC, left_path ASC, right_path ASC",
    ] {
        assert!(
            source.contains(required),
            "a derived ordering lost its explicit tiebreak: {required:?}"
        );
    }
    // And no aggregate orders by a rowid or by nothing at all.
    for forbidden in ["ORDER BY touches DESC LIMIT", "ORDER BY shared DESC LIMIT"] {
        assert!(
            !source.contains(forbidden),
            "an aggregate orders on its count alone: {forbidden:?}"
        );
    }
}

/// The co-change struct may not be renamed into an inference, and the check is on the source.
///
/// A field named `related`, `coupled` or `depends` would be the whole defect §8 exists to prevent,
/// and it would still compile. The positive assertion first: the name that must exist has to be
/// present, or scanning for the four that must not would prove nothing.
#[test]
fn the_cochange_field_is_not_named_after_an_inference() {
    let source = include_str!("../src/history.rs");
    assert!(
        source.contains("pub cochange_observations: i64"),
        "the field this test guards is gone"
    );
    for forbidden in [
        "pub related",
        "pub coupled",
        "pub depends",
        "pub dependency",
        "pub affinity",
        "pub coupling",
    ] {
        assert!(
            !source.contains(forbidden),
            "a co-change field named after an inference: {forbidden:?}"
        );
    }

    // And the derived layer reaches no filesystem, which is the structural answer to the trap that
    // routing a historical path through `discover::canonical_child` refuses every deleted path — it
    // ends in `std::fs::canonicalize`, so it requires existence, and it counts each refusal as
    // path-safety coverage. There is no path to guard here because no path is opened.
    // Call shapes, not prose: the doc comment above `first_last_observed` names
    // `std::fs::canonicalize` in order to say why it is absent, and a scan that could not tell the
    // explanation from the call would force the explanation to be deleted.
    for forbidden in [
        "canonicalize(",
        "canonical_child(",
        "std::fs::read",
        "std::fs::write",
        "File::open(",
        "read_dir(",
        "PathBuf",
        "Path::new",
    ] {
        assert!(
            !source.contains(forbidden),
            "nerve-store/src/history.rs must not reach the filesystem: {forbidden:?}"
        );
    }
}

/// **Four freshness verdicts, and `unverifiable` is not `current`.**
///
/// Reporting "unknown" as "current" is how a truncated sweep becomes a clean bill of health, which is
/// the distinction Slice 7c-i drew between `Stale` and `Unverified`. Every verdict here comes from a
/// real pair of rows, and the transitions are asserted in one database so no verdict can pass because
/// the others were unreachable.
#[test]
fn history_freshness_keeps_unverifiable_apart_from_current() {
    let conn = store();

    // 1. No ingest at all. Nothing whose freshness could be judged.
    let report = history_freshness(&conn, "r").unwrap();
    assert_eq!(report.verdict, HistoryFreshness::NoHistoryIngested);
    assert_eq!(report.ingest_head_oid, None);

    // 2. An ingest, and no index: there is no current commit to compare against.
    let mut ingest = complete_ingest();
    ingest.head_oid = Some(oid("head"));
    nerve_store::upsert_history_ingest(&conn, "r", &ingest).unwrap();
    let report = history_freshness(&conn, "r").unwrap();
    assert_eq!(
        report.verdict,
        HistoryFreshness::Unverifiable,
        "no current commit is not the same as a matching one"
    );
    assert_ne!(report.verdict, HistoryFreshness::Current);
    assert_eq!(report.current_git_commit, None);
    assert_eq!(report.current_state_id, None);

    // 3. An index whose state records no commit — a tree with no readable `.git/HEAD`. Still
    //    unverifiable, and the state is named so the verdict is traceable.
    indexed_at(&conn, "r", "state-none", None);
    let report = history_freshness(&conn, "r").unwrap();
    assert_eq!(report.verdict, HistoryFreshness::Unverifiable);
    assert_eq!(report.current_state_id.as_deref(), Some("state-none"));
    assert_eq!(report.current_git_commit, None);

    // 4. A current commit that differs from the ingest's HEAD.
    indexed_at(&conn, "r", "state-moved", Some(&oid("moved")));
    let report = history_freshness(&conn, "r").unwrap();
    assert_eq!(report.verdict, HistoryFreshness::Stale);
    assert_eq!(report.ingest_head_oid, Some(oid("head")));
    assert_eq!(report.current_git_commit, Some(oid("moved")));
    assert_ne!(report.ingest_head_oid, report.current_git_commit);

    // 5. A current commit that matches. The newest run decides, exactly as `status` decides it.
    indexed_at(&conn, "r", "state-current", Some(&oid("head")));
    let report = history_freshness(&conn, "r").unwrap();
    assert_eq!(report.verdict, HistoryFreshness::Current);
    assert_eq!(report.current_git_commit, Some(oid("head")));
    assert_eq!(report.current_state_id.as_deref(), Some("state-current"));

    // An unborn branch at sync time cannot match a real current commit, and is `stale` rather than
    // `current`: the recorded facts describe a HEAD that has since come into existence.
    let mut unborn = complete_ingest();
    unborn.head_oid = None;
    nerve_store::upsert_history_ingest(&conn, "r", &unborn).unwrap();
    let report = history_freshness(&conn, "r").unwrap();
    assert_eq!(report.verdict, HistoryFreshness::Stale);
    assert_eq!(report.ingest_head_oid, None);

    // All four verdicts were produced, and the other repository's answer is its own.
    assert_eq!(
        history_freshness(&conn, "r2").unwrap().verdict,
        HistoryFreshness::NoHistoryIngested
    );
    assert_eq!(scalar(&conn, "SELECT count(*) FROM extractor_run"), 3);
}

/// **The one history judgment, pinned over every combination that produces it.**
///
/// `earlier_changes_may_exist` was inside the CLI binary until Slice 12c-i, where three more surfaces
/// were each about to derive it. It is `true` for a shallow repository *and* for every walk that did
/// not exhaust — including [`WalkTermination::CommitBudget`], which is Nerve's own boundary and the
/// one a naive implementation drops because it "is not a property of the repository".
#[test]
fn whether_earlier_changes_may_exist_is_pinned_over_every_termination() {
    let pinned: [(WalkTermination, bool); 5] = [
        // The only combination that may say "everything reachable was read".
        (WalkTermination::Exhausted, false),
        (WalkTermination::CommitBudget, true),
        (WalkTermination::ShallowBoundary, true),
        (WalkTermination::MissingObject, true),
        (WalkTermination::Refused, true),
    ];
    let mut listed: Vec<WalkTermination> = pinned.iter().map(|(value, _)| *value).collect();
    listed.sort_unstable();
    let mut all = WalkTermination::ALL.to_vec();
    all.sort_unstable();
    assert_eq!(
        listed, all,
        "a termination reason was added without stating what it implies about earlier changes"
    );

    for (termination, expected) in pinned {
        let mut row = complete_ingest();
        row.walk_terminated_by = termination;
        row.shallow = false;
        assert_eq!(
            earlier_changes_may_exist(&row),
            expected,
            "{termination} changed what may be said about earlier changes"
        );

        // Shallow makes it true regardless: an exhausted walk exhausted what it could *see*.
        row.shallow = true;
        assert!(
            earlier_changes_may_exist(&row),
            "{termination} with a shallow boundary must still warn"
        );
    }

    assert_eq!(
        WalkTermination::ALL
            .iter()
            .filter(|termination| {
                let mut row = complete_ingest();
                row.walk_terminated_by = **termination;
                !earlier_changes_may_exist(&row)
            })
            .count(),
        1,
        "exactly one termination reason permits the claim that nothing earlier is hidden"
    );
}
