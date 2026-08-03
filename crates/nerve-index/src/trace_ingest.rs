//! `nerve trace import`: turn one trace artifact the user names into `TEST_OBSERVED_CALL`
//! observations.
//!
//! [`crate::trace`] wrote the reader; this is the half that touches the world. It reads **one file,
//! the one the user named**, resolves every path in it through the same guards everything else uses,
//! maps every frame onto a symbol with the same function coverage and `#L<n>` document anchors use,
//! and writes observations. **It runs no test and spawns no process** — that is the invariant the
//! whole design exists to keep (`crates/nerve-cli/tests/no_subprocess.rs`), and `nerve trace-tests`
//! does not exist.
//!
//! # What is emitted, and what is deliberately not
//!
//! ```text
//! <caller symbol>  TEST_OBSERVED_CALL  <callee symbol>   TEST_CALL_TRACE / RESOLVED, test-trace 1.0.0
//! ```
//!
//! and nothing else. **The endpoints are the two frames of the call; the test is never an endpoint.**
//! For the stack `test_x → parse → lex`, the second event's source is `parse`. Making `test_x` the
//! source would assert a call `test_x` never made, which is the defect
//! `docs/plans/slice-11a-trace-ingestion.md` §2.1 corrects. Which test observed the edge is
//! provenance and lives in `observation.environment`, which is where ADR-0003 puts provenance and
//! why this slice needs no new `EntityKind`, no schema change and no migration.
//!
//! `RESOLVED`, not `DIRECT` and not `INFERRED`: the artifact states the call outright, so no rule
//! concludes the relation, but it names a file and a line, so the endpoints are resolved. See
//! [`crate::trace::DIRECTNESS`].
//!
//! # Existential, not universal
//!
//! A trace proves *this* run took *this* edge. It proves neither that the edge is always taken nor
//! that unobserved edges do not exist, and `TEST_OBSERVED_CALL` is therefore **not** in
//! [`nerve_store::DEFAULT_IMPACT_RELATIONS`] — see that module's header for the argument. It is
//! reachable explicitly, which is where its value lies.
//!
//! # Repository-state binding is the load-bearing invariant
//!
//! A trace made for state A must never silently become evidence for state B. [`TraceBinding`] is a
//! **three-valued** answer, and `Unverified` is never reported as `Bound`, for the same reason
//! `CoverageEvidence::Absent` and `nerve check`'s `Unverified`-versus-`Stale` split exist: absence of
//! verification is not verification of absence.
//!
//! An artifact naming a different repository is refused **whole**. An artifact whose declared state
//! disagrees with the index is reported `stale` and kept, because it is still about this repository.
//! Independently of either, a record naming a file whose bytes have moved since indexing is refused:
//! the extents a line would map onto describe the *old* file, and recording a claim derived from
//! stale extents and stamping it with the current hash produces a row that says `fresh` and is wrong
//! — the Slice 6b lesson.
//!
//! # Evidence accumulates, and that is why it is read before it is written
//!
//! `docs/plans/slice-11a-trace-ingestion.md` §7 requires that two shards of one run both import, that
//! two runs both import with `run_id` distinguishing them, and that a corrected artifact with a
//! repeated `run_id` is imported with the conflict **reported** rather than silently overwriting
//! anything. Nothing here withdraws a previous artifact's evidence.
//!
//! That collides with the schema, and the collision is real: `idx_observation_identity` keys on
//! `(assertion, extractor, version, source type, path, lines)` and has **no** column for
//! `environment`, so two runs observing one call at one site are one row and `INSERT OR IGNORE` would
//! drop the second run's identity without a word. So each import **reads** what is stored for the
//! sites it touches and restates the union: one observation per `(caller, callee, caller file, caller
//! line)`, whose `environment.runs[]` names every run and every test that reached that site. Two
//! *different* call sites remain two observations, because they are two pieces of evidence.
//!
//! # T9 — the artifact is attacker-controlled input
//!
//! Every recorded path goes through two independent gates in this order: the **shared** traversal
//! refusal from Slice 8b-i ([`nerve_store::selector_shape`]), which is syntactic and refuses `..`
//! spelled with either separator plus absolute paths, and then
//! [`crate::discover::canonical_child`], which resolves symlinks and proves the result is inside the
//! root. A refusal is counted and **never echoed**, and it is reported as a refusal rather than as
//! "not found" — the disguise T2 forbids. A file that is not indexed is refused: **nothing here
//! creates an entity of any kind**, so an artifact cannot bring a path into the graph by naming it.
//!
//! A failed import commits nothing: everything runs inside one transaction, and the checks that
//! refuse an artifact whole run before it is opened.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::time::Instant;

use nerve_core::ids;
use nerve_core::model::{AssertionRecord, GraphBatch, ObservationRecord};
use nerve_core::vocab::{EvidenceSourceType, Relation};
use nerve_store::{FileProber, ObservationKey, SelectorShape};

use crate::config;
use crate::discover::{canonical_child, canonical_root, relative_path};
use crate::docref::{innermost_covering, SymbolExtent};
use crate::error::{IndexError, Result};
use crate::pipeline::RunStatus;
use crate::probe::RepositoryProber;
use crate::trace::{
    self, CompletionState, TraceArtifact, TraceHeader, TraceRecord, DECLARED_RELATIONS,
    DECLARED_SOURCE_TYPES, DIRECTNESS, EXTRACTOR_ID, EXTRACTOR_VERSION,
};

/// Form tags for what ingestion refused, on top of [`crate::trace::form`].
///
/// The vocabularies share one counter map, so a single reading of `nerve trace import` shows both
/// what the reader declined to believe and what the repository declined to confirm. The tags are
/// disjoint from every other extractor's by construction — asserted by test.
pub mod form {
    /// A recorded path the shared traversal refusal or the repository path guard refused.
    ///
    /// `..` in either spelling, an absolute path, a UNC path, a symlink resolving outside the root, a
    /// control character, non-UTF-8 — **and a path that cannot be canonicalized**, because a path
    /// that cannot be canonicalized cannot be proven to be inside the root. The refused text is
    /// counted and never echoed: it is hostile input by assumption.
    pub const PATH_REFUSED: &str = "path-refused";
    /// A path inside the root that Nerve has never indexed.
    ///
    /// Refused rather than trusted into existence (THREAT-MODEL.md T9). No entity is created for it,
    /// so an artifact cannot add a path to the graph.
    pub const FILE_NOT_INDEXED: &str = "file-not-indexed";
    /// An indexed path whose current bytes could not be obtained under the repository's read rules:
    /// deleted, deny-listed, or grown past the file-size ceiling.
    pub const FILE_UNREADABLE: &str = "file-unreadable";
    /// An indexed path whose bytes differ from what the index recorded.
    ///
    /// Refused, because the symbol extents a line would be mapped onto describe the *old* bytes.
    /// Re-index and import again.
    pub const FILE_CHANGED_SINCE_INDEX: &str = "file-changed-since-index";
    /// A caller line no symbol contains — a module top level, an import, a comment.
    ///
    /// The ordinary lossiness of line-to-symbol mapping, counted so that it is a number rather than a
    /// footnote. [`crate::docref::innermost_covering`] also answers `None` when two symbols tie for
    /// innermost, and that is counted here as well rather than distinguished: distinguishing it would
    /// mean a second implementation of "which symbol owns this line", which is the one thing Slice
    /// 5c's mapping must not grow a rival to.
    pub const CALLER_OUTSIDE_ANY_SYMBOL: &str = "caller-outside-any-symbol";
    /// A callee line no symbol contains. The caller is checked first, so a record whose frames are
    /// both unmappable is counted once, under the caller.
    pub const CALLEE_OUTSIDE_ANY_SYMBOL: &str = "callee-outside-any-symbol";
    /// The artifact names a different repository. It is refused **whole**, and counted once.
    pub const OTHER_REPOSITORY: &str = "other-repository";
    /// This artifact's `run_id` was already recorded, by an artifact with different bytes.
    ///
    /// **Counted and reported; nothing is overwritten.** Plan §7 requires that a corrected artifact
    /// with a repeated `run_id` still import, so refusing it would discard the correction — and
    /// refusal would buy nothing, because what a replay could want is to *replace* earlier evidence
    /// and that is exactly what does not happen: both runs survive in `environment.runs[]`, sharing a
    /// `run_id`, which is what makes the collision visible.
    ///
    /// Detected where it could do harm: on a call site both artifacts describe. A replayed `run_id`
    /// overlapping no previously observed site is not detected, and cannot overwrite anything either.
    pub const RUN_ID_CONFLICT: &str = "run-id-conflict";

    /// Every tag in this module, in declaration order.
    pub const ALL: [&str; 8] = [
        PATH_REFUSED,
        FILE_NOT_INDEXED,
        FILE_UNREADABLE,
        FILE_CHANGED_SINCE_INDEX,
        CALLER_OUTSIDE_ANY_SYMBOL,
        CALLEE_OUTSIDE_ANY_SYMBOL,
        OTHER_REPOSITORY,
        RUN_ID_CONFLICT,
    ];
}

/// Counted forms that are **not** a partial import, and each reason.
///
/// The distinction decides an exit code a CI job branches on, so it is a stated decision rather than
/// "whatever the counter map happened to hold". `coverage_ingest` reached the same conclusion for
/// `line-outside-any-symbol`: *"treating it as a failure would make every real repository exit 3
/// forever"*.
///
/// - [`form::CALLER_OUTSIDE_ANY_SYMBOL`] and [`form::CALLEE_OUTSIDE_ANY_SYMBOL`] are the ordinary
///   lossiness of mapping a line onto a symbol. Every real artifact has module-level frames.
/// - [`crate::trace::form::RECORD_UNKNOWN_KEY`] is one ignored datum about one edge — the contract's
///   stated policy, not a failure.
/// - [`crate::trace::form::PRODUCER_UNRESOLVED_FRAME`] is the **producer** saying it could not place
///   a frame. Nerve declined nothing; reporting Nerve's own run as partial for it would attribute
///   someone else's limit to Nerve, which is the category error `extractor_run.status` exists to
///   avoid (`docs/plans/slice-11a-trace-ingestion.md` §2.2).
///
/// Everything else is a case where Nerve declined to believe something it otherwise would have, and
/// that is what `RunStatus::Partial` means here.
const NOT_A_PARTIAL_IMPORT: [&str; 4] = [
    form::CALLER_OUTSIDE_ANY_SYMBOL,
    form::CALLEE_OUTSIDE_ANY_SYMBOL,
    crate::trace::form::RECORD_UNKNOWN_KEY,
    crate::trace::form::PRODUCER_UNRESOLVED_FRAME,
];

/// Whether anything counted amounts to Nerve declining to believe part of the artifact.
fn any_refusal_is_material(counters: &BTreeMap<String, usize>) -> bool {
    counters
        .iter()
        .any(|(tag, hits)| *hits > 0 && !NOT_A_PARTIAL_IMPORT.contains(&tag.as_str()))
}

/// How firmly an artifact is tied to the repository state the index describes.
///
/// **Three-valued on purpose.** A boolean would collapse *"the artifact says it was made against a
/// different tree"* and *"the artifact says nothing about which tree it was made against"* into one
/// answer, and those are different facts. `Unverified` is never reported as `Bound`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum TraceBinding {
    /// At least one declared state field disagrees with the index. Reported, never refused: the
    /// artifact is still about this repository.
    Stale,
    /// No state field was declared at all, so nothing could be checked.
    Unverified,
    /// Every declared state field agrees with the index.
    Bound,
}

impl TraceBinding {
    /// Every binding, in declaration order — weakest claim first.
    pub const ALL: [TraceBinding; 3] = [
        TraceBinding::Stale,
        TraceBinding::Unverified,
        TraceBinding::Bound,
    ];

    /// Canonical name, as it appears in `environment`, `details` and `--json` output.
    pub fn as_str(self) -> &'static str {
        match self {
            TraceBinding::Bound => "bound",
            TraceBinding::Stale => "stale",
            TraceBinding::Unverified => "unverified",
        }
    }

    /// Read the canonical name. Nothing outside the vocabulary parses.
    pub fn parse(text: &str) -> Option<TraceBinding> {
        TraceBinding::ALL
            .into_iter()
            .find(|binding| binding.as_str() == text)
    }

    /// The weaker of two bindings, `Stale < Unverified < Bound`.
    ///
    /// `Stale` is weaker than `Unverified` because a disagreement is a positive reason to distrust,
    /// while silence is only an absence of reason to trust. Used when one observation aggregates
    /// runs: a site observed by a bound run and a stale one must not read as bound.
    pub fn weaker(self, other: TraceBinding) -> TraceBinding {
        self.min(other)
    }

    /// Decide the binding by comparing an artifact's declared state against the index's.
    ///
    /// Each field is three-valued — agrees, disagrees, or could not be checked because one side has
    /// no value — and the verdict is: any disagreement is [`TraceBinding::Stale`]; otherwise at least
    /// one agreement is [`TraceBinding::Bound`]; otherwise [`TraceBinding::Unverified`].
    pub fn decide(
        declared_git_commit: Option<&str>,
        declared_content_merkle: Option<&str>,
        indexed_git_commit: Option<&str>,
        indexed_content_merkle: &str,
    ) -> TraceBinding {
        let mut agreed = false;
        let mut disagreed = false;
        let mut compare = |declared: Option<&str>, indexed: Option<&str>| {
            if let (Some(declared), Some(indexed)) = (declared, indexed) {
                if declared == indexed {
                    agreed = true;
                } else {
                    disagreed = true;
                }
            }
        };
        compare(declared_git_commit, indexed_git_commit);
        compare(declared_content_merkle, Some(indexed_content_merkle));

        if disagreed {
            TraceBinding::Stale
        } else if agreed {
            TraceBinding::Bound
        } else {
            TraceBinding::Unverified
        }
    }
}

/// What one import read, wrote and refused.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TraceOutcome {
    /// Canonical repository root.
    pub root: PathBuf,
    /// Repository-relative path of the artifact that was read.
    pub artifact_path: String,
    /// BLAKE3 of the artifact's bytes, or `None` when it was refused unread.
    pub artifact_content_hash: Option<String>,
    /// Repository state the import was recorded against — the one the index last observed.
    pub state_id: String,
    /// The traced run's identifier, or `None` when the artifact had no usable header.
    pub run_id: Option<String>,
    /// How firmly the artifact is tied to that state, or `None` when it was refused whole.
    pub binding: Option<TraceBinding>,
    /// Whether the traced run finished, as the producer states it.
    pub completion_state: Option<CompletionState>,
    /// Why it did not, when it did not.
    pub partial_reason: Option<String>,
    /// Limitations the producer declared for the whole run.
    pub declared_limitations: Vec<String>,
    /// Lines after the header, whether or not they were believed.
    pub records_in_artifact: usize,
    /// Records that yielded a located call event **and** resolved to two symbols.
    pub records_accepted: usize,
    /// Records the producer marked with an unsupported form.
    pub records_unsupported: usize,
    /// Distinct `(caller, callee)` pairs observed.
    pub edges_observed: usize,
    /// Distinct `(caller, callee, caller file, caller line)` sites, one observation each.
    pub observations_written: usize,
    /// Of those, sites that already had an observation and were restated with the union.
    pub observations_merged: usize,
    /// Refusals by form tag, from [`crate::trace::form`] and [`form`] alike.
    pub refused: BTreeMap<String, usize>,
    /// Producer-declared limitations by form tag, from [`crate::trace::limitation`].
    pub limitations: BTreeMap<String, usize>,
    /// Rows of Nerve's model this import inserted, updated or deleted.
    pub rows_written: usize,
    /// Wall-clock duration.
    pub duration_ms: u128,
    /// Terminal status of **Nerve's own** processing. Never the traced run's completion state.
    pub status: RunStatus,
}

impl TraceOutcome {
    /// Total refusals across every form.
    pub fn refused_total(&self) -> usize {
        self.refused.values().sum()
    }

    /// How many times `tag` was refused.
    pub fn refused_count(&self, tag: &str) -> usize {
        self.refused.get(tag).copied().unwrap_or(0)
    }

    /// Total producer-declared limitations across every form.
    pub fn limitations_total(&self) -> usize {
        self.limitations.values().sum()
    }

    /// How many records declared `tag`.
    pub fn limitation_count(&self, tag: &str) -> usize {
        self.limitations.get(tag).copied().unwrap_or(0)
    }
}

/// A resolved call site: the key an observation is stored under, plus the two symbols.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct Site {
    caller_entity_id: String,
    callee_entity_id: String,
    caller_file: String,
    caller_line: usize,
}

/// What one artifact says about one site, before it is merged with what is stored.
#[derive(Debug, Clone)]
struct SiteEvidence {
    callee_file: String,
    callee_line: usize,
    caller_content_hash: String,
    callee_content_hash: String,
    records: usize,
    observed_count: u64,
    by_test: BTreeMap<String, u64>,
}

/// One indexed file, probed once and reused for every record naming it.
enum FileState {
    /// Current, readable, and with at least one symbol.
    Usable {
        content_hash: String,
        extents: Vec<SymbolExtent>,
    },
    /// Refused, and already counted. The tag is remembered so a second record naming the same file
    /// is counted again — a per-record tally, not a per-file one.
    Refused(&'static str),
}

fn count(counters: &mut BTreeMap<String, usize>, tag: &str) {
    *counters.entry(tag.to_string()).or_insert(0) += 1;
}

/// Resolve one recorded path to an indexed, current file with symbol extents.
///
/// Both gates, in this order, and neither is skipped:
///
/// 1. [`nerve_store::selector_shape`] — the **shared** syntactic traversal refusal from Slice 8b-i,
///    which is what refuses `..\..\x` on a platform where `\` is not a separator. There is no second
///    copy of this check anywhere in Nerve.
/// 2. [`canonical_child`] — resolves symlinks and proves the result is under the root.
fn resolve_file<'cache>(
    root: &Path,
    conn: &nerve_store::Connection,
    prober: &RepositoryProber,
    cache: &'cache mut BTreeMap<String, FileState>,
    raw_path: &str,
) -> Result<&'cache FileState> {
    if !cache.contains_key(raw_path) {
        let state = load_file(root, conn, prober, raw_path)?;
        cache.insert(raw_path.to_string(), state);
    }
    Ok(cache.get(raw_path).expect("just inserted"))
}

fn load_file(
    root: &Path,
    conn: &nerve_store::Connection,
    prober: &RepositoryProber,
    raw_path: &str,
) -> Result<FileState> {
    // Gate 1: the shared refusal. A traversal-shaped path is a **refusal**, never a miss.
    if let SelectorShape::Refused(_) = nerve_store::selector_shape(raw_path) {
        return Ok(FileState::Refused(form::PATH_REFUSED));
    }
    // Gate 2: the repository path guard.
    let Ok(canonical) = canonical_child(root, Path::new(raw_path)) else {
        return Ok(FileState::Refused(form::PATH_REFUSED));
    };
    let Ok(rel_path) = relative_path(root, &canonical) else {
        return Ok(FileState::Refused(form::PATH_REFUSED));
    };

    if !nerve_store::path_is_indexed(conn, &rel_path)? {
        return Ok(FileState::Refused(form::FILE_NOT_INDEXED));
    }
    let Some(indexed_hash) = nerve_store::indexed_content_hash(conn, &rel_path)? else {
        return Ok(FileState::Refused(form::FILE_NOT_INDEXED));
    };
    let nerve_store::FileProbe::Hash(current_hash) = prober.probe(&rel_path) else {
        return Ok(FileState::Refused(form::FILE_UNREADABLE));
    };
    if current_hash != indexed_hash {
        return Ok(FileState::Refused(form::FILE_CHANGED_SINCE_INDEX));
    }

    let extents: Vec<SymbolExtent> = nerve_store::symbol_spans_in_file(conn, &rel_path)?
        .into_iter()
        .map(|row| SymbolExtent {
            entity_id: row.entity_id,
            start_byte: row.start_byte.max(0) as usize,
            end_byte: row.end_byte.max(0) as usize,
            start_line: row.start_line.max(0) as usize,
            end_line: row.end_line.max(0) as usize,
        })
        .collect();
    Ok(FileState::Usable {
        content_hash: current_hash,
        extents,
    })
}

/// The symbol a frame lands in, or the reason it lands in none.
///
/// `innermost_covering` is Slice 5c's mapping, **reused** rather than reimplemented — the same
/// function `coverage_ingest` maps a covered line with, and the same one a `#L<n>` document anchor
/// resolves through. A frame and a covered line landing in different symbols would be two
/// implementations of one question disagreeing.
fn frame_symbol(
    state: &FileState,
    line: u64,
    outside: &'static str,
) -> std::result::Result<(String, String), &'static str> {
    match state {
        FileState::Refused(tag) => Err(tag),
        FileState::Usable {
            content_hash,
            extents,
        } => {
            let mapped = usize::try_from(line)
                .ok()
                .and_then(|line| innermost_covering(extents, line));
            match mapped {
                Some(extent) => Ok((extent.entity_id.clone(), content_hash.clone())),
                None => Err(outside),
            }
        }
    }
}

/// Import one trace artifact into an existing index.
///
/// `artifact` may be given in any form the user typed; it is resolved through the repository path
/// guard and must live inside the repository, because a trace artifact is repository content and
/// nothing outside the root is ever opened.
///
/// Fails with [`IndexError::NotInitialized`] when there is no database and [`IndexError::NotIndexed`]
/// when there is one but nothing has been indexed into it: every frame is resolved against what the
/// index recorded, so without an index there is nothing to resolve against and the honest answer is a
/// refusal rather than an empty success.
pub fn ingest_trace(root: &Path, artifact: &Path) -> Result<TraceOutcome> {
    let started = Instant::now();
    let root = canonical_root(root)?;
    let db_path = config::db_path(&root);
    if !db_path.exists() {
        return Err(IndexError::NotInitialized(root));
    }

    // The artifact is repository content named by a user, and it goes through the same guard as every
    // path the artifact itself contains.
    let canonical_artifact = canonical_child(&root, artifact)?;
    let artifact_path = relative_path(&root, &canonical_artifact)?;

    let mut conn = nerve_store::open(&db_path)?;
    nerve_store::migrate(&conn)?;
    let Some(repository) = nerve_store::repository(&conn)? else {
        return Err(IndexError::NotIndexed(root));
    };
    let Some(state_id) = nerve_store::status(&conn)?.state_id else {
        return Err(IndexError::NotIndexed(root));
    };
    let Some(state) = nerve_store::repository_state(&conn, &state_id)? else {
        return Err(IndexError::NotIndexed(root));
    };

    let mut counters: BTreeMap<String, usize> = BTreeMap::new();
    let mut outcome = TraceOutcome {
        root: root.clone(),
        artifact_path: artifact_path.clone(),
        artifact_content_hash: None,
        state_id: state_id.clone(),
        run_id: None,
        binding: None,
        completion_state: None,
        partial_reason: None,
        declared_limitations: Vec::new(),
        records_in_artifact: 0,
        records_accepted: 0,
        records_unsupported: 0,
        edges_observed: 0,
        observations_written: 0,
        observations_merged: 0,
        refused: BTreeMap::new(),
        limitations: BTreeMap::new(),
        rows_written: 0,
        duration_ms: 0,
        status: RunStatus::Complete,
    };

    // The size bound, enforced before the read rather than after it. The reader refuses an oversized
    // artifact too, but only once it is in memory, and "in memory" is the resource the bound exists
    // to protect. A refused artifact withdraws nothing, because nothing here withdraws anything.
    let metadata = std::fs::metadata(&canonical_artifact)?;
    if !metadata.is_file() {
        return Err(IndexError::NotAFile(canonical_artifact));
    }
    if metadata.len() > trace::MAX_ARTIFACT_BYTES as u64 {
        count(&mut counters, trace::form::ARTIFACT_TOO_LARGE);
        return Ok(refused_whole(outcome, counters, started));
    }

    let bytes = std::fs::read(&canonical_artifact)?;
    let artifact_content_hash = ids::content_hash(&bytes);
    let parsed: TraceArtifact = trace::parse_trace(&bytes);
    for (tag, hits) in &parsed.counters.refused {
        *counters.entry(tag.clone()).or_insert(0) += hits;
    }
    for (tag, hits) in &parsed.counters.limitations {
        *outcome.limitations.entry(tag.clone()).or_insert(0) += hits;
    }
    outcome.artifact_content_hash = Some(artifact_content_hash.clone());
    outcome.records_in_artifact = parsed.records_in_artifact;
    outcome.records_unsupported = parsed.counters.limitations_total();

    let Some(header) = parsed.header else {
        // No header: no run identity, no binding, no completion state. Refused whole, and the reason
        // is already counted by the reader.
        return Ok(refused_whole(outcome, counters, started));
    };

    // The artifact is about another repository. Refused whole: its paths may name real files here and
    // mean something entirely different there.
    let root_name = root
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or_default();
    if header.repository_root_name != root_name {
        count(&mut counters, form::OTHER_REPOSITORY);
        outcome.run_id = Some(header.run_id.clone());
        return Ok(refused_whole(outcome, counters, started));
    }

    let binding = TraceBinding::decide(
        header.git_commit.as_deref(),
        header.content_merkle.as_deref(),
        state.git_commit.as_deref(),
        &state.content_merkle,
    );
    outcome.run_id = Some(header.run_id.clone());
    outcome.binding = Some(binding);
    outcome.completion_state = Some(header.completion_state);
    outcome.partial_reason = header.partial_reason.clone();
    outcome.declared_limitations = header.producer_limitations.clone();

    // ---- resolve every frame to a symbol ---------------------------------------------------
    let prober = RepositoryProber::new(&root)?;
    let mut files: BTreeMap<String, FileState> = BTreeMap::new();
    let mut sites: BTreeMap<Site, SiteEvidence> = BTreeMap::new();

    for record in &parsed.records {
        let accepted = resolve_record(
            &root,
            &conn,
            &prober,
            &mut files,
            &mut counters,
            record,
            &mut sites,
        )?;
        if accepted {
            outcome.records_accepted += 1;
        }
    }

    outcome.observations_written = sites.len();
    outcome.edges_observed = sites
        .keys()
        .map(|site| (site.caller_entity_id.clone(), site.callee_entity_id.clone()))
        .collect::<BTreeSet<_>>()
        .len();

    // ---- read what is stored, then restate the union -----------------------------------------
    let assertion_ids: Vec<String> = sites
        .keys()
        .map(|site| {
            ids::assertion_id(
                &site.caller_entity_id,
                Relation::TestObservedCall,
                &site.callee_entity_id,
            )
        })
        .collect();
    let stored = nerve_store::observations_for_assertions(&conn, EXTRACTOR_ID, &assertion_ids)?;
    let mut stored_by_key: BTreeMap<ObservationKey, (Option<String>, Option<String>)> =
        BTreeMap::new();
    for payload in stored {
        stored_by_key.insert(payload.key, (payload.environment, payload.details));
    }

    let mut batch = GraphBatch::default();
    let mut superseded: Vec<ObservationKey> = Vec::new();

    for (site, evidence) in &sites {
        let assertion_id = ids::assertion_id(
            &site.caller_entity_id,
            Relation::TestObservedCall,
            &site.callee_entity_id,
        );
        let key = ObservationKey {
            assertion_id: assertion_id.clone(),
            file_path: site.caller_file.clone(),
            start_line: site.caller_line as i64,
            end_line: site.caller_line as i64,
        };
        let stored_environment = stored_by_key
            .get(&key)
            .and_then(|(environment, _)| environment.clone());
        let this_run = run_entry(
            &header,
            binding,
            &artifact_path,
            &artifact_content_hash,
            evidence,
        );
        let runs = merge_runs(stored_environment.as_deref(), this_run, &mut counters);
        let environment = environment_json(&runs);
        let details = details_json(site, evidence, &runs);

        if let Some((stored_environment, stored_details)) = stored_by_key.get(&key) {
            outcome.observations_merged += 1;
            if stored_environment.as_deref() == Some(environment.as_str())
                && stored_details.as_deref() == Some(details.as_str())
            {
                // Byte-identical: this artifact says exactly what is already recorded. Writing it
                // would be a delete and an insert for no change, so the row is left alone and the
                // database stays byte-identical — which is what makes a re-import a no-op.
                continue;
            }
            superseded.push(key.clone());
        }

        batch.assertions.push(AssertionRecord {
            assertion_id: assertion_id.clone(),
            source_entity_id: site.caller_entity_id.clone(),
            relation: Relation::TestObservedCall,
            target_entity_id: site.callee_entity_id.clone(),
        });
        batch.observations.push(ObservationRecord {
            assertion_id,
            evidence_source_type: EvidenceSourceType::TestCallTrace,
            directness: DIRECTNESS,
            extractor_id: EXTRACTOR_ID.to_string(),
            extractor_version: EXTRACTOR_VERSION.to_string(),
            // No matching happens here, so a match quality would be a number about nothing.
            match_quality: None,
            // The **caller's** file and line: that is the site of the call, and what freshness
            // re-hashes. The callee's hash is recorded in `details` so a change on that side is
            // visible, though it does not drive `nerve why`'s freshness — a single-anchor limit
            // this extractor shares with coverage.
            file_path: site.caller_file.clone(),
            start_line: site.caller_line,
            end_line: site.caller_line,
            content_hash: evidence.caller_content_hash.clone(),
            environment: Some(environment),
            details: Some(details),
        });
    }
    batch.verify_declared_source_types(EXTRACTOR_ID, &DECLARED_SOURCE_TYPES)?;
    for assertion in &batch.assertions {
        debug_assert!(
            DECLARED_RELATIONS.contains(&assertion.relation),
            "the trace extractor may emit only TEST_OBSERVED_CALL"
        );
    }

    // ---- persist, in one transaction ---------------------------------------------------------
    {
        let tx = conn.transaction().map_err(nerve_store::StoreError::from)?;
        let mut touched = nerve_store::TouchedRows::default();

        let mut rows_written =
            nerve_store::delete_observations_at(&tx, EXTRACTOR_ID, &superseded, &mut touched)?;

        let run = nerve_store::begin_extractor_run(
            &tx,
            &repository.repo_id,
            &state_id,
            EXTRACTOR_ID,
            EXTRACTOR_VERSION,
        )?;
        rows_written +=
            nerve_store::persist_batch(&tx, &repository.repo_id, run, &batch, &mut touched)?;

        // A run that did not finish makes the import partial even when nothing was refused: a
        // partial trace must never read as a complete one, and a script that only looked at
        // refusals would treat an interrupted suite exactly as it treats a finished one.
        let status = if any_refusal_is_material(&counters) || !header.completion_state.is_complete()
        {
            RunStatus::Partial
        } else {
            RunStatus::Complete
        };
        nerve_store::finish_extractor_run(
            &tx,
            run,
            outcome.records_accepted as i64,
            (parsed
                .records_in_artifact
                .saturating_sub(outcome.records_accepted)) as i64,
            status.as_str(),
        )?;

        // Derived state, then pruning, in that order and inside this transaction — the same sequence
        // and the same reason as an index run.
        let derived = nerve_store::derive_assertion_state_for(&tx, &touched.assertions)?;
        rows_written += derived.total();
        let pruned = nerve_store::prune_orphans_scoped(&tx, &touched)?;
        rows_written += pruned.assertions + pruned.entities;

        tx.commit().map_err(nerve_store::StoreError::from)?;
        outcome.status = status;
        outcome.rows_written = rows_written;
    }

    outcome.refused = counters;
    outcome.duration_ms = started.elapsed().as_millis();
    Ok(outcome)
}

/// Finish an outcome that was refused before anything could be written.
///
/// Nothing has been opened as a transaction at this point, so "nothing commits" is structural rather
/// than a rollback: there is no transaction to roll back.
fn refused_whole(
    mut outcome: TraceOutcome,
    counters: BTreeMap<String, usize>,
    started: Instant,
) -> TraceOutcome {
    outcome.status = RunStatus::Partial;
    outcome.refused = counters;
    outcome.duration_ms = started.elapsed().as_millis();
    outcome
}

/// Resolve one record's two frames and fold it into `sites`. Returns whether it was accepted.
#[allow(clippy::too_many_arguments)]
fn resolve_record(
    root: &Path,
    conn: &nerve_store::Connection,
    prober: &RepositoryProber,
    files: &mut BTreeMap<String, FileState>,
    counters: &mut BTreeMap<String, usize>,
    record: &TraceRecord,
    sites: &mut BTreeMap<Site, SiteEvidence>,
) -> Result<bool> {
    // The caller is resolved first, and a failure there costs the record without the callee being
    // looked at. One record therefore contributes exactly one refusal, which is what makes the tally
    // a count of records rather than of frames.
    let caller = {
        let state = resolve_file(root, conn, prober, files, &record.caller_file)?;
        frame_symbol(state, record.caller_line, form::CALLER_OUTSIDE_ANY_SYMBOL)
    };
    let (caller_entity_id, caller_content_hash) = match caller {
        Ok(resolved) => resolved,
        Err(tag) => {
            count(counters, tag);
            return Ok(false);
        }
    };

    let callee = {
        let state = resolve_file(root, conn, prober, files, &record.callee_file)?;
        frame_symbol(state, record.callee_line, form::CALLEE_OUTSIDE_ANY_SYMBOL)
    };
    let (callee_entity_id, callee_content_hash) = match callee {
        Ok(resolved) => resolved,
        Err(tag) => {
            count(counters, tag);
            return Ok(false);
        }
    };

    let site = Site {
        caller_entity_id,
        callee_entity_id,
        caller_file: relative_of(root, &record.caller_file)?,
        caller_line: record.caller_line as usize,
    };
    let evidence = sites.entry(site).or_insert_with(|| SiteEvidence {
        callee_file: String::new(),
        callee_line: record.callee_line as usize,
        caller_content_hash,
        callee_content_hash,
        records: 0,
        observed_count: 0,
        by_test: BTreeMap::new(),
    });
    if evidence.callee_file.is_empty() {
        evidence.callee_file = relative_of(root, &record.callee_file)?;
    }
    evidence.records += 1;
    evidence.observed_count = evidence.observed_count.saturating_add(record.count);
    let per_test = evidence.by_test.entry(record.test_id.clone()).or_insert(0);
    *per_test = per_test.saturating_add(record.count);
    Ok(true)
}

/// The repository-relative spelling of a path already proven to resolve inside the root.
///
/// Called only after [`resolve_file`] answered `Usable`, so both steps are known to succeed; the
/// result is the canonical spelling, which is what two records naming one file by two spellings must
/// collapse onto.
fn relative_of(root: &Path, raw_path: &str) -> Result<String> {
    let canonical = canonical_child(root, Path::new(raw_path))?;
    relative_path(root, &canonical)
}

/// One run's entry in `environment.runs[]`.
fn run_entry(
    header: &TraceHeader,
    binding: TraceBinding,
    artifact_path: &str,
    artifact_content_hash: &str,
    evidence: &SiteEvidence,
) -> serde_json::Value {
    serde_json::json!({
        "run_id": header.run_id,
        "artifact_path": artifact_path,
        "artifact_content_hash": artifact_content_hash,
        "producer": header.producer,
        "producer_version": header.producer_version,
        "test_framework": header.test_framework,
        "runtime": header.runtime,
        "runtime_version": header.runtime_version,
        "platform": header.platform,
        "started_at": header.started_at,
        "completed_at": header.completed_at,
        "completion_state": header.completion_state.as_str(),
        "partial_reason": header.partial_reason,
        "source_map_state": header.source_map_state.as_str(),
        "repository_binding": binding.as_str(),
        "producer_limitations": header.producer_limitations,
        "records": evidence.records,
        "observed_count": evidence.observed_count,
        "tests": evidence
            .by_test
            .iter()
            .map(|(test, count)| (test.clone(), serde_json::json!(count)))
            .collect::<serde_json::Map<String, serde_json::Value>>(),
    })
}

/// Merge one run into whatever `environment.runs[]` already holds.
///
/// Keyed on `(run_id, artifact_content_hash)`, which is what makes a re-import of the same artifact a
/// no-op and a *corrected* artifact with a repeated `run_id` an addition rather than a replacement.
/// The repeat is counted under [`form::RUN_ID_CONFLICT`] and both entries survive, so the collision
/// is visible in the evidence instead of resolved behind the reader's back.
fn merge_runs(
    stored_environment: Option<&str>,
    this_run: serde_json::Value,
    counters: &mut BTreeMap<String, usize>,
) -> Vec<serde_json::Value> {
    let stored: Vec<serde_json::Value> = stored_environment
        .and_then(|text| serde_json::from_str::<serde_json::Value>(text).ok())
        .and_then(|value| value.get("runs").cloned())
        .and_then(|runs| match runs {
            serde_json::Value::Array(items) => Some(items),
            _ => None,
        })
        .unwrap_or_default();

    let identity = |run: &serde_json::Value| {
        (
            run.get("run_id")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string(),
            run.get("artifact_content_hash")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string(),
        )
    };
    let this_identity = identity(&this_run);

    let mut merged = Vec::new();
    let mut replaced = false;
    for run in stored {
        let existing = identity(&run);
        if existing == this_identity {
            // The same run, from the same bytes. Restated, not duplicated.
            replaced = true;
            merged.push(this_run.clone());
            continue;
        }
        if existing.0 == this_identity.0 {
            // The same run id from different bytes. Counted, and the earlier entry is **kept**.
            count(counters, form::RUN_ID_CONFLICT);
        }
        merged.push(run);
    }
    if !replaced {
        merged.push(this_run);
    }
    merged.sort_by_key(identity);
    merged
}

/// Build `observation.environment` from the merged run list.
///
/// Test identity lives here, per the slice's requirement, and it lives here as a **set** rather than
/// as one value because `idx_observation_identity` has no column that could hold a second row per
/// test. The derived scalars are the weakest claim across contributing runs, so a site observed by
/// one complete run and one interrupted run never reads as complete.
fn environment_json(runs: &[serde_json::Value]) -> String {
    let completion = runs
        .iter()
        .filter_map(|run| run.get("completion_state").and_then(|v| v.as_str()))
        .filter_map(CompletionState::parse)
        .reduce(CompletionState::weaker)
        .unwrap_or(CompletionState::Complete);
    let binding = runs
        .iter()
        .filter_map(|run| run.get("repository_binding").and_then(|v| v.as_str()))
        .filter_map(TraceBinding::parse)
        .reduce(TraceBinding::weaker)
        .unwrap_or(TraceBinding::Unverified);
    let mut tests: BTreeSet<String> = BTreeSet::new();
    for run in runs {
        if let Some(serde_json::Value::Object(map)) = run.get("tests") {
            for test in map.keys() {
                tests.insert(test.clone());
            }
        }
    }
    serde_json::json!({
        "runs": runs,
        "completion_state": completion.as_str(),
        "repository_binding": binding.as_str(),
        "tests": tests.into_iter().collect::<Vec<_>>(),
    })
    .to_string()
}

/// Build `observation.details`: the site, and the totals derived from the merged run list.
fn details_json(site: &Site, evidence: &SiteEvidence, runs: &[serde_json::Value]) -> String {
    let mut by_test: BTreeMap<String, u64> = BTreeMap::new();
    let mut observed_count = 0u64;
    let mut records = 0u64;
    let mut run_ids: BTreeSet<String> = BTreeSet::new();
    for run in runs {
        if let Some(id) = run.get("run_id").and_then(|v| v.as_str()) {
            run_ids.insert(id.to_string());
        }
        records = records.saturating_add(run.get("records").and_then(|v| v.as_u64()).unwrap_or(0));
        if let Some(serde_json::Value::Object(map)) = run.get("tests") {
            for (test, hits) in map {
                let hits = hits.as_u64().unwrap_or(0);
                observed_count = observed_count.saturating_add(hits);
                let slot = by_test.entry(test.clone()).or_insert(0);
                *slot = slot.saturating_add(hits);
            }
        }
    }
    let environment: serde_json::Value =
        serde_json::from_str(&environment_json(runs)).unwrap_or_else(|_| serde_json::json!({}));

    serde_json::json!({
        "rule": "trace frames mapped onto the innermost symbol containing each line; \
                 the endpoints are the two frames of the call, never the test",
        "caller_file": site.caller_file,
        "caller_line": site.caller_line,
        "callee_file": evidence.callee_file,
        "callee_line": evidence.callee_line,
        "caller_file_content_hash": evidence.caller_content_hash,
        "callee_file_content_hash": evidence.callee_content_hash,
        // Frequency, never importance. Nothing ranks by it.
        "observed_count": observed_count,
        "records": records,
        "by_test": by_test
            .into_iter()
            .map(|(test, hits)| (test, serde_json::json!(hits)))
            .collect::<serde_json::Map<String, serde_json::Value>>(),
        "runs": run_ids.into_iter().collect::<Vec<_>>(),
        "completion_state": environment["completion_state"],
        "repository_binding": environment["repository_binding"],
    })
    .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// One counter map, several vocabularies, and no tag that could mean either thing.
    #[test]
    fn the_form_vocabularies_are_disjoint_and_distinct() {
        let mut all: Vec<&str> = form::ALL.to_vec();
        all.extend(trace::form::ALL);
        all.extend(trace::limitation::ALL);
        let mut sorted = all.clone();
        sorted.sort_unstable();
        sorted.dedup();
        assert_eq!(sorted.len(), all.len(), "two tags collide");
        assert!(form::ALL.iter().all(|tag| !tag.is_empty()));
    }

    /// Every counted form is classified as material or not, so a new tag cannot default silently.
    #[test]
    fn every_counted_form_is_classified_for_the_partial_decision() {
        let mut material = Vec::new();
        let mut benign = Vec::new();
        for tag in form::ALL.into_iter().chain(trace::form::ALL) {
            let mut counters = BTreeMap::new();
            counters.insert(tag.to_string(), 1);
            if any_refusal_is_material(&counters) {
                material.push(tag);
            } else {
                benign.push(tag);
            }
        }
        assert_eq!(
            benign,
            NOT_A_PARTIAL_IMPORT.to_vec(),
            "a counted form changed side without a stated reason"
        );
        assert_eq!(
            material.len() + benign.len(),
            form::ALL.len() + trace::form::ALL.len(),
            "a form was neither material nor benign"
        );
        // A zero count is not a refusal, so a map full of zeroes leaves the import complete.
        let zeroes: BTreeMap<String, usize> = form::ALL
            .into_iter()
            .map(|tag| (tag.to_string(), 0))
            .collect();
        assert!(!any_refusal_is_material(&zeroes));
        assert!(!any_refusal_is_material(&BTreeMap::new()));
    }

    #[test]
    fn a_binding_is_a_recorded_value_with_a_stable_name_and_three_members() {
        assert_eq!(TraceBinding::ALL.len(), 3);
        assert_eq!(TraceBinding::Bound.as_str(), "bound");
        assert_eq!(TraceBinding::Stale.as_str(), "stale");
        assert_eq!(TraceBinding::Unverified.as_str(), "unverified");
        for binding in TraceBinding::ALL {
            assert_eq!(TraceBinding::parse(binding.as_str()), Some(binding));
        }
        for invented in ["fresh", "verified", "BOUND", ""] {
            assert_eq!(TraceBinding::parse(invented), None);
        }
        // Never the same value, so `unverified` can never be read as `bound`.
        assert_ne!(TraceBinding::Unverified, TraceBinding::Bound);
        assert_ne!(TraceBinding::Unverified, TraceBinding::Stale);
    }

    /// The binding table from `docs/plans/slice-11a-trace-ingestion.md` §5, case by case.
    #[test]
    fn the_binding_decision_follows_the_plans_table() {
        let merkle = "a".repeat(64);
        let other = "b".repeat(64);
        let commit = "c".repeat(40);
        let other_commit = "d".repeat(40);

        // Both declared and both matching.
        assert_eq!(
            TraceBinding::decide(Some(&commit), Some(&merkle), Some(&commit), &merkle),
            TraceBinding::Bound
        );
        // Content merkle differs, git commit matches: bound but stale, reported not refused.
        assert_eq!(
            TraceBinding::decide(Some(&commit), Some(&other), Some(&commit), &merkle),
            TraceBinding::Stale
        );
        // Git commit differs: a disagreement is a disagreement whichever field carries it.
        assert_eq!(
            TraceBinding::decide(Some(&other_commit), Some(&merkle), Some(&commit), &merkle),
            TraceBinding::Stale
        );
        // Neither state field declared: the distinct third value, never "fresh".
        assert_eq!(
            TraceBinding::decide(None, None, Some(&commit), &merkle),
            TraceBinding::Unverified
        );
        // Declared only where the index has nothing to compare against: still unverified, because
        // nothing was actually checked. This is the ordinary case for a tree with no `.git`.
        assert_eq!(
            TraceBinding::decide(Some(&commit), None, None, &merkle),
            TraceBinding::Unverified
        );
        // The merkle alone is enough to bind, and is the field the index always has.
        assert_eq!(
            TraceBinding::decide(None, Some(&merkle), None, &merkle),
            TraceBinding::Bound
        );
    }

    /// A disagreement outranks silence: `Stale` is weaker than `Unverified`.
    #[test]
    fn the_weaker_binding_wins_when_evidence_is_aggregated() {
        use TraceBinding::{Bound, Stale, Unverified};
        assert_eq!(Bound.weaker(Bound), Bound);
        assert_eq!(Bound.weaker(Unverified), Unverified);
        assert_eq!(Bound.weaker(Stale), Stale);
        assert_eq!(Unverified.weaker(Stale), Stale);
        for left in TraceBinding::ALL {
            for right in TraceBinding::ALL {
                assert_eq!(left.weaker(right), right.weaker(left));
            }
        }
    }

    fn run(run_id: &str, hash: &str, completion: &str, binding: &str) -> serde_json::Value {
        serde_json::json!({
            "run_id": run_id,
            "artifact_content_hash": hash,
            "completion_state": completion,
            "repository_binding": binding,
            "records": 1,
            "observed_count": 2,
            "tests": { "t::a": 2 },
        })
    }

    #[test]
    fn re_merging_the_same_run_restates_it_rather_than_duplicating_it() {
        let mut counters = BTreeMap::new();
        let first = environment_json(&merge_runs(
            None,
            run("r1", "h1", "complete", "bound"),
            &mut counters,
        ));
        let again = environment_json(&merge_runs(
            Some(&first),
            run("r1", "h1", "complete", "bound"),
            &mut counters,
        ));
        assert_eq!(first, again, "a re-import must be byte-identical");
        assert_eq!(counters.get(form::RUN_ID_CONFLICT), None);
    }

    #[test]
    fn a_second_run_is_added_and_the_derived_state_is_the_weakest() {
        let mut counters = BTreeMap::new();
        let first = environment_json(&merge_runs(
            None,
            run("r1", "h1", "complete", "bound"),
            &mut counters,
        ));
        let both = environment_json(&merge_runs(
            Some(&first),
            run("r2", "h2", "partial", "bound"),
            &mut counters,
        ));
        let value: serde_json::Value = serde_json::from_str(&both).unwrap();
        assert_eq!(value["runs"].as_array().unwrap().len(), 2);
        assert_eq!(
            value["completion_state"], "partial",
            "one unfinished contributor must make the row say so"
        );
        assert_eq!(value["repository_binding"], "bound");
        assert_eq!(counters.get(form::RUN_ID_CONFLICT), None);
    }

    /// A replayed run id keeps both entries and is counted. Nothing is overwritten.
    #[test]
    fn a_repeated_run_id_from_different_bytes_is_counted_and_both_survive() {
        let mut counters = BTreeMap::new();
        let first = environment_json(&merge_runs(
            None,
            run("r1", "h1", "complete", "bound"),
            &mut counters,
        ));
        let conflicted = merge_runs(
            Some(&first),
            run("r1", "h2", "complete", "stale"),
            &mut counters,
        );
        assert_eq!(conflicted.len(), 2, "the earlier run must be kept");
        assert_eq!(counters.get(form::RUN_ID_CONFLICT), Some(&1));
        let value: serde_json::Value =
            serde_json::from_str(&environment_json(&conflicted)).unwrap();
        assert_eq!(
            value["repository_binding"], "stale",
            "the weaker binding wins, so a replay cannot upgrade the row"
        );
    }

    /// The merge is order-independent, so two shards of one run give one answer either way round.
    #[test]
    fn merging_is_independent_of_the_order_artifacts_are_imported_in() {
        let mut counters = BTreeMap::new();
        let a = run("r1", "h1", "complete", "bound");
        let b = run("r2", "h2", "complete", "bound");
        let forwards = environment_json(&merge_runs(
            Some(&environment_json(&merge_runs(
                None,
                a.clone(),
                &mut counters,
            ))),
            b.clone(),
            &mut counters,
        ));
        let backwards = environment_json(&merge_runs(
            Some(&environment_json(&merge_runs(None, b, &mut counters))),
            a,
            &mut counters,
        ));
        assert_eq!(forwards, backwards);
    }

    /// Corrupt or absent stored evidence never loses the run being imported.
    #[test]
    fn an_unreadable_stored_environment_does_not_discard_the_new_run() {
        let mut counters = BTreeMap::new();
        for stored in [None, Some("not json"), Some("{}"), Some(r#"{"runs":7}"#)] {
            let merged = merge_runs(stored, run("r1", "h1", "complete", "bound"), &mut counters);
            assert_eq!(merged.len(), 1);
            assert_eq!(merged[0]["run_id"], "r1");
        }
    }
}
