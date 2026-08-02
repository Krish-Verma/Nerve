//! The coverage-gap question: *which symbols does no test touch?*
//!
//! This is the question Slice 6's data exists to answer, and the whole difficulty is that the
//! obvious implementation lies. `SELECT symbols WITHOUT a COVERS edge` returns **every symbol**
//! in a repository that has never run `nerve coverage`, and rendering that as a gap list reads as
//! *"your tests cover nothing"* when the truth is *"Nerve has not been told anything about your
//! tests"*. Those are different answers and conflating them is the exact defect this project
//! exists to prevent.
//!
//! So the answer carries its own epistemic state, at two levels.
//!
//! # Level one — is the question answerable at all
//!
//! [`CoverageEvidence::Absent`] means no `CoverageRun` exists in this repository. The gap
//! question is then **unanswerable**, [`GapReport::totals`] is `None` rather than a row of
//! zeroes, and no symbol is listed as a gap. A number that could not be computed must not be
//! printed as `0`, because `0` is a measurement and this is not one.
//!
//! # Level two — what the evidence says about one symbol
//!
//! [`SymbolCoverage`] has four values, and the two that mean "not covered" are deliberately
//! **not** merged:
//!
//! - [`SymbolCoverage::Uncovered`] — a coverage run named this symbol's file, and no line inside
//!   this symbol executed. Absence here is a *measurement*: the run instrumented the file and
//!   found nothing running in this symbol.
//! - [`SymbolCoverage::Unmeasured`] — no coverage evidence names this symbol's file at all.
//!   Absence here is *silence*: the file may be excluded from instrumentation, may not be loaded
//!   by the suite, may not even be reachable. Reporting it as a measured gap would repeat, one
//!   scale down, the same lie that level one exists to prevent.
//!
//! **The limit of that distinction, stated rather than glossed over.** "A file coverage named" is
//! read here as "a file some `COVERS` observation quotes", because that is the only trace an
//! ingested report leaves in the store — `coverage_ingest` writes no edge for a symbol with no
//! covered line, and the `CoverageRun` entity records only *how many* source files the report
//! described, not which. So a file that appears in the report with **every** line dead is
//! indistinguishable here from a file the report never mentioned, and lands in `Unmeasured`.
//! That is the weaker of the two claims, which is the right direction to be wrong in: the symbol
//! is still reported as a gap, and only the strength of the claim about it is understated.
//!
//! # Freshness
//!
//! A gap computed from stale coverage is a stale gap. Coverage evidence cites the **covered
//! file's** content hash at ingestion time, so [`crate::freshness`] re-hashes at query time and
//! every row carries the result. This applies to `Uncovered` rows exactly as it does to `Covered`
//! ones: the claim "the run measured this file and this symbol never ran" is only as current as
//! the file it was measured against.
//!
//! `Partial` is carried through as its own value and never rounded to covered or uncovered, for
//! the reason ADR-0008 §3 gives: a covered line inside a symbol proves the symbol was *entered*,
//! not that it ran to completion.

use std::collections::{BTreeMap, BTreeSet};

use rusqlite::{params, Connection};

use nerve_core::vocab::{EntityKind, Relation};

use crate::error::Result;
use crate::freshness::{FileProber, Freshness, FreshnessCache};
use crate::select::{symbol_kinds_sql, EntityRef, ENTITY_COLUMNS, ENTITY_FROM};

/// Whether this repository holds any coverage evidence at all.
///
/// The difference between *"your tests cover nothing"* and *"Nerve has not been told anything
/// about your tests"*, made a value so that no surface can accidentally render one as the other.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum CoverageEvidence {
    /// No coverage run has ever been ingested. The gap question is unanswerable.
    Absent,
    /// At least one coverage run has been ingested; gaps are measurable against it.
    Present,
}

impl CoverageEvidence {
    /// Every value, in declaration order.
    pub const ALL: [CoverageEvidence; 2] = [CoverageEvidence::Absent, CoverageEvidence::Present];

    /// Canonical name used in rendered and `--json` output.
    pub fn as_str(self) -> &'static str {
        match self {
            CoverageEvidence::Absent => "absent",
            CoverageEvidence::Present => "present",
        }
    }

    /// Whether the gap question can be answered at all.
    pub fn is_answerable(self) -> bool {
        matches!(self, CoverageEvidence::Present)
    }
}

impl std::fmt::Display for CoverageEvidence {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// What the ingested coverage evidence says about one symbol.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum SymbolCoverage {
    /// Every instrumented line inside the symbol executed. Not a gap.
    Covered,
    /// At least one instrumented line inside the symbol did not execute.
    ///
    /// Not a gap, and not fully covered either. Never rounded to either neighbour.
    Partial,
    /// A coverage run named this symbol's file, and no line inside this symbol executed.
    ///
    /// A measured gap.
    Uncovered,
    /// No coverage evidence names this symbol's file.
    ///
    /// A gap, but an unmeasured one — see the module documentation for why the two are kept
    /// apart and for the exact limit of the distinction.
    Unmeasured,
}

impl SymbolCoverage {
    /// Every value, in declaration order.
    pub const ALL: [SymbolCoverage; 4] = [
        SymbolCoverage::Covered,
        SymbolCoverage::Partial,
        SymbolCoverage::Uncovered,
        SymbolCoverage::Unmeasured,
    ];

    /// Canonical name used in rendered and `--json` output.
    pub fn as_str(self) -> &'static str {
        match self {
            SymbolCoverage::Covered => "covered",
            SymbolCoverage::Partial => "partial",
            SymbolCoverage::Uncovered => "uncovered",
            SymbolCoverage::Unmeasured => "unmeasured",
        }
    }

    /// Whether a symbol in this state is a gap — a symbol no test is known to touch.
    pub fn is_gap(self) -> bool {
        matches!(self, SymbolCoverage::Uncovered | SymbolCoverage::Unmeasured)
    }

    /// Whether the symbol was entered but not run through.
    pub fn is_partial(self) -> bool {
        matches!(self, SymbolCoverage::Partial)
    }
}

impl std::fmt::Display for SymbolCoverage {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// What to ask, and how much of the answer to return.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct GapQuery {
    /// Restrict to symbols in this repository-relative path, or under it as a directory.
    ///
    /// Compared as a path prefix on a `/` boundary, never as a `LIKE` pattern, so no caller text
    /// is ever interpreted as a wildcard.
    pub path_prefix: Option<String>,
    /// Restrict to one entity kind. Only kinds for which [`EntityKind::is_symbol`] holds can
    /// match anything.
    pub kind: Option<EntityKind>,
    /// Also return partially covered symbols. They are never *counted* as gaps either way; this
    /// only decides whether they appear as rows.
    pub include_partial: bool,
    /// Largest number of rows returned. The tallies are always exact regardless.
    pub limit: usize,
}

/// One coverage run the answer is relative to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CoverageRunRef {
    /// Entity identifier of the run.
    pub entity_id: String,
    /// Repository-relative path of the report that was ingested.
    pub report_path: Option<String>,
    /// The report's content hash at ingestion.
    pub report_content_hash: Option<String>,
    /// Whether the report file still hashes to what was ingested.
    pub freshness: Option<Freshness>,
    /// How many source files the report described, as recorded at ingestion.
    pub source_files_in_report: Option<i64>,
}

/// The exact tally over every symbol in scope.
///
/// Present only when coverage evidence exists. See [`GapReport::totals`].
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct GapTotals {
    /// Symbols every instrumented line of which executed.
    pub covered: usize,
    /// Symbols with at least one instrumented and unexecuted line.
    pub partial: usize,
    /// Symbols in a measured file that no coverage evidence names.
    pub uncovered: usize,
    /// Symbols whose file no coverage evidence names at all.
    pub unmeasured: usize,
    /// Symbols whose answer rests on coverage that no longer matches the file it measured.
    pub stale: usize,
    /// Distinct files some coverage observation names.
    pub measured_files: usize,
    /// Of those, files whose current bytes differ from what the coverage cited.
    pub stale_files: usize,
}

impl GapTotals {
    /// Symbols in a gap state — [`SymbolCoverage::is_gap`] over the tally.
    pub fn gaps(&self) -> usize {
        self.uncovered + self.unmeasured
    }
}

/// One symbol and what coverage says about it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GapRow {
    /// The symbol.
    pub entity: EntityRef,
    /// What the evidence says.
    pub state: SymbolCoverage,
    /// Whether that evidence still describes the file it was taken from.
    ///
    /// `None` for [`SymbolCoverage::Unmeasured`], where there is no evidence to be fresh or
    /// stale about.
    pub coverage_freshness: Option<Freshness>,
    /// Instrumented lines inside the symbol that executed, when coverage named it.
    pub covered_lines: Option<i64>,
    /// Instrumented lines inside the symbol, whatever their hit count.
    pub instrumented_lines: Option<i64>,
    /// Report paths of every coverage run asserting `COVERS` on this symbol.
    pub covered_by: Vec<String>,
}

/// Everything `nerve gaps` reports.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GapReport {
    /// Whether the question is answerable in this repository at all.
    pub coverage: CoverageEvidence,
    /// The coverage runs the answer is relative to, in report-path order.
    pub runs: Vec<CoverageRunRef>,
    /// Symbols matching the query's filters, whatever their state.
    pub symbols_in_scope: usize,
    /// The tally, or `None` when no coverage has ever been ingested.
    pub totals: Option<GapTotals>,
    /// The rows, capped at [`GapQuery::limit`].
    pub results: Vec<GapRow>,
    /// How many rows matched before the cap. Equal to `results.len()` when nothing was cut.
    pub results_total: usize,
    /// Whether the cap cut anything off.
    pub truncated: bool,
    /// The cap that was applied.
    pub limit: usize,
    /// Distinct files re-hashed to compute freshness.
    pub files_probed: usize,
}

/// One `COVERS` observation, as read from the store.
struct CoverageClaim {
    run_entity_id: String,
    file_path: String,
    content_hash: String,
    degree: SymbolCoverage,
    covered_lines: Option<i64>,
    instrumented_lines: Option<i64>,
}

/// Read `coverage`, `covered_lines` and `instrumented_lines` out of an observation's `details`.
///
/// `details` is extractor-written but lands in the database, which is a file on disk Nerve does
/// not own exclusively. A blob that does not parse, or that names a degree this build does not
/// know, degrades to `Covered`-with-no-numbers rather than panicking or being silently dropped:
/// the edge itself is the claim that the symbol ran, and the degree only refines it.
fn read_degree(details: Option<&str>) -> (SymbolCoverage, Option<i64>, Option<i64>) {
    let Some(parsed) = details
        .and_then(|text| serde_json::from_str::<serde_json::Value>(text).ok())
        .filter(serde_json::Value::is_object)
    else {
        return (SymbolCoverage::Covered, None, None);
    };
    let degree = match parsed.get("coverage").and_then(serde_json::Value::as_str) {
        Some("partial") => SymbolCoverage::Partial,
        _ => SymbolCoverage::Covered,
    };
    (
        degree,
        parsed
            .get("covered_lines")
            .and_then(serde_json::Value::as_i64),
        parsed
            .get("instrumented_lines")
            .and_then(serde_json::Value::as_i64),
    )
}

/// Every coverage run recorded in this repository, in report-path order.
fn coverage_runs(conn: &Connection, cache: &mut FreshnessCache<'_>) -> Result<Vec<CoverageRunRef>> {
    let sql = format!(
        "SELECT e.entity_id, o.file_path, o.content_hash, e.meta
         {ENTITY_FROM}
          WHERE e.kind = ?1
          ORDER BY o.file_path, e.entity_id"
    );
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map(params![EntityKind::CoverageRun.as_str()], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, Option<String>>(1)?,
            row.get::<_, Option<String>>(2)?,
            row.get::<_, Option<String>>(3)?,
        ))
    })?;

    let mut out = Vec::new();
    for row in rows {
        let (entity_id, report_path, report_content_hash, meta) = row?;
        let freshness = match (&report_path, &report_content_hash) {
            (Some(path), Some(hash)) => Some(cache.evaluate(path, hash)),
            _ => None,
        };
        let source_files_in_report = meta
            .as_deref()
            .and_then(|text| serde_json::from_str::<serde_json::Value>(text).ok())
            .and_then(|value| {
                value
                    .get("source_files_in_report")
                    .and_then(serde_json::Value::as_i64)
            });
        out.push(CoverageRunRef {
            entity_id,
            report_path,
            report_content_hash,
            freshness,
            source_files_in_report,
        });
    }
    Ok(out)
}

/// Every `COVERS` claim, keyed by the symbol it names.
fn coverage_claims(conn: &Connection) -> Result<BTreeMap<String, Vec<CoverageClaim>>> {
    let mut stmt = conn.prepare(
        "SELECT a.target_entity_id, a.source_entity_id, o.file_path, o.content_hash, o.details
           FROM assertion a
           INNER JOIN observation o ON o.assertion_id = a.assertion_id
          WHERE a.relation = ?1
          ORDER BY a.target_entity_id, a.source_entity_id, o.observation_id",
    )?;
    let rows = stmt.query_map(params![Relation::Covers.as_str()], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, String>(2)?,
            row.get::<_, String>(3)?,
            row.get::<_, Option<String>>(4)?,
        ))
    })?;

    let mut claims: BTreeMap<String, Vec<CoverageClaim>> = BTreeMap::new();
    for row in rows {
        let (target, run_entity_id, file_path, content_hash, details) = row?;
        let (degree, covered_lines, instrumented_lines) = read_degree(details.as_deref());
        claims.entry(target).or_default().push(CoverageClaim {
            run_entity_id,
            file_path,
            content_hash,
            degree,
            covered_lines,
            instrumented_lines,
        });
    }
    Ok(claims)
}

/// Every symbol in scope, with its first occurrence, in a stable order.
fn symbols_in_scope(conn: &Connection, query: &GapQuery) -> Result<Vec<EntityRef>> {
    // A path scope is matched as a prefix on a `/` boundary using `substr`, so that no caller
    // text is ever interpreted as a `LIKE` wildcard or an escape. The kind list is the other
    // half of the same property: it comes from the closed vocabulary, never from caller text.
    let sql = format!(
        "SELECT {ENTITY_COLUMNS}
         {ENTITY_FROM}
          WHERE e.kind IN ({})
            AND (?1 IS NULL OR e.kind = ?1)
            AND (?2 IS NULL
                 OR o.file_path = ?2
                 OR substr(o.file_path, 1, length(?2) + 1) = ?2 || '/')
          ORDER BY o.file_path, o.start_line, e.entity_id",
        symbol_kinds_sql()
    );
    let prefix = query
        .path_prefix
        .as_deref()
        .map(|value| value.trim_end_matches('/'))
        .filter(|value| !value.is_empty());
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map(
        params![query.kind.map(|kind| kind.as_str()), prefix],
        EntityRef::read,
    )?;
    let mut out = Vec::new();
    for row in rows {
        out.push(row?);
    }
    Ok(out)
}

/// Answer *"which symbols does no test touch?"* — with what is and is not known about it.
///
/// The tallies in [`GapReport::totals`] are exact over every symbol the filters admit;
/// [`GapQuery::limit`] caps only the returned rows, and [`GapReport::truncated`] says when it
/// did. Freshness is computed by re-reading the repository through `prober`, which enforces the
/// repository's path rules on every path the database supplies.
pub fn gaps(conn: &Connection, query: &GapQuery, prober: &dyn FileProber) -> Result<GapReport> {
    let mut cache = FreshnessCache::new(prober);
    let runs = coverage_runs(conn, &mut cache)?;
    let claims = coverage_claims(conn)?;
    let symbols = symbols_in_scope(conn, query)?;

    // Report path by run entity id, so a row can name the report rather than an opaque id.
    let run_paths: BTreeMap<&str, &str> = runs
        .iter()
        .filter_map(|run| {
            run.report_path
                .as_deref()
                .map(|path| (run.entity_id.as_str(), path))
        })
        .collect();

    // The files coverage evidence names, and how current that evidence is. A file quoted by
    // several observations is probed once; where they cite different hashes the worst answer
    // wins, because "some of this is stale" is not a fresh answer.
    let mut recorded_hashes: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for claim in claims.values().flatten() {
        recorded_hashes
            .entry(claim.file_path.clone())
            .or_default()
            .insert(claim.content_hash.clone());
    }
    let mut file_freshness: BTreeMap<String, Freshness> = BTreeMap::new();
    for (path, hashes) in &recorded_hashes {
        let worst = hashes
            .iter()
            .map(|hash| cache.evaluate(path, hash))
            .max()
            .unwrap_or(Freshness::Fresh);
        file_freshness.insert(path.clone(), worst);
    }

    let coverage = if runs.is_empty() && claims.is_empty() {
        CoverageEvidence::Absent
    } else {
        CoverageEvidence::Present
    };

    if coverage == CoverageEvidence::Absent {
        // Nothing is listed and nothing is tallied. Every symbol here is unmeasured, and a list
        // of every symbol in the repository presented as a gap list would be a lie told with
        // real data.
        return Ok(GapReport {
            coverage,
            runs,
            symbols_in_scope: symbols.len(),
            totals: None,
            results: Vec::new(),
            results_total: 0,
            truncated: false,
            limit: query.limit,
            files_probed: cache.files_probed(),
        });
    }

    let mut totals = GapTotals {
        measured_files: file_freshness.len(),
        stale_files: file_freshness
            .values()
            .filter(|freshness| **freshness != Freshness::Fresh)
            .count(),
        ..GapTotals::default()
    };

    let mut matching: Vec<GapRow> = Vec::new();
    for entity in symbols {
        let claims_for_symbol = claims.get(&entity.entity_id);
        let file = entity.file_path.clone().unwrap_or_default();

        let row = match claims_for_symbol {
            Some(list) if !list.is_empty() => {
                // Several runs may name one symbol. `Covered` wins over `Partial`, because a run
                // that executed every instrumented line is a positive observation and the other
                // run's silence about those lines is not a contradiction of it. The numbers come
                // from the same observation the degree does, never mixed across runs.
                let best = list
                    .iter()
                    .enumerate()
                    .min_by_key(|(index, claim)| {
                        (
                            claim.degree,
                            std::cmp::Reverse(claim.covered_lines.unwrap_or(0)),
                            *index,
                        )
                    })
                    .map(|(_, claim)| claim)
                    .expect("the list is not empty");
                let mut covered_by: Vec<String> = list
                    .iter()
                    .map(|claim| {
                        run_paths
                            .get(claim.run_entity_id.as_str())
                            .map(|path| (*path).to_string())
                            .unwrap_or_else(|| claim.run_entity_id.clone())
                    })
                    .collect();
                covered_by.sort();
                covered_by.dedup();
                GapRow {
                    state: best.degree,
                    coverage_freshness: file_freshness.get(&best.file_path).copied(),
                    covered_lines: best.covered_lines,
                    instrumented_lines: best.instrumented_lines,
                    covered_by,
                    entity,
                }
            }
            _ => {
                // No edge. Whether that is a measurement or a silence depends entirely on
                // whether anything measured the file — see the module documentation.
                let measured = file_freshness.get(&file).copied();
                GapRow {
                    state: match measured {
                        Some(_) => SymbolCoverage::Uncovered,
                        None => SymbolCoverage::Unmeasured,
                    },
                    coverage_freshness: measured,
                    covered_lines: None,
                    instrumented_lines: None,
                    covered_by: Vec::new(),
                    entity,
                }
            }
        };

        match row.state {
            SymbolCoverage::Covered => totals.covered += 1,
            SymbolCoverage::Partial => totals.partial += 1,
            SymbolCoverage::Uncovered => totals.uncovered += 1,
            SymbolCoverage::Unmeasured => totals.unmeasured += 1,
        }
        if row
            .coverage_freshness
            .is_some_and(|freshness| freshness != Freshness::Fresh)
        {
            totals.stale += 1;
        }

        let wanted = row.state.is_gap() || (query.include_partial && row.state.is_partial());
        if wanted {
            matching.push(row);
        }
    }

    let results_total = matching.len();
    let truncated = results_total > query.limit;
    matching.truncate(query.limit);

    Ok(GapReport {
        coverage,
        runs,
        symbols_in_scope: totals.covered + totals.partial + totals.uncovered + totals.unmeasured,
        totals: Some(totals),
        results: matching,
        results_total,
        truncated,
        limit: query.limit,
        files_probed: cache.files_probed(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_state_names_are_the_output_contract() {
        let names: Vec<&str> = SymbolCoverage::ALL.iter().map(|s| s.as_str()).collect();
        assert_eq!(names, vec!["covered", "partial", "uncovered", "unmeasured"]);
        let names: Vec<&str> = CoverageEvidence::ALL.iter().map(|c| c.as_str()).collect();
        assert_eq!(names, vec!["absent", "present"]);
    }

    /// The whole point of the four states: `partial` is not a gap, and the two "no edge" states
    /// are both gaps without being the same answer.
    #[test]
    fn only_the_two_absence_states_are_gaps() {
        assert!(!SymbolCoverage::Covered.is_gap());
        assert!(!SymbolCoverage::Partial.is_gap());
        assert!(SymbolCoverage::Uncovered.is_gap());
        assert!(SymbolCoverage::Unmeasured.is_gap());
        assert_ne!(SymbolCoverage::Uncovered, SymbolCoverage::Unmeasured);
    }

    #[test]
    fn absent_coverage_is_not_an_answerable_question() {
        assert!(!CoverageEvidence::Absent.is_answerable());
        assert!(CoverageEvidence::Present.is_answerable());
    }

    /// `details` is a blob in a file Nerve does not own exclusively. It never panics the query.
    #[test]
    fn an_unreadable_details_blob_degrades_rather_than_failing() {
        assert_eq!(
            read_degree(Some(
                "{\"coverage\":\"partial\",\"covered_lines\":1,\"instrumented_lines\":2}"
            )),
            (SymbolCoverage::Partial, Some(1), Some(2))
        );
        assert_eq!(
            read_degree(Some("{\"coverage\":\"covered\"}")),
            (SymbolCoverage::Covered, None, None)
        );
        assert_eq!(
            read_degree(Some("not json")),
            (SymbolCoverage::Covered, None, None)
        );
        assert_eq!(
            read_degree(Some("[1,2]")),
            (SymbolCoverage::Covered, None, None)
        );
        assert_eq!(read_degree(None), (SymbolCoverage::Covered, None, None));
    }

    /// `Covered` sorts before `Partial`, which is what makes the multi-run merge pick the
    /// positive observation rather than depending on row order.
    #[test]
    fn covered_outranks_partial_in_the_merge_order() {
        assert!(SymbolCoverage::Covered < SymbolCoverage::Partial);
        assert!(SymbolCoverage::Partial < SymbolCoverage::Uncovered);
    }

    #[test]
    fn a_tally_reports_gaps_as_the_two_absence_states() {
        let totals = GapTotals {
            covered: 3,
            partial: 2,
            uncovered: 4,
            unmeasured: 5,
            ..GapTotals::default()
        };
        assert_eq!(totals.gaps(), 9);
    }
}
