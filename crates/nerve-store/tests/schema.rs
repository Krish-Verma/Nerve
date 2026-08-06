//! Migration, FTS5 availability, and the derived-state boundary.

use nerve_store::{migrate, open, open_in_memory, schema_version, SCHEMA_VERSION};

/// Tables and indexes a v1 database has, and nothing a later version added.
///
/// Written out rather than generated, so that "upgrading a v1 database" is checked against the
/// shape v1 actually shipped rather than against whatever the current build happens to produce.
const V1_ONLY: &str = r#"
CREATE TABLE schema_version (
    version     INTEGER PRIMARY KEY,
    applied_at  TEXT NOT NULL,
    description TEXT NOT NULL
);
CREATE TABLE repository (
    repo_id TEXT PRIMARY KEY, project_id TEXT NOT NULL,
    root_path TEXT NOT NULL, created_at TEXT NOT NULL);
CREATE TABLE repository_state (
    state_id TEXT PRIMARY KEY, repo_id TEXT NOT NULL REFERENCES repository(repo_id),
    kind TEXT NOT NULL, git_commit TEXT, content_merkle TEXT NOT NULL, created_at TEXT NOT NULL);
CREATE TABLE entity (
    entity_id TEXT PRIMARY KEY, repo_id TEXT NOT NULL REFERENCES repository(repo_id),
    kind TEXT NOT NULL, name TEXT NOT NULL, scope_path TEXT NOT NULL,
    language TEXT, meta TEXT);
CREATE TABLE occurrence (
    occurrence_id TEXT PRIMARY KEY, entity_id TEXT NOT NULL REFERENCES entity(entity_id),
    state_id TEXT NOT NULL REFERENCES repository_state(state_id), file_path TEXT NOT NULL,
    start_byte INTEGER NOT NULL, end_byte INTEGER NOT NULL, start_line INTEGER NOT NULL,
    start_col INTEGER NOT NULL, end_line INTEGER NOT NULL, end_col INTEGER NOT NULL,
    content_hash TEXT NOT NULL);
CREATE TABLE assertion (
    assertion_id TEXT PRIMARY KEY, repo_id TEXT NOT NULL REFERENCES repository(repo_id),
    source_entity_id TEXT NOT NULL REFERENCES entity(entity_id), relation TEXT NOT NULL,
    target_entity_id TEXT NOT NULL REFERENCES entity(entity_id));
CREATE TABLE extractor_run (
    run_id INTEGER PRIMARY KEY AUTOINCREMENT,
    repo_id TEXT NOT NULL REFERENCES repository(repo_id),
    state_id TEXT NOT NULL REFERENCES repository_state(state_id),
    extractor_id TEXT NOT NULL, extractor_version TEXT NOT NULL, started_at TEXT NOT NULL,
    finished_at TEXT, files_processed INTEGER NOT NULL DEFAULT 0,
    files_failed INTEGER NOT NULL DEFAULT 0, status TEXT NOT NULL);
CREATE TABLE observation (
    observation_id INTEGER PRIMARY KEY AUTOINCREMENT,
    assertion_id TEXT NOT NULL REFERENCES assertion(assertion_id),
    extractor_run_id INTEGER NOT NULL REFERENCES extractor_run(run_id),
    evidence_source_type TEXT NOT NULL, directness TEXT NOT NULL, extractor_id TEXT NOT NULL,
    extractor_version TEXT NOT NULL, match_quality REAL,
    state_id TEXT NOT NULL REFERENCES repository_state(state_id), file_path TEXT NOT NULL,
    start_line INTEGER NOT NULL, end_line INTEGER NOT NULL, content_hash TEXT NOT NULL,
    environment TEXT, details TEXT, created_at TEXT NOT NULL);
CREATE TABLE assertion_state (
    assertion_id TEXT PRIMARY KEY REFERENCES assertion(assertion_id), state_id TEXT NOT NULL,
    status TEXT NOT NULL, strongest_source_type TEXT NOT NULL, source_type_mask INTEGER NOT NULL,
    observation_count INTEGER NOT NULL, is_unresolved INTEGER NOT NULL,
    last_seen_state_id TEXT NOT NULL);
CREATE TABLE identity_link (
    link_id INTEGER PRIMARY KEY AUTOINCREMENT,
    repo_id TEXT NOT NULL REFERENCES repository(repo_id), left_entity_id TEXT NOT NULL,
    right_entity_id TEXT NOT NULL, link_kind TEXT NOT NULL, evidence TEXT,
    created_at TEXT NOT NULL);
CREATE VIRTUAL TABLE entity_fts USING fts5(
    name, scope_path, content='entity', content_rowid='rowid');
INSERT INTO schema_version (version, applied_at, description)
    VALUES (1, '2026-01-01T00:00:00.000Z', 'Slice 1');
"#;

/// What v2 added on top of v1, exactly as the Slice 3 build shipped it.
const V2_ONLY: &str = r#"
CREATE TABLE module_facts (
    repo_id             TEXT NOT NULL REFERENCES repository(repo_id),
    rel_path            TEXT NOT NULL,
    content_hash        TEXT NOT NULL,
    language            TEXT NOT NULL,
    structural_version  TEXT NOT NULL,
    reference_version   TEXT NOT NULL,
    facts               TEXT NOT NULL,
    PRIMARY KEY (repo_id, rel_path)
);
CREATE INDEX idx_module_facts_hash ON module_facts(repo_id, content_hash);
CREATE UNIQUE INDEX idx_identity_link_identity
    ON identity_link(repo_id, left_entity_id, right_entity_id, link_kind);
INSERT INTO schema_version (version, applied_at, description)
    VALUES (2, '2026-01-01T00:00:00.000Z', 'Slice 3');
"#;

/// What v3 did to the shape on disk, exactly as the Slice 3b build shipped it.
///
/// Written out rather than reached by calling `migrate`, for the same reason [`V1_ONLY`] is: "a
/// v3 database" has to mean what v3 actually left behind, not what today's migration chain
/// happens to produce on its way past.
const V3_ONLY: &str = r#"
DROP INDEX IF EXISTS idx_occurrence_state;
DROP INDEX IF EXISTS idx_observation_state;
DROP INDEX IF EXISTS idx_observation_identity;
ALTER TABLE occurrence      DROP COLUMN state_id;
ALTER TABLE observation     DROP COLUMN state_id;
ALTER TABLE assertion_state DROP COLUMN state_id;
ALTER TABLE assertion_state DROP COLUMN last_seen_state_id;
CREATE UNIQUE INDEX idx_observation_identity ON observation(
    assertion_id, extractor_id, extractor_version,
    evidence_source_type, file_path, start_line, end_line);
INSERT INTO schema_version (version, applied_at, description)
    VALUES (3, '2026-01-01T00:00:00.000Z', 'Slice 3b');
"#;

/// What v4 did to the shape on disk, which is **nothing**: Slice 5d-i is a data correction with no
/// DDL, so the only trace a v4 build left is its marker row.
///
/// Written out anyway, so that "a v5 database" can be assembled from the same written-out steps as
/// every other starting point rather than by calling `migrate` part of the way and trusting it.
const V4_ONLY: &str = r#"
INSERT INTO schema_version (version, applied_at, description)
    VALUES (4, '2026-01-01T00:00:00.000Z', 'Slice 5d-i');
"#;

/// What v5 added on top of v4, exactly as the Slice 10a build shipped it.
const V5_ONLY: &str = r#"
ALTER TABLE module_facts ADD COLUMN framework_version TEXT NOT NULL DEFAULT '';
INSERT INTO schema_version (version, applied_at, description)
    VALUES (5, '2026-01-01T00:00:00.000Z', 'Slice 10a');
"#;

/// The four tables schema v6 adds, in the order the migration creates them.
const V6_TABLES: [&str; 4] = [
    "git_commit",
    "git_change",
    "git_rename_hypothesis",
    "git_history_ingest",
];

/// What v6 added on top of v5, exactly as the Slice 12b build shipped it.
///
/// Written out rather than reached by calling `migrate` part of the way, for the same reason
/// [`V1_ONLY`] is: "a v6 database" has to mean what v6 actually left behind. It is the starting
/// point for the v7 rebuild, and `git_rename_hypothesis.blob_oid` — the single column v7 splits in
/// two — only exists here.
const V6_ONLY: &str = r#"
CREATE TABLE git_commit (
    repo_id              TEXT    NOT NULL REFERENCES repository(repo_id),
    commit_oid           TEXT    NOT NULL,
    tree_oid             TEXT    NOT NULL,
    parent_oids          TEXT    NOT NULL,
    parent_completeness  TEXT    NOT NULL,
    changes_enumerated   TEXT    NOT NULL,
    author_time          INTEGER NOT NULL,
    author_tz            TEXT    NOT NULL,
    committer_time       INTEGER NOT NULL,
    committer_tz         TEXT    NOT NULL,
    author_ident         TEXT,
    committer_ident      TEXT,
    summary              TEXT    NOT NULL,
    is_merge             INTEGER NOT NULL,
    PRIMARY KEY (repo_id, commit_oid)
);
CREATE INDEX idx_git_commit_time ON git_commit(repo_id, committer_time);
CREATE TABLE git_change (
    repo_id        TEXT    NOT NULL REFERENCES repository(repo_id),
    commit_oid     TEXT    NOT NULL,
    path           TEXT    NOT NULL,
    change_kind    TEXT    NOT NULL,
    blob_oid       TEXT,
    prev_blob_oid  TEXT,
    mode           INTEGER,
    prev_mode      INTEGER,
    PRIMARY KEY (repo_id, commit_oid, path),
    FOREIGN KEY (repo_id, commit_oid) REFERENCES git_commit(repo_id, commit_oid)
);
CREATE INDEX idx_git_change_path ON git_change(repo_id, path);
CREATE INDEX idx_git_change_blob ON git_change(repo_id, blob_oid);
CREATE TABLE git_rename_hypothesis (
    repo_id       TEXT NOT NULL REFERENCES repository(repo_id),
    commit_oid    TEXT NOT NULL,
    from_path     TEXT NOT NULL,
    to_path       TEXT NOT NULL,
    evidence      TEXT NOT NULL,
    blob_oid      TEXT NOT NULL,
    ambiguity     TEXT NOT NULL,
    PRIMARY KEY (repo_id, commit_oid, from_path, to_path),
    FOREIGN KEY (repo_id, commit_oid) REFERENCES git_commit(repo_id, commit_oid)
);
CREATE TABLE git_history_ingest (
    repo_id             TEXT    PRIMARY KEY REFERENCES repository(repo_id),
    head_oid            TEXT,
    walked_from         TEXT    NOT NULL,
    commits_recorded    INTEGER NOT NULL,
    commit_budget       INTEGER NOT NULL,
    walk_terminated_by  TEXT    NOT NULL,
    shallow             INTEGER NOT NULL,
    shallow_boundary    TEXT    NOT NULL,
    promisor            INTEGER NOT NULL,
    refusals            TEXT    NOT NULL,
    reader_version      TEXT    NOT NULL,
    ingested_at         TEXT    NOT NULL
);
INSERT INTO schema_version (version, applied_at, description)
    VALUES (6, '2026-01-01T00:00:00.000Z', 'Slice 12b');
"#;

/// The table v7 adds and the column it adds to `git_commit`.
const V7_TABLE: &str = "git_rename_analysis";

/// A v7 `git_commit` row, named by column rather than by position.
///
/// Positional `VALUES` is what v7 broke: `ALTER TABLE … ADD COLUMN` appends `summary_truncation`
/// after `is_merge`, so an unqualified insert written against v6 now puts the merge flag in the
/// vocabulary column. Naming the columns is not tidiness here — it is the reason these tests keep
/// asserting what they were written to assert.
const COMMIT_INSERT: &str = "INSERT INTO git_commit
     (repo_id, commit_oid, tree_oid, parent_oids, parent_completeness, changes_enumerated,
      author_time, author_tz, committer_time, committer_tz, author_ident, committer_ident,
      summary, summary_truncation, is_merge)
 VALUES ('r', ?1, 'aa', '[]', 'root', 'enumerated', 100, '+0000', 100, '+0000',
         NULL, NULL, 'first', 'complete', 0)";

/// A v7 exact-content rename hypothesis: one blob named twice, no measurement.
const EXACT_RENAME_INSERT: &str = "INSERT INTO git_rename_hypothesis
     (repo_id, commit_oid, from_path, to_path, evidence, from_blob_oid, to_blob_oid,
      matcher_id, matcher_version, match_numerator, match_denominator, ambiguity)
 VALUES ('r', ?1, 'old.ts', 'new.ts', 'exact_content', ?2, ?2,
         'git-blob-oid', '1', NULL, NULL, 'unique')";

/// Filesystem containment as every build before Slice 5d-i wrote it, plus the two controls that
/// must survive the v4 rewrite untouched.
///
/// The qualifying rows are the `CONTAINS` edges whose **source** is the repository or a directory
/// — attributed to `ts-js-structural` / `AST_DIRECT` for a directory and a `.ts` file, and to
/// `md-structural` / `DOCUMENT_STATED` for a `.md` file, which is exactly the split by file
/// extension that Slice 5d-i removes.
///
/// The controls are `File CONTAINS Document` (source kind `file`, so outside the query) and the
/// skeleton's `Module DEFINES Function` (not `CONTAINS` at all). A migration that reached either
/// would be rewriting a claim a heading scan or a parse genuinely produced.
///
/// `state` is `Some` for the v1 and v2 layouts, which still carry `observation.state_id`.
fn filesystem_rows(state: Option<&str>) -> String {
    let state_column = if state.is_some() { ", state_id" } else { "" };
    let state_value = match state {
        Some(state) => format!(", '{state}'"),
        None => String::new(),
    };
    format!(
        "INSERT INTO entity VALUES ('repo1','r','repository','.','',NULL,NULL);
         INSERT INTO entity VALUES ('dir1','r','directory','docs','',NULL,NULL);
         INSERT INTO entity VALUES ('f_md','r','file','ROADMAP.md','docs','markdown',NULL);
         INSERT INTO entity VALUES ('f_ts','r','file','math.ts','','typescript',NULL);
         INSERT INTO entity
             VALUES ('doc1','r','document','ROADMAP','docs/ROADMAP.md','markdown',NULL);
         INSERT INTO assertion VALUES ('c_dir','r','repo1','CONTAINS','dir1');
         INSERT INTO assertion VALUES ('c_md','r','dir1','CONTAINS','f_md');
         INSERT INTO assertion VALUES ('c_ts','r','repo1','CONTAINS','f_ts');
         INSERT INTO assertion VALUES ('c_doc','r','f_md','CONTAINS','doc1');
         INSERT INTO observation
             (assertion_id, extractor_run_id, evidence_source_type, directness,
              extractor_id, extractor_version, file_path, start_line, end_line,
              content_hash, created_at{state_column})
         VALUES
             ('c_dir',1,'AST_DIRECT','DIRECT','ts-js-structural','1.1.0',
              'docs',0,0,'h','t'{state_value}),
             ('c_ts',1,'AST_DIRECT','DIRECT','ts-js-structural','1.1.0',
              'math.ts',0,0,'h','t'{state_value}),
             ('c_md',1,'DOCUMENT_STATED','DIRECT','md-structural','1.1.0',
              'docs/ROADMAP.md',0,0,'h','t'{state_value}),
             ('c_doc',1,'DOCUMENT_STATED','DIRECT','md-structural','1.1.0',
              'docs/ROADMAP.md',1,1,'h','t'{state_value});"
    )
}

/// One repository state's worth of graph rows, as a v1 or v2 build would have written them.
fn rows_at_state(state: &str, occurrence_id: &str, observation_id: i64, run_id: i64) -> String {
    format!(
        "INSERT INTO repository_state VALUES ('{state}','r','content',NULL,'m-{state}','t');
         INSERT INTO extractor_run
             VALUES ({run_id},'r','{state}','ts-js-structural','1.1.0','t','t',1,0,'complete');
         INSERT INTO occurrence
             VALUES ('{occurrence_id}','e1','{state}','src/math.ts',0,10,1,0,1,10,'h');
         INSERT INTO observation
             VALUES ({observation_id},'a1',{run_id},'AST_DIRECT','DIRECT','ts-js-structural',
                     '1.1.0',NULL,'{state}','src/math.ts',1,1,'h',NULL,NULL,'t');"
    )
}

const SKELETON: &str = "INSERT INTO repository VALUES ('r','p','/tmp','t');
     INSERT INTO entity VALUES ('e1','r','function','add','',NULL,NULL);
     INSERT INTO entity VALUES ('e2','r','module','math','src/math.ts',NULL,NULL);
     INSERT INTO assertion VALUES ('a1','r','e2','DEFINES','e1');";

/// Build a database as a v1 build would have left it, with rows in it.
fn v1_database_with_rows() -> nerve_store::Connection {
    let conn = open_in_memory().unwrap();
    conn.execute_batch(V1_ONLY).unwrap();
    conn.execute_batch(SKELETON).unwrap();
    conn.execute_batch(&rows_at_state("s", "o1", 1, 1)).unwrap();
    conn.execute_batch(
        "INSERT INTO assertion_state VALUES ('a1','s','SUPPORTED','AST_DIRECT',1,1,0,'s');",
    )
    .unwrap();
    conn
}

/// Build a database as the Slice 3 (v2) build would have left it, with rows in it.
///
/// v2 restated every surviving row onto one state on every run, so a v2 database has exactly one
/// occurrence per (entity, span) and one observation per claim — the shape v3 wants.
fn v2_database_with_rows() -> nerve_store::Connection {
    let conn = open_in_memory().unwrap();
    conn.execute_batch(V1_ONLY).unwrap();
    conn.execute_batch(V2_ONLY).unwrap();
    conn.execute_batch(SKELETON).unwrap();
    conn.execute_batch(&rows_at_state("s", "o1", 1, 1)).unwrap();
    conn.execute_batch(
        "INSERT INTO assertion_state VALUES ('a1','s','SUPPORTED','AST_DIRECT',1,1,0,'s');",
    )
    .unwrap();
    conn.execute_batch(
        "INSERT INTO module_facts VALUES ('r','src/math.ts','h','typescript','1.1.0','1.0.0','{}');",
    )
    .unwrap();
    conn
}

/// Build a database as the Slice 10a (v5) build would have left it, with rows in it.
///
/// v5 is the version every database in existence is at before Slice 12b, so this is the upgrade
/// path a real user takes. Assembled from the written-out steps rather than by migrating to 5 and
/// stopping, so "a v5 database" means what v5 shipped.
fn v5_database_with_rows() -> nerve_store::Connection {
    let conn = open_in_memory().unwrap();
    conn.execute_batch(V1_ONLY).unwrap();
    conn.execute_batch(V2_ONLY).unwrap();
    conn.execute_batch(SKELETON).unwrap();
    conn.execute_batch(&rows_at_state("s", "o1", 1, 1)).unwrap();
    conn.execute_batch(
        "INSERT INTO assertion_state VALUES ('a1','s','SUPPORTED','AST_DIRECT',1,1,0,'s');",
    )
    .unwrap();
    conn.execute_batch(V3_ONLY).unwrap();
    conn.execute_batch(V4_ONLY).unwrap();
    conn.execute_batch(V5_ONLY).unwrap();
    conn.execute_batch(
        "INSERT INTO module_facts
             VALUES ('r','src/math.ts','h','typescript','1.1.0','1.0.0','{}','2.0.0');",
    )
    .unwrap();
    assert_eq!(schema_version(&conn).unwrap(), Some(5));
    conn
}

/// A v1 database that was re-indexed: insert-only, so the same location and the same evidence
/// appear once per repository state.
fn v1_database_indexed_twice() -> nerve_store::Connection {
    let conn = open_in_memory().unwrap();
    conn.execute_batch(V1_ONLY).unwrap();
    conn.execute_batch(SKELETON).unwrap();
    conn.execute_batch(&rows_at_state("s1", "o1", 1, 1))
        .unwrap();
    conn.execute_batch(&rows_at_state("s2", "o2", 2, 2))
        .unwrap();
    conn.execute_batch(
        "INSERT INTO assertion_state VALUES ('a1','s2','SUPPORTED','AST_DIRECT',2,1,0,'s2');",
    )
    .unwrap();
    conn
}

/// A database at `version`, holding both the qualifying filesystem rows and the controls.
///
/// One builder for all three starting points, so that "v1 → v4", "v2 → v4" and "v3 → v4" are the
/// same assertions applied to three different arrival routes rather than three tests that could
/// drift apart.
fn database_with_filesystem_rows(version: i64) -> nerve_store::Connection {
    let conn = open_in_memory().unwrap();
    conn.execute_batch(V1_ONLY).unwrap();
    if version >= 2 {
        conn.execute_batch(V2_ONLY).unwrap();
    }
    conn.execute_batch(SKELETON).unwrap();
    conn.execute_batch(&rows_at_state("s", "o1", 1, 1)).unwrap();
    conn.execute_batch(
        "INSERT INTO assertion_state VALUES ('a1','s','SUPPORTED','AST_DIRECT',1,1,0,'s');",
    )
    .unwrap();
    // v3 retires the state columns, so the rows go in under whichever layout is current.
    if version >= 3 {
        conn.execute_batch(V3_ONLY).unwrap();
        conn.execute_batch(&filesystem_rows(None)).unwrap();
    } else {
        conn.execute_batch(&filesystem_rows(Some("s"))).unwrap();
    }
    assert_eq!(schema_version(&conn).unwrap(), Some(version));
    conn
}

/// Every observation, as `assertion → source type / extractor id / extractor version`.
fn observation_labels(conn: &nerve_store::Connection) -> Vec<String> {
    let mut stmt = conn
        .prepare(
            "SELECT assertion_id || ' -> ' || evidence_source_type || ' / '
                 || extractor_id || ' / ' || extractor_version
               FROM observation ORDER BY 1",
        )
        .unwrap();
    let rows = stmt.query_map([], |row| row.get::<_, String>(0)).unwrap();
    rows.map(|row| row.unwrap()).collect()
}

/// The v4 outcome, asserted identically whichever version the database started at.
fn assert_v4_re_attributed(conn: &nerve_store::Connection) {
    assert_eq!(schema_version(conn).unwrap(), Some(SCHEMA_VERSION));

    // Five observations in, five observations out. `observation_id` is an autoincrement surrogate
    // key — there is no content-derived observation id — so v4 updates in place and can neither
    // orphan a row nor leave a duplicate behind. This is the assertion that would catch it if it
    // ever did.
    assert_eq!(scalar(conn, "SELECT count(*) FROM observation"), 5);
    assert_eq!(
        observation_labels(conn),
        vec![
            // The two claims a parse and a heading scan genuinely produced: untouched.
            "a1 -> AST_DIRECT / ts-js-structural / 1.1.0".to_string(),
            "c_dir -> FILESYSTEM_OBSERVED / fs-structural / 1.0.0".to_string(),
            "c_doc -> DOCUMENT_STATED / md-structural / 1.1.0".to_string(),
            "c_md -> FILESYSTEM_OBSERVED / fs-structural / 1.0.0".to_string(),
            "c_ts -> FILESYSTEM_OBSERVED / fs-structural / 1.0.0".to_string(),
        ]
    );

    // Derived state was re-derived, not hand-patched: bit 11 is `FILESYSTEM_OBSERVED`, and its
    // value comes from the same runtime-generated `CASE` the indexer uses.
    let mut stmt = conn
        .prepare(
            "SELECT assertion_id || ' -> ' || strongest_source_type || ' / ' || source_type_mask
               FROM assertion_state ORDER BY 1",
        )
        .unwrap();
    let states: Vec<String> = stmt
        .query_map([], |row| row.get::<_, String>(0))
        .unwrap()
        .map(|row| row.unwrap())
        .collect();
    assert_eq!(
        states,
        vec![
            "a1 -> AST_DIRECT / 1".to_string(),
            "c_dir -> FILESYSTEM_OBSERVED / 2048".to_string(),
            "c_doc -> DOCUMENT_STATED / 256".to_string(),
            "c_md -> FILESYSTEM_OBSERVED / 2048".to_string(),
            "c_ts -> FILESYSTEM_OBSERVED / 2048".to_string(),
        ]
    );

    // Nothing else moved: entities, assertions and occurrences are not evidence.
    assert_eq!(scalar(conn, "SELECT count(*) FROM entity"), 7);
    assert_eq!(scalar(conn, "SELECT count(*) FROM assertion"), 5);
    assert_eq!(scalar(conn, "SELECT count(*) FROM occurrence"), 1);
}

fn column_names(conn: &nerve_store::Connection, table: &str) -> Vec<String> {
    let mut stmt = conn
        .prepare(&format!(
            "SELECT name FROM pragma_table_info('{table}') ORDER BY name"
        ))
        .unwrap();
    let rows = stmt.query_map([], |row| row.get::<_, String>(0)).unwrap();
    rows.map(|row| row.unwrap()).collect()
}

fn scalar(conn: &nerve_store::Connection, sql: &str) -> i64 {
    conn.query_row(sql, [], |row| row.get(0)).unwrap()
}

fn table_names(conn: &nerve_store::Connection) -> Vec<String> {
    let mut stmt = conn
        .prepare("SELECT name FROM sqlite_master WHERE type='table' ORDER BY name")
        .unwrap();
    let rows = stmt.query_map([], |row| row.get::<_, String>(0)).unwrap();
    rows.map(|row| row.unwrap()).collect()
}

#[test]
fn fresh_database_reaches_the_current_schema_version() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("nerve.db");
    let conn = open(&path).unwrap();
    assert_eq!(schema_version(&conn).unwrap(), None);
    migrate(&conn).unwrap();
    assert_eq!(schema_version(&conn).unwrap(), Some(SCHEMA_VERSION));
    assert_eq!(SCHEMA_VERSION, 7);
    // A fresh database reaches v6 directly, without a v1, v2, v3, v4 or v5 database ever existing.
    // v5's column is present from the start, with the default that makes a *migrated* row miss the
    // framework cache. On a fresh database nothing has been cached yet, so the default is inert
    // here and the upgrade path is what has to be tested separately.
    assert!(column_names(&conn, "module_facts").contains(&"framework_version".to_string()));
    assert!(table_names(&conn).contains(&"module_facts".to_string()));
    // v6's four tables, present and empty. Empty is the correct state: v6 adds tables and touches
    // no user row, so it is not a destructive migration and there is nothing for it to fill in.
    for table in V6_TABLES {
        assert!(
            table_names(&conn).contains(&table.to_string()),
            "v6 table {table} is missing from a fresh database"
        );
        assert_eq!(scalar(&conn, &format!("SELECT count(*) FROM {table}")), 0);
    }
    // v7's table and column, present from the start. `summary_truncation` carries the migration's
    // `'unknown'` default here too, and that is inert rather than wrong: on a fresh database no
    // commit exists to be mislabelled, and `insert_commit` supplies the value explicitly. The
    // upgrade path is where the default is load-bearing, and it is tested separately.
    assert!(
        table_names(&conn).contains(&V7_TABLE.to_string()),
        "v7 table {V7_TABLE} is missing from a fresh database"
    );
    assert_eq!(
        scalar(&conn, &format!("SELECT count(*) FROM {V7_TABLE}")),
        0
    );
    assert!(column_names(&conn, "git_commit").contains(&"summary_truncation".to_string()));
    for column in [
        "from_blob_oid",
        "to_blob_oid",
        "matcher_id",
        "matcher_version",
    ] {
        assert!(
            column_names(&conn, "git_rename_hypothesis").contains(&column.to_string()),
            "v7 column git_rename_hypothesis.{column} is missing from a fresh database"
        );
    }
    assert!(
        !column_names(&conn, "git_rename_hypothesis").contains(&"blob_oid".to_string()),
        "v6's single blob column survived the rebuild"
    );
    for (table, column) in [
        ("occurrence", "state_id"),
        ("observation", "state_id"),
        ("assertion_state", "state_id"),
        ("assertion_state", "last_seen_state_id"),
    ] {
        assert!(
            !column_names(&conn, table).contains(&column.to_string()),
            "ADR-0006: {table}.{column} must not exist at v3"
        );
    }
    // The state has not vanished from the database — it moved to where it belongs.
    assert!(column_names(&conn, "extractor_run").contains(&"state_id".to_string()));
    assert!(column_names(&conn, "repository_state").contains(&"state_id".to_string()));
}

/// A database written by the Slice 1/2 build must upgrade in place, keeping every row.
#[test]
fn a_v1_database_upgrades_to_the_current_version_without_losing_rows() {
    let conn = v1_database_with_rows();
    assert_eq!(schema_version(&conn).unwrap(), Some(1));
    assert!(!table_names(&conn).contains(&"module_facts".to_string()));

    migrate(&conn).unwrap();

    assert_eq!(schema_version(&conn).unwrap(), Some(SCHEMA_VERSION));
    assert!(table_names(&conn).contains(&"module_facts".to_string()));

    for (table, expected) in [
        ("repository", 1),
        ("repository_state", 1),
        ("entity", 2),
        ("assertion", 1),
        ("occurrence", 1),
        ("observation", 1),
        ("assertion_state", 1),
        ("extractor_run", 1),
        ("module_facts", 0),
    ] {
        let count: i64 = conn
            .query_row(&format!("SELECT count(*) FROM {table}"), [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(
            count, expected,
            "{table} lost or gained rows in the upgrade"
        );
    }

    // v1's own definitions are untouched except for the columns v3 explicitly retires.
    let entity_sql: String = conn
        .query_row(
            "SELECT sql FROM sqlite_master WHERE type='table' AND name='entity'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert!(entity_sql.contains("scope_path"));
    assert!(!entity_sql.contains("module_facts"));

    // The surviving occurrence carries its ADR-0006 identity, not its v1 one.
    let occurrence_id: String = conn
        .query_row("SELECT occurrence_id FROM occurrence", [], |row| row.get(0))
        .unwrap();
    assert_eq!(
        occurrence_id,
        nerve_core::ids::occurrence_id("e1", "src/math.ts", 0, 10),
        "occurrence_id was not restated by the v3 migration"
    );
    // Everything else about the row survived.
    assert_eq!(
        scalar(
            &conn,
            "SELECT count(*) FROM occurrence
              WHERE entity_id='e1' AND file_path='src/math.ts' AND start_byte=0
                AND end_byte=10 AND start_line=1 AND end_col=10 AND content_hash='h'"
        ),
        1
    );
}

/// A database written by the Slice 3 build must upgrade in place, keeping every row.
///
/// v2 databases exist in the wild, so this is not the same test as the v1 one with a different
/// starting point: it is the path most real upgrades will actually take.
#[test]
fn a_v2_database_upgrades_to_the_current_version_without_losing_rows() {
    let conn = v2_database_with_rows();
    assert_eq!(schema_version(&conn).unwrap(), Some(2));

    migrate(&conn).unwrap();

    assert_eq!(schema_version(&conn).unwrap(), Some(SCHEMA_VERSION));
    for (table, expected) in [
        ("repository", 1),
        ("repository_state", 1),
        ("entity", 2),
        ("assertion", 1),
        ("occurrence", 1),
        ("observation", 1),
        ("assertion_state", 1),
        ("extractor_run", 1),
        ("module_facts", 1),
    ] {
        assert_eq!(
            scalar(&conn, &format!("SELECT count(*) FROM {table}")),
            expected,
            "{table} lost or gained rows in the upgrade"
        );
    }
    assert!(!column_names(&conn, "occurrence").contains(&"state_id".to_string()));
    assert_eq!(
        conn.query_row("SELECT occurrence_id FROM occurrence", [], |row| row
            .get::<_, String>(0))
            .unwrap(),
        nerve_core::ids::occurrence_id("e1", "src/math.ts", 0, 10)
    );
    // The extraction cache is not evidence and must come through untouched.
    assert_eq!(
        conn.query_row("SELECT facts FROM module_facts", [], |row| row
            .get::<_, String>(0))
            .unwrap(),
        "{}"
    );
}

// ---- v4: filesystem evidence -----------------------------------------------------------------

/// **Slice 5d-i, from the oldest database Nerve can still read.**
///
/// A re-index would not fix these rows on its own: directory containment is re-derived every run,
/// but repository→file and directory→file rows are re-emitted only for files a run actually
/// re-extracts, so an unchanged file would keep an `AST_DIRECT` label indefinitely. The migration
/// is what makes the correction unconditional.
#[test]
fn a_v1_database_re_attributes_filesystem_containment() {
    let conn = database_with_filesystem_rows(1);
    migrate(&conn).unwrap();
    assert_v4_re_attributed(&conn);
}

/// The same, from the Slice 3 shape.
#[test]
fn a_v2_database_re_attributes_filesystem_containment() {
    let conn = database_with_filesystem_rows(2);
    migrate(&conn).unwrap();
    assert_v4_re_attributed(&conn);
}

/// The same, from the shape most databases in existence are actually at.
#[test]
fn a_v3_database_re_attributes_filesystem_containment() {
    let conn = database_with_filesystem_rows(3);
    assert_eq!(
        scalar(
            &conn,
            "SELECT count(*) FROM observation WHERE extractor_id = 'ts-js-structural'"
        ),
        3,
        "nothing to migrate; the test would prove nothing"
    );
    migrate(&conn).unwrap();
    assert_v4_re_attributed(&conn);
}

/// v4 selects on stored columns, never on the look of a path.
///
/// The two ways to get this wrong are both silent. A `LIKE '%.md'` would sweep up
/// `File CONTAINS Document`, which is a heading scan's claim. Rewriting every `CONTAINS` would
/// sweep up `Document CONTAINS Section`. Neither is filesystem structure, and after the migration
/// no query would say so.
#[test]
fn v4_leaves_every_claim_that_is_not_filesystem_structure_alone() {
    let conn = database_with_filesystem_rows(3);
    conn.execute_batch(
        "INSERT INTO entity VALUES ('sect1','r','section','Overview','docs/ROADMAP.md',NULL,NULL);
         INSERT INTO assertion VALUES ('c_sect','r','doc1','CONTAINS','sect1');
         INSERT INTO observation
             (assertion_id, extractor_run_id, evidence_source_type, directness,
              extractor_id, extractor_version, file_path, start_line, end_line,
              content_hash, created_at)
         VALUES ('c_sect',1,'DOCUMENT_STATED','DIRECT','md-structural','1.1.0',
                 'docs/ROADMAP.md',3,9,'h','t');",
    )
    .unwrap();

    migrate(&conn).unwrap();

    assert_eq!(
        scalar(
            &conn,
            "SELECT count(*) FROM observation
              WHERE extractor_id = 'fs-structural'
                AND assertion_id IN ('c_doc','c_sect','a1')"
        ),
        0,
        "v4 rewrote a claim that no directory walk produced"
    );
    assert_eq!(
        scalar(
            &conn,
            "SELECT count(*) FROM observation WHERE extractor_id = 'fs-structural'"
        ),
        3
    );
}

/// Migrating to v4 twice must change nothing the second time.
///
/// The step is not idempotent by accident — it is an `UPDATE` whose `WHERE` clause still matches
/// after it has run, so re-running it rewrites the same rows to the same values. Worth asserting,
/// because a future step written as a delete-and-reinsert would not have that property.
#[test]
fn re_running_v4_on_an_already_migrated_database_changes_nothing() {
    let conn = database_with_filesystem_rows(3);
    migrate(&conn).unwrap();
    let after_first = observation_labels(&conn);
    migrate(&conn).unwrap();
    assert_eq!(observation_labels(&conn), after_first);
    assert_eq!(scalar(&conn, "SELECT count(*) FROM observation"), 5);
}

// ---- v6: the historical model ------------------------------------------------------------------

/// The rows a v5 database holds, so that "nothing was lost" is checked rather than asserted.
const V5_ROW_COUNTS: [(&str, i64); 9] = [
    ("repository", 1),
    ("repository_state", 1),
    ("entity", 2),
    ("assertion", 1),
    ("occurrence", 1),
    ("observation", 1),
    ("assertion_state", 1),
    ("extractor_run", 1),
    ("module_facts", 1),
];

fn assert_row_counts(conn: &nerve_store::Connection, expected: &[(&str, i64)]) {
    for (table, count) in expected {
        assert_eq!(
            scalar(conn, &format!("SELECT count(*) FROM {table}")),
            *count,
            "{table} lost or gained rows"
        );
    }
}

/// **The upgrade path every existing database takes.** v5 is where Slice 10a left them.
#[test]
fn a_v5_database_upgrades_to_v6_and_the_history_tables_appear() {
    let conn = v5_database_with_rows();
    for table in V6_TABLES {
        assert!(
            !table_names(&conn).contains(&table.to_string()),
            "{table} exists before v6; the test would prove nothing"
        );
    }

    migrate(&conn).unwrap();

    assert_eq!(schema_version(&conn).unwrap(), Some(SCHEMA_VERSION));
    for table in V6_TABLES {
        assert!(
            table_names(&conn).contains(&table.to_string()),
            "v6 did not create {table}"
        );
        assert_eq!(scalar(&conn, &format!("SELECT count(*) FROM {table}")), 0);
    }
    // v6 adds tables and touches no user row, which is why it is not a destructive migration.
    assert_row_counts(&conn, &V5_ROW_COUNTS);
    // And v5's own column is still there with its value, not re-defaulted by the replay.
    assert_eq!(
        conn.query_row("SELECT framework_version FROM module_facts", [], |row| {
            row.get::<_, String>(0)
        })
        .unwrap(),
        "2.0.0"
    );
}

/// Every starting point Nerve can still read reaches v6, by three different routes.
///
/// One test rather than three, because the assertion is identical and the only thing that varies is
/// how the database got to where it started — the same shape `database_with_filesystem_rows` was
/// written for. The v4 correction is asserted on the way past, so this covers "every existing
/// migration still does its job with a sixth step appended" as well as "the sixth step runs".
#[test]
fn every_pre_v6_starting_version_reaches_v6_with_the_history_tables() {
    for version in [1, 2, 3] {
        let conn = database_with_filesystem_rows(version);
        for table in V6_TABLES {
            assert!(
                !table_names(&conn).contains(&table.to_string()),
                "{table} exists at v{version}; the test would prove nothing"
            );
        }

        migrate(&conn).unwrap();

        assert_eq!(
            schema_version(&conn).unwrap(),
            Some(SCHEMA_VERSION),
            "a v{version} database did not reach v{SCHEMA_VERSION}"
        );
        for table in V6_TABLES {
            assert!(
                table_names(&conn).contains(&table.to_string()),
                "v{version} → v6 did not create {table}"
            );
        }
        // The rest of the chain still runs: v3's identity restatement, v4's re-attribution, v5's
        // column. A sixth step must not have displaced any of them.
        assert_v4_re_attributed(&conn);
        assert!(column_names(&conn, "module_facts").contains(&"framework_version".to_string()));
        assert_eq!(
            scalar(&conn, "SELECT count(*) FROM schema_version"),
            SCHEMA_VERSION,
            "one row per applied step, whichever version the database started at"
        );
    }
}

/// Re-migrating a v6 database changes nothing and appends no version row.
///
/// Worth its own test beside the existing no-op ones because v6 is pure DDL with no `IF NOT
/// EXISTS`: if the version guard ever let step 6 replay, `CREATE TABLE git_commit` would be a hard
/// error rather than a silent no-op, and this is where that would surface.
#[test]
fn re_migrating_a_v6_database_changes_nothing_and_appends_no_version_row() {
    let conn = v5_database_with_rows();
    migrate(&conn).unwrap();
    let tables_before = table_names(&conn);
    let versions_before = scalar(&conn, "SELECT count(*) FROM schema_version");
    assert_eq!(versions_before, SCHEMA_VERSION);

    migrate(&conn).unwrap();
    migrate(&conn).unwrap();

    assert_eq!(table_names(&conn), tables_before);
    assert_eq!(
        scalar(&conn, "SELECT count(*) FROM schema_version"),
        versions_before,
        "re-migrating must not append a version row"
    );
    assert_eq!(schema_version(&conn).unwrap(), Some(SCHEMA_VERSION));
    assert_row_counts(&conn, &V5_ROW_COUNTS);
}

/// **A failing step commits nothing at all — not even the statements that succeeded before it.**
///
/// Each entry in `MIGRATIONS` runs inside its own transaction, and that is the property this test
/// exists for. The sabotage is deliberately placed on the migration's **last** statement:
/// `git_history_ingest` is created after `git_commit`, `git_change`, `git_rename_hypothesis` and
/// all four indexes, so without a transaction those seven objects would survive a failure and the
/// database would sit at v5 with three quarters of v6 already in it — a state no migration path
/// could ever repair, because step 6 would never run again to finish the job.
///
/// Sabotaging the *first* statement instead would pass whether or not the transaction existed,
/// which is why the version-row assertion alone is not enough.
#[test]
fn an_interrupted_v6_migration_commits_nothing() {
    let conn = v5_database_with_rows();
    // A table with v6's last name and nothing else in common, so `CREATE TABLE` must fail.
    conn.execute_batch("CREATE TABLE git_history_ingest (sabotage TEXT);")
        .unwrap();

    let err = migrate(&conn).unwrap_err();
    assert!(
        matches!(err, nerve_store::StoreError::Sqlite(_)),
        "expected the CREATE to fail, got {err}"
    );

    // The version row was never written, so the next run replays step 6 in full.
    assert_eq!(
        schema_version(&conn).unwrap(),
        Some(5),
        "a failed step must not record its version"
    );
    // And the statements that had already succeeded were rolled back with it.
    for table in ["git_commit", "git_change", "git_rename_hypothesis"] {
        assert!(
            !table_names(&conn).contains(&table.to_string()),
            "{table} survived a failed migration; step 6 is not transactional"
        );
    }
    for index in [
        "idx_git_commit_time",
        "idx_git_change_path",
        "idx_git_change_blob",
    ] {
        assert_eq!(
            scalar(
                &conn,
                &format!("SELECT count(*) FROM sqlite_master WHERE name = '{index}'")
            ),
            0,
            "{index} survived a failed migration"
        );
    }
    // Nothing else moved either, and the sabotaged table is exactly as it was.
    assert_row_counts(&conn, &V5_ROW_COUNTS);
    assert_eq!(
        column_names(&conn, "git_history_ingest"),
        vec!["sabotage".to_string()]
    );

    // The control: the identical fixture without the sabotage reaches v6. Without this the test
    // would pass just as well against a migration that could never succeed.
    let clean = v5_database_with_rows();
    migrate(&clean).unwrap();
    assert_eq!(schema_version(&clean).unwrap(), Some(SCHEMA_VERSION));
    for table in V6_TABLES {
        assert!(table_names(&clean).contains(&table.to_string()));
    }
}

/// The same, sabotaged on the migration's **first** statement.
///
/// The complementary half: a failure before anything has been created must also leave the version
/// at 5 rather than recording a step that did nothing. `IF NOT EXISTS` on `CREATE TABLE git_commit`
/// would turn this failure into silence, which is the double-apply
/// `nerve-index/tests/documents.rs` refuses to let a migration hide.
#[test]
fn a_v6_migration_that_collides_on_its_first_table_leaves_the_version_at_five() {
    let conn = v5_database_with_rows();
    conn.execute_batch("CREATE TABLE git_commit (sabotage TEXT);")
        .unwrap();

    let err = migrate(&conn).unwrap_err();
    assert!(
        matches!(err, nerve_store::StoreError::Sqlite(_)),
        "expected the CREATE to fail, got {err}"
    );

    assert_eq!(schema_version(&conn).unwrap(), Some(5));
    for table in ["git_change", "git_rename_hypothesis", "git_history_ingest"] {
        assert!(!table_names(&conn).contains(&table.to_string()));
    }
    assert_eq!(
        column_names(&conn, "git_commit"),
        vec!["sabotage".to_string()],
        "the pre-existing table was rewritten rather than left alone"
    );
    assert_row_counts(&conn, &V5_ROW_COUNTS);
}

/// A change row for a commit that was never recorded is **refused**, not orphaned.
///
/// `PRAGMA foreign_keys=ON` is set when the database is opened, so the composite foreign key onto
/// `git_commit` is genuinely enforced rather than decorative. This is the constraint that makes
/// `insert_changes`' plain `INSERT` meaningful: under `INSERT OR IGNORE` the same row would vanish
/// and the commit would read as one that did not touch the path.
#[test]
fn a_change_for_an_unrecorded_commit_is_refused() {
    let conn = open_in_memory().unwrap();
    migrate(&conn).unwrap();
    conn.execute("INSERT INTO repository VALUES ('r','p','/tmp','t')", [])
        .unwrap();
    let commit = "1".repeat(40);
    let absent = "2".repeat(40);

    conn.execute(COMMIT_INSERT, [&commit]).unwrap();

    // The control: the identical row against the recorded commit is accepted, so the refusal below
    // is the foreign key doing its job and not the statement being malformed.
    conn.execute(
        "INSERT INTO git_change VALUES ('r', ?1, 'src/a.ts', 'added', 'bb', NULL, 33188, NULL)",
        [&commit],
    )
    .unwrap();
    assert_eq!(scalar(&conn, "SELECT count(*) FROM git_change"), 1);

    let err = conn
        .execute(
            "INSERT INTO git_change VALUES ('r', ?1, 'src/a.ts', 'added', 'bb', NULL, 33188, NULL)",
            [&absent],
        )
        .unwrap_err();
    assert!(
        err.to_string().to_lowercase().contains("foreign key"),
        "expected a foreign-key refusal, got {err}"
    );
    assert_eq!(
        scalar(&conn, "SELECT count(*) FROM git_change"),
        1,
        "the refused row must not have landed"
    );
}

/// A rename hypothesis for a commit that was never recorded is refused too.
///
/// **The v6 DDL carries the same composite foreign key on `git_rename_hypothesis` as on
/// `git_change`**, and that is a decision this slice made rather than one the plan's §7.1 listing
/// stated. The reason is that a hypothesis is derived from one commit's tree diff, so a hypothesis
/// whose commit is unrecorded cannot be reported by any read — `renames_touching_path` joins
/// `git_commit` to order its results, so such a row would be silently invisible rather than merely
/// orphaned. A migration cannot be edited once shipped, so the choice had to be made now, and the
/// strictly stronger one costs nothing: every writer already has to record the commit first.
#[test]
fn a_rename_hypothesis_for_an_unrecorded_commit_is_refused() {
    let conn = open_in_memory().unwrap();
    migrate(&conn).unwrap();
    conn.execute("INSERT INTO repository VALUES ('r','p','/tmp','t')", [])
        .unwrap();
    let commit = "1".repeat(40);
    let absent = "2".repeat(40);
    let blob = "3".repeat(40);

    conn.execute(COMMIT_INSERT, [&commit]).unwrap();

    conn.execute(EXACT_RENAME_INSERT, [&commit, &blob]).unwrap();
    assert_eq!(
        scalar(&conn, "SELECT count(*) FROM git_rename_hypothesis"),
        1
    );

    let err = conn
        .execute(EXACT_RENAME_INSERT, [&absent, &blob])
        .unwrap_err();
    assert!(
        err.to_string().to_lowercase().contains("foreign key"),
        "expected a foreign-key refusal, got {err}"
    );
    assert_eq!(
        scalar(&conn, "SELECT count(*) FROM git_rename_hypothesis"),
        1,
        "the refused row must not have landed"
    );
}

// ---- v7: two blob oids, a named matcher, and what a summary is ---------------------------------

/// Build a database as the Slice 12b (v6) build would have left it, with history rows in it.
///
/// The rename hypotheses are the point. v7 rebuilds `git_rename_hypothesis` by create-copy-drop-
/// rename, and the only way to know the copy is lossless is to put rows in the old shape and count
/// them out of the new one.
fn v6_database_with_history() -> nerve_store::Connection {
    let conn = open_in_memory().unwrap();
    conn.execute_batch(V1_ONLY).unwrap();
    conn.execute_batch(V2_ONLY).unwrap();
    conn.execute_batch(SKELETON).unwrap();
    conn.execute_batch(&rows_at_state("s", "o1", 1, 1)).unwrap();
    conn.execute_batch(
        "INSERT INTO assertion_state VALUES ('a1','s','SUPPORTED','AST_DIRECT',1,1,0,'s');",
    )
    .unwrap();
    conn.execute_batch(V3_ONLY).unwrap();
    conn.execute_batch(V4_ONLY).unwrap();
    conn.execute_batch(V5_ONLY).unwrap();
    conn.execute_batch(
        "INSERT INTO module_facts
             VALUES ('r','src/math.ts','h','typescript','1.1.0','1.0.0','{}','2.0.0');",
    )
    .unwrap();
    conn.execute_batch(V6_ONLY).unwrap();

    // Two commits, four changes, three hypotheses — one unambiguous and two pairings of one
    // ambiguous match, so the copy is checked against a shape where a "keep one row per blob"
    // mistake would show up as a missing row rather than as a passing test.
    let c1 = "1".repeat(40);
    let c2 = "2".repeat(40);
    let b1 = "a".repeat(40);
    let b2 = "b".repeat(40);
    conn.execute_batch(&format!(
        "INSERT INTO git_commit VALUES
             ('r','{c1}','aa','[]','root','enumerated',100,'+0000',100,'+0000',
              NULL,NULL,'first',0);
         INSERT INTO git_commit VALUES
             ('r','{c2}','bb','[\"{c1}\"]','parents_available','enumerated',
              200,'+0000',200,'+0000',NULL,NULL,'second',0);
         INSERT INTO git_change VALUES ('r','{c1}','old.ts','deleted',NULL,'{b1}',NULL,33188);
         INSERT INTO git_change VALUES ('r','{c1}','new.ts','added','{b1}',NULL,33188,NULL);
         INSERT INTO git_change VALUES ('r','{c2}','gone.ts','deleted',NULL,'{b2}',NULL,33188);
         INSERT INTO git_change VALUES ('r','{c2}','one.ts','added','{b2}',NULL,33188,NULL);
         INSERT INTO git_rename_hypothesis
             VALUES ('r','{c1}','old.ts','new.ts','exact_content','{b1}','unique');
         INSERT INTO git_rename_hypothesis
             VALUES ('r','{c2}','gone.ts','one.ts','exact_content','{b2}','many_to');
         INSERT INTO git_rename_hypothesis
             VALUES ('r','{c2}','gone.ts','two.ts','exact_content','{b2}','many_to');
         INSERT INTO git_history_ingest
             VALUES ('r','{c2}','[\"{c2}\"]',2,5000,'exhausted',0,'[]',0,'{{}}','gitobj-1.0.0','t');"
    ))
    .unwrap();
    assert_eq!(schema_version(&conn).unwrap(), Some(6));
    conn
}

/// The three hypotheses, as `from_path -> to_path : evidence / from_blob / to_blob / matcher`.
fn rename_labels(conn: &nerve_store::Connection) -> Vec<String> {
    let mut stmt = conn
        .prepare(
            "SELECT from_path || ' -> ' || to_path || ' : ' || evidence || ' / '
                 || from_blob_oid || ' / ' || to_blob_oid || ' / '
                 || matcher_id || '@' || matcher_version || ' / ' || ambiguity
                 || ' / ' || coalesce(match_numerator, 'none')
                 || ':' || coalesce(match_denominator, 'none')
               FROM git_rename_hypothesis ORDER BY 1",
        )
        .unwrap();
    let rows = stmt.query_map([], |row| row.get::<_, String>(0)).unwrap();
    rows.map(|row| row.unwrap()).collect()
}

/// **The upgrade path a database with history actually takes, and it must lose nothing.**
///
/// Three assertions carry the slice: every hypothesis row survives, each with
/// `from_blob_oid = to_blob_oid = ` its old `blob_oid`; every commit gets `summary_truncation`
/// with `'unknown'`, which is the only honest value for a row written before the column existed;
/// and the changes and the ingest record are untouched.
#[test]
fn a_v6_database_upgrades_to_v7_and_every_rename_row_survives() {
    let conn = v6_database_with_history();
    assert!(
        column_names(&conn, "git_rename_hypothesis").contains(&"blob_oid".to_string()),
        "the fixture is not at v6; the test would prove nothing"
    );
    assert!(!column_names(&conn, "git_commit").contains(&"summary_truncation".to_string()));
    assert!(!table_names(&conn).contains(&V7_TABLE.to_string()));
    let renames_before = scalar(&conn, "SELECT count(*) FROM git_rename_hypothesis");
    assert_eq!(renames_before, 3, "three rows, or the count proves nothing");

    migrate(&conn).unwrap();

    assert_eq!(schema_version(&conn).unwrap(), Some(SCHEMA_VERSION));
    assert_eq!(
        scalar(&conn, "SELECT count(*) FROM git_rename_hypothesis"),
        renames_before,
        "the rebuild dropped a rename hypothesis"
    );

    // Every row, in full. The old `blob_oid` is on both sides, the copied rows name the exact
    // matcher, and neither measurement column was invented for them.
    let b1 = "a".repeat(40);
    let b2 = "b".repeat(40);
    assert_eq!(
        rename_labels(&conn),
        vec![
            format!("gone.ts -> one.ts : exact_content / {b2} / {b2} / git-blob-oid@1 / many_to / none:none"),
            format!("gone.ts -> two.ts : exact_content / {b2} / {b2} / git-blob-oid@1 / many_to / none:none"),
            format!("old.ts -> new.ts : exact_content / {b1} / {b1} / git-blob-oid@1 / unique / none:none"),
        ]
    );
    assert!(
        !column_names(&conn, "git_rename_hypothesis").contains(&"blob_oid".to_string()),
        "the v6 column survived the rebuild, so the table was altered rather than replaced"
    );
    // The rebuilt table is the real one, not a leftover working copy under its build name.
    assert!(!table_names(&conn).contains(&"git_rename_hypothesis_v7".to_string()));

    // Every pre-existing commit says `unknown`, which is the fact that cannot be recovered — not
    // `complete`, which would be a claim, and not `truncated`, which would be a different one.
    assert_eq!(scalar(&conn, "SELECT count(*) FROM git_commit"), 2);
    assert_eq!(
        scalar(
            &conn,
            "SELECT count(*) FROM git_commit WHERE summary_truncation = 'unknown'"
        ),
        2,
        "a v6 commit cannot be backfilled, so every one of them must say so"
    );

    // Nothing else in the history moved, and the rest of the database is where v5 left it.
    assert_eq!(scalar(&conn, "SELECT count(*) FROM git_change"), 4);
    assert_eq!(scalar(&conn, "SELECT count(*) FROM git_history_ingest"), 1);
    assert_eq!(
        scalar(&conn, &format!("SELECT count(*) FROM {V7_TABLE}")),
        0
    );
    assert_row_counts(&conn, &V5_ROW_COUNTS);
    assert_eq!(
        conn.query_row("SELECT framework_version FROM module_facts", [], |row| {
            row.get::<_, String>(0)
        })
        .unwrap(),
        "2.0.0"
    );
}

/// The oldest database Nerve can still read reaches v7, end to end.
///
/// v1 is the interesting starting point rather than a redundant one: it is the only route on which
/// v3's identity restatement, v4's re-attribution, v5's column, v6's tables and v7's rebuild all
/// run in one call, so a step that displaced an earlier one shows up here and nowhere else.
#[test]
fn a_v1_database_reaches_v7_with_every_earlier_step_still_done() {
    let conn = database_with_filesystem_rows(1);
    assert!(!table_names(&conn).contains(&"git_rename_hypothesis".to_string()));

    migrate(&conn).unwrap();

    assert_eq!(schema_version(&conn).unwrap(), Some(SCHEMA_VERSION));
    assert_eq!(
        scalar(&conn, "SELECT count(*) FROM schema_version"),
        SCHEMA_VERSION,
        "one row per applied step"
    );
    assert_v4_re_attributed(&conn);
    assert!(column_names(&conn, "module_facts").contains(&"framework_version".to_string()));
    for table in V6_TABLES {
        assert!(table_names(&conn).contains(&table.to_string()));
    }
    assert!(table_names(&conn).contains(&V7_TABLE.to_string()));
    assert!(column_names(&conn, "git_commit").contains(&"summary_truncation".to_string()));
    assert!(column_names(&conn, "git_rename_hypothesis").contains(&"to_blob_oid".to_string()));
}

/// Re-migrating a v7 database changes nothing: no version row, no table, no row.
///
/// v7 is where this matters most. Its rebuild is a `DROP TABLE` followed by a rename, so a version
/// guard that ever let step 7 replay would not fail loudly — it would drop the rebuilt table and
/// copy from a `git_rename_hypothesis` that no longer has a `blob_oid` column. Counting rows before
/// and after is what would catch it.
#[test]
fn re_migrating_a_v7_database_changes_nothing_and_appends_no_version_row() {
    let conn = v6_database_with_history();
    migrate(&conn).unwrap();
    let tables_before = table_names(&conn);
    let renames_before = rename_labels(&conn);
    let versions_before = scalar(&conn, "SELECT count(*) FROM schema_version");
    assert_eq!(versions_before, SCHEMA_VERSION);
    assert_eq!(renames_before.len(), 3);

    migrate(&conn).unwrap();
    migrate(&conn).unwrap();

    assert_eq!(table_names(&conn), tables_before);
    assert_eq!(rename_labels(&conn), renames_before, "the rebuild replayed");
    assert_eq!(
        scalar(&conn, "SELECT count(*) FROM schema_version"),
        versions_before,
        "re-migrating must not append a version row"
    );
    assert_eq!(schema_version(&conn).unwrap(), Some(SCHEMA_VERSION));
    assert_eq!(scalar(&conn, "SELECT count(*) FROM git_commit"), 2);
    assert_eq!(scalar(&conn, "SELECT count(*) FROM git_change"), 4);
    assert_row_counts(&conn, &V5_ROW_COUNTS);
}

/// **A failing v7 step commits nothing — including the `ALTER TABLE` that ran first.**
///
/// The sabotage is on the migration's **last** statement, `CREATE TABLE git_rename_analysis`, which
/// runs after the `ALTER TABLE`, after the rebuild's create-copy, after the `DROP TABLE` and after
/// the rename. Without a transaction the database would sit at v6 with `git_commit` already carrying
/// `summary_truncation` and `git_rename_hypothesis` already rebuilt — a state no migration path
/// could repair, because step 7 would never run again to finish the job and, worse, replaying it
/// would copy from a table whose `blob_oid` column had already gone.
///
/// The three rename rows are what make this a measurement rather than a demonstration: they have to
/// come back readable from the *v6* table, which only exists if the drop was rolled back too.
#[test]
fn an_interrupted_v7_migration_commits_nothing() {
    let conn = v6_database_with_history();
    conn.execute_batch("CREATE TABLE git_rename_analysis (sabotage TEXT);")
        .unwrap();

    let err = migrate(&conn).unwrap_err();
    assert!(
        matches!(err, nerve_store::StoreError::Sqlite(_)),
        "expected the CREATE to fail, got {err}"
    );

    assert_eq!(
        schema_version(&conn).unwrap(),
        Some(6),
        "a failed step must not record its version"
    );
    // The `ALTER TABLE` went back with it.
    assert!(
        !column_names(&conn, "git_commit").contains(&"summary_truncation".to_string()),
        "the v7 column survived a failed migration; step 7 is not transactional"
    );
    // So did the whole rebuild: the v6 table is back, with its single blob column and all three rows.
    assert!(
        column_names(&conn, "git_rename_hypothesis").contains(&"blob_oid".to_string()),
        "the rebuilt table survived a failed migration"
    );
    assert!(!column_names(&conn, "git_rename_hypothesis").contains(&"to_blob_oid".to_string()));
    assert!(!table_names(&conn).contains(&"git_rename_hypothesis_v7".to_string()));
    assert_eq!(
        scalar(&conn, "SELECT count(*) FROM git_rename_hypothesis"),
        3,
        "the drop was not rolled back; the hypotheses are gone"
    );
    assert_eq!(scalar(&conn, "SELECT count(*) FROM git_commit"), 2);
    assert_eq!(scalar(&conn, "SELECT count(*) FROM git_change"), 4);
    assert_eq!(
        column_names(&conn, "git_rename_analysis"),
        vec!["sabotage".to_string()],
        "the pre-existing table was rewritten rather than left alone"
    );
    assert_row_counts(&conn, &V5_ROW_COUNTS);

    // The control: the identical fixture without the sabotage reaches v7. Without this the test
    // would pass just as well against a migration that could never succeed.
    let clean = v6_database_with_history();
    migrate(&clean).unwrap();
    assert_eq!(schema_version(&clean).unwrap(), Some(SCHEMA_VERSION));
    assert_eq!(rename_labels(&clean).len(), 3);
}

/// **The `CHECK` is where "evidence is never blended" stops being a convention.**
///
/// Four refusals, each the shape a future writer would actually produce, and each with a control
/// that lands so the refusal is the constraint doing its job rather than a malformed statement:
///
/// - an exact-content row carrying a measurement — a score attached to a match that computed none;
/// - a similar-content row without one — a similarity claim that never says what was counted;
/// - a similar-content row whose two blob oids are equal — the exact matcher's row wearing the
///   other evidence label, which is the blend §6's first constraint forbids;
/// - `match_numerator > match_denominator` — a ratio above one, which is not a measurement.
#[test]
fn the_rename_check_refuses_every_blend_of_the_two_evidence_kinds() {
    let conn = open_in_memory().unwrap();
    migrate(&conn).unwrap();
    conn.execute("INSERT INTO repository VALUES ('r','p','/tmp','t')", [])
        .unwrap();
    let commit = "1".repeat(40);
    let from_blob = "a".repeat(40);
    let to_blob = "b".repeat(40);
    conn.execute(COMMIT_INSERT, [&commit]).unwrap();

    // The controls. One of each evidence kind, well formed, so every refusal below is about the
    // combination and not about the statement.
    conn.execute(EXACT_RENAME_INSERT, [&commit, &from_blob])
        .unwrap();
    let insert = "INSERT INTO git_rename_hypothesis
         (repo_id, commit_oid, from_path, to_path, evidence, from_blob_oid, to_blob_oid,
          matcher_id, matcher_version, match_numerator, match_denominator, ambiguity)
     VALUES ('r', ?1, ?2, ?3, ?4, ?5, ?6, 'nerve-line-multiset', '1', ?7, ?8, 'unique')";
    conn.execute(
        insert,
        rusqlite::params![
            commit,
            "moved.ts",
            "moved-to.ts",
            "similar_content",
            from_blob,
            to_blob,
            1_320,
            1_500
        ],
    )
    .unwrap();
    assert_eq!(
        scalar(&conn, "SELECT count(*) FROM git_rename_hypothesis"),
        2,
        "both well-formed shapes must land, or the refusals prove nothing"
    );

    // 1. An exact match given a score.
    let err = conn
        .execute(
            insert,
            rusqlite::params![
                commit,
                "a.ts",
                "b.ts",
                "exact_content",
                from_blob,
                from_blob,
                1,
                1
            ],
        )
        .unwrap_err();
    assert!(
        err.to_string().to_lowercase().contains("constraint"),
        "an exact-content row carried a measurement: {err}"
    );

    // 2. A similarity claim that says nothing about what was counted.
    let err = conn
        .execute(
            insert,
            rusqlite::params![
                commit,
                "c.ts",
                "d.ts",
                "similar_content",
                from_blob,
                to_blob,
                None::<i64>,
                None::<i64>
            ],
        )
        .unwrap_err();
    assert!(
        err.to_string().to_lowercase().contains("constraint"),
        "a similar-content row omitted its measurement: {err}"
    );

    // 3. The exact matcher's row wearing the other label.
    let err = conn
        .execute(
            insert,
            rusqlite::params![
                commit,
                "e.ts",
                "f.ts",
                "similar_content",
                from_blob,
                from_blob,
                9,
                10
            ],
        )
        .unwrap_err();
    assert!(
        err.to_string().to_lowercase().contains("constraint"),
        "a similar-content row named one blob twice: {err}"
    );

    // 4. A ratio above one.
    let err = conn
        .execute(
            insert,
            rusqlite::params![
                commit,
                "g.ts",
                "h.ts",
                "similar_content",
                from_blob,
                to_blob,
                11,
                10
            ],
        )
        .unwrap_err();
    assert!(
        err.to_string().to_lowercase().contains("constraint"),
        "a measurement above one was accepted: {err}"
    );

    assert_eq!(
        scalar(&conn, "SELECT count(*) FROM git_rename_hypothesis"),
        2,
        "a refused row landed anyway"
    );
}

/// ADR-0006 consequence 3, made explicit rather than left to be discovered.
///
/// An insert-only v1 database holds one occurrence and one observation *per repository state*.
/// Under the new identity those are the same row, so the migration collapses them and keeps the
/// most recently written copy. This is the one place rows are deliberately not preserved.
#[test]
fn a_v1_database_indexed_twice_collapses_its_per_state_duplicates() {
    let conn = v1_database_indexed_twice();
    assert_eq!(scalar(&conn, "SELECT count(*) FROM occurrence"), 2);
    assert_eq!(scalar(&conn, "SELECT count(*) FROM observation"), 2);

    migrate(&conn).unwrap();

    assert_eq!(
        scalar(&conn, "SELECT count(*) FROM occurrence"),
        1,
        "two rows for one location must collapse onto one"
    );
    assert_eq!(scalar(&conn, "SELECT count(*) FROM observation"), 1);
    // The survivor is the most recently written one, and it carries the new identity.
    assert_eq!(
        scalar(&conn, "SELECT observation_id FROM observation"),
        2,
        "the superseded copy must be the one discarded"
    );
    assert_eq!(
        conn.query_row("SELECT occurrence_id FROM occurrence", [], |row| row
            .get::<_, String>(0))
            .unwrap(),
        nerve_core::ids::occurrence_id("e1", "src/math.ts", 0, 10)
    );
    // Both states remain in the log; only the denormalized copies on the rows went away.
    assert_eq!(scalar(&conn, "SELECT count(*) FROM repository_state"), 2);
    assert_eq!(scalar(&conn, "SELECT count(*) FROM extractor_run"), 2);
}

/// The v3 uniqueness rule must actually be enforced, not merely declared.
#[test]
fn the_same_evidence_cannot_be_recorded_twice_across_states() {
    let conn = open_in_memory().unwrap();
    migrate(&conn).unwrap();
    conn.execute_batch(
        "INSERT INTO repository VALUES ('r','p','/tmp','t');
         INSERT INTO repository_state VALUES ('s1','r','content',NULL,'m1','t');
         INSERT INTO repository_state VALUES ('s2','r','content',NULL,'m2','t');
         INSERT INTO extractor_run VALUES (1,'r','s1','x','1','t','t',1,0,'complete');
         INSERT INTO extractor_run VALUES (2,'r','s2','x','1','t','t',1,0,'complete');
         INSERT INTO entity VALUES ('e1','r','function','add','',NULL,NULL);
         INSERT INTO assertion VALUES ('a1','r','e1','CALLS','e1');",
    )
    .unwrap();
    let insert = || {
        conn.execute(
            "INSERT OR IGNORE INTO observation
                 (assertion_id, extractor_run_id, evidence_source_type, directness,
                  extractor_id, extractor_version, file_path, start_line, end_line,
                  content_hash, created_at)
             VALUES ('a1', 1, 'AST_DIRECT', 'DIRECT', 'x', '1', 'a.ts', 1, 1, 'h', 't')",
            [],
        )
        .unwrap()
    };
    assert_eq!(insert(), 1);
    assert_eq!(
        insert(),
        0,
        "the same evidence from the same extractor at the same place is one row"
    );
}

/// Upgrading, then re-running, must be a no-op — including on a database that started at v1.
#[test]
fn re_migrating_an_upgraded_database_changes_nothing() {
    let conn = v1_database_with_rows();
    migrate(&conn).unwrap();
    let before = table_names(&conn);
    let versions_before: i64 = conn
        .query_row("SELECT count(*) FROM schema_version", [], |row| row.get(0))
        .unwrap();

    migrate(&conn).unwrap();
    migrate(&conn).unwrap();

    assert_eq!(table_names(&conn), before);
    let versions_after: i64 = conn
        .query_row("SELECT count(*) FROM schema_version", [], |row| row.get(0))
        .unwrap();
    assert_eq!(
        versions_after, versions_before,
        "re-migrating must not append a version row"
    );
    assert_eq!(schema_version(&conn).unwrap(), Some(SCHEMA_VERSION));
}

#[test]
fn migrating_twice_is_a_no_op() {
    let conn = open_in_memory().unwrap();
    migrate(&conn).unwrap();
    let first = table_names(&conn);
    let rows_before: i64 = conn
        .query_row("SELECT count(*) FROM schema_version", [], |row| row.get(0))
        .unwrap();
    assert_eq!(
        rows_before, SCHEMA_VERSION,
        "one row per applied migration step"
    );

    migrate(&conn).unwrap();
    migrate(&conn).unwrap();

    let rows: i64 = conn
        .query_row("SELECT count(*) FROM schema_version", [], |row| row.get(0))
        .unwrap();
    assert_eq!(
        rows, rows_before,
        "re-migrating must not append a version row"
    );
    assert_eq!(schema_version(&conn).unwrap(), Some(SCHEMA_VERSION));
    assert_eq!(table_names(&conn), first, "schema must be unchanged");
}

/// An identity link is a proposal about one pair; proposing it twice is the same proposal.
#[test]
fn identity_links_are_unique_per_pair_and_kind() {
    let conn = open_in_memory().unwrap();
    migrate(&conn).unwrap();
    conn.execute("INSERT INTO repository VALUES ('r','p','/tmp','t')", [])
        .unwrap();

    for _ in 0..3 {
        nerve_store::insert_identity_link(&conn, "r", "old", "new", "moved_symbol", "{}").unwrap();
    }
    // A different kind of claim about the same pair is a different proposal.
    nerve_store::insert_identity_link(&conn, "r", "old", "new", "moved_file", "{}").unwrap();

    let count: i64 = conn
        .query_row("SELECT count(*) FROM identity_link", [], |row| row.get(0))
        .unwrap();
    assert_eq!(count, 2);
}

#[test]
fn reopening_a_migrated_database_is_a_no_op() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("nerve.db");
    {
        let conn = open(&path).unwrap();
        migrate(&conn).unwrap();
    }
    let conn = open(&path).unwrap();
    migrate(&conn).unwrap();
    assert_eq!(schema_version(&conn).unwrap(), Some(SCHEMA_VERSION));
}

#[test]
fn a_newer_schema_is_refused_rather_than_guessed_at() {
    let conn = open_in_memory().unwrap();
    migrate(&conn).unwrap();
    conn.execute(
        "INSERT INTO schema_version (version, applied_at, description)
         VALUES (999, 'later', 'from the future')",
        [],
    )
    .unwrap();
    let err = migrate(&conn).unwrap_err();
    assert!(
        matches!(
            err,
            nerve_store::StoreError::SchemaTooNew { found: 999, .. }
        ),
        "{err}"
    );
}

#[test]
fn every_adr_0003_table_exists() {
    let conn = open_in_memory().unwrap();
    migrate(&conn).unwrap();
    for table in [
        "schema_version",
        "repository",
        "repository_state",
        "entity",
        "occurrence",
        "assertion",
        "observation",
        "assertion_state",
        "extractor_run",
        "identity_link",
        "entity_fts",
        "module_facts",
    ] {
        let count: i64 = conn
            .query_row(
                "SELECT count(*) FROM sqlite_master WHERE name = ?1",
                [table],
                |row| row.get(0),
            )
            .unwrap();
        assert!(count > 0, "table {table} is missing");
    }
}

#[test]
fn identity_link_exists_and_is_unused() {
    let conn = open_in_memory().unwrap();
    migrate(&conn).unwrap();
    let rows: i64 = conn
        .query_row("SELECT count(*) FROM identity_link", [], |row| row.get(0))
        .unwrap();
    assert_eq!(
        rows, 0,
        "identity_link is created for Slice 3, not used now"
    );
}

/// ADR-0001 requires this to be proven rather than assumed: FTS5 is a compile-time option and
/// the bundled build is what makes it available regardless of the host's SQLite.
#[test]
fn fts5_is_available_in_the_bundled_build() {
    let conn = open_in_memory().unwrap();
    conn.execute_batch("CREATE VIRTUAL TABLE probe USING fts5(body);")
        .expect("CREATE VIRTUAL TABLE ... USING fts5 must succeed");
    conn.execute("INSERT INTO probe(body) VALUES ('hello nerve')", [])
        .unwrap();
    let hits: i64 = conn
        .query_row(
            "SELECT count(*) FROM probe WHERE probe MATCH 'nerve'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(hits, 1);
}

#[test]
fn entity_fts_stays_in_sync_with_entity() {
    let conn = open_in_memory().unwrap();
    migrate(&conn).unwrap();
    conn.execute(
        "INSERT INTO repository (repo_id, project_id, root_path, created_at)
         VALUES ('r', 'p', '/tmp', 'now')",
        [],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO entity (entity_id, repo_id, kind, name, scope_path)
         VALUES ('e1', 'r', 'function', 'computeArea', 'Shapes')",
        [],
    )
    .unwrap();

    let hits: i64 = conn
        .query_row(
            "SELECT count(*) FROM entity_fts WHERE entity_fts MATCH '\"computeArea\"'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(hits, 1);

    conn.execute("DELETE FROM entity WHERE entity_id = 'e1'", [])
        .unwrap();
    let hits: i64 = conn
        .query_row(
            "SELECT count(*) FROM entity_fts WHERE entity_fts MATCH '\"computeArea\"'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(hits, 0, "deleting an entity must remove its FTS row");
}

#[test]
fn write_module_contains_no_statement_touching_assertion_state() {
    // A grep-style guard on the source of the only write path. `assertion_state` must be
    // derived, and the compiler cannot express "this module may not name that table".
    // Doc comments are allowed to mention it — they are what explain the boundary.
    let offenders: Vec<(usize, &str)> = include_str!("../src/write.rs")
        .lines()
        .enumerate()
        .filter(|(_, line)| !line.trim_start().starts_with("//"))
        .filter(|(_, line)| line.contains("assertion_state"))
        .map(|(index, line)| (index + 1, line))
        .collect();
    assert!(
        offenders.is_empty(),
        "nerve_store::write must never reference assertion_state in code: {offenders:?}"
    );
}
