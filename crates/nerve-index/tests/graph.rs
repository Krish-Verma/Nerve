//! Golden graph, determinism, idempotence, unresolved references, and derived state.

mod common;

use std::collections::BTreeMap;

use common::{count, fixture_copy, indexed_fixture, open_db, TEST_PROJECT_ID};

fn golden_path() -> std::path::PathBuf {
    common::fixture_root().join("golden.json")
}

fn dump_json(root: &std::path::Path) -> String {
    let conn = open_db(root);
    nerve_store::canonical_dump(&conn)
        .unwrap()
        .to_canonical_json()
        .unwrap()
}

/// Fixture graph, byte-for-byte, against a committed golden file.
///
/// Regenerate deliberately with `NERVE_UPDATE_GOLDEN=1 cargo test -p nerve-index golden`, and
/// review the diff: a change here is a change in what Nerve claims about code.
#[test]
fn golden_graph_matches_the_committed_dump() {
    let (_dir, root) = indexed_fixture();
    let actual = dump_json(&root);
    let path = golden_path();

    if std::env::var("NERVE_UPDATE_GOLDEN").is_ok() {
        std::fs::write(&path, &actual).unwrap();
        return;
    }

    let expected = std::fs::read_to_string(&path).unwrap_or_else(|err| {
        panic!(
            "{} is missing ({err}). Generate it with NERVE_UPDATE_GOLDEN=1.",
            path.display()
        )
    });
    assert_eq!(
        actual,
        expected,
        "canonical dump differs from {}",
        path.display()
    );
}

#[test]
fn canonical_dump_excludes_absolute_paths_and_timestamps() {
    let (_dir, root) = indexed_fixture();
    let dump = dump_json(&root);
    assert!(
        !dump.contains(root.to_str().unwrap()),
        "dump must not contain the absolute root path"
    );
    for excluded in [
        "created_at",
        "started_at",
        "finished_at",
        "observation_id",
        "run_id",
    ] {
        assert!(!dump.contains(excluded), "dump must not contain {excluded}");
    }
}

/// ARCHITECTURE.md invariant 5: two independent indexes of the same tree are byte-identical.
#[test]
fn indexing_twice_into_separate_databases_is_byte_identical() {
    let (_dir_a, root_a) = fixture_copy();
    let (_dir_b, root_b) = fixture_copy();
    for root in [&root_a, &root_b] {
        nerve_index::init_with_project_id(root, Some(TEST_PROJECT_ID)).unwrap();
        nerve_index::index_repository(root).unwrap();
    }
    assert_eq!(dump_json(&root_a), dump_json(&root_b));
}

/// Re-indexing an unchanged tree adds no graph rows and changes no claim.
#[test]
fn re_indexing_is_idempotent() {
    let (_dir, root) = indexed_fixture();
    let graph_tables = [
        "entity",
        "occurrence",
        "assertion",
        "observation",
        "assertion_state",
        "repository",
        "repository_state",
    ];

    let before: BTreeMap<&str, i64> = {
        let conn = open_db(&root);
        graph_tables
            .iter()
            .map(|table| (*table, count(&conn, table)))
            .collect()
    };
    let dump_before = dump_json(&root);
    let runs_before = count(&open_db(&root), "extractor_run");

    let outcome = nerve_index::index_repository(&root).unwrap();
    assert_eq!(outcome.status, nerve_index::RunStatus::Complete);

    let conn = open_db(&root);
    for table in graph_tables {
        assert_eq!(
            count(&conn, table),
            before[table],
            "{table} grew on re-index"
        );
    }
    assert_eq!(dump_json(&root), dump_before, "logical graph changed");
    assert_eq!(
        count(&conn, "extractor_run"),
        runs_before + 2,
        "extractor_run is a run log and grows by one row per extractor"
    );
}

#[test]
fn unresolved_imports_become_entities_not_omissions() {
    let (_dir, root) = indexed_fixture();
    let conn = open_db(&root);

    let mut stmt = conn
        .prepare(
            "SELECT name, scope_path FROM entity
              WHERE kind = 'unresolved' AND json_extract(meta, '$.category') = 'module'
              ORDER BY name",
        )
        .unwrap();
    let rows: Vec<(String, String)> = stmt
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
        .unwrap()
        .map(|row| row.unwrap())
        .collect();
    assert_eq!(
        rows,
        vec![
            (
                "./does-not-exist".to_string(),
                "src/unresolved.ts".to_string()
            ),
            (
                "some-external-pkg".to_string(),
                "src/unresolved.ts".to_string()
            ),
        ]
    );

    // Each has a real IMPORTS assertion whose derived state is flagged.
    let flagged: i64 = conn
        .query_row(
            "SELECT count(*)
               FROM assertion_state s
               JOIN assertion a ON a.assertion_id = s.assertion_id
               JOIN entity t ON t.entity_id = a.target_entity_id
              WHERE t.kind = 'unresolved' AND a.relation = 'IMPORTS'
                AND s.is_unresolved = 1 AND s.status = 'UNRESOLVED'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(flagged, 2);
}

#[test]
fn a_dynamic_import_without_a_literal_produces_no_edge() {
    let (_dir, root) = indexed_fixture();
    let conn = open_db(&root);
    let outcome = nerve_index::index_repository(&root).unwrap();
    assert_eq!(outcome.dynamic_imports_without_specifier, 1);

    // ambiguous.ts has exactly one import expression and it must contribute nothing.
    let imports_from_ambiguous: i64 = conn
        .query_row(
            "SELECT count(*)
               FROM assertion a
               JOIN entity source ON source.entity_id = a.source_entity_id
              WHERE a.relation = 'IMPORTS' AND source.scope_path = 'src/ambiguous.ts'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(imports_from_ambiguous, 0);
}

/// Each extractor owns a disjoint set of relations, and neither strays.
#[test]
fn each_extractor_emits_only_its_own_relations() {
    let (_dir, root) = indexed_fixture();
    let conn = open_db(&root);

    let relations_of = |extractor: &str| -> Vec<String> {
        let mut stmt = conn
            .prepare(
                "SELECT DISTINCT a.relation
                   FROM assertion a
                   JOIN observation o ON o.assertion_id = a.assertion_id
                  WHERE o.extractor_id = ?1
                  ORDER BY a.relation",
            )
            .unwrap();
        stmt.query_map([extractor], |row| row.get(0))
            .unwrap()
            .map(|row| row.unwrap())
            .collect()
    };

    assert_eq!(
        relations_of("ts-js-structural"),
        vec!["CONTAINS", "DEFINES", "EXPORTS", "IMPORTS"]
    );
    assert_eq!(
        relations_of("ts-js-reference"),
        vec!["CALLS", "IMPLEMENTS", "REFERENCES"],
        "ts-basic has no extends clause"
    );
}

/// ADR-0003: an edge produced by resolution says `AST_RESOLVED`; one the tree literally states
/// says `AST_DIRECT`. Slice 1 labelled resolved imports `AST_DIRECT`; plan P2 corrected it.
#[test]
fn evidence_source_types_distinguish_read_from_resolved() {
    let (_dir, root) = indexed_fixture();
    let conn = open_db(&root);
    let mut stmt = conn
        .prepare("SELECT DISTINCT evidence_source_type FROM observation ORDER BY 1")
        .unwrap();
    let types: Vec<String> = stmt
        .query_map([], |row| row.get(0))
        .unwrap()
        .map(|row| row.unwrap())
        .collect();
    assert_eq!(types, vec!["AST_DIRECT", "AST_RESOLVED"]);

    let mut stmt = conn
        .prepare("SELECT DISTINCT directness FROM observation ORDER BY directness")
        .unwrap();
    let directness: Vec<String> = stmt
        .query_map([], |row| row.get(0))
        .unwrap()
        .map(|row| row.unwrap())
        .collect();
    assert_eq!(directness, vec!["DIRECT", "RESOLVED"]);

    // Source type and directness never disagree.
    let mismatched: i64 = conn
        .query_row(
            "SELECT count(*) FROM observation
              WHERE (evidence_source_type = 'AST_RESOLVED') != (directness = 'RESOLVED')",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(mismatched, 0);

    // A resolved import must not still be labelled AST_DIRECT.
    let mislabelled: i64 = conn
        .query_row(
            "SELECT count(*) FROM observation o
               JOIN assertion a ON a.assertion_id = o.assertion_id
               JOIN entity t ON t.entity_id = a.target_entity_id
              WHERE a.relation = 'IMPORTS' AND t.kind = 'module'
                AND o.evidence_source_type != 'AST_RESOLVED'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(mislabelled, 0);
}

/// Both extractors are recorded, each with its own run row and version.
#[test]
fn two_extractor_runs_are_recorded_per_index() {
    let (_dir, root) = indexed_fixture();
    let conn = open_db(&root);
    let report = nerve_store::status(&conn).unwrap();
    let runs: Vec<(String, String)> = report
        .runs
        .iter()
        .map(|run| (run.extractor_id.clone(), run.extractor_version.clone()))
        .collect();
    assert_eq!(
        runs,
        vec![
            ("ts-js-structural".to_string(), "1.1.0".to_string()),
            ("ts-js-reference".to_string(), "1.0.0".to_string()),
        ]
    );
    assert_eq!(
        report.last_run.as_ref().unwrap().extractor_id,
        "ts-js-reference"
    );
    assert!(report.is_healthy());
}

#[test]
fn the_fixture_graph_has_the_expected_shape() {
    let (_dir, root) = indexed_fixture();
    let conn = open_db(&root);
    let report = nerve_store::status(&conn).unwrap();

    let kinds: BTreeMap<&str, i64> = report
        .entities_by_kind
        .iter()
        .map(|(k, v)| (k.as_str(), *v))
        .collect();
    assert_eq!(kinds["repository"], 1);
    assert_eq!(kinds["directory"], 1, "only src/ contains indexed files");
    assert_eq!(kinds["file"], 8);
    assert_eq!(kinds["module"], 8, "File DEFINES Module is 1:1 for TS/JS");
    assert_eq!(kinds["function"], 14);
    assert_eq!(kinds["class"], 2);
    assert_eq!(kinds["interface"], 2);
    assert_eq!(kinds["method"], 6);
    assert_eq!(
        kinds["unresolved"], 6,
        "2 unresolved module specifiers + 4 unresolved call targets"
    );

    let relations: BTreeMap<&str, i64> = report
        .assertions_by_relation
        .iter()
        .map(|(k, v)| (k.as_str(), *v))
        .collect();
    assert_eq!(relations["CONTAINS"], 9);
    assert_eq!(relations["DEFINES"], 32);
    assert_eq!(relations["EXPORTS"], 19);
    assert_eq!(relations["IMPORTS"], 8);
    assert_eq!(relations["CALLS"], 12);
    assert_eq!(relations["REFERENCES"], 5);
    assert_eq!(relations["IMPLEMENTS"], 2);
    assert!(
        !relations.contains_key("EXTENDS"),
        "ts-basic has no extends clause, so the relation must be absent, not zero-valued"
    );
}

/// The two identically named `shared` functions in `ambiguous.ts` must each be called by their
/// own enclosing function. This is the case Slice 1 refused to guess at.
#[test]
fn identically_named_functions_in_sibling_scopes_resolve_separately() {
    let (_dir, root) = indexed_fixture();
    let conn = open_db(&root);

    let mut stmt = conn
        .prepare(
            "SELECT source.name, target.scope_path
               FROM assertion a
               JOIN entity source ON source.entity_id = a.source_entity_id
               JOIN entity target ON target.entity_id = a.target_entity_id
              WHERE a.relation = 'CALLS' AND target.name = 'shared'
              ORDER BY source.name",
        )
        .unwrap();
    let rows: Vec<(String, String)> = stmt
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
        .unwrap()
        .map(|row| row.unwrap())
        .collect();
    assert_eq!(
        rows,
        vec![
            ("outerA".to_string(), "outerA".to_string()),
            ("outerB".to_string(), "outerB".to_string()),
        ]
    );
}

#[test]
fn excluded_paths_contribute_no_entities() {
    let (_dir, root) = indexed_fixture();
    let conn = open_db(&root);
    for forbidden in ["bundledArtifact", "vendoredHelper", "EXAMPLE_TOKEN"] {
        let hits: i64 = conn
            .query_row(
                "SELECT count(*) FROM entity WHERE name = ?1",
                [forbidden],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(hits, 0, "{forbidden} came from an excluded path");
    }
    let paths: i64 = conn
        .query_row(
            "SELECT count(*) FROM occurrence
              WHERE file_path LIKE 'dist/%' OR file_path LIKE 'node_modules/%'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(paths, 0);
}

#[test]
fn re_exports_keep_the_defining_module_identity() {
    let (_dir, root) = indexed_fixture();
    let conn = open_db(&root);

    // index.ts re-exports `add` from math.ts; the target must be math.ts's own entity.
    let target_scope: String = conn
        .query_row(
            "SELECT t.scope_path
               FROM assertion a
               JOIN entity source ON source.entity_id = a.source_entity_id
               JOIN entity t ON t.entity_id = a.target_entity_id
              WHERE a.relation = 'EXPORTS'
                AND source.scope_path = 'src/index.ts'
                AND t.name = 'add'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(target_scope, "", "add is module-scope in math.ts");

    let defining_module: i64 = conn
        .query_row(
            "SELECT count(*)
               FROM assertion a
               JOIN entity source ON source.entity_id = a.source_entity_id
               JOIN entity t ON t.entity_id = a.target_entity_id
              WHERE a.relation = 'DEFINES' AND t.name = 'add'
                AND source.scope_path = 'src/math.ts'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(defining_module, 1, "barrel files must not clone entities");
}

/// ADR-0003: `assertion_state` is a pure function of `observation`.
#[test]
fn assertion_state_is_a_pure_rebuild() {
    let (_dir, root) = indexed_fixture();
    let conn = open_db(&root);

    let snapshot = |conn: &nerve_store::Connection| -> Vec<String> {
        let mut stmt = conn
            .prepare(
                "SELECT assertion_id || '|' || status || '|' ||
                        strongest_source_type || '|' || source_type_mask || '|' ||
                        observation_count || '|' || is_unresolved
                   FROM assertion_state ORDER BY assertion_id",
            )
            .unwrap();
        stmt.query_map([], |row| row.get(0))
            .unwrap()
            .map(|row| row.unwrap())
            .collect()
    };

    let before = snapshot(&conn);
    assert!(!before.is_empty());

    conn.execute("DELETE FROM assertion_state", []).unwrap();
    assert_eq!(count(&conn, "assertion_state"), 0);

    nerve_store::rebuild_assertion_state(&conn).unwrap();
    assert_eq!(snapshot(&conn), before, "rebuild is not a pure function");

    // Rebuilding again must not accumulate or drift.
    nerve_store::rebuild_assertion_state(&conn).unwrap();
    assert_eq!(snapshot(&conn), before, "rebuild is not idempotent");
}

#[test]
fn derived_state_carries_the_source_type_mask() {
    let (_dir, root) = indexed_fixture();
    let conn = open_db(&root);
    let mut stmt = conn
        .prepare(
            "SELECT DISTINCT source_type_mask, strongest_source_type
               FROM assertion_state ORDER BY source_type_mask",
        )
        .unwrap();
    let rows: Vec<(i64, String)> = stmt
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
        .unwrap()
        .map(|row| row.unwrap())
        .collect();
    // Bit 0 is AST_DIRECT, bit 1 is AST_RESOLVED. No assertion has both yet: one extractor
    // observes each relation, and each relation is either resolved or not.
    assert_eq!(
        rows,
        vec![
            (1, "AST_DIRECT".to_string()),
            (2, "AST_RESOLVED".to_string())
        ]
    );
}

#[test]
fn every_assertion_has_derived_state_and_supporting_evidence() {
    let (_dir, root) = indexed_fixture();
    let conn = open_db(&root);
    let orphans: i64 = conn
        .query_row(
            "SELECT count(*) FROM assertion a
              WHERE NOT EXISTS (SELECT 1 FROM assertion_state s WHERE s.assertion_id = a.assertion_id)
                 OR NOT EXISTS (SELECT 1 FROM observation o WHERE o.assertion_id = a.assertion_id)",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(orphans, 0);
}

#[test]
fn no_source_text_is_stored() {
    let (_dir, root) = indexed_fixture();
    let conn = open_db(&root);
    // A distinctive literal from the fixture that is not a symbol name.
    let needle = "unresolved fixture";
    for (table, column) in [
        ("entity", "meta"),
        ("observation", "details"),
        ("entity", "name"),
    ] {
        let hits: i64 = conn
            .query_row(
                &format!("SELECT count(*) FROM {table} WHERE {column} LIKE '%' || ?1 || '%'"),
                [needle],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(hits, 0, "{table}.{column} contains source text");
    }
}

#[test]
fn index_without_init_is_refused() {
    let (_dir, root) = fixture_copy();
    let err = nerve_index::index_repository(&root).unwrap_err();
    assert!(
        matches!(err, nerve_index::IndexError::NotInitialized(_)),
        "{err}"
    );
}
