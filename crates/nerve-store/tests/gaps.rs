//! The coverage-gap question, over a graph small enough to reason about completely.
//!
//! These tests deliberately do not go through the indexer or the LCOV parser. The property under
//! test is not "does the parser read `DA:` lines" — Slice 6 pinned that — but "does the *answer*
//! say which of four different things it knows", including the one that matters most: that a
//! repository with no coverage at all must not report every symbol as a gap.

use std::collections::BTreeMap;

use nerve_core::vocab::{EntityKind, Relation};
use nerve_store::{
    gaps, migrate, open_in_memory, Connection, CoverageEvidence, FileProbe, FileProber, Freshness,
    GapQuery, SymbolCoverage,
};

const STATE: &str = "state-1";
const FRESH_A: &str = "hash-a";
const FRESH_B: &str = "hash-b";
const FRESH_REPORT: &str = "hash-report";

/// A prober that answers from a table, so freshness can be tested without a filesystem.
struct StubProber {
    answers: BTreeMap<String, FileProbe>,
}

impl FileProber for StubProber {
    fn probe(&self, rel_path: &str) -> FileProbe {
        self.answers
            .get(rel_path)
            .cloned()
            .unwrap_or(FileProbe::Missing)
    }
}

/// Every file hashes to what the coverage recorded.
fn everything_fresh() -> StubProber {
    prober(&[
        ("src/a.ts", FileProbe::Hash(FRESH_A.into())),
        ("src/b.ts", FileProbe::Hash(FRESH_B.into())),
        ("src/untested.ts", FileProbe::Hash("hash-u".into())),
        ("coverage/lcov.info", FileProbe::Hash(FRESH_REPORT.into())),
    ])
}

fn prober(pairs: &[(&str, FileProbe)]) -> StubProber {
    StubProber {
        answers: pairs
            .iter()
            .map(|(path, probe)| ((*path).to_string(), probe.clone()))
            .collect(),
    }
}

fn query(limit: usize) -> GapQuery {
    GapQuery {
        limit,
        ..GapQuery::default()
    }
}

fn entity(conn: &Connection, id: &str, kind: &str, name: &str, file: &str, line: i64, hash: &str) {
    conn.execute(
        "INSERT INTO entity (entity_id, repo_id, kind, name, scope_path, language)
         VALUES (?1, 'repo', ?2, ?3, '', 'typescript')",
        rusqlite::params![id, kind, name],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO occurrence (occurrence_id, entity_id, file_path, start_byte,
                                 end_byte, start_line, start_col, end_line, end_col, content_hash)
         VALUES (?1, ?2, ?3, 0, 1, ?4, 0, ?4, 1, ?5)",
        rusqlite::params![format!("occ-{id}"), id, file, line, hash],
    )
    .unwrap();
}

/// What a `COVERS` observation's `details` blob says: the degree, and the two line counts it
/// was judged from.
#[derive(Debug, Clone, Copy)]
struct Degree {
    name: &'static str,
    covered: i64,
    instrumented: i64,
}

const FULLY: Degree = Degree {
    name: "covered",
    covered: 4,
    instrumented: 4,
};

const PARTLY: Degree = Degree {
    name: "partial",
    covered: 1,
    instrumented: 3,
};

/// One `COVERS` edge, with the `details` blob `coverage_ingest` writes.
fn covers(conn: &Connection, run: &str, symbol: &str, file: &str, hash: &str, degree: Degree) {
    let Degree {
        name: degree,
        covered,
        instrumented,
    } = degree;
    let assertion_id = format!("a-{run}-COVERS-{symbol}");
    conn.execute(
        "INSERT OR IGNORE INTO assertion (assertion_id, repo_id, source_entity_id, relation,
                                          target_entity_id)
         VALUES (?1, 'repo', ?2, ?3, ?4)",
        rusqlite::params![assertion_id, run, Relation::Covers.as_str(), symbol],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO observation (assertion_id, extractor_run_id, evidence_source_type,
                                  directness, extractor_id, extractor_version,
                                  file_path, start_line, end_line, content_hash, details,
                                  created_at)
         VALUES (?1, 1, 'TEST_COVERAGE', 'INFERRED', 'coverage', '1.0.0', ?2, 1, 2, ?3, ?4,
                 '2026-08-01T00:00:00Z')",
        rusqlite::params![
            assertion_id,
            file,
            hash,
            format!(
                "{{\"coverage\":\"{degree}\",\"covered_lines\":{covered},\
                  \"instrumented_lines\":{instrumented}}}"
            )
        ],
    )
    .unwrap();
}

/// A repository with three source files and no coverage of any kind.
///
/// `src/a.ts`  alpha, beta
/// `src/b.ts`  gamma
/// `src/untested.ts`  delta
fn uncovered_repository() -> Connection {
    let conn = open_in_memory().unwrap();
    migrate(&conn).unwrap();
    conn.execute(
        "INSERT INTO repository (repo_id, project_id, root_path, created_at)
         VALUES ('repo', 'project', '/repo', 'now')",
        [],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO repository_state (state_id, repo_id, kind, content_merkle, created_at)
         VALUES (?1, 'repo', 'working-tree', 'merkle', 'now')",
        rusqlite::params![STATE],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO extractor_run (run_id, repo_id, state_id, extractor_id, extractor_version,
                                    started_at, status)
         VALUES (1, 'repo', ?1, 'coverage', '1.0.0', 'now', 'complete')",
        rusqlite::params![STATE],
    )
    .unwrap();

    entity(
        &conn, "S_alpha", "function", "alpha", "src/a.ts", 1, FRESH_A,
    );
    entity(&conn, "S_beta", "method", "beta", "src/a.ts", 10, FRESH_A);
    entity(
        &conn, "S_gamma", "function", "gamma", "src/b.ts", 1, FRESH_B,
    );
    entity(
        &conn,
        "S_delta",
        "function",
        "delta",
        "src/untested.ts",
        1,
        "hash-u",
    );
    // A module is not a symbol, so it must never appear as a gap: a coverage edge to a file
    // would say "the suite covers this file", a different and weaker claim.
    entity(&conn, "M_a", "module", "a", "src/a.ts", 1, FRESH_A);
    conn
}

/// The same repository after one report was ingested.
///
/// `alpha` fully covered, `beta` partial, `gamma` in a measured file with no covered line,
/// `delta` in a file the report never named.
fn covered_repository() -> Connection {
    let conn = uncovered_repository();
    entity(
        &conn,
        "cov_1",
        EntityKind::CoverageRun.as_str(),
        "lcov.info",
        "coverage/lcov.info",
        1,
        FRESH_REPORT,
    );
    conn.execute(
        "UPDATE entity SET meta = ?1 WHERE entity_id = 'cov_1'",
        rusqlite::params!["{\"format\":\"lcov\",\"source_files_in_report\":2}"],
    )
    .unwrap();
    covers(&conn, "cov_1", "S_alpha", "src/a.ts", FRESH_A, FULLY);
    covers(&conn, "cov_1", "S_beta", "src/a.ts", FRESH_A, PARTLY);
    // `src/b.ts` is named by the report through gamma's sibling — there is no sibling here, so
    // the file is measured by way of a symbol that *did* get an edge. Give it one.
    entity(
        &conn,
        "S_epsilon",
        "function",
        "epsilon",
        "src/b.ts",
        20,
        FRESH_B,
    );
    covers(&conn, "cov_1", "S_epsilon", "src/b.ts", FRESH_B, FULLY);
    conn
}

// ---- state one: the question is unanswerable -------------------------------------------------

/// The failure this whole slice exists to prevent.
#[test]
fn a_repository_with_no_coverage_reports_no_gaps_at_all() {
    let conn = uncovered_repository();
    let report = gaps(&conn, &query(100), &everything_fresh()).unwrap();

    assert_eq!(report.coverage, CoverageEvidence::Absent);
    assert!(!report.coverage.is_answerable());
    assert!(
        report.results.is_empty(),
        "a repository that never ran `nerve coverage` must not have every symbol listed as a gap"
    );
    assert_eq!(report.results_total, 0);
    assert!(
        report.totals.is_none(),
        "a tally that could not be computed must not be reported as a row of zeroes"
    );
    assert!(report.runs.is_empty());
    // The symbols are still counted, so the message can say how much is unanswered.
    assert_eq!(report.symbols_in_scope, 4);
    assert!(!report.truncated);
}

// ---- state two: coverage was ingested --------------------------------------------------------

#[test]
fn an_ingested_run_makes_every_state_visible_and_distinct() {
    let conn = covered_repository();
    let report = gaps(&conn, &query(100), &everything_fresh()).unwrap();

    assert_eq!(report.coverage, CoverageEvidence::Present);
    let totals = report.totals.expect("coverage exists, so a tally exists");
    assert_eq!(totals.covered, 2, "alpha and epsilon");
    assert_eq!(totals.partial, 1, "beta");
    assert_eq!(totals.uncovered, 1, "gamma, in a file the report measured");
    assert_eq!(totals.unmeasured, 1, "delta, in a file it never named");
    assert_eq!(totals.gaps(), 2);
    assert_eq!(totals.stale, 0);
    assert_eq!(totals.measured_files, 2);
    assert_eq!(totals.stale_files, 0);
    assert_eq!(report.symbols_in_scope, 5);

    let rows: Vec<(&str, &str)> = report
        .results
        .iter()
        .map(|row| (row.entity.name.as_str(), row.state.as_str()))
        .collect();
    assert_eq!(rows, vec![("gamma", "uncovered"), ("delta", "unmeasured")]);

    // The answer names the runs it is relative to, and what they measured.
    assert_eq!(report.runs.len(), 1);
    assert_eq!(
        report.runs[0].report_path.as_deref(),
        Some("coverage/lcov.info")
    );
    assert_eq!(report.runs[0].freshness, Some(Freshness::Fresh));
    assert_eq!(report.runs[0].source_files_in_report, Some(2));
}

/// A module is not a symbol and can never be a coverage gap.
#[test]
fn only_symbols_are_ever_reported() {
    let conn = covered_repository();
    let report = gaps(&conn, &query(100), &everything_fresh()).unwrap();
    assert!(report.results.iter().all(|row| row.entity.kind != "module"));
    assert_eq!(report.symbols_in_scope, 5);
}

/// "The report measured this file and found nothing here" and "nothing measured this file" are
/// different answers, and the row says which.
#[test]
fn measured_absence_and_unmeasured_absence_are_not_the_same_answer() {
    let conn = covered_repository();
    let report = gaps(&conn, &query(100), &everything_fresh()).unwrap();
    let gamma = &report.results[0];
    let delta = &report.results[1];

    assert_eq!(gamma.state, SymbolCoverage::Uncovered);
    assert_eq!(
        gamma.coverage_freshness,
        Some(Freshness::Fresh),
        "a measured gap rests on evidence, and the evidence has a freshness"
    );

    assert_eq!(delta.state, SymbolCoverage::Unmeasured);
    assert_eq!(
        delta.coverage_freshness, None,
        "there is no evidence about this file to be fresh or stale about"
    );
    assert!(delta.covered_by.is_empty());
}

// ---- state three: the coverage is stale ------------------------------------------------------

/// A gap computed from stale coverage is labelled stale, not presented as current.
#[test]
fn a_gap_derived_from_stale_coverage_says_so() {
    let conn = covered_repository();
    // `src/a.ts` and `src/b.ts` have both been edited since the report was ingested.
    let stale = prober(&[
        ("src/a.ts", FileProbe::Hash("moved-on".into())),
        ("src/b.ts", FileProbe::Hash("moved-on-too".into())),
        ("src/untested.ts", FileProbe::Hash("hash-u".into())),
        ("coverage/lcov.info", FileProbe::Hash(FRESH_REPORT.into())),
    ]);
    let mut wanted = query(100);
    wanted.include_partial = true;
    let report = gaps(&conn, &wanted, &stale).unwrap();

    let totals = report.totals.expect("coverage exists");
    assert_eq!(totals.stale_files, 2);
    assert_eq!(
        totals.stale, 4,
        "alpha, beta, gamma and epsilon all rest on coverage that no longer matches its file"
    );

    let gamma = report
        .results
        .iter()
        .find(|row| row.entity.name == "gamma")
        .expect("the measured gap is still reported");
    assert_eq!(gamma.state, SymbolCoverage::Uncovered);
    assert_eq!(gamma.coverage_freshness, Some(Freshness::Stale));

    let beta = report
        .results
        .iter()
        .find(|row| row.entity.name == "beta")
        .expect("partial rows were requested");
    assert_eq!(beta.coverage_freshness, Some(Freshness::Stale));

    // The unmeasured symbol's file is untouched, and it still has no freshness to report.
    let delta = report
        .results
        .iter()
        .find(|row| row.entity.name == "delta")
        .expect("delta is a gap in every reading");
    assert_eq!(delta.coverage_freshness, None);
}

/// A deleted covered file is a distinct freshness value, not folded into "stale".
#[test]
fn a_missing_covered_file_keeps_its_own_freshness_value() {
    let conn = covered_repository();
    let gone = prober(&[
        ("src/b.ts", FileProbe::Hash(FRESH_B.into())),
        ("src/untested.ts", FileProbe::Hash("hash-u".into())),
        ("coverage/lcov.info", FileProbe::Hash(FRESH_REPORT.into())),
    ]);
    let mut wanted = query(100);
    wanted.include_partial = true;
    let report = gaps(&conn, &wanted, &gone).unwrap();
    let beta = report
        .results
        .iter()
        .find(|row| row.entity.name == "beta")
        .expect("partial rows were requested");
    assert_eq!(beta.coverage_freshness, Some(Freshness::FileMissing));
}

/// The report file itself can go stale independently of the code it measured.
#[test]
fn the_report_carries_its_own_freshness() {
    let conn = covered_repository();
    let rewritten = prober(&[
        ("src/a.ts", FileProbe::Hash(FRESH_A.into())),
        ("src/b.ts", FileProbe::Hash(FRESH_B.into())),
        ("src/untested.ts", FileProbe::Hash("hash-u".into())),
        ("coverage/lcov.info", FileProbe::Hash("rerun".into())),
    ]);
    let report = gaps(&conn, &query(100), &rewritten).unwrap();
    assert_eq!(report.runs[0].freshness, Some(Freshness::Stale));
    assert_eq!(
        report.totals.expect("coverage exists").stale,
        0,
        "a re-run report does not make the code's coverage stale; only editing the code does"
    );
}

// ---- state four: partial ---------------------------------------------------------------------

/// `partial` is surfaced, never rounded to covered or uncovered.
#[test]
fn partial_is_a_reported_value_and_is_not_a_gap() {
    let conn = covered_repository();

    let default = gaps(&conn, &query(100), &everything_fresh()).unwrap();
    assert!(
        default
            .results
            .iter()
            .all(|row| row.state != SymbolCoverage::Partial),
        "a partially covered symbol is not a gap"
    );
    assert_eq!(
        default.totals.expect("coverage exists").partial,
        1,
        "and it is counted whether or not it is listed"
    );

    let mut wanted = query(100);
    wanted.include_partial = true;
    let widened = gaps(&conn, &wanted, &everything_fresh()).unwrap();
    let beta = widened
        .results
        .iter()
        .find(|row| row.entity.name == "beta")
        .expect("beta is partial");
    assert_eq!(beta.state, SymbolCoverage::Partial);
    assert_eq!(beta.covered_lines, Some(1));
    assert_eq!(beta.instrumented_lines, Some(3));
    assert_eq!(beta.covered_by, vec!["coverage/lcov.info".to_string()]);
}

// ---- filters, truncation, determinism ---------------------------------------------------------

#[test]
fn truncation_is_reported_rather_than_silent() {
    let conn = covered_repository();

    let full = gaps(&conn, &query(100), &everything_fresh()).unwrap();
    assert_eq!(full.results.len(), 2);
    assert!(!full.truncated);

    let capped = gaps(&conn, &query(1), &everything_fresh()).unwrap();
    assert_eq!(capped.results.len(), 1);
    assert!(capped.truncated);
    assert_eq!(
        capped.results_total, 2,
        "the count of what matched is exact even when the rows are cut"
    );
    assert_eq!(
        capped.totals.expect("coverage exists").gaps(),
        2,
        "and so is the tally"
    );
}

#[test]
fn a_path_scope_matches_on_a_directory_boundary_and_never_as_a_pattern() {
    let conn = covered_repository();
    let scoped = |prefix: &str| {
        let mut wanted = query(100);
        wanted.path_prefix = Some(prefix.to_string());
        wanted.include_partial = true;
        gaps(&conn, &wanted, &everything_fresh()).unwrap()
    };

    assert_eq!(scoped("src").symbols_in_scope, 5);
    assert_eq!(scoped("src/").symbols_in_scope, 5);
    assert_eq!(scoped("src/untested.ts").symbols_in_scope, 1);
    assert_eq!(scoped("src/untested.ts").results.len(), 1);
    // A prefix that is not a path boundary matches nothing rather than everything under `src`.
    assert_eq!(scoped("sr").symbols_in_scope, 0);
    // Wildcards are text, not syntax.
    assert_eq!(scoped("%").symbols_in_scope, 0);
    assert_eq!(scoped("src/%").symbols_in_scope, 0);
}

#[test]
fn a_kind_filter_narrows_to_one_symbol_kind() {
    let conn = covered_repository();
    let mut wanted = query(100);
    wanted.kind = Some(EntityKind::Method);
    wanted.include_partial = true;
    let report = gaps(&conn, &wanted, &everything_fresh()).unwrap();
    assert_eq!(report.symbols_in_scope, 1);
    assert_eq!(report.results.len(), 1);
    assert_eq!(report.results[0].entity.name, "beta");
}

/// Identical inputs produce an identical answer, row order included.
#[test]
fn the_same_repository_yields_the_same_answer_every_time() {
    let conn = covered_repository();
    let mut wanted = query(100);
    wanted.include_partial = true;
    let first = gaps(&conn, &wanted, &everything_fresh()).unwrap();
    for _ in 0..8 {
        assert_eq!(gaps(&conn, &wanted, &everything_fresh()).unwrap(), first);
    }
}

/// Each distinct file is re-hashed once, however many symbols quote it.
#[test]
fn freshness_probes_each_file_once() {
    let conn = covered_repository();
    let report = gaps(&conn, &query(100), &everything_fresh()).unwrap();
    assert_eq!(
        report.files_probed, 3,
        "the report itself plus the two files coverage measured; the unmeasured file is \
         never probed because there is no recorded hash to compare it against"
    );
}

/// Two runs naming one symbol: the positive observation wins, and both reports are named.
#[test]
fn a_symbol_covered_by_two_runs_reports_both_and_takes_the_stronger_reading() {
    let conn = covered_repository();
    entity(
        &conn,
        "cov_2",
        EntityKind::CoverageRun.as_str(),
        "lcov.info",
        "coverage/second.info",
        1,
        "hash-second",
    );
    // The second run only entered `beta`; the first ran it through.
    covers(&conn, "cov_2", "S_beta", "src/a.ts", FRESH_A, PARTLY);
    conn.execute(
        "UPDATE observation SET details = '{\"coverage\":\"covered\",\"covered_lines\":3,\
          \"instrumented_lines\":3}' WHERE assertion_id = 'a-cov_1-COVERS-S_beta'",
        [],
    )
    .unwrap();

    let mut wanted = query(100);
    wanted.include_partial = true;
    let report = gaps(&conn, &wanted, &everything_fresh()).unwrap();
    assert_eq!(report.runs.len(), 2);
    assert!(
        report.results.iter().all(|row| row.entity.name != "beta"),
        "one run executed every instrumented line, so the symbol is covered, not partial"
    );
    assert_eq!(report.totals.expect("coverage exists").covered, 3);
}

/// A run that covered nothing still answers the question — it just answers "nothing measured".
#[test]
fn a_run_with_no_edges_is_still_evidence_that_coverage_was_ingested() {
    let conn = uncovered_repository();
    entity(
        &conn,
        "cov_empty",
        EntityKind::CoverageRun.as_str(),
        "lcov.info",
        "coverage/lcov.info",
        1,
        FRESH_REPORT,
    );
    let report = gaps(&conn, &query(100), &everything_fresh()).unwrap();

    assert_eq!(report.coverage, CoverageEvidence::Present);
    let totals = report
        .totals
        .expect("a run exists, so the question is answerable");
    assert_eq!(totals.unmeasured, 4);
    assert_eq!(
        totals.uncovered, 0,
        "nothing was measured, so nothing is a measured gap"
    );
    assert_eq!(totals.measured_files, 0);
    assert_eq!(report.results.len(), 4);
}
