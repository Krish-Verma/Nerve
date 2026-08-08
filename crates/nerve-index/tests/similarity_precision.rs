//! Measured precision for the `nerve-line-multiset` rename matcher over `fixtures/history-similar`.
//!
//! ADR-0003 answers *"how confident are we?"* with a matcher's measured precision on a fixture
//! corpus rather than with a per-row number, and this file is that measurement. It is a **gate**:
//! false positives must be zero, and the test fails if a pair the oracle labels `not_rename` is
//! admitted.
//!
//! # Two tables, never one
//!
//! Exact-content precision and similar-content precision are reported **separately** and are never
//! summed or averaged, on exactly the terms `py-framework` and `ts-js-framework` are never summed.
//! They rest on different evidence: one says *these two paths named the same bytes*, the other says
//! *a named method measured how much two different blobs share*. A combined number would describe
//! neither claim.
//!
//! # Where the expected answers come from
//!
//! `fixtures/history-similar/ground_truth.json` is **hand-written** and predates the matcher. It is
//! not produced by `scripts/make_history_fixtures.sh` and it is never produced by running Nerve — a
//! corpus graded by the thing it measures would make this file Nerve agreeing with itself. The
//! generator writes one file out of another, so *rename* and *not a rename* are properties of the
//! construction, and the expected numerator and denominator of every pair were computed by hand and
//! are restated in the fixture's README.
//!
//! # Recall is reported, not optimised
//!
//! The corpus contains a genuine move with twelve of twenty lines rewritten. It measures `8/20` and
//! the threshold misses it. That is a false negative, this file **asserts that it is reported as
//! one**, and it is the price of a gate set for zero false positives. Buying it back by lowering
//! the threshold until the licence-header pair passed would be trading the gate for the number.
//!
//! Run `cargo test -p nerve-index --test similarity_precision -- --nocapture` to see both tables.

mod common;

use std::collections::{BTreeMap, BTreeSet};

use nerve_core::vocab::{
    RenameAmbiguity, RenameAnalysisCompleteness, RenameEvidence, SimilarityUnmeasured,
};
use nerve_index::history::{ingest_history, HistoryOptions};
use nerve_index::similarity::{
    SimilarityLimits, MATCHER_ID, MATCHER_VERSION, SIMILARITY_THRESHOLD_DENOMINATOR,
    SIMILARITY_THRESHOLD_NUMERATOR,
};

const FIXTURE: &str = "history-similar";

/// One `git_rename_hypothesis` row, read back whole so nothing is asserted by inference.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Hypothesis {
    commit_oid: String,
    from_path: String,
    to_path: String,
    evidence: String,
    from_blob_oid: String,
    to_blob_oid: String,
    matcher_id: String,
    matcher_version: String,
    numerator: Option<i64>,
    denominator: Option<i64>,
    ambiguity: String,
}

/// One `git_rename_analysis` row.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Analysis {
    commit_oid: String,
    matcher_id: String,
    matcher_version: String,
    threshold_numerator: i64,
    threshold_denominator: i64,
    deletions: i64,
    additions: i64,
    considered: i64,
    measured: i64,
    completeness: String,
    unmeasured: BTreeMap<String, i64>,
}

fn ground_truth() -> serde_json::Value {
    let path = common::named_fixture_root(FIXTURE).join("ground_truth.json");
    let text = std::fs::read_to_string(&path)
        .unwrap_or_else(|err| panic!("{} is unreadable: {err}", path.display()));
    serde_json::from_str(&text)
        .unwrap_or_else(|err| panic!("{} is not valid JSON: {err}", path.display()))
}

fn hypotheses(conn: &nerve_store::Connection) -> Vec<Hypothesis> {
    let mut stmt = conn
        .prepare(
            "SELECT commit_oid, from_path, to_path, evidence, from_blob_oid, to_blob_oid,
                    matcher_id, matcher_version, match_numerator, match_denominator, ambiguity
               FROM git_rename_hypothesis
              ORDER BY from_path, to_path",
        )
        .unwrap();
    let rows = stmt
        .query_map([], |row| {
            Ok(Hypothesis {
                commit_oid: row.get(0)?,
                from_path: row.get(1)?,
                to_path: row.get(2)?,
                evidence: row.get(3)?,
                from_blob_oid: row.get(4)?,
                to_blob_oid: row.get(5)?,
                matcher_id: row.get(6)?,
                matcher_version: row.get(7)?,
                numerator: row.get(8)?,
                denominator: row.get(9)?,
                ambiguity: row.get(10)?,
            })
        })
        .unwrap();
    rows.map(Result::unwrap).collect()
}

fn analyses(conn: &nerve_store::Connection) -> BTreeMap<String, Analysis> {
    let mut stmt = conn
        .prepare(
            "SELECT commit_oid, matcher_id, matcher_version, threshold_numerator,
                    threshold_denominator, deletions_considered, additions_considered,
                    pairs_considered, pairs_measured, completeness, unmeasured
               FROM git_rename_analysis",
        )
        .unwrap();
    let rows = stmt
        .query_map([], |row| {
            let unmeasured: String = row.get(10)?;
            Ok(Analysis {
                commit_oid: row.get(0)?,
                matcher_id: row.get(1)?,
                matcher_version: row.get(2)?,
                threshold_numerator: row.get(3)?,
                threshold_denominator: row.get(4)?,
                deletions: row.get(5)?,
                additions: row.get(6)?,
                considered: row.get(7)?,
                measured: row.get(8)?,
                completeness: row.get(9)?,
                unmeasured: serde_json::from_str(&unmeasured).expect("a JSON object"),
            })
        })
        .unwrap();
    rows.map(Result::unwrap)
        .map(|row| (row.commit_oid.clone(), row))
        .collect()
}

/// `commit summary -> commit oid`, so the hand-written oracle can name commits without carrying an
/// object id it could only have obtained by running the generator.
fn commits_by_summary(conn: &nerve_store::Connection, repo_id: &str) -> BTreeMap<String, String> {
    let mut out = BTreeMap::new();
    for row in nerve_store::commit_log(conn, repo_id, usize::MAX, 0).unwrap() {
        assert!(
            out.insert(row.summary.clone(), row.commit_oid.clone())
                .is_none(),
            "two commits share the summary {:?}; the oracle cannot name one of them",
            row.summary
        );
    }
    out
}

/// **The measurement.** Both tables, the gate, and the anti-vacuity checks that stop a matcher
/// which returned nothing from passing.
#[test]
fn measured_precision_of_the_similarity_matcher() {
    let truth = ground_truth();
    let (_dir, root) = common::history_fixture(FIXTURE);
    let outcome = ingest_history(&root, &HistoryOptions::default()).expect("the sync must succeed");

    // Two counters, never one. `renames_written` is the exact matcher's and
    // `similar_renames_written` is this one's, and the outcome keeps them apart.
    assert!(
        outcome.renames_written > 0 && outcome.similar_renames_written > 0,
        "both matchers must have produced rows, or one of the two tables below is vacuous"
    );
    assert_eq!(
        outcome.rename_analyses_written, outcome.commits_recorded,
        "one analysis row per commit, including the commits with no candidate at all"
    );

    let conn = common::open_db(&root);
    let repo_id = nerve_store::repository(&conn).unwrap().unwrap().repo_id;
    let by_summary = commits_by_summary(&conn, &repo_id);
    let rows = hypotheses(&conn);
    let analysis = analyses(&conn);

    let similar: Vec<&Hypothesis> = rows
        .iter()
        .filter(|row| row.evidence == RenameEvidence::SimilarContent.as_str())
        .collect();
    let exact: Vec<&Hypothesis> = rows
        .iter()
        .filter(|row| row.evidence == RenameEvidence::ExactContent.as_str())
        .collect();
    assert_eq!(
        similar.len() + exact.len(),
        rows.len(),
        "a row carried an evidence value that is neither"
    );

    // ---- the similar-content table -----------------------------------------------------------
    let expected_pairs = truth["similar_content_pairs"]
        .as_array()
        .expect("the oracle lists its pairs");
    let mut admitted_by_key: BTreeMap<(String, String), &Hypothesis> = BTreeMap::new();
    for row in &similar {
        assert!(
            admitted_by_key
                .insert((row.from_path.clone(), row.to_path.clone()), row)
                .is_none(),
            "{} -> {} was recorded twice",
            row.from_path,
            row.to_path
        );
    }

    let mut true_positives: Vec<String> = Vec::new();
    let mut false_positives: Vec<String> = Vec::new();
    let mut false_negatives: Vec<String> = Vec::new();
    let mut rejected_correctly: Vec<String> = Vec::new();
    let mut unmeasurable: Vec<String> = Vec::new();
    let mut ambiguity_seen: BTreeMap<String, usize> = BTreeMap::new();
    let mut known: BTreeSet<(String, String)> = BTreeSet::new();

    for pair in expected_pairs {
        let from = pair["from_path"].as_str().expect("a from_path").to_string();
        let to = pair["to_path"].as_str().expect("a to_path").to_string();
        let verdict = pair["verdict"].as_str().expect("a verdict");
        let label = pair["commit_label"].as_str().expect("a commit label");
        let name = format!("{label} {from} -> {to}");
        known.insert((from.clone(), to.clone()));

        // What the matcher actually did, read out of the database rather than out of the oracle.
        let produced = admitted_by_key.get(&(from.clone(), to.clone())).copied();

        if let Some(row) = produced {
            // Every admitted row carries its method, both blob oids and the two integers. A row
            // that omitted any of them would be a percentage from nowhere.
            assert_eq!(row.matcher_id, MATCHER_ID, "{name}");
            assert_eq!(row.matcher_version, MATCHER_VERSION, "{name}");
            assert_ne!(
                row.from_blob_oid, row.to_blob_oid,
                "{name}: a similar-content row must name two different blobs"
            );
            assert_eq!(
                row.numerator,
                pair["numerator"].as_i64(),
                "{name}: measured numerator disagrees with the hand-computed one"
            );
            assert_eq!(
                row.denominator,
                pair["denominator"].as_i64(),
                "{name}: measured denominator disagrees with the hand-computed one"
            );
            assert_eq!(
                row.commit_oid,
                by_summary[commit_summary(&truth, label)],
                "{name}: recorded against the wrong commit"
            );
            if let Some(expected) = pair["ambiguity"].as_str() {
                assert_eq!(row.ambiguity, expected, "{name}");
            }
            *ambiguity_seen.entry(row.ambiguity.clone()).or_insert(0) += 1;
        }

        match (verdict, produced.is_some()) {
            ("rename", true) => true_positives.push(name),
            ("rename", false) => false_negatives.push(name),
            ("not_rename", true) => false_positives.push(name),
            ("not_rename", false) => rejected_correctly.push(name),
            ("unmeasurable", false) => unmeasurable.push(name),
            ("unmeasurable", true) => false_positives.push(format!("{name} (unmeasurable)")),
            (other, _) => panic!("{name}: unknown verdict {other:?}"),
        }
    }

    // A row for a pair the oracle never heard of is a false positive too, and the loop above cannot
    // see it. Without this the gate would only cover pairs somebody remembered to write down.
    let unknown: Vec<String> = similar
        .iter()
        .filter(|row| !known.contains(&(row.from_path.clone(), row.to_path.clone())))
        .map(|row| format!("{} -> {}", row.from_path, row.to_path))
        .collect();

    // ---- the exact-content table ---------------------------------------------------------------
    let expected_exact = truth["exact_content_pairs"]
        .as_array()
        .expect("the oracle lists its exact pairs");
    let mut exact_true_positives = 0usize;
    for pair in expected_exact {
        let from = pair["from_path"].as_str().unwrap();
        let to = pair["to_path"].as_str().unwrap();
        let row = exact
            .iter()
            .find(|row| row.from_path == from && row.to_path == to)
            .unwrap_or_else(|| panic!("{from} -> {to} must be an exact-content hypothesis"));
        assert_eq!(
            row.from_blob_oid, row.to_blob_oid,
            "{from} -> {to}: the identity of the two oids *is* the evidence"
        );
        assert_eq!(
            row.numerator, None,
            "{from} -> {to}: an exact match counts nothing, so it carries no measurement"
        );
        assert_eq!(row.denominator, None);
        assert_eq!(row.matcher_id, "git-blob-oid", "{from} -> {to}");
        assert_eq!(row.ambiguity, pair["ambiguity"].as_str().unwrap());
        exact_true_positives += 1;
    }
    let exact_unknown: Vec<String> = exact
        .iter()
        .filter(|row| {
            !expected_exact.iter().any(|pair| {
                pair["from_path"].as_str() == Some(row.from_path.as_str())
                    && pair["to_path"].as_str() == Some(row.to_path.as_str())
            })
        })
        .map(|row| format!("{} -> {}", row.from_path, row.to_path))
        .collect();

    // ---- the report ----------------------------------------------------------------------------
    println!("\n  measured precision, fixtures/{FIXTURE}");
    println!("  ---------------------------------------------------------------");
    println!(
        "  similar-content  matcher {MATCHER_ID} v{MATCHER_VERSION}, threshold \
         {SIMILARITY_THRESHOLD_NUMERATOR}/{SIMILARITY_THRESHOLD_DENOMINATOR}"
    );
    println!("    candidate pairs      {}", expected_pairs.len());
    println!("    admitted             {}", similar.len());
    println!("    true positives       {}", true_positives.len());
    println!("    false positives      {}", false_positives.len());
    println!("    correctly rejected   {}", rejected_correctly.len());
    println!("    false negatives      {}", false_negatives.len());
    for miss in &false_negatives {
        println!("      - {miss}");
    }
    println!("    unmeasurable         {}", unmeasurable.len());
    for case in &unmeasurable {
        println!("      - {case}");
    }
    println!(
        "    precision            {}/{}",
        true_positives.len(),
        similar.len()
    );
    println!(
        "    recall               {}/{}   (over measurable ground-truth renames)",
        true_positives.len(),
        true_positives.len() + false_negatives.len()
    );
    println!("    ambiguity            {ambiguity_seen:?}");
    println!("  ---------------------------------------------------------------");
    println!("  exact-content    matcher git-blob-oid v1, no threshold and no measurement");
    println!("    candidate pairs      {}", expected_exact.len());
    println!("    admitted             {}", exact.len());
    println!("    true positives       {exact_true_positives}");
    println!("    false positives      {}", exact_unknown.len());
    println!("  ---------------------------------------------------------------");
    println!("  the two tables are never summed and never averaged: they rest on different");
    println!("  evidence, and a combined number would describe neither claim.\n");

    // ---- the gate -------------------------------------------------------------------------------
    assert!(
        false_positives.is_empty(),
        "false positives must be zero, and these were admitted: {false_positives:?}"
    );
    assert!(
        unknown.is_empty(),
        "similarity rows for pairs the oracle does not list: {unknown:?}"
    );
    assert!(
        exact_unknown.is_empty(),
        "exact-content rows for pairs the oracle does not list: {exact_unknown:?}"
    );

    // The false negative is **reported**, not hidden. Asserting its presence is what stops a future
    // change from quietly lowering the threshold to make the recall number look better.
    assert_eq!(
        false_negatives.len(),
        truth["totals"]["expected_false_negatives"]
            .as_u64()
            .expect("the oracle states how many it expects") as usize,
        "the published false-negative count changed: {false_negatives:?}"
    );
    assert!(
        false_negatives
            .iter()
            .any(|miss| miss.contains("mod/beta.txt")),
        "the large-edit rename must be reported as a false negative, not dropped"
    );

    // ---- anti-vacuity ---------------------------------------------------------------------------
    // A matcher that returned nothing at all, or a corpus with only one kind of case, must not be
    // able to pass. Each of the three populations has to be non-empty.
    assert!(
        !true_positives.is_empty(),
        "no pair was admitted; a matcher returning nothing would pass this file"
    );
    assert!(
        !rejected_correctly.is_empty(),
        "no pair was measured and rejected; the threshold is untested"
    );
    assert!(
        !unmeasurable.is_empty(),
        "no pair was unmeasurable; the unmeasured vocabulary is untested"
    );
    assert_eq!(
        similar.len(),
        truth["totals"]["admitted"].as_u64().unwrap() as usize
    );
    assert_eq!(
        expected_pairs.len(),
        truth["totals"]["similar_content_pairs"].as_u64().unwrap() as usize,
        "the oracle's own totals disagree with its own list"
    );
    assert!(
        ambiguity_seen.contains_key(RenameAmbiguity::Unique.as_str())
            && ambiguity_seen.contains_key(RenameAmbiguity::ManyTo.as_str())
            && ambiguity_seen.contains_key(RenameAmbiguity::ManyFrom.as_str()),
        "the corpus must exercise more than one ambiguity shape: {ambiguity_seen:?}"
    );

    // ---- the per-commit account -----------------------------------------------------------------
    for commit in truth["commits"].as_array().expect("the oracle's commits") {
        let label = commit["label"].as_str().unwrap();
        let oid = &by_summary[commit["summary"].as_str().unwrap()];
        let row = analysis
            .get(oid)
            .unwrap_or_else(|| panic!("{label} has no git_rename_analysis row"));
        assert_eq!(row.matcher_id, MATCHER_ID, "{label}");
        assert_eq!(row.matcher_version, MATCHER_VERSION, "{label}");
        assert_eq!(
            row.threshold_numerator, SIMILARITY_THRESHOLD_NUMERATOR,
            "{label}"
        );
        assert_eq!(
            row.threshold_denominator, SIMILARITY_THRESHOLD_DENOMINATOR,
            "{label}"
        );
        assert_eq!(
            row.deletions,
            commit["deletions"].as_i64().unwrap(),
            "{label}"
        );
        assert_eq!(
            row.additions,
            commit["additions"].as_i64().unwrap(),
            "{label}"
        );
        assert_eq!(
            row.considered,
            commit["pairs_considered"].as_i64().unwrap(),
            "{label}"
        );
        let measured = commit["pairs_measured"]
            .as_i64()
            .unwrap_or_else(|| commit["pairs_considered"].as_i64().unwrap());
        assert_eq!(row.measured, measured, "{label}");
        assert_eq!(
            row.completeness,
            commit["completeness"].as_str().unwrap(),
            "{label}"
        );
        let expected_unmeasured: BTreeMap<String, i64> = commit["unmeasured"]
            .as_object()
            .map(|map| {
                map.iter()
                    .map(|(key, value)| (key.clone(), value.as_i64().unwrap()))
                    .collect()
            })
            .unwrap_or_default();
        assert_eq!(row.unmeasured, expected_unmeasured, "{label}");
        // The invariant that makes the row checkable by hand.
        assert_eq!(
            row.unmeasured.values().sum::<i64>(),
            row.considered - row.measured,
            "{label}: the unmeasured reasons must account for every unmeasured pair"
        );
    }
    assert_eq!(
        analysis.len(),
        truth["commits"].as_array().unwrap().len(),
        "one analysis row per commit, no more and no fewer"
    );

    // Every reason the corpus produces is a value of the closed vocabulary, never free text.
    let reasons: BTreeSet<&String> = analysis
        .values()
        .flat_map(|row| row.unmeasured.keys())
        .collect();
    for reason in &reasons {
        assert!(
            SimilarityUnmeasured::ALL
                .iter()
                .any(|value| value.as_str() == reason.as_str()),
            "{reason} is not a SimilarityUnmeasured value"
        );
    }
    assert_eq!(
        reasons.len(),
        2,
        "blob-too-small and blob-binary: {reasons:?}"
    );
}

/// The commit summary the oracle gives a label, so a lookup failure names the label rather than
/// panicking on an index.
fn commit_summary<'a>(truth: &'a serde_json::Value, label: &str) -> &'a str {
    truth["commits"]
        .as_array()
        .expect("the oracle's commits")
        .iter()
        .find(|commit| commit["label"].as_str() == Some(label))
        .unwrap_or_else(|| panic!("the oracle has no commit labelled {label}"))["summary"]
        .as_str()
        .expect("a summary")
}

/// **Every bound, exercised end to end through a tight `SimilarityLimits`.**
///
/// A bound that can only be reached from a unit test is a bound whose wiring is untested: Slice
/// 12c-i-b was a corrective slice for exactly that. Each case below syncs the real fixture with one
/// limit tightened and asserts that the affected commit records
/// [`RenameAnalysisCompleteness::RefusedBound`] with **no** similarity row — and that the
/// exact-content row for `empty/a.txt -> empty/b.txt` survives untouched, because a similarity
/// refusal must never disturb the other matcher's evidence.
#[test]
fn every_bound_refuses_end_to_end_and_leaves_the_exact_rows_alone() {
    let base = SimilarityLimits::default();
    let cases: [(&str, SimilarityLimits, &str); 5] = [
        (
            "deletions",
            SimilarityLimits {
                max_deletions: 1,
                ..base
            },
            // c7 deletes two paths.
            "c7: delete fan/left.txt and fan/right.txt and add fan/merged.txt",
        ),
        (
            "additions",
            SimilarityLimits {
                max_additions: 1,
                ..base
            },
            "c6: delete fan/one.txt and add two files that each keep eighteen of its lines",
        ),
        (
            "pairs",
            SimilarityLimits {
                max_pairs: 1,
                ..base
            },
            "c6: delete fan/one.txt and add two files that each keep eighteen of its lines",
        ),
        (
            "lines",
            SimilarityLimits {
                max_lines: 5,
                ..base
            },
            "c1: move mod/alpha.txt to mod/alpha-renamed.txt with two lines edited",
        ),
        (
            "rows per commit",
            SimilarityLimits {
                max_rows_per_commit: 1,
                ..base
            },
            "c6: delete fan/one.txt and add two files that each keep eighteen of its lines",
        ),
    ];

    for (name, limits, summary) in cases {
        let (_dir, root) = common::history_fixture(FIXTURE);
        ingest_history(
            &root,
            &HistoryOptions {
                similarity: limits,
                ..HistoryOptions::default()
            },
        )
        .expect("the sync must succeed");
        let conn = common::open_db(&root);
        let repo_id = nerve_store::repository(&conn).unwrap().unwrap().repo_id;
        let oid = commits_by_summary(&conn, &repo_id)[summary].clone();
        let row = &analyses(&conn)[&oid];
        assert_eq!(
            row.completeness,
            RenameAnalysisCompleteness::RefusedBound.as_str(),
            "the {name} bound did not refuse"
        );
        assert!(
            hypotheses(&conn).iter().all(|row| {
                row.commit_oid != oid || row.evidence == RenameEvidence::ExactContent.as_str()
            }),
            "the {name} bound refused and still wrote a similarity row"
        );
        assert!(
            hypotheses(&conn)
                .iter()
                .any(|row| row.from_path == "empty/a.txt"
                    && row.evidence == RenameEvidence::ExactContent.as_str()),
            "the {name} bound disturbed the exact matcher's evidence"
        );
    }
}

/// The two per-blob ceilings, end to end: over the byte bound and under the line floor are
/// **pair-level** reasons, so the commit is `partial` and the reason is counted rather than the set
/// being refused.
#[test]
fn the_per_blob_ceilings_are_reported_as_reasons_end_to_end() {
    let base = SimilarityLimits::default();
    for (limits, expected) in [
        (
            SimilarityLimits {
                max_blob_bytes: 16,
                ..base
            },
            SimilarityUnmeasured::BlobTooLarge,
        ),
        (
            SimilarityLimits {
                min_lines: 21,
                ..base
            },
            SimilarityUnmeasured::BlobTooSmall,
        ),
    ] {
        let (_dir, root) = common::history_fixture(FIXTURE);
        ingest_history(
            &root,
            &HistoryOptions {
                similarity: limits,
                ..HistoryOptions::default()
            },
        )
        .expect("the sync must succeed");
        let conn = common::open_db(&root);
        let repo_id = nerve_store::repository(&conn).unwrap().unwrap().repo_id;
        let oid = commits_by_summary(&conn, &repo_id)
            ["c1: move mod/alpha.txt to mod/alpha-renamed.txt with two lines edited"]
            .clone();
        let row = &analyses(&conn)[&oid];
        assert_eq!(
            row.completeness,
            RenameAnalysisCompleteness::Partial.as_str(),
            "{expected} must leave the commit partial, not refused"
        );
        assert_eq!(
            row.unmeasured.get(expected.as_str()),
            Some(&1),
            "{expected} was not counted: {:?}",
            row.unmeasured
        );
        assert_eq!(row.measured, 0);
        assert_eq!(row.considered, 1);
    }
}
