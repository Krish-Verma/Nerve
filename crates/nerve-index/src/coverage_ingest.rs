//! `nerve coverage`: turn one LCOV report the user names into `COVERS` observations.
//!
//! Slice 6a wrote the parser; this is the half that touches the world. It reads **one file, the
//! one the user named**, resolves every path in it through the same guard everything else uses,
//! maps every covered line onto a symbol with the same function `#L<n>` document anchors use, and
//! writes observations. It runs no test, spawns no process, discovers no report and opens nothing
//! else.
//!
//! # Why this is a command and not a flag on `nerve index`
//!
//! `docs/plans/slice-06-test-evidence.md` §A.4. A `--coverage` flag on `nerve index` would mean
//! that the ordinary post-edit `nerve index` — run without it, as it always is — silently
//! destroys every coverage edge in the repository. Making ingestion its own verb also makes "no
//! auto-discovery" structural: there is no code path here that looks for a report, only one that
//! is handed a path.
//!
//! # What is emitted, and what is deliberately not
//!
//! ```text
//! CoverageRun  COVERS  <symbol>     TEST_COVERAGE / INFERRED, coverage 1.0.0
//! ```
//!
//! and nothing else. The source endpoint is the **run**, never a test, because LCOV carries no
//! per-test attribution to read (ADR-0008). A symbol with no covered line gets **no edge**:
//! absence is the answer to the gap question, and a `NOT_COVERED` edge would put a negative claim
//! in a positive-evidence store.
//!
//! `INFERRED`, not `DIRECT`: the report says line `n` executed, and a mapping step concludes that
//! a *symbol* is covered. The lossiness that step introduces is recorded rather than smoothed —
//! [`form::LINE_OUTSIDE_ANY_SYMBOL`] counts every instrumented line the mapping could not
//! attribute, and every observation carries `covered` or `partial` as a value that is never
//! rounded.
//!
//! # Freshness is the point
//!
//! Every observation cites the **covered file's** path and that file's content hash at ingestion
//! time, so `nerve why` re-hashes at query time and a report that predates the code reads as
//! `stale` rather than as quietly wrong. The report's own hash is recorded too — on the
//! `CoverageRun` entity, in its `meta`, and on every observation's `details` — so "this coverage
//! came from a report that predates the code" is answerable from either end.
//!
//! Because the extents a line is mapped onto come from the last index, a file whose bytes have
//! moved since it was indexed is **refused** ([`form::FILE_CHANGED_SINCE_INDEX`]) rather than
//! mapped through stale spans. Recording a claim derived from extents known to be out of date and
//! then stamping it with the current hash would produce a row that says `fresh` and is wrong,
//! which is worse than no row at all.
//!
//! # T9 — the report is attacker-controlled input
//!
//! It is a file in the repository. Every `SF:` value goes through
//! [`crate::discover::canonical_child`] — the same choke point discovery, document links and the
//! query-time prober use — and a refusal is counted, never echoed. A path that resolves outside
//! the root, through a symlink or otherwise, is refused. A file that is not indexed is refused:
//! **nothing here creates a `File` entity**, so a report cannot bring a path into the graph by
//! naming it. A line number no symbol covers produces no edge. Every bound the parser enforces is
//! reported through the same counter map, and the report's size bound is enforced here too,
//! before the file is read.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::Instant;

use nerve_core::ids;
use nerve_core::model::{
    AssertionRecord, EntityRecord, GraphBatch, ObservationRecord, OccurrenceRecord, Span,
};
use nerve_core::vocab::{EntityKind, EvidenceSourceType, Relation};
use nerve_store::FileProber;

use crate::config;
use crate::coverage::{
    self, CoverageReport, DECLARED_SOURCE_TYPES, DIRECTNESS, EXTRACTOR_ID, EXTRACTOR_VERSION,
};
use crate::discover::{canonical_child, canonical_root, relative_path};
use crate::docref::{innermost_covering, SymbolExtent};
use crate::error::{IndexError, Result};
use crate::pipeline::RunStatus;
use crate::probe::RepositoryProber;

/// Form tags for what ingestion refused, on top of [`crate::coverage::form`].
///
/// The two vocabularies share one counter map, so a single reading of `nerve coverage` shows both
/// what the parser declined to believe and what the repository declined to confirm. The tags are
/// disjoint by construction — asserted by test.
pub mod form {
    /// An `SF:` path the repository path guard refused.
    ///
    /// Traversal, an absolute path elsewhere, a symlink resolving outside the root, a control
    /// character, non-UTF-8 — **and a path that does not exist**, because the guard canonicalizes
    /// and a path that cannot be canonicalized cannot be proven to be inside the root. The
    /// refused text is counted and never echoed: it is hostile input by assumption.
    pub const PATH_REFUSED: &str = "path-refused";
    /// A path inside the root that Nerve has never indexed.
    ///
    /// Refused rather than trusted into existence (THREAT-MODEL.md T9). No entity is created for
    /// it — in particular no `File` entity — so a report cannot add a path to the graph.
    pub const FILE_NOT_INDEXED: &str = "file-not-indexed";
    /// An indexed path whose current bytes could not be obtained under the repository's read
    /// rules: it has been deleted, deny-listed, or grown past the file-size ceiling.
    pub const FILE_UNREADABLE: &str = "file-unreadable";
    /// An indexed path whose bytes differ from what the index recorded.
    ///
    /// Refused, because the symbol extents a line would be mapped onto describe the *old* bytes.
    /// Re-index and ingest again.
    pub const FILE_CHANGED_SINCE_INDEX: &str = "file-changed-since-index";
    /// An indexed, current, readable file in which no symbol was ever recorded.
    ///
    /// A document, or a module that declares nothing. There is no endpoint for a coverage edge.
    pub const FILE_WITHOUT_SYMBOLS: &str = "file-without-symbols";
    /// Two records that resolved to one repository path. Merged by maximum, as the parser merges
    /// two records naming one path literally.
    pub const DUPLICATE_RESOLVED_PATH: &str = "duplicate-resolved-path";
    /// An instrumented line no symbol covers.
    ///
    /// The ordinary lossiness of line-to-symbol mapping — imports and module-level statements
    /// live outside every symbol — and also where an absurd line number lands. It is counted so
    /// that the lossiness is a number rather than a footnote, and it is **not** treated as a
    /// failure of the run: see [`CoverageOutcome::status`].
    ///
    /// [`crate::docref::innermost_covering`] also answers `None` when two symbols tie for
    /// innermost, and that refusal is counted here as well rather than distinguished, because
    /// distinguishing it would mean a second implementation of "which symbol owns this line" —
    /// the one thing Slice 5c's mapping must not grow a rival to.
    pub const LINE_OUTSIDE_ANY_SYMBOL: &str = "line-outside-any-symbol";

    /// Every tag in this module, in declaration order.
    pub const ALL: [&str; 7] = [
        PATH_REFUSED,
        FILE_NOT_INDEXED,
        FILE_UNREADABLE,
        FILE_CHANGED_SINCE_INDEX,
        FILE_WITHOUT_SYMBOLS,
        DUPLICATE_RESOLVED_PATH,
        LINE_OUTSIDE_ANY_SYMBOL,
    ];
}

/// Whether the report said a symbol ran in full or only in part.
///
/// **`Partial` is a recorded value and is never rounded** to covered or uncovered. A covered line
/// inside a symbol proves the symbol was *entered*, not that it ran to completion, and a store
/// that rounded that away would be asserting something no report said.
///
/// The comparison is against the lines LCOV **instrumented** inside the symbol's extent, not
/// against the extent's line count. A blank line and a closing brace are not instrumented, so
/// judging against the extent would report every symbol in every repository as partial and the
/// value would carry no information.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CoverageDegree {
    /// Every instrumented line inside the symbol executed.
    Covered,
    /// At least one instrumented line inside the symbol did not execute.
    Partial,
}

impl CoverageDegree {
    /// Canonical name, as it appears in `details` and in `--json` output.
    pub fn as_str(self) -> &'static str {
        match self {
            CoverageDegree::Covered => "covered",
            CoverageDegree::Partial => "partial",
        }
    }
}

/// What one ingestion read, wrote and refused.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CoverageOutcome {
    /// Canonical repository root.
    pub root: PathBuf,
    /// Repository-relative path of the report that was read.
    pub report_path: String,
    /// BLAKE3 of the report's bytes, or `None` when the report was refused unread.
    pub report_content_hash: Option<String>,
    /// Entity id of the `CoverageRun`, or `None` when nothing was ingested.
    pub coverage_run_entity_id: Option<String>,
    /// Repository state the ingestion was recorded against — the one the index last observed.
    pub state_id: String,
    /// Source files the report described, after parsing.
    pub files_in_report: usize,
    /// Source files whose coverage was ingested.
    pub files_ingested: usize,
    /// Source files refused at the repository layer.
    pub files_refused: usize,
    /// `COVERS` edges written.
    pub symbols_covered: usize,
    /// Of those, symbols every instrumented line of which executed.
    pub symbols_fully_covered: usize,
    /// Of those, symbols with at least one instrumented and unexecuted line.
    pub symbols_partially_covered: usize,
    /// Instrumented lines attributed to a symbol that executed at least once.
    pub covered_lines: usize,
    /// Instrumented lines attributed to a symbol that did not execute.
    pub uncovered_lines: usize,
    /// Refusals by form tag, from [`crate::coverage::form`] and [`form`] alike.
    pub refused: BTreeMap<String, usize>,
    /// Rows of Nerve's model this ingestion inserted, updated or deleted.
    pub rows_written: usize,
    /// Observations withdrawn from a previous ingestion of the same report path.
    pub observations_removed: usize,
    /// Occurrences withdrawn likewise.
    pub occurrences_removed: usize,
    /// Assertions pruned for want of any supporting observation.
    pub assertions_removed: usize,
    /// Entities pruned for want of any occurrence or incident assertion.
    pub entities_removed: usize,
    /// Wall-clock duration.
    pub duration_ms: u128,
    /// Terminal status. See the field documentation on [`CoverageOutcome::status`].
    pub status: RunStatus,
}

impl CoverageOutcome {
    /// Total refusals across every form.
    pub fn refused_total(&self) -> usize {
        self.refused.values().sum()
    }

    /// How many times `tag` was counted.
    pub fn refused_count(&self, tag: &str) -> usize {
        self.refused.get(tag).copied().unwrap_or(0)
    }
}

/// One symbol's tally while the report is being read.
#[derive(Debug, Clone)]
struct SymbolTally {
    file_path: String,
    content_hash: String,
    start_line: usize,
    end_line: usize,
    instrumented: usize,
    covered: usize,
    first_covered_line: usize,
    last_covered_line: usize,
}

impl SymbolTally {
    fn degree(&self) -> CoverageDegree {
        if self.covered == self.instrumented {
            CoverageDegree::Covered
        } else {
            CoverageDegree::Partial
        }
    }
}

fn count(counters: &mut BTreeMap<String, usize>, tag: &'static str) {
    *counters.entry(tag.to_string()).or_insert(0) += 1;
}

/// Lines a byte buffer spans, 1-based and never zero, so the report's occurrence has a real end.
fn line_count(bytes: &[u8]) -> usize {
    let newlines = bytes.iter().filter(|byte| **byte == b'\n').count();
    let unterminated = usize::from(!bytes.is_empty() && !bytes.ends_with(b"\n"));
    (newlines + unterminated).max(1)
}

/// Ingest one LCOV report into an existing index.
///
/// `report` may be given in any form the user typed; it is resolved through the repository path
/// guard and must live inside the repository, because a coverage report is repository content and
/// nothing outside the root is ever opened.
///
/// Fails with [`IndexError::NotInitialized`] when there is no database and [`IndexError::NotIndexed`]
/// when there is one but nothing has been indexed into it: every path in a report is resolved
/// against what the index recorded, so without an index there is nothing to resolve against and
/// the honest answer is a refusal rather than an empty success.
pub fn ingest_coverage(root: &Path, report: &Path) -> Result<CoverageOutcome> {
    let started = Instant::now();
    let root = canonical_root(root)?;
    let db_path = config::db_path(&root);
    if !db_path.exists() {
        return Err(IndexError::NotInitialized(root));
    }

    // The report is repository content named by a user, and it goes through the same guard as
    // every path the report itself contains. Nothing outside the root is read, including here.
    let canonical_report = canonical_child(&root, report)?;
    let report_path = relative_path(&root, &canonical_report)?;

    let mut conn = nerve_store::open(&db_path)?;
    nerve_store::migrate(&conn)?;
    let Some(repository) = nerve_store::repository(&conn)? else {
        return Err(IndexError::NotIndexed(root));
    };
    let Some(state_id) = nerve_store::status(&conn)?.state_id else {
        return Err(IndexError::NotIndexed(root));
    };

    let mut counters: BTreeMap<String, usize> = BTreeMap::new();
    let mut outcome = CoverageOutcome {
        root: root.clone(),
        report_path: report_path.clone(),
        report_content_hash: None,
        coverage_run_entity_id: None,
        state_id: state_id.clone(),
        files_in_report: 0,
        files_ingested: 0,
        files_refused: 0,
        symbols_covered: 0,
        symbols_fully_covered: 0,
        symbols_partially_covered: 0,
        covered_lines: 0,
        uncovered_lines: 0,
        refused: BTreeMap::new(),
        rows_written: 0,
        observations_removed: 0,
        occurrences_removed: 0,
        assertions_removed: 0,
        entities_removed: 0,
        duration_ms: 0,
        status: RunStatus::Complete,
    };

    // The size bound, enforced before the read rather than after it. The parser refuses an
    // oversized report too, but only once it is in memory, and "in memory" is the resource the
    // bound exists to protect. A refused report is refused **whole** and withdraws nothing: a
    // report Nerve declined to read must not destroy the evidence of the last one it did read.
    let metadata = std::fs::metadata(&canonical_report)?;
    if !metadata.is_file() {
        return Err(IndexError::NotAFile(canonical_report));
    }
    if metadata.len() > coverage::MAX_REPORT_BYTES as u64 {
        count(&mut counters, coverage::form::REPORT_TOO_LARGE);
        outcome.refused = counters;
        outcome.status = RunStatus::Partial;
        outcome.duration_ms = started.elapsed().as_millis();
        return Ok(outcome);
    }

    let bytes = std::fs::read(&canonical_report)?;
    let report_content_hash = ids::content_hash(&bytes);
    let parsed: CoverageReport = coverage::parse_lcov(&bytes);
    for (tag, hits) in &parsed.counters.refused {
        *counters.entry(tag.clone()).or_insert(0) += hits;
    }
    outcome.files_in_report = parsed.files.len();
    outcome.report_content_hash = Some(report_content_hash.clone());

    // ---- resolve every path in the report, then merge ------------------------------------
    //
    // Two records can name one repository file by different spellings (`src/a.ts` and
    // `./src/a.ts`), which the parser cannot see because it compares the text it was given. They
    // are merged here by the same rule the parser merges literal duplicates with — the maximum,
    // never the sum — so that one file is described once however many times it was written down.
    let mut merged: BTreeMap<String, BTreeMap<u64, u64>> = BTreeMap::new();
    for file in &parsed.files {
        let Ok(canonical) = canonical_child(&root, Path::new(file.raw_path.as_str())) else {
            count(&mut counters, form::PATH_REFUSED);
            continue;
        };
        let Ok(rel_path) = relative_path(&root, &canonical) else {
            count(&mut counters, form::PATH_REFUSED);
            continue;
        };
        let entry = merged.entry(rel_path).or_default();
        if !entry.is_empty() {
            count(&mut counters, form::DUPLICATE_RESOLVED_PATH);
        }
        for hit in &file.lines {
            let slot = entry.entry(hit.line).or_insert(0);
            *slot = (*slot).max(hit.hits);
        }
    }

    // ---- map lines onto symbols ------------------------------------------------------------
    let prober = RepositoryProber::new(&root)?;
    let mut tallies: BTreeMap<String, SymbolTally> = BTreeMap::new();
    for (rel_path, lines) in &merged {
        if !nerve_store::path_is_indexed(&conn, rel_path)? {
            count(&mut counters, form::FILE_NOT_INDEXED);
            continue;
        }
        let Some(indexed_hash) = nerve_store::indexed_content_hash(&conn, rel_path)? else {
            count(&mut counters, form::FILE_NOT_INDEXED);
            continue;
        };
        let nerve_store::FileProbe::Hash(current_hash) = prober.probe(rel_path) else {
            count(&mut counters, form::FILE_UNREADABLE);
            continue;
        };
        if current_hash != indexed_hash {
            count(&mut counters, form::FILE_CHANGED_SINCE_INDEX);
            continue;
        }

        let extents: Vec<SymbolExtent> = nerve_store::symbol_spans_in_file(&conn, rel_path)?
            .into_iter()
            .map(|row| SymbolExtent {
                entity_id: row.entity_id,
                start_byte: row.start_byte.max(0) as usize,
                end_byte: row.end_byte.max(0) as usize,
                start_line: row.start_line.max(0) as usize,
                end_line: row.end_line.max(0) as usize,
            })
            .collect();
        if extents.is_empty() {
            count(&mut counters, form::FILE_WITHOUT_SYMBOLS);
            continue;
        }

        outcome.files_ingested += 1;
        for (line, hits) in lines {
            // `innermost_covering` is Slice 5c's mapping, reused rather than reimplemented: the
            // symbol a `#L<n>` anchor lands in and the symbol a covered line lands in must be the
            // same symbol, and two implementations could disagree.
            let mapped = usize::try_from(*line)
                .ok()
                .and_then(|line| innermost_covering(&extents, line));
            let Some(extent) = mapped else {
                count(&mut counters, form::LINE_OUTSIDE_ANY_SYMBOL);
                continue;
            };
            let line = *line as usize;
            let tally = tallies
                .entry(extent.entity_id.clone())
                .or_insert_with(|| SymbolTally {
                    file_path: rel_path.clone(),
                    content_hash: current_hash.clone(),
                    start_line: extent.start_line,
                    end_line: extent.end_line,
                    instrumented: 0,
                    covered: 0,
                    first_covered_line: 0,
                    last_covered_line: 0,
                });
            tally.instrumented += 1;
            if *hits > 0 {
                tally.covered += 1;
                outcome.covered_lines += 1;
                if tally.first_covered_line == 0 {
                    tally.first_covered_line = line;
                }
                tally.last_covered_line = tally.last_covered_line.max(line);
            } else {
                outcome.uncovered_lines += 1;
            }
        }
    }

    // ---- build the batch -------------------------------------------------------------------
    let run_entity_id =
        ids::coverage_run_id(&repository.project_id, &report_path, &report_content_hash);
    let mut batch = GraphBatch::default();
    batch.entities.push(EntityRecord {
        entity_id: run_entity_id.clone(),
        kind: EntityKind::CoverageRun,
        name: last_segment(&report_path).to_string(),
        scope_path: parent_directory(&report_path).unwrap_or_default(),
        language: None,
        meta: Some(
            serde_json::json!({
                "format": "lcov",
                "report_path": report_path,
                "report_content_hash": report_content_hash,
                "source_files_in_report": parsed.files.len(),
                // LCOV's own test-name field, and the reason there is no `Test` endpoint here.
                // Recorded on the run so the ADR's finding is visible from the graph.
                "per_test_attribution": false,
            })
            .to_string(),
        ),
    });
    // A real occurrence at a real path: the report exists, and this is where it is.
    batch.occurrences.push(OccurrenceRecord {
        occurrence_id: ids::occurrence_id(&run_entity_id, &report_path, 0, bytes.len()),
        entity_id: run_entity_id.clone(),
        file_path: report_path.clone(),
        span: Span {
            start_byte: 0,
            end_byte: bytes.len(),
            start_line: 1,
            start_col: 0,
            end_line: line_count(&bytes),
            end_col: 0,
        },
        content_hash: report_content_hash.clone(),
    });

    for (entity_id, tally) in &tallies {
        // A symbol with no covered line gets no edge. Absence is the answer to the gap question.
        if tally.covered == 0 {
            continue;
        }
        let degree = tally.degree();
        outcome.symbols_covered += 1;
        match degree {
            CoverageDegree::Covered => outcome.symbols_fully_covered += 1,
            CoverageDegree::Partial => outcome.symbols_partially_covered += 1,
        }

        let assertion_id = ids::assertion_id(&run_entity_id, Relation::Covers, entity_id);
        batch.assertions.push(AssertionRecord {
            assertion_id: assertion_id.clone(),
            source_entity_id: run_entity_id.clone(),
            relation: Relation::Covers,
            target_entity_id: entity_id.clone(),
        });
        batch.observations.push(ObservationRecord {
            assertion_id,
            evidence_source_type: EvidenceSourceType::TestCoverage,
            directness: DIRECTNESS,
            extractor_id: EXTRACTOR_ID.to_string(),
            extractor_version: EXTRACTOR_VERSION.to_string(),
            // No matching happens here, so a match quality would be a number about nothing.
            match_quality: None,
            // The covered file, not the report: this is what freshness re-hashes.
            file_path: tally.file_path.clone(),
            start_line: tally.first_covered_line,
            end_line: tally.last_covered_line,
            content_hash: tally.content_hash.clone(),
            // LCOV records no environment. Inventing one would be the invention this slice
            // exists to refuse.
            environment: None,
            details: Some(
                serde_json::json!({
                    "rule": "LCOV DA: lines mapped onto the innermost symbol covering each line",
                    "coverage": degree.as_str(),
                    "covered_lines": tally.covered,
                    "instrumented_lines": tally.instrumented,
                    "symbol_start_line": tally.start_line,
                    "symbol_end_line": tally.end_line,
                    "symbol_extent_lines": tally.end_line.saturating_sub(tally.start_line) + 1,
                    "report_path": report_path,
                    "report_content_hash": report_content_hash,
                    "covered_file_content_hash": tally.content_hash,
                })
                .to_string(),
            ),
        });
    }
    batch.verify_declared_source_types(EXTRACTOR_ID, &DECLARED_SOURCE_TYPES)?;

    outcome.files_refused = counters
        .iter()
        .filter(|(tag, _)| {
            [
                form::PATH_REFUSED,
                form::FILE_NOT_INDEXED,
                form::FILE_UNREADABLE,
                form::FILE_CHANGED_SINCE_INDEX,
                form::FILE_WITHOUT_SYMBOLS,
            ]
            .contains(&tag.as_str())
        })
        .map(|(_, hits)| *hits)
        .sum();

    // ---- persist ---------------------------------------------------------------------------
    {
        let tx = conn.transaction().map_err(nerve_store::StoreError::from)?;
        let mut touched = nerve_store::TouchedRows::default();

        // Re-ingesting the same report path replaces what that path previously claimed. Without
        // this, a second run of the suite would leave the first run's edges standing beside its
        // own — two measurements of the same thing, indistinguishable in a query.
        let mut removals =
            nerve_store::delete_claims_sourced_at(&tx, EXTRACTOR_ID, &report_path, &mut touched)?;
        let mut rows_written = removals.observations + removals.occurrences;

        let run_id = nerve_store::begin_extractor_run(
            &tx,
            &repository.repo_id,
            &state_id,
            EXTRACTOR_ID,
            EXTRACTOR_VERSION,
        )?;
        rows_written +=
            nerve_store::persist_batch(&tx, &repository.repo_id, run_id, &batch, &mut touched)?;
        let status = if outcome.files_refused > 0 || parsed.counters.total() > 0 {
            RunStatus::Partial
        } else {
            RunStatus::Complete
        };
        nerve_store::finish_extractor_run(
            &tx,
            run_id,
            outcome.files_ingested as i64,
            outcome.files_refused as i64,
            status.as_str(),
        )?;

        // Derived state, then pruning, in that order and inside this transaction — the same
        // sequence and the same reason as an index run: derivation leaves no row behind for a
        // claim nothing observes, so pruning it breaks no foreign key.
        let derived = nerve_store::derive_assertion_state_for(&tx, &touched.assertions)?;
        rows_written += derived.total();
        let pruned = nerve_store::prune_orphans_scoped(&tx, &touched)?;
        rows_written += pruned.assertions + pruned.entities;
        removals.add(pruned);

        tx.commit().map_err(nerve_store::StoreError::from)?;

        outcome.status = status;
        outcome.rows_written = rows_written;
        outcome.observations_removed = removals.observations;
        outcome.occurrences_removed = removals.occurrences;
        outcome.assertions_removed = removals.assertions;
        outcome.entities_removed = removals.entities;
    }

    outcome.coverage_run_entity_id = Some(run_entity_id);
    outcome.refused = counters;
    outcome.duration_ms = started.elapsed().as_millis();
    Ok(outcome)
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

#[cfg(test)]
mod tests {
    use super::*;

    /// One counter map, two vocabularies, and no tag that could mean either thing.
    #[test]
    fn the_two_form_vocabularies_are_disjoint_and_distinct() {
        let mut all: Vec<&str> = coverage::form::ALL.to_vec();
        all.extend(form::ALL);
        let mut sorted = all.clone();
        sorted.sort_unstable();
        sorted.dedup();
        assert_eq!(
            sorted.len(),
            all.len(),
            "a parser and an ingest tag collide"
        );
        assert!(form::ALL.iter().all(|tag| !tag.is_empty()));
    }

    #[test]
    fn a_degree_is_a_recorded_value_with_a_stable_name() {
        assert_eq!(CoverageDegree::Covered.as_str(), "covered");
        assert_eq!(CoverageDegree::Partial.as_str(), "partial");
    }

    /// `partial` is never rounded: one unexecuted instrumented line is enough, however many ran.
    #[test]
    fn one_unexecuted_instrumented_line_makes_a_symbol_partial() {
        let tally = |instrumented, covered| SymbolTally {
            file_path: "src/a.ts".to_string(),
            content_hash: "h".to_string(),
            start_line: 1,
            end_line: 20,
            instrumented,
            covered,
            first_covered_line: 1,
            last_covered_line: 2,
        };
        assert_eq!(tally(1, 1).degree(), CoverageDegree::Covered);
        assert_eq!(tally(100, 100).degree(), CoverageDegree::Covered);
        assert_eq!(tally(100, 99).degree(), CoverageDegree::Partial);
        assert_eq!(tally(2, 1).degree(), CoverageDegree::Partial);
    }

    #[test]
    fn a_reports_line_count_is_never_zero_and_counts_an_unterminated_last_line() {
        assert_eq!(line_count(b""), 1);
        assert_eq!(line_count(b"a\n"), 1);
        assert_eq!(line_count(b"a\nb\n"), 2);
        assert_eq!(line_count(b"a\nb"), 2);
    }

    #[test]
    fn path_helpers_agree_with_the_pipelines() {
        assert_eq!(
            parent_directory("coverage/lcov.info"),
            Some("coverage".to_string())
        );
        assert_eq!(parent_directory("lcov.info"), None);
        assert_eq!(last_segment("coverage/lcov.info"), "lcov.info");
        assert_eq!(last_segment("lcov.info"), "lcov.info");
    }
}
