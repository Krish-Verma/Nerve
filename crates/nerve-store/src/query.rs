//! Reads: status aggregates and FTS5 symbol search.

use std::collections::BTreeMap;

use rusqlite::{params, Connection};

use crate::error::Result;
use crate::schema;

/// Columns of `extractor_run`, in the order every query below reads them.
const RUN_COLUMNS: &str = "run_id, state_id, extractor_id, extractor_version, started_at,
                           finished_at, files_processed, files_failed, status";

/// Summary of one extractor run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExtractorRunSummary {
    /// Surrogate run id.
    pub run_id: i64,
    /// Repository state the run observed.
    pub state_id: String,
    /// Extractor identifier.
    pub extractor_id: String,
    /// Extractor version.
    pub extractor_version: String,
    /// Start timestamp.
    pub started_at: String,
    /// Finish timestamp, absent if the run did not complete.
    pub finished_at: Option<String>,
    /// Files parsed successfully.
    pub files_processed: i64,
    /// Files that could not be parsed or were too large or not UTF-8.
    pub files_failed: i64,
    /// Terminal status.
    pub status: String,
}

/// Everything `nerve status` reports.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct StatusReport {
    /// Schema version on disk, absent if the database has never been migrated.
    pub schema_version: Option<i64>,
    /// Stable project identifier.
    pub project_id: Option<String>,
    /// Absolute root path recorded at the last index.
    pub root_path: Option<String>,
    /// State of the most recent run.
    pub state_id: Option<String>,
    /// Git HEAD recorded for that state.
    pub git_commit: Option<String>,
    /// Total entities.
    pub entities_total: i64,
    /// Entity counts keyed by kind.
    pub entities_by_kind: BTreeMap<String, i64>,
    /// Total assertions.
    pub assertions_total: i64,
    /// Assertion counts keyed by relation.
    pub assertions_by_relation: BTreeMap<String, i64>,
    /// Total occurrences.
    pub occurrences_total: i64,
    /// Total observations.
    pub observations_total: i64,
    /// Total derived assertion states.
    pub assertion_states_total: i64,
    /// Entities of kind `unresolved`.
    pub unresolved_entities: i64,
    /// Assertion states flagged `is_unresolved`.
    pub unresolved_assertions: i64,
    /// Most recent extractor run.
    pub last_run: Option<ExtractorRunSummary>,
    /// Every run recorded against the most recent state, oldest first.
    ///
    /// An index now runs more than one extractor, so "the last run" no longer describes what
    /// the graph contains. `extractor_run` is a log, not a snapshot: re-indexing an unchanged
    /// tree appends another set of rows for the same state, and they are all reported.
    pub runs: Vec<ExtractorRunSummary>,
}

impl StatusReport {
    /// A status is healthy when the schema is current and no run for the current state is
    /// still open.
    pub fn is_healthy(&self) -> bool {
        self.schema_version == Some(schema::SCHEMA_VERSION)
            && self
                .last_run
                .as_ref()
                .is_some_and(|run| run.status != "running")
            && self.runs.iter().all(|run| run.status != "running")
    }
}

fn read_run(row: &rusqlite::Row<'_>) -> rusqlite::Result<ExtractorRunSummary> {
    Ok(ExtractorRunSummary {
        run_id: row.get(0)?,
        state_id: row.get(1)?,
        extractor_id: row.get(2)?,
        extractor_version: row.get(3)?,
        started_at: row.get(4)?,
        finished_at: row.get(5)?,
        files_processed: row.get(6)?,
        files_failed: row.get(7)?,
        status: row.get(8)?,
    })
}

fn scalar(conn: &Connection, sql: &str) -> Result<i64> {
    Ok(conn.query_row(sql, [], |row| row.get(0))?)
}

fn grouped(conn: &Connection, sql: &str) -> Result<BTreeMap<String, i64>> {
    let mut stmt = conn.prepare(sql)?;
    let rows = stmt.query_map([], |row| {
        Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?))
    })?;
    let mut map = BTreeMap::new();
    for row in rows {
        let (key, count) = row?;
        map.insert(key, count);
    }
    Ok(map)
}

/// Gather everything `nerve status` needs in one pass.
pub fn status(conn: &Connection) -> Result<StatusReport> {
    let version = schema::schema_version(conn)?;
    if version.is_none() {
        return Ok(StatusReport::default());
    }

    let last_run = conn
        .query_row(
            &format!("SELECT {RUN_COLUMNS} FROM extractor_run ORDER BY run_id DESC LIMIT 1"),
            [],
            read_run,
        )
        .ok();

    let mut runs: Vec<ExtractorRunSummary> = Vec::new();
    if let Some(last) = &last_run {
        let mut stmt = conn.prepare(&format!(
            "SELECT {RUN_COLUMNS} FROM extractor_run WHERE state_id = ?1 ORDER BY run_id"
        ))?;
        let rows = stmt.query_map(params![last.state_id], read_run)?;
        for row in rows {
            runs.push(row?);
        }
    }

    let repository: Option<(String, String)> = conn
        .query_row(
            "SELECT project_id, root_path FROM repository ORDER BY repo_id LIMIT 1",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .ok();

    let git_commit = match &last_run {
        Some(run) => conn
            .query_row(
                "SELECT git_commit FROM repository_state WHERE state_id = ?1",
                params![run.state_id],
                |row| row.get::<_, Option<String>>(0),
            )
            .ok()
            .flatten(),
        None => None,
    };

    Ok(StatusReport {
        schema_version: version,
        project_id: repository.as_ref().map(|r| r.0.clone()),
        root_path: repository.as_ref().map(|r| r.1.clone()),
        state_id: last_run.as_ref().map(|r| r.state_id.clone()),
        git_commit,
        entities_total: scalar(conn, "SELECT count(*) FROM entity")?,
        entities_by_kind: grouped(
            conn,
            "SELECT kind, count(*) FROM entity GROUP BY kind ORDER BY kind",
        )?,
        assertions_total: scalar(conn, "SELECT count(*) FROM assertion")?,
        assertions_by_relation: grouped(
            conn,
            "SELECT relation, count(*) FROM assertion GROUP BY relation ORDER BY relation",
        )?,
        occurrences_total: scalar(conn, "SELECT count(*) FROM occurrence")?,
        observations_total: scalar(conn, "SELECT count(*) FROM observation")?,
        assertion_states_total: scalar(conn, "SELECT count(*) FROM assertion_state")?,
        unresolved_entities: scalar(
            conn,
            "SELECT count(*) FROM entity WHERE kind = 'unresolved'",
        )?,
        unresolved_assertions: scalar(
            conn,
            "SELECT count(*) FROM assertion_state WHERE is_unresolved = 1",
        )?,
        last_run,
        runs,
    })
}

/// Source entities of every `IMPORTS` assertion pointing at `target_entity_id`.
///
/// This is the reverse edge incremental indexing walks. `IMPORTS` is emitted for `import`,
/// `require`, a literal dynamic `import()`, **and** `export ... from`, so a barrel file is on
/// this edge even though it names no imported binding — which is why the re-export closure is
/// reachable from a changed leaf module without re-parsing anything.
///
/// Indexed by `idx_assertion_target`. Sorted, so the walk is deterministic.
pub fn importers_of(conn: &Connection, target_entity_id: &str) -> Result<Vec<String>> {
    let mut stmt = conn.prepare(
        "SELECT DISTINCT source_entity_id FROM assertion
          WHERE target_entity_id = ?1 AND relation = 'IMPORTS'
          ORDER BY source_entity_id",
    )?;
    let rows = stmt.query_map(params![target_entity_id], |row| row.get::<_, String>(0))?;
    let mut out = Vec::new();
    for row in rows {
        out.push(row?);
    }
    Ok(out)
}

/// One search result.
#[derive(Debug, Clone, PartialEq)]
pub struct SearchHit {
    /// Matched entity.
    pub entity_id: String,
    /// Entity kind.
    pub kind: String,
    /// Display name.
    pub name: String,
    /// Scope path.
    pub scope_path: String,
    /// Language tag.
    pub language: Option<String>,
    /// First occurrence path, if the entity has one.
    pub file_path: Option<String>,
    /// First occurrence start line.
    pub start_line: Option<i64>,
    /// First occurrence end line.
    pub end_line: Option<i64>,
    /// BM25 score. Lower is a better match.
    pub score: f64,
}

/// Turn user input into a safe FTS5 MATCH expression.
///
/// Repository content and user queries are untrusted input, so the query is not passed to
/// FTS5 verbatim: it is split into alphanumeric tokens, each quoted as a phrase and given a
/// prefix wildcard, then combined with implicit AND. This makes FTS5 operator characters
/// inert rather than a syntax error or an injection surface.
pub fn build_match_expression(query: &str) -> Option<String> {
    let tokens: Vec<String> = query
        .split(|c: char| !c.is_alphanumeric() && c != '_')
        .filter(|token| !token.is_empty())
        .map(|token| format!("\"{token}\"*"))
        .collect();
    if tokens.is_empty() {
        None
    } else {
        Some(tokens.join(" "))
    }
}

/// FTS5 search over entity name and scope path.
///
/// Returns an empty vector (not an error) when the query contains no usable tokens.
pub fn search_entities(
    conn: &Connection,
    query: &str,
    kind: Option<&str>,
    limit: usize,
) -> Result<Vec<SearchHit>> {
    let Some(expression) = build_match_expression(query) else {
        return Ok(Vec::new());
    };

    let sql = "
        SELECT e.entity_id, e.kind, e.name, e.scope_path, e.language,
               o.file_path, o.start_line, o.end_line, bm25(entity_fts) AS score
          FROM entity_fts
          JOIN entity e ON e.rowid = entity_fts.rowid
          LEFT JOIN occurrence o ON o.occurrence_id = (
               SELECT occurrence_id FROM occurrence
                WHERE entity_id = e.entity_id
                ORDER BY file_path, start_byte, end_byte LIMIT 1)
         WHERE entity_fts MATCH ?1
           AND (?2 IS NULL OR e.kind = ?2)
         ORDER BY score ASC, e.entity_id ASC
         LIMIT ?3";

    let mut stmt = conn.prepare(sql)?;
    let rows = stmt.query_map(params![expression, kind, limit as i64], |row| {
        Ok(SearchHit {
            entity_id: row.get(0)?,
            kind: row.get(1)?,
            name: row.get(2)?,
            scope_path: row.get(3)?,
            language: row.get(4)?,
            file_path: row.get(5)?,
            start_line: row.get(6)?,
            end_line: row.get(7)?,
            score: row.get(8)?,
        })
    })?;

    let mut hits = Vec::new();
    for row in rows {
        hits.push(row?);
    }
    Ok(hits)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn match_expression_quotes_tokens_and_adds_prefix_wildcard() {
        assert_eq!(build_match_expression("area"), Some("\"area\"*".into()));
        assert_eq!(
            build_match_expression("get area"),
            Some("\"get\"* \"area\"*".into())
        );
    }

    #[test]
    fn match_expression_neutralises_fts_operators() {
        // Bare `*`, `"` and `OR` would be FTS5 syntax; none survives tokenization as syntax.
        assert_eq!(build_match_expression("*"), None);
        assert_eq!(build_match_expression("  \"  "), None);
        assert_eq!(
            build_match_expression("a OR b"),
            Some("\"a\"* \"OR\"* \"b\"*".into())
        );
        assert_eq!(
            build_match_expression("x\" OR y:\""),
            Some("\"x\"* \"OR\"* \"y\"*".into())
        );
    }

    #[test]
    fn match_expression_keeps_underscores() {
        assert_eq!(
            build_match_expression("my_thing"),
            Some("\"my_thing\"*".into())
        );
    }
}
