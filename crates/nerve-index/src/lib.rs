//! Nerve indexing pipeline.
//!
//! Discovery and ignore rules, path safety, tree-sitter parsing, the `fs-structural`,
//! `ts-js-structural`, `ts-js-reference`, `ts-js-framework`, `py-structural`, `py-reference`,
//! `py-framework`, `md-structural` and `coverage` extractors, lexical binding, export closure, specifier
//! resolution, a hand-written Markdown block scanner, the `init` / `index` / `coverage`
//! application entry points, and the query-time file prober that gives `nerve why` its freshness
//! answer without loosening any of those path rules.
//!
//! Each language family has its **own** extractor id, and that is not organisational tidiness:
//! an observation carries the id of whatever produced it, so a Python fact stamped
//! `ts-js-structural` would be a false statement about where the evidence came from. Slice 5d-i
//! was a corrective slice for exactly that, and [`pystruct`] exists so it cannot recur for
//! Python.
//!
//! The `coverage` extractor is split in two on purpose: [`coverage`] is a pure LCOV reader with
//! no way to reach the world, and [`coverage_ingest`] is the half that resolves paths, maps lines
//! onto symbols and writes. It is driven by its own command rather than by `index`, so that an
//! ordinary re-index cannot destroy evidence it has no way to reproduce.
//!
//! The `test-trace` extractor is split the same way and for the same reasons: [`trace`] is a pure
//! reader of the `nerve-trace/v1` artifact and [`trace_ingest`] resolves frames onto symbols and
//! writes. **Nerve does not run the test suite** — the user runs their own tests under their own
//! tracer and Nerve reads the artifact, so `crates/nerve-cli/tests/no_subprocess.rs` keeps passing
//! untouched and `nerve trace-tests` does not exist.
//!
//! [`gitobj`] is a third reader of the same shape, added by Slice 12a: it reads Git objects — loose
//! objects, `.idx` v2, packfile entries and delta chains — and **writes nothing at all**. No entity,
//! no row, no schema change, no command. Slice 12b builds the historical model on top of it. It is
//! in this crate rather than a sixth one because [`gitinfo`] already reads `.git/HEAD` and
//! `.git/packed-refs` here, so reading Git is already an indexing concern; and it reads the format
//! itself rather than depending on a Git implementation, because every such implementation ships a
//! network transport that `crates/nerve-cli/tests/no_network.rs` exists to keep out of the tree.
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
pub mod gitobj;
pub mod incremental;
pub mod init;
pub mod inspect;
pub mod lang;
pub mod markdown;
pub mod pipeline;
pub mod probe;
pub mod pybind;
pub mod pyframework;
pub mod pyrefs;
pub mod pyresolve;
pub mod pystruct;
pub mod pysurface;
pub mod refs;
pub mod resolve;
pub mod trace;
pub mod trace_ingest;
pub mod tsframework;

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
pub use facts::{
    CachedCounters, CachedPyMethod, CachedPyRebind, CachedReExport, CachedSymbol, DocumentCounters,
    ModuleFacts, PythonFacts,
};
pub use fsstruct::{
    FsEntry, EXTRACTOR_ID as FILESYSTEM_EXTRACTOR_ID,
    EXTRACTOR_VERSION as FILESYSTEM_EXTRACTOR_VERSION,
};
pub use gitobj::{
    ObjectStore as GitObjectStore, Oid as GitOid, StoreLimits as GitStoreLimits,
    MAX_DELTA_DEPTH as GIT_MAX_DELTA_DEPTH, MAX_OBJECT_BYTES as GIT_MAX_OBJECT_BYTES,
    MAX_PACK_COUNT as GIT_MAX_PACK_COUNT,
};
pub use incremental::{
    classify, invalidation_set, propose_moves, ChangeKind, ChangeSet, MoveCandidate, MoveProposal,
    PreviousModule,
};
pub use init::{init, init_with_project_id, InitOutcome};
pub use inspect::{
    index_freshness, partial_parses, untracked_files, IndexFreshness, PartialParse, UntrackedFiles,
};
pub use lang::{
    path_is_document, path_is_python, FileKind, Language, DOCUMENT_EXTENSIONS, MARKDOWN_LANGUAGE,
    PYTHON_LANGUAGE,
};
pub use markdown::{scan as scan_markdown, DocumentScan, Heading, HeadingStyle, ScanCounters};
pub use pipeline::{
    index_repository, index_repository_with, IncrementalReport, IndexOptions, IndexOutcome,
    RunStatus, INDEX_EXTRACTOR_IDS,
};
pub use probe::{RepositoryProber, SourceSnippet, MAX_SNIPPET_BYTES, MAX_SNIPPET_LINES};
pub use pybind::{PyBinding, PyBindingTable, PyScopeKind};
pub use pyframework::{
    extract_framework as extract_python_framework, Framework, PyEndpoint, PyFrameworkExtraction,
    UnsupportedForm as PyFrameworkUnsupportedForm,
    DECLARED_RELATIONS as PYTHON_FRAMEWORK_RELATIONS,
    EXTRACTOR_ID as PYTHON_FRAMEWORK_EXTRACTOR_ID,
    EXTRACTOR_VERSION as PYTHON_FRAMEWORK_EXTRACTOR_VERSION,
};
pub use pyrefs::{
    extract_references as extract_python_references, PyRefTarget, PyReferenceExtraction,
    PyReferenceSite, PyUnresolvedReason, EXTRACTOR_ID as PYTHON_REFERENCE_EXTRACTOR_ID,
    EXTRACTOR_VERSION as PYTHON_REFERENCE_EXTRACTOR_VERSION, PY_UNMODELLED_FORMS,
};
pub use pystruct::{
    extract_module as extract_python_module, AllDeclaration, PyImportForm, PyImportSite,
    PyModuleExtraction, PySymbol, EXTRACTOR_ID as PYTHON_EXTRACTOR_ID,
    EXTRACTOR_VERSION as PYTHON_EXTRACTOR_VERSION,
};
pub use pysurface::{PyModuleSurface, PySurfaceIndex};
pub use refs::{
    extract_references, RefTarget, ReferenceExtraction, ReferenceSite, UnresolvedReason,
    EXTRACTOR_ID as REFERENCE_EXTRACTOR_ID, EXTRACTOR_VERSION as REFERENCE_EXTRACTOR_VERSION,
};
pub use trace::{
    parse_trace, CompletionState, SourceMapState, TraceArtifact, TraceCounters, TraceHeader,
    TraceRecord, EXTRACTOR_ID as TRACE_EXTRACTOR_ID, EXTRACTOR_VERSION as TRACE_EXTRACTOR_VERSION,
};
pub use trace_ingest::{ingest_trace, TraceBinding, TraceOutcome};
pub use tsframework::{
    extract_framework as extract_ts_framework, TsEndpoint, TsFrameworkExtraction,
    DECLARED_RELATIONS as TS_FRAMEWORK_RELATIONS, EXTRACTOR_ID as TS_FRAMEWORK_EXTRACTOR_ID,
    EXTRACTOR_VERSION as TS_FRAMEWORK_EXTRACTOR_VERSION, FRAMEWORK as TS_FRAMEWORK_NAME,
};
