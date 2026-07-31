//! Derivation of `assertion_state`.
//!
//! This module is the **only** writer of `assertion_state`. It performs a pure rebuild:
//! every row is deleted and then recomputed from `observation` (joined to `assertion` and
//! `entity` for the target kind). Nothing incremental, nothing accumulated.
//!
//! The consequence ADR-0003 wants is that a buggy extractor can only add filterable noise at
//! a declared source type, and its contribution is revocable with one
//! `DELETE FROM observation WHERE extractor_id = ?` followed by a rebuild.

use rusqlite::Connection;

use nerve_core::vocab::EvidenceSourceType;

use crate::error::{Result, StoreError};

/// SQL expression mapping `observation.evidence_source_type` to its stable ordinal.
fn ordinal_case(column: &str) -> String {
    let mut sql = format!("CASE {column}");
    for source in EvidenceSourceType::ALL {
        sql.push_str(&format!(
            " WHEN '{}' THEN {}",
            source.as_str(),
            source.ordinal()
        ));
    }
    sql.push_str(" END");
    sql
}

/// SQL expression mapping `observation.evidence_source_type` to its mask bit.
fn mask_case(column: &str) -> String {
    let mut sql = format!("CASE {column}");
    for source in EvidenceSourceType::ALL {
        sql.push_str(&format!(
            " WHEN '{}' THEN {}",
            source.as_str(),
            source.mask_bit()
        ));
    }
    sql.push_str(" END");
    sql
}

/// SQL expression mapping an ordinal back to its source-type name.
fn name_case(expr: &str) -> String {
    let mut sql = format!("CASE ({expr})");
    for source in EvidenceSourceType::ALL {
        sql.push_str(&format!(
            " WHEN {} THEN '{}'",
            source.ordinal(),
            source.as_str()
        ));
    }
    sql.push_str(" END");
    sql
}

/// Reject any observation carrying a source type outside the closed vocabulary.
fn assert_closed_vocabulary(conn: &Connection) -> Result<()> {
    let allowed: Vec<String> = EvidenceSourceType::ALL
        .iter()
        .map(|s| format!("'{}'", s.as_str()))
        .collect();
    let sql = format!(
        "SELECT evidence_source_type FROM observation
          WHERE evidence_source_type NOT IN ({}) LIMIT 1",
        allowed.join(", ")
    );
    let offender: Option<String> = conn
        .query_row(&sql, [], |row| row.get(0))
        .ok()
        .flatten()
        .or(None);
    if let Some(value) = offender {
        return Err(StoreError::Core(nerve_core::NerveError::unknown(
            "EvidenceSourceType",
            value,
        )));
    }
    Ok(())
}

/// Delete and recompute every `assertion_state` row. Returns the number of rows written.
///
/// Semantics of the derived columns:
///
/// - `status` is `UNRESOLVED` when the assertion's target entity is an `Unresolved` entity,
///   `SUPPORTED` otherwise. `CONTRADICTED`, `STALE` and `DELETED` require multiple extractors
///   or incremental indexing and cannot occur in Slice 1.
/// - `strongest_source_type` is the source type with the lowest ordinal among the supporting
///   observations. ADR-0003 is explicit that the vocabulary is not a truth ranking, so this is
///   the **default structural ordering** (declaration order, most syntactically direct first).
///   Query-time evidence policies override it; nothing downstream should read it as "truth".
/// - `source_type_mask` is the bitwise OR of the source-type bits present, computed as
///   `SUM(DISTINCT bit)` — the bits are distinct powers of two, so the sum is the OR.
/// - `state_id` and `last_seen_state_id` both come from the most recently inserted supporting
///   observation. They diverge only once incremental indexing lands.
pub fn rebuild_assertion_state(conn: &Connection) -> Result<usize> {
    assert_closed_vocabulary(conn)?;

    conn.execute("DELETE FROM assertion_state", [])?;

    let ordinal = ordinal_case("o.evidence_source_type");
    let mask = mask_case("o.evidence_source_type");
    let strongest = name_case(&format!("MIN({ordinal})"));

    let sql = format!(
        "INSERT INTO assertion_state (
             assertion_id, state_id, status, strongest_source_type,
             source_type_mask, observation_count, is_unresolved, last_seen_state_id)
         SELECT
             o.assertion_id,
             latest.state_id,
             CASE WHEN target.kind = 'unresolved' THEN 'UNRESOLVED' ELSE 'SUPPORTED' END,
             {strongest},
             SUM(DISTINCT {mask}),
             COUNT(*),
             CASE WHEN target.kind = 'unresolved' THEN 1 ELSE 0 END,
             latest.state_id
         FROM observation o
         JOIN assertion a  ON a.assertion_id = o.assertion_id
         JOIN entity target ON target.entity_id = a.target_entity_id
         JOIN (
             SELECT assertion_id, state_id
               FROM observation
              WHERE observation_id IN (
                    SELECT MAX(observation_id) FROM observation GROUP BY assertion_id)
         ) latest ON latest.assertion_id = o.assertion_id
         GROUP BY o.assertion_id, target.kind, latest.state_id"
    );

    let written = conn.execute(&sql, [])?;
    Ok(written)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generated_case_expressions_cover_every_variant() {
        let ordinal = ordinal_case("x");
        let mask = mask_case("x");
        let name = name_case("y");
        for source in EvidenceSourceType::ALL {
            assert!(ordinal.contains(source.as_str()));
            assert!(mask.contains(source.as_str()));
            assert!(name.contains(source.as_str()));
            assert!(mask.contains(&source.mask_bit().to_string()));
        }
    }
}
