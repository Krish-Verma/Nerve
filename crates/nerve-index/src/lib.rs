//! Nerve indexing pipeline.
//!
//! Discovery and ignore rules, path safety, tree-sitter parsing, the `ts-js-structural` and
//! `ts-js-reference` extractors, lexical binding, export closure, specifier resolution, the
//! `init` / `index` application entry points, and the query-time file prober that gives
//! `nerve why` its freshness answer without loosening any of those path rules.
//!
//! This crate emits **observations only**. It cannot write `assertion_state`: that table is
//! rebuilt by [`nerve_store::rebuild_assertion_state`] as a pure function of what was
//! observed.

#![forbid(unsafe_code)]
#![deny(missing_docs)]

pub mod bind;
pub mod config;
pub mod discover;
pub mod error;
pub mod exports;
pub mod extract;
pub mod gitinfo;
pub mod init;
pub mod lang;
pub mod pipeline;
pub mod probe;
pub mod refs;
pub mod resolve;

pub use bind::{Binding, BindingTable, ThisResolution};
pub use config::Config;
pub use discover::{discover, DiscoveredFile, DiscoveryReport};
pub use error::{IndexError, Result};
pub use exports::ExportIndex;
pub use extract::{extract_module, ModuleExtraction, EXTRACTOR_ID, EXTRACTOR_VERSION};
pub use init::{init, init_with_project_id, InitOutcome};
pub use lang::Language;
pub use pipeline::{index_repository, IndexOutcome, RunStatus};
pub use probe::RepositoryProber;
pub use refs::{
    extract_references, RefTarget, ReferenceExtraction, ReferenceSite, UnresolvedReason,
    EXTRACTOR_ID as REFERENCE_EXTRACTOR_ID, EXTRACTOR_VERSION as REFERENCE_EXTRACTOR_VERSION,
};
