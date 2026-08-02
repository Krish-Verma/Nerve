//! Nerve indexing pipeline.
//!
//! Discovery and ignore rules, path safety, tree-sitter parsing, the `fs-structural`,
//! `ts-js-structural`, `ts-js-reference` and `md-structural` extractors, lexical binding,
//! export closure, specifier
//! resolution, a hand-written Markdown block scanner, the `init` / `index` application entry
//! points, the query-time file prober that gives `nerve why` its freshness answer without
//! loosening any of those path rules, and a standalone LCOV reader for the `coverage` extractor
//! (Slice 6a: the parser only — nothing in this crate ingests a report yet).
//!
//! This crate emits **observations only**. It cannot write `assertion_state`: that table is
//! rebuilt by [`nerve_store::rebuild_assertion_state`] as a pure function of what was
//! observed.

#![forbid(unsafe_code)]
#![deny(missing_docs)]

pub mod bind;
pub mod config;
pub mod coverage;
pub mod discover;
pub mod docref;
pub mod docs;
pub mod error;
pub mod exports;
pub mod extract;
pub mod facts;
pub mod fsstruct;
pub mod gitinfo;
pub mod incremental;
pub mod init;
pub mod inspect;
pub mod lang;
pub mod markdown;
pub mod pipeline;
pub mod probe;
pub mod refs;
pub mod resolve;

pub use bind::{Binding, BindingTable, ThisResolution};
pub use config::Config;
pub use coverage::{
    parse_lcov, CoverageCounters, CoverageReport, FileCoverage, LineHit,
    EXTRACTOR_ID as COVERAGE_EXTRACTOR_ID, EXTRACTOR_VERSION as COVERAGE_EXTRACTOR_VERSION,
};
pub use discover::{discover, DiscoveredFile, DiscoveryReport};
pub use docs::{
    extract_document, AdrFacts, AdrStatus, DocumentExtraction, SectionDef,
    EXTRACTOR_ID as DOCUMENT_EXTRACTOR_ID, EXTRACTOR_VERSION as DOCUMENT_EXTRACTOR_VERSION,
};
pub use error::{IndexError, Result};
pub use exports::ExportIndex;
pub use extract::{extract_module, ModuleExtraction, EXTRACTOR_ID, EXTRACTOR_VERSION};
pub use facts::{CachedCounters, CachedReExport, CachedSymbol, DocumentCounters, ModuleFacts};
pub use fsstruct::{
    FsEntry, EXTRACTOR_ID as FILESYSTEM_EXTRACTOR_ID,
    EXTRACTOR_VERSION as FILESYSTEM_EXTRACTOR_VERSION,
};
pub use incremental::{
    classify, invalidation_set, propose_moves, ChangeKind, ChangeSet, MoveCandidate, MoveProposal,
    PreviousModule,
};
pub use init::{init, init_with_project_id, InitOutcome};
pub use inspect::{index_freshness, partial_parses, IndexFreshness, PartialParse};
pub use lang::{path_is_document, FileKind, Language, DOCUMENT_EXTENSIONS, MARKDOWN_LANGUAGE};
pub use markdown::{scan as scan_markdown, DocumentScan, Heading, HeadingStyle, ScanCounters};
pub use pipeline::{
    index_repository, index_repository_with, IncrementalReport, IndexOptions, IndexOutcome,
    RunStatus,
};
pub use probe::{RepositoryProber, SourceSnippet, MAX_SNIPPET_BYTES, MAX_SNIPPET_LINES};
pub use refs::{
    extract_references, RefTarget, ReferenceExtraction, ReferenceSite, UnresolvedReason,
    EXTRACTOR_ID as REFERENCE_EXTRACTOR_ID, EXTRACTOR_VERSION as REFERENCE_EXTRACTOR_VERSION,
};
