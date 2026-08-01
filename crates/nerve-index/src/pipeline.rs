//! `nerve index`: discover, read, parse, extract, persist, derive.
//!
//! Parsing is serial. That is a determinism decision, not an oversight: the canonical dump must
//! be byte-identical across runs, and an ordered parallel merge is a later slice — introducing
//! it alongside deletion and invalidation would confound two new causes of divergence.
//!
//! Two extractors run per index, each with its own `extractor_run` row and its own batch
//! verified against its own declared source types. Keeping them separate is what makes a
//! `DELETE FROM observation WHERE extractor_id = 'ts-js-reference'` a complete retraction of
//! everything resolution claimed, with the structural graph untouched.
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
//! 2. **Surviving rows are advanced to the new repository state.** An observation records the
//!    state it was made in; carrying one forward without restating it would leave the database
//!    describing two states at once. Restating is sound precisely because the file's bytes are
//!    unchanged and the extractors are deterministic, so re-running them would have produced the
//!    identical row.
//! 3. **Entities are upserted, not ignored.** An entity id excludes body content by design, so
//!    an edit can change a row without changing its id.
//!
//! `--full` is the same code path with every file seeded, so the full and incremental paths
//! cannot drift apart by being two implementations.

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
use crate::facts::{self, ModuleFacts};
use crate::gitinfo;
use crate::incremental::{self, MoveCandidate, PreviousModule};
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
    /// Occurrences advanced to the new repository state.
    pub occurrences_restated: usize,
    /// Observations advanced to the new repository state.
    pub observations_restated: usize,
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
    /// What was re-extracted, skipped and removed.
    pub incremental: IncrementalReport,
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

    let repo_id = ids::repository_id(&config.project_id);
    let mut conn = nerve_store::open(&db_path)?;

    // ---- what changed --------------------------------------------------------------------
    let previous = load_previous_modules(&conn, &repo_id)?;
    let current_hashes: BTreeMap<String, String> = loaded
        .iter()
        .map(|file| (file.rel_path.clone(), file.content_hash.clone()))
        .collect();
    let changes = incremental::classify(&previous, &current_hashes, options.full);

    let indexed: BTreeSet<String> = current_hashes.keys().cloned().collect();
    let mut known_paths = indexed.clone();
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
    let mut extractions: BTreeMap<String, ModuleExtraction> = BTreeMap::new();
    for file in &loaded {
        if !target_paths.contains(&file.rel_path) {
            continue;
        }
        extractions.insert(
            file.rel_path.clone(),
            extract::extract_module(
                &config.project_id,
                &file.rel_path,
                file.language,
                &file.source,
            )?,
        );
    }

    // The export closure spans every module, so it is built from the whole corpus: freshly
    // parsed modules for the invalidation set, and modules reconstructed from the cache for
    // everything else. A barrel chain in one file can reach a declaration in any other, and
    // that has to keep working when the other file was not re-parsed.
    let export_sources: Vec<ModuleExtraction> = loaded
        .iter()
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
        reference_extractions.insert(
            file.rel_path.clone(),
            refs::extract_references(
                &config.project_id,
                &file.rel_path,
                file.language,
                &file.source,
                extraction,
                &export_index,
                &indexed,
            )?,
        );
    }

    // ---- whole-repository view, from fresh work plus cache -------------------------------
    //
    // Reported totals describe the repository, not the fraction of it this run touched, so the
    // per-file tallies of skipped files come from the cache.
    let mut facts_by_path: BTreeMap<String, ModuleFacts> = BTreeMap::new();
    for file in &loaded {
        let facts = match (
            extractions.get(&file.rel_path),
            reference_extractions.get(&file.rel_path),
        ) {
            (Some(extraction), Some(references)) => {
                ModuleFacts::from_extraction(extraction, references, &file.source)
            }
            _ => cached_facts(&previous, &file.rel_path),
        };
        facts_by_path.insert(file.rel_path.clone(), facts);
    }

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

    let batch = build_graph(
        &config.project_id,
        &state_id,
        &loaded,
        &targets,
        &module_exports,
        &indexed,
    )?;
    batch.verify_declared_source_types(EXTRACTOR_ID, &DECLARED_SOURCE_TYPES)?;

    let reference_targets: Vec<(&LoadedFile, &ReferenceExtraction)> = loaded
        .iter()
        .filter_map(|file| {
            reference_extractions
                .get(&file.rel_path)
                .map(|extraction| (file, extraction))
        })
        .collect();
    let reference_batch = build_reference_graph(&config.project_id, &state_id, &reference_targets);
    reference_batch
        .verify_declared_source_types(refs::EXTRACTOR_ID, &refs::DECLARED_SOURCE_TYPES)?;

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

        // 1. Withdraw the evidence this run is about to replace or has lost. Entities and
        //    assertions are left standing until it is known what the new rows support.
        let mut superseded: BTreeSet<String> = target_paths.clone();
        superseded.extend(changes.removed.iter().cloned());
        let mut removals = nerve_store::delete_file_rows(&tx, &superseded)?;
        // Directory containment is re-derived from the current file set every run, so clearing
        // it is not a removal and is deliberately not counted as one. A directory that really
        // did go away shows up in the reported counts as a pruned assertion and entity.
        nerve_store::delete_directory_containment(&tx)?;

        // 2. Advance everything that survived to the state this run observed.
        let (occurrences_restated, observations_restated) =
            nerve_store::restamp_state(&tx, &state_id)?;

        // 3. Write the new evidence. One `extractor_run` row per extractor: the rows are what
        //    make a contribution attributable, and therefore revocable, per extractor version.
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

        // 4. Derivation runs inside the same transaction: the graph and the state derived from
        //    it become visible together or not at all. It runs *before* orphan pruning, which is
        //    what makes the pruning safe — a rebuild leaves no derived row behind for an
        //    assertion nothing observes, so deleting that assertion breaks no foreign key.
        nerve_store::rebuild_assertion_state(&tx)?;

        // 5. Remove what nothing supports any more.
        removals.add(nerve_store::prune_orphans(&tx)?);

        // 6. Refresh the extraction cache.
        for path in &changes.removed {
            nerve_store::delete_module_facts(&tx, &repo_id, path)?;
        }
        for file in &loaded {
            if !target_paths.contains(&file.rel_path) {
                continue;
            }
            let facts = facts_by_path
                .get(&file.rel_path)
                .cloned()
                .unwrap_or_default();
            nerve_store::upsert_module_facts(
                &tx,
                &repo_id,
                &nerve_store::ModuleFactsRow {
                    rel_path: file.rel_path.clone(),
                    content_hash: file.content_hash.clone(),
                    language: file.language.as_str().to_string(),
                    structural_version: EXTRACTOR_VERSION.to_string(),
                    reference_version: refs::EXTRACTOR_VERSION.to_string(),
                    facts: facts.to_json()?,
                },
            )?;
        }

        // 7. Propose identity links. Proposals only — nothing merges the two identities.
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
        incremental_report.occurrences_restated = occurrences_restated;
        incremental_report.observations_restated = observations_restated;

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
        incremental: incremental_report,
    })
}

/// Read the previous run's per-module cache, deciding for each row whether it can be reused.
///
/// A row is reusable only when its payload parses **and** both extractor versions match this
/// build. A version bump means the extractor's output may differ over identical bytes, which is
/// exactly what the version field exists to say, so every file is re-extracted.
fn load_previous_modules(
    conn: &nerve_store::Connection,
    repo_id: &str,
) -> Result<BTreeMap<String, PreviousModule>> {
    let rows = nerve_store::load_module_facts(conn, repo_id)?;
    let mut previous = BTreeMap::new();
    for (rel_path, row) in rows {
        let facts = ModuleFacts::from_json(&row.facts);
        let reusable = facts.is_some()
            && row.structural_version == EXTRACTOR_VERSION
            && row.reference_version == refs::EXTRACTOR_VERSION;
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

/// Turn per-module extractions into entities, occurrences, assertions and observations.
///
/// `loaded` is the whole tree and `targets` is the subset being re-extracted. The **skeleton** —
/// the repository entity, the directory entities, and the `CONTAINS` edges between directories —
/// is re-derived from `loaded` on every run, because it needs no parsing and because directory
/// containment is the one part of the graph that no file path owns. Everything file-scoped is
/// emitted only for `targets`; a file that was not re-extracted keeps the rows it already has.
///
/// `module_exports` covers **every** module, not just the targets: resolving a re-export in a
/// target file requires the export map of the module it names, which may not have been parsed.
fn build_graph(
    project_id: &str,
    state_id: &str,
    loaded: &[LoadedFile],
    targets: &[(&LoadedFile, &ModuleExtraction)],
    module_exports: &BTreeMap<String, BTreeMap<String, String>>,
    indexed: &BTreeSet<String>,
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

    for (file, extraction) in targets.iter().copied() {
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

/// Turn resolved reference sites into assertions and observations.
///
/// This runs as a **separate batch under a separate extractor run**. It creates no symbol
/// entities — every resolved target already exists in the structural batch — and only the
/// `Unresolved` entities it names itself.
fn build_reference_graph(
    project_id: &str,
    state_id: &str,
    targets: &[(&LoadedFile, &ReferenceExtraction)],
) -> GraphBatch {
    let mut builder = GraphBuilder::new(state_id, refs::EXTRACTOR_ID, refs::EXTRACTOR_VERSION);

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
