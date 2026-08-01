//! Derivation of `assertion_state`.
//!
//! This module is the **only** writer of `assertion_state`. `assertion_state` is a *pure
//! function of `observation`* (joined to `assertion` and `entity` for the target kind).
//!
//! The consequence ADR-0003 wants is that a buggy extractor can only add filterable noise at
//! a declared source type, and its contribution is revocable with one
//! `DELETE FROM observation WHERE extractor_id = ?` followed by a rebuild.
//!
//! # Two evaluations of one function
//!
//! [`rebuild_assertion_state`] deletes every row and recomputes the whole table. It is the
//! **reference implementation and the test oracle**, and it is what runs whenever a run
//! re-extracts the entire repository.
//!
//! [`derive_assertion_state_for`] recomputes only a named set of assertions. **Purity is not
//! violated by lazy evaluation** — a pure function may be evaluated on demand — but equality has
//! to be earned, not assumed, so:
//!
//! - The scoped path is exact because a row of `assertion_state` depends on nothing outside its
//!   own assertion: on that assertion's observations, and on the *kind* of its target entity.
//!   Entity kind is a function of entity id (every ADR-0002 canonical tuple begins with the kind
//!   and every id carries a kind prefix), and `assertion_id` is a function of the target, so the
//!   target kind cannot move under a fixed assertion. ADR-0006 §6 records this as a standing
//!   invariant, because the dependency is not local to this file.
//! - It is gated by tests asserting `scoped(edits) == rebuild()` after arbitrary edit sequences,
//!   in this module for a synthetic graph and in `nerve-index/tests/incremental.rs` for real
//!   ones. If they ever disagree, the scoped path is wrong and the full one stands.

use std::collections::BTreeSet;

use rusqlite::{params, Connection};

use nerve_core::vocab::EvidenceSourceType;

use crate::error::{Result, StoreError};

/// Rows one derivation removed and wrote.
///
/// Both halves are counted because both are work: an evaluation that deletes the whole table and
/// rewrites it has done twice the writing of one that touches nothing, and a metric that hid the
/// deletes would report the whole-table rebuild as cheaper than it is.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct DerivationCounts {
    /// Derived rows deleted.
    pub deleted: usize,
    /// Derived rows written.
    pub written: usize,
}

impl DerivationCounts {
    /// Rows touched in total.
    pub fn total(self) -> usize {
        self.deleted + self.written
    }
}

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
///
/// `scoped` restricts the check to the assertions being recomputed, which is the only part of
/// the table the caller is about to read anyway.
fn assert_closed_vocabulary(conn: &Connection, scoped: bool) -> Result<()> {
    let allowed: Vec<String> = EvidenceSourceType::ALL
        .iter()
        .map(|s| format!("'{}'", s.as_str()))
        .collect();
    let restriction = if scoped {
        " AND assertion_id IN (SELECT assertion_id FROM scoped_assertion)"
    } else {
        ""
    };
    let sql = format!(
        "SELECT evidence_source_type FROM observation
          WHERE evidence_source_type NOT IN ({}){restriction} LIMIT 1",
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

/// The `INSERT ... SELECT` that computes derived state, optionally restricted to a scope.
///
/// One expression, used by both evaluations, so the two cannot drift apart by being two
/// implementations of the same rule.
fn insert_statement(restriction: &str) -> String {
    let ordinal = ordinal_case("o.evidence_source_type");
    let mask = mask_case("o.evidence_source_type");
    let strongest = name_case(&format!("MIN({ordinal})"));
    format!(
        "INSERT INTO assertion_state (
             assertion_id, status, strongest_source_type,
             source_type_mask, observation_count, is_unresolved)
         SELECT
             o.assertion_id,
             CASE WHEN target.kind = 'unresolved' THEN 'UNRESOLVED' ELSE 'SUPPORTED' END,
             {strongest},
             SUM(DISTINCT {mask}),
             COUNT(*),
             CASE WHEN target.kind = 'unresolved' THEN 1 ELSE 0 END
         FROM observation o
         JOIN assertion a  ON a.assertion_id = o.assertion_id
         JOIN entity target ON target.entity_id = a.target_entity_id
         {restriction}
         GROUP BY o.assertion_id, target.kind"
    )
}

/// Stage a set of assertion ids in the temporary table the scoped statements read.
fn stage_scope(conn: &Connection, assertion_ids: &BTreeSet<String>) -> Result<()> {
    conn.execute_batch(
        "CREATE TEMP TABLE IF NOT EXISTS scoped_assertion (assertion_id TEXT PRIMARY KEY);
         DELETE FROM scoped_assertion;",
    )?;
    let mut insert =
        conn.prepare("INSERT OR IGNORE INTO scoped_assertion (assertion_id) VALUES (?1)")?;
    for assertion_id in assertion_ids {
        insert.execute(params![assertion_id])?;
    }
    Ok(())
}

/// Recompute `assertion_state` for exactly `assertion_ids`, leaving every other row alone.
///
/// The scope must contain every assertion whose observation set this transaction changed —
/// inserted or deleted. An assertion in the scope that no longer has any observation loses its
/// derived row, which is what makes orphan pruning safe afterwards.
///
/// Including an assertion that did not actually change is harmless: recomputing it yields the
/// same row. Omitting one that did change is a bug, and it is what the
/// `scoped == rebuild` gates exist to catch.
///
pub fn derive_assertion_state_for(
    conn: &Connection,
    assertion_ids: &BTreeSet<String>,
) -> Result<DerivationCounts> {
    if assertion_ids.is_empty() {
        return Ok(DerivationCounts::default());
    }
    stage_scope(conn, assertion_ids)?;
    assert_closed_vocabulary(conn, true)?;

    let deleted = conn.execute(
        "DELETE FROM assertion_state
          WHERE assertion_id IN (SELECT assertion_id FROM scoped_assertion)",
        [],
    )?;
    let written = conn.execute(
        &insert_statement("WHERE o.assertion_id IN (SELECT assertion_id FROM scoped_assertion)"),
        [],
    )?;
    conn.execute_batch("DELETE FROM scoped_assertion;")?;
    Ok(DerivationCounts { deleted, written })
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
///
/// Since ADR-0006 no state is recorded here. `assertion_state.state_id` and `last_seen_state_id`
/// were derived from `observation.state_id`, so keeping them would have meant rewriting every
/// derived row on every index run — the whole-repository write ADR-0006 removes. Which state a
/// claim was last observed in is still answerable, by joining `observation` to `extractor_run`.
pub fn rebuild_assertion_state(conn: &Connection) -> Result<DerivationCounts> {
    assert_closed_vocabulary(conn, false)?;
    let deleted = conn.execute("DELETE FROM assertion_state", [])?;
    let written = conn.execute(&insert_statement(""), [])?;
    Ok(DerivationCounts { deleted, written })
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::db::open_in_memory;
    use crate::schema::migrate;

    /// Every derived row, rendered so two derivations can be compared as text.
    fn snapshot(conn: &Connection) -> Vec<String> {
        let mut stmt = conn
            .prepare(
                "SELECT assertion_id || '|' || status || '|' || strongest_source_type || '|' ||
                        source_type_mask || '|' || observation_count || '|' || is_unresolved
                   FROM assertion_state ORDER BY assertion_id",
            )
            .unwrap();
        let rows = stmt.query_map([], |row| row.get::<_, String>(0)).unwrap();
        rows.map(|row| row.unwrap()).collect()
    }

    fn observe(conn: &Connection, assertion_id: &str, source_type: &str, line: i64) {
        conn.execute(
            "INSERT OR IGNORE INTO observation
                 (assertion_id, extractor_run_id, evidence_source_type, directness,
                  extractor_id, extractor_version, file_path, start_line, end_line,
                  content_hash, created_at)
             VALUES (?1, 1, ?2, 'DIRECT', 'x', '1', 'a.ts', ?3, ?3, 'h', 't')",
            params![assertion_id, source_type, line],
        )
        .unwrap();
    }

    /// A graph with an unresolved target, two source types on one claim, and a claim that is
    /// about to lose its only evidence.
    fn fixture() -> Connection {
        let conn = open_in_memory().unwrap();
        migrate(&conn).unwrap();
        conn.execute_batch(
            "INSERT INTO repository VALUES ('r','p','/tmp','t');
             INSERT INTO repository_state VALUES ('s','r','content',NULL,'m','t');
             INSERT INTO extractor_run VALUES (1,'r','s','x','1','t','t',1,0,'complete');
             INSERT INTO entity VALUES ('e1','r','function','a','',NULL,NULL);
             INSERT INTO entity VALUES ('e2','r','function','b','',NULL,NULL);
             INSERT INTO entity VALUES ('e3','r','unresolved','c','',NULL,NULL);
             INSERT INTO assertion VALUES ('a1','r','e1','CALLS','e2');
             INSERT INTO assertion VALUES ('a2','r','e1','IMPORTS','e3');
             INSERT INTO assertion VALUES ('a3','r','e2','CALLS','e1');",
        )
        .unwrap();
        observe(&conn, "a1", "AST_DIRECT", 1);
        observe(&conn, "a1", "AST_RESOLVED", 2);
        observe(&conn, "a2", "AST_DIRECT", 3);
        observe(&conn, "a3", "AST_RESOLVED", 4);
        rebuild_assertion_state(&conn).unwrap();
        conn
    }

    /// The gate ADR-0003 purity depends on: lazy evaluation must give the same answer.
    #[test]
    fn scoped_derivation_equals_the_whole_table_rebuild() {
        let conn = fixture();

        // Mutate two claims in opposite directions: one gains evidence, one loses all of it.
        observe(&conn, "a3", "AST_DIRECT", 9);
        conn.execute("DELETE FROM observation WHERE assertion_id = 'a2'", [])
            .unwrap();

        let scope: BTreeSet<String> = ["a2".to_string(), "a3".to_string()].into_iter().collect();
        derive_assertion_state_for(&conn, &scope).unwrap();
        let scoped = snapshot(&conn);

        rebuild_assertion_state(&conn).unwrap();
        assert_eq!(scoped, snapshot(&conn), "scoped derivation != full rebuild");

        // The claim that lost its evidence must have lost its derived row, or orphan pruning
        // would trip over a foreign key that still points at it.
        assert!(!scoped.iter().any(|row| row.starts_with("a2|")));
        // The untouched claim was not rewritten and is still correct.
        assert!(scoped.iter().any(|row| row.starts_with("a1|")));
    }

    #[test]
    fn an_empty_scope_writes_nothing_and_disturbs_nothing() {
        let conn = fixture();
        let before = snapshot(&conn);
        assert_eq!(
            derive_assertion_state_for(&conn, &BTreeSet::new())
                .unwrap()
                .total(),
            0
        );
        assert_eq!(snapshot(&conn), before);
    }

    /// A source type outside the closed vocabulary must be refused by both evaluations.
    #[test]
    fn both_evaluations_reject_a_source_type_outside_the_vocabulary() {
        let conn = fixture();
        conn.execute(
            "UPDATE observation SET evidence_source_type = 'MADE_UP' WHERE assertion_id = 'a3'",
            [],
        )
        .unwrap();
        assert!(rebuild_assertion_state(&conn).is_err());
        let scope: BTreeSet<String> = ["a3".to_string()].into_iter().collect();
        assert!(derive_assertion_state_for(&conn, &scope).is_err());
    }

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
