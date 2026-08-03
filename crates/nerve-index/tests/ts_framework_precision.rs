//! Measured precision for `ts-js-framework` over `fixtures/ts-framework`.
//!
//! **Express gets its own table.** It is never summed with FastAPI or Flask, and
//! `py_framework_precision.rs` does not read this fixture. Three framework rules, three numbers —
//! the same discipline that keeps the TypeScript and Python reference tables apart, for the same
//! reason: one working rule must not be able to hide a broken one.
//!
//! Gate order matches `expected.json`: `forbidden` first, because Slice 9a found a `forbidden` list
//! behind a set-equality assertion where it was unreachable and had never fired.

mod common;

use std::collections::{BTreeMap, BTreeSet};

use common::{indexed_named_fixture, open_db};

const FIXTURE: &str = "ts-framework";
const EXTRACTOR: &str = "ts-js-framework";

/// One `SERVED_BY` edge as the database has it.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct Served {
    module: String,
    address: String,
    framework: String,
    handler: String,
    handler_kind: String,
    source_type: String,
    directness: String,
    relation: String,
}

impl Served {
    fn key(&self) -> (String, String, String) {
        (
            self.module.clone(),
            self.address.clone(),
            self.handler.clone(),
        )
    }
}

fn expected() -> serde_json::Value {
    let path = common::named_fixture_root(FIXTURE).join("expected.json");
    let text = std::fs::read_to_string(&path)
        .unwrap_or_else(|err| panic!("{} must be readable: {err}", path.display()));
    serde_json::from_str(&text).expect("expected.json must be valid JSON")
}

fn served_edges(conn: &nerve_store::Connection) -> Vec<Served> {
    let mut statement = conn
        .prepare(
            "SELECT e.scope_path, e.name, e.meta, h.name, h.kind,
                    o.evidence_source_type, o.directness, a.relation
               FROM assertion a
               JOIN entity e ON e.entity_id = a.source_entity_id
               JOIN entity h ON h.entity_id = a.target_entity_id
               JOIN observation o ON o.assertion_id = a.assertion_id
              WHERE o.extractor_id = ?1
              ORDER BY e.scope_path, e.name, h.name",
        )
        .unwrap();
    let rows = statement
        .query_map([EXTRACTOR], |row| {
            let meta: Option<String> = row.get(2)?;
            let meta: serde_json::Value = meta
                .as_deref()
                .and_then(|text| serde_json::from_str(text).ok())
                .unwrap_or(serde_json::Value::Null);
            Ok(Served {
                module: row.get(0)?,
                address: row.get(1)?,
                framework: meta["framework"].as_str().unwrap_or_default().to_string(),
                handler: row.get(3)?,
                handler_kind: row.get(4)?,
                source_type: row.get(5)?,
                directness: row.get(6)?,
                relation: row.get(7)?,
            })
        })
        .unwrap();
    rows.map(|row| row.unwrap()).collect()
}

/// Express, measured on its own.
#[test]
fn the_express_rule_is_measured_on_its_own() {
    let ((_dir, root), _outcome) = indexed_named_fixture(FIXTURE);
    let conn = open_db(&root);
    let expected = expected();
    let actual = served_edges(&conn);

    // ---- forbidden first ----------------------------------------------------------------------
    let mut forgeries = Vec::new();
    for entry in expected["forbidden"].as_array().unwrap() {
        let address = entry["address"].as_str().unwrap();
        for edge in &actual {
            if edge.address == address {
                forgeries.push(format!(
                    "FORBIDDEN {address} exists at {}:{} — {}",
                    edge.module,
                    edge.handler,
                    entry["why"].as_str().unwrap_or_default()
                ));
            }
        }
    }
    assert!(
        forgeries.is_empty(),
        "endpoints that must never exist were emitted:\n  {}",
        forgeries.join("\n  ")
    );

    // ---- declared false negatives must stay absent ---------------------------------------------
    let mut promoted = Vec::new();
    for entry in expected["known_false_negatives"].as_array().unwrap() {
        let address = entry["address"].as_str().unwrap();
        if actual.iter().any(|edge| edge.address == address) {
            promoted.push(format!(
                "{address} is now produced — promote it to `resolved` deliberately. Declared \
                 reason: {}",
                entry["reason"].as_str().unwrap_or_default()
            ));
        }
    }
    assert!(
        promoted.is_empty(),
        "a declared false negative started being emitted:\n  {}",
        promoted.join("\n  ")
    );

    // ---- nothing in `not_a_route` has an incoming edge -----------------------------------------
    let mut over_eager = Vec::new();
    for entry in expected["not_a_route"].as_array().unwrap() {
        let module = entry["module"].as_str().unwrap();
        let symbol = entry["symbol"].as_str().unwrap();
        if actual
            .iter()
            .any(|edge| edge.module == module && edge.handler == symbol)
        {
            over_eager.push(format!("{module}#{symbol}"));
        }
    }
    assert!(
        over_eager.is_empty(),
        "symbols that declare no route were given one: {over_eager:?}"
    );

    // ---- the table ----------------------------------------------------------------------------
    let want: BTreeSet<(String, String, String)> = expected["resolved"]["express"]
        .as_array()
        .unwrap()
        .iter()
        .map(|entry| {
            (
                entry["module"].as_str().unwrap().to_string(),
                entry["address"].as_str().unwrap().to_string(),
                entry["handler"].as_str().unwrap().to_string(),
            )
        })
        .collect();
    let have: BTreeSet<(String, String, String)> = actual.iter().map(Served::key).collect();

    let false_positives: Vec<_> = have.difference(&want).cloned().collect();
    let false_negatives: Vec<_> = want.difference(&have).cloned().collect();

    println!("\nfixtures/{FIXTURE} — Express only, never summed with FastAPI or Flask");
    println!("framework    TP   FP   FN");
    println!(
        "express     {:>3}  {:>3}  {:>3}",
        have.intersection(&want).count(),
        false_positives.len(),
        false_negatives.len()
    );

    assert!(
        false_positives.is_empty(),
        "FP must be 0, found {false_positives:#?}"
    );
    assert!(
        false_negatives.is_empty(),
        "an expected endpoint is missing: {false_negatives:#?}"
    );

    // ---- evidence typing on every edge --------------------------------------------------------
    for edge in &actual {
        assert_eq!(edge.source_type, "FRAMEWORK_RULE", "{edge:?}");
        assert_eq!(edge.directness, "DIRECT", "{edge:?}");
        assert_eq!(edge.relation, "SERVED_BY", "{edge:?}");
        assert_eq!(edge.framework, "express", "{edge:?}");
        assert!(
            matches!(edge.handler_kind.as_str(), "function" | "method"),
            "{edge:?}"
        );
    }
    assert!(
        !actual.is_empty(),
        "the fixture produced no endpoints, so every assertion above passed vacuously"
    );
}

/// `ts-js-framework` asserts `SERVED_BY` and nothing else, over the whole closed vocabulary.
#[test]
fn the_express_extractor_asserts_no_relation_but_served_by() {
    let ((_dir, root), _outcome) = indexed_named_fixture(FIXTURE);
    let conn = open_db(&root);

    for relation in nerve_core::vocab::Relation::ALL {
        let count: i64 = conn
            .query_row(
                "SELECT count(*) FROM assertion a
                   JOIN observation o ON o.assertion_id = a.assertion_id
                  WHERE o.extractor_id = ?1 AND a.relation = ?2",
                [EXTRACTOR, relation.as_str()],
                |row| row.get(0),
            )
            .unwrap();
        if relation == nerve_core::vocab::Relation::ServedBy {
            assert!(
                count > 0,
                "SERVED_BY must be emitted, or nothing is measured"
            );
        } else {
            assert_eq!(
                count, 0,
                "ts-js-framework asserted {relation}, which it does not declare"
            );
        }
    }
}

/// The unsupported tally matches by form, exactly.
#[test]
fn every_unsupported_form_is_counted_and_the_tally_is_pinned() {
    let ((_dir, root), outcome) = indexed_named_fixture(FIXTURE);
    let _ = open_db(&root);
    let expected = expected();

    let want: BTreeMap<String, usize> = expected["unsupported_by_form"]
        .as_object()
        .unwrap()
        .iter()
        .map(|(form, count)| (form.clone(), count.as_u64().unwrap() as usize))
        .collect();
    let have: BTreeMap<String, usize> = outcome
        .framework_unsupported_by_form
        .iter()
        .map(|(form, count)| (form.clone(), *count))
        .collect();

    assert_eq!(
        have, want,
        "the unsupported tally moved. A new form means the precision denominator changed; an \
         absent one means a form no code produces."
    );

    // `methods-not-literal` is Flask-specific. Its absence here is correct, and asserting the
    // absence is what stops someone "fixing" the shared vocabulary by inventing an Express use.
    assert!(
        !have.contains_key("methods-not-literal"),
        "Express has no methods= keyword; a count here would be an invented construct"
    );
}

/// The 5d-i invariant, restated for a third time: no other language's extractor fires here.
///
/// A repository with no Python must produce **zero** `py-framework` observations. Slice 5d-i was a
/// corrective slice for directory containment stamped `ts-js-structural` in a repository with no
/// TypeScript; 9a restated it for Python structure; this restates it for framework rules, where two
/// extractors now answer the same question in two languages and mislabelling is easiest.
#[test]
fn a_repository_with_no_python_produces_no_python_framework_evidence() {
    let ((_dir, root), _outcome) = indexed_named_fixture(FIXTURE);
    let conn = open_db(&root);

    let python: i64 = conn
        .query_row(
            "SELECT count(*) FROM observation WHERE extractor_id = 'py-framework'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(
        python, 0,
        "a repository with no Python in it produced py-framework observations"
    );

    let express: i64 = conn
        .query_row(
            "SELECT count(*) FROM observation WHERE extractor_id = ?1",
            [EXTRACTOR],
            |row| row.get(0),
        )
        .unwrap();
    assert!(
        express > 0,
        "no Express observations either, so the check above is vacuous"
    );
}

/// Express-shaped code that is not a route contributes zero endpoints **and** zero counts.
#[test]
fn express_shaped_code_that_is_not_a_route_contributes_nothing() {
    let ((_dir, root), _outcome) = indexed_named_fixture(FIXTURE);
    let conn = open_db(&root);

    let from_negative: i64 = conn
        .query_row(
            "SELECT count(*) FROM entity WHERE kind = 'endpoint' AND scope_path = 'negative.ts'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(from_negative, 0, "negative.ts declares no routes");

    // And it was indexed, or the assertion above is vacuous.
    let symbols: i64 = conn
        .query_row(
            "SELECT count(*) FROM entity e
               JOIN occurrence o ON o.entity_id = e.entity_id
              WHERE e.kind IN ('function', 'method', 'class') AND o.file_path = 'negative.ts'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert!(
        symbols > 0,
        "negative.ts produced no symbols, so it was never really indexed"
    );
}
