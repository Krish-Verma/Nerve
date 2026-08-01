//! Schema v1 (ADR-0003), v2 (Slice 3), v3 (Slice 3b), and migrations.
//!
//! Migrations are append-only. `V1` is immutable: a database written by an older build must be
//! upgradable by replaying the later steps, so editing an already-shipped step in place would
//! make old and new databases disagree about what "version 1" means.

use rusqlite::{params, Connection};

use nerve_core::ids;

use crate::error::{Result, StoreError};

/// The schema version this build writes and understands.
pub const SCHEMA_VERSION: i64 = 3;

/// Human-readable description recorded in `schema_version`.
pub const SCHEMA_V1_DESCRIPTION: &str =
    "Slice 1: entities, occurrences, assertions, observations, derived assertion_state, FTS5";

/// Human-readable description recorded in `schema_version` for the Slice 3 upgrade.
pub const SCHEMA_V2_DESCRIPTION: &str =
    "Slice 3: module_facts extraction cache for incremental indexing; identity_link uniqueness";

/// Human-readable description recorded in `schema_version` for the Slice 3b upgrade.
pub const SCHEMA_V3_DESCRIPTION: &str =
    "Slice 3b (ADR-0006): repository state normalized out of occurrence, observation \
     and assertion_state; occurrence_id no longer digests the state";

const V1: &str = r#"
CREATE TABLE repository (
    repo_id     TEXT PRIMARY KEY,
    project_id  TEXT NOT NULL,
    root_path   TEXT NOT NULL,
    created_at  TEXT NOT NULL
);

CREATE TABLE repository_state (
    state_id        TEXT PRIMARY KEY,
    repo_id         TEXT NOT NULL REFERENCES repository(repo_id),
    kind            TEXT NOT NULL,
    git_commit      TEXT,
    content_merkle  TEXT NOT NULL,
    created_at      TEXT NOT NULL
);

CREATE TABLE entity (
    entity_id   TEXT PRIMARY KEY,
    repo_id     TEXT NOT NULL REFERENCES repository(repo_id),
    kind        TEXT NOT NULL,
    name        TEXT NOT NULL,
    scope_path  TEXT NOT NULL,
    language    TEXT,
    meta        TEXT
);

CREATE TABLE occurrence (
    occurrence_id  TEXT PRIMARY KEY,
    entity_id      TEXT NOT NULL REFERENCES entity(entity_id),
    state_id       TEXT NOT NULL REFERENCES repository_state(state_id),
    file_path      TEXT NOT NULL,
    start_byte     INTEGER NOT NULL,
    end_byte       INTEGER NOT NULL,
    start_line     INTEGER NOT NULL,
    start_col      INTEGER NOT NULL,
    end_line       INTEGER NOT NULL,
    end_col        INTEGER NOT NULL,
    content_hash   TEXT NOT NULL
);

CREATE TABLE assertion (
    assertion_id      TEXT PRIMARY KEY,
    repo_id           TEXT NOT NULL REFERENCES repository(repo_id),
    source_entity_id  TEXT NOT NULL REFERENCES entity(entity_id),
    relation          TEXT NOT NULL,
    target_entity_id  TEXT NOT NULL REFERENCES entity(entity_id)
);

CREATE TABLE extractor_run (
    run_id             INTEGER PRIMARY KEY AUTOINCREMENT,
    repo_id            TEXT NOT NULL REFERENCES repository(repo_id),
    state_id           TEXT NOT NULL REFERENCES repository_state(state_id),
    extractor_id       TEXT NOT NULL,
    extractor_version  TEXT NOT NULL,
    started_at         TEXT NOT NULL,
    finished_at        TEXT,
    files_processed    INTEGER NOT NULL DEFAULT 0,
    files_failed       INTEGER NOT NULL DEFAULT 0,
    status             TEXT NOT NULL
);

CREATE TABLE observation (
    observation_id        INTEGER PRIMARY KEY AUTOINCREMENT,
    assertion_id          TEXT NOT NULL REFERENCES assertion(assertion_id),
    extractor_run_id      INTEGER NOT NULL REFERENCES extractor_run(run_id),
    evidence_source_type  TEXT NOT NULL,
    directness            TEXT NOT NULL,
    extractor_id          TEXT NOT NULL,
    extractor_version     TEXT NOT NULL,
    match_quality         REAL,
    state_id              TEXT NOT NULL REFERENCES repository_state(state_id),
    file_path             TEXT NOT NULL,
    start_line            INTEGER NOT NULL,
    end_line              INTEGER NOT NULL,
    content_hash          TEXT NOT NULL,
    environment           TEXT,
    details               TEXT,
    created_at            TEXT NOT NULL
);

-- DERIVED. Only nerve_store::rebuild_assertion_state may write this table.
CREATE TABLE assertion_state (
    assertion_id           TEXT PRIMARY KEY REFERENCES assertion(assertion_id),
    state_id               TEXT NOT NULL,
    status                 TEXT NOT NULL,
    strongest_source_type  TEXT NOT NULL,
    source_type_mask       INTEGER NOT NULL,
    observation_count      INTEGER NOT NULL,
    is_unresolved          INTEGER NOT NULL,
    last_seen_state_id     TEXT NOT NULL
);

-- Created in Slice 1, deliberately unused until Slice 3.
CREATE TABLE identity_link (
    link_id          INTEGER PRIMARY KEY AUTOINCREMENT,
    repo_id          TEXT NOT NULL REFERENCES repository(repo_id),
    left_entity_id   TEXT NOT NULL,
    right_entity_id  TEXT NOT NULL,
    link_kind        TEXT NOT NULL,
    evidence         TEXT,
    created_at       TEXT NOT NULL
);

CREATE INDEX idx_entity_repo_kind        ON entity(repo_id, kind);
CREATE INDEX idx_entity_name             ON entity(name);
CREATE INDEX idx_occurrence_entity       ON occurrence(entity_id);
CREATE INDEX idx_occurrence_state        ON occurrence(state_id);
CREATE INDEX idx_occurrence_path         ON occurrence(file_path);
CREATE INDEX idx_assertion_source        ON assertion(source_entity_id, relation);
CREATE INDEX idx_assertion_target        ON assertion(target_entity_id, relation);
CREATE INDEX idx_assertion_repo_relation ON assertion(repo_id, relation);
CREATE INDEX idx_observation_assertion   ON observation(assertion_id);
CREATE INDEX idx_observation_run         ON observation(extractor_run_id);
CREATE INDEX idx_observation_state       ON observation(state_id);
CREATE INDEX idx_assertion_state_status  ON assertion_state(status);
CREATE INDEX idx_extractor_run_state     ON extractor_run(state_id);

-- Logical uniqueness for observations. The surrogate key is an autoincrement integer, so
-- without this a re-index of an unchanged tree would append duplicate evidence rows forever.
CREATE UNIQUE INDEX idx_observation_identity ON observation(
    assertion_id, state_id, extractor_id, extractor_version,
    evidence_source_type, file_path, start_line, end_line
);

CREATE VIRTUAL TABLE entity_fts USING fts5(
    name,
    scope_path,
    content='entity',
    content_rowid='rowid'
);

CREATE TRIGGER entity_fts_after_insert AFTER INSERT ON entity BEGIN
    INSERT INTO entity_fts(rowid, name, scope_path)
    VALUES (new.rowid, new.name, new.scope_path);
END;

CREATE TRIGGER entity_fts_after_delete AFTER DELETE ON entity BEGIN
    INSERT INTO entity_fts(entity_fts, rowid, name, scope_path)
    VALUES ('delete', old.rowid, old.name, old.scope_path);
END;

CREATE TRIGGER entity_fts_after_update AFTER UPDATE ON entity BEGIN
    INSERT INTO entity_fts(entity_fts, rowid, name, scope_path)
    VALUES ('delete', old.rowid, old.name, old.scope_path);
    INSERT INTO entity_fts(rowid, name, scope_path)
    VALUES (new.rowid, new.name, new.scope_path);
END;
"#;

/// Schema v2 — Slice 3. Additive only: one new table, two new indexes, nothing altered.
///
/// `module_facts` is a **cache of extractor inputs**, not part of the evidence graph. It holds,
/// per indexed module, the content hash it was extracted at plus the small amount of
/// cross-module information the extractors need about *other* modules: the export map, the
/// re-export specifiers, and the import specifiers.
///
/// Without it, re-extracting one file would still require parsing every other file, because
/// `exports::ExportIndex` spans the whole corpus and a module's resolution outcome depends on
/// the export maps of everything it imports — which is precisely the cost incremental indexing
/// exists to avoid. It also stores the previous `(rel_path, content_hash)` set, which is what
/// change detection compares against, and per-file counters so that whole-repository totals stay
/// reportable when only part of the repository was re-extracted.
///
/// It stores no source text: identifiers, specifiers, entity ids, and BLAKE3 digests only.
const V2: &str = r#"
CREATE TABLE module_facts (
    repo_id             TEXT NOT NULL REFERENCES repository(repo_id),
    rel_path            TEXT NOT NULL,
    content_hash        TEXT NOT NULL,
    language            TEXT NOT NULL,
    structural_version  TEXT NOT NULL,
    reference_version   TEXT NOT NULL,
    facts               TEXT NOT NULL,
    PRIMARY KEY (repo_id, rel_path)
);

CREATE INDEX idx_module_facts_hash ON module_facts(repo_id, content_hash);

-- An identity link is a proposal about one pair; proposing it twice is the same proposal.
CREATE UNIQUE INDEX idx_identity_link_identity
    ON identity_link(repo_id, left_entity_id, right_entity_id, link_kind);
"#;

/// Schema v3 — Slice 3b, ADR-0006. The part that is expressible in SQL.
///
/// Deduplication comes first and is load-bearing. A Slice 1/2 (v1) database was insert-only:
/// re-indexing appended another `occurrence` row for the same entity at the same span under a
/// new `state_id`, and another `observation` row for the same claim. Under the new identity
/// those are the *same* row, so the superseded copies must go before the state column does —
/// otherwise the primary key and the uniqueness index would both be violated by rows that were
/// legal a moment earlier. The surviving copy is the most recently written one (highest rowid).
/// On a v2 database this deletes nothing: the Slice 3 restatement pass had already collapsed
/// every row onto a single state.
const V3: &str = r#"
DELETE FROM occurrence
 WHERE rowid NOT IN (
       SELECT MAX(rowid) FROM occurrence
        GROUP BY entity_id, file_path, start_byte, end_byte);

DELETE FROM observation
 WHERE observation_id NOT IN (
       SELECT MAX(observation_id) FROM observation
        GROUP BY assertion_id, extractor_id, extractor_version,
                 evidence_source_type, file_path, start_line, end_line);

DROP INDEX IF EXISTS idx_occurrence_state;
DROP INDEX IF EXISTS idx_observation_state;
DROP INDEX IF EXISTS idx_observation_identity;

ALTER TABLE occurrence      DROP COLUMN state_id;
ALTER TABLE observation     DROP COLUMN state_id;
ALTER TABLE assertion_state DROP COLUMN state_id;
ALTER TABLE assertion_state DROP COLUMN last_seen_state_id;

-- The same tuple as v1 minus the state. A tightening, not a loosening: the same evidence at the
-- same place from the same extractor is now one row across states rather than one row per state.
CREATE UNIQUE INDEX idx_observation_identity ON observation(
    assertion_id, extractor_id, extractor_version,
    evidence_source_type, file_path, start_line, end_line
);
"#;

/// One migration step. Almost all of them are SQL; v3 is not, because it must recompute a
/// BLAKE3 digest and SQLite has no such function.
enum Step {
    /// A batch of statements.
    Sql(&'static str),
    /// Rust that owns its own statements. Runs inside the step's transaction.
    Rust(fn(&Connection) -> Result<()>),
}

/// Migration steps, in application order: `(version, description, step)`.
///
/// Appending to this list is how the schema evolves. Editing an existing entry is prohibited.
const MIGRATIONS: [(i64, &str, Step); 3] = [
    (1, SCHEMA_V1_DESCRIPTION, Step::Sql(V1)),
    (2, SCHEMA_V2_DESCRIPTION, Step::Sql(V2)),
    (3, SCHEMA_V3_DESCRIPTION, Step::Rust(migrate_v3)),
];

/// Apply [`V3`], then restate every `occurrence_id` under the ADR-0006 tuple.
///
/// The new id is `blake3(entity_id, rel_path, start_byte, end_byte)`, which SQLite cannot
/// compute, so the pairs are staged in a temporary table and applied by one statement rather
/// than by a loop of keyed updates — rewriting a primary key rewrites every index entry over the
/// row, and the per-statement overhead of the loop is measurable.
///
/// The deduplication in [`V3`] runs first and guarantees the new ids are unique, so this update
/// cannot collide.
fn migrate_v3(conn: &Connection) -> Result<()> {
    conn.execute_batch(V3)?;

    let restated: Vec<(String, String)> = {
        let mut stmt = conn.prepare(
            "SELECT occurrence_id, entity_id, file_path, start_byte, end_byte
               FROM occurrence ORDER BY occurrence_id",
        )?;
        let rows = stmt.query_map([], |row| {
            let was: String = row.get(0)?;
            let entity_id: String = row.get(1)?;
            let file_path: String = row.get(2)?;
            let start_byte: i64 = row.get(3)?;
            let end_byte: i64 = row.get(4)?;
            let now = ids::occurrence_id(
                &entity_id,
                &file_path,
                start_byte as usize,
                end_byte as usize,
            );
            Ok((was, now))
        })?;
        let mut out = Vec::new();
        for row in rows {
            out.push(row?);
        }
        out
    };

    if restated.is_empty() {
        return Ok(());
    }

    conn.execute_batch(
        "CREATE TEMP TABLE IF NOT EXISTS occurrence_v3_ids (
             was TEXT PRIMARY KEY,
             now TEXT NOT NULL
         );
         DELETE FROM occurrence_v3_ids;",
    )?;
    {
        let mut insert =
            conn.prepare("INSERT INTO occurrence_v3_ids (was, now) VALUES (?1, ?2)")?;
        for (was, now) in &restated {
            insert.execute(params![was, now])?;
        }
    }
    conn.execute(
        "UPDATE occurrence
            SET occurrence_id = (SELECT r.now FROM occurrence_v3_ids r
                                  WHERE r.was = occurrence.occurrence_id)
          WHERE occurrence_id IN (SELECT was FROM occurrence_v3_ids)",
        [],
    )?;
    conn.execute_batch("DROP TABLE occurrence_v3_ids;")?;
    Ok(())
}

/// Bring a connection up to [`SCHEMA_VERSION`].
///
/// Idempotent: running it on an already-current database is a no-op. A database at an older
/// version has only the missing steps replayed, inside one transaction each, so an interrupted
/// upgrade leaves a coherent version rather than a half-applied one. Running it on a database
/// written by a newer build is a hard error rather than a best-effort guess.
pub fn migrate(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS schema_version (
            version     INTEGER PRIMARY KEY,
            applied_at  TEXT NOT NULL,
            description TEXT NOT NULL
        );",
    )?;

    let current: Option<i64> =
        conn.query_row("SELECT MAX(version) FROM schema_version", [], |row| {
            row.get(0)
        })?;

    if let Some(found) = current {
        if found > SCHEMA_VERSION {
            return Err(StoreError::SchemaTooNew {
                found,
                supported: SCHEMA_VERSION,
            });
        }
    }

    let applied = current.unwrap_or(0);
    for (version, description, step) in MIGRATIONS {
        if version <= applied {
            continue;
        }
        let tx = conn.unchecked_transaction()?;
        match step {
            Step::Sql(sql) => tx.execute_batch(sql)?,
            Step::Rust(run) => run(&tx)?,
        }
        tx.execute(
            "INSERT INTO schema_version (version, applied_at, description)
             VALUES (?1, strftime('%Y-%m-%dT%H:%M:%fZ','now'), ?2)",
            rusqlite::params![version, description],
        )?;
        tx.commit()?;
    }
    Ok(())
}

/// Read the schema version currently on disk.
pub fn schema_version(conn: &Connection) -> Result<Option<i64>> {
    let exists: i64 = conn.query_row(
        "SELECT count(*) FROM sqlite_master WHERE type='table' AND name='schema_version'",
        [],
        |row| row.get(0),
    )?;
    if exists == 0 {
        return Ok(None);
    }
    Ok(
        conn.query_row("SELECT MAX(version) FROM schema_version", [], |row| {
            row.get(0)
        })?,
    )
}
