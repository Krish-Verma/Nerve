//! Migration, FTS5 availability, and the derived-state boundary.

use nerve_store::{migrate, open, open_in_memory, schema_version, SCHEMA_VERSION};

#[test]
fn fresh_database_reaches_the_current_schema_version() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("nerve.db");
    let conn = open(&path).unwrap();
    assert_eq!(schema_version(&conn).unwrap(), None);
    migrate(&conn).unwrap();
    assert_eq!(schema_version(&conn).unwrap(), Some(SCHEMA_VERSION));
    assert_eq!(SCHEMA_VERSION, 1);
}

#[test]
fn migrating_twice_is_a_no_op() {
    let conn = open_in_memory().unwrap();
    migrate(&conn).unwrap();
    let first: String = conn
        .query_row(
            "SELECT group_concat(name) FROM sqlite_master WHERE type='table' ORDER BY name",
            [],
            |row| row.get(0),
        )
        .unwrap();

    migrate(&conn).unwrap();
    migrate(&conn).unwrap();

    let rows: i64 = conn
        .query_row("SELECT count(*) FROM schema_version", [], |row| row.get(0))
        .unwrap();
    assert_eq!(rows, 1, "re-migrating must not append a version row");
    assert_eq!(schema_version(&conn).unwrap(), Some(SCHEMA_VERSION));

    let second: String = conn
        .query_row(
            "SELECT group_concat(name) FROM sqlite_master WHERE type='table' ORDER BY name",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(first, second, "schema must be unchanged");
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
