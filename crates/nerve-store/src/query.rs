//! Reads: status aggregates and FTS5 symbol search.

use std::collections::BTreeMap;

use rusqlite::{params, Connection};

use crate::error::Result;
use crate::schema;
use crate::select::symbol_kinds_sql;

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
    /// Total entities, of **every** kind — repositories, directories, files, modules, documents,
    /// sections, unresolved references and coverage runs as well as symbols.
    pub entities_total: i64,
    /// Entities that are symbols: functions, methods, classes and interfaces.
    ///
    /// Derived from [`EntityKind::is_symbol`](nerve_core::vocab::EntityKind::is_symbol), so it is
    /// the vocabulary's own answer to "how many symbols does this repository have" and cannot
    /// drift from it. This is the number to show against the word *symbols*;
    /// [`entities_total`](Self::entities_total) counts every kind and is always at least as large.
    pub symbols_total: i64,
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
        // The kind list is generated from the closed compile-time vocabulary, never from caller
        // text, and comes from the one helper so it cannot disagree with the other symbol-only
        // queries in this crate.
        symbols_total: scalar(
            conn,
            &format!(
                "SELECT count(*) FROM entity WHERE kind IN ({})",
                symbol_kinds_sql()
            ),
        )?,
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

/// The single repository row, as recorded by `nerve init`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RepositoryInfo {
    /// Content-derived repository identifier, the scope key of every other table.
    pub repo_id: String,
    /// Stable project identifier from `.nerve/config.toml`.
    pub project_id: String,
    /// Absolute root path recorded at `init`.
    pub root_path: String,
}

/// Read the repository row.
///
/// [`status`] reports `project_id` and `root_path` but not `repo_id`, and `repo_id` is what
/// every per-repository query is scoped by. Callers that need to ask a scoped question — the
/// module cache, the partial-parse list — need it without recomputing it from the config.
pub fn repository(conn: &Connection) -> Result<Option<RepositoryInfo>> {
    Ok(conn
        .query_row(
            "SELECT repo_id, project_id, root_path FROM repository ORDER BY repo_id LIMIT 1",
            [],
            |row| {
                Ok(RepositoryInfo {
                    repo_id: row.get(0)?,
                    project_id: row.get(1)?,
                    root_path: row.get(2)?,
                })
            },
        )
        .ok())
}

/// One recorded occurrence of an entity.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OccurrenceRow {
    /// Content-derived occurrence identifier.
    pub occurrence_id: String,
    /// Repository-relative path.
    pub file_path: String,
    /// First byte of the span.
    pub start_byte: i64,
    /// One past the last byte of the span.
    pub end_byte: i64,
    /// First line of the span, 1-based.
    pub start_line: i64,
    /// Column of the first byte, 0-based.
    pub start_col: i64,
    /// Last line of the span, 1-based.
    pub end_line: i64,
    /// Column one past the last byte, 0-based.
    pub end_col: i64,
    /// Content hash of the file when the occurrence was recorded.
    pub content_hash: String,
}

/// Every occurrence of one entity, in a stable order.
pub fn occurrences_of(conn: &Connection, entity_id: &str) -> Result<Vec<OccurrenceRow>> {
    let mut stmt = conn.prepare(
        "SELECT occurrence_id, file_path, start_byte, end_byte, start_line, start_col,
                end_line, end_col, content_hash
           FROM occurrence
          WHERE entity_id = ?1
          ORDER BY file_path, start_byte, end_byte, occurrence_id",
    )?;
    let rows = stmt.query_map(params![entity_id], |row| {
        Ok(OccurrenceRow {
            occurrence_id: row.get(0)?,
            file_path: row.get(1)?,
            start_byte: row.get(2)?,
            end_byte: row.get(3)?,
            start_line: row.get(4)?,
            start_col: row.get(5)?,
            end_line: row.get(6)?,
            end_col: row.get(7)?,
            content_hash: row.get(8)?,
        })
    })?;
    let mut out = Vec::new();
    for row in rows {
        out.push(row?);
    }
    Ok(out)
}

/// How many assertions of each relation touch one entity, per side.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct EntityRelationCounts {
    /// Relations where the entity is the assertion's source.
    pub outgoing: BTreeMap<String, i64>,
    /// Relations where the entity is the assertion's target.
    pub incoming: BTreeMap<String, i64>,
}

/// Assertion counts around one entity, keyed by relation.
///
/// This is what lets a surface say "12 outgoing `CALLS`" without loading twelve edges.
pub fn entity_relation_counts(conn: &Connection, entity_id: &str) -> Result<EntityRelationCounts> {
    let mut counts = EntityRelationCounts::default();
    for (side, column) in [
        ("outgoing", "source_entity_id"),
        ("incoming", "target_entity_id"),
    ] {
        // `column` is one of two literals chosen here, never caller text.
        let sql = format!(
            "SELECT relation, count(*) FROM assertion
              WHERE {column} = ?1 GROUP BY relation ORDER BY relation"
        );
        let mut stmt = conn.prepare(&sql)?;
        let rows = stmt.query_map(params![entity_id], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?))
        })?;
        let target = if side == "outgoing" {
            &mut counts.outgoing
        } else {
            &mut counts.incoming
        };
        for row in rows {
            let (relation, count) = row?;
            target.insert(relation, count);
        }
    }
    Ok(counts)
}

/// One reference target Nerve could not resolve.
///
/// Invariant 4: unresolved is a value, not an omission. This is the list that makes it visible.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnresolvedEntity {
    /// Entity identifier.
    pub entity_id: String,
    /// Display name — the specifier or binding that did not resolve.
    pub name: String,
    /// Where the failed reference was written.
    pub scope_path: String,
    /// Entity `meta` as stored, if any.
    pub meta: Option<String>,
    /// How many assertions point at this unresolved target.
    pub referencing_assertions: i64,
}

/// Entities of kind `unresolved`, most-referenced first, then by name.
pub fn unresolved_entities(
    conn: &Connection,
    limit: usize,
    offset: usize,
) -> Result<Vec<UnresolvedEntity>> {
    let mut stmt = conn.prepare(
        "SELECT e.entity_id, e.name, e.scope_path, e.meta,
                (SELECT count(*) FROM assertion a WHERE a.target_entity_id = e.entity_id)
                    AS referencing
           FROM entity e
          WHERE e.kind = 'unresolved'
          ORDER BY referencing DESC, e.scope_path, e.name, e.entity_id
          LIMIT ?1 OFFSET ?2",
    )?;
    let rows = stmt.query_map(params![limit as i64, offset as i64], |row| {
        Ok(UnresolvedEntity {
            entity_id: row.get(0)?,
            name: row.get(1)?,
            scope_path: row.get(2)?,
            meta: row.get(3)?,
            referencing_assertions: row.get(4)?,
        })
    })?;
    let mut out = Vec::new();
    for row in rows {
        out.push(row?);
    }
    Ok(out)
}

/// Where one symbol was recorded to live, as the index recorded it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SymbolSpanRow {
    /// The symbol.
    pub entity_id: String,
    /// Its kind. Always one for which
    /// [`EntityKind::is_symbol`](nerve_core::vocab::EntityKind::is_symbol) holds.
    pub kind: String,
    /// Inclusive start byte.
    pub start_byte: i64,
    /// Exclusive end byte.
    pub end_byte: i64,
    /// 1-based first line.
    pub start_line: i64,
    /// 1-based last line.
    pub end_line: i64,
}

/// Every **symbol** occurrence recorded in one file, in a stable order.
///
/// Files, modules, documents and sections also have occurrences in a file; none of them is a
/// symbol, and a coverage edge to a `Module` would say "the test suite covers this file", which
/// is a different and weaker claim. The kind filter comes from [`symbol_kinds_sql`], so it is
/// generated from the closed vocabulary rather than written out here.
pub fn symbol_spans_in_file(conn: &Connection, file_path: &str) -> Result<Vec<SymbolSpanRow>> {
    let sql = format!(
        "SELECT o.entity_id, e.kind, o.start_byte, o.end_byte, o.start_line, o.end_line
           FROM occurrence o
           JOIN entity e ON e.entity_id = o.entity_id
          WHERE o.file_path = ?1 AND e.kind IN ({})
          ORDER BY o.entity_id, o.start_byte, o.end_byte",
        symbol_kinds_sql()
    );
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map(params![file_path], |row| {
        Ok(SymbolSpanRow {
            entity_id: row.get(0)?,
            kind: row.get(1)?,
            start_byte: row.get(2)?,
            end_byte: row.get(3)?,
            start_line: row.get(4)?,
            end_line: row.get(5)?,
        })
    })?;
    let mut out = Vec::new();
    for row in rows {
        out.push(row?);
    }
    Ok(out)
}

/// The content hash every occurrence in `file_path` was recorded at.
///
/// `None` when the file has no occurrences — it was never indexed — and `None` when its
/// occurrences disagree, which means the rows did not come from one index run and nothing here
/// may pick a winner between them.
pub fn indexed_content_hash(conn: &Connection, file_path: &str) -> Result<Option<String>> {
    let mut stmt = conn.prepare(
        "SELECT DISTINCT content_hash FROM occurrence WHERE file_path = ?1 ORDER BY content_hash",
    )?;
    let rows = stmt.query_map(params![file_path], |row| row.get::<_, String>(0))?;
    let mut hashes = Vec::new();
    for row in rows {
        hashes.push(row?);
    }
    match hashes.len() {
        1 => Ok(hashes.pop()),
        _ => Ok(None),
    }
}

/// Whether any occurrence was recorded at `file_path`.
///
/// The source endpoint serves by **indexed path only** (THREAT-MODEL T6). This is that check;
/// it is a necessary condition, never a sufficient one — the path is still resolved through the
/// repository path guard afterwards, because the database is a file on disk and not a trusted
/// channel.
pub fn path_is_indexed(conn: &Connection, file_path: &str) -> Result<bool> {
    let found: i64 = conn.query_row(
        "SELECT EXISTS (SELECT 1 FROM occurrence WHERE file_path = ?1)",
        params![file_path],
        |row| row.get(0),
    )?;
    Ok(found != 0)
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
