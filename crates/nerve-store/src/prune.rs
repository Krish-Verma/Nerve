//! Removal and re-statement of graph rows.
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
//! 3. **`assertion_state` is not touched here.** It is rebuilt by
//!    [`crate::derive::rebuild_assertion_state`] before orphan pruning runs, which is what makes
//!    the pruning safe under the foreign key from `assertion_state` to `assertion`: after a
//!    rebuild, an assertion with no observation has no derived row to violate.
//!
//! [`restamp_state`] is the one operation that edits existing rows. It advances the repository
//! state recorded on rows whose file was proven byte-identical to the previous run, and it is
//! sound for exactly that reason: the extractor is deterministic, so re-running it over
//! identical bytes would have produced the identical row at the new state. This is what makes an
//! incrementally maintained database indistinguishable from one built from scratch.

use std::collections::BTreeSet;

use rusqlite::{params, Connection};

use nerve_core::ids;

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

/// Delete every observation and occurrence recorded against any of `file_paths`.
///
/// Used for files that vanished from the tree and for files about to be re-extracted. Entities
/// and assertions are left alone here: whether they survive depends on the rows the same
/// transaction is about to write, so that decision belongs to [`prune_orphans`].
pub fn delete_file_rows(conn: &Connection, file_paths: &BTreeSet<String>) -> Result<RemovalCounts> {
    let mut counts = RemovalCounts::default();
    if file_paths.is_empty() {
        return Ok(counts);
    }

    let mut observations = conn.prepare("DELETE FROM observation WHERE file_path = ?1")?;
    let mut occurrences = conn.prepare("DELETE FROM occurrence WHERE file_path = ?1")?;
    for path in file_paths {
        counts.observations += observations.execute(params![path])?;
        counts.occurrences += occurrences.execute(params![path])?;
    }
    Ok(counts)
}

/// Delete the observations behind `CONTAINS` edges whose target is a directory.
///
/// Directory containment is the one part of the graph not attributable to a file path: the
/// evidence for `src CONTAINS src/lib` is the directory itself. It is re-derived from the
/// current file set on every run — which costs no parsing — so the previous run's rows are
/// cleared first and a directory that no longer holds indexed files simply does not come back.
pub fn delete_directory_containment(conn: &Connection) -> Result<usize> {
    Ok(conn.execute(
        "DELETE FROM observation
          WHERE assertion_id IN (
                SELECT a.assertion_id
                  FROM assertion a
                  JOIN entity t ON t.entity_id = a.target_entity_id
                 WHERE a.relation = 'CONTAINS' AND t.kind = 'directory')",
        [],
    )?)
}

/// Advance every surviving occurrence and observation to `state_id`.
///
/// Rows already at `state_id` are untouched. `occurrence_id` is a digest over the state
/// (ADR-0002), so it is recomputed rather than left inconsistent with the row it identifies — a
/// surrogate key that disagrees with its own fields would let a later insert of the same logical
/// occurrence create a duplicate.
///
/// The new ids are staged in a temporary table and applied by one statement rather than by a
/// loop of keyed updates. Rewriting a primary key rewrites every index entry for the row, so the
/// per-statement overhead of the loop was measurable: on a 520-module repository this is the
/// difference between roughly 22 ms and roughly 8 ms.
///
/// This pass is unavoidably proportional to repository size, not to the size of the change,
/// because the repository state participates in the identity of every occurrence. That is an
/// ADR-0002 consequence, not an incremental-indexing one, and it is the floor on how cheap a
/// re-index can be.
///
/// Returns `(occurrences, observations)` restated.
pub fn restamp_state(conn: &Connection, state_id: &str) -> Result<(usize, usize)> {
    let stale: Vec<(String, String)> = {
        let mut stmt = conn.prepare(
            "SELECT occurrence_id, entity_id, file_path, start_byte, end_byte
               FROM occurrence WHERE state_id <> ?1 ORDER BY occurrence_id",
        )?;
        let rows = stmt.query_map(params![state_id], |row| {
            let occurrence_id: String = row.get(0)?;
            let entity_id: String = row.get(1)?;
            let file_path: String = row.get(2)?;
            let start_byte: i64 = row.get(3)?;
            let end_byte: i64 = row.get(4)?;
            let restated = ids::occurrence_id(
                &entity_id,
                state_id,
                &file_path,
                start_byte as usize,
                end_byte as usize,
            );
            Ok((occurrence_id, restated))
        })?;
        let mut out = Vec::new();
        for row in rows {
            out.push(row?);
        }
        out
    };

    let mut occurrences = 0usize;
    if !stale.is_empty() {
        conn.execute_batch(
            "CREATE TEMP TABLE IF NOT EXISTS restamped (
                 was TEXT PRIMARY KEY,
                 now TEXT NOT NULL
             );
             DELETE FROM restamped;",
        )?;
        {
            let mut insert = conn.prepare("INSERT INTO restamped (was, now) VALUES (?1, ?2)")?;
            for (was, now) in &stale {
                insert.execute(params![was, now])?;
            }
        }
        occurrences = conn.execute(
            "UPDATE occurrence
                SET occurrence_id = (SELECT r.now FROM restamped r
                                      WHERE r.was = occurrence.occurrence_id),
                    state_id = ?1
              WHERE state_id <> ?1",
            params![state_id],
        )?;
        conn.execute_batch("DELETE FROM restamped;")?;
    }

    let observations = conn.execute(
        "UPDATE observation SET state_id = ?1 WHERE state_id <> ?1",
        params![state_id],
    )?;

    Ok((occurrences, observations))
}

/// Delete assertions no observation supports, then entities nothing locates or references.
///
/// Must run **after** [`crate::derive::rebuild_assertion_state`]: the rebuild empties
/// `assertion_state` and repopulates it only from surviving observations, so the assertions this
/// function deletes provably have no derived row pointing at them.
///
/// The repository entity is exempt. A tree with no indexed files still has a repository, and a
/// from-scratch index emits it unconditionally; pruning it would make an emptied repository
/// differ from a freshly indexed empty one.
pub fn prune_orphans(conn: &Connection) -> Result<RemovalCounts> {
    let assertions = conn.execute(
        "DELETE FROM assertion
          WHERE NOT EXISTS (SELECT 1 FROM observation o WHERE o.assertion_id = assertion.assertion_id)",
        [],
    )?;

    let entities = conn.execute(
        "DELETE FROM entity
          WHERE kind <> 'repository'
            AND NOT EXISTS (SELECT 1 FROM occurrence o WHERE o.entity_id = entity.entity_id)
            AND NOT EXISTS (SELECT 1 FROM assertion a WHERE a.source_entity_id = entity.entity_id)
            AND NOT EXISTS (SELECT 1 FROM assertion a WHERE a.target_entity_id = entity.entity_id)",
        [],
    )?;

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
