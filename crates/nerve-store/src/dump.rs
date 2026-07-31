//! Reading a [`CanonicalDump`] out of a database.

use rusqlite::Connection;

use nerve_core::dump::{
    CanonicalDump, DumpAssertion, DumpAssertionState, DumpEntity, DumpObservation, DumpOccurrence,
};

use crate::error::{Result, StoreError};
use crate::schema;

fn parse_json(column: &'static str, raw: Option<String>) -> Result<Option<serde_json::Value>> {
    match raw {
        None => Ok(None),
        Some(text) => serde_json::from_str(&text)
            .map(Some)
            .map_err(|source| StoreError::Json { column, source }),
    }
}

/// Read the canonical dump of the graph.
///
/// See [`nerve_core::dump`] for what is deliberately excluded and why.
pub fn canonical_dump(conn: &Connection) -> Result<CanonicalDump> {
    let schema_version = schema::schema_version(conn)?.unwrap_or(0);

    let project_id: String = conn
        .query_row(
            "SELECT project_id FROM repository ORDER BY repo_id LIMIT 1",
            [],
            |row| row.get(0),
        )
        .unwrap_or_default();

    let mut state_ids = Vec::new();
    {
        let mut stmt = conn.prepare("SELECT state_id FROM repository_state ORDER BY state_id")?;
        let rows = stmt.query_map([], |row| row.get::<_, String>(0))?;
        for row in rows {
            state_ids.push(row?);
        }
    }

    let mut entities = Vec::new();
    {
        let mut stmt = conn.prepare(
            "SELECT entity_id, kind, name, scope_path, language, meta
               FROM entity ORDER BY entity_id",
        )?;
        let rows = stmt.query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, Option<String>>(4)?,
                row.get::<_, Option<String>>(5)?,
            ))
        })?;
        for row in rows {
            let (entity_id, kind, name, scope_path, language, meta) = row?;
            entities.push(DumpEntity {
                entity_id,
                kind,
                name,
                scope_path,
                language,
                meta: parse_json("entity.meta", meta)?,
            });
        }
    }

    let mut occurrences = Vec::new();
    {
        let mut stmt = conn.prepare(
            "SELECT entity_id, state_id, file_path, start_byte, end_byte,
                    start_line, start_col, end_line, end_col, content_hash
               FROM occurrence ORDER BY entity_id, file_path, start_byte, end_byte",
        )?;
        let rows = stmt.query_map([], |row| {
            Ok(DumpOccurrence {
                entity_id: row.get(0)?,
                state_id: row.get(1)?,
                file_path: row.get(2)?,
                start_byte: row.get(3)?,
                end_byte: row.get(4)?,
                start_line: row.get(5)?,
                start_col: row.get(6)?,
                end_line: row.get(7)?,
                end_col: row.get(8)?,
                content_hash: row.get(9)?,
            })
        })?;
        for row in rows {
            occurrences.push(row?);
        }
    }

    let mut assertions = Vec::new();
    {
        let mut stmt = conn.prepare(
            "SELECT assertion_id, source_entity_id, relation, target_entity_id
               FROM assertion ORDER BY assertion_id",
        )?;
        let rows = stmt.query_map([], |row| {
            Ok(DumpAssertion {
                assertion_id: row.get(0)?,
                source_entity_id: row.get(1)?,
                relation: row.get(2)?,
                target_entity_id: row.get(3)?,
            })
        })?;
        for row in rows {
            assertions.push(row?);
        }
    }

    let mut observations = Vec::new();
    {
        let mut stmt = conn.prepare(
            "SELECT assertion_id, evidence_source_type, directness, extractor_id,
                    extractor_version, match_quality, state_id, file_path, start_line,
                    end_line, content_hash, environment, details
               FROM observation ORDER BY observation_id",
        )?;
        let rows = stmt.query_map([], |row| {
            Ok((
                DumpObservation {
                    assertion_id: row.get(0)?,
                    evidence_source_type: row.get(1)?,
                    directness: row.get(2)?,
                    extractor_id: row.get(3)?,
                    extractor_version: row.get(4)?,
                    match_quality: row.get(5)?,
                    state_id: row.get(6)?,
                    file_path: row.get(7)?,
                    start_line: row.get(8)?,
                    end_line: row.get(9)?,
                    content_hash: row.get(10)?,
                    environment: row.get(11)?,
                    details: None,
                },
                row.get::<_, Option<String>>(12)?,
            ))
        })?;
        for row in rows {
            let (mut observation, details) = row?;
            observation.details = parse_json("observation.details", details)?;
            observations.push(observation);
        }
    }

    let mut assertion_states = Vec::new();
    {
        let mut stmt = conn.prepare(
            "SELECT assertion_id, state_id, status, strongest_source_type, source_type_mask,
                    observation_count, is_unresolved, last_seen_state_id
               FROM assertion_state ORDER BY assertion_id",
        )?;
        let rows = stmt.query_map([], |row| {
            Ok(DumpAssertionState {
                assertion_id: row.get(0)?,
                state_id: row.get(1)?,
                status: row.get(2)?,
                strongest_source_type: row.get(3)?,
                source_type_mask: row.get(4)?,
                observation_count: row.get(5)?,
                is_unresolved: row.get(6)?,
                last_seen_state_id: row.get(7)?,
            })
        })?;
        for row in rows {
            assertion_states.push(row?);
        }
    }

    let mut dump = CanonicalDump {
        schema_version,
        project_id,
        state_ids,
        entities,
        occurrences,
        assertions,
        observations,
        assertion_states,
    };
    dump.sort();
    Ok(dump)
}
