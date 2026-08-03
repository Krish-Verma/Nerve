//! Read-only facts about the database file itself, for `nerve doctor`.
//!
//! Every judgement — what is fatal, what is merely a warning, what the user should do about it —
//! lives in the CLI. This module only *measures*, and it is here rather than in a surface crate
//! because measuring means SQL, and SQL lives in this crate (ARCHITECTURE.md).
//!
//! It differs from [`crate::query::status`] in one way that is the whole reason it exists:
//! **every fact is individually optional.** `status` assumes the tables it queries are the ones
//! this build wrote. A database `doctor` is pointed at may have been written by an older build,
//! by a newer one, or may be damaged — so a missing or unreadable table degrades that one fact to
//! `None` instead of failing the report. A diagnostic that refuses to run on a broken database is
//! useless precisely when it is needed.

use rusqlite::Connection;

use crate::error::Result;
use crate::schema;

/// How many `PRAGMA integrity_check` rows are asked for.
///
/// SQLite stops looking after this many problems, which keeps the check affordable on a badly
/// damaged file. The count is a reporting bound, not a judgement: one row reading `ok` is a sound
/// database and anything else is not.
const INTEGRITY_ROWS: usize = 5;

/// What the database file says about itself.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DatabaseDiagnostics {
    /// Rows returned by `PRAGMA integrity_check`. Exactly `["ok"]` when the file is sound.
    pub integrity: Vec<String>,
    /// Highest applied schema version. Absent when there is no `schema_version` table.
    pub schema_version: Option<i64>,
    /// Every applied schema version, ascending. Empty when the table is absent or unreadable.
    pub applied_versions: Vec<i64>,
    /// Rows in `entity`. Absent when the table cannot be read.
    pub entities: Option<i64>,
    /// Documents held by the FTS5 index over `entity`. Absent when its shadow table cannot be
    /// read — which is not a failure, only a fact this build could not establish.
    pub fts_documents: Option<i64>,
    /// Rows in `extractor_run`. Absent when the table cannot be read.
    pub extractor_runs: Option<i64>,
    /// Extractor runs with no `finished_at`: an index that started and never reported finishing.
    /// Absent when the table cannot be read.
    pub unfinished_runs: Option<i64>,
    /// The absolute root path recorded for this repository. Absent when there is no row.
    pub repository_root: Option<String>,
    /// The most recent run's finish time, or its start time when it never finished. Absent when
    /// nothing has ever been indexed.
    pub last_run_at: Option<String>,
}

/// Measure everything [`DatabaseDiagnostics`] reports.
///
/// The connection should already be `query_only`; nothing here writes in any case. Only a failing
/// `PRAGMA integrity_check` is propagated as an error, because a database whose integrity cannot
/// even be asked about is not a database this function can describe.
pub fn diagnose(conn: &Connection) -> Result<DatabaseDiagnostics> {
    Ok(DatabaseDiagnostics {
        integrity: integrity_check(conn)?,
        schema_version: schema::schema_version(conn).ok().flatten(),
        applied_versions: applied_versions(conn),
        entities: count(conn, "SELECT count(*) FROM entity"),
        // The FTS5 shadow table, which holds one row per indexed document. `count(*)` on the
        // virtual table itself counts the *content* table instead — it reports the entity count
        // even when the index has drifted from it — so it cannot answer this question.
        fts_documents: count(conn, "SELECT count(*) FROM entity_fts_docsize"),
        extractor_runs: count(conn, "SELECT count(*) FROM extractor_run"),
        unfinished_runs: count(
            conn,
            "SELECT count(*) FROM extractor_run WHERE finished_at IS NULL",
        ),
        repository_root: conn
            .query_row(
                "SELECT root_path FROM repository ORDER BY repo_id LIMIT 1",
                [],
                |row| row.get(0),
            )
            .ok(),
        last_run_at: conn
            .query_row(
                "SELECT coalesce(finished_at, started_at) FROM extractor_run
                 ORDER BY run_id DESC LIMIT 1",
                [],
                |row| row.get(0),
            )
            .ok(),
    })
}

/// `PRAGMA integrity_check`, bounded to [`INTEGRITY_ROWS`] problems.
fn integrity_check(conn: &Connection) -> Result<Vec<String>> {
    let mut stmt = conn.prepare(&format!("PRAGMA integrity_check({INTEGRITY_ROWS})"))?;
    let rows = stmt.query_map([], |row| row.get::<_, String>(0))?;
    let mut out = Vec::new();
    for row in rows {
        out.push(row?);
    }
    Ok(out)
}

/// Every applied schema version, ascending. A gap here means a migration step was skipped.
fn applied_versions(conn: &Connection) -> Vec<i64> {
    let Ok(mut stmt) = conn.prepare("SELECT version FROM schema_version ORDER BY version") else {
        return Vec::new();
    };
    let Ok(rows) = stmt.query_map([], |row| row.get::<_, i64>(0)) else {
        return Vec::new();
    };
    rows.filter_map(|row| row.ok()).collect()
}

/// One count, or `None` when the table is not there to count.
fn count(conn: &Connection, sql: &str) -> Option<i64> {
    conn.query_row(sql, [], |row| row.get(0)).ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::schema::{migrate, SCHEMA_VERSION};

    fn migrated() -> Connection {
        let conn = crate::db::open_in_memory().unwrap();
        migrate(&conn).unwrap();
        conn
    }

    #[test]
    fn a_migrated_database_reports_itself_sound_and_current() {
        let conn = migrated();
        let facts = diagnose(&conn).unwrap();
        assert_eq!(facts.integrity, vec!["ok".to_string()]);
        assert_eq!(facts.schema_version, Some(SCHEMA_VERSION));
        assert_eq!(
            facts.applied_versions,
            (1..=SCHEMA_VERSION).collect::<Vec<_>>()
        );
        assert_eq!(facts.entities, Some(0));
        assert_eq!(facts.fts_documents, Some(0));
        assert_eq!(facts.extractor_runs, Some(0));
        assert_eq!(facts.unfinished_runs, Some(0));
        assert_eq!(facts.repository_root, None);
        assert_eq!(facts.last_run_at, None);
    }

    /// The property the whole module is shaped around: a database with none of Nerve's tables
    /// still produces a report, with the facts it could not establish left absent rather than
    /// guessed at or reported as zero.
    #[test]
    fn a_database_with_no_nerve_tables_still_reports() {
        let conn = crate::db::open_in_memory().unwrap();
        let facts = diagnose(&conn).unwrap();
        assert_eq!(facts.integrity, vec!["ok".to_string()]);
        assert_eq!(facts.schema_version, None);
        assert!(facts.applied_versions.is_empty());
        assert_eq!(facts.entities, None, "absent is not zero");
        assert_eq!(facts.fts_documents, None);
        assert_eq!(facts.unfinished_runs, None);
    }

    #[test]
    fn a_skipped_migration_shows_as_a_gap_in_the_applied_versions() {
        let conn = migrated();
        conn.execute("DELETE FROM schema_version WHERE version = 2", [])
            .unwrap();
        let facts = diagnose(&conn).unwrap();
        assert_eq!(
            facts.schema_version,
            Some(SCHEMA_VERSION),
            "the database still claims the latest version"
        );
        assert!(
            !facts.applied_versions.contains(&2),
            "and the history is what shows the step that never ran: {:?}",
            facts.applied_versions
        );
    }

    /// The FTS5 index is maintained by triggers, and a drift between it and `entity` is what
    /// makes `nerve search` miss rows. It is only visible in the shadow table.
    #[test]
    fn the_full_text_document_count_tracks_the_index_and_not_the_entity_table() {
        let conn = migrated();
        conn.execute(
            "INSERT INTO repository (repo_id, project_id, root_path, created_at)
             VALUES ('r', 'p', '/tmp/r', 'now')",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO entity (entity_id, repo_id, kind, name, scope_path)
             VALUES ('e', 'r', 'Function', 'alpha', 'a.ts#alpha')",
            [],
        )
        .unwrap();
        assert_eq!(diagnose(&conn).unwrap().fts_documents, Some(1));

        conn.execute(
            "INSERT INTO entity_fts(entity_fts, rowid, name, scope_path)
             SELECT 'delete', rowid, name, scope_path FROM entity",
            [],
        )
        .unwrap();
        let facts = diagnose(&conn).unwrap();
        assert_eq!(facts.entities, Some(1));
        assert_eq!(
            facts.fts_documents,
            Some(0),
            "the entity is still there and the full-text index no longer holds it"
        );
    }

    #[test]
    fn an_interrupted_run_is_visible_as_a_run_with_no_finish_time() {
        let conn = migrated();
        conn.execute(
            "INSERT INTO repository (repo_id, project_id, root_path, created_at)
             VALUES ('r', 'p', '/tmp/r', 'now')",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO repository_state (state_id, repo_id, kind, content_merkle, created_at)
             VALUES ('s', 'r', 'working_tree', 'm', 'now')",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO extractor_run
                 (repo_id, state_id, extractor_id, extractor_version, started_at, status)
             VALUES ('r', 's', 'ts-js-structural', '1', '2026-08-02T00:00:00Z', 'running')",
            [],
        )
        .unwrap();
        let facts = diagnose(&conn).unwrap();
        assert_eq!(facts.extractor_runs, Some(1));
        assert_eq!(facts.unfinished_runs, Some(1));
        assert_eq!(
            facts.last_run_at.as_deref(),
            Some("2026-08-02T00:00:00Z"),
            "a run that never finished still dates the attempt"
        );
        assert_eq!(facts.repository_root.as_deref(), Some("/tmp/r"));
    }
}
