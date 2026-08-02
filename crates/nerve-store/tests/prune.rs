//! The two scoped withdrawals, over a graph built directly in SQL.
//!
//! `delete_file_rows` withdraws everything recorded against a path, which is right for a file that
//! has vanished. `delete_extractor_file_rows` withdraws only the named extractors' observations,
//! which is right for a file about to be re-extracted — since Slice 6b a `coverage` observation
//! cites a file `nerve index` is re-reading but does not own, and withdrawing it would make an
//! ordinary re-index destroy coverage evidence it cannot restate.
//!
//! `delete_claims_sourced_at` is the third case: an extractor whose claims hang off a source
//! entity that occurs somewhere else entirely, which is how re-ingesting one coverage report
//! replaces what that report previously said.

use std::collections::{BTreeMap, BTreeSet};

use nerve_store::{
    delete_claims_sourced_at, delete_extractor_file_rows, delete_file_rows, migrate,
    open_in_memory, Connection, TouchedRows,
};

/// Two source files, one coverage report, three extractors, and evidence from each.
fn fixture() -> Connection {
    let conn = open_in_memory().unwrap();
    migrate(&conn).unwrap();
    conn.execute_batch(
        "INSERT INTO repository VALUES ('repo','p','/tmp','t');
         INSERT INTO repository_state VALUES ('s','repo','content',NULL,'m','t');
         INSERT INTO extractor_run VALUES (1,'repo','s','x','1','t','t',1,0,'complete');

         INSERT INTO entity VALUES ('sym-a','repo','function','a','',NULL,NULL);
         INSERT INTO entity VALUES ('sym-b','repo','function','b','',NULL,NULL);
         INSERT INTO entity VALUES ('run-1','repo','coverage_run','lcov.info','coverage',NULL,NULL);

         INSERT INTO occurrence VALUES ('occ-a','sym-a','src/a.ts',0,10,1,0,3,1,'hash-a');
         INSERT INTO occurrence VALUES ('occ-b','sym-b','src/b.ts',0,10,1,0,3,1,'hash-b');
         INSERT INTO occurrence VALUES
             ('occ-run','run-1','coverage/lcov.info',0,42,1,0,4,0,'hash-report');

         INSERT INTO assertion VALUES ('calls','repo','sym-a','CALLS','sym-b');
         INSERT INTO assertion VALUES ('covers-a','repo','run-1','COVERS','sym-a');
         INSERT INTO assertion VALUES ('covers-b','repo','run-1','COVERS','sym-b');",
    )
    .unwrap();
    observe(&conn, "calls", "ts-js-reference", "src/a.ts");
    observe(&conn, "covers-a", "coverage", "src/a.ts");
    observe(&conn, "covers-b", "coverage", "src/b.ts");
    conn
}

fn observe(conn: &Connection, assertion_id: &str, extractor_id: &str, file_path: &str) {
    conn.execute(
        "INSERT INTO observation
             (assertion_id, extractor_run_id, evidence_source_type, directness,
              extractor_id, extractor_version, file_path, start_line, end_line,
              content_hash, created_at)
         VALUES (?1, 1, 'AST_DIRECT', 'DIRECT', ?2, '1.0.0', ?3, 1, 1, 'h', 't')",
        rusqlite::params![assertion_id, extractor_id, file_path],
    )
    .unwrap();
}

fn rows(conn: &Connection, sql: &str) -> Vec<String> {
    let mut stmt = conn.prepare(sql).unwrap();
    let rows = stmt.query_map([], |row| row.get::<_, String>(0)).unwrap();
    rows.map(|row| row.unwrap()).collect()
}

fn snapshot(conn: &Connection) -> BTreeMap<&'static str, Vec<String>> {
    BTreeMap::from([
        (
            "observation",
            rows(
                conn,
                "SELECT assertion_id || '|' || extractor_id || '|' || file_path
                   FROM observation ORDER BY 1",
            ),
        ),
        (
            "occurrence",
            rows(
                conn,
                "SELECT entity_id || '|' || file_path FROM occurrence ORDER BY 1",
            ),
        ),
    ])
}

fn paths(values: &[&str]) -> BTreeSet<String> {
    values.iter().map(|value| (*value).to_string()).collect()
}

/// The equivalence `delete_extractor_file_rows`'s documentation claims: naming every extractor
/// that wrote about the path makes it identical to the unrestricted withdrawal.
#[test]
fn naming_every_extractor_is_the_same_as_withdrawing_everything() {
    let unrestricted = fixture();
    let restricted = fixture();
    let targets = paths(&["src/a.ts", "src/b.ts"]);

    let mut touched_unrestricted = TouchedRows::default();
    let all = delete_file_rows(&unrestricted, &targets, &mut touched_unrestricted).unwrap();

    let mut touched_restricted = TouchedRows::default();
    let some = delete_extractor_file_rows(
        &restricted,
        &targets,
        &["ts-js-reference", "coverage"],
        &mut touched_restricted,
    )
    .unwrap();

    assert_eq!(all, some);
    assert_eq!(snapshot(&unrestricted), snapshot(&restricted));
    assert_eq!(touched_unrestricted, touched_restricted);
}

/// A re-extraction withdraws what it is about to rewrite and nothing else. The coverage
/// observation citing the same file survives, and stays available to be re-derived and reported
/// as stale.
#[test]
fn a_restricted_withdrawal_leaves_another_extractors_evidence_standing() {
    let conn = fixture();
    let mut touched = TouchedRows::default();
    let counts = delete_extractor_file_rows(
        &conn,
        &paths(&["src/a.ts"]),
        &["ts-js-reference"],
        &mut touched,
    )
    .unwrap();

    assert_eq!(counts.observations, 1);
    // Occurrences at the path go regardless: the re-extraction is about to rewrite every span.
    assert_eq!(counts.occurrences, 1);
    assert_eq!(
        snapshot(&conn)["observation"],
        vec![
            "covers-a|coverage|src/a.ts".to_string(),
            "covers-b|coverage|src/b.ts".to_string(),
        ]
    );
    assert!(touched.assertions.contains("calls"));
    assert!(touched.entities.contains("sym-a"));
}

#[test]
fn an_empty_extractor_list_withdraws_nothing() {
    let conn = fixture();
    let before = snapshot(&conn);
    let mut touched = TouchedRows::default();
    let counts =
        delete_extractor_file_rows(&conn, &paths(&["src/a.ts"]), &[], &mut touched).unwrap();
    assert!(counts.is_empty());
    assert!(touched.is_empty());
    assert_eq!(snapshot(&conn), before);
}

/// Re-reading an artifact replaces what it said: every claim whose source entity occurs there
/// loses this extractor's evidence, wherever the observations themselves point.
#[test]
fn withdrawing_by_artifact_finds_claims_whose_observations_point_elsewhere() {
    let conn = fixture();
    let mut touched = TouchedRows::default();
    let counts =
        delete_claims_sourced_at(&conn, "coverage", "coverage/lcov.info", &mut touched).unwrap();

    // Both coverage observations went, though neither cites `coverage/lcov.info`.
    assert_eq!(counts.observations, 2);
    assert_eq!(counts.occurrences, 1);
    assert_eq!(
        snapshot(&conn)["observation"],
        vec!["calls|ts-js-reference|src/a.ts".to_string()]
    );
    assert_eq!(
        snapshot(&conn)["occurrence"],
        vec!["sym-a|src/a.ts".to_string(), "sym-b|src/b.ts".to_string()]
    );
    assert!(touched.assertions.contains("covers-a"));
    assert!(touched.assertions.contains("covers-b"));
    assert!(touched.entities.contains("run-1"));
}

/// Only the named extractor's contribution is withdrawn. A claim two extractors observe keeps the
/// other one's evidence, and the claim with it.
#[test]
fn withdrawing_by_artifact_takes_only_that_extractors_contribution() {
    let conn = fixture();
    observe(&conn, "covers-a", "some-other-extractor", "src/a.ts");
    let mut touched = TouchedRows::default();
    delete_claims_sourced_at(&conn, "coverage", "coverage/lcov.info", &mut touched).unwrap();
    assert_eq!(
        snapshot(&conn)["observation"],
        vec![
            "calls|ts-js-reference|src/a.ts".to_string(),
            "covers-a|some-other-extractor|src/a.ts".to_string(),
        ]
    );
}

#[test]
fn withdrawing_from_an_artifact_nothing_was_read_from_removes_nothing() {
    let conn = fixture();
    let before = snapshot(&conn);
    let mut touched = TouchedRows::default();
    let counts =
        delete_claims_sourced_at(&conn, "coverage", "coverage/absent.info", &mut touched).unwrap();
    assert!(counts.is_empty());
    assert!(touched.is_empty());
    assert_eq!(snapshot(&conn), before);
}
