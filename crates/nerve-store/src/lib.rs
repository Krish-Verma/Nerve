//! Nerve storage layer.
//!
//! Owns the SQLite schema, migrations, every SQL statement in the product, FTS5 symbol
//! search, and the derivation of `assertion_state`.
//!
//! The load-bearing boundary in this crate is that [`derive::rebuild_assertion_state`] is the
//! only writer of `assertion_state`, and it rebuilds the table from scratch. [`write`]
//! contains no statement that touches it.
//!
//! Graph reads — selector resolution, bounded path traversal, evidence assembly — live in
//! [`select`] and [`graph`] rather than in a surface crate, so the CLI, the Slice 4 server and
//! the Slice 8 MCP tools all answer the same question the same way.

#![forbid(unsafe_code)]
#![deny(missing_docs)]

pub mod db;
pub mod derive;
pub mod dump;
pub mod error;
pub mod facts;
pub mod freshness;
pub mod graph;
pub mod prune;
pub mod query;
pub mod schema;
pub mod select;
pub mod write;

pub use db::{database_bytes, open, open_in_memory};
pub use derive::rebuild_assertion_state;
pub use dump::canonical_dump;
pub use error::{Result, StoreError};
pub use facts::{delete_module_facts, load_module_facts, upsert_module_facts, ModuleFactsRow};
pub use freshness::{FileProbe, FileProber, Freshness, FreshnessCache};
pub use graph::{
    explain, find_paths, AssertionEvidence, Direction, EdgeDirection, GraphPath,
    ObservationEvidence, PathHop, PathQuery, PathReport, WhyDirection, WhyQuery, WhyReport,
};
pub use prune::{
    delete_directory_containment, delete_file_rows, prune_orphans, restamp_state, RemovalCounts,
};
pub use query::{
    importers_of, search_entities, status, ExtractorRunSummary, SearchHit, StatusReport,
};
pub use schema::{migrate, schema_version, SCHEMA_VERSION};
pub use select::{
    entity_by_id, resolve_selector, EntityRef, Selection, SelectorKind, SUGGESTION_LIMIT,
};
pub use write::{
    begin_extractor_run, finish_extractor_run, insert_identity_link, persist_batch,
    upsert_repository, upsert_repository_state, RepositoryRow, RepositoryStateRow,
};

pub use rusqlite::Connection;
