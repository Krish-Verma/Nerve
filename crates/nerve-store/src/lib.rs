//! Nerve storage layer.
//!
//! Owns the SQLite schema, migrations, every SQL statement in the product, FTS5 symbol
//! search, and the derivation of `assertion_state`.
//!
//! The load-bearing boundary in this crate is that [`derive`] is the only writer of
//! `assertion_state`, and that what it writes is a pure function of `observation`. [`write`]
//! contains no statement that touches it. The whole-table rebuild is the reference evaluation of
//! that function; the scoped one is checked against it (ADR-0006, Slice 3b).
//!
//! Graph reads — selector resolution, bounded path traversal, evidence assembly — live in
//! [`select`] and [`graph`] rather than in a surface crate, so the CLI, the Slice 4 server and
//! the Slice 8 MCP tools all answer the same question the same way.

#![forbid(unsafe_code)]
#![deny(missing_docs)]

pub mod db;
pub mod derive;
pub mod diagnose;
pub mod dump;
pub mod error;
pub mod facts;
pub mod freshness;
pub mod gaps;
pub mod graph;
pub mod history;
pub mod impact;
pub mod prune;
pub mod query;
pub mod schema;
pub mod select;
pub mod write;

pub use db::{database_bytes, open, open_in_memory};
pub use derive::{derive_assertion_state_for, rebuild_assertion_state};
pub use diagnose::{diagnose, DatabaseDiagnostics};
pub use dump::canonical_dump;
pub use error::{Result, StoreError};
pub use facts::{delete_module_facts, load_module_facts, upsert_module_facts, ModuleFactsRow};
pub use freshness::{FileProbe, FileProber, Freshness, FreshnessCache};
pub use gaps::{
    gaps, CoverageEvidence, CoverageRunRef, GapQuery, GapReport, GapRow, GapTotals, SymbolCoverage,
};
pub use graph::{
    explain, find_paths, neighbourhood, AssertionEvidence, Direction, EdgeDirection, GraphPath,
    NeighbourEdge, NeighbourNode, NeighbourhoodQuery, NeighbourhoodReport, ObservationEvidence,
    PathHop, PathQuery, PathReport, WhyDirection, WhyQuery, WhyReport,
};
pub use history::{
    change_frequency, changes_for_commit, cochange, commit_by_oid, commit_log,
    commits_touching_path, earlier_changes_may_exist, first_last_observed, history_freshness,
    history_ingest, history_totals, insert_changes, insert_commit, insert_renames,
    recorded_commit_oids, renames_touching_path, state_diff, upsert_history_ingest,
    ChangeFrequency, ChangeFrequencyRow, ChangeRow, Cochange, CochangeRow, CommitRow, CurrentTree,
    EarlierHistoryUnavailable, FirstLastObserved, HistoryFreshnessReport, HistoryTotals, IngestRow,
    PathChange, RenameRow, StateDiff, StateDiffLimits, StateDiffReport,
    COCHANGE_IS_NOT_A_DEPENDENCY, CURRENT_TREE_BASIS,
};
pub use impact::{
    impact, ImpactQuery, ImpactReport, ImpactRow, ImpactTotals, UnresolvedAccount,
    DEFAULT_RELATIONS as DEFAULT_IMPACT_RELATIONS, UNCATEGORISED as UNCATEGORISED_UNRESOLVED,
};
pub use prune::{
    delete_claims_sourced_at, delete_directory_containment, delete_extractor_file_rows,
    delete_file_rows, delete_observations_at, prune_orphans, prune_orphans_scoped, RemovalCounts,
    TouchedRows,
};
pub use query::{
    entity_relation_counts, environments_for_extractor, importers_of, indexed_content_hash,
    observations_for_assertions, occurrences_of, path_is_indexed, repository, repository_state,
    search_entities, status, symbol_spans_in_file, unresolved_entities, EntityRelationCounts,
    ExtractorRunSummary, ObservationKey, ObservationPayload, OccurrenceRow, RepositoryInfo,
    SearchHit, StatusReport, SymbolSpanRow, UnresolvedEntity,
};
pub use schema::{migrate, schema_version, SCHEMA_VERSION};
pub use select::{
    entity_by_id, history_path_refusal, qualifiers, resolve_selector, selector_shape, EntityRef,
    HistoryPathRefusal, InvalidSelector, Qualifier, Selection, SelectorKind, SelectorRefusal,
    SelectorShape, SUGGESTION_LIMIT,
};
pub use write::{
    begin_extractor_run, finish_extractor_run, insert_identity_link, persist_batch,
    upsert_repository, upsert_repository_state, RepositoryRow, RepositoryStateRow,
};

pub use rusqlite::Connection;
