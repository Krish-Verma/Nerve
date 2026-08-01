//! Measured precision for `ts-js-reference` over `fixtures/ts-resolution`.
//!
//! ADR-0003 says "how confident are we?" is answered by an extractor's measured precision on
//! its fixture corpus, not by a per-row number. This file is that measurement, and it is a
//! gate, not a report: false positives and false negatives must both be zero, no forbidden edge
//! may exist in any form, and every unresolved edge must be declared with its reason.
//!
//! Run `cargo test -p nerve-index precision -- --nocapture` to see the table.

mod common;

use std::collections::{BTreeMap, BTreeSet};

use common::{indexed_named_fixture, open_db};

const FIXTURE: &str = "ts-resolution";
const RELATIONS: [&str; 4] = ["CALLS", "REFERENCES", "EXTENDS", "IMPLEMENTS"];

/// One edge as the database has it, with enough context to diagnose a failure.
///
/// `from` / `to` are entity ids, which is what every comparison uses. `from_label` / `to_label`
/// are the selectors a reader recognises, which is what every message prints.
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
            "function" | "method" | "class" | "interface" => {
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

/// Every `CALLS` / `REFERENCES` / `EXTENDS` / `IMPLEMENTS` edge with its evidence.
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
              WHERE a.relation IN ('CALLS', 'REFERENCES', 'EXTENDS', 'IMPLEMENTS')
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
fn measured_precision_meets_the_slice_2a_gates() {
    let ((_dir, root), outcome) = indexed_named_fixture(FIXTURE);
    let conn = open_db(&root);
    let ground_truth = expected();
    let names = selectors(&conn);
    let all_edges = edges(&conn, &names);

    // ---- resolve every selector, failing loudly on anything that names nothing ------------
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
        // The corpus is only a measurement if the ground truth is internally consistent.
        assert!(
            !expected_resolved.contains(&key),
            "expected.json declares {from} {relation} {to} both resolved and forbidden"
        );
        forbidden.insert(key, why);
    }

    // ---- observed --------------------------------------------------------------------
    let resolved_observed: BTreeSet<(String, String, String)> = all_edges
        .iter()
        .filter(|edge| edge.source_type == "AST_RESOLVED")
        .map(Edge::key)
        .collect();

    let mut failures: Vec<String> = Vec::new();

    // ---- gate 2: no undeclared resolved edge ---------------------------------------------
    let mut false_positives: BTreeMap<String, Vec<&Edge>> = BTreeMap::new();
    for edge in &all_edges {
        if edge.source_type != "AST_RESOLVED" {
            continue;
        }
        if !expected_resolved.contains(&edge.key()) {
            false_positives
                .entry(edge.relation.clone())
                .or_default()
                .push(edge);
        }
    }
    for (relation, edges) in &false_positives {
        for edge in edges {
            failures.push(format!(
                "FALSE POSITIVE {relation}: {} -> {} at {} (not in expected.resolved)",
                edge.from_label,
                edge.to_label,
                edge.location()
            ));
        }
    }

    // ---- gate 3: no missing resolved edge -------------------------------------------------
    let mut false_negatives: BTreeMap<String, usize> = BTreeMap::new();
    for (from, relation, to, note) in triples(&ground_truth, "resolved") {
        let key = (
            entity_of(&names, &from),
            relation.clone(),
            entity_of(&names, &to),
        );
        if !resolved_observed.contains(&key) {
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
    }

    // ---- gate 4: no forbidden edge, resolved or unresolved --------------------------------
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

    // ---- gate 5: every unresolved edge is declared, with its reason -----------------------
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
                "MISSING UNRESOLVED {relation}: {from} -> {target_name:?} reason {reason:?}. Unresolved edges from that source: {seen:#?}"
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

    // ---- gate 6: report -------------------------------------------------------------------
    println!("\n=== ts-js-reference precision on fixtures/{FIXTURE} ===");
    println!(
        "{:<12} {:>4} {:>4} {:>4} {:>10} {:>16}",
        "relation", "TP", "FP", "FN", "unresolved", "unresolved-rate"
    );
    for relation in RELATIONS {
        let resolved_count = all_edges
            .iter()
            .filter(|edge| edge.relation == relation && edge.source_type == "AST_RESOLVED")
            .count();
        let unresolved_count = all_edges
            .iter()
            .filter(|edge| edge.relation == relation && edge.target_is_unresolved)
            .count();
        let false_positive = false_positives
            .get(relation)
            .map(|edges| edges.len())
            .unwrap_or(0);
        let false_negative = false_negatives.get(relation).copied().unwrap_or(0);
        let true_positive = resolved_count - false_positive;
        let total = resolved_count + unresolved_count;
        let rate = if total == 0 {
            0.0
        } else {
            unresolved_count as f64 / total as f64
        };
        println!(
            "{relation:<12} {true_positive:>4} {false_positive:>4} {false_negative:>4} {unresolved_count:>10} {:>15.1}%",
            rate * 100.0
        );
    }
    let modelled: usize = all_edges
        .iter()
        .filter(|edge| RELATIONS.contains(&edge.relation.as_str()))
        .count();
    println!(
        "totals: {modelled} modelled edges, {} unmodelled call/heritage sites {:?}",
        outcome.unmodelled_call_sites, outcome.unmodelled_by_form
    );
    println!("=== end ===\n");

    // ---- unmodelled counting --------------------------------------------------------------
    let expected_unmodelled = ground_truth["unmodelled_call_sites"]
        .as_u64()
        .expect("unmodelled_call_sites") as usize;
    if outcome.unmodelled_call_sites != expected_unmodelled {
        failures.push(format!(
            "UNMODELLED COUNT: expected {expected_unmodelled}, observed {} {:?}",
            outcome.unmodelled_call_sites, outcome.unmodelled_by_form
        ));
    }
    let expected_forms = ground_truth["unmodelled_by_form"]
        .as_object()
        .expect("unmodelled_by_form");
    let observed_forms: BTreeMap<String, u64> = outcome
        .unmodelled_by_form
        .iter()
        .map(|(form, count)| (form.clone(), *count as u64))
        .collect();
    let declared_forms: BTreeMap<String, u64> = expected_forms
        .iter()
        .map(|(form, count)| (form.clone(), count.as_u64().expect("count")))
        .collect();
    if observed_forms != declared_forms {
        failures.push(format!(
            "UNMODELLED FORMS: expected {declared_forms:?}, observed {observed_forms:?}"
        ));
    }

    assert!(
        failures.is_empty(),
        "precision gates failed ({} problem(s)):\n{}",
        failures.len(),
        failures.join("\n")
    );
}

/// The reference extractor must not emit an evidence type it did not declare, and must not
/// silently take over the structural extractor's relations.
#[test]
fn the_reference_extractor_stays_inside_its_declaration() {
    let ((_dir, root), _) = indexed_named_fixture(FIXTURE);
    let conn = open_db(&root);

    let mut stmt = conn
        .prepare(
            "SELECT DISTINCT evidence_source_type FROM observation
              WHERE extractor_id = 'ts-js-reference' ORDER BY evidence_source_type",
        )
        .unwrap();
    let types: Vec<String> = stmt
        .query_map([], |row| row.get(0))
        .unwrap()
        .map(|row| row.unwrap())
        .collect();
    assert_eq!(types, vec!["AST_DIRECT", "AST_RESOLVED"]);

    let mut stmt = conn
        .prepare(
            "SELECT DISTINCT a.relation
               FROM assertion a
               JOIN observation o ON o.assertion_id = a.assertion_id
              WHERE o.extractor_id = 'ts-js-reference'
              ORDER BY a.relation",
        )
        .unwrap();
    let relations: Vec<String> = stmt
        .query_map([], |row| row.get(0))
        .unwrap()
        .map(|row| row.unwrap())
        .collect();
    assert_eq!(
        relations,
        vec!["CALLS", "EXTENDS", "IMPLEMENTS", "REFERENCES"]
    );

    // And the structural extractor must not have grown any of them.
    let leaked: i64 = conn
        .query_row(
            "SELECT count(*) FROM observation o
               JOIN assertion a ON a.assertion_id = o.assertion_id
              WHERE o.extractor_id = 'ts-js-structural'
                AND a.relation IN ('CALLS', 'REFERENCES', 'EXTENDS', 'IMPLEMENTS')",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(leaked, 0);
}

/// Every resolved edge carries `AST_RESOLVED` + `RESOLVED`; every unresolved-target edge
/// carries `AST_DIRECT` + `DIRECT`. Nothing in between.
#[test]
fn evidence_labelling_matches_whether_resolution_happened() {
    let ((_dir, root), _) = indexed_named_fixture(FIXTURE);
    let conn = open_db(&root);

    let mismatched: i64 = conn
        .query_row(
            "SELECT count(*) FROM observation o
               JOIN assertion a ON a.assertion_id = o.assertion_id
               JOIN entity t ON t.entity_id = a.target_entity_id
              WHERE o.extractor_id = 'ts-js-reference'
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

    // No observation carries a scalar confidence: the column exists only for matching
    // extractors, and neither of these two performs matching.
    let with_quality: i64 = conn
        .query_row(
            "SELECT count(*) FROM observation WHERE match_quality IS NOT NULL",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(with_quality, 0);
}

/// Determinism and idempotence hold for the resolution corpus too, not only `ts-basic`.
#[test]
fn the_resolution_corpus_indexes_deterministically() {
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

/// `assertion_state` is still a pure rebuild with the new relations present.
#[test]
fn assertion_state_is_a_pure_rebuild_with_resolved_relations_present() {
    let ((_dir, root), _) = indexed_named_fixture(FIXTURE);
    let conn = open_db(&root);

    let snapshot = |conn: &nerve_store::Connection| -> Vec<String> {
        let mut stmt = conn
            .prepare(
                "SELECT assertion_id || '|' || status || '|' ||
                        strongest_source_type || '|' || source_type_mask || '|' ||
                        observation_count || '|' || is_unresolved
                   FROM assertion_state ORDER BY assertion_id",
            )
            .unwrap();
        stmt.query_map([], |row| row.get(0))
            .unwrap()
            .map(|row| row.unwrap())
            .collect()
    };

    let resolved_relations: i64 = conn
        .query_row(
            "SELECT count(*) FROM assertion
              WHERE relation IN ('CALLS', 'REFERENCES', 'EXTENDS', 'IMPLEMENTS')",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert!(resolved_relations > 0, "corpus produced no new relations");

    let before = snapshot(&conn);
    conn.execute("DELETE FROM assertion_state", []).unwrap();
    nerve_store::rebuild_assertion_state(&conn).unwrap();
    assert_eq!(snapshot(&conn), before, "rebuild is not a pure function");
}
