//! Nerve indexing pipeline.
//!
//! Discovery and ignore rules, path safety, tree-sitter parsing, the `fs-structural`,
//! `ts-js-structural`, `ts-js-reference`, `md-structural` and `coverage` extractors, lexical
//! binding, export closure, specifier
//! resolution, a hand-written Markdown block scanner, the `init` / `index` / `coverage`
//! application entry points, and the query-time file prober that gives `nerve why` its freshness
//! answer without loosening any of those path rules.
//!
//! The `coverage` extractor is split in two on purpose: [`coverage`] is a pure LCOV reader with
//! no way to reach the world, and [`coverage_ingest`] is the half that resolves paths, maps lines
//! onto symbols and writes. It is driven by its own command rather than by `index`, so that an
//! ordinary re-index cannot destroy evidence it has no way to reproduce.
//!
//! This crate emits **observations only**. It cannot write `assertion_state`: that table is
//! rebuilt by [`nerve_store::rebuild_assertion_state`] as a pure function of what was
//! observed.

#![forbid(unsafe_code)]
#![deny(missing_docs)]

pub mod bind;
pub mod config;
pub mod coverage;
pub mod coverage_ingest;
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
pub use coverage_ingest::{ingest_coverage, CoverageDegree, CoverageOutcome};
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
pub use inspect::{
    index_freshness, partial_parses, untracked_files, IndexFreshness, PartialParse, UntrackedFiles,
};
pub use lang::{path_is_document, FileKind, Language, DOCUMENT_EXTENSIONS, MARKDOWN_LANGUAGE};
pub use markdown::{scan as scan_markdown, DocumentScan, Heading, HeadingStyle, ScanCounters};
pub use pipeline::{
    index_repository, index_repository_with, IncrementalReport, IndexOptions, IndexOutcome,
    RunStatus, INDEX_EXTRACTOR_IDS,
};
pub use probe::{RepositoryProber, SourceSnippet, MAX_SNIPPET_BYTES, MAX_SNIPPET_LINES};
pub use refs::{
    extract_references, RefTarget, ReferenceExtraction, ReferenceSite, UnresolvedReason,
    EXTRACTOR_ID as REFERENCE_EXTRACTOR_ID, EXTRACTOR_VERSION as REFERENCE_EXTRACTOR_VERSION,
};
