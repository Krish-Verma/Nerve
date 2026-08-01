//! Measured precision for `md-structural`'s supersession extraction over
//! `fixtures/md-supersession`.
//!
//! ADR-0003 answers "how confident are we?" with an extractor's measured precision on its fixture
//! corpus, not with a per-row number. This file is that measurement for `Document SUPERSEDES
//! Document`, and like `precision.rs` and `document_links.rs` it is a **gate rather than a
//! report**:
//!
//! - **false positives must be zero** — every `SUPERSEDES` edge in the database is declared in
//!   `expected.json`, and nothing else resolves;
//! - every declared edge must be present, so recall is measured rather than asserted;
//! - every declared edge must be stated in exactly the files the ground truth says, so that "one
//!   assertion, two observations" is asserted rather than assumed;
//! - every unresolved edge must be declared *with its reason*, from the closed vocabulary;
//! - every file declared **silent** must produce no supersession row at all — prose containing
//!   the word, a `Superseded` status with no target, and a field inside a fenced code block are
//!   all things Nerve declines to read as evidence, and declining is not the same as failing;
//! - the cycle and status-contradiction counters must equal the ground truth's own tally.
//!
//! **This is a regression gate, not an accuracy claim.** It measures one hand-built corpus of 26
//! files against ground truth written before the resolver existed. It says nothing about how
//! often Nerve is right on a repository nobody wrote for it, and it must not be quoted as if it
//! did. If a case here stops passing, the rule is what moves.
//!
//! Run `cargo test -p nerve-index --test document_supersession -- --nocapture` to see the table.

mod common;

use std::collections::{BTreeMap, BTreeSet};

use common::{indexed_named_fixture, open_db};
use nerve_index::docref::{outcome, reason};

const FIXTURE: &str = "md-supersession";

/// Every reason a supersession field may fail to name a document. Closed, and pinned here so
/// that `expected.json` cannot quietly introduce a sixth.
const REASONS: [&str; 5] = [
    reason::SUPERSEDES_TARGET_NOT_INDEXED,
    reason::SUPERSEDES_TARGET_AMBIGUOUS,
    reason::SUPERSEDES_SELF,
    reason::SUPERSEDES_UNPARSED,
    reason::REFUSED,
];

/// One `SUPERSEDES` observation, with the evidence a failure needs.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct Edge {
    from: String,
    to: String,
    from_label: String,
    to_label: String,
    target_kind: String,
    target_name: String,
    source_type: String,
    directness: String,
    file_path: String,
    start_line: i64,
    field: String,
    form: String,
    raw_target: String,
    resolved_path: Option<String>,
    reason: Option<String>,
    candidates: Vec<String>,
}

impl Edge {
    fn location(&self) -> String {
        format!("{}:{}", self.file_path, self.start_line)
    }

    fn is_resolved(&self) -> bool {
        self.target_kind != "unresolved"
    }
}

fn ground_truth() -> serde_json::Value {
    let path = common::named_fixture_root(FIXTURE).join("expected.json");
    let text = std::fs::read_to_string(&path)
        .unwrap_or_else(|err| panic!("{} is unreadable: {err}", path.display()));
    serde_json::from_str(&text)
        .unwrap_or_else(|err| panic!("{} is not valid JSON: {err}", path.display()))
}

/// Build the `doc:<path>` selector map the ground truth is written in, refusing anything
/// ambiguous.
///
/// One shape only. Supersession runs between documents: the subject is a decision, never the
/// heading it happens to be written under, so a `sec:` selector would name something that can
/// never be an endpoint.
fn selectors(conn: &nerve_store::Connection) -> BTreeMap<String, String> {
    let mut candidates: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    let mut stmt = conn
        .prepare(
            "SELECT e.entity_id, o.file_path
               FROM entity e
               JOIN occurrence o ON o.entity_id = e.entity_id
              WHERE e.kind = 'document'
              ORDER BY e.entity_id, o.file_path",
        )
        .unwrap();
    let rows = stmt
        .query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })
        .unwrap();
    for row in rows {
        let (entity_id, file_path) = row.unwrap();
        candidates
            .entry(format!("doc:{file_path}"))
            .or_default()
            .insert(entity_id);
    }

    let mut resolved = BTreeMap::new();
    for (selector, entity_ids) in candidates {
        assert_eq!(
            entity_ids.len(),
            1,
            "selector {selector:?} is ambiguous: {entity_ids:?}"
        );
        resolved.insert(selector, entity_ids.into_iter().next().unwrap());
    }
    resolved
}

fn entity_of(map: &BTreeMap<String, String>, selector: &str) -> String {
    map.get(selector)
        .unwrap_or_else(|| panic!("selector {selector:?} matches no document entity"))
        .clone()
}

/// Every `SUPERSEDES` observation `md-structural` emitted, with its details unpacked.
fn edges(conn: &nerve_store::Connection, names: &BTreeMap<String, String>) -> Vec<Edge> {
    let reverse: BTreeMap<&str, &str> = names
        .iter()
        .map(|(selector, entity_id)| (entity_id.as_str(), selector.as_str()))
        .collect();

    let mut stmt = conn
        .prepare(
            "SELECT a.source_entity_id, a.target_entity_id, o.evidence_source_type, o.directness,
                    o.file_path, o.start_line, o.details, target.kind, target.name
               FROM assertion a
               JOIN observation o ON o.assertion_id = a.assertion_id
               JOIN entity target ON target.entity_id = a.target_entity_id
              WHERE a.relation = 'SUPERSEDES' AND o.extractor_id = 'md-structural'
              ORDER BY o.file_path, o.start_line, a.assertion_id",
        )
        .unwrap();

    let rows = stmt
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, String>(4)?,
                row.get::<_, i64>(5)?,
                row.get::<_, Option<String>>(6)?,
                row.get::<_, String>(7)?,
                row.get::<_, String>(8)?,
            ))
        })
        .unwrap();

    let mut out = Vec::new();
    for row in rows {
        let (from, to, source_type, directness, file_path, start_line, details, kind, name) =
            row.unwrap();
        let details: serde_json::Value =
            serde_json::from_str(details.as_deref().unwrap_or_else(|| {
                panic!("a supersession observation in {file_path} has no details")
            }))
            .expect("observation details must be JSON");
        let text = |key: &str| -> Option<String> {
            details
                .get(key)
                .and_then(|value| value.as_str().map(str::to_string))
        };
        let label = |entity_id: &str| -> String {
            reverse
                .get(entity_id)
                .map(|selector| (*selector).to_string())
                .unwrap_or_else(|| format!("<unresolved {name:?}>"))
        };
        out.push(Edge {
            from_label: label(&from),
            to_label: label(&to),
            from,
            to,
            target_kind: kind,
            target_name: name,
            source_type,
            directness,
            file_path,
            start_line,
            field: text("field").expect("field"),
            form: text("form").expect("form"),
            raw_target: text("raw_target").expect("raw_target"),
            resolved_path: text("resolved_path"),
            reason: text("reason"),
            candidates: details["candidates"]
                .as_array()
                .map(|values| {
                    values
                        .iter()
                        .filter_map(|value| value.as_str().map(str::to_string))
                        .collect()
                })
                .unwrap_or_default(),
        });
    }
    out
}

/// Sum one supersession outcome counter over every document's metadata.
fn outcome_total(conn: &nerve_store::Connection, key: &str) -> usize {
    let mut stmt = conn
        .prepare("SELECT meta FROM entity WHERE kind = 'document'")
        .unwrap();
    let rows = stmt
        .query_map([], |row| row.get::<_, Option<String>>(0))
        .unwrap();
    rows.map(|row| {
        let meta: serde_json::Value = serde_json::from_str(&row.unwrap().expect("document meta"))
            .expect("document meta must be JSON");
        meta["supersession"]
            .get(key)
            .and_then(serde_json::Value::as_u64)
            .unwrap_or(0) as usize
    })
    .sum()
}

#[test]
fn measured_supersession_precision_meets_the_slice_5d_gates() {
    let ((_dir, root), report) = indexed_named_fixture(FIXTURE);
    let conn = open_db(&root);
    let truth = ground_truth();
    let names = selectors(&conn);
    let all_edges = edges(&conn, &names);
    let mut failures: Vec<String> = Vec::new();

    // ---- declared resolved edges ----------------------------------------------------------
    let mut expected_resolved: BTreeMap<(String, String), BTreeSet<String>> = BTreeMap::new();
    for entry in truth["resolved"].as_array().expect("resolved array") {
        let key = (
            entity_of(&names, entry["from"].as_str().expect("from")),
            entity_of(&names, entry["to"].as_str().expect("to")),
        );
        let stated: BTreeSet<String> = entry["stated_in"]
            .as_array()
            .expect("stated_in")
            .iter()
            .map(|value| value.as_str().expect("stated_in path").to_string())
            .collect();
        expected_resolved.insert(key, stated);
    }
    let mut forbidden: BTreeMap<(String, String), String> = BTreeMap::new();
    for entry in truth["forbidden"].as_array().expect("forbidden array") {
        let key = (
            entity_of(&names, entry["from"].as_str().expect("from")),
            entity_of(&names, entry["to"].as_str().expect("to")),
        );
        // A corpus is only a measurement if its ground truth is internally consistent.
        assert!(
            !expected_resolved.contains_key(&key),
            "expected.json declares {:?} -> {:?} both resolved and forbidden",
            entry["from"],
            entry["to"]
        );
        forbidden.insert(key, entry["why"].as_str().unwrap_or_default().to_string());
    }

    // ---- gate 1: no undeclared resolved edge (FP = 0) -------------------------------------
    //
    // Counted per **edge**, not per observation: ADR-0001 and ADR-0002 state the same edge from
    // opposite sides, and counting the observations would make one claim look like two.
    let observed_resolved: BTreeSet<(String, String)> = all_edges
        .iter()
        .filter(|edge| edge.is_resolved())
        .map(|edge| (edge.from.clone(), edge.to.clone()))
        .collect();
    let mut false_positives = 0usize;
    for key in &observed_resolved {
        if expected_resolved.contains_key(key) {
            continue;
        }
        false_positives += 1;
        let edge = all_edges
            .iter()
            .find(|edge| (&edge.from, &edge.to) == (&key.0, &key.1))
            .expect("the pair came from this list");
        failures.push(format!(
            "FALSE POSITIVE: {} -> {} at {} (field {}, raw target {:?})",
            edge.from_label,
            edge.to_label,
            edge.location(),
            edge.field,
            edge.raw_target
        ));
    }

    // ---- gate 2: every declared edge is present, and stated exactly where declared ---------
    let mut false_negatives = 0usize;
    for (key, stated_in) in &expected_resolved {
        if !observed_resolved.contains(key) {
            false_negatives += 1;
            failures.push(format!(
                "FALSE NEGATIVE: {} -> {} was declared and not emitted",
                names
                    .iter()
                    .find(|(_, id)| *id == &key.0)
                    .map(|(selector, _)| selector.clone())
                    .unwrap_or_else(|| key.0.clone()),
                names
                    .iter()
                    .find(|(_, id)| *id == &key.1)
                    .map(|(selector, _)| selector.clone())
                    .unwrap_or_else(|| key.1.clone()),
            ));
            continue;
        }
        // "One assertion, two observations" is the whole point of the ADR-0001/ADR-0002 pair.
        // Declaring only the edge would leave a build that dropped one of the two passing.
        let observed_files: BTreeSet<String> = all_edges
            .iter()
            .filter(|edge| (&edge.from, &edge.to) == (&key.0, &key.1))
            .map(|edge| edge.file_path.clone())
            .collect();
        if &observed_files != stated_in {
            failures.push(format!(
                "OBSERVATIONS: edge {} -> {} is stated in {observed_files:?}, expected {stated_in:?}",
                all_edges
                    .iter()
                    .find(|edge| (&edge.from, &edge.to) == (&key.0, &key.1))
                    .map(|edge| edge.from_label.clone())
                    .unwrap_or_default(),
                all_edges
                    .iter()
                    .find(|edge| (&edge.from, &edge.to) == (&key.0, &key.1))
                    .map(|edge| edge.to_label.clone())
                    .unwrap_or_default(),
            ));
        }
    }

    // ---- gate 3: unresolved edges are declared, with their reasons ------------------------
    let observed_unresolved: BTreeSet<(String, String, String, String)> = all_edges
        .iter()
        .filter(|edge| !edge.is_resolved())
        .map(|edge| {
            (
                edge.file_path.clone(),
                edge.field.clone(),
                edge.target_name.clone(),
                edge.reason.clone().unwrap_or_default(),
            )
        })
        .collect();
    let mut expected_unresolved: BTreeSet<(String, String, String, String)> = BTreeSet::new();
    for entry in truth["unresolved"].as_array().expect("unresolved array") {
        let reason = entry["reason"].as_str().expect("reason").to_string();
        assert!(
            REASONS.contains(&reason.as_str()),
            "expected.json uses reason {reason:?}, which is outside the closed vocabulary"
        );
        let key = (
            entry["stated_in"].as_str().expect("stated_in").to_string(),
            entry["field"].as_str().expect("field").to_string(),
            entry["target_name"]
                .as_str()
                .expect("target_name")
                .to_string(),
            reason,
        );
        if !observed_unresolved.contains(&key) {
            let seen: Vec<String> = all_edges
                .iter()
                .filter(|edge| !edge.is_resolved() && edge.file_path == key.0)
                .map(|edge| format!("{:?} reason {:?}", edge.target_name, edge.reason))
                .collect();
            failures.push(format!(
                "MISSING UNRESOLVED: {key:?}. Unresolved edges stated there: {seen:#?}"
            ));
        }
        // An ambiguous refusal must name what it refused; a refusal that says only "ambiguous"
        // leaves the reader no way to check the judgement.
        if let Some(declared) = entry["candidates"].as_array() {
            let declared: Vec<String> = declared
                .iter()
                .map(|value| value.as_str().expect("candidate").to_string())
                .collect();
            let observed: Vec<String> = all_edges
                .iter()
                .find(|edge| edge.file_path == key.0 && edge.target_name == key.2)
                .map(|edge| edge.candidates.clone())
                .unwrap_or_default();
            if observed != declared {
                failures.push(format!(
                    "CANDIDATES: {} recorded {observed:?}, expected {declared:?}",
                    key.0
                ));
            }
        }
        expected_unresolved.insert(key);
    }
    for key in &observed_unresolved {
        if !expected_unresolved.contains(key) {
            failures.push(format!("UNDECLARED UNRESOLVED: {key:?}"));
        }
    }

    // ---- gate 4: no forbidden edge, resolved or not ---------------------------------------
    for edge in &all_edges {
        if let Some(why) = forbidden.get(&(edge.from.clone(), edge.to.clone())) {
            failures.push(format!(
                "FORBIDDEN: {} -> {} at {} ({why})",
                edge.from_label,
                edge.to_label,
                edge.location()
            ));
        }
    }

    // ---- gate 5: a silent file produces no supersession row at all -------------------------
    //
    // Stronger than "no resolved edge": prose containing the word, a `Superseded` status with no
    // target, and a field inside a fenced code block must produce no observation and no
    // `Unresolved` entity. "Nerve declined to read this as evidence" and "Nerve failed at this"
    // are different claims, and the graph must not blur them.
    for entry in truth["silent"].as_array().expect("silent array") {
        let file = entry["file"].as_str().expect("file");
        let why = entry["why"].as_str().unwrap_or_default();
        if let Some(edge) = all_edges.iter().find(|edge| edge.file_path == file) {
            failures.push(format!(
                "NOT SILENT: {file} produced {} -> {} at {} ({why})",
                edge.from_label,
                edge.to_label,
                edge.location()
            ));
        }
        let named: i64 = conn
            .query_row(
                "SELECT count(*) FROM entity
                  WHERE kind = 'unresolved' AND scope_path = ?1
                    AND json_extract(meta, '$.category') = 'document_supersedes'",
                [file],
                |row| row.get(0),
            )
            .unwrap();
        if named != 0 {
            failures.push(format!(
                "NOT SILENT: {file} produced an unresolved supersession entity ({why})"
            ));
        }
    }

    // ---- gate 6: an external target is counted and never entity-ised -----------------------
    for entry in truth["external"].as_array().expect("external array") {
        let stated_in = entry["stated_in"].as_str().expect("stated_in");
        let destination = entry["destination"].as_str().expect("destination");
        let why = entry["why"].as_str().unwrap_or_default();
        if let Some(edge) = all_edges.iter().find(|edge| edge.file_path == stated_in) {
            failures.push(format!(
                "NOT SILENT: {stated_in} produced {} -> {} ({why})",
                edge.from_label, edge.to_label
            ));
        }
        let named: i64 = conn
            .query_row(
                "SELECT count(*) FROM entity WHERE kind = 'unresolved' AND name LIKE ?1",
                [format!("%{destination}%")],
                |row| row.get(0),
            )
            .unwrap();
        if named != 0 {
            failures.push(format!(
                "NOT SILENT: {destination:?} became an unresolved entity ({why})"
            ));
        }
    }
    let declared_external = truth["external"].as_array().unwrap().len();
    let observed_external = outcome_total(&conn, outcome::SUPERSEDES_EXTERNAL);
    if declared_external != observed_external {
        failures.push(format!(
            "EXTERNAL COUNT: expected.json declares {declared_external}, \
             the documents counted {observed_external}"
        ));
    }

    // ---- gate 7: evidence labelling matches whether resolution happened --------------------
    for edge in &all_edges {
        let expected = if edge.is_resolved() {
            ("DOCUMENT_STATED", "RESOLVED")
        } else {
            ("DOCUMENT_STATED", "DIRECT")
        };
        if (edge.source_type.as_str(), edge.directness.as_str()) != expected {
            failures.push(format!(
                "EVIDENCE: {} -> {} at {} carries {}/{}, expected {}/{}",
                edge.from_label,
                edge.to_label,
                edge.location(),
                edge.source_type,
                edge.directness,
                expected.0,
                expected.1
            ));
        }
        if edge.field != "supersedes" && edge.field != "superseded_by" {
            failures.push(format!(
                "FIELD: {:?} is outside the closed vocabulary",
                edge.field
            ));
        }
        if edge.form != "header-line" && edge.form != "supersession-section" {
            failures.push(format!(
                "FORM: {:?} is outside the closed vocabulary",
                edge.form
            ));
        }
        // A resolved edge names the path it resolved to; an unresolved one names none.
        if edge.is_resolved() != edge.resolved_path.is_some() {
            failures.push(format!(
                "RESOLVED PATH: {} -> {} at {} records {:?}",
                edge.from_label,
                edge.to_label,
                edge.location(),
                edge.resolved_path
            ));
        }
    }

    // ---- gate 8: the cycle is detected, counted, and not suppressed -------------------------
    let cycle = &truth["cycles"];
    let declared_cycles = cycle["count"].as_u64().expect("cycles.count") as usize;
    let cycle_documents: Vec<String> = cycle["documents"]
        .as_array()
        .expect("cycles.documents")
        .iter()
        .map(|value| value.as_str().expect("cycle document").to_string())
        .collect();
    if report.supersession_cycles != declared_cycles {
        failures.push(format!(
            "CYCLES: the run reported {}, expected.json declares {declared_cycles}",
            report.supersession_cycles
        ));
    }
    if report.supersession_cycle_documents != cycle_documents.len() {
        failures.push(format!(
            "CYCLE DOCUMENTS: the run reported {}, expected.json declares {}",
            report.supersession_cycle_documents,
            cycle_documents.len()
        ));
    }
    // Not suppressed: every edge stated by a document on the cycle must still be in the graph.
    for file in &cycle_documents {
        if !all_edges
            .iter()
            .any(|edge| &edge.file_path == file && edge.is_resolved())
        {
            failures.push(format!(
                "CYCLE SUPPRESSED: {file} states a supersession edge that is not in the graph"
            ));
        }
    }

    // ---- gate 9: the status contradiction is counted ----------------------------------------
    let declared_contradictions = truth["contradictions"].as_array().expect("contradictions");
    if report.supersession_contradictions != declared_contradictions.len() {
        failures.push(format!(
            "CONTRADICTIONS: the run reported {}, expected.json declares {}",
            report.supersession_contradictions,
            declared_contradictions.len()
        ));
    }
    for entry in declared_contradictions {
        let document = entry["document"].as_str().expect("document");
        let target = entity_of(&names, &format!("doc:{document}"));
        if !observed_resolved.iter().any(|(_, to)| to == &target) {
            failures.push(format!(
                "CONTRADICTION: {document} is declared superseded and is the target of no edge"
            ));
        }
        let status: Option<String> = conn
            .query_row(
                "SELECT json_extract(meta, '$.status') FROM entity WHERE entity_id = ?1",
                [&target],
                |row| row.get(0),
            )
            .unwrap();
        if status.as_deref() != Some("Accepted") {
            failures.push(format!(
                "CONTRADICTION: {document} records status {status:?}, expected \"Accepted\""
            ));
        }
    }

    // ---- gate 10: the chain is a path, and is not collapsed ---------------------------------
    let hops: Vec<String> = truth["chain"]["hops"]
        .as_array()
        .expect("chain.hops")
        .iter()
        .map(|value| entity_of(&names, value.as_str().expect("hop")))
        .collect();
    for pair in hops.windows(2) {
        if !observed_resolved.contains(&(pair[0].clone(), pair[1].clone())) {
            failures.push("CHAIN: a declared hop is missing from the graph".to_string());
        }
    }

    // ---- the table -------------------------------------------------------------------------
    let resolved_edges = observed_resolved.len();
    let unresolved_edges = observed_unresolved.len();
    let true_positives = resolved_edges - false_positives;
    let declared = expected_resolved.len();
    let statements = all_edges.len() + observed_external;

    let percent = |numerator: usize, denominator: usize| -> f64 {
        if denominator == 0 {
            0.0
        } else {
            numerator as f64 * 100.0 / denominator as f64
        }
    };

    println!("\n=== md-structural supersession precision on fixtures/{FIXTURE} (fixture-only) ===");
    println!("documents scanned      {}", report.documents_processed);
    println!("supersession fields    {statements}");
    println!(
        "  resolved             {}",
        outcome_total(&conn, outcome::SUPERSEDES_RESOLVED)
    );
    println!("  external             {observed_external}  (counted, never fetched)");
    for key in REASONS {
        println!("  {key:<44} {}", outcome_total(&conn, key));
    }
    println!("---");
    println!(
        "edges: {resolved_edges} resolved, {unresolved_edges} unresolved \
         (from {} observations)",
        all_edges.len()
    );
    println!(
        "TP {true_positives} · FP {false_positives} · FN {false_negatives} · declared {declared}"
    );
    println!(
        "precision       {:.1}%",
        percent(true_positives, true_positives + false_positives)
    );
    println!(
        "recall          {:.1}%",
        percent(declared - false_negatives, declared)
    );
    println!(
        "unresolved rate {:.1}%",
        percent(unresolved_edges, resolved_edges + unresolved_edges)
    );
    println!(
        "cycles          {} over {} documents  (detected, counted, never suppressed)",
        report.supersession_cycles, report.supersession_cycle_documents
    );
    println!(
        "contradictions  {}  (superseded, and still says Accepted)",
        report.supersession_contradictions
    );
    println!("A regression gate, not an accuracy claim.");
    println!("=== end ===\n");

    assert!(
        failures.is_empty(),
        "supersession precision gates failed ({} problem(s)):\n{}",
        failures.len(),
        failures.join("\n")
    );
    assert_eq!(false_positives, 0, "the gate is zero false positives");
    assert!(
        resolved_edges > 0 && unresolved_edges > 0,
        "the corpus produced no edges of one kind; the gates proved nothing"
    );
}

/// Supersession must not let `md-structural` grow an evidence type it does not declare, and the
/// relation must stay between documents.
#[test]
fn supersession_stays_inside_the_extractor_declaration() {
    let ((_dir, root), _) = indexed_named_fixture(FIXTURE);
    let conn = open_db(&root);

    let distinct = |sql: &str| -> Vec<String> {
        let mut stmt = conn.prepare(sql).unwrap();
        let rows = stmt.query_map([], |row| row.get::<_, String>(0)).unwrap();
        rows.map(|row| row.unwrap()).collect()
    };

    assert_eq!(
        distinct(
            "SELECT DISTINCT evidence_source_type FROM observation
              WHERE extractor_id = 'md-structural' ORDER BY 1"
        ),
        vec!["DOCUMENT_STATED".to_string()],
        "a resolved supersession edge is still only a document's claim"
    );
    assert_eq!(
        distinct(
            "SELECT DISTINCT o.extractor_id FROM assertion a
               JOIN observation o ON o.assertion_id = a.assertion_id
              WHERE a.relation = 'SUPERSEDES' ORDER BY 1"
        ),
        vec!["md-structural".to_string()],
        "no other extractor may claim supersession"
    );

    // Every endpoint of a resolved supersession edge is a document; an unresolved one is the
    // only other kind that may appear, and only at the end the field named.
    assert_eq!(
        distinct(
            "SELECT DISTINCT source.kind FROM assertion a
               JOIN entity source ON source.entity_id = a.source_entity_id
              WHERE a.relation = 'SUPERSEDES' ORDER BY 1"
        ),
        vec!["document".to_string()]
    );
    assert_eq!(
        distinct(
            "SELECT DISTINCT target.kind FROM assertion a
               JOIN entity target ON target.entity_id = a.target_entity_id
              WHERE a.relation = 'SUPERSEDES' ORDER BY 1"
        ),
        vec!["document".to_string(), "unresolved".to_string()]
    );

    // No supersession row carries a scalar confidence: this extractor performs no matching.
    let with_quality: i64 = conn
        .query_row(
            "SELECT count(*) FROM observation o
               JOIN assertion a ON a.assertion_id = o.assertion_id
              WHERE a.relation = 'SUPERSEDES' AND o.match_quality IS NOT NULL",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(with_quality, 0);
}

/// The corpus indexes deterministically and re-indexing it changes nothing. Supersession
/// resolution reads the whole repository, so it is exactly the kind of step that could make a
/// second run disagree with the first.
#[test]
fn the_supersession_corpus_indexes_deterministically() {
    let dump = |root: &std::path::Path| {
        let conn = open_db(root);
        nerve_store::canonical_dump(&conn)
            .unwrap()
            .to_canonical_json()
            .unwrap()
    };

    let ((_dir_a, root_a), _) = indexed_named_fixture(FIXTURE);
    let ((_dir_b, root_b), _) = indexed_named_fixture(FIXTURE);
    assert_eq!(dump(&root_a), dump(&root_b));

    let before = dump(&root_a);
    nerve_index::index_repository(&root_a).unwrap();
    assert_eq!(dump(&root_a), before, "re-indexing changed the graph");
}
