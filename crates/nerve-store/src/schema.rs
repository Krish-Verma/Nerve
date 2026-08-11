//! Schema v1 (ADR-0003), v2 (Slice 3), v3 (Slice 3b), v4 (Slice 5d-i), v5 (Slice 10a),
//! v6 (Slice 12b), v7 (Slice 12c-ii), v8 (Slice 13a-i), v9 (Slice 14a), v10 (Slice 14b-i),
//! and migrations.
//!
//! Migrations are append-only. `V1` is immutable: a database written by an older build must be
//! upgradable by replaying the later steps, so editing an already-shipped step in place would
//! make old and new databases disagree about what "version 1" means.

use rusqlite::{params, Connection};

use nerve_core::ids;

use crate::error::{Result, StoreError};

/// The schema version this build writes and understands.
pub const SCHEMA_VERSION: i64 = 10;

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

/// Human-readable description recorded in `schema_version` for the Slice 13a-i upgrade.
pub const SCHEMA_V8_DESCRIPTION: &str =
    "Slice 13a-i: repo_registry and contract_link — one repository's stated view of its \
     neighbours, with the target recorded as a snapshot because it lives in another database";

/// Human-readable description recorded in `schema_version` for the Slice 14a upgrade.
pub const SCHEMA_V9_DESCRIPTION: &str =
    "Slice 14a: memory, memory_citation and memory_event — human-confirmed project memory, with \
     the subject recorded as a snapshot because entity rows are routinely pruned";

/// Human-readable description recorded in `schema_version` for the Slice 14b-i upgrade.
pub const SCHEMA_V10_DESCRIPTION: &str =
    "Slice 14b-i: memory.scope and memory_event.operation closed in SQL — a typo in a scope would \
     otherwise suppress a conflict report, and an unrenderable operation would reach the interface";

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

/// Schema v8 — Slice 13a-i. Two new tables, no `ALTER` of any existing one.
///
/// # Where a cross-repository link lives, when there are two databases
///
/// Nerve's database is per repository. A cross-repository link has one end in each of two of them,
/// and the placement that looks natural is wrong twice over: writing the link into **both**
/// databases makes two writable copies of one fact and means indexing B writes into A, which Nerve
/// has never done; a **separate global** registry database puts a new writable location outside
/// every repository, for a product whose whole storage story is one gitignored directory.
///
/// So both tables live in the database of the repository the command was run from, and they are
/// **that repository's stated view of its neighbours**. The consequence is stated rather than
/// hidden: a link is directional and one-sided. A's database knows A depends on B; B's database
/// does not know it is depended upon until B registers A.
///
/// # `contract_link.target_entity_id` is deliberately **not** a foreign key
///
/// Contrast `assertion.target_entity_id` at [`V1`] (`schema.rs:97`), which is
/// `NOT NULL REFERENCES entity(entity_id)` and enforced — `PRAGMA foreign_keys=ON` is set on every
/// connection (`db.rs:37`). That constraint is the guarantee that every endpoint of every assertion
/// is a thing Nerve actually saw in *this* repository.
///
/// A contract link's target is by construction **not** in this repository, so the same constraint
/// cannot hold and must not be faked. The only two ways to force one are to create a proxy entity
/// for the foreign file inside this database — inventing a local entity for something never indexed
/// here — or to drop the foreign key from `assertion`, removing the guarantee for everything else.
/// Both are refused, so a cross-repository link lives here and in no other table, and no ordinary
/// `path` or `impact` traversal may reach it: a traversal that silently crossed repositories would
/// answer a question about A with facts about B whose freshness A cannot vouch for.
///
/// The same reasoning applies to every `*_snapshot` column and to `target_state_at_resolution`:
/// they name rows in a database this one does not own. The **source** side is the opposite and its
/// columns say so — `source_entity_id` and `source_state_at_resolution` *are* foreign keys, because
/// that end is local and verifiable. The asymmetry in the DDL is the point of the table.
///
/// # Why the target is a snapshot rather than a pointer
///
/// A bare `target_entity_id` is a pointer into a database Nerve cannot hold still. When B renames or
/// deletes the file, the link degrades to a dangling reference with nothing left to *name* what it
/// used to point at, and `contract_deleted`, `target_changed` and `contract_file_missing` become
/// indistinguishable — the failure `git_commit.changes_enumerated` exists to prevent, one row over.
/// So the kind, name, path and span the target had at resolution are copied in, and
/// `expected_target_repository_id` is the identity a re-validation is checked against, because
/// checking the *path* is what makes `target_repository_moved` undetectable.
///
/// Two version columns rather than one, because `contract_version_mismatch` is by definition a
/// disagreement between two numbers and one column cannot hold a disagreement.
///
/// # `withdrawn_at` + `status` rather than deletion
///
/// Both tables retire a row instead of removing it, for the reason the evidence model withdraws an
/// assertion rather than dropping it: **a row that vanished from the table cannot be reported as
/// having ended.** `registry_entry_removed` and `contract_deleted` are two of the twelve situations
/// row 13 must keep distinguishable, and both are reports made *from* the kept row. Hard deletion
/// is a separate, explicit purge. The `CHECK` on each table is what stops a status and a timestamp
/// from disagreeing — an active row with a withdrawal date, or a retired row without one, is
/// refused rather than reviewed later, in the manner of v7's `git_rename_hypothesis` CHECK.
///
/// `local_path` is the one field that is user-specific and absolute. It lives only in
/// `.nerve/nerve.db`, which `.gitignore` already covers.
///
/// No `IF NOT EXISTS`: no permanent table in this schema has it, and a migration that tolerated
/// re-application would hide a real double-apply.
const V8: &str = r#"
CREATE TABLE repo_registry (
    repo_id                 TEXT NOT NULL REFERENCES repository(repo_id),
    registry_id             TEXT NOT NULL,   -- stable local id for this entry
    expected_repository_id  TEXT NOT NULL,   -- the target's own repo_id, recorded at registration
    display_name            TEXT NOT NULL,   -- untrusted repository content (T7)
    local_path              TEXT NOT NULL,   -- user-specific and absolute; never tracked by git
    added_at                TEXT NOT NULL,
    last_seen_state         TEXT,
    last_seen_at            TEXT,
    availability_checked_at TEXT,
    status                  TEXT NOT NULL,   -- closed vocabulary: RegistryEntryStatus
    withdrawn_at            TEXT,            -- set when tombstoned; NULL while active
    PRIMARY KEY (repo_id, registry_id),
    -- A tombstone is a status and a moment. Either alone is half a fact.
    CHECK (
        (status = 'active'     AND withdrawn_at IS NULL)
     OR (status = 'tombstoned' AND withdrawn_at IS NOT NULL)
    ),
    -- A state observed at no time, or a time with no state, cannot be compared against anything.
    CHECK (
        (last_seen_state IS NULL     AND last_seen_at IS NULL)
     OR (last_seen_state IS NOT NULL AND last_seen_at IS NOT NULL)
    ),
    -- An empty identity or an empty path is a row that names nothing.
    CHECK (registry_id <> '' AND expected_repository_id <> '' AND local_path <> '')
);

CREATE INDEX idx_repo_registry_status ON repo_registry(repo_id, status);

CREATE TABLE contract_link (
    -- Surrogate key, in the manner of `observation`: the logical identity is the unique index
    -- below, and a link has no content-derived id of its own.
    link_id                       INTEGER PRIMARY KEY AUTOINCREMENT,
    repo_id                       TEXT NOT NULL REFERENCES repository(repo_id),

    -- Source. All local, all verifiable, and the foreign keys say so.
    source_repository_id          TEXT NOT NULL,
    source_state_at_resolution    TEXT NOT NULL REFERENCES repository_state(state_id),
    source_entity_id              TEXT REFERENCES entity(entity_id),  -- NULL when repo-to-repo
    source_kind_snapshot          TEXT,
    source_path                   TEXT NOT NULL,
    source_span                   TEXT NOT NULL,

    -- Target. A SNAPSHOT, because the target lives in another database and may move, change
    -- kind, or vanish. NONE of these is a foreign key, and that is the point of this table —
    -- contrast assertion.target_entity_id at schema.rs:97, which IS one.
    registry_entry_id             TEXT NOT NULL,
    expected_target_repository_id TEXT NOT NULL,
    target_state_at_resolution    TEXT,
    target_entity_id              TEXT,
    target_kind_snapshot          TEXT,
    target_name_snapshot          TEXT,
    target_path_snapshot          TEXT,
    target_span_snapshot          TEXT,

    -- The contract itself. Two version columns, because a mismatch needs two numbers.
    relation_semantics            TEXT NOT NULL,
    contract_kind                 TEXT NOT NULL,
    contract_identity             TEXT NOT NULL,   -- untrusted repository content (T7)
    expected_contract_version     TEXT,
    observed_contract_version     TEXT,

    -- How it was resolved, and what could not be.
    resolution_method             TEXT NOT NULL,   -- closed vocabulary: ContractResolutionMethod
    extractor_id                  TEXT NOT NULL,
    extractor_version             TEXT NOT NULL,
    evidence_details              TEXT,            -- JSON object, or NULL
    ambiguity                     TEXT,
    unsupported_reason            TEXT,            -- the form named, never silently dropped

    -- Lifecycle.
    first_seen_at                 TEXT NOT NULL,
    last_seen_at                  TEXT NOT NULL,
    withdrawn_at                  TEXT,
    status                        TEXT NOT NULL,   -- closed vocabulary: ContractLinkStatus

    FOREIGN KEY (repo_id, registry_entry_id) REFERENCES repo_registry(repo_id, registry_id),

    -- A withdrawal is a status and a moment, exactly as in repo_registry.
    CHECK (
        (status = 'active'    AND withdrawn_at IS NULL)
     OR (status = 'withdrawn' AND withdrawn_at IS NOT NULL)
    ),
    -- A target id with no snapshot is the dangling pointer this table exists to prevent: there
    -- would be nothing left to name what the link used to point at.
    CHECK (
        target_entity_id IS NULL
     OR (target_kind_snapshot IS NOT NULL
            AND target_name_snapshot IS NOT NULL
            AND target_path_snapshot IS NOT NULL)
    ),
    -- A form recorded as unsupported cannot also have been resolved.
    CHECK (
        unsupported_reason IS NULL
     OR (target_entity_id IS NULL AND target_path_snapshot IS NULL)
    ),
    -- Last seen before first seen is not a lifecycle. Timestamps are ISO-8601 UTC, so text
    -- comparison is chronological.
    CHECK (last_seen_at >= first_seen_at),
    CHECK (
        source_repository_id <> '' AND source_path <> '' AND source_span <> ''
        AND registry_entry_id <> '' AND expected_target_repository_id <> ''
        AND relation_semantics <> '' AND contract_kind <> '' AND contract_identity <> ''
        AND extractor_id <> '' AND extractor_version <> ''
    )
);

-- Logical uniqueness, for the reason v1 gives for observation: the surrogate key is an
-- autoincrement integer, so without this a re-index of an unchanged tree would append a duplicate
-- link on every run. `contract_identity` is deliberately NOT unique on its own — two repositories
-- declaring one identity is `duplicate_contract_identity`, a fact to report rather than to refuse.
CREATE UNIQUE INDEX idx_contract_link_identity ON contract_link(
    repo_id, registry_entry_id, contract_kind, contract_identity,
    source_path, source_span, resolution_method
);

CREATE INDEX idx_contract_link_registry ON contract_link(repo_id, registry_entry_id, status);
"#;

/// Schema v9 — Slice 14a. Three new tables, no `ALTER` of any existing one.
///
/// # `memory.subject_entity_id_snapshot` is deliberately **not** a foreign key
///
/// This is the correction row 14 was rewritten for, and it is mechanical rather than a matter of
/// taste. `entity` rows are **routinely deleted**: [`crate::prune::prune_orphans`] issues
/// `DELETE FROM entity WHERE …` (`prune.rs:376`, and again scoped at `:440`), and
/// `deleting_a_file_removes_its_entities_assertions_and_observations`
/// (`nerve-index/tests/incremental.rs:290`) pins that as required behaviour. With
/// `PRAGMA foreign_keys=ON` set on every connection (`db.rs:37`), a memory row holding a foreign key
/// into `entity` leaves exactly two outcomes and both are unacceptable:
///
/// - **the delete is refused** — a human note about a file now blocks re-indexing that file, so
///   writing memory would break indexing; or
/// - **the delete cascades** — a routine re-index silently destroys the human's note, which is the
///   one thing a memory feature must never do.
///
/// So memory stores a **snapshot of its subject** and resolves the live one at query time, reporting
/// `resolved` · `resolved_through_identity_link` · `missing` · `ambiguous` ·
/// `repository_state_unavailable` (`nerve_core::vocab::MemorySubjectResolution`). The snapshot is
/// what lets a pruned subject still be *named*: without the kind, name, path and selector the
/// human used, a record whose subject is gone would be a note about nothing.
///
/// `contract_link` took exactly this shape at [`V8`] for exactly this reason, one table over. The
/// difference is which constraint is impossible: there the target lives in another database, here
/// the subject lives in a table this database prunes.
///
/// **`memory_citation` gets the identical treatment**, because a citation into a pruned entity has
/// the identical problem. `cited_entity_id_snapshot` is nullable — a citation may name a path and a
/// span with no entity at all — and the `CHECK` refuses an entity id with no snapshot beside it,
/// which is the dangling pointer [`V8`]'s equivalent `CHECK` exists to prevent.
///
/// # What *is* a foreign key here, and why
///
/// `repo_id`, `anchor_state_id` and `cited_at_state` are real foreign keys, because nothing deletes
/// `repository` or `repository_state` — no statement in this crate does, and the state a record was
/// anchored to is the thing query-time staleness is measured against, so a dangling anchor would
/// make `potentially_stale` unanswerable rather than merely imprecise.
///
/// `memory_citation` and `memory_event` carry composite foreign keys onto `(repo_id, memory_id)`,
/// and `memory.supersedes_memory_id` carries one onto the same pair. Those are safe *because
/// nothing deletes a memory row*: there is no `nerve memory delete`, and the row's retirement is a
/// status plus a timestamp in the manner of [`V8`]'s tombstones. The composite form also states
/// something worth stating — a citation, an event or a supersession may not cross repositories.
///
/// # Supersession is stored in **one** direction
///
/// The plan's sketch had both `supersedes` and `superseded_by`. Two independently writable
/// directions of one fact can disagree with nothing in the schema to notice — the "two writable
/// copies of one fact" row 13 §4.1 already rejected for cross-repository links. Only
/// `supersedes_memory_id` exists; the inverse is a query. `idx_memory_supersedes` is unique, so at
/// most one record supersedes any given record and the derived inverse is single-valued rather than
/// a set a reader would have to interpret.
///
/// The `CHECK` refuses a record that supersedes itself, which would otherwise be a cycle of length
/// one that every walk of the chain has to defend against. Longer cycles are **not** refused here
/// and cannot be: a `CHECK` sees one row. They are detected and reported by the read model, in the
/// manner supersession cycles are already *detected, counted and never suppressed*.
///
/// # `status` holds four values and none of them is derived
///
/// `proposed` · `active` · `superseded` · `invalidated` (`nerve_core::vocab::MemoryStatus`).
/// `potentially_stale`, `conflicted` and `multiple_active` are `MemoryView`s computed at read time
/// and never written: keeping a stored copy true would need a writer, and the writer would be a
/// query. The vocabulary is closed in Rust rather than in SQL, as it is for every other vocabulary
/// column in this schema; what the `CHECK`s enforce is the *correlation* between a status and its
/// timestamps, which is the part a vocabulary cannot state.
///
/// # `author_label` is a local label, not an identity
///
/// Nerve has no accounts, no network and no identity provider. The column records what the caller
/// said it was and nothing verified it, which is why it is named `author_label` rather than
/// `author`: a field called `author` in a product with no accounts invites being read as
/// authentication. Untrusted string on T7's terms, exactly like `repo_registry.display_name`.
///
/// # `memory_event` is append-only, and that is enforced by there being no writer
///
/// No `DELETE` and no `UPDATE` statement against `memory_event` exists in the workspace, and a
/// source scan asserts it. The alternative — a `BEFORE DELETE … RAISE(ABORT)` trigger — was
/// considered and declined: it would state the same thing less directly (a trigger can be dropped
/// by a later migration, and a scan cannot be satisfied by anything except the absence of the code),
/// and it would make any future whole-database purge require a v10 migration to remove it. The
/// guarantee this row needs is *"no code path deletes an event"*, which the scan says outright.
///
/// `operation` is an **open** string, deliberately. Slice 14a is storage; the lifecycle commands
/// that name the operations are 14b's, and inventing a closed vocabulary here would pin names to
/// verbs that do not exist yet. `from_status` is `NULL` on the event that created a record and set
/// on every later one; a status-preserving event (a citation added to an active record) is
/// legitimate and is not refused.
///
/// No `IF NOT EXISTS`: no permanent table in this schema has it, and a migration that tolerated
/// re-application would hide a real double-apply.
const V9: &str = r#"
CREATE TABLE memory (
    memory_id                  TEXT NOT NULL,
    repo_id                    TEXT NOT NULL REFERENCES repository(repo_id),

    -- The subject, as it was when the human wrote it. NONE of these is a foreign key, and that is
    -- the point of this table: entity rows are pruned on re-index, and a note must outlive its
    -- subject. Contrast assertion.source_entity_id at schema.rs:97, which IS one.
    subject_entity_id_snapshot TEXT NOT NULL,
    subject_kind_snapshot      TEXT NOT NULL,
    subject_name_snapshot      TEXT NOT NULL,
    subject_path_snapshot      TEXT NOT NULL,   -- '' for the repository entity, which is no file
    subject_selector_snapshot  TEXT NOT NULL,   -- how the human named it, kept verbatim
    anchor_state_id            TEXT NOT NULL REFERENCES repository_state(state_id),

    scope                      TEXT NOT NULL,   -- caller-supplied grouping label, never interpreted
    claim_key                  TEXT,            -- NULL means this record answers no named claim

    content                    TEXT NOT NULL,   -- the human's own sentence, never rewritten
    author_label               TEXT NOT NULL,   -- a LOCAL LABEL, NOT AN IDENTITY (T7)
    created_at                 TEXT NOT NULL,

    status                     TEXT NOT NULL,   -- closed vocabulary: MemoryStatus, four values
    supersedes_memory_id       TEXT,            -- one direction only; the inverse is derived

    invalidated_at             TEXT,
    invalidation_reason        TEXT,

    PRIMARY KEY (repo_id, memory_id),
    FOREIGN KEY (repo_id, supersedes_memory_id) REFERENCES memory(repo_id, memory_id),

    -- **The four stored statuses, enumerated here and not only in Rust.** This schema usually closes
    -- a vocabulary in `nerve-core` and stores the text (V4's doc comment states that), and the usual
    -- way is right when the column merely *names* a value. It is wrong here, because an invariant
    -- rests on the column's domain: `potentially_stale`, `conflicted` and `multiple_active` are
    -- **derived at query time and must never be stored**, and a vocabulary closed only in Rust
    -- leaves a raw-SQL writer free to store one. It would then fail on the next read through
    -- `MemoryStatus::FromStr` -- loudly, but *after* the row is on disk, which is a repair job
    -- rather than a refusal.
    --
    -- V7's `git_rename_hypothesis` set this precedent: it enumerates 'exact_content' and
    -- 'similar_content' in SQL precisely because an invariant -- evidence is never blended --
    -- depended on the domain rather than on the spelling. The cost is stated: a fifth *stored*
    -- status needs a table rebuild in a later migration. That is the intended price. A fifth stored
    -- status is a change to what a memory record can be, and it should not be reachable by adding a
    -- Rust variant and finding the database already accepted it.
    CHECK (status IN ('proposed', 'active', 'superseded', 'invalidated')),
    -- An ending is a status and a moment. A record that says it stopped being true without saying
    -- when cannot be reported as having ended, and a live record carrying an ending contradicts
    -- itself -- the pairing repo_registry's tombstone CHECK refuses, one table over.
    CHECK (
        (status =  'invalidated' AND invalidated_at IS NOT NULL)
     OR (status <> 'invalidated' AND invalidated_at IS NULL)
    ),
    -- A reason for an ending that never happened is not a fact about anything.
    CHECK (invalidation_reason IS NULL OR invalidated_at IS NOT NULL),
    -- A record cannot replace itself: that is a cycle of length one, and the only cycle a CHECK
    -- can see. Longer ones are detected by the read model rather than refused here.
    CHECK (supersedes_memory_id IS NULL OR supersedes_memory_id <> memory_id),
    -- An empty claim key is not a claim key. It would silently gather every keyless record into
    -- one competing claim and report ordinary notes as contradictions.
    CHECK (claim_key IS NULL OR claim_key <> ''),
    -- A row that names nothing, is about nothing, or says nothing. `subject_path_snapshot` is
    -- deliberately absent: the repository entity has no file path and records '' honestly.
    CHECK (
        memory_id <> '' AND subject_entity_id_snapshot <> '' AND subject_kind_snapshot <> ''
        AND subject_name_snapshot <> '' AND subject_selector_snapshot <> ''
        AND scope <> '' AND content <> '' AND author_label <> '' AND status <> ''
    )
);

CREATE INDEX idx_memory_subject ON memory(repo_id, subject_entity_id_snapshot, status);
CREATE INDEX idx_memory_scope   ON memory(repo_id, scope, status);

-- The grouping a conflict is decided over: repository + subject + scope + claim_key. Partial,
-- because a record with no claim key competes with nothing and belongs in no claim group.
CREATE INDEX idx_memory_claim ON memory(repo_id, subject_entity_id_snapshot, scope, claim_key)
    WHERE claim_key IS NOT NULL;

-- One direction is stored, so the inverse must be a function rather than a set: at most one record
-- may supersede any given record. Without this, "what replaced it" would have several answers and
-- deriving the inverse would mean choosing between them.
CREATE UNIQUE INDEX idx_memory_supersedes ON memory(repo_id, supersedes_memory_id)
    WHERE supersedes_memory_id IS NOT NULL;

CREATE TABLE memory_citation (
    -- Surrogate key, in the manner of `observation` and `contract_link`: a citation has no
    -- content-derived identity of its own, and the same passage may honestly be cited twice.
    citation_id              INTEGER PRIMARY KEY AUTOINCREMENT,
    repo_id                  TEXT NOT NULL REFERENCES repository(repo_id),
    memory_id                TEXT NOT NULL,

    -- A SNAPSHOT, for the reason the subject is one: the cited entity may be pruned.
    cited_entity_id_snapshot TEXT,             -- NULL when the citation names a place, not a thing
    cited_kind_snapshot      TEXT,
    cited_name_snapshot      TEXT,
    cited_path_snapshot      TEXT NOT NULL,
    cited_span_snapshot      TEXT,             -- 'start_line:end_line', or NULL for a whole file
    cited_at_state           TEXT NOT NULL REFERENCES repository_state(state_id),
    created_at               TEXT NOT NULL,

    FOREIGN KEY (repo_id, memory_id) REFERENCES memory(repo_id, memory_id),

    -- An entity id with no snapshot beside it is the dangling pointer this table exists to
    -- prevent: once the entity is pruned there would be nothing left to name what was cited.
    CHECK (
        cited_entity_id_snapshot IS NULL
     OR (cited_entity_id_snapshot <> ''
            AND cited_kind_snapshot IS NOT NULL AND cited_kind_snapshot <> ''
            AND cited_name_snapshot IS NOT NULL AND cited_name_snapshot <> '')
    ),
    -- A citation with no place is not a citation.
    CHECK (cited_path_snapshot <> '' AND (cited_span_snapshot IS NULL OR cited_span_snapshot <> ''))
);

CREATE INDEX idx_memory_citation_memory ON memory_citation(repo_id, memory_id);
CREATE INDEX idx_memory_citation_path   ON memory_citation(repo_id, cited_path_snapshot);

CREATE TABLE memory_event (
    event_id    INTEGER PRIMARY KEY AUTOINCREMENT,
    repo_id     TEXT NOT NULL REFERENCES repository(repo_id),
    memory_id   TEXT NOT NULL,
    at          TEXT NOT NULL,
    operation   TEXT NOT NULL,   -- open string: 14b's commands own the verbs
    from_status TEXT,            -- NULL on the event that created the record
    to_status   TEXT NOT NULL,
    note        TEXT,

    FOREIGN KEY (repo_id, memory_id) REFERENCES memory(repo_id, memory_id),

    -- An event that names no operation, no moment or no resulting status records nothing.
    CHECK (at <> '' AND operation <> '' AND to_status <> ''),
    CHECK (from_status IS NULL OR from_status <> '')
);

CREATE INDEX idx_memory_event_memory ON memory_event(repo_id, memory_id, event_id);
"#;

/// The `memory.scope` domain v10 introduces, **pinned as literals rather than read from
/// `MemoryScope::ALL`**.
///
/// A migration must mean the same thing forever, which is the reason [`V4_SOURCE_TYPE`] is pinned
/// one screen down and the reason [`V1`] is immutable. If a sixth scope is ever admitted, v10 must
/// still refuse it — because the `CHECK` v10 writes is frozen SQL text that refuses it, and a
/// validation reading the live constant would start disagreeing with the constraint it is guarding.
/// `nerve-store/tests/schema.rs` asserts the two agree today, in the manner
/// `nerve-index/tests/graph.rs` does for v4.
const V10_SCOPES: [&str; 4] = ["implementation", "interface", "operations", "process"];

/// The `memory_event.operation` domain v10 introduces. Pinned for the reason [`V10_SCOPES`] is.
const V10_OPERATIONS: [&str; 5] = [
    "proposed",
    "confirmed",
    "superseded",
    "invalidated",
    "cited",
];

/// Schema v10 — Slice 14b-i. Two `CHECK`s, and the table rebuild they cost.
///
/// # What is being closed, and why the argument is not the one v9 used for `status`
///
/// v9 closed `status` in SQL because a *derived* value (`potentially_stale`) could be *stored*.
/// That confusion does not exist for `scope`, so that argument does not transfer and is not reused.
/// The argument that applies is about what `scope` is load-bearing for: `multiple_active` groups by
/// `(repository, subject, scope)` and `conflicted` by that plus `claim_key`, so against a free-form
/// column **a typo silently suppresses a conflict report** — `operations` and `opertions` land in
/// different groups and two records answering one named claim are reported as unrelated notes. No
/// test can catch that, because both spellings are legal. And `--scope opertions` returns zero
/// records, which reads as *there are no notes* rather than *there is no such scope*; `absence is
/// not zero` is the rule 7c-ii's `doctor` and 7b's unresolved account exist to enforce.
///
/// `memory_event.operation` is closed in the same step for 14d's invariant: **every event must be
/// renderable**, and the interface's vocabulary guard can only guard a vocabulary it knows exists —
/// 12c-iv found eight unmirrored vocabularies for which the guard *could not fail*.
///
/// # Why this is a rebuild, and why **v7's rebuild is not the precedent it looks like**
///
/// SQLite has no `ALTER TABLE … ADD CONSTRAINT`, so a `CHECK` arrives only by the documented
/// create-copy-drop-rename. [`V7`] did exactly that for `git_rename_hypothesis` — and it could,
/// because **nothing referenced that table**. `memory` is the opposite: `memory_citation` and
/// `memory_event` both carry `FOREIGN KEY (repo_id, memory_id) REFERENCES memory(…)`, and `memory`
/// references *itself* through `supersedes_memory_id`. Three facts then close off v7's shape:
///
/// 1. `PRAGMA foreign_keys=ON` is set on every connection (`db.rs:37`), and the pragma is a
///    **documented no-op inside a transaction** — measured, not assumed: it still reads `1` after
///    being set to `OFF` inside `BEGIN`. Each step of [`MIGRATIONS`] runs in a transaction, so the
///    pragma cannot be turned off here.
/// 2. With foreign keys on, `DROP TABLE` performs an **implicit `DELETE FROM`** first, which
///    orphans every citation and event row.
/// 3. `PRAGMA defer_foreign_keys=ON` — the mechanism that *does* work inside a transaction — was
///    tried and **does not rescue v7's shape**. Deferring moves the failure to `COMMIT`: the
///    implicit delete increments SQLite's deferred-violation counter, and renaming the replacement
///    table into place never decrements it, so the commit fails with `FOREIGN KEY constraint
///    failed` *while `PRAGMA foreign_key_check` reports zero rows*. The database is consistent and
///    the commit is refused anyway, which is the worst available failure: it lands outside the step
///    that caused it and the obvious diagnostic cannot see it.
///
/// # The procedure this step actually uses
///
/// Immediate foreign keys are checked **at the conclusion of each statement**, not per row. So the
/// rebuild is ordered such that every statement leaves the database consistent at its own boundary,
/// and enforcement stays immediate — a mistake then fails *inside* this step rather than at commit,
/// which is why `defer_foreign_keys` is deliberately **not** set:
///
/// 1. park `memory`, `memory_event` and `memory_citation` rows in `TEMP` tables (the [`migrate_v3`]
///    precedent for staging inside a step);
/// 2. **drop** `memory_event`, which is being rebuilt anyway — emptying it instead would mean
///    writing a `DELETE` statement against that table, and the absence of any such statement from
///    the whole workspace is exactly how v9 chose to enforce append-only. Then empty
///    `memory_citation`, which is not being rebuilt, and then `memory` itself. A whole-table
///    `DELETE` satisfies the self-reference at its conclusion; a single-row delete of a superseded
///    record would not, which is why the delete is unqualified;
/// 3. drop `memory` — now empty, so the implicit delete orphans nothing;
/// 4. **re-create them under their own names**, so there is no `ALTER TABLE … RENAME` anywhere in
///    this step. That avoids a second trap, also measured: since SQLite 3.25 a rename rewrites
///    references in *other* tables, so `ALTER TABLE memory RENAME TO memory_old` silently repoints
///    `memory_citation`'s foreign key at `"memory_old"` — a child table quietly attached to a
///    scratch table that is about to be dropped;
/// 5. re-insert every parked row and drop the staging tables.
///
/// `memory_citation` is emptied and refilled rather than rebuilt: its rows must not be present when
/// their parent is dropped, and its DDL needs no change. Its `citation_id` values are re-inserted
/// explicitly, so no surrogate key moves.
///
/// [`migrate_v10`] runs `PRAGMA foreign_key_check` afterwards, which is belt to the ordering's
/// braces rather than the guarantee itself — as the deferral finding above shows, it reports zero
/// rows against a database whose commit is about to be refused, so it can confirm the ordering and
/// cannot substitute for it.
///
/// # Out-of-domain rows are **refused**, not repaired
///
/// v9 stored `scope` opaque, so a v9 database may hold anything (14a's own tests used `"file"` and
/// `"repository"`). [`migrate_v10`] therefore checks both columns before touching a table and
/// refuses with the offending distinct values named, rather than dropping the rows or rewriting
/// them to a default. Memory is the only thing in this database re-indexing cannot rebuild, so a
/// migration that silently edited a human's note would be refused on the same ground as a delete
/// verb. That check is Rust, which is why v10 is a [`Step::Rust`] rather than a [`Step::Sql`].
const V10: &str = r#"
CREATE TEMP TABLE memory_v10_rows AS SELECT * FROM memory;
CREATE TEMP TABLE memory_event_v10_rows AS SELECT * FROM memory_event;
CREATE TEMP TABLE memory_citation_v10_rows AS SELECT * FROM memory_citation;

-- Children first. A parent row may not be dropped while a child row points at it, and the check
-- happens at the conclusion of each statement -- so the order of these four is the whole trick.
--
-- The event table is **dropped rather than emptied**, and that is not a stylistic choice: no
-- `DELETE` statement against this table exists anywhere in this workspace, which is how
-- "append-only" is enforced (a source scan in `tests/memory.rs`, chosen over a trigger at v9).
-- Writing one here to save a line would satisfy the migration and defeat the guard. Dropping the
-- table is what a rebuild is; every row is parked above and re-inserted below, and the v9->v10 test
-- compares all three tables as a full serialisation before and after, so the history is carried
-- across rather than trusted to be.
DROP TABLE memory_event;

DELETE FROM memory_citation;
-- Unqualified on purpose: `memory` references itself, and only a whole-table delete is consistent
-- at its own conclusion. `DELETE FROM memory WHERE memory_id = ...` on a superseded record is not.
DELETE FROM memory;

DROP TABLE memory;

CREATE TABLE memory (
    memory_id                  TEXT NOT NULL,
    repo_id                    TEXT NOT NULL REFERENCES repository(repo_id),

    subject_entity_id_snapshot TEXT NOT NULL,
    subject_kind_snapshot      TEXT NOT NULL,
    subject_name_snapshot      TEXT NOT NULL,
    subject_path_snapshot      TEXT NOT NULL,
    subject_selector_snapshot  TEXT NOT NULL,
    anchor_state_id            TEXT NOT NULL REFERENCES repository_state(state_id),

    scope                      TEXT NOT NULL,   -- closed vocabulary: MemoryScope, four values
    claim_key                  TEXT,

    content                    TEXT NOT NULL,
    author_label               TEXT NOT NULL,   -- a LOCAL LABEL, NOT AN IDENTITY (T7)
    created_at                 TEXT NOT NULL,

    status                     TEXT NOT NULL,   -- closed vocabulary: MemoryStatus, four values
    supersedes_memory_id       TEXT,

    invalidated_at             TEXT,
    invalidation_reason        TEXT,

    PRIMARY KEY (repo_id, memory_id),
    FOREIGN KEY (repo_id, supersedes_memory_id) REFERENCES memory(repo_id, memory_id),

    -- **New in v10, and the reason this table was rebuilt.** `scope` is in both derived-view
    -- grouping keys, so a free-form column lets a typo split one group in two and report two
    -- records answering the same named claim as unrelated notes -- a false negative on the one
    -- contradiction claim this feature makes, which no test can catch because both spellings are
    -- legal. Closing it in Rust alone is not enough: `MemoryScope::FromStr` fails on the next
    -- *read*, which is a repair job rather than a refusal. Same reasoning as the `status` CHECK
    -- below, arrived at from a different direction.
    CHECK (scope IN ('implementation', 'interface', 'operations', 'process')),
    CHECK (status IN ('proposed', 'active', 'superseded', 'invalidated')),
    CHECK (
        (status =  'invalidated' AND invalidated_at IS NOT NULL)
     OR (status <> 'invalidated' AND invalidated_at IS NULL)
    ),
    CHECK (invalidation_reason IS NULL OR invalidated_at IS NOT NULL),
    CHECK (supersedes_memory_id IS NULL OR supersedes_memory_id <> memory_id),
    CHECK (claim_key IS NULL OR claim_key <> ''),
    -- `scope <> ''` and `status <> ''` are gone from this list, and deliberately: the enumerations
    -- above already exclude the empty string, and restating it would be a second place to keep
    -- true. Every other column keeps its v9 emptiness check unchanged.
    CHECK (
        memory_id <> '' AND subject_entity_id_snapshot <> '' AND subject_kind_snapshot <> ''
        AND subject_name_snapshot <> '' AND subject_selector_snapshot <> ''
        AND content <> '' AND author_label <> ''
    )
);

CREATE INDEX idx_memory_subject ON memory(repo_id, subject_entity_id_snapshot, status);
CREATE INDEX idx_memory_scope   ON memory(repo_id, scope, status);

CREATE INDEX idx_memory_claim ON memory(repo_id, subject_entity_id_snapshot, scope, claim_key)
    WHERE claim_key IS NOT NULL;

CREATE UNIQUE INDEX idx_memory_supersedes ON memory(repo_id, supersedes_memory_id)
    WHERE supersedes_memory_id IS NOT NULL;

CREATE TABLE memory_event (
    event_id    INTEGER PRIMARY KEY AUTOINCREMENT,
    repo_id     TEXT NOT NULL REFERENCES repository(repo_id),
    memory_id   TEXT NOT NULL,
    at          TEXT NOT NULL,
    operation   TEXT NOT NULL,   -- closed vocabulary: MemoryOperation, five values
    from_status TEXT,            -- NULL on the event that created the record
    to_status   TEXT NOT NULL,
    note        TEXT,

    FOREIGN KEY (repo_id, memory_id) REFERENCES memory(repo_id, memory_id),

    -- **New in v10.** 14a left this open because 14b's commands owned the verbs; they exist now.
    -- One value per mutating operation and no more, so that every event is renderable and the
    -- interface's vocabulary guard has something it can fail against. `cited` is the one that is
    -- not a transition: its event carries from_status = to_status, which stays legitimate below.
    CHECK (operation IN ('proposed', 'confirmed', 'superseded', 'invalidated', 'cited')),
    CHECK (at <> '' AND to_status <> ''),
    CHECK (from_status IS NULL OR from_status <> '')
);

CREATE INDEX idx_memory_event_memory ON memory_event(repo_id, memory_id, event_id);

-- Columns named on both sides rather than `SELECT *`, so a future column added to one of these
-- tables cannot be copied into the wrong slot by position.
INSERT INTO memory
    (memory_id, repo_id, subject_entity_id_snapshot, subject_kind_snapshot,
     subject_name_snapshot, subject_path_snapshot, subject_selector_snapshot, anchor_state_id,
     scope, claim_key, content, author_label, created_at, status, supersedes_memory_id,
     invalidated_at, invalidation_reason)
SELECT memory_id, repo_id, subject_entity_id_snapshot, subject_kind_snapshot,
       subject_name_snapshot, subject_path_snapshot, subject_selector_snapshot, anchor_state_id,
       scope, claim_key, content, author_label, created_at, status, supersedes_memory_id,
       invalidated_at, invalidation_reason
  FROM memory_v10_rows;

INSERT INTO memory_event
    (event_id, repo_id, memory_id, at, operation, from_status, to_status, note)
SELECT event_id, repo_id, memory_id, at, operation, from_status, to_status, note
  FROM memory_event_v10_rows;

INSERT INTO memory_citation
    (citation_id, repo_id, memory_id, cited_entity_id_snapshot, cited_kind_snapshot,
     cited_name_snapshot, cited_path_snapshot, cited_span_snapshot, cited_at_state, created_at)
SELECT citation_id, repo_id, memory_id, cited_entity_id_snapshot, cited_kind_snapshot,
       cited_name_snapshot, cited_path_snapshot, cited_span_snapshot, cited_at_state, created_at
  FROM memory_citation_v10_rows;

DROP TABLE memory_v10_rows;
DROP TABLE memory_event_v10_rows;
DROP TABLE memory_citation_v10_rows;
"#;

/// Refuse the upgrade if `column` holds anything outside `admitted`, naming what it found.
///
/// The distinct offending values, ordered, and nothing else: a count would tell a human that
/// something is wrong without telling them what to fix, and the row ids would name rows they would
/// then have to go and read. The values are what they typed.
fn refuse_out_of_domain(
    conn: &Connection,
    table: &'static str,
    column: &'static str,
    admitted: &[&str],
) -> Result<()> {
    let list = admitted
        .iter()
        .map(|value| format!("'{value}'"))
        .collect::<Vec<_>>()
        .join(", ");
    let mut stmt = conn.prepare(&format!(
        "SELECT DISTINCT {column} FROM {table} WHERE {column} NOT IN ({list}) ORDER BY 1"
    ))?;
    let mut rows = stmt.query([])?;
    let mut found = Vec::new();
    while let Some(row) = rows.next()? {
        found.push(format!("{:?}", row.get::<_, String>(0)?));
    }
    if found.is_empty() {
        return Ok(());
    }
    Err(StoreError::MigrationDomain {
        version: 10,
        table,
        column,
        found: found.join(", "),
        admitted: list,
    })
}

/// Check both domains, then rebuild the two tables that now enumerate them.
///
/// The check runs **before** any DDL, so a database that will be refused is never partially taken
/// apart — even though the step's transaction would roll it back anyway. That ordering is what
/// makes the refusal a refusal rather than a rollback: nothing about the failure depends on the
/// transaction working.
///
/// `PRAGMA foreign_key_check` afterwards is the belt to the statement ordering's braces. It is
/// deliberately not the guarantee: it reports rows that violate a constraint *now*, and it cannot
/// see SQLite's deferred-violation counter, which is exactly what defeats the naive rebuild [`V10`]
/// describes.
fn migrate_v10(conn: &Connection) -> Result<()> {
    refuse_out_of_domain(conn, "memory", "scope", &V10_SCOPES)?;
    refuse_out_of_domain(conn, "memory_event", "operation", &V10_OPERATIONS)?;

    conn.execute_batch(V10)?;

    let violations: i64 =
        conn.query_row("SELECT count(*) FROM pragma_foreign_key_check", [], |row| {
            row.get(0)
        })?;
    if violations > 0 {
        return Err(StoreError::Memory(format!(
            "the v10 rebuild left {violations} foreign key violations; the upgrade is rolled back"
        )));
    }
    Ok(())
}

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
const MIGRATIONS: [(i64, &str, Step); 10] = [
    (1, SCHEMA_V1_DESCRIPTION, Step::Sql(V1)),
    (2, SCHEMA_V2_DESCRIPTION, Step::Sql(V2)),
    (3, SCHEMA_V3_DESCRIPTION, Step::Rust(migrate_v3)),
    (4, SCHEMA_V4_DESCRIPTION, Step::Rust(migrate_v4)),
    (5, SCHEMA_V5_DESCRIPTION, Step::Sql(V5)),
    (6, SCHEMA_V6_DESCRIPTION, Step::Sql(V6)),
    (7, SCHEMA_V7_DESCRIPTION, Step::Sql(V7)),
    (8, SCHEMA_V8_DESCRIPTION, Step::Sql(V8)),
    (9, SCHEMA_V9_DESCRIPTION, Step::Sql(V9)),
    (10, SCHEMA_V10_DESCRIPTION, Step::Rust(migrate_v10)),
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
