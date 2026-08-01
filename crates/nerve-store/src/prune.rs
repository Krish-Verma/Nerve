//! Removal of graph rows.
//!
//! This is the first destructive code in the product, so the rules it implements are written
//! down rather than implied:
//!
//! 1. **Evidence is removed, never rewritten.** When a file vanishes or is re-extracted, its
//!    observations and occurrences are deleted. Nothing edits what an observation claimed.
//! 2. **Entities and assertions are removed only once they are unsupported.** An assertion with
//!    no observation is not a claim; an entity with no occurrence and no incident assertion is
//!    not in the graph. Both are deleted rather than left dangling, because a graph that only
//!    grows is a graph that is wrong after the first deletion.
//! 3. **`assertion_state` is not touched here.** It is derived by [`crate::derive`] before
//!    orphan pruning runs, which is what makes the pruning safe under the foreign key from
//!    `assertion_state` to `assertion`: after derivation, an assertion with no observation has
//!    no derived row to violate.
//!
//! Nothing here edits an existing row. Slice 3's `restamp_state`, which advanced the repository
//! state recorded on every surviving row, is **gone**: since ADR-0006 no graph row carries a
//! state, so there is nothing to advance. That pass was 1330 ms of a 2900 ms incremental run on
//! a 520-module repository, and it was proportional to repository size rather than to the change.
//!
//! # Scoped and whole-table pruning
//!
//! [`prune_orphans`] is the reference implementation: two whole-table anti-joins.
//! [`prune_orphans_scoped`] restricts both to the rows this transaction could possibly have
//! orphaned, recorded in a [`TouchedRows`] as the deletions happen. The candidate set is complete
//! by construction:
//!
//! - an assertion can only lose its last observation if one of its observations was deleted, and
//!   every such deletion goes through this module;
//! - an entity can only lose its last occurrence if one of its occurrences was deleted (likewise),
//!   and can only lose its last incident assertion if that assertion was deleted here, whose
//!   endpoints are collected before it goes;
//! - deleting an entity orphans nothing further, because no entity references another entity.
//!
//! That argument is checked rather than trusted: `nerve-index/tests/incremental.rs` runs the full
//! pruner immediately after the scoped one and asserts it finds nothing left to remove.

use std::collections::BTreeSet;

use rusqlite::{params, Connection};

use crate::error::Result;

/// What a maintenance pass removed. Reported by `nerve index`; silent deletion is not acceptable.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct RemovalCounts {
    /// Observations deleted.
    pub observations: usize,
    /// Occurrences deleted.
    pub occurrences: usize,
    /// Assertions deleted because nothing observed them any more.
    pub assertions: usize,
    /// Entities deleted because nothing located or referenced them any more.
    pub entities: usize,
}

impl RemovalCounts {
    /// True when nothing at all was removed.
    pub fn is_empty(self) -> bool {
        self.observations == 0
            && self.occurrences == 0
            && self.assertions == 0
            && self.entities == 0
    }

    /// Accumulate another pass's counts.
    pub fn add(&mut self, other: RemovalCounts) {
        self.observations += other.observations;
        self.occurrences += other.occurrences;
        self.assertions += other.assertions;
        self.entities += other.entities;
    }
}

/// What a transaction disturbed, and therefore what it might have orphaned or restated.
///
/// Recorded as the deletions happen, because afterwards the rows that would say so are gone.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TouchedRows {
    /// Assertions whose set of supporting observations changed — gained or lost.
    pub assertions: BTreeSet<String>,
    /// Entities that lost an occurrence.
    pub entities: BTreeSet<String>,
}

impl TouchedRows {
    /// Record that this transaction wrote or withdrew evidence for `assertion_id`.
    pub fn touch_assertion(&mut self, assertion_id: &str) {
        self.assertions.insert(assertion_id.to_string());
    }

    /// True when nothing was disturbed at all.
    pub fn is_empty(&self) -> bool {
        self.assertions.is_empty() && self.entities.is_empty()
    }
}

/// Delete every observation and occurrence recorded against any of `file_paths`.
///
/// Used for files that vanished from the tree and for files about to be re-extracted. Entities
/// and assertions are left alone here: whether they survive depends on the rows the same
/// transaction is about to write, so that decision belongs to [`prune_orphans`].
///
/// The assertions and entities the deleted rows belonged to are recorded in `touched` **before**
/// the deletes, because that is the last moment at which they are knowable.
pub fn delete_file_rows(
    conn: &Connection,
    file_paths: &BTreeSet<String>,
    touched: &mut TouchedRows,
) -> Result<RemovalCounts> {
    let mut counts = RemovalCounts::default();
    if file_paths.is_empty() {
        return Ok(counts);
    }

    let mut doomed_assertions =
        conn.prepare("SELECT DISTINCT assertion_id FROM observation WHERE file_path = ?1")?;
    let mut doomed_entities =
        conn.prepare("SELECT DISTINCT entity_id FROM occurrence WHERE file_path = ?1")?;
    let mut observations = conn.prepare("DELETE FROM observation WHERE file_path = ?1")?;
    let mut occurrences = conn.prepare("DELETE FROM occurrence WHERE file_path = ?1")?;

    for path in file_paths {
        collect_into(&mut doomed_assertions, path, &mut touched.assertions)?;
        collect_into(&mut doomed_entities, path, &mut touched.entities)?;
        counts.observations += observations.execute(params![path])?;
        counts.occurrences += occurrences.execute(params![path])?;
    }
    Ok(counts)
}

fn collect_into(
    stmt: &mut rusqlite::Statement<'_>,
    path: &str,
    sink: &mut BTreeSet<String>,
) -> Result<()> {
    let rows = stmt.query_map(params![path], |row| row.get::<_, String>(0))?;
    for row in rows {
        sink.insert(row?);
    }
    Ok(())
}

/// Delete the observations behind `CONTAINS` edges whose target is a directory.
///
/// Directory containment is the one part of the graph not attributable to a file path: the
/// evidence for `src CONTAINS src/lib` is the directory itself. It is re-derived from the
/// current file set on every run — which costs no parsing — so the previous run's rows are
/// cleared and a directory that no longer holds indexed files simply does not come back.
///
/// Callers should invoke this only when a file was **removed**, because that is the only way a
/// directory can stop holding indexed files. Adding or editing files cannot retire a directory,
/// and re-deriving the rows unconditionally would make an unrelated edit rewrite one row per
/// directory in the repository — a whole-repository write for a one-file change.
pub fn delete_directory_containment(conn: &Connection, touched: &mut TouchedRows) -> Result<usize> {
    const DIRECTORY_CONTAINMENT: &str = "SELECT a.assertion_id
              FROM assertion a
              JOIN entity t ON t.entity_id = a.target_entity_id
             WHERE a.relation = 'CONTAINS' AND t.kind = 'directory'";
    {
        let mut stmt = conn.prepare(DIRECTORY_CONTAINMENT)?;
        let rows = stmt.query_map([], |row| row.get::<_, String>(0))?;
        for row in rows {
            touched.assertions.insert(row?);
        }
    }
    Ok(conn.execute(
        &format!("DELETE FROM observation WHERE assertion_id IN ({DIRECTORY_CONTAINMENT})"),
        [],
    )?)
}

/// SQL predicate for an assertion nothing observes any more.
const UNSUPPORTED_ASSERTION: &str =
    "NOT EXISTS (SELECT 1 FROM observation o WHERE o.assertion_id = assertion.assertion_id)";

/// SQL predicate for an entity nothing locates or references any more.
///
/// The repository entity is exempt. A tree with no indexed files still has a repository, and a
/// from-scratch index emits it unconditionally; pruning it would make an emptied repository
/// differ from a freshly indexed empty one.
const ORPHAN_ENTITY: &str = "kind <> 'repository'
       AND NOT EXISTS (SELECT 1 FROM occurrence o WHERE o.entity_id = entity.entity_id)
       AND NOT EXISTS (SELECT 1 FROM assertion a WHERE a.source_entity_id = entity.entity_id)
       AND NOT EXISTS (SELECT 1 FROM assertion a WHERE a.target_entity_id = entity.entity_id)";

/// Delete assertions no observation supports, then entities nothing locates or references.
///
/// Whole-table. This is the reference implementation and the oracle
/// [`prune_orphans_scoped`] is checked against.
///
/// Must run **after** derivation: `assertion_state` is repopulated only from surviving
/// observations, so the assertions this function deletes provably have no derived row pointing
/// at them.
pub fn prune_orphans(conn: &Connection) -> Result<RemovalCounts> {
    let assertions = conn.execute(
        &format!("DELETE FROM assertion WHERE {UNSUPPORTED_ASSERTION}"),
        [],
    )?;
    let entities = conn.execute(&format!("DELETE FROM entity WHERE {ORPHAN_ENTITY}"), [])?;
    Ok(RemovalCounts {
        observations: 0,
        occurrences: 0,
        assertions,
        entities,
    })
}

/// [`prune_orphans`], restricted to the rows this transaction could have orphaned.
///
/// Same two rules, same order; only the candidate set is narrowed. See the module documentation
/// for why the narrowing is complete, and `nerve-index/tests/incremental.rs` for the gate that
/// runs the whole-table pruner afterwards and asserts it finds nothing.
pub fn prune_orphans_scoped(conn: &Connection, touched: &TouchedRows) -> Result<RemovalCounts> {
    if touched.is_empty() {
        return Ok(RemovalCounts::default());
    }

    conn.execute_batch(
        "CREATE TEMP TABLE IF NOT EXISTS prune_assertion (assertion_id TEXT PRIMARY KEY);
         CREATE TEMP TABLE IF NOT EXISTS prune_entity (entity_id TEXT PRIMARY KEY);
         DELETE FROM prune_assertion;
         DELETE FROM prune_entity;",
    )?;
    {
        let mut insert =
            conn.prepare("INSERT OR IGNORE INTO prune_assertion (assertion_id) VALUES (?1)")?;
        for assertion_id in &touched.assertions {
            insert.execute(params![assertion_id])?;
        }
        let mut insert =
            conn.prepare("INSERT OR IGNORE INTO prune_entity (entity_id) VALUES (?1)")?;
        for entity_id in &touched.entities {
            insert.execute(params![entity_id])?;
        }
    }

    // An assertion that is about to go takes its endpoints into the entity candidate set: losing
    // an incident assertion is the other way an entity becomes an orphan.
    {
        let mut stmt = conn.prepare(&format!(
            "INSERT OR IGNORE INTO prune_entity (entity_id)
             SELECT source_entity_id FROM assertion
              WHERE assertion_id IN (SELECT assertion_id FROM prune_assertion)
                AND {UNSUPPORTED_ASSERTION}
             UNION
             SELECT target_entity_id FROM assertion
              WHERE assertion_id IN (SELECT assertion_id FROM prune_assertion)
                AND {UNSUPPORTED_ASSERTION}"
        ))?;
        stmt.execute([])?;
    }

    let assertions = conn.execute(
        &format!(
            "DELETE FROM assertion
              WHERE assertion_id IN (SELECT assertion_id FROM prune_assertion)
                AND {UNSUPPORTED_ASSERTION}"
        ),
        [],
    )?;
    let entities = conn.execute(
        &format!(
            "DELETE FROM entity
              WHERE entity_id IN (SELECT entity_id FROM prune_entity)
                AND {ORPHAN_ENTITY}"
        ),
        [],
    )?;

    conn.execute_batch("DELETE FROM prune_assertion; DELETE FROM prune_entity;")?;

    Ok(RemovalCounts {
        observations: 0,
        occurrences: 0,
        assertions,
        entities,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn removal_counts_accumulate() {
        let mut total = RemovalCounts::default();
        assert!(total.is_empty());
        total.add(RemovalCounts {
            observations: 1,
            occurrences: 2,
            assertions: 3,
            entities: 4,
        });
        total.add(RemovalCounts {
            observations: 1,
            ..Default::default()
        });
        assert!(!total.is_empty());
        assert_eq!(total.observations, 2);
        assert_eq!(total.occurrences, 2);
        assert_eq!(total.assertions, 3);
        assert_eq!(total.entities, 4);
    }
}
