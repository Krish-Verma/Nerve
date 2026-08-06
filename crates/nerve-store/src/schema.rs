//! Schema v1 (ADR-0003), v2 (Slice 3), v3 (Slice 3b), v4 (Slice 5d-i), v5 (Slice 10a),
//! v6 (Slice 12b), v7 (Slice 12c-ii), and migrations.
//!
//! Migrations are append-only. `V1` is immutable: a database written by an older build must be
//! upgradable by replaying the later steps, so editing an already-shipped step in place would
//! make old and new databases disagree about what "version 1" means.

use rusqlite::{params, Connection};

use nerve_core::ids;

use crate::error::{Result, StoreError};

/// The schema version this build writes and understands.
pub const SCHEMA_VERSION: i64 = 7;

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

/// Human-readable description recorded in `schema_version` for the Slice 5d-i upgrade.
pub const SCHEMA_V4_DESCRIPTION: &str =
    "Slice 5d-i: filesystem containment re-attributed from ts-js-structural/AST_DIRECT and \
     md-structural/DOCUMENT_STATED to fs-structural/FILESYSTEM_OBSERVED";

/// Human-readable description recorded in `schema_version` for the Slice 10a upgrade.
pub const SCHEMA_V5_DESCRIPTION: &str =
    "Slice 10a: module_facts.framework_version, the cache slot a third extractor per language \
     family needs; defaults to '' so every row written before the framework rules misses";

/// Human-readable description recorded in `schema_version` for the Slice 12b upgrade.
pub const SCHEMA_V6_DESCRIPTION: &str =
    "Slice 12b: the historical model — git_commit, git_change, git_rename_hypothesis and \
     git_history_ingest; history availability recorded as data, never inferred from absence";

/// Human-readable description recorded in `schema_version` for the Slice 12c-ii upgrade.
pub const SCHEMA_V7_DESCRIPTION: &str =
    "Slice 12c-ii: git_rename_hypothesis rebuilt with two blob oids, a named matcher and an \
     integer measurement; git_rename_analysis for per-commit candidate-set completeness; \
     git_commit.summary_truncation, defaulting to 'unknown' because v6 rows cannot be backfilled";

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

/// Schema v5 — Slice 10a. One additive column, and the default is the whole point.
///
/// `module_facts` had exactly two version columns, `structural_version` and `reference_version`,
/// reused positionally per language family: a document writes the `md-structural` version twice,
/// a Python module writes `py-structural` / `py-reference`, a TS/JS module writes
/// `ts-js-structural` / `ts-js-reference`. **There was no slot for a third extractor**, and that
/// is the precise location of the Slice 9b upgrade defect, where two extractors happened to share
/// a version string and an existing index hit the cache forever.
///
/// `NOT NULL DEFAULT ''` gives every row written before this slice a value that equals no released
/// extractor version, so every file whose family *has* a framework extractor misses the cache and
/// is re-extracted — which is the required behaviour, and the one no test that builds a fresh
/// index can observe. A family with no framework extractor expects `''` and keeps hitting, so
/// documents do not churn.
///
/// Rejected: folding the version into `reference_version` as a compound string (drifts,
/// unparseable), and bumping `reference_version` whenever a framework rule changes (couples two
/// independent extractors, and re-extracts references in order to publish a route change).
const V5: &str = r#"
ALTER TABLE module_facts ADD COLUMN framework_version TEXT NOT NULL DEFAULT '';
"#;

/// Schema v6 — Slice 12b, the historical model. Four new tables, no `ALTER` of any existing one.
///
/// History is **not** in the evidence model, and that is a decision rather than an omission
/// (`docs/plans/slice-12b-historical-model.md` §3). A tree diff is a primary-source fact read out
/// of an immutable object: every field an `observation` carries — source type, directness,
/// extractor version, match quality, query-time freshness — exists to qualify a *derived* claim,
/// and routing a certainty through them would cost three rows per fact to express doubt that does
/// not exist. Nerve already keeps primary facts in plain tables: `repository_state`,
/// `extractor_run`, `module_facts`. Nothing is collapsed; the model is declined for facts it was
/// not built for.
///
/// The one genuinely uncertain fact here is a **rename hypothesis**, and the reason it still
/// cannot be an observation is mechanical: an `observation` requires an `assertion_id`, and an
/// `assertion` requires two `entity_id`s. A rename relates two *paths*, and a rename's `from_path`
/// is by definition no longer in the tree, so there is no entity to point at. Its uncertainty is
/// carried by two named columns — `evidence` and `ambiguity` — which is weaker than an evidence
/// profile and is recorded as such rather than dressed up.
///
/// Three columns exist so that an **absence is never inferred**, which is the whole point of the
/// slice:
///
/// - `git_commit.parent_completeness` — five reasons a commit may have no visible parent, of which
///   exactly one means the project's history begins there. A shallow boundary diffed against the
///   empty tree would report every file in the boundary tree as newly added, which is *"history
///   begins here"* stated as data.
/// - `git_commit.changes_enumerated` — which of four silences a commit with zero `git_change` rows
///   is. Stored rather than inferred, because "the parent was unreadable" and "nothing changed"
///   look identical from the row count alone.
/// - `git_history_ingest.walk_terminated_by` — whether the boundary is the repository's or Nerve's
///   own. A bounded ingest that read as a complete history would turn a budget into a claim about
///   the project's origin.
///
/// Storage is a **delta**, measured rather than assumed: per-commit snapshots cost
/// `O(commits × tree_size)` against the delta's `O(total churn)`, measured at 30.1× on this
/// repository at 85 commits and 177× on a 1,214-commit one (§4). The row amplification grows with
/// history depth, so the larger the repository the worse the alternative gets.
///
/// **`repo_id` is on every table** rather than reached through a join, because every read is
/// scoped to one repository and the composite primary keys are what make re-ingest of an immutable
/// commit an `INSERT OR IGNORE` instead of a diff.
///
/// No `IF NOT EXISTS`: no permanent table in this schema has it, and a migration that tolerated
/// re-application would hide a real double-apply.
///
/// **Superseded in part by [`V7`], which is why the SQL below still says `blob_oid`.** Slice 12c-ii
/// replaces `git_rename_hypothesis.blob_oid` with `from_blob_oid` and `to_blob_oid`, because a
/// similarity pair has two blobs and one `NOT NULL` column cannot hold both. This string is not
/// edited: a v5 database still has to reach v6 by running exactly what v6 shipped, and then reach
/// v7 by running the rebuild. Reading the current shape off this constant is therefore wrong —
/// [`V7`] is where `git_rename_hypothesis` is defined today.
const V6: &str = r#"
CREATE TABLE git_commit (
    repo_id              TEXT    NOT NULL REFERENCES repository(repo_id),
    commit_oid           TEXT    NOT NULL,   -- 40 lowercase hex
    tree_oid             TEXT    NOT NULL,
    parent_oids          TEXT    NOT NULL,   -- JSON array, listed order; [] for a root commit
    parent_completeness  TEXT    NOT NULL,   -- closed vocabulary: ParentCompleteness
    changes_enumerated   TEXT    NOT NULL,   -- closed vocabulary: ChangesEnumerated
    author_time          INTEGER NOT NULL,   -- epoch seconds, signed, as the object records it
    author_tz            TEXT    NOT NULL,
    committer_time       INTEGER NOT NULL,
    committer_tz         TEXT    NOT NULL,
    author_ident         TEXT,               -- NULL unless --with-identity
    committer_ident      TEXT,               -- NULL unless --with-identity
    summary              TEXT    NOT NULL,   -- first message line, bounded, lossy UTF-8
    is_merge             INTEGER NOT NULL,
    PRIMARY KEY (repo_id, commit_oid)
);

CREATE INDEX idx_git_commit_time ON git_commit(repo_id, committer_time);

CREATE TABLE git_change (
    repo_id        TEXT    NOT NULL REFERENCES repository(repo_id),
    commit_oid     TEXT    NOT NULL,
    path           TEXT    NOT NULL,   -- as recorded in the tree
    change_kind    TEXT    NOT NULL,   -- added | modified | deleted | mode_changed
    blob_oid       TEXT,               -- NULL iff deleted
    prev_blob_oid  TEXT,               -- NULL iff added
    mode           INTEGER,
    prev_mode      INTEGER,
    PRIMARY KEY (repo_id, commit_oid, path),
    FOREIGN KEY (repo_id, commit_oid) REFERENCES git_commit(repo_id, commit_oid)
);

CREATE INDEX idx_git_change_path ON git_change(repo_id, path);
CREATE INDEX idx_git_change_blob ON git_change(repo_id, blob_oid);

CREATE TABLE git_rename_hypothesis (
    repo_id       TEXT NOT NULL REFERENCES repository(repo_id),
    commit_oid    TEXT NOT NULL,
    from_path     TEXT NOT NULL,
    to_path       TEXT NOT NULL,
    evidence      TEXT NOT NULL,   -- exact_content (12b); similar_content added in 12c
    blob_oid      TEXT NOT NULL,
    ambiguity     TEXT NOT NULL,   -- unique | many_from | many_to | many_both
    PRIMARY KEY (repo_id, commit_oid, from_path, to_path),
    FOREIGN KEY (repo_id, commit_oid) REFERENCES git_commit(repo_id, commit_oid)
);

CREATE TABLE git_history_ingest (
    repo_id             TEXT    PRIMARY KEY REFERENCES repository(repo_id),
    head_oid            TEXT,               -- NULL on an unborn branch
    walked_from         TEXT    NOT NULL,   -- JSON array of tip oids
    commits_recorded    INTEGER NOT NULL,
    commit_budget       INTEGER NOT NULL,
    walk_terminated_by  TEXT    NOT NULL,   -- closed vocabulary: WalkTermination
    shallow             INTEGER NOT NULL,
    shallow_boundary    TEXT    NOT NULL,   -- JSON array of boundary oids, [] when not shallow
    promisor            INTEGER NOT NULL,
    refusals            TEXT    NOT NULL,   -- JSON object, form -> count
    reader_version      TEXT    NOT NULL,
    ingested_at         TEXT    NOT NULL
);
"#;

/// Schema v7 — Slice 12c-ii. Storage for a second kind of rename evidence, and for what a summary is.
///
/// Three changes, and the first two are the ones a reader will want justified.
///
/// **`git_rename_hypothesis` is rebuilt rather than altered**, because `blob_oid TEXT NOT NULL`
/// becomes `from_blob_oid` and `to_blob_oid`: a similarity pair names two blobs and one column
/// cannot hold both. SQLite has no `ALTER TABLE … RENAME COLUMN` that could split one column into
/// two, and no way to add a table-level `CHECK` to an existing table, so this is the documented
/// create-copy-drop-rename. It is **lossless**: every v6 row copies with
/// `from_blob_oid = to_blob_oid = blob_oid`, `matcher_id = 'git-blob-oid'`, `matcher_version = '1'`
/// and no measurement, which is precisely what an exact-content hypothesis is.
///
/// **Nothing in this schema references `git_rename_hypothesis` by foreign key**, so the drop and the
/// rename violate no constraint even though `PRAGMA foreign_keys=ON` is set for every connection
/// (`db.rs:37`). The direction of the dependency is the other way round — the table points at
/// `repository` and `git_commit`, and both survive the rebuild untouched. That is also why the
/// pragma cannot be, and is not, turned off here: it cannot be changed inside a transaction, and
/// this step has no need to. Each entry in [`MIGRATIONS`] runs in its own transaction
/// (`migrate()`, below), so a failure anywhere in this step rolls the whole of it back and leaves
/// the database at v6 — including the `ALTER TABLE` that ran first.
///
/// **The `CHECK` is where "evidence is never blended" stops being a convention.** An exact-content
/// row cannot carry a measurement and a similar-content row cannot omit one, so a future writer
/// that tried to give an exact match a score, or to record a similarity hypothesis without saying
/// what was counted, gets a constraint violation rather than a code review. The measurement is two
/// integers rather than a float on purpose: `1320 / 1500` says what was counted and can be checked
/// by hand, where `0.88` is a number comparable against anything and rounds away its own meaning —
/// which is the generic `confidence: float` `CLAUDE.md` §3 forbids, arriving by the back door.
///
/// The primary key needs no widening. For one `(commit, from_path, to_path)` the two blob oids are
/// fixed by the tree diff, so a pair is exact or similar and never both.
///
/// **`git_rename_analysis` is per commit, not per row**, because the decisive case has no row to
/// carry a flag: when the candidate set exceeds a bound the commit records *no* similarity
/// hypothesis, and an absence would once again have to be interpreted — the failure
/// `git_commit.changes_enumerated` exists to prevent. `matcher_id` is in the primary key so a
/// second matcher can analyse the same commit later without a migration. Exact-content renames get
/// no analysis row at all, and that is a claim rather than an omission: the exact matcher reads no
/// blob content, so it is complete exactly when the diff was enumerated, which
/// `git_commit.changes_enumerated` already records.
///
/// **`git_commit.summary_truncation` defaults to `'unknown'`, and that is the honest migration.** A
/// v6 row cannot be backfilled, and length is not the answer: a summary of exactly
/// `MAX_SUMMARY_BYTES` is *not* truncated, so `length(summary) = bound ⟹ truncated` would
/// manufacture a false positive on the one boundary case that matters. `unknown` is a third
/// vocabulary value rather than a boolean precisely so the past does not have to be guessed at.
const V7: &str = r#"
ALTER TABLE git_commit ADD COLUMN summary_truncation TEXT NOT NULL DEFAULT 'unknown';

CREATE TABLE git_rename_hypothesis_v7 (
    repo_id           TEXT    NOT NULL REFERENCES repository(repo_id),
    commit_oid        TEXT    NOT NULL,
    from_path         TEXT    NOT NULL,
    to_path           TEXT    NOT NULL,
    evidence          TEXT    NOT NULL,   -- closed vocabulary: RenameEvidence
    from_blob_oid     TEXT    NOT NULL,
    to_blob_oid       TEXT    NOT NULL,
    matcher_id        TEXT    NOT NULL,   -- which method produced this row
    matcher_version   TEXT    NOT NULL,
    match_numerator   INTEGER,            -- NULL iff evidence = exact_content
    match_denominator INTEGER,            -- NULL iff evidence = exact_content
    ambiguity         TEXT    NOT NULL,   -- closed vocabulary: RenameAmbiguity
    PRIMARY KEY (repo_id, commit_oid, from_path, to_path),
    FOREIGN KEY (repo_id, commit_oid) REFERENCES git_commit(repo_id, commit_oid),
    CHECK (
        (evidence = 'exact_content'
            AND from_blob_oid = to_blob_oid
            AND match_numerator IS NULL AND match_denominator IS NULL)
     OR (evidence = 'similar_content'
            AND from_blob_oid <> to_blob_oid
            AND match_numerator IS NOT NULL AND match_denominator IS NOT NULL
            AND match_denominator > 0
            AND match_numerator >= 0
            AND match_numerator <= match_denominator)
    )
);

INSERT INTO git_rename_hypothesis_v7
    (repo_id, commit_oid, from_path, to_path, evidence,
     from_blob_oid, to_blob_oid, matcher_id, matcher_version,
     match_numerator, match_denominator, ambiguity)
SELECT repo_id, commit_oid, from_path, to_path, evidence,
       blob_oid, blob_oid, 'git-blob-oid', '1',
       NULL, NULL, ambiguity
  FROM git_rename_hypothesis;

DROP TABLE git_rename_hypothesis;

ALTER TABLE git_rename_hypothesis_v7 RENAME TO git_rename_hypothesis;

CREATE TABLE git_rename_analysis (
    repo_id               TEXT    NOT NULL REFERENCES repository(repo_id),
    commit_oid            TEXT    NOT NULL,
    matcher_id            TEXT    NOT NULL,
    matcher_version       TEXT    NOT NULL,
    threshold_numerator   INTEGER NOT NULL,
    threshold_denominator INTEGER NOT NULL,
    deletions_considered  INTEGER NOT NULL,
    additions_considered  INTEGER NOT NULL,
    pairs_considered      INTEGER NOT NULL,
    pairs_measured        INTEGER NOT NULL,
    completeness          TEXT    NOT NULL,   -- closed vocabulary: RenameAnalysisCompleteness
    unmeasured            TEXT    NOT NULL,   -- JSON object, reason -> count
    PRIMARY KEY (repo_id, commit_oid, matcher_id),
    FOREIGN KEY (repo_id, commit_oid) REFERENCES git_commit(repo_id, commit_oid),
    CHECK (threshold_denominator > 0 AND pairs_measured <= pairs_considered)
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
const MIGRATIONS: [(i64, &str, Step); 7] = [
    (1, SCHEMA_V1_DESCRIPTION, Step::Sql(V1)),
    (2, SCHEMA_V2_DESCRIPTION, Step::Sql(V2)),
    (3, SCHEMA_V3_DESCRIPTION, Step::Rust(migrate_v3)),
    (4, SCHEMA_V4_DESCRIPTION, Step::Rust(migrate_v4)),
    (5, SCHEMA_V5_DESCRIPTION, Step::Sql(V5)),
    (6, SCHEMA_V6_DESCRIPTION, Step::Sql(V6)),
    (7, SCHEMA_V7_DESCRIPTION, Step::Sql(V7)),
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

/// The evidence label v4 writes, pinned as literals rather than read from the live constants.
///
/// A migration must mean the same thing forever. `fs-structural`'s version will move one day, and
/// when it does this step must still write `1.0.0`, because `1.0.0` is what the rules being
/// applied here are — that is the same reason [`V1`] is immutable. The `nerve-index` crate that
/// owns these names is downstream of this one and cannot be imported, which makes the literals
/// unavoidable as well as correct; `nerve-index/tests/graph.rs` asserts the two agree today.
const V4_SOURCE_TYPE: &str = "FILESYSTEM_OBSERVED";
const V4_EXTRACTOR_ID: &str = "fs-structural";
const V4_EXTRACTOR_VERSION: &str = "1.0.0";

/// Schema v4 — Slice 5d-i. **No DDL**: the correction is entirely in the data.
///
/// `observation.evidence_source_type`, `extractor_id` and `extractor_version` are `TEXT` with no
/// `CHECK` constraint and no lookup table — the vocabulary is closed in Rust, not in SQL — so
/// adding `FILESYSTEM_OBSERVED` forces no schema change. What it does force is a rewrite of rows
/// already on disk that say `AST_DIRECT` / `ts-js-structural` for structure no syntax tree ever
/// stated.
///
/// A re-index cannot be assumed, and would not be sufficient if it were: directory containment is
/// re-derived every run, but repository→file and directory→file rows are re-emitted only for
/// files a run actually re-extracts, so an unchanged file would keep a wrong row indefinitely.
///
/// "Is this filesystem structure?" is decided **without guessing**: the qualifying set is
/// `CONTAINS` assertions whose *source* entity kind is `repository` or `directory`, which is
/// exactly the set the emission sites produce and is a closed query over stored columns. No
/// `LIKE` on a path, no extension sniffing, no heuristic. `File CONTAINS Document` — source kind
/// `file` — is deliberately outside it and stays `DOCUMENT_STATED`, as does everything a parse or
/// a heading scan produced.
///
/// Row identity is untouched. `observation_id` is an autoincrement surrogate key (there is no
/// `ids::observation_id`), so re-stamping the evidence columns updates in place and cannot orphan
/// or duplicate anything. The uniqueness index does cover `extractor_id`, `extractor_version` and
/// `evidence_source_type`, so a collision would be a hard SQLite error rather than a silent
/// merge; it cannot arise here because no `fs-structural` row exists before this step runs.
const V4: &str = r#"
UPDATE observation
   SET evidence_source_type = ?1,
       extractor_id         = ?2,
       extractor_version    = ?3
 WHERE assertion_id IN (
       SELECT a.assertion_id
         FROM assertion a
         JOIN entity e ON e.entity_id = a.source_entity_id
        WHERE a.relation = 'CONTAINS'
          AND e.kind IN ('repository', 'directory'))
"#;

/// Re-stamp filesystem containment, then re-derive `assertion_state` from the corrected rows.
///
/// The second half reuses [`crate::derive::rebuild_assertion_state`] rather than patching
/// `strongest_source_type` and `source_type_mask` by hand. That is the Slice 3b precedent and it
/// is not a style preference: the mask's bit layout is generated from `EvidenceSourceType::ALL`
/// at runtime, so the derivation is the only thing that knows it, and a second implementation
/// here would be a second thing to keep in step with the vocabulary.
fn migrate_v4(conn: &Connection) -> Result<()> {
    conn.execute(
        V4,
        params![V4_SOURCE_TYPE, V4_EXTRACTOR_ID, V4_EXTRACTOR_VERSION],
    )?;
    crate::derive::rebuild_assertion_state(conn)?;
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
