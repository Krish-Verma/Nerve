//! Nerve indexing pipeline.
//!
//! Discovery and ignore rules, path safety, tree-sitter parsing, the `ts-js-structural`
//! extractor, specifier resolution, and the `init` / `index` application entry points.
//!
//! This crate emits **observations only**. It cannot write `assertion_state`: that table is
//! rebuilt by [`nerve_store::rebuild_assertion_state`] as a pure function of what was
//! observed.

#![forbid(unsafe_code)]
#![deny(missing_docs)]

pub mod config;
pub mod discover;
pub mod error;
pub mod extract;
pub mod gitinfo;
pub mod init;
pub mod lang;
pub mod pipeline;
pub mod resolve;

pub use config::Config;
pub use discover::{discover, DiscoveredFile, DiscoveryReport};
pub use error::{IndexError, Result};
pub use extract::{extract_module, ModuleExtraction, EXTRACTOR_ID, EXTRACTOR_VERSION};
pub use init::{init, init_with_project_id, InitOutcome};
pub use lang::Language;
pub use pipeline::{index_repository, IndexOutcome, RunStatus};
