//! `nerve index`: discover, read, parse, extract, persist, derive.
//!
//! Parsing is serial. That is a determinism decision, not an oversight: the canonical dump must
//! be byte-identical across runs, and an ordered parallel merge is Slice 3 work.
//!
//! Two extractors run per index, each with its own `extractor_run` row and its own batch
//! verified against its own declared source types. Keeping them separate is what makes a
//! `DELETE FROM observation WHERE extractor_id = 'ts-js-reference'` a complete retraction of
//! everything resolution claimed, with the structural graph untouched.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::time::Instant;

use nerve_core::ids;
use nerve_core::model::{
    AssertionRecord, EntityRecord, GraphBatch, ObservationRecord, OccurrenceRecord, Span,
};
use nerve_core::vocab::{Directness, EntityKind, EvidenceSourceType, Relation, UnresolvedCategory};

use crate::config::{self, Config};
use crate::discover;
use crate::error::{IndexError, Result};
use crate::exports::ExportIndex;
use crate::extract::{
    self, ExportTarget, ModuleExtraction, DECLARED_SOURCE_TYPES, EXTRACTOR_ID, EXTRACTOR_VERSION,
};
use crate::gitinfo;
use crate::lang::Language;
use crate::refs::{self, RefTarget, ReferenceExtraction};
use crate::resolve;

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
    /// Entity counts by kind, over the whole database.
    pub entities_by_kind: BTreeMap<String, i64>,
    /// Assertion counts by relation, over the whole database.
    pub assertions_by_relation: BTreeMap<String, i64>,
    /// Total entities.
    pub entities_total: i64,
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
}

struct LoadedFile {
    rel_path: String,
    language: Language,
    source: String,
    content_hash: String,
    size_bytes: u64,
}

/// Accumulates the graph for one run, deduplicating by content-derived key as it goes.
struct GraphBuilder {
    state_id: String,
    extractor_id: &'static str,
    extractor_version: &'static str,
    batch: GraphBatch,
    seen_entities: BTreeSet<String>,
    seen_assertions: BTreeSet<String>,
}

impl GraphBuilder {
    fn new(state_id: &str, extractor_id: &'static str, extractor_version: &'static str) -> Self {
        GraphBuilder {
            state_id: state_id.to_string(),
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
            occurrence_id: ids::occurrence_id(
                entity_id,
                &self.state_id,
                file_path,
                span.start_byte,
                span.end_byte,
            ),
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

    /// Module, export or binding resolution produced this.
    const RESOLVED: Evidence = Evidence {
        source_type: EvidenceSourceType::AstResolved,
        directness: Directness::Resolved,
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

/// Index a repository. One transaction; `assertion_state` rebuilt afterwards.
pub fn index_repository(root: &Path) -> Result<IndexOutcome> {
    let started = Instant::now();
    let root = discover::canonical_root(root)?;
    let db_path = config::db_path(&root);
    if !db_path.exists() {
        return Err(IndexError::NotInitialized(root));
    }
    let config = Config::load(&root)?;
    let discovery = discover::discover(&root, &config)?;

    // ---- read and hash -------------------------------------------------------------------
    let mut loaded: Vec<LoadedFile> = Vec::new();
    let mut files_failed = 0usize;
    for file in &discovery.files {
        let Ok(metadata) = std::fs::metadata(&file.abs_path) else {
            files_failed += 1;
            continue;
        };
        if metadata.len() > config.index.max_file_bytes {
            files_failed += 1;
            continue;
        }
        let Ok(bytes) = std::fs::read(&file.abs_path) else {
            files_failed += 1;
            continue;
        };
        let content_hash = ids::content_hash(&bytes);
        let Ok(source) = String::from_utf8(bytes) else {
            files_failed += 1;
            continue;
        };
        loaded.push(LoadedFile {
            rel_path: file.rel_path.clone(),
            language: file.language,
            source,
            content_hash,
            size_bytes: metadata.len(),
        });
    }
    loaded.sort_by(|a, b| a.rel_path.cmp(&b.rel_path));

    // ---- repository state ----------------------------------------------------------------
    let mut pairs: Vec<(String, String)> = loaded
        .iter()
        .map(|file| (file.rel_path.clone(), file.content_hash.clone()))
        .collect();
    let content_merkle = ids::content_merkle(&mut pairs);
    let state_id = content_merkle.clone();
    let git_commit = gitinfo::head_commit(&root);

    // ---- extract -------------------------------------------------------------------------
    let mut extractions: Vec<ModuleExtraction> = Vec::with_capacity(loaded.len());
    for file in &loaded {
        extractions.push(extract::extract_module(
            &config.project_id,
            &file.rel_path,
            file.language,
            &file.source,
        )?);
    }

    let files_with_syntax_errors = extractions.iter().filter(|e| e.has_syntax_error).count();
    let dynamic_imports_without_specifier: usize = extractions
        .iter()
        .map(|e| e.dynamic_imports_without_specifier)
        .sum();

    // The export closure spans every module, so it is built once from the whole corpus and
    // then shared: a barrel chain in one file can reach a declaration in any other.
    let indexed: BTreeSet<String> = loaded.iter().map(|file| file.rel_path.clone()).collect();
    let export_index = ExportIndex::build(&extractions, &indexed);

    let mut reference_extractions: Vec<ReferenceExtraction> = Vec::with_capacity(loaded.len());
    for (file, extraction) in loaded.iter().zip(extractions.iter()) {
        reference_extractions.push(refs::extract_references(
            &config.project_id,
            &file.rel_path,
            file.language,
            &file.source,
            extraction,
            &export_index,
            &indexed,
        )?);
    }

    let unmodelled_call_sites: usize = reference_extractions
        .iter()
        .map(|extraction| extraction.unmodelled_call_sites)
        .sum();
    let mut unmodelled_by_form: BTreeMap<String, usize> = BTreeMap::new();
    for extraction in &reference_extractions {
        for (form, count) in &extraction.unmodelled_by_form {
            *unmodelled_by_form.entry(form.clone()).or_insert(0) += count;
        }
    }

    // ---- build ---------------------------------------------------------------------------
    let batch = build_graph(&config.project_id, &state_id, &loaded, &extractions)?;
    batch.verify_declared_source_types(EXTRACTOR_ID, &DECLARED_SOURCE_TYPES)?;

    let reference_batch = build_reference_graph(
        &config.project_id,
        &state_id,
        &loaded,
        &reference_extractions,
    );
    reference_batch
        .verify_declared_source_types(refs::EXTRACTOR_ID, &refs::DECLARED_SOURCE_TYPES)?;

    // ---- persist -------------------------------------------------------------------------
    let repo_id = ids::repository_id(&config.project_id);
    let status = if files_failed > 0 {
        RunStatus::Partial
    } else {
        RunStatus::Complete
    };

    let mut conn = nerve_store::open(&db_path)?;
    {
        let tx = conn.transaction().map_err(nerve_store::StoreError::from)?;
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
        // One `extractor_run` row per extractor. The rows are what make a contribution
        // attributable, and therefore revocable, per extractor and version.
        let structural_run = nerve_store::begin_extractor_run(
            &tx,
            &repo_id,
            &state_id,
            EXTRACTOR_ID,
            EXTRACTOR_VERSION,
        )?;
        nerve_store::persist_batch(&tx, &repo_id, &state_id, structural_run, &batch)?;
        nerve_store::finish_extractor_run(
            &tx,
            structural_run,
            loaded.len() as i64,
            files_failed as i64,
            status.as_str(),
        )?;

        let reference_run = nerve_store::begin_extractor_run(
            &tx,
            &repo_id,
            &state_id,
            refs::EXTRACTOR_ID,
            refs::EXTRACTOR_VERSION,
        )?;
        nerve_store::persist_batch(&tx, &repo_id, &state_id, reference_run, &reference_batch)?;
        nerve_store::finish_extractor_run(
            &tx,
            reference_run,
            loaded.len() as i64,
            files_failed as i64,
            status.as_str(),
        )?;

        // Derivation runs inside the same transaction: the graph and the state derived from it
        // become visible together or not at all.
        nerve_store::rebuild_assertion_state(&tx)?;
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
        entities_by_kind: report.entities_by_kind,
        assertions_by_relation: report.assertions_by_relation,
        entities_total: report.entities_total,
        assertions_total: report.assertions_total,
        observations_total: report.observations_total,
        unresolved_entities: report.unresolved_entities,
        unresolved_assertions: report.unresolved_assertions,
        duration_ms: started.elapsed().as_millis(),
        status,
    })
}

/// Turn per-module extractions into entities, occurrences, assertions and observations.
fn build_graph(
    project_id: &str,
    state_id: &str,
    loaded: &[LoadedFile],
    extractions: &[ModuleExtraction],
) -> Result<GraphBatch> {
    let mut builder = GraphBuilder::new(state_id, EXTRACTOR_ID, EXTRACTOR_VERSION);

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

    let indexed: BTreeSet<String> = loaded.iter().map(|f| f.rel_path.clone()).collect();

    // Directories that actually contain an indexed file.
    let mut directories: BTreeSet<String> = BTreeSet::new();
    for file in loaded {
        let mut ancestor = parent_directory(&file.rel_path);
        while let Some(directory) = ancestor {
            ancestor = parent_directory(&directory);
            directories.insert(directory);
        }
    }

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
            Evidence::DIRECT,
            directory,
            Span::NONE,
            &directory_hash(directory),
            serde_json::json!({ "child_kind": "directory" }),
        );
    }

    // Exported name -> entity id, per module. Built before the graph so that re-exports can be
    // resolved without depending on file order.
    let mut module_exports: BTreeMap<String, BTreeMap<String, String>> = BTreeMap::new();
    for extraction in extractions {
        let mut exports = BTreeMap::new();
        for export in &extraction.local_exports {
            let entity_id = match &export.target {
                ExportTarget::Symbol(index) => Some(extraction.symbols[*index].entity_id.clone()),
                ExportTarget::LocalName(name) => extraction
                    .top_level_symbol(name)
                    .map(|index| extraction.symbols[index].entity_id.clone()),
            };
            if let Some(entity_id) = entity_id {
                exports.insert(export.exported_name.clone(), entity_id);
            }
        }
        module_exports.insert(extraction.rel_path.clone(), exports);
    }

    for (file, extraction) in loaded.iter().zip(extractions.iter()) {
        let rel_path = file.rel_path.as_str();
        let hash = file.content_hash.as_str();
        let file_entity = ids::file_id(project_id, rel_path);
        let module_entity = ids::module_id(project_id, rel_path);
        let parent = parent_directory(rel_path);
        let name = last_segment(rel_path);

        builder.add_entity(EntityRecord {
            entity_id: file_entity.clone(),
            kind: EntityKind::File,
            name: name.to_string(),
            scope_path: parent.clone().unwrap_or_default(),
            language: Some(file.language.as_str().to_string()),
            meta: Some(
                serde_json::json!({
                    "extension": name.rsplit('.').next().unwrap_or_default(),
                    "size_bytes": file.size_bytes,
                })
                .to_string(),
            ),
        });
        builder.add_occurrence(&file_entity, rel_path, extraction.file_span, hash);

        // Repository or directory CONTAINS this file.
        let parent_id = match &parent {
            Some(parent) => ids::directory_id(project_id, parent),
            None => repo_id.clone(),
        };
        builder.claim(
            &parent_id,
            Relation::Contains,
            &file_entity,
            Evidence::DIRECT,
            rel_path,
            Span::NONE,
            hash,
            serde_json::json!({ "child_kind": "file" }),
        );

        // File DEFINES Module, 1:1 for TS/JS.
        builder.add_entity(EntityRecord {
            entity_id: module_entity.clone(),
            kind: EntityKind::Module,
            name: file_stem(name).to_string(),
            scope_path: rel_path.to_string(),
            language: Some(file.language.as_str().to_string()),
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
            serde_json::json!({ "language": file.language.as_str() }),
        );

        // Symbols.
        for symbol in &extraction.symbols {
            builder.add_entity(EntityRecord {
                entity_id: symbol.entity_id.clone(),
                kind: symbol.kind,
                name: symbol.name.clone(),
                scope_path: symbol.scope_path.clone(),
                language: Some(file.language.as_str().to_string()),
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
            let Some(target_module) =
                resolve::resolve(rel_path, &re_export.raw_specifier, &indexed)
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
            let resolved = resolve::resolve(rel_path, &import.raw_specifier, &indexed);
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

/// Turn resolved reference sites into assertions and observations.
///
/// This runs as a **separate batch under a separate extractor run**. It creates no symbol
/// entities — every resolved target already exists in the structural batch — and only the
/// `Unresolved` entities it names itself.
fn build_reference_graph(
    project_id: &str,
    state_id: &str,
    loaded: &[LoadedFile],
    extractions: &[ReferenceExtraction],
) -> GraphBatch {
    let mut builder = GraphBuilder::new(state_id, refs::EXTRACTOR_ID, refs::EXTRACTOR_VERSION);

    for (file, extraction) in loaded.iter().zip(extractions.iter()) {
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
}
