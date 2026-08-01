//! Measured precision for `md-structural`'s link resolution over `fixtures/md-links`.
//!
//! ADR-0003 answers "how confident are we?" with an extractor's measured precision on its fixture
//! corpus, not with a per-row number. This file is that measurement for document links, and like
//! `precision.rs` it is a **gate rather than a report**:
//!
//! - **false positives must be zero** — every resolved edge in the database is declared in
//!   `expected.json`, and nothing else resolves;
//! - every declared edge must be present, so recall is measured rather than asserted;
//! - every unresolved edge must be declared *with its reason*, from the closed vocabulary;
//! - every destination declared **silent** must produce no row anywhere — a link in a fence, a
//!   link in a code span, a bare code-span name, a heading fragment and an external URL are all
//!   things Nerve declines to model, and declining is not the same as failing.
//!
//! If a case here stops passing, the rule is what moves. Lowering the gate would turn a
//! measurement into a description of whatever the code happens to do.
//!
//! Run `cargo test -p nerve-index --test document_links -- --nocapture` to see the table.

mod common;

use std::collections::{BTreeMap, BTreeSet};

use common::{indexed_named_fixture, open_db};
use nerve_index::docref::{outcome, reason};

const FIXTURE: &str = "md-links";

/// One `REFERENCES` edge emitted by `md-structural`, with the evidence a failure needs.
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
    raw_destination: String,
    link_target: String,
    source_kind: String,
    reason: Option<String>,
    resolved_path: Option<String>,
    target_content_hash: Option<String>,
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

/// Build the selector -> entity id map the ground truth is written in, refusing anything
/// ambiguous.
///
/// Four shapes, each naming exactly one kind of thing so that a typo in `expected.json` cannot
/// silently match the wrong entity:
///
/// - `file:<path>` — the `File` entity a resolved destination names
/// - `doc:<path>` — the `Document` entity, which is the source of a link written before the
///   first heading
/// - `sec:<path>#<heading path>` — a `Section`, addressed by the `>`-joined heading chain that
///   its own metadata records
/// - `sym:<path>#<scope>.<name>` — a symbol a `#L<n>` anchor resolved to
fn selectors(conn: &nerve_store::Connection) -> BTreeMap<String, String> {
    let mut candidates: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    let mut stmt = conn
        .prepare(
            "SELECT e.entity_id, e.kind, e.name, e.scope_path, e.meta, o.file_path
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
                row.get::<_, Option<String>>(4)?,
                row.get::<_, String>(5)?,
            ))
        })
        .unwrap();

    for row in rows {
        let (entity_id, kind, name, scope_path, meta, file_path) = row.unwrap();
        let selector = match kind.as_str() {
            "file" => format!("file:{file_path}"),
            "document" => format!("doc:{file_path}"),
            "section" => {
                let heading_path = meta
                    .as_deref()
                    .and_then(|raw| serde_json::from_str::<serde_json::Value>(raw).ok())
                    .and_then(|value| {
                        value
                            .get("heading_path")
                            .and_then(|path| path.as_str().map(str::to_string))
                    })
                    .unwrap_or_else(|| panic!("section {entity_id} has no heading_path"));
                format!("sec:{file_path}#{heading_path}")
            }
            "function" | "method" | "class" | "interface" => {
                if scope_path.is_empty() {
                    format!("sym:{file_path}#{name}")
                } else {
                    format!("sym:{file_path}#{scope_path}.{name}")
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
            let prefix = selector.split('#').next().unwrap_or(selector);
            let near: Vec<&String> = map.keys().filter(|key| key.starts_with(prefix)).collect();
            panic!("selector {selector:?} matches no entity. Near it: {near:#?}")
        })
        .clone()
}

/// Every `REFERENCES` edge `md-structural` emitted, with its details unpacked.
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
              WHERE a.relation = 'REFERENCES' AND o.extractor_id = 'md-structural'
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
        let details: serde_json::Value = serde_json::from_str(
            details
                .as_deref()
                .unwrap_or_else(|| panic!("a link observation in {file_path} has no details")),
        )
        .expect("observation details must be JSON");
        let text = |key: &str| -> Option<String> {
            details
                .get(key)
                .and_then(|value| value.as_str().map(str::to_string))
        };
        let from_label = reverse
            .get(from.as_str())
            .map(|selector| (*selector).to_string())
            .unwrap_or_else(|| from.clone());
        let to_label = reverse
            .get(to.as_str())
            .map(|selector| (*selector).to_string())
            .unwrap_or_else(|| format!("<unresolved {name:?}>"));
        out.push(Edge {
            from,
            to,
            from_label,
            to_label,
            target_kind: kind,
            target_name: name,
            source_type,
            directness,
            file_path,
            start_line,
            raw_destination: text("raw_destination").expect("raw_destination"),
            link_target: text("link_target").expect("link_target"),
            source_kind: text("source_kind").expect("source_kind"),
            reason: text("reason"),
            resolved_path: text("resolved_path"),
            target_content_hash: text("target_content_hash"),
        });
    }
    out
}

/// Sum one outcome counter over every document's `links` metadata.
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
        meta["links"]
            .get(key)
            .and_then(serde_json::Value::as_u64)
            .unwrap_or(0) as usize
    })
    .sum()
}

fn code_span_mentions(conn: &nerve_store::Connection) -> usize {
    let mut stmt = conn
        .prepare("SELECT meta FROM entity WHERE kind = 'document'")
        .unwrap();
    let rows = stmt
        .query_map([], |row| row.get::<_, Option<String>>(0))
        .unwrap();
    rows.map(|row| {
        let meta: serde_json::Value = serde_json::from_str(&row.unwrap().expect("document meta"))
            .expect("document meta must be JSON");
        meta["code_span_mentions"].as_u64().unwrap_or(0) as usize
    })
    .sum()
}

#[test]
fn measured_document_link_precision_meets_the_slice_5c_gates() {
    let ((_dir, root), outcome_report) = indexed_named_fixture(FIXTURE);
    let conn = open_db(&root);
    let truth = ground_truth();
    let names = selectors(&conn);
    let all_edges = edges(&conn, &names);
    let mut failures: Vec<String> = Vec::new();

    // ---- declared resolved edges ----------------------------------------------------------
    let mut expected_resolved: BTreeMap<(String, String), String> = BTreeMap::new();
    for entry in truth["resolved"].as_array().expect("resolved array") {
        let from = entry["from"].as_str().expect("from");
        let to = entry["to"].as_str().expect("to");
        let note = entry["note"].as_str().unwrap_or_default().to_string();
        expected_resolved.insert((entity_of(&names, from), entity_of(&names, to)), note);
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
    // Counted per **edge**, not per observation. Two links in one section that name the same
    // file are one claim observed twice, and counting the observations would inflate both the
    // numerator and the denominator of a precision figure that is supposed to be about claims.
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
            "FALSE POSITIVE: {} -> {} at {} (destination {:?}, link_target {})",
            edge.from_label,
            edge.to_label,
            edge.location(),
            edge.raw_destination,
            edge.link_target
        ));
    }

    // ---- gate 2: every declared resolved edge is present ----------------------------------
    let mut false_negatives = 0usize;
    for entry in truth["resolved"].as_array().unwrap() {
        let from = entry["from"].as_str().unwrap();
        let to = entry["to"].as_str().unwrap();
        let key = (entity_of(&names, from), entity_of(&names, to));
        if !observed_resolved.contains(&key) {
            false_negatives += 1;
            let seen: Vec<String> = all_edges
                .iter()
                .filter(|edge| edge.from == key.0)
                .map(|edge| format!("{} at {}", edge.to_label, edge.location()))
                .collect();
            failures.push(format!(
                "FALSE NEGATIVE: {from} -> {to} ({}). Edges from that source: {seen:#?}",
                entry["note"].as_str().unwrap_or_default()
            ));
        }
    }

    // ---- gate 3: unresolved edges are declared, with their reasons ------------------------
    let observed_unresolved: BTreeSet<(String, String, String)> = all_edges
        .iter()
        .filter(|edge| !edge.is_resolved())
        .map(|edge| {
            (
                edge.from.clone(),
                edge.target_name.clone(),
                edge.reason.clone().unwrap_or_default(),
            )
        })
        .collect();
    let mut expected_unresolved: BTreeSet<(String, String, String)> = BTreeSet::new();
    for entry in truth["unresolved"].as_array().expect("unresolved array") {
        let from = entry["from"].as_str().expect("from");
        let target_name = entry["target_name"]
            .as_str()
            .expect("target_name")
            .to_string();
        let reason = entry["reason"].as_str().expect("reason").to_string();
        assert!(
            [
                reason::TARGET_NOT_INDEXED,
                reason::REFUSED,
                reason::ANCHOR_NO_SYMBOL
            ]
            .contains(&reason.as_str()),
            "expected.json uses reason {reason:?}, which is outside the closed vocabulary"
        );
        let key = (entity_of(&names, from), target_name.clone(), reason.clone());
        if !observed_unresolved.contains(&key) {
            let seen: Vec<String> = all_edges
                .iter()
                .filter(|edge| !edge.is_resolved() && edge.from == key.0)
                .map(|edge| format!("{:?} reason {:?}", edge.target_name, edge.reason))
                .collect();
            failures.push(format!(
                "MISSING UNRESOLVED: {from} -> {target_name:?} reason {reason:?}. \
                 Unresolved edges from that source: {seen:#?}"
            ));
        }
        expected_unresolved.insert(key);
    }
    for key in &observed_unresolved {
        if !expected_unresolved.contains(key) {
            failures.push(format!(
                "UNDECLARED UNRESOLVED: {} -> {:?} reason {:?}",
                all_edges
                    .iter()
                    .find(|edge| edge.from == key.0)
                    .map(|edge| edge.from_label.clone())
                    .unwrap_or_else(|| key.0.clone()),
                key.1,
                key.2
            ));
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

    // ---- gate 5: the innermost symbol wins, and the enclosing one is not also claimed -------
    //
    // The ambiguous case has a rule, and the rule is "innermost". Declaring only the winner
    // would leave "the class was claimed as well" passing, which is the failure that matters.
    for entry in truth["ambiguous"].as_array().expect("ambiguous array") {
        let from = entity_of(&names, entry["from"].as_str().expect("from"));
        let winner = entity_of(&names, entry["winner"].as_str().expect("winner"));
        let loser = entity_of(&names, entry["loser"].as_str().expect("loser"));
        let why = entry["why"].as_str().unwrap_or_default();
        if !observed_resolved.contains(&(from.clone(), winner)) {
            failures.push(format!(
                "AMBIGUITY: {} did not resolve to {} ({why})",
                entry["from"], entry["winner"]
            ));
        }
        if observed_resolved.contains(&(from, loser)) {
            failures.push(format!(
                "AMBIGUITY: {} also claimed {} ({why})",
                entry["from"], entry["loser"]
            ));
        }
    }

    // ---- gate 6: unsupported constructs produce no row at all ------------------------------
    //
    // Stronger than "no resolved edge": a destination in a fence, a destination in a code span,
    // a bare code-span name, a heading fragment and an external URL must produce no observation
    // and no `Unresolved` entity. "Nerve declined to model this" and "Nerve failed at this" are
    // different claims, and the graph must not blur them.
    //
    // The entity check is scoped to kind `unresolved` on purpose: a code-span mention of
    // `describe` must not invent an unresolved entity, and the real symbol called `describe`
    // legitimately exists under that name.
    let mut declared_by_outcome: BTreeMap<String, usize> = BTreeMap::new();
    for entry in truth["unsupported"].as_array().expect("unsupported array") {
        let destination = entry["destination"].as_str().expect("destination");
        let why = entry["why"].as_str().unwrap_or_default();
        *declared_by_outcome
            .entry(entry["outcome"].as_str().expect("outcome").to_string())
            .or_insert(0) += 1;
        if let Some(edge) = all_edges
            .iter()
            .find(|edge| edge.raw_destination == destination)
        {
            failures.push(format!(
                "NOT SILENT: {destination:?} produced {} -> {} at {} ({why})",
                edge.from_label,
                edge.to_label,
                edge.location()
            ));
        }
        let named: i64 = conn
            .query_row(
                "SELECT count(*) FROM entity WHERE kind = 'unresolved' AND name = ?1",
                [destination],
                |row| row.get(0),
            )
            .unwrap();
        if named != 0 {
            failures.push(format!(
                "NOT SILENT: {destination:?} became an unresolved entity ({why})"
            ));
        }
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
        // The target file's hash is recorded for an anchored symbol edge and for nothing else.
        let wants_hash = edge.link_target == "symbol";
        if wants_hash != edge.target_content_hash.is_some() {
            failures.push(format!(
                "TARGET HASH: {} -> {} at {} link_target {} target_content_hash {:?}",
                edge.from_label,
                edge.to_label,
                edge.location(),
                edge.link_target,
                edge.target_content_hash
            ));
        }
        if edge.source_kind != "section" && edge.source_kind != "document" {
            failures.push(format!(
                "SOURCE KIND: {:?} is outside the closed vocabulary",
                edge.source_kind
            ));
        }
    }

    // A symbol edge's recorded hash must be the target file's hash as this run read it, not the
    // citing document's. Getting those two the wrong way round is invisible without this check.
    for edge in all_edges.iter().filter(|edge| edge.link_target == "symbol") {
        let path = edge
            .resolved_path
            .clone()
            .expect("symbol edge resolved_path");
        let bytes = std::fs::read(root.join(&path)).expect("target file");
        let expected = nerve_core::ids::content_hash(&bytes);
        if edge.target_content_hash.as_deref() != Some(expected.as_str()) {
            failures.push(format!(
                "TARGET HASH: {} -> {} recorded {:?}, {path} hashes to {expected}",
                edge.from_label, edge.to_label, edge.target_content_hash
            ));
        }
    }

    // ---- the table -------------------------------------------------------------------------
    let resolved_edges = observed_resolved.len();
    let unresolved_edges = observed_unresolved.len();
    let true_positives = resolved_edges - false_positives;
    let declared = expected_resolved.len();

    // The counters the extractor itself kept must agree with the ground truth's own tally of
    // what it declined to model. A miscount here is a report that quietly stops being true.
    for (outcome_name, counter) in [
        ("external", outcome::EXTERNAL),
        ("fragment-only", outcome::FRAGMENT_ONLY),
    ] {
        let declared_count = declared_by_outcome.get(outcome_name).copied().unwrap_or(0);
        let observed_count = outcome_total(&conn, counter);
        if declared_count != observed_count {
            failures.push(format!(
                "UNSUPPORTED COUNT {outcome_name}: expected.json declares {declared_count}, \
                 the documents counted {observed_count}"
            ));
        }
    }

    let resolved_file = outcome_total(&conn, outcome::RESOLVED_FILE);
    let resolved_symbol = outcome_total(&conn, outcome::RESOLVED_SYMBOL);
    let external = outcome_total(&conn, outcome::EXTERNAL);
    let fragment_only = outcome_total(&conn, outcome::FRAGMENT_ONLY);
    let refused = outcome_total(&conn, reason::REFUSED);
    let not_indexed = outcome_total(&conn, reason::TARGET_NOT_INDEXED);
    let anchor_no_symbol = outcome_total(&conn, reason::ANCHOR_NO_SYMBOL);
    let mentions = code_span_mentions(&conn);
    // One site per destination the scanner recorded. `resolved_symbol` and `anchor_no_symbol`
    // are outcomes *of* a resolved file site, so counting them again would inflate the total.
    let sites = resolved_file + external + fragment_only + refused + not_indexed;
    // Link-shaped things Nerve declines to model: an external destination, a heading fragment, a
    // bare code-span mention, and any construct the scanner itself refused.
    let unsupported = external + fragment_only + mentions + outcome_report.unsupported_markdown;

    let percent = |numerator: usize, denominator: usize| -> f64 {
        if denominator == 0 {
            0.0
        } else {
            numerator as f64 * 100.0 / denominator as f64
        }
    };

    println!("\n=== md-structural link precision on fixtures/{FIXTURE} (fixture-only) ===");
    println!("sites scanned          {sites}");
    println!("  resolved to a file   {resolved_file}");
    println!("  resolved to a symbol {resolved_symbol}  (a further outcome of a resolved site)");
    println!("  external             {external}  (counted, never fetched)");
    println!("  fragment only        {fragment_only}  (heading anchors are not modelled)");
    println!(
        "  unresolved: refused {refused} · not indexed {not_indexed} · anchor {anchor_no_symbol}"
    );
    println!("code-span mentions     {mentions}  (counted, never emitted)");
    println!(
        "scanner-refused        {}  {:?}",
        outcome_report.unsupported_markdown, outcome_report.unsupported_markdown_by_form
    );
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
        "unsupported rate {:.1}%",
        percent(
            unsupported,
            sites + mentions + outcome_report.unsupported_markdown
        )
    );
    println!("=== end ===\n");

    assert!(
        failures.is_empty(),
        "document-link precision gates failed ({} problem(s)):\n{}",
        failures.len(),
        failures.join("\n")
    );
    assert_eq!(false_positives, 0, "the gate is zero false positives");
    assert!(
        resolved_edges > 0 && unresolved_edges > 0,
        "the corpus produced no edges of one kind; the gates proved nothing"
    );
}

/// `md-structural` must not grow an evidence type it did not declare, and must not take over a
/// relation that belongs to another extractor.
#[test]
fn the_document_extractor_stays_inside_its_declaration() {
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
        "the declared source type list is one element and must stay one element"
    );
    assert_eq!(
        distinct(
            "SELECT DISTINCT a.relation FROM assertion a
               JOIN observation o ON o.assertion_id = a.assertion_id
              WHERE o.extractor_id = 'md-structural' ORDER BY 1"
        ),
        vec!["CONTAINS".to_string(), "REFERENCES".to_string()]
    );

    // `SUPERSEDES` is declared and emitted by nothing. Slice 5c deliberately does not build it:
    // an ADR superseding another is a claim about decisions, and the evidence for it is not the
    // same evidence a path resolution provides.
    let supersedes: i64 = conn
        .query_row(
            "SELECT count(*) FROM assertion WHERE relation = 'SUPERSEDES'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(supersedes, 0);

    // No document-link row carries a scalar confidence. `match_quality` exists for matching
    // extractors, and this one performs no matching.
    let with_quality: i64 = conn
        .query_row(
            "SELECT count(*) FROM observation
              WHERE extractor_id = 'md-structural' AND match_quality IS NOT NULL",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(with_quality, 0);
}

/// The corpus indexes deterministically and re-indexing it changes nothing, with link resolution
/// in the loop. Resolution reads the whole repository, so it is exactly the kind of step that
/// could make a second run disagree with the first.
#[test]
fn the_link_corpus_indexes_deterministically() {
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
