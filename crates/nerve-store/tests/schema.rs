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

/// The two tables schema v8 adds, in the order the migration creates them.
const V8_TABLES: [&str; 2] = ["repo_registry", "contract_link"];

/// The three tables schema v9 adds, in the order the migration creates them.
const V9_TABLES: [&str; 3] = ["memory", "memory_citation", "memory_event"];

/// What v7 added on top of v6, exactly as the Slice 12c-ii build shipped it.
///
/// Written out rather than reached by calling `migrate` part of the way, for the same reason
/// [`V1_ONLY`] and [`V6_ONLY`] are: "a v7 database" has to mean what v7 actually left behind, which
/// is the starting point every real database takes to v8.
const V7_ONLY: &str = r#"
ALTER TABLE git_commit ADD COLUMN summary_truncation TEXT NOT NULL DEFAULT 'unknown';
CREATE TABLE git_rename_hypothesis_v7 (
    repo_id           TEXT    NOT NULL REFERENCES repository(repo_id),
    commit_oid        TEXT    NOT NULL,
    from_path         TEXT    NOT NULL,
    to_path           TEXT    NOT NULL,
    evidence          TEXT    NOT NULL,
    from_blob_oid     TEXT    NOT NULL,
    to_blob_oid       TEXT    NOT NULL,
    matcher_id        TEXT    NOT NULL,
    matcher_version   TEXT    NOT NULL,
    match_numerator   INTEGER,
    match_denominator INTEGER,
    ambiguity         TEXT    NOT NULL,
    PRIMARY KEY (repo_id, commit_oid, from_path, to_path),
    FOREIGN KEY (repo_id, commit_oid) REFERENCES git_commit(repo_id, commit_oid),
    CHECK (
        (evidence = 'exact_content'
            AND from_blob_oid = to_blob_oid
            AND match_numerator IS NULL AND match_denominator IS NULL)
     OR (evidence = 'similar_content'
            AND from_blob_oid <> to_blob_oid
            AND match_numerator IS NOT NULL AND match_denominator IS NOT NULL
            AND match_denominator > 0
            AND match_numerator >= 0
            AND match_numerator <= match_denominator)
    )
);
INSERT INTO git_rename_hypothesis_v7
    (repo_id, commit_oid, from_path, to_path, evidence,
     from_blob_oid, to_blob_oid, matcher_id, matcher_version,
     match_numerator, match_denominator, ambiguity)
SELECT repo_id, commit_oid, from_path, to_path, evidence,
       blob_oid, blob_oid, 'git-blob-oid', '1',
       NULL, NULL, ambiguity
  FROM git_rename_hypothesis;
DROP TABLE git_rename_hypothesis;
ALTER TABLE git_rename_hypothesis_v7 RENAME TO git_rename_hypothesis;
CREATE TABLE git_rename_analysis (
    repo_id               TEXT    NOT NULL REFERENCES repository(repo_id),
    commit_oid            TEXT    NOT NULL,
    matcher_id            TEXT    NOT NULL,
    matcher_version       TEXT    NOT NULL,
    threshold_numerator   INTEGER NOT NULL,
    threshold_denominator INTEGER NOT NULL,
    deletions_considered  INTEGER NOT NULL,
    additions_considered  INTEGER NOT NULL,
    pairs_considered      INTEGER NOT NULL,
    pairs_measured        INTEGER NOT NULL,
    completeness          TEXT    NOT NULL,
    unmeasured            TEXT    NOT NULL,
    PRIMARY KEY (repo_id, commit_oid, matcher_id),
    FOREIGN KEY (repo_id, commit_oid) REFERENCES git_commit(repo_id, commit_oid),
    CHECK (threshold_denominator > 0 AND pairs_measured <= pairs_considered)
);
INSERT INTO schema_version (version, applied_at, description)
    VALUES (7, '2026-01-01T00:00:00.000Z', 'Slice 12c-ii');
"#;

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
    assert_eq!(SCHEMA_VERSION, 10);
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
    // v8's two tables, present and empty. Empty is the correct state: a registry is created by an
    // explicit command and never by discovery, so a fresh database knowing about a neighbour would
    // be the auto-registration row 13 refuses.
    for table in V8_TABLES {
        assert!(
            table_names(&conn).contains(&table.to_string()),
            "v8 table {table} is missing from a fresh database"
        );
        assert_eq!(scalar(&conn, &format!("SELECT count(*) FROM {table}")), 0);
    }
    // v9's three tables, present and empty. Empty is the only correct state: a memory record is
    // something a human wrote, so a fresh database holding one would be a note nobody made.
    for table in V9_TABLES {
        assert!(
            table_names(&conn).contains(&table.to_string()),
            "v9 table {table} is missing from a fresh database"
        );
        assert_eq!(scalar(&conn, &format!("SELECT count(*) FROM {table}")), 0);
    }
    // v10 closes two columns that v9 left open, and the tables it rebuilt are still the same three.
    // A fresh database gets them straight from the replayed step, so this is where a rebuild that
    // dropped an index or a constraint on the way through would show up.
    for value in ["implementation", "interface", "operations", "process"] {
        assert!(
            value.parse::<nerve_core::vocab::MemoryScope>().is_ok(),
            "the v10 CHECK and MemoryScope have drifted: `{value}`"
        );
    }
    assert_eq!(nerve_core::vocab::MemoryScope::ALL.len(), 4);
    assert_eq!(nerve_core::vocab::MemoryOperation::ALL.len(), 5);
    for index in [
        "idx_memory_subject",
        "idx_memory_scope",
        "idx_memory_claim",
        "idx_memory_supersedes",
        "idx_memory_citation_memory",
        "idx_memory_citation_path",
        "idx_memory_event_memory",
    ] {
        assert_eq!(
            scalar(
                &conn,
                &format!("SELECT count(*) FROM sqlite_master WHERE name = '{index}'")
            ),
            1,
            "{index} did not survive the v10 rebuild"
        );
    }

    // The subject is a snapshot and nothing else. A column named `subject_entity_id` would be the
    // foreign key row 14 was rewritten to remove, so its absence is asserted rather than assumed.
    assert!(column_names(&conn, "memory").contains(&"subject_entity_id_snapshot".to_string()));
    assert!(
        !column_names(&conn, "memory").contains(&"subject_entity_id".to_string()),
        "memory holds a live subject pointer; entity rows are pruned on re-index"
    );
    // Supersession has exactly one writable direction.
    assert!(column_names(&conn, "memory").contains(&"supersedes_memory_id".to_string()));
    assert!(
        !column_names(&conn, "memory").contains(&"superseded_by".to_string()),
        "memory stores both directions of supersession; the two can disagree"
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

/// The oldest database Nerve can still read reaches the current version, end to end.
///
/// v1 is the interesting starting point rather than a redundant one: it is the only route on which
/// v3's identity restatement, v4's re-attribution, v5's column, v6's tables, v7's rebuild, v8's
/// registry and v9's memory all run in one call, so a step that displaced an earlier one shows up
/// here and nowhere else.
#[test]
fn a_v1_database_reaches_the_current_version_with_every_earlier_step_still_done() {
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
    for table in V8_TABLES {
        assert!(
            table_names(&conn).contains(&table.to_string()),
            "v1 → v8 did not create {table}"
        );
        assert_eq!(scalar(&conn, &format!("SELECT count(*) FROM {table}")), 0);
    }
    for table in V9_TABLES {
        assert!(
            table_names(&conn).contains(&table.to_string()),
            "v1 → v9 did not create {table}"
        );
        assert_eq!(scalar(&conn, &format!("SELECT count(*) FROM {table}")), 0);
    }
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

// ---- v8: the cross-repository registry and its contract links ----------------------------------

/// A registry entry, named by column. The insert every v8 test below builds on.
const REGISTRY_INSERT: &str = "INSERT INTO repo_registry
     (repo_id, registry_id, expected_repository_id, display_name, local_path, added_at,
      last_seen_state, last_seen_at, availability_checked_at, status, withdrawn_at)
 VALUES ('r', ?1, ?2, 'Neighbour', '/somewhere/else', '2026-01-01T00:00:00.000Z',
         NULL, NULL, NULL, ?3, ?4)";

/// A contract link naming a target that exists in no local table, which is the whole point.
///
/// Every column is named. `contract_link` is the widest table in the schema and a positional
/// `VALUES` would be unreadable and one column-order change away from writing a path into a status.
const LINK_INSERT: &str = "INSERT INTO contract_link
     (repo_id, source_repository_id, source_state_at_resolution, source_entity_id,
      source_kind_snapshot, source_path, source_span, registry_entry_id,
      expected_target_repository_id, target_state_at_resolution, target_entity_id,
      target_kind_snapshot, target_name_snapshot, target_path_snapshot, target_span_snapshot,
      relation_semantics, contract_kind, contract_identity, expected_contract_version,
      observed_contract_version, resolution_method, extractor_id, extractor_version,
      evidence_details, ambiguity, unsupported_reason, first_seen_at, last_seen_at,
      withdrawn_at, status)
 VALUES ('r', 'r', 's', ?1, ?2, 'package.json', '3:3', ?3, ?4,
         ?5, ?6, ?7, ?8, ?9, ?10,
         'REFERENCES', 'npm_package_export', 'pkg-b/sub', '^1.0.0', '1.2.0',
         ?11, 'ts-contract', '1.0.0', NULL, NULL, ?12,
         '2026-01-01T00:00:00.000Z', ?13, ?14, ?15)";

/// A v8 database with one registry entry and one contract link, both active.
///
/// The link's target names an `entity_id` and a `state_id` that exist in **no** local table. That is
/// not sloppiness in the fixture — it is the shape every real cross-repository link has, and a test
/// that used a local entity id would prove the opposite of what this table is for.
fn v8_database_with_a_link() -> nerve_store::Connection {
    let conn = open_in_memory().unwrap();
    migrate(&conn).unwrap();
    conn.execute_batch(
        "INSERT INTO repository VALUES ('r','p','/tmp','t');
         INSERT INTO repository_state VALUES ('s','r','content',NULL,'m','t');
         INSERT INTO entity VALUES ('local1','r','file','app.ts','src','typescript',NULL);",
    )
    .unwrap();
    conn.execute(
        REGISTRY_INSERT,
        rusqlite::params!["reg1", "repo-b", "active", None::<String>],
    )
    .unwrap();
    conn.execute(
        LINK_INSERT,
        rusqlite::params![
            "local1",
            "file",
            "reg1",
            "repo-b",
            "b-state-77",
            "b-entity-42",
            "file",
            "sub.ts",
            "src/sub.ts",
            "1:40",
            "export_map_resolved",
            None::<String>,
            "2026-01-02T00:00:00.000Z",
            None::<String>,
            "active"
        ],
    )
    .unwrap();
    conn
}

/// Build a database as the Slice 12c-ii (v7) build would have left it, with history rows in it.
///
/// **This is the upgrade path every existing database now takes.** Assembled from the written-out
/// steps rather than by migrating to 7 and stopping, so "a v7 database" means what v7 shipped and
/// not what today's chain happens to produce on the way past.
fn v7_database_with_history() -> nerve_store::Connection {
    let conn = v6_database_with_history();
    conn.execute_batch(V7_ONLY).unwrap();
    // One analysis row, so v8 is asserted against a v7 database that used all of v7 rather than
    // one that merely reached it.
    let c1 = "1".repeat(40);
    conn.execute_batch(&format!(
        "INSERT INTO git_rename_analysis VALUES
             ('r','{c1}','nerve-line-multiset','1',1,2,1,1,1,1,'complete','{{}}');"
    ))
    .unwrap();
    assert_eq!(schema_version(&conn).unwrap(), Some(7));
    conn
}

/// **The upgrade path every existing database takes, and it must lose nothing.**
///
/// v8 adds two tables and touches no row, so the assertion is symmetric: the registry appears empty,
/// and every history row a v7 database was holding is still there and still readable through the v7
/// shape. Empty is the correct state for a new registry — an entry that appeared without an explicit
/// command would be the auto-registration row 13 refuses.
#[test]
fn a_v7_database_upgrades_to_v8_and_every_history_row_survives() {
    let conn = v7_database_with_history();
    for table in V8_TABLES {
        assert!(
            !table_names(&conn).contains(&table.to_string()),
            "{table} exists before v8; the test would prove nothing"
        );
    }
    let renames_before = rename_labels(&conn);
    assert_eq!(renames_before.len(), 3, "or the count proves nothing");

    migrate(&conn).unwrap();

    assert_eq!(schema_version(&conn).unwrap(), Some(SCHEMA_VERSION));
    for table in V8_TABLES {
        assert!(
            table_names(&conn).contains(&table.to_string()),
            "v8 did not create {table}"
        );
        assert_eq!(scalar(&conn, &format!("SELECT count(*) FROM {table}")), 0);
    }

    // Every pre-existing row, still where it was. The rename labels are read through the v7 column
    // names, so this also proves v8 did not disturb the shape v7 built.
    assert_eq!(rename_labels(&conn), renames_before);
    assert_eq!(scalar(&conn, "SELECT count(*) FROM git_commit"), 2);
    assert_eq!(scalar(&conn, "SELECT count(*) FROM git_change"), 4);
    assert_eq!(scalar(&conn, "SELECT count(*) FROM git_history_ingest"), 1);
    assert_eq!(
        scalar(&conn, &format!("SELECT count(*) FROM {V7_TABLE}")),
        1,
        "v7's analysis row was lost"
    );
    assert_eq!(
        scalar(
            &conn,
            "SELECT count(*) FROM git_commit WHERE summary_truncation = 'unknown'"
        ),
        2
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

/// Re-migrating a v8 database changes nothing: no version row, no table, no row.
#[test]
fn re_migrating_a_v8_database_changes_nothing_and_appends_no_version_row() {
    let conn = v8_database_with_a_link();
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
    assert_eq!(scalar(&conn, "SELECT count(*) FROM repo_registry"), 1);
    assert_eq!(scalar(&conn, "SELECT count(*) FROM contract_link"), 1);
}

/// **A failing v8 step commits nothing — including the table that was created first.**
///
/// The sabotage is on the migration's **last** object, `idx_contract_link_registry`, which runs
/// after both `CREATE TABLE`s, after the registry index and after the uniqueness index. Without a
/// transaction the database would sit at v7 with both tables and two of the three indexes already
/// in it — a state no migration path could repair, because step 8 would never run again to finish
/// the job and replaying it would collide on `CREATE TABLE repo_registry`.
#[test]
fn an_interrupted_v8_migration_commits_nothing() {
    let conn = v7_database_with_history();
    conn.execute_batch("CREATE TABLE idx_contract_link_registry (sabotage TEXT);")
        .unwrap();

    let err = migrate(&conn).unwrap_err();
    assert!(
        matches!(err, nerve_store::StoreError::Sqlite(_)),
        "expected the CREATE INDEX to fail, got {err}"
    );

    assert_eq!(
        schema_version(&conn).unwrap(),
        Some(7),
        "a failed step must not record its version"
    );
    for table in V8_TABLES {
        assert!(
            !table_names(&conn).contains(&table.to_string()),
            "{table} survived a failed migration; step 8 is not transactional"
        );
    }
    for index in ["idx_repo_registry_status", "idx_contract_link_identity"] {
        assert_eq!(
            scalar(
                &conn,
                &format!("SELECT count(*) FROM sqlite_master WHERE name = '{index}'")
            ),
            0,
            "{index} survived a failed migration"
        );
    }
    // Nothing else moved, and the sabotaged name is exactly as it was.
    assert_eq!(rename_labels(&conn).len(), 3);
    assert_eq!(scalar(&conn, "SELECT count(*) FROM git_commit"), 2);
    assert_row_counts(&conn, &V5_ROW_COUNTS);
    assert_eq!(
        column_names(&conn, "idx_contract_link_registry"),
        vec!["sabotage".to_string()]
    );

    // The control: the identical fixture without the sabotage reaches v8. Without this the test
    // would pass just as well against a migration that could never succeed.
    let clean = v7_database_with_history();
    migrate(&clean).unwrap();
    assert_eq!(schema_version(&clean).unwrap(), Some(SCHEMA_VERSION));
    for table in V8_TABLES {
        assert!(table_names(&clean).contains(&table.to_string()));
    }
}

/// **A tombstone is a status and a moment, and neither may travel without the other.**
///
/// Two refusals per table, each with a control that lands. An active row carrying a withdrawal date
/// says it ended while claiming it has not; a retired row without one cannot say *when* the entry
/// stopped counting, which is exactly the fact `registry_entry_removed` reports.
#[test]
fn the_lifecycle_checks_refuse_a_status_that_disagrees_with_its_timestamp() {
    let conn = v8_database_with_a_link();
    let when = "2026-02-01T00:00:00.000Z";

    // The controls: both well-formed shapes land, so every refusal below is the constraint.
    conn.execute(
        REGISTRY_INSERT,
        rusqlite::params!["reg2", "repo-c", "tombstoned", Some(when)],
    )
    .unwrap();
    assert_eq!(scalar(&conn, "SELECT count(*) FROM repo_registry"), 2);

    // 1. An active registry entry that carries a withdrawal date.
    let err = conn
        .execute(
            REGISTRY_INSERT,
            rusqlite::params!["reg3", "repo-d", "active", Some(when)],
        )
        .unwrap_err();
    assert!(
        err.to_string().to_lowercase().contains("constraint"),
        "an active entry carried a withdrawal date: {err}"
    );

    // 2. A tombstoned registry entry that does not say when.
    let err = conn
        .execute(
            REGISTRY_INSERT,
            rusqlite::params!["reg4", "repo-e", "tombstoned", None::<String>],
        )
        .unwrap_err();
    assert!(
        err.to_string().to_lowercase().contains("constraint"),
        "a tombstone omitted its moment: {err}"
    );

    // 3. An active link that carries a withdrawal date, and 4. a withdrawn one that does not.
    let link = |identity: &str, withdrawn: Option<&str>, status: &str| {
        conn.execute(
            &LINK_INSERT.replace("'pkg-b/sub'", &format!("'{identity}'")),
            rusqlite::params![
                None::<String>,
                None::<String>,
                "reg1",
                "repo-b",
                None::<String>,
                None::<String>,
                None::<String>,
                None::<String>,
                None::<String>,
                None::<String>,
                "manifest_declared",
                None::<String>,
                "2026-01-02T00:00:00.000Z",
                withdrawn,
                status
            ],
        )
    };
    link("pkg-b", None, "active").expect("the well-formed active link must land");
    link("pkg-c", Some(when), "withdrawn").expect("the well-formed withdrawn link must land");

    let err = link("pkg-d", Some(when), "active").unwrap_err();
    assert!(
        err.to_string().to_lowercase().contains("constraint"),
        "an active link carried a withdrawal date: {err}"
    );
    let err = link("pkg-e", None, "withdrawn").unwrap_err();
    assert!(
        err.to_string().to_lowercase().contains("constraint"),
        "a withdrawn link omitted its moment: {err}"
    );

    assert_eq!(
        scalar(&conn, "SELECT count(*) FROM repo_registry"),
        2,
        "a refused entry landed anyway"
    );
    assert_eq!(
        scalar(&conn, "SELECT count(*) FROM contract_link"),
        3,
        "a refused link landed anyway"
    );
}

/// **A target id with no snapshot is the dangling pointer this table exists to prevent.**
///
/// The correction row 13 was rewritten for: a bare `target_entity_id` points into a database Nerve
/// cannot hold still, so when the target is renamed or deleted there is nothing left to *name* what
/// the link used to point at and `contract_deleted`, `target_changed` and `contract_file_missing`
/// become one indistinguishable failure. Two further refusals here: a form recorded as unsupported
/// cannot also have been resolved, and a link last seen before it was first seen is not a lifecycle.
#[test]
fn the_snapshot_checks_refuse_a_target_that_cannot_be_named() {
    let conn = v8_database_with_a_link();
    let insert = |identity: &str,
                  target_entity: Option<&str>,
                  kind: Option<&str>,
                  name: Option<&str>,
                  path: Option<&str>,
                  unsupported: Option<&str>,
                  last_seen: &str| {
        conn.execute(
            &LINK_INSERT.replace("'pkg-b/sub'", &format!("'{identity}'")),
            rusqlite::params![
                None::<String>,
                None::<String>,
                "reg1",
                "repo-b",
                None::<String>,
                target_entity,
                kind,
                name,
                path,
                None::<String>,
                "manifest_declared",
                unsupported,
                last_seen,
                None::<String>,
                "active"
            ],
        )
    };

    // The control: a full snapshot lands, so every refusal below is the constraint doing its job.
    insert(
        "ok",
        Some("b-entity-1"),
        Some("file"),
        Some("index.ts"),
        Some("src/index.ts"),
        None,
        "2026-01-02T00:00:00.000Z",
    )
    .expect("a fully snapshotted target must land");

    // 1. A target id with no snapshot at all.
    let err = insert(
        "bare",
        Some("b-entity-2"),
        None,
        None,
        None,
        None,
        "2026-01-02T00:00:00.000Z",
    )
    .unwrap_err();
    assert!(
        err.to_string().to_lowercase().contains("constraint"),
        "a target id was recorded with nothing to name it: {err}"
    );

    // 2. A partial snapshot — a path but no kind or name — is still unnameable when it moves.
    let err = insert(
        "partial",
        Some("b-entity-3"),
        None,
        None,
        Some("src/index.ts"),
        None,
        "2026-01-02T00:00:00.000Z",
    )
    .unwrap_err();
    assert!(
        err.to_string().to_lowercase().contains("constraint"),
        "a half-recorded snapshot was accepted: {err}"
    );

    // 3. A form recorded as unsupported that nonetheless claims a resolved target.
    let err = insert(
        "unsupported",
        Some("b-entity-4"),
        Some("file"),
        Some("index.ts"),
        Some("src/index.ts"),
        Some("registry-version-range"),
        "2026-01-02T00:00:00.000Z",
    )
    .unwrap_err();
    assert!(
        err.to_string().to_lowercase().contains("constraint"),
        "a form recorded as unsupported also claimed a target: {err}"
    );
    // The control for it: the same unsupported form with no target lands.
    insert(
        "unsupported-ok",
        None,
        None,
        None,
        None,
        Some("registry-version-range"),
        "2026-01-02T00:00:00.000Z",
    )
    .expect("an unsupported form must be recordable, never silently dropped");

    // 4. Last seen before first seen.
    let err = insert(
        "backwards",
        None,
        None,
        None,
        None,
        None,
        "2025-12-31T00:00:00.000Z",
    )
    .unwrap_err();
    assert!(
        err.to_string().to_lowercase().contains("constraint"),
        "a link was last seen before it was first seen: {err}"
    );

    assert_eq!(
        scalar(&conn, "SELECT count(*) FROM contract_link"),
        3,
        "a refused link landed anyway"
    );
}

/// A contract link for a registry entry that was never registered is **refused**, not orphaned.
///
/// The one foreign key `contract_link` does have, and it is deliberate: the registry entry is in
/// *this* database, so it can be enforced, and a link resolved through an entry nobody registered
/// would be a link Nerve drew without being asked to look at anything. The contrast with the target
/// side — where no foreign key is possible and none is faked — is the shape of the whole table.
#[test]
fn a_contract_link_through_an_unregistered_entry_is_refused() {
    let conn = v8_database_with_a_link();

    let err = conn
        .execute(
            &LINK_INSERT.replace("'pkg-b/sub'", "'pkg-z'"),
            rusqlite::params![
                None::<String>,
                None::<String>,
                "never-registered",
                "repo-z",
                None::<String>,
                None::<String>,
                None::<String>,
                None::<String>,
                None::<String>,
                None::<String>,
                "manifest_declared",
                None::<String>,
                "2026-01-02T00:00:00.000Z",
                None::<String>,
                "active"
            ],
        )
        .unwrap_err();
    assert!(
        err.to_string().to_lowercase().contains("foreign key"),
        "expected a foreign-key refusal, got {err}"
    );
    assert_eq!(
        scalar(&conn, "SELECT count(*) FROM contract_link"),
        1,
        "the refused row must not have landed"
    );
}

/// The same declaration recorded twice is one link, not two.
///
/// `link_id` is an autoincrement surrogate, so without the uniqueness index a re-index of an
/// unchanged tree would append a duplicate link on every run — the failure v1's comment on
/// `idx_observation_identity` records. `contract_identity` is deliberately **not** unique on its
/// own: two registered repositories declaring one identity is `duplicate_contract_identity`, a fact
/// to report rather than a row to refuse, and the control below proves it still lands.
#[test]
fn the_same_declaration_recorded_twice_is_one_link() {
    let conn = v8_database_with_a_link();
    let insert = |registry: &str, source_path_span: &str| {
        conn.execute(
            &LINK_INSERT.replace("'package.json', '3:3'", source_path_span),
            rusqlite::params![
                None::<String>,
                None::<String>,
                registry,
                "repo-b",
                None::<String>,
                None::<String>,
                None::<String>,
                None::<String>,
                None::<String>,
                None::<String>,
                "manifest_declared",
                None::<String>,
                "2026-01-02T00:00:00.000Z",
                None::<String>,
                "active"
            ],
        )
    };

    insert("reg1", "'package.json', '3:3'").expect("the first recording must land");
    let err = insert("reg1", "'package.json', '3:3'").unwrap_err();
    assert!(
        err.to_string().to_lowercase().contains("unique"),
        "the same declaration was recorded twice: {err}"
    );

    // A different line in the same manifest is a different declaration.
    insert("reg1", "'package.json', '9:9'").expect("a second declaration must land");

    // And the same contract identity through a *different* registry entry is
    // `duplicate_contract_identity` — two facts to report, not one row to refuse.
    conn.execute(
        REGISTRY_INSERT,
        rusqlite::params!["reg2", "repo-c", "active", None::<String>],
    )
    .unwrap();
    insert("reg2", "'package.json', '3:3'")
        .expect("two repositories may declare one identity; that is a fact, not a conflict");

    assert_eq!(scalar(&conn, "SELECT count(*) FROM contract_link"), 4);
    assert_eq!(
        scalar(
            &conn,
            "SELECT count(*) FROM contract_link WHERE contract_identity = 'pkg-b/sub'"
        ),
        4,
        "one identity, four recordings, none of them refused for sharing the name"
    );
}

// ---- v9: human-confirmed memory ----------------------------------------------------------------

/// What v8 added on top of v7, exactly as the Slice 13a-i build shipped it.
///
/// Written out rather than reached by calling `migrate` part of the way, for the reason [`V1_ONLY`],
/// [`V6_ONLY`] and [`V7_ONLY`] are: "a v8 database" has to mean what v8 actually left behind, which
/// is the starting point every real database now takes to v9.
const V8_ONLY: &str = r#"
CREATE TABLE repo_registry (
    repo_id                 TEXT NOT NULL REFERENCES repository(repo_id),
    registry_id             TEXT NOT NULL,
    expected_repository_id  TEXT NOT NULL,
    display_name            TEXT NOT NULL,
    local_path              TEXT NOT NULL,
    added_at                TEXT NOT NULL,
    last_seen_state         TEXT,
    last_seen_at            TEXT,
    availability_checked_at TEXT,
    status                  TEXT NOT NULL,
    withdrawn_at            TEXT,
    PRIMARY KEY (repo_id, registry_id),
    CHECK (
        (status = 'active'     AND withdrawn_at IS NULL)
     OR (status = 'tombstoned' AND withdrawn_at IS NOT NULL)
    ),
    CHECK (
        (last_seen_state IS NULL     AND last_seen_at IS NULL)
     OR (last_seen_state IS NOT NULL AND last_seen_at IS NOT NULL)
    ),
    CHECK (registry_id <> '' AND expected_repository_id <> '' AND local_path <> '')
);
CREATE INDEX idx_repo_registry_status ON repo_registry(repo_id, status);
CREATE TABLE contract_link (
    link_id                       INTEGER PRIMARY KEY AUTOINCREMENT,
    repo_id                       TEXT NOT NULL REFERENCES repository(repo_id),
    source_repository_id          TEXT NOT NULL,
    source_state_at_resolution    TEXT NOT NULL REFERENCES repository_state(state_id),
    source_entity_id              TEXT REFERENCES entity(entity_id),
    source_kind_snapshot          TEXT,
    source_path                   TEXT NOT NULL,
    source_span                   TEXT NOT NULL,
    registry_entry_id             TEXT NOT NULL,
    expected_target_repository_id TEXT NOT NULL,
    target_state_at_resolution    TEXT,
    target_entity_id              TEXT,
    target_kind_snapshot          TEXT,
    target_name_snapshot          TEXT,
    target_path_snapshot          TEXT,
    target_span_snapshot          TEXT,
    relation_semantics            TEXT NOT NULL,
    contract_kind                 TEXT NOT NULL,
    contract_identity             TEXT NOT NULL,
    expected_contract_version     TEXT,
    observed_contract_version     TEXT,
    resolution_method             TEXT NOT NULL,
    extractor_id                  TEXT NOT NULL,
    extractor_version             TEXT NOT NULL,
    evidence_details              TEXT,
    ambiguity                     TEXT,
    unsupported_reason            TEXT,
    first_seen_at                 TEXT NOT NULL,
    last_seen_at                  TEXT NOT NULL,
    withdrawn_at                  TEXT,
    status                        TEXT NOT NULL,
    FOREIGN KEY (repo_id, registry_entry_id) REFERENCES repo_registry(repo_id, registry_id),
    CHECK (
        (status = 'active'    AND withdrawn_at IS NULL)
     OR (status = 'withdrawn' AND withdrawn_at IS NOT NULL)
    ),
    CHECK (
        target_entity_id IS NULL
     OR (target_kind_snapshot IS NOT NULL
            AND target_name_snapshot IS NOT NULL
            AND target_path_snapshot IS NOT NULL)
    ),
    CHECK (
        unsupported_reason IS NULL
     OR (target_entity_id IS NULL AND target_path_snapshot IS NULL)
    ),
    CHECK (last_seen_at >= first_seen_at),
    CHECK (
        source_repository_id <> '' AND source_path <> '' AND source_span <> ''
        AND registry_entry_id <> '' AND expected_target_repository_id <> ''
        AND relation_semantics <> '' AND contract_kind <> '' AND contract_identity <> ''
        AND extractor_id <> '' AND extractor_version <> ''
    )
);
CREATE UNIQUE INDEX idx_contract_link_identity ON contract_link(
    repo_id, registry_entry_id, contract_kind, contract_identity,
    source_path, source_span, resolution_method
);
CREATE INDEX idx_contract_link_registry ON contract_link(repo_id, registry_entry_id, status);
INSERT INTO schema_version (version, applied_at, description)
    VALUES (8, '2026-01-01T00:00:00.000Z', 'Slice 13a-i');
"#;

/// A memory record, named by column. The insert every v9 test below builds on.
///
/// The subject columns hold a snapshot of `local1`, which the fixture also creates as a real
/// `entity` row — so the deletion test below can remove the entity and watch the record survive.
const MEMORY_INSERT: &str = "INSERT INTO memory
     (memory_id, repo_id, subject_entity_id_snapshot, subject_kind_snapshot,
      subject_name_snapshot, subject_path_snapshot, subject_selector_snapshot, anchor_state_id,
      scope, claim_key, content, author_label, created_at, status, supersedes_memory_id,
      invalidated_at, invalidation_reason)
 VALUES (?1, 'r', ?2, 'file', 'app.ts', 'src/app.ts', 'file:src/app.ts', 's',
         'implementation', ?3, 'the retry budget here is deliberate', 'krish',
         '2026-01-01T00:00:00.000Z', ?4, ?5, ?6, ?7)";

/// A v8 database with a registry entry and a contract link, assembled from what v8 shipped.
fn v8_database_from_the_written_out_steps() -> nerve_store::Connection {
    let conn = v7_database_with_history();
    conn.execute_batch(V8_ONLY).unwrap();
    conn.execute(
        REGISTRY_INSERT,
        rusqlite::params!["reg1", "repo-b", "active", None::<String>],
    )
    .unwrap();
    assert_eq!(schema_version(&conn).unwrap(), Some(8));
    conn
}

/// A current database holding one repository, one state, one entity and nothing in memory yet.
fn v9_database() -> nerve_store::Connection {
    let conn = open_in_memory().unwrap();
    migrate(&conn).unwrap();
    conn.execute_batch(
        "INSERT INTO repository VALUES ('r','p','/tmp','t');
         INSERT INTO repository_state VALUES ('s','r','content',NULL,'m','t');
         INSERT INTO entity VALUES ('local1','r','file','app.ts','src','typescript',NULL);",
    )
    .unwrap();
    conn
}

/// A well-formed active record with no claim key, which every refusal below is measured against.
fn insert_active_memory(conn: &nerve_store::Connection, memory_id: &str) {
    conn.execute(
        MEMORY_INSERT,
        rusqlite::params![
            memory_id,
            "local1",
            None::<String>,
            "active",
            None::<String>,
            None::<String>,
            None::<String>
        ],
    )
    .expect("a well-formed record must land");
}

/// **The upgrade path every existing database takes, and it must lose nothing.**
///
/// v9 adds three tables and touches no row, so the assertion is symmetric: memory appears empty, and
/// every history and registry row a v8 database was holding is still there. Empty is the only
/// correct state — memory is the one thing in this database a human authored, so a record appearing
/// without anybody writing it would be a note nobody made.
#[test]
fn a_v8_database_upgrades_to_v9_and_every_earlier_row_survives() {
    let conn = v8_database_from_the_written_out_steps();
    for table in V9_TABLES {
        assert!(
            !table_names(&conn).contains(&table.to_string()),
            "{table} exists before v9; the test would prove nothing"
        );
    }
    let renames_before = rename_labels(&conn);
    assert_eq!(renames_before.len(), 3, "or the count proves nothing");

    migrate(&conn).unwrap();

    assert_eq!(schema_version(&conn).unwrap(), Some(SCHEMA_VERSION));
    for table in V9_TABLES {
        assert!(
            table_names(&conn).contains(&table.to_string()),
            "v9 did not create {table}"
        );
        assert_eq!(scalar(&conn, &format!("SELECT count(*) FROM {table}")), 0);
    }

    // Every pre-existing row, still where it was.
    assert_eq!(rename_labels(&conn), renames_before);
    assert_eq!(scalar(&conn, "SELECT count(*) FROM git_commit"), 2);
    assert_eq!(scalar(&conn, "SELECT count(*) FROM git_change"), 4);
    assert_eq!(scalar(&conn, "SELECT count(*) FROM git_history_ingest"), 1);
    assert_eq!(
        scalar(&conn, &format!("SELECT count(*) FROM {V7_TABLE}")),
        1
    );
    assert_eq!(scalar(&conn, "SELECT count(*) FROM repo_registry"), 1);
    assert_row_counts(&conn, &V5_ROW_COUNTS);
    assert_eq!(
        conn.query_row("SELECT framework_version FROM module_facts", [], |row| {
            row.get::<_, String>(0)
        })
        .unwrap(),
        "2.0.0"
    );
}

/// Re-migrating a v9 database changes nothing: no version row, no table, no row.
#[test]
fn re_migrating_a_v9_database_changes_nothing_and_appends_no_version_row() {
    let conn = v9_database();
    insert_active_memory(&conn, "m1");
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
    assert_eq!(scalar(&conn, "SELECT count(*) FROM memory"), 1);
}

/// **A failing v9 step commits nothing — including the table that was created first.**
///
/// The sabotage is on the migration's **last** object, `idx_memory_event_memory`, which runs after
/// all three `CREATE TABLE`s and after six other indexes. Without a transaction the database would
/// sit at v8 with every memory table already in it — a state no migration path could repair,
/// because step 9 would never run again to finish the job and replaying it would collide on
/// `CREATE TABLE memory`.
#[test]
fn an_interrupted_v9_migration_commits_nothing() {
    let conn = v8_database_from_the_written_out_steps();
    conn.execute_batch("CREATE TABLE idx_memory_event_memory (sabotage TEXT);")
        .unwrap();

    let err = migrate(&conn).unwrap_err();
    assert!(
        matches!(err, nerve_store::StoreError::Sqlite(_)),
        "expected the CREATE INDEX to fail, got {err}"
    );

    assert_eq!(
        schema_version(&conn).unwrap(),
        Some(8),
        "a failed step must not record its version"
    );
    for table in V9_TABLES {
        assert!(
            !table_names(&conn).contains(&table.to_string()),
            "{table} survived a failed migration; step 9 is not transactional"
        );
    }
    for index in [
        "idx_memory_subject",
        "idx_memory_scope",
        "idx_memory_claim",
        "idx_memory_supersedes",
        "idx_memory_citation_memory",
        "idx_memory_citation_path",
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
    // Nothing else moved, and the sabotaged name is exactly as it was.
    assert_eq!(rename_labels(&conn).len(), 3);
    assert_eq!(scalar(&conn, "SELECT count(*) FROM repo_registry"), 1);
    assert_row_counts(&conn, &V5_ROW_COUNTS);
    assert_eq!(
        column_names(&conn, "idx_memory_event_memory"),
        vec!["sabotage".to_string()]
    );

    // The control: the identical fixture without the sabotage reaches v9.
    let clean = v8_database_from_the_written_out_steps();
    migrate(&clean).unwrap();
    assert_eq!(schema_version(&clean).unwrap(), Some(SCHEMA_VERSION));
    for table in V9_TABLES {
        assert!(table_names(&clean).contains(&table.to_string()));
    }
}

/// **A memory record survives the deletion of its subject entity.**
///
/// The property row 14 was rewritten for, asserted at the schema level: with
/// `PRAGMA foreign_keys=ON`, a `DELETE FROM entity` naming the subject must neither be refused nor
/// take the record with it. A foreign key would give one of those two outcomes and both are
/// unacceptable — a note blocking re-indexing of the file it is about, or a routine re-index
/// silently destroying the human's note.
///
/// The same property is asserted against the **real indexer** in `nerve-index/tests/memory.rs`,
/// where `prune_orphans` does the deleting; this is the unit-level half that says which constraint
/// is responsible.
#[test]
fn a_memory_record_survives_the_deletion_of_its_subject_entity() {
    let conn = v9_database();
    insert_active_memory(&conn, "m1");

    // The control: the subject is genuinely an entity, so the delete below is a real one.
    assert_eq!(
        scalar(
            &conn,
            "SELECT count(*) FROM entity WHERE entity_id = 'local1'"
        ),
        1
    );

    let deleted = conn
        .execute("DELETE FROM entity WHERE entity_id = 'local1'", [])
        .expect("a memory record must not block the deletion of its subject");
    assert_eq!(deleted, 1);

    assert_eq!(
        scalar(&conn, "SELECT count(*) FROM memory WHERE memory_id = 'm1'"),
        1,
        "the record was destroyed with its subject"
    );
    // And it can still say what it was about, which is the whole point of the snapshot.
    assert_eq!(
        conn.query_row(
            "SELECT subject_selector_snapshot || ' / ' || subject_name_snapshot
               FROM memory WHERE memory_id = 'm1'",
            [],
            |row| row.get::<_, String>(0)
        )
        .unwrap(),
        "file:src/app.ts / app.ts"
    );
}

/// **An ending is a status and a moment, and neither may travel without the other.**
///
/// Four refusals, each with a control that lands, so every refusal is the constraint doing its job
/// rather than a malformed statement:
///
/// - an `invalidated` record with no moment — it says it stopped being true and cannot say when;
/// - an active record carrying an invalidation date — it contradicts itself;
/// - a reason for an ending that never happened;
/// - a record that replaces itself, which is the only cycle a `CHECK` can see.
#[test]
fn the_memory_checks_refuse_a_status_that_disagrees_with_its_timestamps() {
    let conn = v9_database();
    let when = "2026-02-01T00:00:00.000Z";

    // The controls: both well-formed shapes land.
    insert_active_memory(&conn, "m1");
    conn.execute(
        MEMORY_INSERT,
        rusqlite::params![
            "m2",
            "local1",
            None::<String>,
            "invalidated",
            None::<String>,
            Some(when),
            Some("the service it described was removed")
        ],
    )
    .expect("an invalidated record with a moment and a reason must land");
    assert_eq!(scalar(&conn, "SELECT count(*) FROM memory"), 2);

    let refused = |memory_id: &str,
                   status: &str,
                   supersedes: Option<&str>,
                   invalidated_at: Option<&str>,
                   reason: Option<&str>| {
        let err = conn
            .execute(
                MEMORY_INSERT,
                rusqlite::params![
                    memory_id,
                    "local1",
                    None::<String>,
                    status,
                    supersedes,
                    invalidated_at,
                    reason
                ],
            )
            .unwrap_err();
        assert!(
            err.to_string().to_lowercase().contains("constraint"),
            "expected a constraint refusal for {memory_id}, got {err}"
        );
    };

    // 1. Invalidated, with no moment. The ending cannot be reported.
    refused("m3", "invalidated", None, None, None);
    // 2. Active, carrying an ending it claims has not happened.
    refused("m4", "active", None, Some(when), None);
    // 3. A reason for an ending that never happened.
    refused("m5", "active", None, None, Some("because"));
    // 4. A record that replaces itself.
    refused("m6", "active", Some("m6"), None, None);

    assert_eq!(
        scalar(&conn, "SELECT count(*) FROM memory"),
        2,
        "a refused row landed anyway"
    );
}

/// **A row that names nothing, is about nothing, or says nothing is refused.**
///
/// The empty `claim_key` is the one worth its own case. An empty string is not a missing value, so
/// it would gather every keyless record into a single competing claim group and report ordinary
/// notes about one file as contradictions — the exact false claim the corrected §3 removes.
#[test]
fn the_memory_checks_refuse_a_record_that_names_or_says_nothing() {
    let conn = v9_database();
    insert_active_memory(&conn, "m1");

    let refused = |label: &str, columns: &str, values: &str| {
        let sql = format!(
            "INSERT INTO memory
                 (memory_id, repo_id, subject_entity_id_snapshot, subject_kind_snapshot,
                  subject_name_snapshot, subject_path_snapshot, subject_selector_snapshot,
                  anchor_state_id, scope, claim_key, content, author_label, created_at, status,
                  supersedes_memory_id, invalidated_at, invalidation_reason)
             VALUES ({values})"
        );
        let err = conn.execute(&sql, []).unwrap_err();
        assert!(
            err.to_string().to_lowercase().contains("constraint"),
            "expected a constraint refusal for {label} ({columns}), got {err}"
        );
    };

    let well_formed = "'x','r','local1','file','app.ts','src/app.ts','file:src/app.ts','s',\
                       'implementation',NULL,'something','krish','t','active',NULL,NULL,NULL";
    // The control: the shape every case below perturbs by one column does land.
    conn.execute(&format!("INSERT INTO memory VALUES ({well_formed})"), [])
        .expect("the well-formed control must land");

    refused(
        "an empty claim key",
        "claim_key",
        "'e1','r','local1','file','app.ts','src/app.ts','file:src/app.ts','s',\
         'implementation','','something','krish','t','active',NULL,NULL,NULL",
    );
    refused(
        "an empty subject id",
        "subject_entity_id_snapshot",
        "'e2','r','','file','app.ts','src/app.ts','file:src/app.ts','s',\
         'implementation',NULL,'something','krish','t','active',NULL,NULL,NULL",
    );
    refused(
        "an empty selector",
        "subject_selector_snapshot",
        "'e3','r','local1','file','app.ts','src/app.ts','','s',\
         'implementation',NULL,'something','krish','t','active',NULL,NULL,NULL",
    );
    refused(
        "an empty content",
        "content",
        "'e4','r','local1','file','app.ts','src/app.ts','file:src/app.ts','s',\
         'implementation',NULL,'','krish','t','active',NULL,NULL,NULL",
    );
    refused(
        "an empty author label",
        "author_label",
        "'e5','r','local1','file','app.ts','src/app.ts','file:src/app.ts','s',\
         'implementation',NULL,'something','','t','active',NULL,NULL,NULL",
    );
    // v10 enumerates the scope, so the empty string is refused by the enumeration rather than by
    // an emptiness check — and so is every other value outside the domain, which is the stronger
    // claim. `file` is asserted by name because it is what 14a's own fixtures used.
    refused(
        "an empty scope",
        "scope",
        "'e6','r','local1','file','app.ts','src/app.ts','file:src/app.ts','s',\
         '',NULL,'something','krish','t','active',NULL,NULL,NULL",
    );
    refused(
        "the subject's kind used as a scope",
        "scope",
        "'e7','r','local1','file','app.ts','src/app.ts','file:src/app.ts','s',\
         'file',NULL,'something','krish','t','active',NULL,NULL,NULL",
    );
    refused(
        "a misspelt scope",
        "scope",
        "'e8','r','local1','file','app.ts','src/app.ts','file:src/app.ts','s',\
         'opertions',NULL,'something','krish','t','active',NULL,NULL,NULL",
    );

    // An empty *path* is deliberately legal: the repository entity is no file and says so honestly.
    conn.execute(
        "INSERT INTO memory VALUES
             ('repo-note','r','repo1','repository','.','','repo:.','s',
              'process',NULL,'this project is offline-first','krish','t','active',
              NULL,NULL,NULL)",
        [],
    )
    .expect("a note about the repository itself must land with an empty path");

    assert_eq!(scalar(&conn, "SELECT count(*) FROM memory"), 3);
}

/// **At most one record may replace any given record**, so the derived inverse is a function.
///
/// There is no `superseded_by` column, and this uniqueness is what makes its absence safe: with two
/// records claiming to replace one, "what replaced it" would have several answers and deriving the
/// inverse would mean choosing between them.
#[test]
fn only_one_record_may_supersede_a_given_record() {
    let conn = v9_database();
    insert_active_memory(&conn, "m1");

    let supersede = |memory_id: &str, target: &str| {
        conn.execute(
            MEMORY_INSERT,
            rusqlite::params![
                memory_id,
                "local1",
                None::<String>,
                "active",
                Some(target),
                None::<String>,
                None::<String>
            ],
        )
    };

    supersede("m2", "m1").expect("the first successor must land");
    let err = supersede("m3", "m1").unwrap_err();
    assert!(
        err.to_string().to_lowercase().contains("unique"),
        "a second record claims to replace m1: {err}"
    );

    // Two records superseding *different* predecessors is ordinary and must still land.
    supersede("m4", "m2").expect("a chain must be recordable");
    // And a record superseding nothing is the common case: the partial index must not refuse it.
    insert_active_memory(&conn, "m5");
    insert_active_memory(&conn, "m6");

    assert_eq!(scalar(&conn, "SELECT count(*) FROM memory"), 5);
}

/// A supersession, citation or event naming a record that does not exist is **refused**.
///
/// The foreign keys memory *can* have, and they are enforced: `PRAGMA foreign_keys=ON` is set on
/// every connection. The contrast with the subject side — where no foreign key is possible and none
/// is faked — is the shape of the whole table.
#[test]
fn a_citation_event_or_supersession_naming_no_record_is_refused() {
    let conn = v9_database();
    insert_active_memory(&conn, "m1");

    let citation = "INSERT INTO memory_citation
         (repo_id, memory_id, cited_entity_id_snapshot, cited_kind_snapshot, cited_name_snapshot,
          cited_path_snapshot, cited_span_snapshot, cited_at_state, created_at)
     VALUES ('r', ?1, NULL, NULL, NULL, 'src/app.ts', '3:9', 's', 't')";
    let event = "INSERT INTO memory_event
         (repo_id, memory_id, at, operation, from_status, to_status, note)
     VALUES ('r', ?1, 't', 'confirmed', 'proposed', 'active', NULL)";

    // The controls: both land against the record that exists.
    conn.execute(citation, ["m1"]).unwrap();
    conn.execute(event, ["m1"]).unwrap();

    for (label, sql) in [("citation", citation), ("event", event)] {
        let err = conn.execute(sql, ["absent"]).unwrap_err();
        assert!(
            err.to_string().to_lowercase().contains("foreign key"),
            "expected a foreign-key refusal for a {label} naming no record, got {err}"
        );
    }

    let err = conn
        .execute(
            MEMORY_INSERT,
            rusqlite::params![
                "m2",
                "local1",
                None::<String>,
                "active",
                Some("absent"),
                None::<String>,
                None::<String>
            ],
        )
        .unwrap_err();
    assert!(
        err.to_string().to_lowercase().contains("foreign key"),
        "expected a foreign-key refusal for a supersession naming no record, got {err}"
    );

    assert_eq!(scalar(&conn, "SELECT count(*) FROM memory_citation"), 1);
    assert_eq!(scalar(&conn, "SELECT count(*) FROM memory_event"), 1);
    assert_eq!(scalar(&conn, "SELECT count(*) FROM memory"), 1);
}

/// **A cited entity id with no snapshot beside it is the dangling pointer this table prevents.**
///
/// The same refusal `contract_link` makes for its target, one table over and for the same reason:
/// once the entity is pruned there would be nothing left to name what was cited.
#[test]
fn the_citation_check_refuses_a_cited_entity_that_cannot_be_named() {
    let conn = v9_database();
    insert_active_memory(&conn, "m1");

    let insert = |entity: Option<&str>, kind: Option<&str>, name: Option<&str>, path: &str| {
        conn.execute(
            "INSERT INTO memory_citation
                 (repo_id, memory_id, cited_entity_id_snapshot, cited_kind_snapshot,
                  cited_name_snapshot, cited_path_snapshot, cited_span_snapshot, cited_at_state,
                  created_at)
             VALUES ('r', 'm1', ?1, ?2, ?3, ?4, NULL, 's', 't')",
            rusqlite::params![entity, kind, name, path],
        )
    };

    // The controls: a citation that names a thing, and one that names only a place.
    insert(Some("local1"), Some("file"), Some("app.ts"), "src/app.ts")
        .expect("a fully named citation must land");
    insert(None, None, None, "src/app.ts").expect("a citation naming only a place must land");

    for (label, entity, kind, name, path) in [
        (
            "no kind",
            Some("local1"),
            None,
            Some("app.ts"),
            "src/app.ts",
        ),
        ("no name", Some("local1"), Some("file"), None, "src/app.ts"),
        (
            "an empty id",
            Some(""),
            Some("file"),
            Some("app.ts"),
            "src/app.ts",
        ),
        ("no path", None, None, None, ""),
    ] {
        let err = insert(entity, kind, name, path).unwrap_err();
        assert!(
            err.to_string().to_lowercase().contains("constraint"),
            "expected a constraint refusal for a citation with {label}, got {err}"
        );
    }

    assert_eq!(scalar(&conn, "SELECT count(*) FROM memory_citation"), 2);
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

// ---- v10: the two closed domains, and the rebuild they cost ------------------------------------

/// The v9 schema as it shipped in `2190643`, written out so a **genuine v9 database** can be built.
///
/// v10 rebuilds `memory` and `memory_event`, and the only way to know a rebuild is lossless is to
/// put rows in the *old* shape and count them out of the new one — the same reason `V6_ONLY` and
/// `V8_ONLY` exist above. The difference that matters is here: v9's `memory.scope` and
/// `memory_event.operation` carry **no enumeration**, which is exactly what makes the refusal test
/// below possible at all.
const V9_ONLY: &str = r#"
CREATE TABLE memory (
    memory_id                  TEXT NOT NULL,
    repo_id                    TEXT NOT NULL REFERENCES repository(repo_id),
    subject_entity_id_snapshot TEXT NOT NULL,
    subject_kind_snapshot      TEXT NOT NULL,
    subject_name_snapshot      TEXT NOT NULL,
    subject_path_snapshot      TEXT NOT NULL,
    subject_selector_snapshot  TEXT NOT NULL,
    anchor_state_id            TEXT NOT NULL REFERENCES repository_state(state_id),
    scope                      TEXT NOT NULL,
    claim_key                  TEXT,
    content                    TEXT NOT NULL,
    author_label               TEXT NOT NULL,
    created_at                 TEXT NOT NULL,
    status                     TEXT NOT NULL,
    supersedes_memory_id       TEXT,
    invalidated_at             TEXT,
    invalidation_reason        TEXT,
    PRIMARY KEY (repo_id, memory_id),
    FOREIGN KEY (repo_id, supersedes_memory_id) REFERENCES memory(repo_id, memory_id),
    CHECK (status IN ('proposed', 'active', 'superseded', 'invalidated')),
    CHECK (
        (status =  'invalidated' AND invalidated_at IS NOT NULL)
     OR (status <> 'invalidated' AND invalidated_at IS NULL)
    ),
    CHECK (invalidation_reason IS NULL OR invalidated_at IS NOT NULL),
    CHECK (supersedes_memory_id IS NULL OR supersedes_memory_id <> memory_id),
    CHECK (claim_key IS NULL OR claim_key <> ''),
    CHECK (
        memory_id <> '' AND subject_entity_id_snapshot <> '' AND subject_kind_snapshot <> ''
        AND subject_name_snapshot <> '' AND subject_selector_snapshot <> ''
        AND scope <> '' AND content <> '' AND author_label <> '' AND status <> ''
    )
);
CREATE INDEX idx_memory_subject ON memory(repo_id, subject_entity_id_snapshot, status);
CREATE INDEX idx_memory_scope   ON memory(repo_id, scope, status);
CREATE INDEX idx_memory_claim ON memory(repo_id, subject_entity_id_snapshot, scope, claim_key)
    WHERE claim_key IS NOT NULL;
CREATE UNIQUE INDEX idx_memory_supersedes ON memory(repo_id, supersedes_memory_id)
    WHERE supersedes_memory_id IS NOT NULL;
CREATE TABLE memory_citation (
    citation_id              INTEGER PRIMARY KEY AUTOINCREMENT,
    repo_id                  TEXT NOT NULL REFERENCES repository(repo_id),
    memory_id                TEXT NOT NULL,
    cited_entity_id_snapshot TEXT,
    cited_kind_snapshot      TEXT,
    cited_name_snapshot      TEXT,
    cited_path_snapshot      TEXT NOT NULL,
    cited_span_snapshot      TEXT,
    cited_at_state           TEXT NOT NULL REFERENCES repository_state(state_id),
    created_at               TEXT NOT NULL,
    FOREIGN KEY (repo_id, memory_id) REFERENCES memory(repo_id, memory_id),
    CHECK (
        cited_entity_id_snapshot IS NULL
     OR (cited_entity_id_snapshot <> ''
            AND cited_kind_snapshot IS NOT NULL AND cited_kind_snapshot <> ''
            AND cited_name_snapshot IS NOT NULL AND cited_name_snapshot <> '')
    ),
    CHECK (cited_path_snapshot <> '' AND (cited_span_snapshot IS NULL OR cited_span_snapshot <> ''))
);
CREATE INDEX idx_memory_citation_memory ON memory_citation(repo_id, memory_id);
CREATE INDEX idx_memory_citation_path   ON memory_citation(repo_id, cited_path_snapshot);
CREATE TABLE memory_event (
    event_id    INTEGER PRIMARY KEY AUTOINCREMENT,
    repo_id     TEXT NOT NULL REFERENCES repository(repo_id),
    memory_id   TEXT NOT NULL,
    at          TEXT NOT NULL,
    operation   TEXT NOT NULL,
    from_status TEXT,
    to_status   TEXT NOT NULL,
    note        TEXT,
    FOREIGN KEY (repo_id, memory_id) REFERENCES memory(repo_id, memory_id),
    CHECK (at <> '' AND operation <> '' AND to_status <> ''),
    CHECK (from_status IS NULL OR from_status <> '')
);
CREATE INDEX idx_memory_event_memory ON memory_event(repo_id, memory_id, event_id);
INSERT INTO schema_version (version, applied_at, description)
    VALUES (9, '2026-01-01T00:00:00.000Z', 'Slice 14a');
"#;

/// A v9 database holding a memory record, a citation, an event **and a supersession**.
///
/// The child rows are the whole point. v7's rebuild had none — `git_rename_hypothesis` is referenced
/// by nothing — so a v10 test against an empty `memory` table would prove exactly the thing that was
/// never in doubt. `scope` and `operation` are caller-supplied so the refusal tests can hand this
/// fixture a value v10 does not admit.
fn v9_database_with_memory(scope: &str, operation: &str) -> nerve_store::Connection {
    let conn = v8_database_from_the_written_out_steps();
    conn.execute_batch(V9_ONLY).unwrap();
    assert_eq!(schema_version(&conn).unwrap(), Some(9));

    // `e2` is a real entity in this fixture, so the subject snapshot names something that exists.
    conn.execute_batch(&format!(
        "INSERT INTO memory VALUES
             ('m1','r','e2','module','math','src/math.ts','module:math','s','{scope}','owner',
              'the payments team owns this code','krish','2026-01-01T00:00:00.000Z','superseded',
              NULL,NULL,NULL);
         INSERT INTO memory VALUES
             ('m2','r','e2','module','math','src/math.ts','module:math','s','{scope}','owner',
              'platform owns its deployment','krish','2026-01-02T00:00:00.000Z','active',
              'm1',NULL,NULL);
         INSERT INTO memory_citation VALUES
             (1,'r','m1','e1','function','add','src/math.ts','1:1','s',
              '2026-01-01T00:00:00.000Z');
         INSERT INTO memory_event VALUES
             (1,'r','m1','2026-01-01T00:00:00.000Z','proposed',NULL,'proposed','written down');
         INSERT INTO memory_event VALUES
             (2,'r','m1','2026-01-02T00:00:00.000Z','{operation}','active','superseded','handover');"
    ))
    .unwrap();
    conn
}

/// Every memory row, citation and event in one deterministic string.
///
/// A full serialisation rather than a count, for the reason `tests/memory.rs` gives: a count says
/// *that* the rebuild kept the rows and this says *what* it kept, which is the claim a lossless
/// copy actually makes.
fn memory_tables(conn: &nerve_store::Connection) -> String {
    let mut out = String::new();
    for (label, sql) in [
        (
            "memory",
            "SELECT memory_id || '|' || repo_id || '|' || subject_entity_id_snapshot || '|'
                 || subject_kind_snapshot || '|' || subject_name_snapshot || '|'
                 || subject_path_snapshot || '|' || subject_selector_snapshot || '|'
                 || anchor_state_id || '|' || scope || '|' || coalesce(claim_key,'-') || '|'
                 || content || '|' || author_label || '|' || created_at || '|' || status || '|'
                 || coalesce(supersedes_memory_id,'-') || '|' || coalesce(invalidated_at,'-')
                 || '|' || coalesce(invalidation_reason,'-')
               FROM memory ORDER BY memory_id",
        ),
        (
            "memory_citation",
            "SELECT citation_id || '|' || repo_id || '|' || memory_id || '|'
                 || coalesce(cited_entity_id_snapshot,'-') || '|'
                 || coalesce(cited_kind_snapshot,'-') || '|' || coalesce(cited_name_snapshot,'-')
                 || '|' || cited_path_snapshot || '|' || coalesce(cited_span_snapshot,'-') || '|'
                 || cited_at_state || '|' || created_at
               FROM memory_citation ORDER BY citation_id",
        ),
        (
            "memory_event",
            "SELECT event_id || '|' || repo_id || '|' || memory_id || '|' || at || '|' || operation
                 || '|' || coalesce(from_status,'-') || '|' || to_status || '|'
                 || coalesce(note,'-')
               FROM memory_event ORDER BY event_id",
        ),
    ] {
        out.push_str(label);
        out.push('\n');
        let mut stmt = conn.prepare(sql).unwrap();
        let rows = stmt.query_map([], |row| row.get::<_, String>(0)).unwrap();
        for row in rows {
            out.push_str(&row.unwrap());
            out.push('\n');
        }
    }
    out
}

/// **The v9 → v10 rebuild carries every child row, and the foreign keys still enforce afterwards.**
///
/// This is the test v7's precedent could not stand in for. `git_rename_hypothesis` was referenced by
/// nothing, so its create-copy-drop-rename had no child rows to orphan; `memory` is referenced by
/// `memory_citation`, by `memory_event` and by *itself* through `supersedes_memory_id`, and
/// `PRAGMA foreign_keys=ON` is set on every connection. So the assertions are:
///
/// - every row of all three tables survives **with identical content**, including the surrogate keys
///   `citation_id` and `event_id`, which a copy that re-issued them would silently renumber;
/// - the self-supersession still resolves, by joining `m2` back to `m1` through the schema rather
///   than by reading the column — a dangling `supersedes_memory_id` would still *read* fine;
/// - the child foreign keys still **enforce**, which is the half a rebuild is most likely to lose:
///   drop the parent, recreate it, and a child whose constraint silently stopped pointing anywhere
///   accepts a row naming a record that does not exist.
#[test]
fn the_v10_rebuild_keeps_every_child_row_and_the_foreign_keys_still_enforce() {
    let conn = v9_database_with_memory("implementation", "superseded");

    // Anti-vacuity: there is something to lose, and v9 genuinely has no scope constraint — the
    // control that makes the refusal test below a measurement rather than a tautology.
    let before = memory_tables(&conn);
    assert_eq!(before.lines().count(), 8, "{before}");
    assert!(before.contains("platform owns its deployment"), "{before}");

    migrate(&conn).unwrap();
    assert_eq!(schema_version(&conn).unwrap(), Some(SCHEMA_VERSION));

    assert_eq!(
        memory_tables(&conn),
        before,
        "the v10 rebuild did not carry every row through unchanged"
    );
    // Every earlier table is still where it was, too: this step rebuilds three tables and must not
    // touch the other twenty.
    assert_eq!(rename_labels(&conn).len(), 3);
    assert_eq!(scalar(&conn, "SELECT count(*) FROM repo_registry"), 1);
    assert_row_counts(&conn, &V5_ROW_COUNTS);

    // The self-supersession resolves through the schema rather than through the column.
    assert_eq!(
        conn.query_row(
            "SELECT successor.memory_id || ' replaced ' || predecessor.memory_id
               FROM memory successor
               JOIN memory predecessor
                 ON predecessor.repo_id = successor.repo_id
                AND predecessor.memory_id = successor.supersedes_memory_id
              WHERE successor.memory_id = 'm2'",
            [],
            |row| row.get::<_, String>(0)
        )
        .unwrap(),
        "m2 replaced m1"
    );
    assert_eq!(
        scalar(&conn, "SELECT count(*) FROM pragma_foreign_key_check"),
        0,
        "the rebuilt tables left a dangling reference"
    );

    // **The foreign keys still enforce.** A citation and an event naming no record are refused, and
    // so is a supersession of one — the three constraints the rebuild had to carry across.
    let orphan_citation = conn.execute(
        "INSERT INTO memory_citation
             (repo_id, memory_id, cited_path_snapshot, cited_at_state, created_at)
         VALUES ('r','absent','src/math.ts','s','t')",
        [],
    );
    assert!(
        orphan_citation.is_err(),
        "a citation for a nonexistent record was accepted after the rebuild"
    );
    let orphan_event = conn.execute(
        "INSERT INTO memory_event (repo_id, memory_id, at, operation, to_status)
         VALUES ('r','absent','t','cited','active')",
        [],
    );
    assert!(
        orphan_event.is_err(),
        "an event for a nonexistent record was accepted after the rebuild"
    );
    let orphan_supersession = conn.execute(
        MEMORY_INSERT,
        rusqlite::params![
            "m3",
            "e2",
            None::<String>,
            "active",
            Some("absent"),
            None::<String>,
            None::<String>
        ],
    );
    assert!(
        orphan_supersession.is_err(),
        "a record superseding a nonexistent one was accepted after the rebuild"
    );

    // And the two new CHECKs are live, each refused by name with the control beside it. The event
    // probe is an INSERT rather than an UPDATE on purpose: no statement anywhere in this workspace
    // may `UPDATE` that table, and `tests/memory.rs` scans every source file for one.
    let refused = conn
        .execute(
            "UPDATE memory SET scope = ?1 WHERE memory_id = 'm1'",
            rusqlite::params!["file"],
        )
        .expect_err("`file` is a subject kind and was accepted as a scope")
        .to_string();
    assert!(
        refused.contains("CHECK constraint failed"),
        "`file` was refused, but not by the CHECK: {refused}"
    );
    let refused = conn
        .execute(
            "INSERT INTO memory_event (repo_id, memory_id, at, operation, to_status)
             VALUES ('r','m1','t',?1,'active')",
            rusqlite::params!["deleted"],
        )
        .expect_err("`deleted` is not an operation and it was stored")
        .to_string();
    assert!(
        refused.contains("CHECK constraint failed"),
        "`deleted` was refused, but not by the CHECK: {refused}"
    );

    conn.execute(
        "UPDATE memory SET scope = 'operations' WHERE memory_id = 'm1'",
        [],
    )
    .expect("a scope in the domain must still be accepted");
    conn.execute(
        "INSERT INTO memory_event (repo_id, memory_id, at, operation, to_status)
         VALUES ('r','m1','t','cited','superseded')",
        [],
    )
    .expect("an operation in the domain must still be accepted");
}

/// **A scope v10 does not admit stops the migration, and nothing is dropped or rewritten.**
///
/// v9 stored `scope` opaque, and 14a's own tests used `"file"` and `"repository"` — so a database on
/// disk may genuinely hold one. A migration has three options at that point and only one of them is
/// honest: memory is the single thing in this database re-indexing cannot rebuild, so dropping the
/// row or rewriting it to a default would destroy the one artefact the feature exists to protect.
/// The offending values are **named**, because a refusal that will not say what it refused leaves a
/// human to go and find out.
#[test]
fn a_scope_outside_the_v10_domain_refuses_the_migration_and_changes_nothing() {
    let conn = v9_database_with_memory("file", "superseded");
    let before = memory_tables(&conn);
    let tables_before = table_names(&conn);

    let err = migrate(&conn).unwrap_err();
    let message = err.to_string();
    assert!(
        matches!(err, nerve_store::StoreError::MigrationDomain { .. }),
        "expected a domain refusal, got {message}"
    );
    assert!(
        message.contains("\"file\""),
        "the refusal does not name what it found: {message}"
    );
    assert!(
        message.contains("memory.scope"),
        "the refusal does not name the column: {message}"
    );
    assert!(
        message.contains("implementation"),
        "the refusal does not say what would be admitted: {message}"
    );

    // Nothing moved: not the version, not a table, not a row, not a character of a human's note.
    assert_eq!(schema_version(&conn).unwrap(), Some(9));
    assert_eq!(table_names(&conn), tables_before);
    assert_eq!(memory_tables(&conn), before);
    assert_eq!(
        scalar(&conn, "SELECT count(*) FROM memory WHERE scope = 'file'"),
        2,
        "the migration rewrote or dropped a row it had refused to migrate"
    );

    // The control: the identical fixture with an admitted scope reaches v10, so the refusal above
    // is the domain check and not a broken fixture.
    let clean = v9_database_with_memory("process", "superseded");
    migrate(&clean).unwrap();
    assert_eq!(schema_version(&clean).unwrap(), Some(SCHEMA_VERSION));
    assert_eq!(scalar(&clean, "SELECT count(*) FROM memory"), 2);
}

/// The same refusal for `memory_event.operation`, which v9 also left open.
///
/// Asserted separately rather than folded into the test above, because the two columns are checked
/// in sequence: a single fixture violating both would pass while the second check did nothing.
#[test]
fn an_operation_outside_the_v10_domain_refuses_the_migration_and_changes_nothing() {
    let conn = v9_database_with_memory("interface", "supersede");
    let before = memory_tables(&conn);

    let err = migrate(&conn).unwrap_err();
    let message = err.to_string();
    assert!(
        matches!(err, nerve_store::StoreError::MigrationDomain { .. }),
        "expected a domain refusal, got {message}"
    );
    assert!(
        message.contains("memory_event.operation"),
        "the refusal names the wrong column: {message}"
    );
    assert!(
        message.contains("\"supersede\""),
        "the refusal does not name what it found: {message}"
    );
    // The verb it *would* have admitted is one character different, and the message has to show
    // both or a reader cannot see what to change.
    assert!(message.contains("'superseded'"), "{message}");

    assert_eq!(schema_version(&conn).unwrap(), Some(9));
    assert_eq!(memory_tables(&conn), before);
    assert_eq!(
        scalar(
            &conn,
            "SELECT count(*) FROM memory_event WHERE operation = 'supersede'"
        ),
        1
    );
}

/// Re-migrating a v10 database changes nothing: no version row, no table, no rebuilt row.
///
/// The rebuild is the reason this needs its own test rather than reusing v9's. A step that ran
/// twice would drop and recreate two tables a second time, and the failure mode is not an error —
/// it is a silent re-copy that could renumber a surrogate key.
#[test]
fn re_migrating_a_v10_database_changes_nothing_and_appends_no_version_row() {
    let conn = v9_database_with_memory("operations", "invalidated");
    migrate(&conn).unwrap();

    let memory_before = memory_tables(&conn);
    let tables_before = table_names(&conn);
    let versions_before = scalar(&conn, "SELECT count(*) FROM schema_version");
    assert_eq!(versions_before, SCHEMA_VERSION);

    migrate(&conn).unwrap();
    migrate(&conn).unwrap();

    assert_eq!(table_names(&conn), tables_before);
    assert_eq!(memory_tables(&conn), memory_before);
    assert_eq!(
        scalar(&conn, "SELECT count(*) FROM schema_version"),
        versions_before,
        "re-migrating must not append a version row"
    );
    assert_eq!(schema_version(&conn).unwrap(), Some(SCHEMA_VERSION));
}

/// **A failing v10 step commits nothing, and the tables it took apart come back.**
///
/// The sabotage is a table named `idx_memory_claim`, so the rebuild fails at a `CREATE INDEX` that
/// runs *after* `memory` has already been dropped and recreated. Without the step's transaction the
/// database would sit at v9 with `memory` emptied and its rows only in a temporary table that the
/// connection is about to discard — the human's notes gone, which is the one outcome this row must
/// never produce.
#[test]
fn an_interrupted_v10_migration_leaves_every_memory_row_where_it_was() {
    let conn = v9_database_with_memory("process", "superseded");
    let before = memory_tables(&conn);
    // The sabotage has to bite *after* `memory` has been dropped and recreated, and every name
    // v10 creates already exists at v9 — so a colliding table cannot be the probe. A trigger on
    // `memory_citation` can: it fires on the final re-insert, which is the last statement of the
    // step and runs long after the parent table was taken apart.
    conn.execute_batch(
        "CREATE TRIGGER sabotage BEFORE INSERT ON memory_citation
         BEGIN SELECT RAISE(ABORT, 'sabotage'); END;",
    )
    .unwrap();

    let err = migrate(&conn).unwrap_err();
    assert!(
        matches!(err, nerve_store::StoreError::Sqlite(_)),
        "expected the CREATE INDEX to fail, got {err}"
    );

    assert_eq!(
        schema_version(&conn).unwrap(),
        Some(9),
        "a failed step must not record its version"
    );
    assert_eq!(
        memory_tables(&conn),
        before,
        "an interrupted v10 rebuild lost a human's note"
    );
    // And v9's shape is back, not v10's: the free-form scope is accepted again.
    conn.execute(
        "UPDATE memory SET scope = 'file' WHERE memory_id = 'm1'",
        [],
    )
    .expect("the v9 table did not come back");

    // The control: the identical fixture without the sabotage reaches v10.
    let clean = v9_database_with_memory("process", "superseded");
    migrate(&clean).unwrap();
    assert_eq!(schema_version(&clean).unwrap(), Some(SCHEMA_VERSION));
    assert_eq!(scalar(&clean, "SELECT count(*) FROM memory"), 2);
}
