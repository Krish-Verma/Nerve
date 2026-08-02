//! Measured precision for the `coverage` extractor over `fixtures/ts-coverage`.
//!
//! ADR-0003 answers "how confident are we?" with an extractor's measured precision on its fixture
//! corpus rather than with a per-row number. This file is that measurement for coverage, and it is
//! a **gate**: false positives and false negatives must both be zero, every recorded degree must
//! match the one a human read out of the report, and every symbol the report leaves uncovered must
//! have no edge at all.
//!
//! **This is a regression gate, not an accuracy claim.** One hand-built corpus of eight symbols
//! says nothing about how line-to-symbol mapping behaves on a real repository with decorators,
//! generated code, or a bundler's source maps in the way. What it does say is that the mapping
//! Slice 6b ships cannot change silently.
//!
//! Run `cargo test -p nerve-index --test coverage_precision -- --nocapture` to see the table.

mod common;

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use common::{copy_tree, named_fixture_root, open_db, TEST_PROJECT_ID};

const FIXTURE: &str = "ts-coverage";
const REPORT: &str = "coverage/lcov.info";

/// One `COVERS` edge as the database has it.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Edge {
    selector: String,
    degree: String,
    covered: i64,
    instrumented: i64,
    file_path: String,
    start_line: i64,
    end_line: i64,
}

fn expected() -> serde_json::Value {
    let path = named_fixture_root(FIXTURE).join("expected.json");
    let text = std::fs::read_to_string(&path)
        .unwrap_or_else(|err| panic!("{} is unreadable: {err}", path.display()));
    serde_json::from_str(&text)
        .unwrap_or_else(|err| panic!("{} is not valid JSON: {err}", path.display()))
}

/// `sym:<file>#<scope>.<name>`, the selector `expected.json` is written in.
fn selector(file_path: &str, scope_path: &str, name: &str) -> String {
    if scope_path.is_empty() {
        format!("sym:{file_path}#{name}")
    } else {
        format!("sym:{file_path}#{scope_path}.{name}")
    }
}

/// Every symbol in the indexed fixture, by selector.
fn symbols(conn: &nerve_store::Connection) -> BTreeMap<String, String> {
    let mut stmt = conn
        .prepare(
            "SELECT e.entity_id, e.name, e.scope_path, o.file_path
               FROM entity e
               JOIN occurrence o ON o.entity_id = e.entity_id
              WHERE e.kind IN ('function', 'method', 'class', 'interface')
              ORDER BY o.file_path, o.start_line",
        )
        .unwrap();
    let rows = stmt
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
            ))
        })
        .unwrap();
    let mut out = BTreeMap::new();
    for row in rows {
        let (entity_id, name, scope_path, file_path) = row.unwrap();
        let key = selector(&file_path, &scope_path, &name);
        assert!(
            out.insert(key.clone(), entity_id).is_none(),
            "{key} is ambiguous in the fixture; precision cannot be measured against it"
        );
    }
    out
}

fn edges(conn: &nerve_store::Connection) -> Vec<Edge> {
    let mut stmt = conn
        .prepare(
            "SELECT t.name, t.scope_path, o.file_path, o.start_line, o.end_line, o.details
               FROM assertion a
               JOIN entity t ON t.entity_id = a.target_entity_id
               JOIN observation o ON o.assertion_id = a.assertion_id
              WHERE a.relation = 'COVERS' AND o.extractor_id = 'coverage'
              ORDER BY o.file_path, o.start_line, t.name",
        )
        .unwrap();
    let rows = stmt
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, i64>(3)?,
                row.get::<_, i64>(4)?,
                row.get::<_, String>(5)?,
            ))
        })
        .unwrap();
    rows.map(|row| {
        let (name, scope_path, file_path, start_line, end_line, details) = row.unwrap();
        let details: serde_json::Value = serde_json::from_str(&details).unwrap();
        Edge {
            selector: selector(&file_path, &scope_path, &name),
            degree: details["coverage"].as_str().unwrap().to_string(),
            covered: details["covered_lines"].as_i64().unwrap(),
            instrumented: details["instrumented_lines"].as_i64().unwrap(),
            file_path,
            start_line,
            end_line,
        }
    })
    .collect()
}

/// The measurement, and the gate.
#[test]
fn measured_coverage_precision_meets_the_slice_6b_gates() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("repo");
    copy_tree(&named_fixture_root(FIXTURE), &root);
    nerve_index::init_with_project_id(&root, Some(TEST_PROJECT_ID)).unwrap();
    nerve_index::index_repository(&root).unwrap();
    let outcome = nerve_index::ingest_coverage(&root, Path::new(REPORT)).unwrap();

    let conn = open_db(&root);
    let known = symbols(&conn);
    let actual = edges(&conn);
    let ground_truth = expected();

    let mut problems: Vec<String> = Vec::new();

    // Every selector in the ground truth must name a symbol that actually exists, or the corpus
    // is measuring the implementation against a typo.
    let mut expected_edges: BTreeMap<String, serde_json::Value> = BTreeMap::new();
    for entry in ground_truth["covers"].as_array().unwrap() {
        let selector = entry["symbol"].as_str().unwrap().to_string();
        if !known.contains_key(&selector) {
            problems.push(format!(
                "expected.json names {selector}, which is not a symbol"
            ));
            continue;
        }
        expected_edges.insert(selector, entry.clone());
    }
    for entry in ground_truth["no_edge"].as_array().unwrap() {
        let selector = entry["symbol"].as_str().unwrap();
        if !known.contains_key(selector) {
            problems.push(format!(
                "expected.json names {selector}, which is not a symbol"
            ));
        }
    }

    let found: BTreeMap<String, Edge> = actual
        .iter()
        .map(|edge| (edge.selector.clone(), edge.clone()))
        .collect();
    assert_eq!(
        found.len(),
        actual.len(),
        "one symbol carries two COVERS edges"
    );

    // False negatives: an expected edge that is missing, or that disagrees about the degree.
    let mut false_negatives = 0usize;
    for (selector, entry) in &expected_edges {
        let Some(edge) = found.get(selector) else {
            false_negatives += 1;
            problems.push(format!("missing COVERS edge for {selector}"));
            continue;
        };
        for (field, expected_value, actual_value) in [
            (
                "degree",
                entry["degree"].as_str().unwrap().to_string(),
                edge.degree.clone(),
            ),
            (
                "covered",
                entry["covered"].as_i64().unwrap().to_string(),
                edge.covered.to_string(),
            ),
            (
                "instrumented",
                entry["instrumented"].as_i64().unwrap().to_string(),
                edge.instrumented.to_string(),
            ),
        ] {
            if expected_value != actual_value {
                false_negatives += 1;
                problems.push(format!(
                    "{selector}: {field} is {actual_value}, ground truth says {expected_value}"
                ));
            }
        }
        // The observation's line range must be the covered lines the ground truth lists.
        let lines: Vec<i64> = entry["lines"]
            .as_array()
            .unwrap()
            .iter()
            .map(|value| value.as_i64().unwrap())
            .collect();
        let covered_lines: Vec<i64> = lines.clone();
        let first_covered = covered_lines.first().copied().unwrap_or_default();
        if edge.start_line < first_covered {
            problems.push(format!(
                "{selector}: observation starts at line {}, before the first instrumented line {}",
                edge.start_line, first_covered
            ));
        }
        if edge.end_line > *covered_lines.last().unwrap() {
            problems.push(format!(
                "{selector}: observation ends at line {}, past the last instrumented line {}",
                edge.end_line,
                covered_lines.last().unwrap()
            ));
        }
    }

    // False positives: an edge for a symbol the ground truth says has none, or one for a symbol
    // the ground truth does not mention at all.
    let mut false_positives = 0usize;
    let forbidden: BTreeSet<String> = ground_truth["no_edge"]
        .as_array()
        .unwrap()
        .iter()
        .map(|entry| entry["symbol"].as_str().unwrap().to_string())
        .collect();
    for edge in &actual {
        if forbidden.contains(&edge.selector) {
            false_positives += 1;
            problems.push(format!(
                "{} has a COVERS edge; the report gives it no covered line",
                edge.selector
            ));
        } else if !expected_edges.contains_key(&edge.selector) {
            false_positives += 1;
            problems.push(format!("unexpected COVERS edge for {}", edge.selector));
        }
    }

    // Every symbol in the fixture is accounted for one way or the other, so a symbol that the
    // corpus simply forgot cannot pass as "no edge expected, none found".
    for selector in known.keys() {
        let expected_here = expected_edges.contains_key(selector);
        let forbidden_here = forbidden.contains(selector);
        if !expected_here && !forbidden_here {
            problems.push(format!(
                "{selector} is in the fixture but in neither `covers` nor `no_edge`"
            ));
        }
    }

    // The reported totals, which are what a user sees.
    let totals = &ground_truth["totals"];
    for (field, expected_value, actual_value) in [
        (
            "files_in_report",
            totals["files_in_report"].as_u64().unwrap() as usize,
            outcome.files_in_report,
        ),
        (
            "files_ingested",
            totals["files_ingested"].as_u64().unwrap() as usize,
            outcome.files_ingested,
        ),
        (
            "files_refused",
            totals["files_refused"].as_u64().unwrap() as usize,
            outcome.files_refused,
        ),
        (
            "symbols_covered",
            totals["symbols_covered"].as_u64().unwrap() as usize,
            outcome.symbols_covered,
        ),
        (
            "symbols_fully_covered",
            totals["symbols_fully_covered"].as_u64().unwrap() as usize,
            outcome.symbols_fully_covered,
        ),
        (
            "symbols_partially_covered",
            totals["symbols_partially_covered"].as_u64().unwrap() as usize,
            outcome.symbols_partially_covered,
        ),
        (
            "covered_lines",
            totals["covered_lines"].as_u64().unwrap() as usize,
            outcome.covered_lines,
        ),
        (
            "uncovered_lines",
            totals["uncovered_lines"].as_u64().unwrap() as usize,
            outcome.uncovered_lines,
        ),
        (
            "line_outside_any_symbol",
            totals["line_outside_any_symbol"].as_u64().unwrap() as usize,
            outcome.refused_count(nerve_index::coverage_ingest::form::LINE_OUTSIDE_ANY_SYMBOL),
        ),
    ] {
        if expected_value != actual_value {
            problems.push(format!(
                "totals.{field} is {actual_value}, ground truth says {expected_value}"
            ));
        }
    }

    let true_positives = expected_edges
        .len()
        .saturating_sub(false_negatives.min(expected_edges.len()));
    let precision = if actual.is_empty() {
        0.0
    } else {
        (actual.len() - false_positives.min(actual.len())) as f64 / actual.len() as f64
    };
    let recall = if expected_edges.is_empty() {
        0.0
    } else {
        true_positives as f64 / expected_edges.len() as f64
    };

    println!("\n=== coverage precision on fixtures/{FIXTURE} ===");
    println!("(a regression gate, not an accuracy claim: one hand-built corpus)\n");
    println!(
        "{:<44} {:<9} {:>8} {:>13}",
        "symbol", "degree", "covered", "instrumented"
    );
    for edge in &actual {
        println!(
            "{:<44} {:<9} {:>8} {:>13}",
            edge.selector, edge.degree, edge.covered, edge.instrumented
        );
    }
    for entry in ground_truth["no_edge"].as_array().unwrap() {
        println!(
            "{:<44} {:<9} {:>8} {:>13}",
            entry["symbol"].as_str().unwrap(),
            "(none)",
            0,
            "-"
        );
    }
    println!(
        "\nsymbols in fixture      {}\nedges expected          {}\nedges emitted           {}\n\
         false positives         {false_positives}\nfalse negatives         {false_negatives}\n\
         precision               {precision:.3}\nrecall                  {recall:.3}\n\
         lines unattributed      {}",
        known.len(),
        expected_edges.len(),
        actual.len(),
        outcome.refused_count(nerve_index::coverage_ingest::form::LINE_OUTSIDE_ANY_SYMBOL)
    );

    assert!(
        problems.is_empty(),
        "coverage precision gates failed ({} problem(s)):\n{}",
        problems.len(),
        problems
            .iter()
            .map(|problem| format!("  - {problem}"))
            .collect::<Vec<_>>()
            .join("\n")
    );
    assert_eq!(false_positives, 0);
    assert_eq!(false_negatives, 0);
    assert!((precision - 1.0).abs() < f64::EPSILON);
    assert!((recall - 1.0).abs() < f64::EPSILON);
}
