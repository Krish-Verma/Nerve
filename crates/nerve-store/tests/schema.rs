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

/// Build a database as a v1 build would have left it, with rows in it.
fn v1_database_with_rows() -> nerve_store::Connection {
    let conn = open_in_memory().unwrap();
    conn.execute_batch(V1_ONLY).unwrap();
    conn.execute_batch(
        "INSERT INTO repository VALUES ('r','p','/tmp','t');
         INSERT INTO repository_state VALUES ('s','r','content',NULL,'m','t');
         INSERT INTO entity VALUES ('e1','r','function','add','',NULL,NULL);
         INSERT INTO entity VALUES ('e2','r','module','math','src/math.ts',NULL,NULL);
         INSERT INTO assertion VALUES ('a1','r','e2','DEFINES','e1');
         INSERT INTO extractor_run
             VALUES (1,'r','s','ts-js-structural','1.1.0','t','t',1,0,'complete');
         INSERT INTO occurrence
             VALUES ('o1','e1','s','src/math.ts',0,10,1,0,1,10,'h');
         INSERT INTO observation
             VALUES (1,'a1',1,'AST_DIRECT','DIRECT','ts-js-structural','1.1.0',NULL,'s',
                     'src/math.ts',1,1,'h',NULL,NULL,'t');
         INSERT INTO assertion_state VALUES ('a1','s','SUPPORTED','AST_DIRECT',1,1,0,'s');",
    )
    .unwrap();
    conn
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
    assert_eq!(SCHEMA_VERSION, 2);
    // A fresh database reaches v2 directly, without a v1 database ever existing.
    assert!(table_names(&conn).contains(&"module_facts".to_string()));
}

/// A database written by the Slice 1/2 build must upgrade in place, keeping every row.
#[test]
fn a_v1_database_upgrades_to_v2_without_losing_rows() {
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

    // The upgrade is additive: v1's own definitions are untouched.
    let entity_sql: String = conn
        .query_row(
            "SELECT sql FROM sqlite_master WHERE type='table' AND name='entity'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert!(entity_sql.contains("scope_path"));
    assert!(!entity_sql.contains("module_facts"));
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
