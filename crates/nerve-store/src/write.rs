//! Writes. Everything here inserts facts; nothing here derives state.
//!
//! `assertion_state` is deliberately absent from this module. It is written only by
//! [`crate::derive::rebuild_assertion_state`].

use rusqlite::{params, Connection};

use nerve_core::model::GraphBatch;

use crate::error::Result;

/// Identity and location of the repository being indexed.
#[derive(Debug, Clone)]
pub struct RepositoryRow {
    /// Repository entity id.
    pub repo_id: String,
    /// Stable project identifier from `.nerve/config.toml`.
    pub project_id: String,
    /// Absolute root path at index time. Not part of any identity.
    pub root_path: String,
}

/// A repository state: what the tree looked like when the run happened.
#[derive(Debug, Clone)]
pub struct RepositoryStateRow {
    /// State identifier (the content merkle in Slice 1).
    pub state_id: String,
    /// Owning repository.
    pub repo_id: String,
    /// How the state was derived. Slice 1 always writes `content`.
    pub kind: String,
    /// Resolved git HEAD commit, when `.git` is present.
    pub git_commit: Option<String>,
    /// BLAKE3 merkle over sorted `(rel_path, content_hash)`.
    pub content_merkle: String,
}

/// Insert the repository, or refresh its recorded root path if it moved.
pub fn upsert_repository(conn: &Connection, row: &RepositoryRow) -> Result<()> {
    conn.execute(
        "INSERT INTO repository (repo_id, project_id, root_path, created_at)
         VALUES (?1, ?2, ?3, strftime('%Y-%m-%dT%H:%M:%fZ','now'))
         ON CONFLICT(repo_id) DO UPDATE SET root_path = excluded.root_path",
        params![row.repo_id, row.project_id, row.root_path],
    )?;
    Ok(())
}

/// Insert a repository state. Re-indexing an unchanged tree reuses the existing row.
pub fn upsert_repository_state(conn: &Connection, row: &RepositoryStateRow) -> Result<()> {
    conn.execute(
        "INSERT OR IGNORE INTO repository_state
             (state_id, repo_id, kind, git_commit, content_merkle, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5, strftime('%Y-%m-%dT%H:%M:%fZ','now'))",
        params![
            row.state_id,
            row.repo_id,
            row.kind,
            row.git_commit,
            row.content_merkle
        ],
    )?;
    Ok(())
}

/// Open an extractor run and return its surrogate id.
pub fn begin_extractor_run(
    conn: &Connection,
    repo_id: &str,
    state_id: &str,
    extractor_id: &str,
    extractor_version: &str,
) -> Result<i64> {
    conn.execute(
        "INSERT INTO extractor_run
             (repo_id, state_id, extractor_id, extractor_version, started_at, status)
         VALUES (?1, ?2, ?3, ?4, strftime('%Y-%m-%dT%H:%M:%fZ','now'), 'running')",
        params![repo_id, state_id, extractor_id, extractor_version],
    )?;
    Ok(conn.last_insert_rowid())
}

/// Close an extractor run with its file tallies and terminal status.
pub fn finish_extractor_run(
    conn: &Connection,
    run_id: i64,
    files_processed: i64,
    files_failed: i64,
    status: &str,
) -> Result<()> {
    conn.execute(
        "UPDATE extractor_run
            SET finished_at = strftime('%Y-%m-%dT%H:%M:%fZ','now'),
                files_processed = ?2,
                files_failed = ?3,
                status = ?4
          WHERE run_id = ?1",
        params![run_id, files_processed, files_failed, status],
    )?;
    Ok(())
}

/// Persist one extractor run's entities, occurrences, assertions and observations.
///
/// Every insert is `OR IGNORE` against a content-derived key, which is what makes re-indexing
/// an unchanged tree add no rows.
pub fn persist_batch(
    conn: &Connection,
    repo_id: &str,
    state_id: &str,
    run_id: i64,
    batch: &GraphBatch,
) -> Result<()> {
    {
        let mut stmt = conn.prepare(
            "INSERT OR IGNORE INTO entity
                 (entity_id, repo_id, kind, name, scope_path, language, meta)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        )?;
        for entity in &batch.entities {
            stmt.execute(params![
                entity.entity_id,
                repo_id,
                entity.kind.as_str(),
                entity.name,
                entity.scope_path,
                entity.language,
                entity.meta,
            ])?;
        }
    }

    {
        let mut stmt = conn.prepare(
            "INSERT OR IGNORE INTO occurrence
                 (occurrence_id, entity_id, state_id, file_path, start_byte, end_byte,
                  start_line, start_col, end_line, end_col, content_hash)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
        )?;
        for occurrence in &batch.occurrences {
            stmt.execute(params![
                occurrence.occurrence_id,
                occurrence.entity_id,
                state_id,
                occurrence.file_path,
                occurrence.span.start_byte as i64,
                occurrence.span.end_byte as i64,
                occurrence.span.start_line as i64,
                occurrence.span.start_col as i64,
                occurrence.span.end_line as i64,
                occurrence.span.end_col as i64,
                occurrence.content_hash,
            ])?;
        }
    }

    {
        let mut stmt = conn.prepare(
            "INSERT OR IGNORE INTO assertion
                 (assertion_id, repo_id, source_entity_id, relation, target_entity_id)
             VALUES (?1, ?2, ?3, ?4, ?5)",
        )?;
        for assertion in &batch.assertions {
            stmt.execute(params![
                assertion.assertion_id,
                repo_id,
                assertion.source_entity_id,
                assertion.relation.as_str(),
                assertion.target_entity_id,
            ])?;
        }
    }

    {
        let mut stmt = conn.prepare(
            "INSERT OR IGNORE INTO observation
                 (assertion_id, extractor_run_id, evidence_source_type, directness,
                  extractor_id, extractor_version, match_quality, state_id, file_path,
                  start_line, end_line, content_hash, environment, details, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14,
                     strftime('%Y-%m-%dT%H:%M:%fZ','now'))",
        )?;
        for observation in &batch.observations {
            stmt.execute(params![
                observation.assertion_id,
                run_id,
                observation.evidence_source_type.as_str(),
                observation.directness.as_str(),
                observation.extractor_id,
                observation.extractor_version,
                observation.match_quality,
                state_id,
                observation.file_path,
                observation.start_line as i64,
                observation.end_line as i64,
                observation.content_hash,
                observation.environment,
                observation.details,
            ])?;
        }
    }

    Ok(())
}
