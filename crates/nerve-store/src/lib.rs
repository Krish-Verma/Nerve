//! Nerve storage layer.
//!
//! Owns the SQLite schema, migrations, every SQL statement in the product, FTS5 symbol
//! search, and the derivation of `assertion_state`.
//!
//! The load-bearing boundary in this crate is that [`derive::rebuild_assertion_state`] is the
//! only writer of `assertion_state`, and it rebuilds the table from scratch. [`write`]
//! contains no statement that touches it.

#![forbid(unsafe_code)]
#![deny(missing_docs)]

pub mod db;
pub mod derive;
pub mod dump;
pub mod error;
pub mod query;
pub mod schema;
pub mod write;

pub use db::{database_bytes, open, open_in_memory};
pub use derive::rebuild_assertion_state;
pub use dump::canonical_dump;
pub use error::{Result, StoreError};
pub use query::{search_entities, status, ExtractorRunSummary, SearchHit, StatusReport};
pub use schema::{migrate, schema_version, SCHEMA_VERSION};
pub use write::{
    begin_extractor_run, finish_extractor_run, persist_batch, upsert_repository,
    upsert_repository_state, RepositoryRow, RepositoryStateRow,
};

pub use rusqlite::Connection;
