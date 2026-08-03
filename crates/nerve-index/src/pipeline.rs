//! `nerve index`: discover, read, parse, extract, persist, derive.
//!
//! Parsing is serial. That is a determinism decision, not an oversight: the canonical dump must
//! be byte-identical across runs, and an ordered parallel merge is a later slice — introducing
//! it alongside deletion and invalidation would confound two new causes of divergence.
//!
//! Six extractors run per index, each with its own `extractor_run` row and its own batch
//! verified against its own declared source types. Keeping them separate is what makes a
//! `DELETE FROM observation WHERE extractor_id = 'ts-js-reference'` a complete retraction of
//! everything resolution claimed, with the structural graph untouched.
//!
//! # Python (Slices 9a and 9b)
//!
//! `py-structural` and `py-reference` are separate extractors rather than branches inside
//! `ts-js-structural` / `ts-js-reference` for the reason 5d-i established: an observation names
//! what produced it, so a Python fact must never claim a TypeScript extractor read it. The split
//! is total — [`crate::lang::Language::is_ts_js`] and [`crate::lang::Language::is_python`]
//! partition the code files, a Python file never reaches [`crate::extract`] or [`crate::refs`],
//! and a TS/JS file never reaches [`crate::pystruct`] or [`crate::pyrefs`]. Python specifier
//! resolution is [`crate::pyresolve`], which shares nothing with [`crate::resolve`] but its
//! discipline: a specifier resolves only to a file that is in the index, and everything else is
//! an `Unresolved` value with a reason.
//!
//! The two languages' cross-module views are also separate. [`crate::exports::ExportIndex`]
//! answers "what does this module export?" for TypeScript; [`crate::pysurface::PySurfaceIndex`]
//! answers "what does this module provide at top level, and which class declares which method?"
//! for Python. Both are built from the **whole** repository — freshly parsed modules for the
//! invalidation set, cached facts for everything else — because a chain in one file can reach a
//! declaration in any other and that has to keep working when the other file was not re-parsed.
//!
//! # Filesystem structure (Slice 5d-i)
//!
//! `fs-structural` runs first and owns everything a **directory walk** knows: the repository
//! entity, every directory entity, and every `CONTAINS` edge whose source is the repository or a
//! directory. It emits `FILESYSTEM_OBSERVED` and nothing else. Before 5d-i those claims were
//! `ts-js-structural` / `AST_DIRECT`, which made a documentation-only tree — no TypeScript in it
//! at all — produce observations asserting that a syntax tree contained them. See
//! [`crate::fsstruct`] for why the extractor cannot read file content, which is what the amended
//! T7 rests on.
//!
//! # Documents (Slice 5a)
//!
//! `md-structural` runs over the discovered `.md` / `.markdown` files. The split is total: a
//! document is never parsed with a grammar, never enters module resolution, and never
//! contributes an observation that is not `DOCUMENT_STATED`. The last of those is the
//! THREAT-MODEL.md **T7** control, and it is stated as an invariant over the whole database
//! rather than as a rule each emission site is trusted to follow —
//!
//! > No observation whose `file_path` is a document has any `evidence_source_type` other than
//! > `DOCUMENT_STATED`.
//!
//! Slice 5d-i amends that invariant rather than weakening it. `Directory CONTAINS <file>` is a
//! filesystem fact whatever the extension, so it moved to `fs-structural`, and the allowed set on
//! a document path became exactly `{DOCUMENT_STATED, FILESYSTEM_OBSERVED}` keyed on extractor id
//! — still total, still checked exhaustively. The reason it is not a loosening is that
//! `fs-structural` cannot carry document content anywhere, because it never reads any.
//!
//! Slice 5c adds `REFERENCES` edges for resolved links and leaves that invariant untouched: a
//! resolved link is still only a document's claim, so it stays `DOCUMENT_STATED` and moves the
//! *directness* axis alone. What it does add is a dependency documents did not previously have —
//! on the indexed path set, and on the bytes of any file a `#L<n>` anchor points into — which is
//! why [`crate::incremental::classify`] now re-resolves cached destinations and why a document
//! being re-scanned drags its anchor targets into the extraction set. Both are stated where they
//! are implemented.
//!
//! # Incremental indexing (Slice 3)
//!
//! Every run costs `discover + read + hash` over the whole tree — that is what the repository
//! state is a hash of, so it cannot be skipped — and then re-extracts only the **invalidation
//! set**: the files whose bytes changed, plus everything that imports them transitively (see
//! [`crate::incremental`]). Unchanged files keep the rows they already have.
//!
//! The load-bearing property is stronger than "the graph is stable":
//!
//! > After any run, the database is byte-identical to a from-scratch index of the same tree.
//!
//! Three things make that true, and each would break it on its own if omitted:
//!
//! 1. **Rows for vanished and re-extracted files are deleted**, and assertions and entities left
//!    without support are pruned. Insert-only indexing leaves a deleted file's graph behind
//!    forever, which is not merely stale — it is wrong.
//! 2. **No row carries a repository state** (ADR-0006). An occurrence is a location fact and an
//!    observation is evidence about a file at a content hash; neither depends on which run
//!    noticed it. A surviving row is therefore already correct and needs no rewriting. Slice 3
//!    had to restate every surviving row instead, at 1330 ms of a 2900 ms run on 520 modules.
//! 3. **Entities are upserted, not ignored.** An entity id excludes body content by design, so
//!    an edit can change a row without changing its id.
//!
//! `--full` is the same code path with every file seeded, so the full and incremental paths
//! cannot drift apart by being two implementations.
//!
//! # Work proportional to the change (Slice 3b)
//!
//! Everything the transaction writes is scoped to what moved:
//!
//! - `assertion_state` is recomputed only for the assertions whose evidence this run wrote or
//!   withdrew, and orphan pruning only considers rows this run could have orphaned. Both are
//!   lazy evaluations of the whole-table statements, which stay in the codebase as the reference
//!   implementations and are what run when a run re-extracts the entire repository.
//! - Directory containment is re-derived only when a file was **removed**, the only way a
//!   directory can stop holding indexed files. Clearing it unconditionally made an unrelated
//!   one-file edit rewrite one row per directory in the repository.
//!
//! [`IncrementalReport::rows_written`] counts the rows the transaction actually changed, and
//! `nerve-index/tests/incremental.rs` gates it: the same one-file leaf edit in a small repository
//! and in a 520-module one must write the same number of rows.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::time::Instant;

use nerve_core::ids;
use nerve_core::model::{
    AssertionRecord, EntityRecord, GraphBatch, ObservationRecord, OccurrenceRecord, Span,
};
use nerve_core::vocab::{
    Directness, EndpointKind, EntityKind, EvidenceSourceType, Relation, UnresolvedCategory,
};

use crate::config::{self, Config};
use crate::discover;
use crate::docref::{
    self, AnchorOutcome, DocumentLinks, DocumentSupersessions, LinkOutcome, LinkSite,
    SupersessionOutcome, SupersessionSite,
};
use crate::docs::{self, DocumentExtraction, SupersessionDirection};
use crate::error::{IndexError, Result};
use crate::exports::ExportIndex;
use crate::extract::{
    self, ExportTarget, ModuleExtraction, DECLARED_SOURCE_TYPES, EXTRACTOR_ID, EXTRACTOR_VERSION,
};
use crate::facts::{self, ModuleFacts};
use crate::fsstruct::{self, FsEntry};
use crate::gitinfo;
use crate::incremental::{self, MoveCandidate, PreviousModule};
use crate::lang::{path_is_document, path_is_python, FileKind, Language, MARKDOWN_LANGUAGE};
use crate::pyframework::{self, PyFrameworkExtraction};
use crate::pyrefs::{self, PyRefTarget, PyReferenceExtraction};
use crate::pyresolve;
use crate::pystruct::{self, PyImportForm, PyImportSite, PyModuleExtraction};
use crate::pysurface::{PyModuleSurface, PySurfaceIndex};
use crate::refs::{self, RefTarget, ReferenceExtraction};
use crate::resolve;

/// Every extractor `nerve index` runs, and therefore every extractor whose evidence it may
/// withdraw when it re-extracts a file.
///
/// `coverage` is deliberately absent: it is ingested by a separate, explicitly invoked command
/// (`docs/plans/slice-06-test-evidence.md` §A.4), and an index run neither produces nor replaces
/// it. The list is used exactly once, at the withdrawal in [`index_repository_with`].
pub const INDEX_EXTRACTOR_IDS: [&str; 7] = [
    fsstruct::EXTRACTOR_ID,
    EXTRACTOR_ID,
    refs::EXTRACTOR_ID,
    pystruct::EXTRACTOR_ID,
    pyrefs::EXTRACTOR_ID,
    // Added in Slice 10a. Present here is what lets a re-extraction *withdraw* an endpoint whose
    // decorator was deleted; a framework extractor missing from this list would leave stale routes
    // in the graph forever.
    pyframework::EXTRACTOR_ID,
    docs::EXTRACTOR_ID,
];

/// Terminal status of an index run.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RunStatus {
    /// Every discovered file was read and parsed.
    Complete,
    /// At least one file was skipped as too large, unreadable, or not UTF-8.
    Partial,
}

impl RunStatus {
    /// Value stored in `extractor_run.status`.
    pub fn as_str(self) -> &'static str {
        match self {
            RunStatus::Complete => "complete",
            RunStatus::Partial => "partial",
        }
    }
}

/// How much of the repository an index run rebuilds.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct IndexOptions {
    /// Re-extract every file, ignoring the change-detection cache.
    ///
    /// Not a different algorithm — the same run with every present file seeded. That is
    /// deliberate: a `--full` that took its own path could not be used to check the incremental
    /// one, because a shared bug would cancel out.
    pub full: bool,
}

/// What incremental indexing decided, re-extracted and removed.
///
/// Deletion is the first destructive operation in the product, so every count here is reported
/// by `nerve index`. Silent deletion is not acceptable output.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct IncrementalReport {
    /// Whether the run was forced full.
    pub full: bool,
    /// Files whose content and extractor versions were unchanged.
    pub files_unchanged: usize,
    /// Files whose content or extractor version changed.
    pub files_modified: usize,
    /// Files not previously indexed.
    pub files_added: usize,
    /// Files no longer present, whose rows were removed.
    pub files_removed: usize,
    /// Unchanged files whose specifier resolution moved because the file set changed.
    pub files_resolution_changed: usize,
    /// Seed of the invalidation walk: changed files plus resolution-affected files.
    pub files_seeded: usize,
    /// Files actually re-extracted — the seed plus its reverse `IMPORTS` closure.
    pub files_re_extracted: usize,
    /// Files present but not re-extracted.
    pub files_skipped_unchanged: usize,
    /// Paths whose rows were removed, sorted.
    pub removed_paths: Vec<String>,
    /// Observations deleted.
    pub observations_removed: usize,
    /// Occurrences deleted.
    pub occurrences_removed: usize,
    /// Assertions deleted for want of any supporting observation.
    pub assertions_removed: usize,
    /// Entities deleted for want of any occurrence or incident assertion.
    pub entities_removed: usize,
    /// Assertions whose derived state this run recomputed.
    ///
    /// The scope of the lazy `assertion_state` evaluation. Equal to the whole table when the run
    /// re-extracted the entire repository.
    pub assertions_derived: usize,
    /// Rows of Nerve's own model this run inserted, updated or deleted.
    ///
    /// The structural gate on incremental indexing, and the one that cannot be gamed by a fast
    /// machine: a one-file leaf edit must write a number of rows proportional to the change, not
    /// to the size of the repository.
    ///
    /// Counts `entity`, `occurrence`, `assertion`, `observation`, `assertion_state`,
    /// `module_facts` and `identity_link`. Two things are deliberately outside it:
    ///
    /// - `repository`, `repository_state` and `extractor_run`, which are six statements per run
    ///   whatever the repository and whatever the change;
    /// - SQLite's own index maintenance, above all the FTS5 shadow tables. Updating one entity
    ///   row provokes a segment flush whose size depends on how much is already indexed, so
    ///   counting it would make the metric report a repository-proportional cost for a
    ///   single-row write. That work is real, but it is SQLite's bookkeeping over its own index,
    ///   not a row of Nerve's model, and it is amortized rather than paid per run.
    pub rows_written: usize,
    /// Identity links proposed by this run.
    pub identity_links_proposed: usize,
    /// Identity links this run wrote; a link already proposed is not written twice.
    pub identity_links_recorded: usize,
}

impl IncrementalReport {
    /// Files changed on disk: modified, added and removed.
    pub fn files_changed(&self) -> usize {
        self.files_modified + self.files_added + self.files_removed
    }

    /// Files re-extracted per file that changed on disk.
    ///
    /// The number the invalidation rule is judged on. `None` when nothing changed, because
    /// "infinity" and "zero" are both the wrong answer to a division by nothing.
    pub fn amplification(&self) -> Option<f64> {
        let changed = self.files_changed();
        if changed == 0 {
            return None;
        }
        Some(self.files_re_extracted as f64 / changed as f64)
    }
}

/// What `nerve index` did and what the graph now contains.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IndexOutcome {
    /// Canonical repository root.
    pub root: PathBuf,
    /// Repository state observed by this run.
    pub state_id: String,
    /// Git HEAD, when `.git` is readable.
    pub git_commit: Option<String>,
    /// Files parsed.
    pub files_processed: usize,
    /// Files skipped as too large, unreadable, or not UTF-8.
    pub files_failed: usize,
    /// Files that parsed with at least one ERROR node. Not a failure; the graph is partial.
    pub files_with_syntax_errors: usize,
    /// Paths refused by the secret deny-list.
    pub denied_secrets: Vec<String>,
    /// Files with no grammar for their extension.
    pub skipped_unsupported: usize,
    /// Symlinks that were not followed.
    pub skipped_symlinks: usize,
    /// `import(expr)` calls that named no specifier and so produced no edge.
    pub dynamic_imports_without_specifier: usize,
    /// Call sites and heritage clauses whose form Nerve does not model. Counted, never guessed.
    pub unmodelled_call_sites: usize,
    /// Breakdown of the above by form tag.
    pub unmodelled_by_form: BTreeMap<String, usize>,
    /// Endpoints declared by a framework rule.
    pub endpoints: usize,
    /// Framework-rule constructs read and declined, by form tag. Counted, never guessed.
    ///
    /// Cache-backed, so this describes the repository rather than the fraction this run touched.
    pub framework_unsupported_by_form: BTreeMap<String, usize>,
    /// Documents read, over the whole repository.
    pub documents_processed: usize,
    /// Documents recognised as ADRs.
    pub adr_documents: usize,
    /// Sections the documents contributed.
    pub document_sections: usize,
    /// Markdown constructs refused by the scanner, over the whole repository.
    ///
    /// Every resource bound and every construct outside the supported subset lands here. Nothing
    /// is truncated silently: a document that hit a bound says so through this map.
    pub unsupported_markdown: usize,
    /// Breakdown of the above by form tag.
    pub unsupported_markdown_by_form: BTreeMap<String, usize>,
    /// `Document SUPERSEDES Document` edges over the whole repository.
    pub supersession_edges: usize,
    /// Groups of mutually superseding documents. **Not suppressed** — see
    /// [`supersession_cycles`]: each edge is individually evidenced, so a cycle is reported
    /// rather than broken.
    pub supersession_cycles: usize,
    /// Documents lying in one of those groups.
    pub supersession_cycle_documents: usize,
    /// Documents another document claims to have superseded, whose own status still reads
    /// `Accepted`. A string comparison; no semantics required.
    pub supersession_contradictions: usize,
    /// Entity counts by kind, over the whole database.
    pub entities_by_kind: BTreeMap<String, i64>,
    /// Assertion counts by relation, over the whole database.
    pub assertions_by_relation: BTreeMap<String, i64>,
    /// Total entities, of every kind.
    pub entities_total: i64,
    /// Entities that are symbols — functions, methods, classes, interfaces. Always at most
    /// [`entities_total`](Self::entities_total); see [`nerve_store::StatusReport::symbols_total`].
    pub symbols_total: i64,
    /// Total assertions.
    pub assertions_total: i64,
    /// Total observations.
    pub observations_total: i64,
    /// Entities of kind `unresolved`.
    pub unresolved_entities: i64,
    /// Assertion states flagged `is_unresolved`.
    pub unresolved_assertions: i64,
    /// Wall-clock duration.
    pub duration_ms: u128,
    /// Terminal status.
    pub status: RunStatus,
    /// What was re-extracted, skipped and removed.
    pub incremental: IncrementalReport,
}

/// One document the graph builder is about to emit, with everything resolution concluded.
///
/// A struct rather than a tuple because it now carries four things and a positional read of
/// `(file, extraction, links, supersessions)` at the call site says nothing about which is which.
struct DocumentTarget<'a> {
    file: &'a LoadedFile,
    extraction: &'a DocumentExtraction,
    links: &'a DocumentLinks,
    supersessions: &'a DocumentSupersessions,
}

struct LoadedFile {
    rel_path: String,
    kind: FileKind,
    source: String,
    content_hash: String,
    size_bytes: u64,
}

impl LoadedFile {
    /// The grammar this file is parsed with. `None` for a document, which is never parsed.
    fn language(&self) -> Option<Language> {
        self.kind.language()
    }
}

/// Accumulates the graph for one run, deduplicating by content-derived key as it goes.
struct GraphBuilder {
    extractor_id: &'static str,
    extractor_version: &'static str,
    batch: GraphBatch,
    seen_entities: BTreeSet<String>,
    seen_assertions: BTreeSet<String>,
}

impl GraphBuilder {
    fn new(extractor_id: &'static str, extractor_version: &'static str) -> Self {
        GraphBuilder {
            extractor_id,
            extractor_version,
            batch: GraphBatch::default(),
            seen_entities: BTreeSet::new(),
            seen_assertions: BTreeSet::new(),
        }
    }

    fn add_entity(&mut self, entity: EntityRecord) {
        if self.seen_entities.insert(entity.entity_id.clone()) {
            self.batch.entities.push(entity);
        }
    }

    fn add_occurrence(&mut self, entity_id: &str, file_path: &str, span: Span, content_hash: &str) {
        self.batch.occurrences.push(OccurrenceRecord {
            occurrence_id: ids::occurrence_id(entity_id, file_path, span.start_byte, span.end_byte),
            entity_id: entity_id.to_string(),
            file_path: file_path.to_string(),
            span,
            content_hash: content_hash.to_string(),
        });
    }

    fn add_assertion(&mut self, source: &str, relation: Relation, target: &str) -> String {
        let assertion_id = ids::assertion_id(source, relation, target);
        if self.seen_assertions.insert(assertion_id.clone()) {
            self.batch.assertions.push(AssertionRecord {
                assertion_id: assertion_id.clone(),
                source_entity_id: source.to_string(),
                relation,
                target_entity_id: target.to_string(),
            });
        }
        assertion_id
    }

    #[allow(clippy::too_many_arguments)]
    fn observe(
        &mut self,
        assertion_id: &str,
        evidence: Evidence,
        file_path: &str,
        start_line: usize,
        end_line: usize,
        content_hash: &str,
        details: serde_json::Value,
    ) {
        self.batch.observations.push(ObservationRecord {
            assertion_id: assertion_id.to_string(),
            evidence_source_type: evidence.source_type,
            directness: evidence.directness,
            extractor_id: self.extractor_id.to_string(),
            extractor_version: self.extractor_version.to_string(),
            // Neither extractor performs matching, so match_quality is meaningless for both.
            match_quality: None,
            file_path: file_path.to_string(),
            start_line,
            end_line,
            content_hash: content_hash.to_string(),
            environment: None,
            details: Some(details.to_string()),
        });
    }

    #[allow(clippy::too_many_arguments)]
    fn claim(
        &mut self,
        source: &str,
        relation: Relation,
        target: &str,
        evidence: Evidence,
        file_path: &str,
        span: Span,
        content_hash: &str,
        details: serde_json::Value,
    ) {
        let assertion_id = self.add_assertion(source, relation, target);
        self.observe(
            &assertion_id,
            evidence,
            file_path,
            span.start_line,
            span.end_line,
            content_hash,
            details,
        );
    }
}

/// The evidence profile attached to one observation.
///
/// The pairing is not decoration. ADR-0003 defines `AST_RESOLVED` as "resolved through
/// import/module resolution", so an edge that survived a resolution step must say so; an edge
/// whose target is an `Unresolved` entity states only what the tree literally wrote and stays
/// `AST_DIRECT`.
#[derive(Debug, Clone, Copy)]
struct Evidence {
    source_type: EvidenceSourceType,
    directness: Directness,
}

impl Evidence {
    /// The syntax tree literally states this.
    const DIRECT: Evidence = Evidence {
        source_type: EvidenceSourceType::AstDirect,
        directness: Directness::Direct,
    };

    /// The filesystem contains this. The only profile `fs-structural` may emit.
    ///
    /// `DIRECT` because the directory walk literally found the entry: no resolution step, no
    /// rule, and — the point of the whole extractor — no file was opened to learn it.
    const FILESYSTEM: Evidence = Evidence {
        source_type: EvidenceSourceType::FilesystemObserved,
        directness: fsstruct::DIRECTNESS,
    };

    /// Module, export or binding resolution produced this.
    const RESOLVED: Evidence = Evidence {
        source_type: EvidenceSourceType::AstResolved,
        directness: Directness::Resolved,
    };

    /// A framework rule read a declaration. The only profile `py-framework` may emit.
    ///
    /// `Direct` rather than `Inferred` because the decorator and the function it decorates are
    /// adjacent in one syntax tree: the rule reads a declaration, it does not conclude one. What
    /// the declaration proves is bounded by [`Relation::ServedBy`]'s documentation — a table entry,
    /// never an execution.
    const FRAMEWORK: Evidence = Evidence {
        source_type: EvidenceSourceType::FrameworkRule,
        directness: Directness::Direct,
    };

    /// A document states this. The only profile `md-structural` may emit (THREAT-MODEL.md T7).
    const DOCUMENT: Evidence = Evidence {
        source_type: EvidenceSourceType::DocumentStated,
        directness: docs::DIRECTNESS,
    };

    /// A document states this, and **link resolution** turned it into an entity.
    ///
    /// The source type is deliberately the same: a resolved link is still only a document's
    /// claim, so promoting it to `AST_RESOLVED` would make a README's assertion indistinguishable
    /// from one the compiler could check — exactly the collapse THREAT-MODEL.md T7 forbids. Only
    /// directness moves, on the same reading of ADR-0003 that separates `AST_DIRECT` from
    /// `AST_RESOLVED`: a step happened, and the row says so.
    const DOCUMENT_RESOLVED: Evidence = Evidence {
        source_type: EvidenceSourceType::DocumentStated,
        directness: docs::RESOLVED_DIRECTNESS,
    };
}

fn parent_directory(rel_path: &str) -> Option<String> {
    rel_path
        .rfind('/')
        .map(|index| rel_path[..index].to_string())
}

fn last_segment(rel_path: &str) -> &str {
    match rel_path.rfind('/') {
        Some(index) => &rel_path[index + 1..],
        None => rel_path,
    }
}

fn file_stem(name: &str) -> &str {
    match name.rfind('.') {
        Some(index) if index > 0 => &name[..index],
        _ => name,
    }
}

/// Index a repository incrementally. One transaction; `assertion_state` rebuilt inside it.
pub fn index_repository(root: &Path) -> Result<IndexOutcome> {
    index_repository_with(root, IndexOptions::default())
}

/// Index a repository. See [`IndexOptions`].
pub fn index_repository_with(root: &Path, options: IndexOptions) -> Result<IndexOutcome> {
    let started = Instant::now();
    let root = discover::canonical_root(root)?;
    let db_path = config::db_path(&root);
    if !db_path.exists() {
        return Err(IndexError::NotInitialized(root));
    }
    let config = Config::load(&root)?;
    let discovery = discover::discover(&root, &config)?;

    // ---- read and hash -------------------------------------------------------------------
    //
    // Every file is read every run. That is not waste that incremental indexing failed to
    // remove: the repository state *is* a Merkle over the file contents, so change detection
    // cannot be cheaper than reading and hashing the tree. What incremental indexing removes is
    // parsing and extraction, which dominate.
    let mut loaded: Vec<LoadedFile> = Vec::new();
    let mut files_failed = 0usize;
    let mut documents_failed = 0usize;
    let mut python_failed = 0usize;
    for file in &discovery.files {
        // A refusal is attributed to the extractor that would have read the file, so that a
        // document too large to scan is reported against `md-structural` and not against a
        // TypeScript run that never saw it. The `index.max_file_bytes` ceiling is the document
        // byte bound: reusing it is deliberate (plan §3.6) rather than inventing a second limit.
        let readable = std::fs::metadata(&file.abs_path).ok().and_then(|metadata| {
            if metadata.len() > config.index.max_file_bytes {
                return None;
            }
            let bytes = std::fs::read(&file.abs_path).ok()?;
            let content_hash = ids::content_hash(&bytes);
            let source = String::from_utf8(bytes).ok()?;
            Some((metadata.len(), content_hash, source))
        });
        let Some((size_bytes, content_hash, source)) = readable else {
            files_failed += 1;
            if file.kind.is_doc() {
                documents_failed += 1;
            }
            if file.kind.language().is_some_and(Language::is_python) {
                python_failed += 1;
            }
            continue;
        };
        loaded.push(LoadedFile {
            rel_path: file.rel_path.clone(),
            kind: file.kind,
            source,
            content_hash,
            size_bytes,
        });
    }
    loaded.sort_by(|a, b| a.rel_path.cmp(&b.rel_path));
    let documents_loaded = loaded.iter().filter(|file| file.kind.is_doc()).count();
    // Per-extractor file tallies. An `extractor_run` row states how many files *that* extractor
    // saw, so a repository holding both languages must not report every Python file against the
    // TypeScript run — the same category error, one level up, that Slice 5d-i removed from the
    // observation rows.
    let python_loaded = loaded
        .iter()
        .filter(|file| file.language().is_some_and(Language::is_python))
        .count();
    let ts_js_loaded = loaded.len() - documents_loaded - python_loaded;
    let ts_js_failed = files_failed - documents_failed - python_failed;

    // ---- repository state ----------------------------------------------------------------
    let mut pairs: Vec<(String, String)> = loaded
        .iter()
        .map(|file| (file.rel_path.clone(), file.content_hash.clone()))
        .collect();
    let content_merkle = ids::content_merkle(&mut pairs);
    let state_id = content_merkle.clone();
    let git_commit = gitinfo::head_commit(&root);

    let repo_id = ids::repository_id(&config.project_id);
    let mut conn = nerve_store::open(&db_path)?;

    // Bring the schema up to date before writing a single row.
    //
    // `nerve init` migrates, but a database written by an older build and then indexed by a
    // newer one never passed through `init` again. Until Slice 3b that was merely a loud failure
    // (a missing table); with schema v3 it became a silent one. `persist_batch` inserts
    // `OR IGNORE` — which is what makes re-indexing an unchanged tree free — and `OR IGNORE`
    // swallows `NOT NULL` violations as readily as duplicate keys. Writing v3's column list into
    // a v2 `occurrence` therefore dropped every insert on the floor **after** the re-extracted
    // files' rows had already been deleted, leaving a smaller graph and a zero exit code.
    //
    // Migration is idempotent and costs one `SELECT MAX(version)` on an up-to-date database, so
    // it runs unconditionally rather than behind a version check that could itself drift. A
    // database written by a *newer* build is refused here rather than guessed at.
    nerve_store::migrate(&conn)?;

    // ---- what changed --------------------------------------------------------------------
    let previous = load_previous_modules(&conn, &repo_id)?;
    let current_hashes: BTreeMap<String, String> = loaded
        .iter()
        .map(|file| (file.rel_path.clone(), file.content_hash.clone()))
        .collect();
    let changes = incremental::classify(&previous, &current_hashes, options.full);

    // `indexed` is what module resolution consults, and it holds **code paths only**. A
    // specifier that could resolve to a document would produce an `IMPORTS` edge whose target is
    // a `Module` entity no extractor ever creates — a dangling foreign key, and a claim that a
    // README is a module.
    let indexed: BTreeSet<String> = loaded
        .iter()
        .filter(|file| !file.kind.is_doc())
        .map(|file| file.rel_path.clone())
        .collect();

    // What a **document link** resolves against: every indexed path, documents included. Wider
    // than `indexed` on purpose — a README linking to another README is an ordinary and useful
    // thing to record, whereas a module *specifier* resolving to a document would invent a
    // `Module` entity no extractor creates.
    let all_indexed: BTreeSet<String> = current_hashes.keys().cloned().collect();

    let mut known_paths: BTreeSet<String> = current_hashes.keys().cloned().collect();
    known_paths.extend(previous.keys().cloned());
    let invalidated =
        incremental::invalidation_set(&conn, &config.project_id, &changes.seed(), &known_paths)?;

    // Present files to re-extract. A file whose cached facts this build cannot reuse is added
    // even if the walk did not reach it: extraction is the only way to obtain them.
    let mut target_paths: BTreeSet<String> = BTreeSet::new();
    for file in &loaded {
        let reusable = previous
            .get(&file.rel_path)
            .is_some_and(|module| module.reusable && module.facts.is_some());
        if invalidated.contains(&file.rel_path) || !reusable {
            target_paths.insert(file.rel_path.clone());
        }
    }

    // ---- extract the invalidation set ----------------------------------------------------
    //
    // Documents are scanned first, and not for tidiness. A `#L<n>` anchor is resolved against the
    // **symbol extents** of the file it points into, and extents exist only for a file this run
    // parsed. A document being re-extracted therefore drags its anchor targets into the
    // extraction set: without that, an incremental run would conclude "no symbol covers that
    // line" exactly where a from-scratch run resolves the symbol, and the two would disagree.
    //
    // Only **code** targets are forced. An anchor into a document resolves to no symbol in
    // either kind of run — no extractor gives a document symbol extents — so forcing one would
    // buy nothing, and the expansion stays a single round with no cycle to bound.
    let mut document_extractions: BTreeMap<String, DocumentExtraction> = BTreeMap::new();
    for file in &loaded {
        // A document has no grammar and no imports, so nothing resolves *through* it: it is
        // never in another file's invalidation closure, only in its own.
        if target_paths.contains(&file.rel_path) && file.language().is_none() {
            document_extractions.insert(
                file.rel_path.clone(),
                docs::extract_document(&config.project_id, &file.rel_path, &file.source),
            );
        }
    }
    for (rel_path, extraction) in &document_extractions {
        for link in &extraction.links {
            if docref::anchor_of(&link.destination).is_none() {
                continue;
            }
            let Some(target) = docref::resolve_path(rel_path, &link.destination, &all_indexed)
            else {
                continue;
            };
            if indexed.contains(&target) {
                target_paths.insert(target);
            }
        }
    }

    // Dispatch by language family, never by "is it code?". A file reaches exactly one structural
    // extractor, and the two maps below are disjoint by construction.
    let mut extractions: BTreeMap<String, ModuleExtraction> = BTreeMap::new();
    let mut python_extractions: BTreeMap<String, PyModuleExtraction> = BTreeMap::new();
    for file in &loaded {
        if !target_paths.contains(&file.rel_path) {
            continue;
        }
        let Some(language) = file.language() else {
            continue;
        };
        if language.is_python() {
            python_extractions.insert(
                file.rel_path.clone(),
                pystruct::extract_module(&config.project_id, &file.rel_path, &file.source)?,
            );
        } else {
            extractions.insert(
                file.rel_path.clone(),
                extract::extract_module(
                    &config.project_id,
                    &file.rel_path,
                    language,
                    &file.source,
                )?,
            );
        }
    }

    // ---- resolve document links ----------------------------------------------------------
    //
    // Resolution reads a whole-repository view and produces no filesystem access beyond the path
    // guard: membership of the indexed path set decides the answer, and nothing here opens,
    // fetches or follows a destination. See `crate::docref` for why the guard is in the loop
    // even though its success is ignored.
    // Both structural extractors contribute extents, so a `#L<n>` anchor into a `.py` file
    // resolves to the symbol covering that line exactly as one into a `.ts` file does. The two
    // maps are keyed by path and are disjoint, so the merge cannot lose or overwrite an entry.
    let mut symbol_extents: BTreeMap<String, Vec<docref::SymbolExtent>> = extractions
        .iter()
        .map(|(rel_path, extraction)| {
            let extents = extraction
                .symbols
                .iter()
                .map(|symbol| docref::SymbolExtent {
                    entity_id: symbol.entity_id.clone(),
                    start_byte: symbol.span.start_byte,
                    end_byte: symbol.span.end_byte,
                    start_line: symbol.span.start_line,
                    end_line: symbol.span.end_line,
                })
                .collect();
            (rel_path.clone(), extents)
        })
        .collect();
    for (rel_path, extraction) in &python_extractions {
        let extents = extraction
            .symbols
            .iter()
            .map(|symbol| docref::SymbolExtent {
                entity_id: symbol.entity_id.clone(),
                start_byte: symbol.span.start_byte,
                end_byte: symbol.span.end_byte,
                start_line: symbol.span.start_line,
                end_line: symbol.span.end_line,
            })
            .collect();
        symbol_extents.insert(rel_path.clone(), extents);
    }
    let corpus = docref::Corpus {
        indexed: &all_indexed,
        symbols: &symbol_extents,
        content_hashes: &current_hashes,
    };
    let document_links: BTreeMap<String, DocumentLinks> = document_extractions
        .iter()
        .map(|(rel_path, extraction)| {
            (
                rel_path.clone(),
                docref::resolve_document_links(&root, rel_path, &extraction.links, &corpus),
            )
        })
        .collect();
    let document_supersessions: BTreeMap<String, DocumentSupersessions> = document_extractions
        .iter()
        .map(|(rel_path, extraction)| {
            (
                rel_path.clone(),
                docref::resolve_document_supersessions(
                    &root,
                    rel_path,
                    &extraction.supersession,
                    &corpus,
                ),
            )
        })
        .collect();

    // The export closure spans every module, so it is built from the whole corpus: freshly
    // parsed modules for the invalidation set, and modules reconstructed from the cache for
    // everything else. A barrel chain in one file can reach a declaration in any other, and
    // that has to keep working when the other file was not re-parsed.
    //
    // Python is excluded, and not by oversight: the closure exists to answer "which module
    // declares the symbol this specifier's name refers to?" for `ts-js-reference`, which never
    // sees a `.py` file. Feeding Python modules in would put entries in the index that nothing
    // reads and that a `ts-js` specifier could otherwise be resolved against.
    let export_sources: Vec<ModuleExtraction> = loaded
        .iter()
        .filter(|file| file.language().is_some_and(Language::is_ts_js))
        .map(|file| match extractions.get(&file.rel_path) {
            Some(extraction) => facts::export_source_of(extraction),
            None => cached_facts(&previous, &file.rel_path).as_export_source(&file.rel_path),
        })
        .collect();
    let export_index = ExportIndex::build(&export_sources, &indexed);

    let mut reference_extractions: BTreeMap<String, ReferenceExtraction> = BTreeMap::new();
    for file in &loaded {
        let Some(extraction) = extractions.get(&file.rel_path) else {
            continue;
        };
        let Some(language) = file.language() else {
            continue;
        };
        reference_extractions.insert(
            file.rel_path.clone(),
            refs::extract_references(
                &config.project_id,
                &file.rel_path,
                language,
                &file.source,
                extraction,
                &export_index,
                &indexed,
            )?,
        );
    }

    // Python's cross-module view, built over the whole corpus for the same reason the export
    // closure is: `from pkg import Engine` reaches `pkg/core.py` through `pkg/__init__.py`, and
    // that chain must resolve identically whether or not this run re-parsed the package init.
    // A TypeScript module contributes nothing here, and a Python module contributes nothing to
    // `export_index` — the two indexes are disjoint by construction, not by filtering.
    let python_surfaces: Vec<PyModuleSurface> = loaded
        .iter()
        .filter(|file| file.language().is_some_and(Language::is_python))
        .map(|file| match python_extractions.get(&file.rel_path) {
            Some(extraction) => PyModuleSurface::from_extraction(extraction),
            None => cached_facts(&previous, &file.rel_path).as_python_surface(&file.rel_path),
        })
        .collect();
    let python_surface_index = PySurfaceIndex::build(python_surfaces, &indexed);

    let mut python_reference_extractions: BTreeMap<String, PyReferenceExtraction> = BTreeMap::new();
    let mut python_framework_extractions: BTreeMap<String, PyFrameworkExtraction> = BTreeMap::new();
    for file in &loaded {
        let Some(extraction) = python_extractions.get(&file.rel_path) else {
            continue;
        };
        python_reference_extractions.insert(
            file.rel_path.clone(),
            pyrefs::extract_references(
                &config.project_id,
                &file.rel_path,
                &file.source,
                extraction,
                &python_surface_index,
            )?,
        );
        // The framework rules read the same tree again rather than sharing the reference walk: they
        // answer a different question, they emit a different source type, and keeping them separate
        // is what lets `py-framework` be versioned and invalidated on its own.
        python_framework_extractions.insert(
            file.rel_path.clone(),
            pyframework::extract_framework(
                &config.project_id,
                &file.rel_path,
                &file.source,
                extraction,
            )?,
        );
    }

    // ---- whole-repository view, from fresh work plus cache -------------------------------
    //
    // Reported totals describe the repository, not the fraction of it this run touched, so the
    // per-file tallies of skipped files come from the cache.
    let mut facts_by_path: BTreeMap<String, ModuleFacts> = BTreeMap::new();
    for file in &loaded {
        let facts = match document_extractions.get(&file.rel_path) {
            Some(extraction) => ModuleFacts::from_document(extraction),
            None => match (
                python_extractions.get(&file.rel_path),
                python_reference_extractions.get(&file.rel_path),
                python_framework_extractions.get(&file.rel_path),
            ) {
                (Some(extraction), Some(references), Some(framework)) => {
                    ModuleFacts::from_python(extraction, references, framework, &file.source)
                }
                _ => match (
                    extractions.get(&file.rel_path),
                    reference_extractions.get(&file.rel_path),
                ) {
                    (Some(extraction), Some(references)) => {
                        ModuleFacts::from_extraction(extraction, references, &file.source)
                    }
                    _ => cached_facts(&previous, &file.rel_path),
                },
            },
        };
        facts_by_path.insert(file.rel_path.clone(), facts);
    }

    // Document tallies over the whole repository: freshly scanned documents plus the cached
    // counts of the ones this run did not re-read.
    let mut unsupported_markdown_by_form: BTreeMap<String, usize> = BTreeMap::new();
    let mut adr_documents = 0usize;
    let mut document_sections = 0usize;
    for file in &loaded {
        if !file.kind.is_doc() {
            continue;
        }
        let Some(facts) = facts_by_path.get(&file.rel_path) else {
            continue;
        };
        document_sections += facts.document.sections;
        if facts.document.is_adr {
            adr_documents += 1;
        }
        for (form, count) in &facts.document.unsupported {
            *unsupported_markdown_by_form
                .entry(form.clone())
                .or_insert(0) += count;
        }
    }
    let unsupported_markdown: usize = unsupported_markdown_by_form.values().sum();

    // Supersession over the **whole** repository, from cached and fresh facts alike. A cycle and
    // a status contradiction are properties of the graph, not of one file, so a run that
    // re-extracted one document must still report them; computing them from this run's
    // extractions alone would report "no cycles" for a repository that has one.
    let supersedes = supersession_edges(&root, &facts_by_path, &corpus);
    let (supersession_cycle_count, supersession_cycle_documents) = supersession_cycles(&supersedes);
    let supersession_contradiction_count = supersession_contradictions(&supersedes, &facts_by_path);

    let files_with_syntax_errors = facts_by_path
        .values()
        .filter(|facts| facts.counters.has_syntax_error)
        .count();
    let dynamic_imports_without_specifier: usize = facts_by_path
        .values()
        .map(|facts| facts.counters.dynamic_imports_without_specifier)
        .sum();
    let unmodelled_call_sites: usize = facts_by_path
        .values()
        .map(|facts| facts.counters.unmodelled_call_sites)
        .sum();
    let mut unmodelled_by_form: BTreeMap<String, usize> = BTreeMap::new();
    for facts in facts_by_path.values() {
        for (form, count) in &facts.counters.unmodelled_by_form {
            *unmodelled_by_form.entry(form.clone()).or_insert(0) += count;
        }
    }
    // Read from the cache-backed facts rather than from this run's fresh extractions, so a run that
    // re-extracted one file still reports the whole repository's tally. The precision gate audits
    // its denominator against this number.
    let mut framework_unsupported_by_form: BTreeMap<String, usize> = BTreeMap::new();
    for facts in facts_by_path.values() {
        for (form, count) in &facts.counters.framework_unsupported_by_form {
            *framework_unsupported_by_form
                .entry(form.clone())
                .or_insert(0) += count;
        }
    }
    let endpoints: usize = python_framework_extractions
        .values()
        .map(|extraction| extraction.endpoints.len())
        .sum();

    let module_exports: BTreeMap<String, BTreeMap<String, String>> = facts_by_path
        .iter()
        .map(|(path, facts)| (path.clone(), facts.exports.clone()))
        .collect();

    // ---- build ---------------------------------------------------------------------------
    let targets: Vec<(&LoadedFile, &ModuleExtraction)> = loaded
        .iter()
        .filter_map(|file| {
            extractions
                .get(&file.rel_path)
                .map(|extraction| (file, extraction))
        })
        .collect();

    // `fs-structural` sees a projection of the load, never the load itself: `FsEntry` has no
    // field that can hold file text, so the filesystem extractor cannot reach content even by
    // accident. This is the one place the projection happens, and dropping `source` here is what
    // makes the guarantee structural rather than a convention.
    let fs_entries: Vec<FsEntry> = loaded
        .iter()
        .map(|file| FsEntry {
            rel_path: file.rel_path.clone(),
            kind: file.kind,
            size_bytes: file.size_bytes,
            content_hash: file.content_hash.clone(),
        })
        .collect();
    let fs_batch = build_fs_graph(&config.project_id, &fs_entries, &target_paths);
    fs_batch
        .verify_declared_source_types(fsstruct::EXTRACTOR_ID, &fsstruct::DECLARED_SOURCE_TYPES)?;

    let batch = build_graph(&config.project_id, &targets, &module_exports, &indexed)?;
    batch.verify_declared_source_types(EXTRACTOR_ID, &DECLARED_SOURCE_TYPES)?;

    let python_targets: Vec<(&LoadedFile, &PyModuleExtraction)> = loaded
        .iter()
        .filter_map(|file| {
            python_extractions
                .get(&file.rel_path)
                .map(|extraction| (file, extraction))
        })
        .collect();
    let python_batch = build_python_graph(&config.project_id, &python_targets, &indexed)?;
    python_batch
        .verify_declared_source_types(pystruct::EXTRACTOR_ID, &pystruct::DECLARED_SOURCE_TYPES)?;

    let python_reference_targets: Vec<(&LoadedFile, &PyReferenceExtraction)> = loaded
        .iter()
        .filter_map(|file| {
            python_reference_extractions
                .get(&file.rel_path)
                .map(|extraction| (file, extraction))
        })
        .collect();
    let python_reference_batch =
        build_python_reference_graph(&config.project_id, &python_reference_targets);
    python_reference_batch
        .verify_declared_source_types(pyrefs::EXTRACTOR_ID, &pyrefs::DECLARED_SOURCE_TYPES)?;

    let python_framework_targets: Vec<(&LoadedFile, &PyFrameworkExtraction)> = loaded
        .iter()
        .filter_map(|file| {
            python_framework_extractions
                .get(&file.rel_path)
                .map(|extraction| (file, extraction))
        })
        .collect();
    let python_framework_batch = build_python_framework_graph(&python_framework_targets);
    python_framework_batch.verify_declared_source_types(
        pyframework::EXTRACTOR_ID,
        &pyframework::DECLARED_SOURCE_TYPES,
    )?;

    let reference_targets: Vec<(&LoadedFile, &ReferenceExtraction)> = loaded
        .iter()
        .filter_map(|file| {
            reference_extractions
                .get(&file.rel_path)
                .map(|extraction| (file, extraction))
        })
        .collect();
    let reference_batch = build_reference_graph(&config.project_id, &reference_targets);
    reference_batch
        .verify_declared_source_types(refs::EXTRACTOR_ID, &refs::DECLARED_SOURCE_TYPES)?;

    let document_targets: Vec<DocumentTarget<'_>> = loaded
        .iter()
        .filter_map(|file| {
            let extraction = document_extractions.get(&file.rel_path)?;
            let links = document_links.get(&file.rel_path)?;
            let supersessions = document_supersessions.get(&file.rel_path)?;
            Some(DocumentTarget {
                file,
                extraction,
                links,
                supersessions,
            })
        })
        .collect();
    let document_batch = build_document_graph(&config.project_id, &document_targets);
    document_batch
        .verify_declared_source_types(docs::EXTRACTOR_ID, &docs::DECLARED_SOURCE_TYPES)?;

    // ---- identity links ------------------------------------------------------------------
    let removed_candidates: Vec<MoveCandidate> = changes
        .removed
        .iter()
        .filter_map(|path| {
            let module = previous.get(path)?;
            let facts = module.facts.as_ref()?;
            Some(MoveCandidate {
                rel_path: path.clone(),
                content_hash: module.content_hash.clone(),
                symbols: facts.symbols.clone(),
            })
        })
        .collect();
    let added_candidates: Vec<MoveCandidate> = changes
        .added
        .iter()
        .filter_map(|path| {
            let facts = facts_by_path.get(path)?;
            let content_hash = current_hashes.get(path)?.clone();
            Some(MoveCandidate {
                rel_path: path.clone(),
                content_hash,
                symbols: facts.symbols.clone(),
            })
        })
        .collect();
    let proposals = incremental::propose_moves(&removed_candidates, &added_candidates);

    // ---- persist -------------------------------------------------------------------------
    let status = if files_failed > 0 {
        RunStatus::Partial
    } else {
        RunStatus::Complete
    };

    let mut incremental_report = IncrementalReport {
        full: options.full,
        files_unchanged: changes.unchanged.len(),
        files_modified: changes.modified.len(),
        files_added: changes.added.len(),
        files_removed: changes.removed.len(),
        files_resolution_changed: changes.resolution_changed.len(),
        files_seeded: changes.seed().len(),
        files_re_extracted: target_paths.len(),
        files_skipped_unchanged: loaded.len().saturating_sub(target_paths.len()),
        removed_paths: changes.removed.iter().cloned().collect(),
        // One link for the file pair, one for each symbol whose shape survived the move.
        identity_links_proposed: proposals
            .iter()
            .map(|proposal| 1 + proposal.pairs.len())
            .sum(),
        ..IncrementalReport::default()
    };

    // A run that re-extracts every file has the whole repository in scope, so the scoped
    // derivation and pruning would be doing the whole-table work with staging overhead on top.
    // The whole-table statements are used instead. They are the same function evaluated eagerly;
    // `scoped == whole-table` is gated by test, so choosing between them cannot change the
    // answer — only the cost.
    let whole_repository_in_scope = target_paths.len() == loaded.len();

    {
        let tx = conn.transaction().map_err(nerve_store::StoreError::from)?;
        // Rows of Nerve's own model written, updated or deleted by this transaction. See
        // `IncrementalReport::rows_written` for what is deliberately outside the count.
        let mut rows_written = 0usize;
        nerve_store::upsert_repository(
            &tx,
            &nerve_store::RepositoryRow {
                repo_id: repo_id.clone(),
                project_id: config.project_id.clone(),
                root_path: root.to_string_lossy().to_string(),
            },
        )?;
        nerve_store::upsert_repository_state(
            &tx,
            &nerve_store::RepositoryStateRow {
                state_id: state_id.clone(),
                repo_id: repo_id.clone(),
                kind: "content".to_string(),
                git_commit: git_commit.clone(),
                content_merkle,
            },
        )?;

        // 1. Withdraw the evidence this run is about to replace or has lost. Entities and
        //    assertions are left standing until it is known what the new rows support.
        //
        //    `touched` accumulates what this transaction disturbs, which is what bounds the
        //    derivation and pruning below. Deletions must record it as they go: afterwards the
        //    rows that would say so are gone.
        //    Two withdrawals, not one, and the difference is load-bearing. A file that is about
        //    to be **re-extracted** loses only the evidence *these* extractors are about to
        //    replace; a file that has **vanished** loses everything recorded against it, because
        //    evidence about a file that no longer exists is evidence about nothing. Since Slice
        //    6b that distinction has teeth: `coverage` observations cite the covered file's path,
        //    so withdrawing them here would make an ordinary post-edit `nerve index` destroy
        //    every coverage edge — exactly what `docs/plans/slice-06-test-evidence.md` §A.4 made
        //    ingestion a standalone command to prevent. An edited file's coverage stays, and
        //    `observation.content_hash` makes it visibly stale.
        let mut touched = nerve_store::TouchedRows::default();
        let removed_paths: BTreeSet<String> = changes.removed.iter().cloned().collect();
        let mut removals = nerve_store::delete_extractor_file_rows(
            &tx,
            &target_paths,
            &INDEX_EXTRACTOR_IDS,
            &mut touched,
        )?;
        removals.add(nerve_store::delete_file_rows(
            &tx,
            &removed_paths,
            &mut touched,
        )?);
        rows_written += removals.observations + removals.occurrences;

        // Directory containment is the one part of the graph no file path owns, and it is
        // re-derived from the current file set rather than parsed. Clearing it first is only
        // necessary when a file was removed, because that is the only way a directory can stop
        // holding indexed files; doing it unconditionally would make a one-file edit rewrite one
        // row per directory in the repository. Clearing it is not a removal and is deliberately
        // not counted as one — a directory that really did go away shows up in the reported
        // counts as a pruned assertion and entity.
        if !changes.removed.is_empty() {
            rows_written += nerve_store::delete_directory_containment(&tx, &mut touched)?;
        }

        // 2. Write the new evidence. One `extractor_run` row per extractor: the rows are what
        //    make a contribution attributable, and therefore revocable, per extractor version.
        //    The run carries the repository state; no graph row does (ADR-0006).
        //    `fs-structural` goes first, and the order is load-bearing rather than tidy: it now
        //    owns the `File` entity, and the `File` occurrence written by the two extractors that
        //    do read the file carries a foreign key to it. Entity before occurrence, in one
        //    transaction.
        //
        //    Its file tallies count the whole walk. A file that could not be read is still a file
        //    the walk found, but it is not one this extractor could describe, so it counts as
        //    failed here exactly as it does for the extractor that would have parsed it.
        let filesystem_run = nerve_store::begin_extractor_run(
            &tx,
            &repo_id,
            &state_id,
            fsstruct::EXTRACTOR_ID,
            fsstruct::EXTRACTOR_VERSION,
        )?;
        rows_written +=
            nerve_store::persist_batch(&tx, &repo_id, filesystem_run, &fs_batch, &mut touched)?;
        nerve_store::finish_extractor_run(
            &tx,
            filesystem_run,
            loaded.len() as i64,
            files_failed as i64,
            status.as_str(),
        )?;

        let structural_run = nerve_store::begin_extractor_run(
            &tx,
            &repo_id,
            &state_id,
            EXTRACTOR_ID,
            EXTRACTOR_VERSION,
        )?;
        rows_written +=
            nerve_store::persist_batch(&tx, &repo_id, structural_run, &batch, &mut touched)?;
        nerve_store::finish_extractor_run(
            &tx,
            structural_run,
            ts_js_loaded as i64,
            ts_js_failed as i64,
            status.as_str(),
        )?;

        let reference_run = nerve_store::begin_extractor_run(
            &tx,
            &repo_id,
            &state_id,
            refs::EXTRACTOR_ID,
            refs::EXTRACTOR_VERSION,
        )?;
        rows_written += nerve_store::persist_batch(
            &tx,
            &repo_id,
            reference_run,
            &reference_batch,
            &mut touched,
        )?;
        nerve_store::finish_extractor_run(
            &tx,
            reference_run,
            ts_js_loaded as i64,
            ts_js_failed as i64,
            status.as_str(),
        )?;

        // `py-structural` runs unconditionally, for the same reason `md-structural` does: a row
        // saying Nerve looked for Python and found none is a fact, and a row that appeared only
        // when a `.py` file existed would make its absence ambiguous between "no Python here" and
        // "Python was never looked for".
        let python_run = nerve_store::begin_extractor_run(
            &tx,
            &repo_id,
            &state_id,
            pystruct::EXTRACTOR_ID,
            pystruct::EXTRACTOR_VERSION,
        )?;
        rows_written +=
            nerve_store::persist_batch(&tx, &repo_id, python_run, &python_batch, &mut touched)?;
        nerve_store::finish_extractor_run(
            &tx,
            python_run,
            python_loaded as i64,
            python_failed as i64,
            status.as_str(),
        )?;

        // `py-reference` sits after `py-structural` exactly as `ts-js-reference` sits after
        // `ts-js-structural`: every target it resolves to is an entity the structural batch
        // created, and only the `Unresolved` entities are its own.
        let python_reference_run = nerve_store::begin_extractor_run(
            &tx,
            &repo_id,
            &state_id,
            pyrefs::EXTRACTOR_ID,
            pyrefs::EXTRACTOR_VERSION,
        )?;
        rows_written += nerve_store::persist_batch(
            &tx,
            &repo_id,
            python_reference_run,
            &python_reference_batch,
            &mut touched,
        )?;
        nerve_store::finish_extractor_run(
            &tx,
            python_reference_run,
            python_loaded as i64,
            python_failed as i64,
            status.as_str(),
        )?;

        // `py-framework` sits after `py-reference` for the same reason that sits after
        // `py-structural`: every handler it names is an entity the structural batch created. The
        // `Endpoint` entities are its own, and nothing else creates them.
        let python_framework_run = nerve_store::begin_extractor_run(
            &tx,
            &repo_id,
            &state_id,
            pyframework::EXTRACTOR_ID,
            pyframework::EXTRACTOR_VERSION,
        )?;
        rows_written += nerve_store::persist_batch(
            &tx,
            &repo_id,
            python_framework_run,
            &python_framework_batch,
            &mut touched,
        )?;
        nerve_store::finish_extractor_run(
            &tx,
            python_framework_run,
            python_loaded as i64,
            python_failed as i64,
            status.as_str(),
        )?;

        // `md-structural` runs unconditionally, exactly as the others do. A run over a
        // repository with no documents records that Nerve looked and found none, which is a
        // fact; making the row conditional on content would make its absence ambiguous.
        let document_run = nerve_store::begin_extractor_run(
            &tx,
            &repo_id,
            &state_id,
            docs::EXTRACTOR_ID,
            docs::EXTRACTOR_VERSION,
        )?;
        rows_written +=
            nerve_store::persist_batch(&tx, &repo_id, document_run, &document_batch, &mut touched)?;
        nerve_store::finish_extractor_run(
            &tx,
            document_run,
            documents_loaded as i64,
            documents_failed as i64,
            status.as_str(),
        )?;

        // 3. Derivation runs inside the same transaction: the graph and the state derived from
        //    it become visible together or not at all. It runs *before* orphan pruning, which is
        //    what makes the pruning safe — derivation leaves no derived row behind for an
        //    assertion nothing observes, so deleting that assertion breaks no foreign key.
        let derived = if whole_repository_in_scope {
            nerve_store::rebuild_assertion_state(&tx)?
        } else {
            nerve_store::derive_assertion_state_for(&tx, &touched.assertions)?
        };
        rows_written += derived.total();

        // 4. Remove what nothing supports any more.
        let pruned = if whole_repository_in_scope {
            nerve_store::prune_orphans(&tx)?
        } else {
            nerve_store::prune_orphans_scoped(&tx, &touched)?
        };
        rows_written += pruned.assertions + pruned.entities;
        removals.add(pruned);

        // 5. Refresh the extraction cache.
        for path in &changes.removed {
            if nerve_store::delete_module_facts(&tx, &repo_id, path)? {
                rows_written += 1;
            }
        }
        for file in &loaded {
            if !target_paths.contains(&file.rel_path) {
                continue;
            }
            let facts = facts_by_path
                .get(&file.rel_path)
                .cloned()
                .unwrap_or_default();
            // A row records the versions of the extractors that actually produced it. A document
            // is `md-structural` twice over, so its two columns are the same; a Python module has
            // two extractors of its own, exactly as a TypeScript one does. That is what makes a
            // bump to a TS/JS extractor leave every `.py` file alone, and a bump to either Python
            // extractor invalidate exactly the Python ones.
            // `load_previous_modules` reads the columns back through the same rule.
            //
            // The third column is the framework extractor, added in Slice 10a. Empty means *no
            // framework extractor ran and none was expected*, which is what a document says and
            // what TS/JS says until Slice 10b gives it a rule. It is a separate slot rather than a
            // suffix on either of the others because the three extractors are independent: a
            // framework-rule change must re-extract framework observations without re-parsing
            // every reference.
            let (structural_version, reference_version, framework_version) = if file.kind.is_doc() {
                (docs::EXTRACTOR_VERSION, docs::EXTRACTOR_VERSION, "")
            } else if file.language().is_some_and(Language::is_python) {
                (
                    pystruct::EXTRACTOR_VERSION,
                    pyrefs::EXTRACTOR_VERSION,
                    pyframework::EXTRACTOR_VERSION,
                )
            } else {
                (EXTRACTOR_VERSION, refs::EXTRACTOR_VERSION, "")
            };
            rows_written += nerve_store::upsert_module_facts(
                &tx,
                &repo_id,
                &nerve_store::ModuleFactsRow {
                    rel_path: file.rel_path.clone(),
                    content_hash: file.content_hash.clone(),
                    language: file.kind.as_str().to_string(),
                    structural_version: structural_version.to_string(),
                    reference_version: reference_version.to_string(),
                    framework_version: framework_version.to_string(),
                    facts: facts.to_json()?,
                },
            )?;
        }

        // 6. Propose identity links. Proposals only — nothing merges the two identities.
        for proposal in &proposals {
            let file_evidence = serde_json::json!({
                "rule": "body-digest symbol correspondence between a removed and an added file",
                "from_path": proposal.from_path,
                "to_path": proposal.to_path,
                "matched_symbols": proposal.matched,
                "from_symbols": proposal.from_symbols,
                "to_symbols": proposal.to_symbols,
                "file_content_hash_equal": proposal.content_hash_equal,
                "similarity_threshold": format!(
                    "{}/{}",
                    incremental::MOVE_SIMILARITY_NUMERATOR,
                    incremental::MOVE_SIMILARITY_DENOMINATOR
                ),
            });
            if nerve_store::insert_identity_link(
                &tx,
                &repo_id,
                &ids::file_id(&config.project_id, &proposal.from_path),
                &ids::file_id(&config.project_id, &proposal.to_path),
                "moved_file",
                &file_evidence.to_string(),
            )? {
                incremental_report.identity_links_recorded += 1;
            }

            for (before, after) in &proposal.pairs {
                let evidence = serde_json::json!({
                    "rule": "identical (kind, name, scope_path, body digest) across a file move",
                    "from_path": proposal.from_path,
                    "to_path": proposal.to_path,
                    "kind": before.kind,
                    "name": before.name,
                    "scope_path": before.scope_path,
                    "body_hash": before.body_hash,
                    "matched_symbols": proposal.matched,
                    "from_symbols": proposal.from_symbols,
                    "to_symbols": proposal.to_symbols,
                });
                if nerve_store::insert_identity_link(
                    &tx,
                    &repo_id,
                    &before.entity_id,
                    &after.entity_id,
                    "moved_symbol",
                    &evidence.to_string(),
                )? {
                    incremental_report.identity_links_recorded += 1;
                }
            }
        }

        incremental_report.observations_removed = removals.observations;
        incremental_report.occurrences_removed = removals.occurrences;
        incremental_report.assertions_removed = removals.assertions;
        incremental_report.entities_removed = removals.entities;
        rows_written += incremental_report.identity_links_recorded;
        incremental_report.assertions_derived = derived.written;
        incremental_report.rows_written = rows_written;

        tx.commit().map_err(nerve_store::StoreError::from)?;
    }

    let report = nerve_store::status(&conn)?;

    Ok(IndexOutcome {
        root,
        state_id,
        git_commit,
        files_processed: loaded.len(),
        files_failed,
        files_with_syntax_errors,
        denied_secrets: discovery.denied_secrets,
        skipped_unsupported: discovery.skipped_unsupported,
        skipped_symlinks: discovery.skipped_symlinks,
        dynamic_imports_without_specifier,
        unmodelled_call_sites,
        unmodelled_by_form,
        endpoints,
        framework_unsupported_by_form,
        documents_processed: documents_loaded,
        adr_documents,
        document_sections,
        unsupported_markdown,
        unsupported_markdown_by_form,
        supersession_edges: supersedes.len(),
        supersession_cycles: supersession_cycle_count,
        supersession_cycle_documents,
        supersession_contradictions: supersession_contradiction_count,
        entities_by_kind: report.entities_by_kind,
        assertions_by_relation: report.assertions_by_relation,
        entities_total: report.entities_total,
        symbols_total: report.symbols_total,
        assertions_total: report.assertions_total,
        observations_total: report.observations_total,
        unresolved_entities: report.unresolved_entities,
        unresolved_assertions: report.unresolved_assertions,
        duration_ms: started.elapsed().as_millis(),
        status,
        incremental: incremental_report,
    })
}

/// Read the previous run's per-module cache, deciding for each row whether it can be reused.
///
/// A row is reusable only when its payload parses **and** the extractor versions that wrote it
/// match this build. A version bump means the extractor's output may differ over identical
/// bytes, which is exactly what the version field exists to say, so every affected file is
/// re-extracted.
///
/// The versions compared depend on which extractor owns the row. A document is produced entirely
/// by `md-structural`, and a Python module by `py-structural` plus `py-reference`, so bumping a
/// TypeScript extractor must not re-scan either, and bumping either Python extractor must re-scan
/// every `.py` file and nothing else. The row says which it is, through the `language` column and
/// the path — both, so that a row rewritten by hand cannot smuggle a document or a Python module
/// past the TS/JS version check.
fn load_previous_modules(
    conn: &nerve_store::Connection,
    repo_id: &str,
) -> Result<BTreeMap<String, PreviousModule>> {
    let rows = nerve_store::load_module_facts(conn, repo_id)?;
    let mut previous = BTreeMap::new();
    for (rel_path, row) in rows {
        let facts = ModuleFacts::from_json(&row.facts);
        let is_document = path_is_document(&rel_path) || row.language == MARKDOWN_LANGUAGE;
        let is_python = path_is_python(&rel_path) || row.language == crate::lang::PYTHON_LANGUAGE;
        // All three slots must match, and the third is the one a pre-Slice-10a row cannot satisfy:
        // it holds `''` there, `py-framework` has a real version, so every Python file in an
        // existing index misses and is re-extracted. That is the required upgrade behaviour, and it
        // is invisible to every test that builds a fresh index — the exact defect Slice 9b shipped.
        let versions_match = if is_document {
            row.structural_version == docs::EXTRACTOR_VERSION
                && row.reference_version == docs::EXTRACTOR_VERSION
                && row.framework_version.is_empty()
        } else if is_python {
            row.structural_version == pystruct::EXTRACTOR_VERSION
                && row.reference_version == pyrefs::EXTRACTOR_VERSION
                && row.framework_version == pyframework::EXTRACTOR_VERSION
        } else {
            row.structural_version == EXTRACTOR_VERSION
                && row.reference_version == refs::EXTRACTOR_VERSION
                && row.framework_version.is_empty()
        };
        let reusable = facts.is_some() && versions_match;
        previous.insert(
            rel_path,
            PreviousModule {
                content_hash: row.content_hash,
                reusable,
                facts,
            },
        );
    }
    Ok(previous)
}

/// Cached facts for a path that was not re-extracted.
///
/// Empty facts are the safe default: a module that exports nothing and imports nothing
/// contributes nothing to anyone else's resolution. It cannot occur in practice, because a file
/// without reusable cached facts is forced into the invalidation set before this is reached.
fn cached_facts(previous: &BTreeMap<String, PreviousModule>, rel_path: &str) -> ModuleFacts {
    previous
        .get(rel_path)
        .and_then(|module| module.facts.clone())
        .unwrap_or_default()
}

/// Turn the discovery walk into the repository skeleton: `fs-structural`.
///
/// Everything here is a **filesystem** fact — the repository exists, these directories exist,
/// this directory holds that entry — and every observation carries `FILESYSTEM_OBSERVED`. Until
/// Slice 5d-i these same rows said `AST_DIRECT` and named `ts-js-structural`, which was false in
/// any repository containing no TypeScript and misleading in every other.
///
/// `entries` is the whole tree and `targets` is the subset being re-extracted. The **skeleton** —
/// the repository entity, the directory entities, and the `CONTAINS` edges between directories —
/// is re-derived from `entries` on every run, because it needs no parsing and because directory
/// containment is the one part of the graph that no file path owns. `File` entities and the
/// `CONTAINS` edge that holds each one are emitted only for `targets`; a file that was not
/// re-extracted keeps the rows it already has, which is what keeps a run proportional to the
/// change rather than to the repository.
///
/// The signature is the content-independence proof: [`FsEntry`] has no field that can hold file
/// text, so this function has nothing to leak even if it tried. See [`crate::fsstruct`].
fn build_fs_graph(project_id: &str, entries: &[FsEntry], targets: &BTreeSet<String>) -> GraphBatch {
    let mut builder = GraphBuilder::new(fsstruct::EXTRACTOR_ID, fsstruct::EXTRACTOR_VERSION);

    let repo_id = ids::repository_id(project_id);
    // The repository's display name is its own relative path. The directory basename is
    // deliberately not used: it is machine state, and an index must survive a move.
    builder.add_entity(EntityRecord {
        entity_id: repo_id.clone(),
        kind: EntityKind::Repository,
        name: ".".to_string(),
        scope_path: String::new(),
        language: None,
        meta: None,
    });

    // Directories that actually contain an indexed file.
    let mut directories: BTreeSet<String> = BTreeSet::new();
    for entry in entries {
        let mut ancestor = parent_directory(&entry.rel_path);
        while let Some(directory) = ancestor {
            ancestor = parent_directory(&directory);
            directories.insert(directory);
        }
    }

    // A directory has no bytes of its own, so the observation cites a digest of its path. That is
    // deliberately not a file hash: nothing about a directory's existence depends on what is
    // inside the files it holds.
    let directory_hash = |rel_path: &str| ids::content_hash(rel_path.as_bytes());

    for directory in &directories {
        let entity_id = ids::directory_id(project_id, directory);
        let parent = parent_directory(directory);
        builder.add_entity(EntityRecord {
            entity_id: entity_id.clone(),
            kind: EntityKind::Directory,
            name: last_segment(directory).to_string(),
            scope_path: parent.clone().unwrap_or_default(),
            language: None,
            meta: None,
        });
        let parent_id = match &parent {
            Some(parent) => ids::directory_id(project_id, parent),
            None => repo_id.clone(),
        };
        builder.claim(
            &parent_id,
            Relation::Contains,
            &entity_id,
            Evidence::FILESYSTEM,
            directory,
            Span::NONE,
            &directory_hash(directory),
            serde_json::json!({ "child_kind": "directory" }),
        );
    }

    for entry in entries {
        let rel_path = entry.rel_path.as_str();
        if !targets.contains(rel_path) {
            continue;
        }
        let hash = entry.content_hash.as_str();
        let file_entity = ids::file_id(project_id, rel_path);
        let parent = parent_directory(rel_path);
        let name = last_segment(rel_path);

        // Name, extension, size and language tag are all walk metadata. Nothing here was read
        // out of the file; the extension decides the language tag, not the bytes.
        builder.add_entity(EntityRecord {
            entity_id: file_entity.clone(),
            kind: EntityKind::File,
            name: name.to_string(),
            scope_path: parent.clone().unwrap_or_default(),
            language: Some(entry.kind.as_str().to_string()),
            meta: Some(
                serde_json::json!({
                    "extension": name.rsplit('.').next().unwrap_or_default(),
                    "size_bytes": entry.size_bytes,
                })
                .to_string(),
            ),
        });

        // Repository or directory CONTAINS this file — for a `.ts` and a `.md` alike. Having two
        // extractors answer that question differently by file extension was the incoherence
        // Slice 5d-i exists to remove.
        let parent_id = match &parent {
            Some(parent) => ids::directory_id(project_id, parent),
            None => repo_id.clone(),
        };
        builder.claim(
            &parent_id,
            Relation::Contains,
            &file_entity,
            Evidence::FILESYSTEM,
            rel_path,
            Span::NONE,
            hash,
            serde_json::json!({ "child_kind": "file" }),
        );
    }

    builder.batch
}

/// Turn per-module extractions into entities, occurrences, assertions and observations.
///
/// `targets` is the subset of the tree being re-extracted. The repository skeleton is **not**
/// built here — it is a filesystem fact and belongs to [`build_fs_graph`]. What stays is what a
/// parse genuinely produced: `File DEFINES Module`, `Module DEFINES <symbol>`, and every symbol
/// entity, occurrence and edge, all `AST_DIRECT` or `AST_RESOLVED`.
///
/// The `File` **occurrence** stays here rather than moving with the entity, and the distinction
/// is not arbitrary: an occurrence is a *span*, and a whole-file span's end line is the file's
/// line count — something only reading the file can produce. Attributing it to an extractor that
/// never opens a file would be the same category error this slice exists to remove. An occurrence
/// carries no evidence label, so nothing false is asserted by leaving it where the span is made.
///
/// `module_exports` covers **every** module, not just the targets: resolving a re-export in a
/// target file requires the export map of the module it names, which may not have been parsed.
fn build_graph(
    project_id: &str,
    targets: &[(&LoadedFile, &ModuleExtraction)],
    module_exports: &BTreeMap<String, BTreeMap<String, String>>,
    indexed: &BTreeSet<String>,
) -> Result<GraphBatch> {
    let mut builder = GraphBuilder::new(EXTRACTOR_ID, EXTRACTOR_VERSION);

    for (file, extraction) in targets.iter().copied() {
        let rel_path = file.rel_path.as_str();
        let hash = file.content_hash.as_str();
        let file_entity = ids::file_id(project_id, rel_path);
        let module_entity = ids::module_id(project_id, rel_path);
        let name = last_segment(rel_path);

        builder.add_occurrence(&file_entity, rel_path, extraction.file_span, hash);

        // File DEFINES Module, 1:1 for TS/JS.
        builder.add_entity(EntityRecord {
            entity_id: module_entity.clone(),
            kind: EntityKind::Module,
            name: file_stem(name).to_string(),
            scope_path: rel_path.to_string(),
            language: Some(file.kind.as_str().to_string()),
            meta: None,
        });
        builder.add_occurrence(&module_entity, rel_path, extraction.file_span, hash);
        builder.claim(
            &file_entity,
            Relation::Defines,
            &module_entity,
            Evidence::DIRECT,
            rel_path,
            extraction.file_span,
            hash,
            serde_json::json!({ "language": file.kind.as_str() }),
        );

        // Symbols.
        for symbol in &extraction.symbols {
            builder.add_entity(EntityRecord {
                entity_id: symbol.entity_id.clone(),
                kind: symbol.kind,
                name: symbol.name.clone(),
                scope_path: symbol.scope_path.clone(),
                language: Some(file.kind.as_str().to_string()),
                meta: symbol.meta.clone(),
            });
            builder.add_occurrence(&symbol.entity_id, rel_path, symbol.span, hash);

            // A method is defined by its class; everything else by the module. Lexical nesting
            // deeper than that is carried by scope_path, not by an edge: Slice 1's declared
            // graph shape has no Function DEFINES Function.
            let (definer, detail) = match &symbol.owner_class {
                Some(class_id) => (class_id.clone(), "class"),
                None => (module_entity.clone(), "module"),
            };
            builder.claim(
                &definer,
                Relation::Defines,
                &symbol.entity_id,
                Evidence::DIRECT,
                rel_path,
                symbol.span,
                hash,
                serde_json::json!({
                    "declaration_kind": symbol.kind.as_str(),
                    "definer": detail,
                    "scope_path": symbol.scope_path,
                }),
            );
        }

        // Exports of locally defined symbols.
        for export in &extraction.local_exports {
            let target = match &export.target {
                ExportTarget::Symbol(index) => Some(extraction.symbols[*index].entity_id.clone()),
                ExportTarget::LocalName(name) => extraction
                    .top_level_symbol(name)
                    .map(|index| extraction.symbols[index].entity_id.clone()),
            };
            let Some(target) = target else {
                // `export { somethingImported }` re-exports a binding this module did not
                // define. Slice 1 does not track bindings, so no edge is invented.
                continue;
            };
            builder.claim(
                &module_entity,
                Relation::Exports,
                &target,
                Evidence::DIRECT,
                rel_path,
                export.span,
                hash,
                serde_json::json!({
                    "export_kind": if export.exported_name == "default" { "default" } else { "named" },
                    "exported_name": export.exported_name,
                }),
            );
        }

        // Re-exports: the entity keeps its defining module's identity (ADR-0002).
        for re_export in &extraction.re_exports {
            let Some(target_module) = resolve::resolve(rel_path, &re_export.raw_specifier, indexed)
            else {
                continue;
            };
            let Some(exports) = module_exports.get(&target_module) else {
                continue;
            };
            let selected: Vec<(String, String)> = match &re_export.names {
                Some(names) => names
                    .iter()
                    .filter_map(|(name, alias)| {
                        exports.get(name).map(|entity_id| {
                            (
                                alias.clone().unwrap_or_else(|| name.clone()),
                                entity_id.clone(),
                            )
                        })
                    })
                    .collect(),
                None => exports
                    .iter()
                    .filter(|(name, _)| name.as_str() != "default")
                    .map(|(name, entity_id)| (name.clone(), entity_id.clone()))
                    .collect(),
            };
            for (exported_name, target) in selected {
                builder.claim(
                    &module_entity,
                    Relation::Exports,
                    &target,
                    Evidence::RESOLVED,
                    rel_path,
                    re_export.span,
                    hash,
                    serde_json::json!({
                        "export_kind": "re-export",
                        "exported_name": exported_name,
                        "raw_specifier": re_export.raw_specifier,
                        "resolved_path": target_module,
                    }),
                );
            }
        }

        // Imports.
        for import in &extraction.imports {
            let resolved = resolve::resolve(rel_path, &import.raw_specifier, indexed);
            let specifiers: Vec<serde_json::Value> = import
                .specifiers
                .iter()
                .map(|specifier| {
                    serde_json::json!({
                        "imported": specifier.imported,
                        "kind": specifier.kind,
                        "local": specifier.local,
                    })
                })
                .collect();

            let (target, evidence) = match &resolved {
                // Module resolution ran and succeeded: AST_RESOLVED, per ADR-0003.
                Some(target_module) => (
                    ids::module_id(project_id, target_module),
                    Evidence::RESOLVED,
                ),
                // Nothing was resolved. All the tree states is a specifier, so AST_DIRECT.
                None => {
                    let entity_id = ids::unresolved_id(
                        project_id,
                        rel_path,
                        UnresolvedCategory::Module,
                        &import.raw_specifier,
                    );
                    builder.add_entity(EntityRecord {
                        entity_id: entity_id.clone(),
                        kind: EntityKind::Unresolved,
                        name: import.raw_specifier.clone(),
                        scope_path: rel_path.to_string(),
                        language: None,
                        meta: Some(
                            serde_json::json!({
                                "category": UnresolvedCategory::Module.as_str(),
                                "importer": rel_path,
                                "raw_specifier": import.raw_specifier,
                                "reason": if resolve::is_relative(&import.raw_specifier) {
                                    "relative specifier does not name an indexed file"
                                } else {
                                    "non-relative specifier; resolution is relative-path only"
                                },
                            })
                            .to_string(),
                        ),
                    });
                    builder.add_occurrence(&entity_id, rel_path, import.span, hash);
                    (entity_id, Evidence::DIRECT)
                }
            };

            builder.claim(
                &module_entity,
                Relation::Imports,
                &target,
                evidence,
                rel_path,
                import.span,
                hash,
                serde_json::json!({
                    "form": import.form.as_str(),
                    "raw_specifier": import.raw_specifier,
                    "resolved_path": resolved,
                    "specifiers": specifiers,
                    "type_only": import.type_only,
                }),
            );
        }
    }

    Ok(builder.batch)
}

/// Why a Python specifier did not resolve. Closed, because a reader who wants to know what a
/// repository's imports actually reach needs to enumerate the refusals rather than guess them.
mod py_reason {
    /// The specifier names no file in the index: the standard library, an installed package, or
    /// a package root Nerve does not model (a `src/` layout, `PYTHONPATH`, an editable install).
    pub const NOT_INDEXED: &str =
        "specifier names no indexed module (standard library, installed package, or a package \
         root outside the repository)";
    /// The specifier names a directory holding indexed files but no `__init__.py`.
    pub const NAMESPACE_PACKAGE: &str =
        "names a directory with no __init__.py: a namespace package, whose contents are \
         assembled at import time from every sys.path entry of that name";
    /// A relative specifier with more dots than the importer has parent packages.
    pub const CLIMBS_ABOVE_ROOT: &str = "relative specifier climbs above the repository root";
    /// The importing module rewrites `sys.path`.
    pub const SYS_PATH_MUTATED: &str =
        "sys.path mutated by the importing module, so no absolute specifier in it can be claimed \
         to name a repository file; relative imports are unaffected";
    /// A wildcard import.
    pub const WILDCARD: &str =
        "wildcard import: the names it binds live in the imported module's runtime namespace and \
         are not knowable from this file";
    /// An import that is not at module top level.
    pub const CONDITIONAL: &str =
        "conditional import: the statement is not at module top level, so whether it binds at \
         module scope is not statically decidable";
    /// `importlib.import_module(...)` or `__import__(...)`.
    pub const DYNAMIC: &str = "dynamic import: the module is chosen at runtime";
}

/// The `Unresolved` name for the bindings an import site makes unknowable.
///
/// Prefixed by *why*, so that a wildcard and a conditional import of the same module are two
/// findings rather than one, and so that the entity's name states its own reason.
fn py_binding_name(site: &PyImportSite, prefix: &str) -> String {
    match (&site.dynamic_callee, site.raw_specifier.as_str()) {
        (Some(callee), _) => format!("{prefix}:{callee}"),
        (None, specifier) => format!("{prefix}:{specifier}"),
    }
}

/// Add one `Unresolved` entity and the `IMPORTS` edge that cites it.
#[allow(clippy::too_many_arguments)]
fn claim_unresolved_python_import(
    builder: &mut GraphBuilder,
    project_id: &str,
    rel_path: &str,
    content_hash: &str,
    module_entity: &str,
    category: UnresolvedCategory,
    name: &str,
    reason: &str,
    site: &PyImportSite,
    details: serde_json::Value,
) {
    let entity_id = ids::unresolved_id(project_id, rel_path, category, name);
    builder.add_entity(EntityRecord {
        entity_id: entity_id.clone(),
        kind: EntityKind::Unresolved,
        name: name.to_string(),
        scope_path: rel_path.to_string(),
        language: None,
        meta: Some(
            serde_json::json!({
                "category": category.as_str(),
                "importer": rel_path,
                "raw_specifier": if site.raw_specifier.is_empty() {
                    serde_json::Value::Null
                } else {
                    serde_json::Value::String(site.raw_specifier.clone())
                },
                "reason": reason,
            })
            .to_string(),
        ),
    });
    builder.add_occurrence(&entity_id, rel_path, site.span, content_hash);
    builder.claim(
        module_entity,
        Relation::Imports,
        &entity_id,
        // Nothing was resolved. All the tree states is a statement, so `AST_DIRECT`.
        Evidence::DIRECT,
        rel_path,
        site.span,
        content_hash,
        details,
    );
}

/// The `details` payload shared by every Python import observation.
///
/// Uniform across the resolved and the unresolved shape, for the same reason `link_details` is:
/// "which imports in this repository reach nothing?" must read the same keys whatever the answer
/// was.
fn py_import_details(
    site: &PyImportSite,
    resolved_path: Option<&str>,
    reason: Option<&str>,
) -> serde_json::Value {
    let bindings: Vec<serde_json::Value> = site
        .bindings
        .iter()
        .map(|binding| {
            serde_json::json!({
                "imported": binding.imported,
                "local": binding.local,
            })
        })
        .collect();
    serde_json::json!({
        "form": site.form.as_str(),
        "raw_specifier": if site.raw_specifier.is_empty() {
            serde_json::Value::Null
        } else {
            serde_json::Value::String(site.raw_specifier.clone())
        },
        "resolved_path": resolved_path,
        "bindings": bindings,
        "conditional": site.conditional,
        "dynamic_callee": site.dynamic_callee,
        "literal_argument": site.literal_argument,
        "reason": reason,
    })
}

/// Turn per-module Python extractions into entities, occurrences, assertions and observations.
///
/// A separate batch under a separate extractor run, exactly as `ts-js-reference` and
/// `md-structural` are. What it emits:
///
/// - `File DEFINES Module`, 1:1 with a `.py` file, with packageness on the module's `meta`;
/// - `Module DEFINES <symbol>` and `Class DEFINES <method>`;
/// - `Module IMPORTS Module` for a specifier that names an indexed module, `AST_RESOLVED`;
/// - `Module IMPORTS Unresolved` for one that does not, `AST_DIRECT`, with a closed-vocabulary
///   reason;
/// - `Module IMPORTS Unresolved` a **second** time for a site whose *bound names* are not
///   knowable — a wildcard, a conditional import, a dynamic import. The two are separate claims
///   about one statement: which module it names, and which names it binds. Recording only the
///   first would silently assert that `from x import *` binds nothing;
/// - `Module EXPORTS <symbol>` for each `__all__` entry naming a symbol this module defines.
///
/// **No `EXTENDS`, no `CALLS`, no `REFERENCES`** — Slice 9b's — and no `IMPLEMENTS` in any slice,
/// because Python has no `implements` keyword to state one.
fn build_python_graph(
    project_id: &str,
    targets: &[(&LoadedFile, &PyModuleExtraction)],
    indexed: &BTreeSet<String>,
) -> Result<GraphBatch> {
    let mut builder = GraphBuilder::new(pystruct::EXTRACTOR_ID, pystruct::EXTRACTOR_VERSION);

    for (file, extraction) in targets.iter().copied() {
        let rel_path = file.rel_path.as_str();
        let hash = file.content_hash.as_str();
        let file_entity = ids::file_id(project_id, rel_path);
        let module_entity = ids::module_id(project_id, rel_path);
        let name = last_segment(rel_path);

        builder.add_occurrence(&file_entity, rel_path, extraction.file_span, hash);

        // File DEFINES Module, 1:1 for Python as for TS/JS.
        //
        // Packageness rides on `meta` rather than on a new `EntityKind`. A directory holding an
        // `__init__.py` really is a package, but the vocabulary is mirrored in
        // `apps/nerve-web/src/api/types.ts`, classified by `EntityKind::path_role` and pinned by
        // two exhaustiveness tests — and the fact is already 1:1 with a file Nerve indexes, so a
        // member would buy nothing a boolean does not.
        builder.add_entity(EntityRecord {
            entity_id: module_entity.clone(),
            kind: EntityKind::Module,
            name: file_stem(name).to_string(),
            scope_path: rel_path.to_string(),
            language: Some(file.kind.as_str().to_string()),
            meta: Some(
                serde_json::json!({
                    "package": extraction.is_package,
                    "sys_path_mutated": extraction.sys_path_mutated,
                })
                .to_string(),
            ),
        });
        builder.add_occurrence(&module_entity, rel_path, extraction.file_span, hash);
        builder.claim(
            &file_entity,
            Relation::Defines,
            &module_entity,
            Evidence::DIRECT,
            rel_path,
            extraction.file_span,
            hash,
            serde_json::json!({
                "language": file.kind.as_str(),
                "package": extraction.is_package,
            }),
        );

        // Symbols.
        for symbol in &extraction.symbols {
            builder.add_entity(EntityRecord {
                entity_id: symbol.entity_id.clone(),
                kind: symbol.kind,
                name: symbol.name.clone(),
                scope_path: symbol.scope_path.clone(),
                language: Some(file.kind.as_str().to_string()),
                meta: symbol.meta.clone(),
            });
            builder.add_occurrence(&symbol.entity_id, rel_path, symbol.span, hash);

            let (definer, detail) = match &symbol.owner_class {
                Some(class_id) => (class_id.clone(), "class"),
                None => (module_entity.clone(), "module"),
            };
            builder.claim(
                &definer,
                Relation::Defines,
                &symbol.entity_id,
                Evidence::DIRECT,
                rel_path,
                symbol.span,
                hash,
                serde_json::json!({
                    "declaration_kind": symbol.kind.as_str(),
                    "definer": detail,
                    "scope_path": symbol.scope_path,
                }),
            );
        }

        // Exports. Python states a public surface exactly once, through `__all__`; a module
        // without one exports nothing, and saying so is more honest than promoting every
        // underscore-free top-level name on a naming convention.
        for exported_name in extraction.exported_names() {
            let Some(index) = extraction.top_level_symbol(exported_name) else {
                // `__all__` lists a name this module imported rather than defined, or one it
                // never binds at all. Slice 9a does not track bindings, so no edge is invented —
                // the same refusal `ts-js-structural` makes for `export { somethingImported }`.
                continue;
            };
            let symbol = &extraction.symbols[index];
            builder.claim(
                &module_entity,
                Relation::Exports,
                &symbol.entity_id,
                Evidence::DIRECT,
                rel_path,
                symbol.span,
                hash,
                serde_json::json!({
                    "export_kind": "__all__",
                    "exported_name": exported_name,
                }),
            );
        }

        // Imports.
        for site in &extraction.imports {
            if site.form != PyImportForm::Dynamic {
                let relative = pyresolve::is_relative(&site.raw_specifier);
                // `sys.path` is consulted for absolute specifiers only; a relative import is
                // resolved from `__package__` and is untouched by the rewrite.
                let poisoned = extraction.sys_path_mutated && !relative;
                let resolved = if poisoned {
                    None
                } else {
                    pyresolve::resolve(rel_path, &site.raw_specifier, indexed)
                };

                match &resolved {
                    Some(target_module) => builder.claim(
                        &module_entity,
                        Relation::Imports,
                        &ids::module_id(project_id, target_module),
                        Evidence::RESOLVED,
                        rel_path,
                        site.span,
                        hash,
                        py_import_details(site, Some(target_module), None),
                    ),
                    None => {
                        let reason = if poisoned {
                            py_reason::SYS_PATH_MUTATED
                        } else if pyresolve::climbs_above_root(rel_path, &site.raw_specifier) {
                            py_reason::CLIMBS_ABOVE_ROOT
                        } else if pyresolve::is_namespace_package(
                            rel_path,
                            &site.raw_specifier,
                            indexed,
                        ) {
                            py_reason::NAMESPACE_PACKAGE
                        } else {
                            py_reason::NOT_INDEXED
                        };
                        claim_unresolved_python_import(
                            &mut builder,
                            project_id,
                            rel_path,
                            hash,
                            &module_entity,
                            UnresolvedCategory::Module,
                            &site.raw_specifier,
                            reason,
                            site,
                            py_import_details(site, None, Some(reason)),
                        );
                    }
                }
            }

            // The second claim: which names does this statement bind? A wildcard, a conditional
            // site and a dynamic call each make that unknowable, and each is recorded on its own
            // so that the finding survives the module edge resolving perfectly well.
            //
            // A dynamic import is exempt from the conditional finding, and not to reduce noise:
            // `importlib.import_module(name)` is an **expression**, not a binding statement. It
            // binds no name at module scope whether or not it is reached, so "whether this
            // binding takes effect" is a question it never raises. Recording both would state a
            // second finding about a binding that does not exist — and every dynamic import in
            // practice sits inside a function, so the pair would always travel together and the
            // conditional one would never mean anything.
            let unknowable: [(bool, &str, &str); 3] = [
                (
                    site.form == PyImportForm::Wildcard,
                    "wildcard",
                    py_reason::WILDCARD,
                ),
                (
                    site.conditional.is_some() && site.form != PyImportForm::Dynamic,
                    "conditional",
                    py_reason::CONDITIONAL,
                ),
                (
                    site.form == PyImportForm::Dynamic,
                    "dynamic",
                    py_reason::DYNAMIC,
                ),
            ];
            for (applies, prefix, reason) in unknowable {
                if !applies {
                    continue;
                }
                let name = py_binding_name(site, prefix);
                claim_unresolved_python_import(
                    &mut builder,
                    project_id,
                    rel_path,
                    hash,
                    &module_entity,
                    UnresolvedCategory::Value,
                    &name,
                    reason,
                    site,
                    py_import_details(site, None, Some(reason)),
                );
            }
        }
    }

    Ok(builder.batch)
}

/// Turn Python reference sites into assertions and observations.
///
/// A **separate batch under a separate extractor run**, exactly as `ts-js-reference` is. It
/// creates no symbol entities — every resolved target already exists in the `py-structural`
/// batch — and only the `Unresolved` entities it names itself.
///
/// `CALLS`, `REFERENCES` and `EXTENDS`. **Never `IMPLEMENTS`**: Python has no `implements`
/// keyword, so a base list states inheritance and nothing else.
fn build_python_reference_graph(
    project_id: &str,
    targets: &[(&LoadedFile, &PyReferenceExtraction)],
) -> GraphBatch {
    let mut builder = GraphBuilder::new(pyrefs::EXTRACTOR_ID, pyrefs::EXTRACTOR_VERSION);

    for (file, extraction) in targets.iter().copied() {
        let rel_path = file.rel_path.as_str();
        let hash = file.content_hash.as_str();

        for site in &extraction.sites {
            let (target, evidence) = match &site.target {
                PyRefTarget::Resolved(entity_id) => (entity_id.clone(), Evidence::RESOLVED),
                PyRefTarget::Unresolved { name, reason } => {
                    // `Value` covers it: the closed vocabulary's own gloss is "a value or type
                    // name that no binding in scope could resolve", which is what a callee or a
                    // base class Nerve could not name is. No member was needed.
                    let entity_id =
                        ids::unresolved_id(project_id, rel_path, UnresolvedCategory::Value, name);
                    builder.add_entity(EntityRecord {
                        entity_id: entity_id.clone(),
                        kind: EntityKind::Unresolved,
                        name: name.clone(),
                        scope_path: rel_path.to_string(),
                        language: None,
                        meta: Some(
                            serde_json::json!({
                                "category": UnresolvedCategory::Value.as_str(),
                                "importer": rel_path,
                                "raw_specifier": serde_json::Value::Null,
                                // The first reason recorded for this name in this file. Each
                                // individual site carries its own reason in observation details.
                                "reason": reason.as_str(),
                            })
                            .to_string(),
                        ),
                    });
                    builder.add_occurrence(&entity_id, rel_path, site.span, hash);
                    (entity_id, Evidence::DIRECT)
                }
            };

            builder.claim(
                &site.source_entity_id,
                site.relation,
                &target,
                evidence,
                rel_path,
                site.span,
                hash,
                site.details.clone(),
            );
        }
    }

    builder.batch
}

/// Turn declared HTTP routes into `Endpoint` entities and `SERVED_BY` assertions.
///
/// A **separate batch under a separate extractor run**, exactly as the reference graph is. It
/// creates the `Endpoint` entities — nothing else names them — and no symbol entities: every
/// handler already exists in the structural batch.
///
/// The `Endpoint` is the assertion's **source** and the handler its target. That direction is what
/// makes `nerve impact <handler>` report the endpoint, which is the measured defect Slice 10
/// exists to fix; see `nerve_store::impact`.
fn build_python_framework_graph(targets: &[(&LoadedFile, &PyFrameworkExtraction)]) -> GraphBatch {
    let mut builder = GraphBuilder::new(pyframework::EXTRACTOR_ID, pyframework::EXTRACTOR_VERSION);

    for (file, extraction) in targets.iter().copied() {
        let rel_path = file.rel_path.as_str();
        let hash = file.content_hash.as_str();

        for endpoint in &extraction.endpoints {
            builder.add_entity(EntityRecord {
                entity_id: endpoint.entity_id.clone(),
                kind: EntityKind::Endpoint,
                // The canonical address. FTS5 indexes `name`, so this is what makes
                // `nerve search users` find a route — the second defect the slice measured.
                name: endpoint.address.clone(),
                // The declaring module, exactly as a section's scope is its document.
                scope_path: rel_path.to_string(),
                language: None,
                meta: Some(
                    serde_json::json!({
                        "endpoint_kind": EndpointKind::HttpRoute.as_str(),
                        "framework": endpoint.framework.as_str(),
                        "method": endpoint.method,
                        // The **declared** path. No prefix from `APIRouter(prefix=…)` or a
                        // blueprint registration is composed in.
                        "path": endpoint.path,
                        "rule_id": endpoint.framework.rule_id(),
                        // Whether this address is declared more than once in this module. Both
                        // edges are kept; the ambiguity is reported rather than resolved.
                        "declarations_in_module": extraction
                            .ambiguous_addresses
                            .get(&endpoint.address)
                            .copied()
                            .unwrap_or(1),
                    })
                    .to_string(),
                ),
            });
            builder.add_occurrence(&endpoint.entity_id, rel_path, endpoint.span, hash);

            builder.claim(
                &endpoint.entity_id,
                Relation::ServedBy,
                &endpoint.handler_entity_id,
                Evidence::FRAMEWORK,
                rel_path,
                endpoint.span,
                hash,
                serde_json::json!({
                    "framework": endpoint.framework.as_str(),
                    "rule_id": endpoint.framework.rule_id(),
                    "method": endpoint.method,
                    "path": endpoint.path,
                    // Stated on every observation, because this is the claim a reader is most
                    // likely to over-read.
                    "proves": "the source declares this endpoint and names this symbol as its \
                               handler; not that the route is reachable, permitted by middleware, \
                               or unreplaced by dynamic configuration",
                }),
            );
        }
    }

    builder.batch
}

/// Turn resolved reference sites into assertions and observations.
///
/// This runs as a **separate batch under a separate extractor run**. It creates no symbol
/// entities — every resolved target already exists in the structural batch — and only the
/// `Unresolved` entities it names itself.
fn build_reference_graph(
    project_id: &str,
    targets: &[(&LoadedFile, &ReferenceExtraction)],
) -> GraphBatch {
    let mut builder = GraphBuilder::new(refs::EXTRACTOR_ID, refs::EXTRACTOR_VERSION);

    for (file, extraction) in targets.iter().copied() {
        let rel_path = file.rel_path.as_str();
        let hash = file.content_hash.as_str();

        for site in &extraction.sites {
            let (target, evidence) = match &site.target {
                RefTarget::Resolved(entity_id) => (entity_id.clone(), Evidence::RESOLVED),
                RefTarget::Unresolved { name, reason } => {
                    let entity_id =
                        ids::unresolved_id(project_id, rel_path, UnresolvedCategory::Value, name);
                    builder.add_entity(EntityRecord {
                        entity_id: entity_id.clone(),
                        kind: EntityKind::Unresolved,
                        name: name.clone(),
                        scope_path: rel_path.to_string(),
                        language: None,
                        meta: Some(
                            serde_json::json!({
                                "category": UnresolvedCategory::Value.as_str(),
                                "importer": rel_path,
                                "raw_specifier": serde_json::Value::Null,
                                // The first reason recorded for this name in this file. Each
                                // individual site carries its own reason in observation details.
                                "reason": reason.as_str(),
                            })
                            .to_string(),
                        ),
                    });
                    builder.add_occurrence(&entity_id, rel_path, site.span, hash);
                    (entity_id, Evidence::DIRECT)
                }
            };

            builder.claim(
                &site.source_entity_id,
                site.relation,
                &target,
                evidence,
                rel_path,
                site.span,
                hash,
                site.details.clone(),
            );
        }
    }

    builder.batch
}

/// Observation `details.link_target`: what one document-link edge points at.
///
/// Closed, because a reader filtering the graph for broken documentation links needs to
/// enumerate the answers rather than guess them.
mod link_target {
    /// The edge names the indexed file the destination resolves to.
    pub const FILE: &str = "file";
    /// The edge names the symbol a `#L<n>` anchor resolved to inside that file.
    pub const SYMBOL: &str = "symbol";
    /// The edge names an `Unresolved` entity, and `details.reason` says why.
    pub const UNRESOLVED: &str = "unresolved";
}

/// The `details` payload shared by every document-link observation.
///
/// Uniform across all four emission sites on purpose: a query that asks "which documentation
/// links are broken?" reads the same keys whether the answer is a file, a symbol or a refusal,
/// and a key that is absent for one shape and present for another turns that query into a
/// special case per shape.
fn link_details(
    site: &LinkSite,
    source_kind: &str,
    target_kind: &str,
    resolved_path: Option<&str>,
    target_content_hash: Option<&str>,
    reason: Option<&str>,
) -> serde_json::Value {
    // What the document *wrote*, whether or not it resolved. An anchor that named nothing is
    // still the anchor the author typed, and the citation is owed it.
    let anchor = docref::anchor_of(&site.raw_destination)
        .map(|anchor| serde_json::json!({ "start_line": anchor.start, "end_line": anchor.end }));
    serde_json::json!({
        "form": site.form.as_str(),
        "raw_destination": site.raw_destination,
        "source_kind": source_kind,
        "link_target": target_kind,
        "resolved_path": resolved_path,
        "anchor": anchor,
        // Recorded for an anchored symbol edge only. It is the **target** file's hash, so a
        // later `nerve why` can say the anchor was resolved against bytes the file no longer
        // has. Putting it on the file edge as well would make every document that links to a
        // file a dependent of that file's contents.
        "target_content_hash": target_content_hash,
        "reason": reason,
    })
}

/// Emit one document-link edge whose target is an `Unresolved` entity.
///
/// Unresolved is a value, not an omission: a stale `docs/` link is the single most useful thing
/// this extractor produces, and a link that quietly emitted nothing would be indistinguishable
/// from a document Nerve never read.
///
/// The entity is named by the destination **exactly as written**, fragment included, so that a
/// broken `./util.ts#L900` and a broken `./util.ts` are two unresolved things rather than one.
/// The destination is repository content and stays inert: it becomes an entity name and a JSON
/// string, never a path this process opens.
#[allow(clippy::too_many_arguments)]
fn claim_unresolved_document_link(
    builder: &mut GraphBuilder,
    project_id: &str,
    rel_path: &str,
    content_hash: &str,
    source_entity: &str,
    source_kind: &str,
    site: &LinkSite,
    reason: &str,
    resolved_path: Option<&str>,
) {
    let entity_id = ids::unresolved_id(
        project_id,
        rel_path,
        UnresolvedCategory::DocumentLink,
        &site.raw_destination,
    );
    builder.add_entity(EntityRecord {
        entity_id: entity_id.clone(),
        kind: EntityKind::Unresolved,
        name: site.raw_destination.clone(),
        scope_path: rel_path.to_string(),
        language: None,
        meta: Some(
            serde_json::json!({
                "category": UnresolvedCategory::DocumentLink.as_str(),
                "importer": rel_path,
                "raw_specifier": site.raw_destination,
                "reason": reason,
            })
            .to_string(),
        ),
    });
    builder.add_occurrence(&entity_id, rel_path, site.span, content_hash);
    builder.claim(
        source_entity,
        Relation::References,
        &entity_id,
        // Nothing was resolved. All the document states is a destination, so `DIRECT`.
        Evidence::DOCUMENT,
        rel_path,
        site.span,
        content_hash,
        link_details(
            site,
            source_kind,
            link_target::UNRESOLVED,
            resolved_path,
            None,
            Some(reason),
        ),
    );
}

/// The whole repository's supersession graph, oriented `source replaces target`.
///
/// Built from [`ModuleFacts`] rather than from this run's extractions, so that it describes the
/// repository and not the fraction of it that was re-read — the same reason the Markdown
/// refusal tallies are built that way. Resolution is the same function the graph builder used, so
/// a cached statement and a freshly scanned one cannot be answered differently.
fn supersession_edges(
    root: &Path,
    facts_by_path: &BTreeMap<String, ModuleFacts>,
    corpus: &docref::Corpus<'_>,
) -> BTreeSet<(String, String)> {
    let mut edges = BTreeSet::new();
    for (rel_path, facts) in facts_by_path {
        for statement in &facts.document.supersession {
            let outcome = docref::resolve_supersession(
                root,
                rel_path,
                &statement.target,
                statement.link.as_deref(),
                corpus,
            );
            let SupersessionOutcome::Document { path } = outcome else {
                continue;
            };
            if statement.direction == SupersessionDirection::SupersededBy.as_str() {
                edges.insert((path, rel_path.clone()));
            } else {
                edges.insert((rel_path.clone(), path));
            }
        }
    }
    edges
}

/// Groups of mutually superseding documents, and how many documents are in them.
///
/// **A cycle is not suppressed.** Each edge is individually backed by an explicit statement in a
/// real file, and deleting one to make the graph acyclic would hide evidence and make the choice
/// of which to delete arbitrary. Nerve's rule is that contradictory is a value rather than an
/// omission, so a cycle is detected, counted and reported, and every edge stands.
///
/// The count is of strongly connected components of size greater than one — exactly "sets of
/// documents that each transitively replace the other" — computed by Kosaraju with explicit
/// stacks, because a recursive walk over an attacker-supplied graph is a stack overflow waiting
/// for a long enough chain. Self-supersession never reaches here: it resolves to `Unresolved`,
/// so no self-loop exists to special-case.
fn neighbours<'a>(map: &'a BTreeMap<&str, Vec<&'a str>>, node: &str) -> &'a [&'a str] {
    map.get(node).map(Vec::as_slice).unwrap_or(&[])
}

fn supersession_cycles(edges: &BTreeSet<(String, String)>) -> (usize, usize) {
    let mut nodes: BTreeSet<&str> = BTreeSet::new();
    let mut forward: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
    let mut backward: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
    for (source, target) in edges {
        nodes.insert(source.as_str());
        nodes.insert(target.as_str());
        forward
            .entry(source.as_str())
            .or_default()
            .push(target.as_str());
        backward
            .entry(target.as_str())
            .or_default()
            .push(source.as_str());
    }
    let mut visited: BTreeSet<&str> = BTreeSet::new();
    let mut order: Vec<&str> = Vec::new();
    for node in &nodes {
        if !visited.insert(node) {
            continue;
        }
        let mut stack = vec![(*node, 0usize)];
        while let Some((current, index)) = stack.pop() {
            let next = neighbours(&forward, current);
            if index < next.len() {
                stack.push((current, index + 1));
                if visited.insert(next[index]) {
                    stack.push((next[index], 0));
                }
            } else {
                order.push(current);
            }
        }
    }

    let mut assigned: BTreeSet<&str> = BTreeSet::new();
    let (mut components, mut documents) = (0usize, 0usize);
    for node in order.iter().rev() {
        if !assigned.insert(node) {
            continue;
        }
        let mut size = 0usize;
        let mut stack = vec![*node];
        while let Some(current) = stack.pop() {
            size += 1;
            for next in neighbours(&backward, current) {
                if assigned.insert(next) {
                    stack.push(next);
                }
            }
        }
        if size > 1 {
            components += 1;
            documents += size;
        }
    }
    (components, documents)
}

/// Documents that are the target of a `SUPERSEDES` edge while their own status still reads
/// `Accepted`.
///
/// A string comparison, exactly as the Slice 5 plan §2.3 promised: no semantics are required to
/// see that a document another document claims to have replaced still claims to be in force.
/// Counted per document, not per edge — a decision superseded twice contradicts itself once.
fn supersession_contradictions(
    edges: &BTreeSet<(String, String)>,
    facts_by_path: &BTreeMap<String, ModuleFacts>,
) -> usize {
    let targets: BTreeSet<&String> = edges.iter().map(|(_, target)| target).collect();
    targets
        .into_iter()
        .filter(|target| {
            facts_by_path
                .get(*target)
                .and_then(|facts| facts.document.adr_status.as_deref())
                == Some(docs::AdrStatus::Accepted.as_str())
        })
        .count()
}

/// The `details` payload shared by every supersession observation.
///
/// Uniform across the resolved and the unresolved shape, for the same reason
/// [`link_details`] is: "which supersession claims are broken?" must read the same keys whatever
/// the answer was.
fn supersession_details(
    site: &SupersessionSite,
    resolved_path: Option<&str>,
    reason: Option<&str>,
) -> serde_json::Value {
    // Every path that matched an ambiguous identifier. Recorded so the refusal names what it
    // refused rather than only that it refused: the whole value of declining to guess is that a
    // reader can see the alternatives and decide.
    let candidates = match &site.outcome {
        SupersessionOutcome::Ambiguous { candidates } => Some(candidates.clone()),
        _ => None,
    };
    serde_json::json!({
        // Which field was written. The stored edge always means "source replaces target", so
        // this is the only record of which way round the document said it.
        "field": site.direction.as_str(),
        "form": site.form,
        "raw_target": site.raw_target,
        "resolved_path": resolved_path,
        "candidates": candidates,
        "reason": reason,
    })
}

/// Emit one supersession edge whose far end is an `Unresolved` entity.
///
/// The endpoint the unresolved entity takes is the one the document named it for. A
/// `**Superseded by:** ADR-9999` says "ADR-9999 replaces me"; recording it the other way round
/// would be a claim the document did not make, and the relation is stored one way only.
fn claim_unresolved_supersession(
    builder: &mut GraphBuilder,
    project_id: &str,
    rel_path: &str,
    content_hash: &str,
    document_entity: &str,
    site: &SupersessionSite,
    reason: &str,
) {
    let entity_id = ids::unresolved_id(
        project_id,
        rel_path,
        UnresolvedCategory::DocumentSupersedes,
        &site.raw_target,
    );
    builder.add_entity(EntityRecord {
        entity_id: entity_id.clone(),
        kind: EntityKind::Unresolved,
        // Named by the field value exactly as written, which is the one rule that covers a bare
        // identifier, a Markdown link and an empty field alike.
        name: site.raw_target.clone(),
        scope_path: rel_path.to_string(),
        language: None,
        meta: Some(
            serde_json::json!({
                "category": UnresolvedCategory::DocumentSupersedes.as_str(),
                "importer": rel_path,
                "raw_specifier": site.raw_target,
                "field": site.direction.as_str(),
                "reason": reason,
            })
            .to_string(),
        ),
    });
    builder.add_occurrence(&entity_id, rel_path, site.span, content_hash);
    let (source, target) = match site.direction {
        SupersessionDirection::Supersedes => (document_entity, entity_id.as_str()),
        SupersessionDirection::SupersededBy => (entity_id.as_str(), document_entity),
    };
    builder.claim(
        source,
        Relation::Supersedes,
        target,
        // Nothing was resolved. All the document states is a field value, so `DIRECT`.
        Evidence::DOCUMENT,
        rel_path,
        site.span,
        content_hash,
        supersession_details(site, None, Some(reason)),
    );
}

/// Turn scanned documents into entities, occurrences, assertions and observations.
///
/// A separate batch under a separate extractor run, exactly as `ts-js-reference` is. It emits:
///
/// - the document's `File` entity and the `CONTAINS` edge from its directory,
/// - `File CONTAINS Document`,
/// - `Document CONTAINS Section` for top-level headings,
/// - `Section CONTAINS Section` for nested ones,
/// - `Section REFERENCES File` for a destination that names an indexed file, and
///   `Section REFERENCES <symbol>` in addition when a `#L<n>` anchor resolves,
/// - `Section REFERENCES Unresolved` for a destination that names nothing here.
///
/// The source of a link edge is the **innermost section containing it**, and the document itself
/// for a link written before the first heading. Attributing every link to the document would
/// throw away the only thing that says *where* in a long file the claim was made.
///
/// Three shapes deliberately emit nothing:
///
/// - an **external** destination (any URI scheme, or protocol-relative `//host/…`). It is a
///   legitimate reference to something outside the repository; nothing failed, and it is
///   counted, never fetched and never entity-ised.
/// - a bare `#fragment`, which names a heading. Heading-anchor resolution is not modelled.
/// - a **bare code-span mention** — `` `parseConfig` `` in prose. Turning that into an edge is
///   name matching, which ADR-0002 refuses as a basis for identity.
///
/// Slice 5d-ii adds `Document SUPERSEDES Document`, from **explicit evidence only**: one of the
/// four supersession fields, resolved to exactly one indexed document. A `Superseded` status with
/// no target, adjacency of ADR numbers, similarity of subject and prose containing the word are
/// none of them evidence, and none of them produce an edge.
///
/// Every observation is `DOCUMENT_STATED`, including the two filesystem-shaped edges and every
/// resolved link. That is what makes THREAT-MODEL.md T7 a property of the *file* rather than of
/// each emission site, and therefore checkable by one query over the whole `observation` table.
/// Only `directness` distinguishes a resolved claim from a stated one.
///
/// Heading text and link destinations become entity names and byte spans. They are never
/// interpreted, rendered, followed or executed. `nerve_core::ids::section_id` strips control
/// characters before heading text can reach an identity tuple.
fn build_document_graph(project_id: &str, targets: &[DocumentTarget<'_>]) -> GraphBatch {
    let mut builder = GraphBuilder::new(docs::EXTRACTOR_ID, docs::EXTRACTOR_VERSION);

    for target in targets {
        let DocumentTarget {
            file,
            extraction,
            links: resolved,
            supersessions,
        } = *target;
        let rel_path = file.rel_path.as_str();
        let hash = file.content_hash.as_str();
        let name = last_segment(rel_path);
        let file_entity = ids::file_id(project_id, rel_path);
        let document_entity = extraction.entity_id.as_str();

        // The `File` entity and `<directory> CONTAINS <file>` are **not** emitted here. Until
        // Slice 5d-i they were, so that T7 stayed a total function of the source path; they are
        // now `fs-structural` / `FILESYSTEM_OBSERVED`, because "`docs/` contains `ROADMAP.md`" is
        // a filesystem fact whoever asks. T7 stayed total by widening the allowed set to exactly
        // two values keyed on extractor id — see the module header. The occurrence stays: its
        // span is the document's line count, which only reading the file produces.
        builder.add_occurrence(&file_entity, rel_path, extraction.file_span, hash);

        let adr = &extraction.adr;
        builder.add_entity(EntityRecord {
            entity_id: document_entity.to_string(),
            kind: EntityKind::Document,
            name: file_stem(name).to_string(),
            scope_path: rel_path.to_string(),
            language: Some(MARKDOWN_LANGUAGE.to_string()),
            meta: Some(
                serde_json::json!({
                    "adr": adr.is_adr,
                    "adr_id": adr.id,
                    "status": adr.status_value(),
                    "sections": extraction.sections.len(),
                    "has_front_matter": extraction.front_matter.is_some(),
                    "unsupported": extraction.unsupported,
                    // Link outcomes are counted rather than left to be inferred from the graph,
                    // because three of them produce no row at all: an external destination, a
                    // bare fragment, and a code-span mention would otherwise be
                    // indistinguishable from a document Nerve never looked at.
                    "links": resolved.outcomes,
                    // Counted for the same reason `links` is, and with one more: an external
                    // supersession target produces no row at all, so without the count it would
                    // be indistinguishable from a document that named nothing.
                    "supersession": supersessions.outcomes,
                    "code_span_mentions": extraction.code_span_mentions,
                })
                .to_string(),
            ),
        });
        builder.add_occurrence(document_entity, rel_path, extraction.file_span, hash);
        builder.claim(
            &file_entity,
            Relation::Contains,
            document_entity,
            Evidence::DOCUMENT,
            rel_path,
            extraction.file_span,
            hash,
            serde_json::json!({
                "child_kind": "document",
                "adr": adr.is_adr,
                "adr_id": adr.id,
                "status": adr.status_value(),
                // The exact place the status was read from, so the claim carries its citation.
                "status_line": adr.status_line,
                "status_form": adr.status_form,
                // Preserved only when the value is outside the closed vocabulary, and only when
                // it is short enough to be a status word at all. Never coerced.
                "status_raw": adr.status_raw,
                "status_refused_as_too_long": adr.status_refused_as_too_long,
                "unsupported": extraction.unsupported,
            }),
        );

        for section in &extraction.sections {
            builder.add_entity(EntityRecord {
                entity_id: section.entity_id.clone(),
                kind: EntityKind::Section,
                name: section.name.clone(),
                scope_path: rel_path.to_string(),
                language: Some(MARKDOWN_LANGUAGE.to_string()),
                meta: Some(
                    serde_json::json!({
                        "level": section.level,
                        "heading_path": section.heading_path,
                        "sibling_ordinal": section.sibling_ordinal,
                        "style": section.style.as_str(),
                    })
                    .to_string(),
                ),
            });
            builder.add_occurrence(&section.entity_id, rel_path, section.section_span, hash);

            let (container, child_kind) = match &section.parent {
                Some(parent_section) => (parent_section.clone(), "section"),
                None => (document_entity.to_string(), "document"),
            };
            builder.claim(
                &container,
                Relation::Contains,
                &section.entity_id,
                Evidence::DOCUMENT,
                rel_path,
                // The heading line is the evidence; the section span is where the entity is.
                section.heading_span,
                hash,
                serde_json::json!({
                    "child_kind": "section",
                    "container_kind": child_kind,
                    "level": section.level,
                    "heading_path": section.heading_path,
                    "sibling_ordinal": section.sibling_ordinal,
                    "style": section.style.as_str(),
                }),
            );
        }

        for site in &resolved.sites {
            // The innermost enclosing section, and the document itself for a link written before
            // the first heading. Both are entities this batch has already emitted.
            let (source_entity, source_kind) = match extraction.owning_section(site.span.start_byte)
            {
                Some(section) => (section.entity_id.as_str(), "section"),
                None => (document_entity, "document"),
            };

            match &site.outcome {
                // Counted in the document's `links` meta and emitted nowhere. Nothing failed,
                // and nothing in this repository was named.
                LinkOutcome::External { .. } | LinkOutcome::FragmentOnly => {}

                // The reason is read back off the outcome rather than restated here, so the
                // closed vocabulary has one definition and cannot drift into two.
                LinkOutcome::Refused | LinkOutcome::NotIndexed { .. } => {
                    let Some(reason) = site.outcome.unresolved_reason() else {
                        continue;
                    };
                    claim_unresolved_document_link(
                        &mut builder,
                        project_id,
                        rel_path,
                        hash,
                        source_entity,
                        source_kind,
                        site,
                        reason,
                        match &site.outcome {
                            // The path the destination normalized to. It named nothing, and
                            // saying *which* path named nothing is what makes the refusal
                            // actionable. A refused destination normalized to no path at all.
                            LinkOutcome::NotIndexed { path } => Some(path.as_str()),
                            _ => None,
                        },
                    );
                }

                LinkOutcome::File { path, anchor } => {
                    // The file edge, whatever the fragment did. A `#L900` that resolves to no
                    // symbol does not stop the document from referencing the file.
                    builder.claim(
                        source_entity,
                        Relation::References,
                        &ids::file_id(project_id, path),
                        Evidence::DOCUMENT_RESOLVED,
                        rel_path,
                        site.span,
                        hash,
                        link_details(
                            site,
                            source_kind,
                            link_target::FILE,
                            Some(path.as_str()),
                            None,
                            None,
                        ),
                    );
                    match anchor {
                        None => {}
                        Some(AnchorOutcome::Symbol {
                            entity_id,
                            target_content_hash,
                            ..
                        }) => builder.claim(
                            source_entity,
                            Relation::References,
                            entity_id,
                            Evidence::DOCUMENT_RESOLVED,
                            rel_path,
                            site.span,
                            hash,
                            link_details(
                                site,
                                source_kind,
                                link_target::SYMBOL,
                                Some(path.as_str()),
                                Some(target_content_hash.as_str()),
                                None,
                            ),
                        ),
                        // The file resolved and the anchor did not, so the site produces both a
                        // resolved edge and an unresolved one. Collapsing them would either
                        // hide the working half of the link or overstate the broken half.
                        Some(AnchorOutcome::NoSymbol { .. }) => {
                            if let Some(reason) = site.outcome.unresolved_reason() {
                                claim_unresolved_document_link(
                                    &mut builder,
                                    project_id,
                                    rel_path,
                                    hash,
                                    source_entity,
                                    source_kind,
                                    site,
                                    reason,
                                    Some(path.as_str()),
                                );
                            }
                        }
                    }
                }
            }
        }

        // ---- supersession ----------------------------------------------------------------
        //
        // The subject is always the **document**, never the section the field sits in: a
        // decision is superseded, not a heading. The two directions produce the same edge with
        // the endpoints swapped, so `A SUPERSEDES B` always means A replaced B, and asking "what
        // replaced this?" is a reverse traversal rather than a second row.
        for site in &supersessions.sites {
            match &site.outcome {
                // Counted in the document's `supersession` meta and emitted nowhere. Nothing
                // failed, and nothing in this repository was named. Never fetched.
                SupersessionOutcome::External { .. } => {}

                SupersessionOutcome::Document { path } => {
                    let target_document = ids::document_id(project_id, path);
                    let (source, target) = match site.direction {
                        SupersessionDirection::Supersedes => {
                            (document_entity, target_document.as_str())
                        }
                        SupersessionDirection::SupersededBy => {
                            (target_document.as_str(), document_entity)
                        }
                    };
                    builder.claim(
                        source,
                        Relation::Supersedes,
                        target,
                        Evidence::DOCUMENT_RESOLVED,
                        rel_path,
                        site.span,
                        hash,
                        supersession_details(site, Some(path.as_str()), None),
                    );
                }

                // The reason is read back off the outcome rather than restated here, so the
                // closed vocabulary has one definition and cannot drift into two.
                other => {
                    let Some(reason) = other.unresolved_reason() else {
                        continue;
                    };
                    claim_unresolved_supersession(
                        &mut builder,
                        project_id,
                        rel_path,
                        hash,
                        document_entity,
                        site,
                        reason,
                    );
                }
            }
        }
    }

    builder.batch
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::extract::ImportForm;

    #[test]
    fn path_helpers() {
        assert_eq!(parent_directory("a/b/c.ts"), Some("a/b".to_string()));
        assert_eq!(parent_directory("c.ts"), None);
        assert_eq!(last_segment("a/b/c.ts"), "c.ts");
        assert_eq!(last_segment("c.ts"), "c.ts");
        assert_eq!(file_stem("c.ts"), "c");
        assert_eq!(file_stem(".env"), ".env");
        assert_eq!(file_stem("index.d.ts"), "index.d");
    }

    #[test]
    fn import_form_tags_are_stable() {
        assert_eq!(ImportForm::Static.as_str(), "static");
        assert_eq!(ImportForm::ReExport.as_str(), "re-export");
        assert_eq!(ImportForm::Require.as_str(), "require");
        assert_eq!(ImportForm::DynamicLiteral.as_str(), "dynamic-literal");
    }

    fn edge_set(pairs: &[(&str, &str)]) -> BTreeSet<(String, String)> {
        pairs
            .iter()
            .map(|(from, to)| ((*from).to_string(), (*to).to_string()))
            .collect()
    }

    /// A cycle is counted, and an acyclic graph produces no false one. Both directions matter:
    /// under-reporting hides a contradiction, over-reporting turns an ordinary chain into an
    /// alarm.
    #[test]
    fn a_supersession_cycle_is_counted_and_a_chain_is_not() {
        // The fixture's shape: a three-document cycle beside a two-hop chain that touches it.
        assert_eq!(
            supersession_cycles(&edge_set(&[("a", "b"), ("b", "c"), ("c", "a")])),
            (1, 3)
        );
        assert_eq!(
            supersession_cycles(&edge_set(&[("x", "y"), ("y", "z")])),
            (0, 0)
        );
        assert_eq!(supersession_cycles(&edge_set(&[])), (0, 0));

        // Two disjoint cycles joined by one edge are two groups, not one. A weakly connected
        // component would say one, and would be wrong.
        assert_eq!(
            supersession_cycles(&edge_set(&[
                ("a", "b"),
                ("b", "a"),
                ("b", "c"),
                ("c", "d"),
                ("d", "c"),
            ])),
            (2, 4)
        );
    }

    /// A long chain must not overflow the stack. The walk is iterative for exactly this reason:
    /// the graph is built out of repository content, and repository content is hostile.
    #[test]
    fn a_very_long_supersession_chain_does_not_overflow() {
        let paths: Vec<String> = (0..50_000).map(|index| format!("d{index}.md")).collect();
        let edges: BTreeSet<(String, String)> = paths
            .windows(2)
            .map(|pair| (pair[0].clone(), pair[1].clone()))
            .collect();
        assert_eq!(supersession_cycles(&edges), (0, 0));
    }

    #[test]
    fn a_contradiction_is_a_string_comparison_over_the_targets_own_status() {
        let facts = |status: Option<&str>| ModuleFacts {
            document: facts::DocumentCounters {
                adr_status: status.map(str::to_string),
                ..facts::DocumentCounters::default()
            },
            ..ModuleFacts::default()
        };
        let by_path: BTreeMap<String, ModuleFacts> = [
            ("a.md".to_string(), facts(Some("Accepted"))),
            ("b.md".to_string(), facts(Some("Superseded"))),
            ("c.md".to_string(), facts(None)),
        ]
        .into_iter()
        .collect();

        // Counted per document, not per edge: `a.md` is superseded twice and contradicts itself
        // once. `b.md` says Superseded and agrees; `c.md` is not an ADR and states no status.
        let edges = edge_set(&[
            ("x.md", "a.md"),
            ("y.md", "a.md"),
            ("z.md", "b.md"),
            ("w.md", "c.md"),
        ]);
        assert_eq!(supersession_contradictions(&edges, &by_path), 1);
        assert_eq!(supersession_contradictions(&edge_set(&[]), &by_path), 0);
    }

    fn fs_entry(rel_path: &str, kind: FileKind) -> FsEntry {
        FsEntry {
            rel_path: rel_path.to_string(),
            kind,
            size_bytes: 1,
            content_hash: "h".to_string(),
        }
    }

    /// **`fs-structural` never reads file bytes, proved by construction.**
    ///
    /// There is no repository here, no temporary directory, no file on disk and no source text
    /// anywhere in this test — and the whole filesystem skeleton comes out anyway. That is the
    /// proof: the builder's only input is `&[FsEntry]`, [`FsEntry`] has no field that can hold
    /// file content, and a function that produces its complete output from metadata alone did not
    /// consult anything else. The empirical companion —
    /// `nerve-index/tests/documents.rs::fs_structural_carries_no_file_content` — indexes a real
    /// tree and checks no document prose reaches any `fs-structural` column.
    ///
    /// This is what the amended THREAT-MODEL.md **T7** rests on: the filesystem extractor may
    /// emit on document paths because it cannot carry document content.
    #[test]
    fn fs_graph_needs_no_file_on_disk() {
        let entries = vec![
            fs_entry("README.md", FileKind::Doc),
            fs_entry("docs/decisions/ADR-0001.md", FileKind::Doc),
            fs_entry("src/math.ts", FileKind::Code(Language::TypeScript)),
        ];
        let targets: BTreeSet<String> =
            entries.iter().map(|entry| entry.rel_path.clone()).collect();

        let batch = build_fs_graph("p", &entries, &targets);

        // Every observation is the declared source type, and the declaration is enforced by the
        // same call the pipeline makes.
        batch
            .verify_declared_source_types(fsstruct::EXTRACTOR_ID, &fsstruct::DECLARED_SOURCE_TYPES)
            .expect("fs-structural emitted a source type it does not declare");
        assert!(!batch.observations.is_empty());
        for observation in &batch.observations {
            assert_eq!(
                observation.evidence_source_type,
                EvidenceSourceType::FilesystemObserved
            );
            assert_eq!(observation.directness, Directness::Direct);
            assert_eq!(observation.extractor_id, fsstruct::EXTRACTOR_ID);
        }

        // The repository, both intermediate directories, and one entity per file.
        let kinds: Vec<EntityKind> = batch.entities.iter().map(|entity| entity.kind).collect();
        assert_eq!(
            kinds
                .iter()
                .filter(|k| **k == EntityKind::Repository)
                .count(),
            1
        );
        assert_eq!(
            kinds
                .iter()
                .filter(|k| **k == EntityKind::Directory)
                .count(),
            3,
            "docs, docs/decisions, src"
        );
        assert_eq!(kinds.iter().filter(|k| **k == EntityKind::File).count(), 3);

        // Three directory-containment edges and three file-containment edges, and nothing else.
        assert!(batch
            .assertions
            .iter()
            .all(|assertion| assertion.relation == Relation::Contains));
        assert_eq!(batch.assertions.len(), 6);

        // No occurrence: a whole-file span's end line is the file's line count, which only
        // reading the file produces. It stays with the extractor that read it.
        assert!(batch.occurrences.is_empty());
    }

    /// A file outside the re-extraction set contributes no file-scoped row, while the directory
    /// skeleton is still re-derived in full. That is what keeps a run proportional to the change.
    #[test]
    fn fs_graph_emits_file_rows_only_for_targets() {
        let entries = vec![
            fs_entry("src/changed.ts", FileKind::Code(Language::TypeScript)),
            fs_entry("src/untouched.ts", FileKind::Code(Language::TypeScript)),
        ];
        let mut targets = BTreeSet::new();
        targets.insert("src/changed.ts".to_string());

        let batch = build_fs_graph("p", &entries, &targets);

        assert_eq!(
            batch
                .entities
                .iter()
                .filter(|e| e.kind == EntityKind::File)
                .count(),
            1
        );
        assert_eq!(
            batch
                .entities
                .iter()
                .filter(|e| e.kind == EntityKind::Directory)
                .count(),
            1,
            "the skeleton is re-derived from the whole tree, not from the targets"
        );
        assert_eq!(batch.assertions.len(), 2, "repo/src, and src/changed.ts");
    }
}
