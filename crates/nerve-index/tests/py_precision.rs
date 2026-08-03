//! Measured precision for `py-reference` over `fixtures/py-resolution`.
//!
//! **Python's own numbers, on Python's own corpus.** `precision.rs` measures `ts-js-reference`
//! over `fixtures/ts-resolution`; nothing here reads that fixture and nothing there reads this
//! one. A combined figure would let a strong TypeScript result hide a weak Python one, which is
//! exactly what a second language makes easy to do by accident.
//!
//! The gates are the Slice 9b acceptance criteria:
//!
//! - **FP = 0.** Any `AST_RESOLVED` edge the ground truth does not name fails the run.
//! - **FN is measured and printed, and may be non-zero.** `expected.json` pins three edges a
//!   semantically complete resolver would produce and Nerve deliberately does not. Each must be
//!   *absent*: if one starts being produced the gate fails and says to promote it to `resolved`.
//!   So the FN column is a measurement, not a zero asserted into existence.
//! - **No forbidden edge exists in any form**, resolved or unresolved.
//! - **Every unresolved edge is declared, with its reason**, and no undeclared one exists.
//! - **The unresolved rate is printed**, per relation, as the TypeScript one is.
//!
//! Run `cargo test -p nerve-index py_precision -- --nocapture` to see the table.

mod common;

use std::collections::{BTreeMap, BTreeSet};

use common::{indexed_named_fixture, open_db};

const FIXTURE: &str = "py-resolution";

/// The relations `py-reference` may assert. `IMPLEMENTS` is deliberately absent and is asserted
/// to stay absent below: Python has no `implements` keyword.
const RELATIONS: [&str; 3] = ["CALLS", "REFERENCES", "EXTENDS"];

const EXTRACTOR: &str = "py-reference";

/// One edge as the database has it, with enough context to diagnose a failure.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct Edge {
    relation: String,
    from: String,
    to: String,
    from_label: String,
    to_label: String,
    source_type: String,
    file_path: String,
    start_line: i64,
    reason: Option<String>,
    target_is_unresolved: bool,
    target_name: String,
}

impl Edge {
    fn location(&self) -> String {
        format!("{}:{}", self.file_path, self.start_line)
    }

    fn key(&self) -> (String, String, String) {
        (self.from.clone(), self.relation.clone(), self.to.clone())
    }
}

fn expected() -> serde_json::Value {
    let path = common::named_fixture_root(FIXTURE).join("expected.json");
    let text = std::fs::read_to_string(&path)
        .unwrap_or_else(|err| panic!("{} is unreadable: {err}", path.display()));
    serde_json::from_str(&text)
        .unwrap_or_else(|err| panic!("{} is not valid JSON: {err}", path.display()))
}

/// Build the selector -> entity id map, refusing anything ambiguous.
fn selectors(conn: &nerve_store::Connection) -> BTreeMap<String, String> {
    let mut candidates: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    let mut stmt = conn
        .prepare(
            "SELECT e.entity_id, e.kind, e.name, e.scope_path, o.file_path
               FROM entity e
               JOIN occurrence o ON o.entity_id = e.entity_id
              ORDER BY e.entity_id, o.file_path",
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
            ))
        })
        .unwrap();

    for row in rows {
        let (entity_id, kind, name, scope_path, file_path) = row.unwrap();
        let selector = match kind.as_str() {
            "module" => file_path.clone(),
            "function" | "method" | "class" => {
                if scope_path.is_empty() {
                    format!("{file_path}#{name}")
                } else {
                    format!("{file_path}#{scope_path}.{name}")
                }
            }
            _ => continue,
        };
        candidates.entry(selector).or_default().insert(entity_id);
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
        .unwrap_or_else(|| {
            let near: Vec<&String> = map
                .keys()
                .filter(|key| key.split('#').next() == selector.split('#').next())
                .collect();
            panic!("selector {selector:?} matches no entity. Selectors in that file: {near:#?}")
        })
        .clone()
}

/// Every edge **`py-reference` itself wrote**. Scoped to the extractor on purpose: the number
/// this file reports must be Python's, not the repository's.
fn edges(conn: &nerve_store::Connection, names: &BTreeMap<String, String>) -> Vec<Edge> {
    let reverse: BTreeMap<&str, &str> = names
        .iter()
        .map(|(selector, entity_id)| (entity_id.as_str(), selector.as_str()))
        .collect();

    let mut stmt = conn
        .prepare(
            "SELECT a.relation, a.source_entity_id, a.target_entity_id,
                    o.evidence_source_type, o.file_path, o.start_line, o.details,
                    target.kind, target.name
               FROM assertion a
               JOIN observation o ON o.assertion_id = a.assertion_id
               JOIN entity target ON target.entity_id = a.target_entity_id
              WHERE o.extractor_id = 'py-reference'
              ORDER BY o.file_path, o.start_line, a.relation, a.assertion_id",
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
        let (
            relation,
            source_entity_id,
            target_entity_id,
            source_type,
            file_path,
            start_line,
            details,
            target_kind,
            target_name,
        ) = row.unwrap();
        let reason = details
            .as_deref()
            .and_then(|raw| serde_json::from_str::<serde_json::Value>(raw).ok())
            .and_then(|value| {
                value
                    .get("reason")
                    .and_then(|reason| reason.as_str().map(str::to_string))
            });
        let from_label = reverse
            .get(source_entity_id.as_str())
            .map(|selector| (*selector).to_string())
            .unwrap_or_else(|| source_entity_id.clone());
        let to_label = reverse
            .get(target_entity_id.as_str())
            .map(|selector| (*selector).to_string())
            .unwrap_or_else(|| format!("<unresolved {target_name:?}>"));
        out.push(Edge {
            relation,
            from: source_entity_id,
            to: target_entity_id,
            from_label,
            to_label,
            source_type,
            file_path,
            start_line,
            reason,
            target_is_unresolved: target_kind == "unresolved",
            target_name,
        });
    }
    out.sort();
    out.dedup();
    out
}

/// `(from, relation, to, note)` for each entry of a triple-shaped array.
fn triples(value: &serde_json::Value, key: &str) -> Vec<(String, String, String, String)> {
    value[key]
        .as_array()
        .unwrap_or_else(|| panic!("expected.json has no {key} array"))
        .iter()
        .map(|entry| {
            (
                entry["from"].as_str().expect("from").to_string(),
                entry["relation"].as_str().expect("relation").to_string(),
                entry["to"].as_str().expect("to").to_string(),
                entry
                    .get("note")
                    .or_else(|| entry.get("why"))
                    .and_then(|note| note.as_str())
                    .unwrap_or_default()
                    .to_string(),
            )
        })
        .collect()
}

#[test]
fn measured_precision_meets_the_slice_9b_gates() {
    let ((_dir, root), outcome) = indexed_named_fixture(FIXTURE);
    let conn = open_db(&root);
    let ground_truth = expected();
    let names = selectors(&conn);
    let all_edges = edges(&conn, &names);

    let mut failures: Vec<String> = Vec::new();

    // ---- the ground truth must be internally consistent before it can measure anything ------
    let mut expected_resolved: BTreeSet<(String, String, String)> = BTreeSet::new();
    for (from, relation, to, _) in triples(&ground_truth, "resolved") {
        assert!(
            RELATIONS.contains(&relation.as_str()),
            "expected.resolved names relation {relation:?}, which is not measured"
        );
        expected_resolved.insert((entity_of(&names, &from), relation, entity_of(&names, &to)));
    }
    let mut forbidden: BTreeMap<(String, String, String), String> = BTreeMap::new();
    for (from, relation, to, why) in triples(&ground_truth, "forbidden") {
        let key = (
            entity_of(&names, &from),
            relation.clone(),
            entity_of(&names, &to),
        );
        assert!(
            !expected_resolved.contains(&key),
            "expected.json declares {from} {relation} {to} both resolved and forbidden"
        );
        forbidden.insert(key, why);
    }
    let mut known_false_negatives: BTreeMap<(String, String, String), String> = BTreeMap::new();
    for (from, relation, to, why) in triples(&ground_truth, "known_false_negatives") {
        let key = (
            entity_of(&names, &from),
            relation.clone(),
            entity_of(&names, &to),
        );
        assert!(
            !expected_resolved.contains(&key),
            "expected.json declares {from} {relation} {to} both resolved and a known FN"
        );
        assert!(
            !forbidden.contains_key(&key),
            "expected.json declares {from} {relation} {to} both forbidden and a known FN; a \
             forbidden edge is one that would be wrong, a known FN is one that would be right"
        );
        known_false_negatives.insert(key, why);
    }

    // ---- gate 1: no forbidden edge, in any form --------------------------------------------
    //
    // Checked **first**, and deliberately. The comparisons below already catch every one of
    // these — a forbidden edge is by construction an unexpected one — but they report a set
    // difference, while this reports the named wrong answer and the reason it is wrong. Running
    // the specific check before the general one is what makes it ever fire; behind the general
    // one it would be unreachable, which is how a test comes to be trusted without having been
    // exercised. Slice 9a found exactly that defect by probe.
    for edge in &all_edges {
        if let Some(why) = forbidden.get(&edge.key()) {
            failures.push(format!(
                "FORBIDDEN {}: {} -> {} at {} [{}] ({why})",
                edge.relation,
                edge.from_label,
                edge.to_label,
                edge.location(),
                edge.source_type
            ));
        }
    }

    // ---- gate 2: a pinned false negative must stay absent ----------------------------------
    for edge in &all_edges {
        if edge.source_type != "AST_RESOLVED" {
            continue;
        }
        if let Some(why) = known_false_negatives.get(&edge.key()) {
            failures.push(format!(
                "KNOWN FALSE NEGATIVE NOW PRODUCED {}: {} -> {} at {}. It was declared absent \
                 because: {why}. If the refusal was lifted on purpose, move the entry from \
                 known_false_negatives to resolved.",
                edge.relation,
                edge.from_label,
                edge.to_label,
                edge.location()
            ));
        }
    }

    // ---- gate 3: FP = 0 --------------------------------------------------------------------
    let resolved_observed: BTreeSet<(String, String, String)> = all_edges
        .iter()
        .filter(|edge| edge.source_type == "AST_RESOLVED")
        .map(Edge::key)
        .collect();

    let mut false_positives: BTreeMap<String, usize> = BTreeMap::new();
    for edge in &all_edges {
        if edge.source_type != "AST_RESOLVED" || expected_resolved.contains(&edge.key()) {
            continue;
        }
        *false_positives.entry(edge.relation.clone()).or_insert(0) += 1;
        failures.push(format!(
            "FALSE POSITIVE {}: {} -> {} at {} (not in expected.resolved)",
            edge.relation,
            edge.from_label,
            edge.to_label,
            edge.location()
        ));
    }

    // ---- gate 4: every declared resolved edge exists ---------------------------------------
    let mut false_negatives: BTreeMap<String, usize> = BTreeMap::new();
    for (_, relation, _, _) in triples(&ground_truth, "known_false_negatives") {
        *false_negatives.entry(relation).or_insert(0) += 1;
    }
    for (from, relation, to, note) in triples(&ground_truth, "resolved") {
        let key = (
            entity_of(&names, &from),
            relation.clone(),
            entity_of(&names, &to),
        );
        if resolved_observed.contains(&key) {
            continue;
        }
        *false_negatives.entry(relation.clone()).or_insert(0) += 1;
        let seen: Vec<String> = all_edges
            .iter()
            .filter(|edge| edge.relation == relation && edge.from == key.0)
            .map(|edge| {
                format!(
                    "{} [{}] at {}",
                    edge.to_label,
                    edge.source_type,
                    edge.location()
                )
            })
            .collect();
        failures.push(format!(
            "FALSE NEGATIVE {relation}: {from} -> {to} ({note}). Edges from that source: {seen:#?}"
        ));
    }

    // ---- gate 5: every unresolved edge is declared, with its reason ------------------------
    let observed_unresolved: BTreeSet<(String, String, String, String)> = all_edges
        .iter()
        .filter(|edge| edge.target_is_unresolved)
        .map(|edge| {
            (
                edge.from.clone(),
                edge.relation.clone(),
                edge.target_name.clone(),
                edge.reason.clone().unwrap_or_default(),
            )
        })
        .collect();

    let mut expected_unresolved: BTreeSet<(String, String, String, String)> = BTreeSet::new();
    for entry in ground_truth["unresolved"]
        .as_array()
        .expect("expected.json has no unresolved array")
    {
        let from = entry["from"].as_str().expect("from");
        let relation = entry["relation"].as_str().expect("relation").to_string();
        let target_name = entry["target_name"]
            .as_str()
            .expect("target_name")
            .to_string();
        let reason = entry["reason"].as_str().expect("reason").to_string();
        let key = (
            entity_of(&names, from),
            relation.clone(),
            target_name.clone(),
            reason.clone(),
        );
        if !observed_unresolved.contains(&key) {
            let seen: Vec<String> = all_edges
                .iter()
                .filter(|edge| edge.target_is_unresolved && edge.from == key.0)
                .map(|edge| {
                    format!(
                        "{} reason={:?} at {}",
                        edge.target_name,
                        edge.reason,
                        edge.location()
                    )
                })
                .collect();
            failures.push(format!(
                "MISSING UNRESOLVED {relation}: {from} -> {target_name:?} reason {reason:?}. \
                 Unresolved edges from that source: {seen:#?}"
            ));
        }
        expected_unresolved.insert(key);
    }
    for key in &observed_unresolved {
        if expected_unresolved.contains(key) {
            continue;
        }
        let offender = all_edges.iter().find(|edge| {
            edge.target_is_unresolved && edge.target_name == key.2 && edge.from == key.0
        });
        let (label, where_) = match offender {
            Some(edge) => (edge.from_label.clone(), edge.location()),
            None => (key.0.clone(), String::new()),
        };
        failures.push(format!(
            "UNDECLARED UNRESOLVED {}: {label} -> {:?} reason {:?} at {where_}",
            key.1, key.2, key.3
        ));
    }

    // ---- gate 6: the report ----------------------------------------------------------------
    println!("\n=== py-reference precision on fixtures/{FIXTURE} ===");
    println!(
        "{:<12} {:>4} {:>4} {:>4} {:>10} {:>16}",
        "relation", "TP", "FP", "FN", "unresolved", "unresolved-rate"
    );
    let mut modelled = 0usize;
    let mut all_resolved = 0usize;
    let mut all_unresolved = 0usize;
    for relation in RELATIONS {
        let resolved_count = all_edges
            .iter()
            .filter(|edge| edge.relation == relation && edge.source_type == "AST_RESOLVED")
            .count();
        let unresolved_count = all_edges
            .iter()
            .filter(|edge| edge.relation == relation && edge.target_is_unresolved)
            .count();
        let false_positive = false_positives.get(relation).copied().unwrap_or(0);
        let false_negative = false_negatives.get(relation).copied().unwrap_or(0);
        let true_positive = resolved_count - false_positive;
        let total = resolved_count + unresolved_count;
        let rate = if total == 0 {
            0.0
        } else {
            unresolved_count as f64 / total as f64
        };
        modelled += total;
        all_resolved += resolved_count;
        all_unresolved += unresolved_count;
        println!(
            "{relation:<12} {true_positive:>4} {false_positive:>4} {false_negative:>4} {unresolved_count:>10} {:>15.1}%",
            rate * 100.0
        );
    }
    let overall = if modelled == 0 {
        0.0
    } else {
        all_unresolved as f64 / modelled as f64
    };
    println!(
        "{:<12} {:>4} {:>4} {:>4} {:>10} {:>15.1}%",
        "(all)",
        all_resolved - false_positives.values().sum::<usize>(),
        false_positives.values().sum::<usize>(),
        false_negatives.values().sum::<usize>(),
        all_unresolved,
        overall * 100.0
    );
    println!(
        "IMPLEMENTS   {:>4} {:>4} {:>4} {:>10} {:>16}",
        "-", "-", "-", "-", "not a Python relation"
    );
    println!(
        "totals: {modelled} modelled edges, {} unmodelled call/base-class sites {:?}",
        outcome.unmodelled_call_sites, outcome.unmodelled_by_form
    );
    println!(
        "FN is the count of edges pinned in expected.known_false_negatives — a complete resolver \
         would produce them and Nerve declines to. Each is listed with its reason in that file."
    );
    println!("=== end ===\n");

    // ---- gate 7: the unmodelled tally ------------------------------------------------------
    let expected_unmodelled = ground_truth["unmodelled_call_sites"]
        .as_u64()
        .expect("unmodelled_call_sites") as usize;
    if outcome.unmodelled_call_sites != expected_unmodelled {
        failures.push(format!(
            "UNMODELLED COUNT: expected {expected_unmodelled}, observed {} {:?}",
            outcome.unmodelled_call_sites, outcome.unmodelled_by_form
        ));
    }
    let declared_forms: BTreeMap<String, u64> = ground_truth["unmodelled_by_form"]
        .as_object()
        .expect("unmodelled_by_form")
        .iter()
        .map(|(form, count)| (form.clone(), count.as_u64().expect("count")))
        .collect();
    let observed_forms: BTreeMap<String, u64> = outcome
        .unmodelled_by_form
        .iter()
        .map(|(form, count)| (form.clone(), *count as u64))
        .collect();
    if observed_forms != declared_forms {
        failures.push(format!(
            "UNMODELLED FORMS: expected {declared_forms:?}, observed {observed_forms:?}"
        ));
    }

    assert!(
        failures.is_empty(),
        "py-reference precision gates failed ({} problem(s)):\n{}",
        failures.len(),
        failures.join("\n")
    );
}

/// `py-reference` must not emit an evidence type it did not declare, must not assert a relation
/// outside its three, and must not take over `py-structural`'s.
///
/// `IMPLEMENTS` is the one that matters: Python has no `implements` keyword, so no slice may ever
/// produce one from a base list.
#[test]
fn the_python_reference_extractor_stays_inside_its_declaration() {
    let ((_dir, root), _) = indexed_named_fixture(FIXTURE);
    let conn = open_db(&root);

    let mut stmt = conn
        .prepare(
            "SELECT DISTINCT evidence_source_type FROM observation
              WHERE extractor_id = ?1 ORDER BY evidence_source_type",
        )
        .unwrap();
    let types: Vec<String> = stmt
        .query_map([EXTRACTOR], |row| row.get(0))
        .unwrap()
        .map(|row| row.unwrap())
        .collect();
    assert_eq!(types, vec!["AST_DIRECT", "AST_RESOLVED"]);

    let mut stmt = conn
        .prepare(
            "SELECT DISTINCT a.relation
               FROM assertion a
               JOIN observation o ON o.assertion_id = a.assertion_id
              WHERE o.extractor_id = ?1
              ORDER BY a.relation",
        )
        .unwrap();
    let relations: Vec<String> = stmt
        .query_map([EXTRACTOR], |row| row.get(0))
        .unwrap()
        .map(|row| row.unwrap())
        .collect();
    assert_eq!(relations, vec!["CALLS", "EXTENDS", "REFERENCES"]);

    let implements: i64 = conn
        .query_row(
            "SELECT count(*) FROM assertion WHERE relation = 'IMPLEMENTS'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(
        implements, 0,
        "a Python corpus produced an IMPLEMENTS edge; `class C(SomeABC)` states inheritance and \
         nothing else, and Python has no `implements` keyword to state more"
    );

    // The structural extractor must not have grown any of the reference relations.
    let leaked: i64 = conn
        .query_row(
            "SELECT count(*) FROM observation o
               JOIN assertion a ON a.assertion_id = o.assertion_id
              WHERE o.extractor_id = 'py-structural'
                AND a.relation IN ('CALLS', 'REFERENCES', 'EXTENDS', 'IMPLEMENTS')",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(leaked, 0);
}

/// The 5d-i invariant, restated for the reference extractor: a Python-only corpus produces zero
/// `ts-js-*` observations, and `py-reference` produced something so the check is not vacuous.
#[test]
fn the_python_resolution_corpus_produces_no_ts_js_observations() {
    let ((_dir, root), _) = indexed_named_fixture(FIXTURE);
    let conn = open_db(&root);
    let by_extractor: Vec<(String, i64)> = conn
        .prepare("SELECT extractor_id, count(*) FROM observation GROUP BY 1 ORDER BY 1")
        .unwrap()
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
        .unwrap()
        .map(|row| row.unwrap())
        .collect();
    let ts_js: Vec<&(String, i64)> = by_extractor
        .iter()
        .filter(|(id, _)| id.starts_with("ts-js-"))
        .collect();
    assert!(
        ts_js.is_empty(),
        "a repository with no TypeScript in it produced {ts_js:?}; by extractor: {by_extractor:?}"
    );
    assert!(
        by_extractor
            .iter()
            .any(|(id, count)| id == EXTRACTOR && *count > 0),
        "py-reference wrote nothing, so the assertion above would pass vacuously: {by_extractor:?}"
    );
}

/// Every resolved edge carries `AST_RESOLVED` + `RESOLVED`; every unresolved-target edge carries
/// `AST_DIRECT` + `DIRECT`. Nothing in between.
#[test]
fn python_reference_evidence_labelling_matches_whether_resolution_happened() {
    let ((_dir, root), _) = indexed_named_fixture(FIXTURE);
    let conn = open_db(&root);

    let mismatched: i64 = conn
        .query_row(
            "SELECT count(*) FROM observation o
               JOIN assertion a ON a.assertion_id = o.assertion_id
               JOIN entity t ON t.entity_id = a.target_entity_id
              WHERE o.extractor_id = 'py-reference'
                AND ((t.kind = 'unresolved'
                      AND NOT (o.evidence_source_type = 'AST_DIRECT' AND o.directness = 'DIRECT'))
                  OR (t.kind != 'unresolved'
                      AND NOT (o.evidence_source_type = 'AST_RESOLVED'
                               AND o.directness = 'RESOLVED')))",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(mismatched, 0);

    // No observation carries a scalar confidence: the column exists only for matching extractors,
    // and neither Python extractor performs matching.
    let with_quality: i64 = conn
        .query_row(
            "SELECT count(*) FROM observation WHERE match_quality IS NOT NULL",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(with_quality, 0);
}

/// Determinism and idempotence hold for the Python resolution corpus too.
#[test]
fn the_python_resolution_corpus_indexes_deterministically() {
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
