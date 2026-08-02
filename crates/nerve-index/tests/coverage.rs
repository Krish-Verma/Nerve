//! Coverage ingestion: what it emits, what it refuses, and the one relation it may never emit.
//!
//! The load-bearing test in this file is
//! [`the_coverage_extractor_emits_no_call_shaped_relation_at_all`]. It is ADR-0005's control, and
//! it is stated exhaustively over the whole relation vocabulary rather than as a spot check: a
//! test that looked for `CALLS` and found none would say nothing about `TEST_OBSERVED_CALL` or
//! about whatever a future slice adds.
//!
//! Everything under "T9" is an **attack**, not an assertion. Each one supplies the hostile input
//! and then checks both halves of the required outcome: the refusal happened, and it was counted.

mod common;

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use common::{copy_tree, named_fixture_root, open_db, TEST_PROJECT_ID};

use nerve_index::coverage::form as parse_form;
use nerve_index::coverage_ingest::form;
use nerve_index::{CoverageOutcome, IndexError, IndexOptions};

const FIXTURE: &str = "ts-coverage";
const REPORT: &str = "coverage/lcov.info";

// ---- helpers ---------------------------------------------------------------------------------

fn write(root: &Path, rel_path: &str, contents: &str) {
    let path = root.join(rel_path);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, contents).unwrap();
}

/// Copy the coverage fixture, initialize it with the fixed project id, and index it.
fn indexed_fixture() -> (tempfile::TempDir, PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("repo");
    copy_tree(&named_fixture_root(FIXTURE), &root);
    nerve_index::init_with_project_id(&root, Some(TEST_PROJECT_ID)).unwrap();
    nerve_index::index_repository(&root).unwrap();
    (dir, root)
}

fn ingest(root: &Path, report: &str) -> CoverageOutcome {
    nerve_index::ingest_coverage(root, Path::new(report)).unwrap()
}

/// Replace the fixture's report with `body` and ingest it.
fn ingest_report(root: &Path, body: &str) -> CoverageOutcome {
    write(root, REPORT, body);
    ingest(root, REPORT)
}

fn dump_json(root: &Path) -> String {
    nerve_store::canonical_dump(&open_db(root))
        .unwrap()
        .to_canonical_json()
        .unwrap()
}

fn scalar(conn: &nerve_store::Connection, sql: &str) -> i64 {
    conn.query_row(sql, [], |row| row.get(0)).unwrap()
}

fn strings(conn: &nerve_store::Connection, sql: &str) -> Vec<String> {
    let mut stmt = conn.prepare(sql).unwrap();
    let rows = stmt.query_map([], |row| row.get::<_, String>(0)).unwrap();
    rows.map(|row| row.unwrap()).collect()
}

/// `COVERS` edges as `scope#name -> degree`, read back out of the database.
fn covers(conn: &nerve_store::Connection) -> BTreeMap<String, String> {
    let mut stmt = conn
        .prepare(
            "SELECT t.name, o.details
               FROM assertion a
               JOIN entity t ON t.entity_id = a.target_entity_id
               JOIN observation o ON o.assertion_id = a.assertion_id
              WHERE a.relation = 'COVERS'
              ORDER BY t.name",
        )
        .unwrap();
    let rows = stmt
        .query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })
        .unwrap();
    rows.map(|row| {
        let (name, details) = row.unwrap();
        let details: serde_json::Value = serde_json::from_str(&details).unwrap();
        (
            name,
            details["coverage"].as_str().unwrap_or_default().to_string(),
        )
    })
    .collect()
}

// ---- emission --------------------------------------------------------------------------------

/// The whole shape of one ingestion, against a fixture whose every `DA:` line is deliberate.
#[test]
fn the_fixture_report_produces_exactly_the_edges_it_states() {
    let (_dir, root) = indexed_fixture();
    let outcome = ingest(&root, REPORT);

    assert_eq!(outcome.report_path, REPORT);
    assert_eq!(outcome.files_in_report, 2);
    assert_eq!(outcome.files_ingested, 2);
    assert_eq!(outcome.files_refused, 0);
    assert_eq!(outcome.symbols_covered, 6);
    assert_eq!(outcome.symbols_fully_covered, 4);
    assert_eq!(outcome.symbols_partially_covered, 2);
    assert_eq!(outcome.covered_lines, 12);
    assert_eq!(outcome.uncovered_lines, 4);

    let conn = open_db(&root);
    assert_eq!(
        covers(&conn),
        BTreeMap::from([
            ("add".to_string(), "covered".to_string()),
            ("area".to_string(), "covered".to_string()),
            ("clamp".to_string(), "partial".to_string()),
            ("constructor".to_string(), "covered".to_string()),
            ("perimeter".to_string(), "partial".to_string()),
            ("Rectangle".to_string(), "covered".to_string()),
        ])
    );
}

/// A symbol with no covered line gets **no edge**. Absence is the answer to the gap question, and
/// a `NOT_COVERED` edge would be a negative claim in a positive-evidence store.
#[test]
fn a_symbol_the_run_never_entered_gets_no_edge_in_either_direction() {
    let (_dir, root) = indexed_fixture();
    ingest(&root, REPORT);
    let conn = open_db(&root);

    // `neverRun` is instrumented and never executed; `Shape` is not in the report at all.
    for absent in ["neverRun", "Shape"] {
        assert_eq!(
            scalar(
                &conn,
                &format!(
                    "SELECT count(*) FROM assertion a JOIN entity e
                       ON e.entity_id IN (a.source_entity_id, a.target_entity_id)
                      WHERE e.name = '{absent}'
                        AND a.relation IN ('COVERS', 'NOT_COVERED')"
                )
            ),
            0,
            "{absent} must have no coverage edge"
        );
    }
    // The uncovered *lines* are still recorded on the observations that do exist; it is the edge
    // that would be the invention.
    assert_eq!(
        scalar(
            &conn,
            "SELECT count(*) FROM assertion WHERE relation = 'COVERS'"
        ),
        6
    );
}

/// `partial` is a recorded value and is never rounded to covered or uncovered.
#[test]
fn partial_is_recorded_with_its_line_counts_and_never_rounded() {
    let (_dir, root) = indexed_fixture();
    ingest(&root, REPORT);
    let conn = open_db(&root);

    let details: String = conn
        .query_row(
            "SELECT o.details FROM observation o
               JOIN assertion a ON a.assertion_id = o.assertion_id
               JOIN entity t ON t.entity_id = a.target_entity_id
              WHERE t.name = 'clamp'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    let details: serde_json::Value = serde_json::from_str(&details).unwrap();
    assert_eq!(details["coverage"], "partial");
    assert_eq!(details["covered_lines"], 5);
    assert_eq!(details["instrumented_lines"], 6);
    assert_eq!(details["symbol_start_line"], 5);
    assert_eq!(details["symbol_end_line"], 13);
    assert_eq!(details["symbol_extent_lines"], 9);
    assert_eq!(details["report_path"], REPORT);
}

/// The source endpoint is a `CoverageRun` with a real occurrence at the report's real path, and
/// it is never a test — because LCOV carries nothing that could name one (ADR-0008).
#[test]
fn the_source_endpoint_is_a_coverage_run_occurring_at_the_report() {
    let (_dir, root) = indexed_fixture();
    let outcome = ingest(&root, REPORT);
    let conn = open_db(&root);

    let run_id = outcome.coverage_run_entity_id.clone().unwrap();
    assert!(run_id.starts_with("cov_"), "{run_id}");
    assert_eq!(
        run_id,
        nerve_core::ids::coverage_run_id(
            TEST_PROJECT_ID,
            REPORT,
            outcome.report_content_hash.as_deref().unwrap()
        )
    );

    let (kind, name, scope): (String, String, String) = conn
        .query_row(
            "SELECT kind, name, scope_path FROM entity WHERE entity_id = ?1",
            [&run_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .unwrap();
    assert_eq!(kind, "coverage_run");
    assert_eq!(name, "lcov.info");
    assert_eq!(scope, "coverage");

    let occurrences = nerve_store::occurrences_of(&conn, &run_id).unwrap();
    assert_eq!(occurrences.len(), 1);
    assert_eq!(occurrences[0].file_path, REPORT);
    assert_eq!(
        occurrences[0].content_hash,
        outcome.report_content_hash.unwrap()
    );
    assert!(occurrences[0].end_byte > 0);

    // Every `COVERS` edge starts at that run, and no coverage edge starts anywhere else.
    assert_eq!(
        strings(
            &conn,
            "SELECT DISTINCT e.kind FROM assertion a JOIN entity e
               ON e.entity_id = a.source_entity_id WHERE a.relation = 'COVERS'"
        ),
        vec!["coverage_run".to_string()]
    );
}

/// Every observation the extractor writes declares `TEST_COVERAGE` and infers rather than states.
#[test]
fn every_coverage_observation_is_test_coverage_and_inferred() {
    let (_dir, root) = indexed_fixture();
    ingest(&root, REPORT);
    let conn = open_db(&root);

    assert_eq!(
        strings(
            &conn,
            "SELECT DISTINCT evidence_source_type || '/' || directness || '/' ||
                    extractor_id || '/' || extractor_version
               FROM observation WHERE extractor_id = 'coverage'"
        ),
        vec!["TEST_COVERAGE/INFERRED/coverage/1.0.0".to_string()]
    );
    // And nothing else in the database claims `TEST_COVERAGE`.
    assert_eq!(
        strings(
            &conn,
            "SELECT DISTINCT extractor_id FROM observation
              WHERE evidence_source_type = 'TEST_COVERAGE'"
        ),
        vec!["coverage".to_string()]
    );
}

// ---- ADR-0005: coverage is not a call graph ---------------------------------------------------

/// **The control ADR-0005 exists for**, stated exhaustively rather than by inspection.
///
/// Every assertion attributable to the `coverage` extractor is enumerated, and the check is made
/// over the *whole* relation vocabulary: for every relation that is not `COVERS`, the count of
/// coverage-attributable assertions carrying it must be zero. A future relation added to
/// `Relation::ALL` is therefore covered by this test the day it is added, without anyone
/// remembering to extend a list of forbidden names.
#[test]
fn the_coverage_extractor_emits_no_call_shaped_relation_at_all() {
    let (_dir, root) = indexed_fixture();
    let outcome = ingest(&root, REPORT);
    assert!(outcome.symbols_covered > 0, "the test needs edges to check");
    let conn = open_db(&root);

    // The exhaustive query: every relation any coverage observation supports.
    let relations = strings(
        &conn,
        "SELECT DISTINCT a.relation
           FROM observation o
           JOIN assertion a ON a.assertion_id = o.assertion_id
          WHERE o.extractor_id = 'coverage'
          ORDER BY a.relation",
    );
    assert_eq!(
        relations,
        vec!["COVERS".to_string()],
        "the coverage extractor supported a relation other than COVERS"
    );

    // Said the other way round, over the closed vocabulary, so that nothing rests on the query
    // above having been written before the offending relation existed.
    for relation in nerve_core::Relation::ALL {
        let attributable = scalar(
            &conn,
            &format!(
                "SELECT count(*) FROM observation o
                   JOIN assertion a ON a.assertion_id = o.assertion_id
                  WHERE o.extractor_id = 'coverage' AND a.relation = '{}'",
                relation.as_str()
            ),
        );
        let expected = i64::from(relation == nerve_core::Relation::Covers);
        assert_eq!(
            attributable.min(1),
            expected,
            "coverage attributed {} assertion(s) to {}",
            attributable,
            relation.as_str()
        );
    }

    // And the two source types ADR-0005 keeps permanently distinct from coverage are absent
    // from the database entirely — nothing may reach them by relabelling a coverage row.
    for forbidden in ["TEST_CALL_TRACE", "RUNTIME_CALL_TRACE"] {
        assert_eq!(
            scalar(
                &conn,
                &format!(
                    "SELECT count(*) FROM observation WHERE evidence_source_type = '{forbidden}'"
                )
            ),
            0
        );
    }
}

// ---- T9: the report is attacker-controlled input ----------------------------------------------

/// Traversal. The path guard refuses it, the refusal is counted, and nothing about the target
/// reaches the graph.
#[test]
fn t9_a_report_naming_a_path_outside_the_root_is_refused_and_counted() {
    let (dir, root) = indexed_fixture();
    // A real file outside the repository, so the attack fails on the rule rather than on luck.
    write(dir.path(), "outside.ts", "export const secret = 1;\n");

    let outcome = ingest_report(
        &root,
        "TN:\nSF:../../../../etc/passwd\nDA:1,1\nend_of_record\n\
         TN:\nSF:../outside.ts\nDA:1,1\nend_of_record\n\
         TN:\nSF:/etc/passwd\nDA:1,1\nend_of_record\n",
    );

    assert_eq!(outcome.refused_count(form::PATH_REFUSED), 3);
    assert_eq!(outcome.files_ingested, 0);
    assert_eq!(outcome.files_refused, 3);
    assert_eq!(outcome.symbols_covered, 0);

    // Zero content leakage: no row anywhere names the target, in any table that holds text.
    let conn = open_db(&root);
    for table in ["entity", "occurrence", "observation"] {
        for column in ["name", "scope_path", "file_path", "meta", "details"] {
            let sql = format!(
                "SELECT count(*) FROM pragma_table_info('{table}') WHERE name = '{column}'"
            );
            if scalar(&conn, &sql) == 0 {
                continue;
            }
            assert_eq!(
                scalar(
                    &conn,
                    &format!(
                        "SELECT count(*) FROM {table}
                          WHERE {column} LIKE '%passwd%' OR {column} LIKE '%outside%'"
                    )
                ),
                0,
                "{table}.{column} carried the refused path"
            );
        }
    }
}

/// A symlink inside the repository pointing out of it. The guard canonicalizes, so the escape is
/// visible to it however it is spelled.
#[cfg(unix)]
#[test]
fn t9_a_report_naming_a_symlink_escaping_the_root_is_refused_and_counted() {
    let (dir, root) = indexed_fixture();
    let outside = dir.path().join("outside.ts");
    std::fs::write(&outside, "export const secret = 1;\n").unwrap();
    std::os::unix::fs::symlink(&outside, root.join("src/linked.ts")).unwrap();
    // A symlinked *directory* too: the escape is one component up from the file named.
    std::fs::create_dir_all(dir.path().join("vendor")).unwrap();
    std::fs::write(dir.path().join("vendor/lib.ts"), "export const x = 1;\n").unwrap();
    std::os::unix::fs::symlink(dir.path().join("vendor"), root.join("src/vendor")).unwrap();

    let outcome = ingest_report(
        &root,
        "TN:\nSF:src/linked.ts\nDA:1,1\nend_of_record\n\
         TN:\nSF:src/vendor/lib.ts\nDA:1,1\nend_of_record\n",
    );

    assert_eq!(outcome.refused_count(form::PATH_REFUSED), 2);
    assert_eq!(outcome.files_ingested, 0);
    assert_eq!(outcome.symbols_covered, 0);

    let conn = open_db(&root);
    assert_eq!(
        scalar(
            &conn,
            "SELECT count(*) FROM occurrence WHERE file_path LIKE 'src/linked%'
                                                OR file_path LIKE 'src/vendor%'"
        ),
        0
    );
}

/// A file inside the root that Nerve never indexed. Refused and counted — and, the part that
/// matters, **not trusted into existence**: no `File` entity, no occurrence, no entity of any
/// kind appears for it.
#[test]
fn t9_a_report_naming_an_unindexed_file_is_refused_and_creates_nothing() {
    let (_dir, root) = indexed_fixture();
    // Present on disk, inside the root, and never indexed: `.txt` has no grammar.
    write(&root, "notes.txt", "not indexed by anything\n");

    let entities_before = scalar(&open_db(&root), "SELECT count(*) FROM entity");
    let outcome = ingest_report(&root, "TN:\nSF:notes.txt\nDA:1,1\nDA:2,1\nend_of_record\n");

    assert_eq!(outcome.refused_count(form::FILE_NOT_INDEXED), 1);
    assert_eq!(outcome.files_ingested, 0);
    assert_eq!(outcome.symbols_covered, 0);

    let conn = open_db(&root);
    assert_eq!(
        scalar(
            &conn,
            "SELECT count(*) FROM occurrence WHERE file_path = 'notes.txt'"
        ),
        0
    );
    assert_eq!(
        scalar(
            &conn,
            "SELECT count(*) FROM entity WHERE name = 'notes.txt'"
        ),
        0,
        "a report must not create a File entity by naming a path"
    );
    // One entity was added and exactly one: the coverage run itself, at the report's own path.
    assert_eq!(
        scalar(&conn, "SELECT count(*) FROM entity"),
        entities_before + 1
    );
    assert_eq!(
        strings(&conn, "SELECT kind FROM entity WHERE kind = 'coverage_run'"),
        vec!["coverage_run".to_string()]
    );
}

/// A symbol that does not exist. LCOV names lines rather than symbols, so the way a report claims
/// a symbol Nerve does not have is to name a line no symbol covers — including a line number no
/// file could have.
#[test]
fn t9_a_line_naming_no_existing_symbol_is_refused_and_counted() {
    let (_dir, root) = indexed_fixture();
    let outcome = ingest_report(
        &root,
        "TN:\nSF:src/math.ts\n\
         DA:18446744073709551615,1\n\
         DA:900000,1\n\
         DA:4,1\n\
         DA:1,1\n\
         end_of_record\n",
    );

    // Three lines mapped to nothing: `u64::MAX`, a line past the end of the file, and line 4,
    // which is blank and lies between two functions.
    assert_eq!(outcome.refused_count(form::LINE_OUTSIDE_ANY_SYMBOL), 3);
    // The one real line still produced its edge; a refusal costs the line, never the record.
    assert_eq!(outcome.symbols_covered, 1);
    assert_eq!(
        covers(&open_db(&root)).keys().collect::<Vec<_>>(),
        vec!["add"]
    );
}

/// An indexed file whose bytes have moved since it was indexed. The extents a line would be
/// mapped onto describe the *old* file, so the record is refused rather than mapped through them.
#[test]
fn t9_a_file_that_changed_since_it_was_indexed_is_refused_and_counted() {
    let (_dir, root) = indexed_fixture();
    write(
        &root,
        "src/math.ts",
        "export function add(): number {\n  return 1;\n}\n",
    );

    let outcome = ingest(&root, REPORT);
    assert_eq!(outcome.refused_count(form::FILE_CHANGED_SINCE_INDEX), 1);
    assert_eq!(outcome.files_ingested, 1, "only src/shapes.ts survives");
    assert_eq!(
        covers(&open_db(&root))
            .keys()
            .cloned()
            .collect::<BTreeSet<_>>(),
        BTreeSet::from([
            "Rectangle".to_string(),
            "area".to_string(),
            "constructor".to_string(),
            "perimeter".to_string()
        ])
    );
}

/// A deny-listed file cannot be read at ingestion either, however it is named.
#[test]
fn t9_a_deny_listed_file_is_never_read_by_ingestion() {
    let (_dir, root) = indexed_fixture();
    write(&root, ".env", "SECRET=1\n");
    let outcome = ingest_report(&root, "TN:\nSF:.env\nDA:1,1\nend_of_record\n");
    // Discovery never indexed it, so it is refused before the prober is even asked. Both rules
    // hold; the first one to fire is the point.
    assert_eq!(outcome.refused_count(form::FILE_NOT_INDEXED), 1);
    assert_eq!(outcome.files_ingested, 0);
    assert_eq!(
        scalar(
            &open_db(&root),
            "SELECT count(*) FROM observation WHERE file_path = '.env'"
        ),
        0
    );
}

/// An indexed file with no symbol in it — a document — has no endpoint for a coverage edge.
#[test]
fn t9_an_indexed_file_without_symbols_is_refused_and_counted() {
    let (_dir, root) = indexed_fixture();
    let outcome = ingest_report(&root, "TN:\nSF:README.md\nDA:1,1\nend_of_record\n");
    assert_eq!(outcome.refused_count(form::FILE_WITHOUT_SYMBOLS), 1);
    assert_eq!(outcome.symbols_covered, 0);
    // A document must not acquire a coverage edge by any route (THREAT-MODEL.md T7 and T9).
    assert_eq!(
        scalar(
            &open_db(&root),
            "SELECT count(*) FROM observation WHERE extractor_id = 'coverage'
                                                AND file_path LIKE '%.md'"
        ),
        0
    );
}

/// Every resource bound the parser enforces refuses and counts **at the ingestion layer too**,
/// which is where a user sees it.
#[test]
fn t9_every_parser_bound_refuses_and_counts_through_the_command() {
    let (_dir, root) = indexed_fixture();

    // 1. The report size bound, enforced before the file is read.
    let mut oversized = b"TN:\nSF:src/math.ts\nDA:1,1\nend_of_record\n".to_vec();
    oversized.resize(nerve_index::coverage::MAX_REPORT_BYTES + 1, b'\n');
    std::fs::write(root.join(REPORT), &oversized).unwrap();
    let outcome = ingest(&root, REPORT);
    assert_eq!(outcome.refused_count(parse_form::REPORT_TOO_LARGE), 1);
    assert_eq!(outcome.refused_total(), 1);
    assert_eq!(outcome.symbols_covered, 0);
    assert_eq!(outcome.status, nerve_index::RunStatus::Partial);
    // Refused whole means refused *unread*: no run entity was written for it.
    assert!(outcome.coverage_run_entity_id.is_none());
    assert!(outcome.report_content_hash.is_none());

    // 2. The record-count bound.
    let mut many = String::new();
    for index in 0..nerve_index::coverage::MAX_RECORDS + 3 {
        many.push_str(&format!("SF:src/f{index}.js\nDA:1,1\nend_of_record\n"));
    }
    let outcome = ingest_report(&root, &many);
    assert_eq!(outcome.refused_count(parse_form::RECORDS_EXCEEDED), 3);

    // 3. The per-record line bound.
    let mut lines = String::from("SF:src/math.ts\n");
    for line in 1..=nerve_index::coverage::MAX_LINES_PER_RECORD as u64 + 2 {
        lines.push_str(&format!("DA:{line},1\n"));
    }
    lines.push_str("end_of_record\n");
    let outcome = ingest_report(&root, &lines);
    assert_eq!(outcome.refused_count(parse_form::LINES_EXCEEDED), 2);
}

/// Malformed input never panics **end to end**, not merely in the parser.
#[test]
fn t9_no_malformed_report_can_panic_the_command() {
    let (_dir, root) = indexed_fixture();
    let hostile: [&[u8]; 12] = [
        b"",
        b"\n\n\n",
        b"not an lcov file at all",
        b"TN:\nSF:src/math.ts\nDA:1,1\nDA:2", // truncated mid-record
        b"TN:\nSF:src/math.ts\nDA:1,1\n",     // no end_of_record
        b"end_of_record\n",                   // record with no source file
        b"SF:src/math.ts\nSF:src/shapes.ts\nDA:1,1\nend_of_record\n",
        b"SF:\nDA:1,1\nend_of_record\n", // empty path
        b"SF:src/math.ts\nDA:0,1\nDA:-1,1\nDA:x,y\nend_of_record\n",
        b"SF:src/math.ts\r\nDA:1,1\nDA:2,0\r\nend_of_record\r\n", // mixed line endings
        b"SF:src/math.ts\nDA:1,1\nDA:1,0\nDA:1,9\nend_of_record\n", // duplicates
        &[0xff, 0xfe, 0x80, b'\n', b'S', b'F', b':', 0xff, b'\n'], // invalid UTF-8
    ];
    for (index, body) in hostile.iter().enumerate() {
        std::fs::write(root.join(REPORT), body).unwrap();
        let outcome = nerve_index::ingest_coverage(&root, Path::new(REPORT));
        assert!(
            outcome.is_ok(),
            "input {index} produced an error: {outcome:?}"
        );
    }
    // The database is still usable and still healthy after all of that.
    assert!(nerve_store::status(&open_db(&root)).unwrap().is_healthy());
}

/// A path that leaves the repository is refused by the command itself, before anything is read.
#[test]
fn t9_a_report_outside_the_repository_is_refused_by_the_command() {
    let (dir, root) = indexed_fixture();
    write(
        dir.path(),
        "outside.info",
        "SF:src/math.ts\nDA:1,1\nend_of_record\n",
    );

    for named in ["../outside.info", "/etc/passwd"] {
        let err = nerve_index::ingest_coverage(&root, Path::new(named)).unwrap_err();
        assert!(
            matches!(err, IndexError::PathEscapesRoot(_)),
            "{named}: {err}"
        );
    }
    assert_eq!(
        scalar(
            &open_db(&root),
            "SELECT count(*) FROM entity WHERE kind = 'coverage_run'"
        ),
        0
    );
}

// ---- freshness ---------------------------------------------------------------------------------

/// The point of the slice: a report that predates the code is **visibly** stale, and an ordinary
/// re-index does not delete it (plan §A.4).
#[test]
fn an_edit_to_a_covered_file_makes_its_coverage_stale_rather_than_deleting_it() {
    let (_dir, root) = indexed_fixture();
    ingest(&root, REPORT);

    let prober = nerve_index::RepositoryProber::new(&root).unwrap();
    let freshness = |root: &Path| -> Vec<String> {
        let conn = open_db(root);
        let subject = nerve_store::resolve_selector(&conn, "src/math.ts#clamp").unwrap();
        let nerve_store::Selection::Resolved { entity, .. } = subject else {
            panic!("src/math.ts#clamp must resolve");
        };
        let report = nerve_store::explain(
            &conn,
            &entity.entity_id,
            None,
            &nerve_store::WhyQuery {
                direction: nerve_store::WhyDirection::Both,
                relations: vec![nerve_core::Relation::Covers],
            },
            &prober,
        )
        .unwrap();
        report
            .assertions
            .iter()
            .flat_map(|assertion| assertion.observations.iter())
            .map(|observation| observation.freshness.as_str().to_string())
            .collect()
    };

    assert_eq!(freshness(&root), vec!["fresh".to_string()]);

    // Append a line to the covered file and re-index, exactly as a developer would.
    let source = std::fs::read_to_string(root.join("src/math.ts")).unwrap();
    write(
        &root,
        "src/math.ts",
        &format!("{source}\n// a later thought\n"),
    );
    nerve_index::index_repository(&root).unwrap();

    let conn = open_db(&root);
    assert_eq!(
        scalar(
            &conn,
            "SELECT count(*) FROM assertion WHERE relation = 'COVERS'"
        ),
        6,
        "an ordinary re-index must not destroy coverage edges"
    );
    assert_eq!(
        freshness(&root),
        vec!["stale".to_string()],
        "coverage of an edited file must read as stale"
    );
}

/// A covered file that is **deleted** is a different case from one that is edited: evidence about
/// a file that no longer exists is evidence about nothing, so it goes with the file.
#[test]
fn removing_a_covered_file_withdraws_the_coverage_that_named_it() {
    let (_dir, root) = indexed_fixture();
    ingest(&root, REPORT);
    std::fs::remove_file(root.join("src/shapes.ts")).unwrap();
    nerve_index::index_repository(&root).unwrap();

    let conn = open_db(&root);
    assert_eq!(
        covers(&conn).keys().cloned().collect::<BTreeSet<_>>(),
        BTreeSet::from(["add".to_string(), "clamp".to_string()]),
        "only src/math.ts's coverage survives"
    );
    // The symbols that went with the file are gone, not left as endpoints of a stale edge.
    assert_eq!(
        scalar(
            &conn,
            "SELECT count(*) FROM entity WHERE name IN ('Rectangle', 'area')"
        ),
        0
    );
}

// ---- re-ingestion and idempotence --------------------------------------------------------------

/// Ingesting the same report twice is the same ingestion: byte-identical graph, nothing appended.
///
/// The evidence is withdrawn and restated rather than left alone, because the withdrawal is keyed
/// on the report's *path* and cannot know in advance that the bytes are unchanged. What matters is
/// that the graph does not grow and does not move: an unchanged report says the same thing twice.
#[test]
fn ingesting_the_same_report_twice_changes_nothing() {
    let (_dir, root) = indexed_fixture();
    let first = ingest(&root, REPORT);
    let once = dump_json(&root);

    let second = ingest(&root, REPORT);
    assert_eq!(
        dump_json(&root),
        once,
        "a second ingestion changed the graph"
    );
    assert_eq!(first.coverage_run_entity_id, second.coverage_run_entity_id);
    assert_eq!(second.observations_removed, first.symbols_covered);
    assert_eq!(second.assertions_removed, 0, "nothing lost its support");
    assert_eq!(
        second.entities_removed, 0,
        "the run was restated, not replaced"
    );
}

/// A second run of the suite replaces the first, rather than standing beside it.
#[test]
fn re_ingesting_a_changed_report_replaces_what_that_path_previously_claimed() {
    let (_dir, root) = indexed_fixture();
    let first = ingest(&root, REPORT);

    // A narrower report at the same path: only `add` ran this time.
    let second = ingest_report(
        &root,
        "TN:\nSF:src/math.ts\nDA:1,1\nDA:2,1\nend_of_record\n",
    );
    assert_ne!(
        first.coverage_run_entity_id, second.coverage_run_entity_id,
        "different bytes are a different measurement and a different run"
    );
    assert!(second.observations_removed >= 6);

    let conn = open_db(&root);
    assert_eq!(covers(&conn).keys().collect::<Vec<_>>(), vec!["add"]);
    assert_eq!(
        scalar(
            &conn,
            "SELECT count(*) FROM entity WHERE kind = 'coverage_run'"
        ),
        1,
        "the superseded run must not linger"
    );
}

/// Two reports at two paths are two runs, and both are kept: they are different measurements.
#[test]
fn two_reports_at_two_paths_are_two_runs() {
    let (_dir, root) = indexed_fixture();
    ingest(&root, REPORT);
    write(
        &root,
        "coverage/unit.info",
        "TN:\nSF:src/math.ts\nDA:15,1\nDA:16,1\nend_of_record\n",
    );
    ingest(&root, "coverage/unit.info");

    let conn = open_db(&root);
    assert_eq!(
        scalar(
            &conn,
            "SELECT count(*) FROM entity WHERE kind = 'coverage_run'"
        ),
        2
    );
    // `neverRun` is covered by the second report and by nothing in the first.
    assert!(covers(&conn).contains_key("neverRun"));
    assert!(covers(&conn).contains_key("clamp"));
}

// ---- equivalence ------------------------------------------------------------------------------

/// **Plan §A.4's equivalence.** A full index followed by ingestion and an incremental index
/// followed by ingestion must be byte-identical.
///
/// The incremental side is given a history: the tree is indexed, edited three ways — a symbol
/// body changed, a file added, a file deleted — re-indexed after each, and only then ingested.
/// The reference side indexes the final tree from scratch into a new database and ingests the
/// same report. If the two disagree the incremental path is wrong by definition.
#[test]
fn full_and_incremental_indexes_agree_after_ingestion() {
    let (dir, root) = indexed_fixture();

    write(
        &root,
        "src/math.ts",
        "export function add(a: number, b: number): number {\n  return a + b;\n}\n\n\
         export function clamp(value: number, low: number, high: number): number {\n\
        \x20 if (value < low) {\n    return low;\n  }\n  if (value > high) {\n\
        \x20   return high;\n  }\n  return value;\n}\n\n\
         export function neverRun(value: number): number {\n  return value * 3;\n}\n",
    );
    nerve_index::index_repository(&root).unwrap();

    write(
        &root,
        "src/extra.ts",
        "import { add } from \"./math\";\n\nexport function twice(n: number): number {\n\
        \x20 return add(n, n);\n}\n",
    );
    nerve_index::index_repository(&root).unwrap();

    std::fs::remove_file(root.join("src/extra.ts")).unwrap();
    nerve_index::index_repository(&root).unwrap();

    let incremental = ingest(&root, REPORT);
    let incremental_dump = dump_json(&root);

    // The same tree, indexed from scratch into a fresh database, then ingested.
    let reference_dir = tempfile::tempdir().unwrap();
    let reference = reference_dir.path().join("repo");
    copy_tree(&root, &reference);
    nerve_index::init_with_project_id(&reference, Some(TEST_PROJECT_ID)).unwrap();
    nerve_index::index_repository_with(&reference, IndexOptions { full: true }).unwrap();
    let full = ingest(&reference, REPORT);
    let full_dump = dump_json(&reference);

    if incremental_dump != full_dump {
        let divergence = incremental_dump
            .lines()
            .zip(full_dump.lines())
            .enumerate()
            .find(|(_, (a, b))| a != b)
            .map(|(line, (a, b))| format!("line {line}:\n  incremental: {a}\n  full:        {b}"))
            .unwrap_or_else(|| {
                format!(
                    "lengths differ: incremental {} lines, full {} lines",
                    incremental_dump.lines().count(),
                    full_dump.lines().count()
                )
            });
        panic!("incremental and full disagree after ingestion\n{divergence}");
    }
    assert_eq!(incremental.symbols_covered, full.symbols_covered);
    assert!(incremental.symbols_covered > 0);
    drop(dir);
}

/// The scoped pruning ingestion uses must agree with the whole-table pruner, exactly as the index
/// path's does: after an ingestion, the reference pruner must find nothing left to remove.
#[test]
fn scoped_pruning_after_ingestion_leaves_nothing_for_the_whole_table_pruner() {
    let (_dir, root) = indexed_fixture();
    ingest(&root, REPORT);
    ingest_report(&root, "TN:\nSF:src/math.ts\nDA:1,1\nend_of_record\n");

    let conn = open_db(&root);
    let leftover = nerve_store::prune_orphans(&conn).unwrap();
    assert!(
        leftover.is_empty(),
        "scoped pruning left {leftover:?} for the whole-table pruner"
    );
}

// ---- the command's preconditions ---------------------------------------------------------------

#[test]
fn ingesting_without_an_index_is_refused_with_a_reason() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("repo");
    copy_tree(&named_fixture_root(FIXTURE), &root);

    // No `nerve init` at all.
    let err = nerve_index::ingest_coverage(&root, Path::new(REPORT)).unwrap_err();
    assert!(matches!(err, IndexError::NotInitialized(_)), "{err}");

    // Initialized, but never indexed: there is nothing to resolve a path against.
    nerve_index::init_with_project_id(&root, Some(TEST_PROJECT_ID)).unwrap();
    let err = nerve_index::ingest_coverage(&root, Path::new(REPORT)).unwrap_err();
    assert!(matches!(err, IndexError::NotIndexed(_)), "{err}");
}

/// A path that is not an ordinary file is a wrong argument, not an internal fault.
#[test]
fn a_report_that_is_not_a_file_is_refused_as_a_wrong_argument() {
    let (_dir, root) = indexed_fixture();
    let err = nerve_index::ingest_coverage(&root, Path::new("coverage")).unwrap_err();
    assert!(matches!(err, IndexError::NotAFile(_)), "{err}");
}

/// Ingestion reads the report and the covered files, and touches nothing else in the repository.
#[test]
fn ingestion_leaves_the_rest_of_the_index_exactly_as_it_found_it() {
    let (_dir, root) = indexed_fixture();
    let conn = open_db(&root);
    let before: Vec<String> = strings(
        &conn,
        "SELECT assertion_id || '|' || relation FROM assertion
          WHERE relation <> 'COVERS' ORDER BY assertion_id",
    );
    let observations_before = scalar(
        &conn,
        "SELECT count(*) FROM observation WHERE extractor_id <> 'coverage'",
    );
    drop(conn);

    ingest(&root, REPORT);

    let conn = open_db(&root);
    assert_eq!(
        strings(
            &conn,
            "SELECT assertion_id || '|' || relation FROM assertion
              WHERE relation <> 'COVERS' ORDER BY assertion_id"
        ),
        before
    );
    assert_eq!(
        scalar(
            &conn,
            "SELECT count(*) FROM observation WHERE extractor_id <> 'coverage'"
        ),
        observations_before
    );
}
