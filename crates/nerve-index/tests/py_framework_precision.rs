//! Measured precision for `py-framework` over `fixtures/py-framework`, **per framework**.
//!
//! FastAPI and Flask are reported as two tables and are never summed. A combined figure would let a
//! working FastAPI rule hide a broken Flask one, which is the same mistake a combined TypeScript and
//! Python figure would be — and neither `precision.rs` nor `py_precision.rs` reads this fixture, so
//! the three measurements stay independent.
//!
//! The gates are the Slice 10a acceptance criteria, and they run in the order `expected.json` states:
//!
//! - **`forbidden` first.** Slice 9a found a `forbidden` list sitting behind a set-equality
//!   assertion, where it was unreachable and had never fired. Specific checks run before the
//!   general one so a failure names the wrong answer instead of dumping a set difference.
//! - **FP = 0.** Any endpoint the ground truth does not name fails the run.
//! - **FN is measured, and the declared ones must stay absent.** A false negative cannot be
//!   "fixed" by a guess without deliberately promoting it in `expected.json`.
//! - **The unsupported tally matches by form**, so a new unreadable construct cannot appear
//!   silently and the precision denominator stays auditable.
//! - **`negative.py` contributes nothing at all** — no endpoint *and* no unsupported count.
//!
//! Run `cargo test -p nerve-index py_framework_precision -- --nocapture` to see the tables.

mod common;

use std::collections::{BTreeMap, BTreeSet};

use common::{indexed_named_fixture, open_db};

const FIXTURE: &str = "py-framework";
const EXTRACTOR: &str = "py-framework";

/// One `SERVED_BY` edge as the database has it.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct Served {
    module: String,
    address: String,
    method: String,
    path: String,
    framework: String,
    handler: String,
    handler_kind: String,
    source_type: String,
    directness: String,
    relation: String,
}

impl Served {
    /// The identity the ground truth names an edge by.
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

/// Every `SERVED_BY` edge `py-framework` wrote, with both endpoints resolved to readable names.
fn served_edges(conn: &nerve_store::Connection) -> Vec<Served> {
    let mut statement = conn
        .prepare(
            "SELECT e.scope_path, e.name, e.meta,
                    h.name, h.kind,
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
                method: meta["method"].as_str().unwrap_or_default().to_string(),
                path: meta["path"].as_str().unwrap_or_default().to_string(),
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

/// The ground truth's edges for one framework, as comparable keys.
fn expected_keys(
    expected: &serde_json::Value,
    framework: &str,
) -> BTreeSet<(String, String, String)> {
    expected["resolved"][framework]
        .as_array()
        .unwrap_or_else(|| panic!("expected.resolved.{framework} must be an array"))
        .iter()
        .map(|entry| {
            (
                entry["module"].as_str().unwrap().to_string(),
                entry["address"].as_str().unwrap().to_string(),
                entry["handler"].as_str().unwrap().to_string(),
            )
        })
        .collect()
}

/// FastAPI and Flask, measured separately and printed as two tables.
#[test]
fn each_framework_rule_is_measured_on_its_own() {
    let ((_dir, root), _outcome) = indexed_named_fixture(FIXTURE);
    let conn = open_db(&root);
    let expected = expected();
    let actual = served_edges(&conn);

    // ---- gate 2: forbidden first, so a failure names the wrong answer ------------------------
    //
    // Slice 9a found this exact list unreachable behind a set-equality assertion. It runs first.
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

    // ---- gate 4: every declared false negative must stay absent -------------------------------
    let mut promoted = Vec::new();
    for entry in expected["known_false_negatives"].as_array().unwrap() {
        let address = entry["address"].as_str().unwrap();
        if actual.iter().any(|edge| edge.address == address) {
            promoted.push(format!(
                "{address} is now produced — promote it to `resolved` deliberately. Declared \
                 reason was: {}",
                entry["reason"].as_str().unwrap_or_default()
            ));
        }
    }
    assert!(
        promoted.is_empty(),
        "a declared false negative started being emitted:\n  {}",
        promoted.join("\n  ")
    );

    // ---- gate 3: nothing in `not_a_route` has an incoming edge --------------------------------
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

    // ---- gates 1, 6 and 7: per-framework tables ----------------------------------------------
    let mut rows = Vec::new();
    let mut any_false_positive = false;
    for framework in ["fastapi", "flask"] {
        let want = expected_keys(&expected, framework);
        let have: BTreeSet<(String, String, String)> = actual
            .iter()
            .filter(|edge| edge.framework == framework)
            .map(Served::key)
            .collect();

        let false_positives: Vec<_> = have.difference(&want).cloned().collect();
        let false_negatives: Vec<_> = want.difference(&have).cloned().collect();
        let true_positives = have.intersection(&want).count();

        if !false_positives.is_empty() {
            any_false_positive = true;
        }

        rows.push(format!(
            "{framework:<9} {tp:>4} {fp:>4} {fn_:>4}",
            tp = true_positives,
            fp = false_positives.len(),
            fn_ = false_negatives.len(),
        ));

        assert!(
            false_positives.is_empty(),
            "{framework}: FP must be 0, found {false_positives:#?}"
        );
        assert!(
            false_negatives.is_empty(),
            "{framework}: an expected endpoint is missing: {false_negatives:#?}"
        );
    }

    println!("\nfixtures/{FIXTURE} — per framework, never summed");
    println!("framework    TP   FP   FN");
    for row in &rows {
        println!("{row}");
    }
    // Repository-wide and deliberately outside the per-framework tables: an `app-not-local` or
    // `path-not-literal` construct has no framework attached — tracing it to one is exactly the
    // step the rule declined — so attributing the tally per framework would invent that attribution.
    let unsupported: BTreeMap<&str, u64> = expected["unsupported_by_form"]
        .as_object()
        .unwrap()
        .iter()
        .map(|(form, count)| (form.as_str(), count.as_u64().unwrap()))
        .collect();
    let total: u64 = unsupported.values().sum();
    println!("unsupported, repository-wide, not attributable to a framework: {total}");
    for (form, count) in &unsupported {
        println!("  {form:<22} {count}");
    }
    assert!(!any_false_positive);

    // ---- gate 8: evidence typing on every edge ------------------------------------------------
    for edge in &actual {
        assert_eq!(
            edge.source_type, "FRAMEWORK_RULE",
            "{} {} carries {} rather than FRAMEWORK_RULE",
            edge.module, edge.address, edge.source_type
        );
        assert_eq!(
            edge.directness, "DIRECT",
            "{} {}",
            edge.module, edge.address
        );
        assert_eq!(
            edge.relation, "SERVED_BY",
            "a registration proves a table entry, not an execution: {} {} asserted {}",
            edge.module, edge.address, edge.relation
        );
        assert!(
            matches!(edge.handler_kind.as_str(), "function" | "method"),
            "{} {} serves a {}, which is not a symbol that can handle a request",
            edge.module,
            edge.address,
            edge.handler_kind
        );
    }
    assert!(
        !actual.is_empty(),
        "the fixture produced no endpoints at all, so every assertion above passed vacuously"
    );
}

/// `py-framework` asserts `SERVED_BY` and nothing else, checked over the whole closed vocabulary.
///
/// The Slice 6b precedent: it is not enough to assert the relations we expect are present. Every
/// *other* member of `Relation::ALL` must be absent, or a future rule could add a call-shaped edge
/// and no test would notice.
#[test]
fn the_framework_extractor_asserts_no_relation_but_served_by() {
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
                "py-framework asserted {relation}, which it does not declare. A route \
                 registration is not a call, a containment, or a coverage fact."
            );
        }
    }
}

/// The unsupported tally matches by form, exactly.
///
/// Slice 9b's gate 7 applied to a new extractor: the counts make the precision denominator
/// auditable, and a construct that becomes unreadable without anyone noticing fails the build
/// rather than quietly shrinking the numerator.
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
        .map(|(form, count)| ((*form).to_string(), *count))
        .collect();

    assert_eq!(
        have, want,
        "the unsupported tally moved. Every form here is a construct Nerve read and declined; a \
         new one means the precision denominator changed, and an absent one means a form no code \
         produces — an untested member of the vocabulary."
    );

    // Every counted form must be a member of the closed vocabulary.
    let known: BTreeSet<&str> = nerve_index::PyFrameworkUnsupportedForm::ALL
        .iter()
        .map(|form| form.as_str())
        .collect();
    for form in have.keys() {
        assert!(
            known.contains(form.as_str()),
            "{form} is counted but is not a member of UnsupportedForm::ALL"
        );
    }
}

/// `negative.py` contributes zero endpoints **and** zero unsupported counts.
///
/// The second half is the part that is easy to get wrong. Nerve does not know that
/// `@cache.get("/x")` was meant to be a route, so counting it as one Nerve missed would be a false
/// claim about the repository in the opposite direction from a false positive.
#[test]
fn framework_shaped_code_that_is_not_a_route_contributes_nothing() {
    let ((_dir, root), _outcome) = indexed_named_fixture(FIXTURE);
    let conn = open_db(&root);

    let from_negative: i64 = conn
        .query_row(
            "SELECT count(*) FROM entity WHERE kind = 'endpoint' AND scope_path = 'negative.py'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(
        from_negative, 0,
        "negative.py declares no routes; every endpoint from it is a false positive"
    );

    // And the file must have been indexed, or the assertion above is vacuous.
    //
    // Joined through `occurrence.file_path`, not `entity.scope_path`: a Python symbol's scope_path
    // is its enclosing *lexical* scope — empty for a top-level function — so matching the file name
    // against it finds nothing and the guard would fire on a correctly indexed repository. It did.
    let symbols: i64 = conn
        .query_row(
            "SELECT count(*) FROM entity e
               JOIN occurrence o ON o.entity_id = e.entity_id
              WHERE e.kind IN ('function', 'method', 'class') AND o.file_path = 'negative.py'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert!(
        symbols > 0,
        "negative.py produced no symbols, so it was never really indexed and the check above \
         proves nothing"
    );
}

/// An endpoint is findable by a fragment of its path, which is the second measured defect closed.
#[test]
fn a_route_is_searchable_by_its_path() {
    let ((_dir, root), _outcome) = indexed_named_fixture(FIXTURE);
    let conn = open_db(&root);

    let hits = nerve_store::search_entities(&conn, "users", None, 25).unwrap();
    let addresses: Vec<String> = hits
        .iter()
        .filter(|hit| hit.kind == "endpoint")
        .map(|hit| hit.name.clone())
        .collect();
    assert!(
        addresses.iter().any(|address| address.contains("/users")),
        "searching for a path fragment found no endpoint; got {addresses:?}"
    );
}
