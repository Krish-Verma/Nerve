//! `/api/memory*` — human-confirmed memory over HTTP (Slice 14c).
//!
//! Every assertion here is about **content**. A status code establishes that a route dispatched and
//! nothing else, so wherever a test asserts an absence — that no write verb is served, that no
//! derived value is offered as a filter — a nonzero tally is asserted beside it, and the database
//! is hashed rather than inspected.
//!
//! Four properties are load-bearing:
//!
//! 1. **The surface is read-only, on the bytes.** `POST`, `PUT`, `PATCH` and `DELETE` each answer
//!    `405`, and the database is byte-identical across a session that includes all four attempts
//!    plus every read. That is row 14 §5's promise and Slice 4a's, applied to this family.
//! 2. **A refused filter is not an empty answer.** An unknown scope or status is a `400` naming the
//!    admitted set. Both halves are checked, because a route that refused everything would pass the
//!    first on its own.
//! 3. **Stored and derived stay apart**, under different keys, and no derived view can be filtered
//!    on.
//! 4. **This surface and the command line answer the same question.** The CLI's renderer is read
//!    out of its own source and its key set compared against the one served here, so a field added
//!    to one and forgotten in the other fails rather than leaving a client reading a different
//!    answer depending on where it asked. Value agreement needs both surfaces running at once and
//!    is asserted in `scripts/final_acceptance.sh`, which is where 13d-ii's equivalent lives.

mod common;

use std::path::Path;

use nerve_core::ids::content_hash;
use serde_json::Value;

/// Every memory route, with arguments where one is required.
///
/// Taken from `router::ROUTES` rather than retyped, so a route added later is gated by the tests
/// below without this file being edited.
fn memory_targets() -> Vec<String> {
    nerve_server::router::ROUTES
        .iter()
        .filter(|route| route.starts_with("/api/memory"))
        .map(|route| match *route {
            "/api/memory/record" => format!("{route}?memory_id=m1"),
            other => other.to_string(),
        })
        .collect()
}

fn record<'a>(answer: &'a Value, memory_id: &str) -> &'a Value {
    answer["records"]
        .as_array()
        .unwrap_or_else(|| panic!("no records array in {answer}"))
        .iter()
        .find(|record| record["memory_id"] == memory_id)
        .unwrap_or_else(|| panic!("no record {memory_id} in {answer}"))
}

fn ids(answer: &Value) -> Vec<String> {
    answer["records"]
        .as_array()
        .unwrap()
        .iter()
        .map(|record| record["memory_id"].as_str().unwrap().to_string())
        .collect()
}

// ---- the list ----------------------------------------------------------------------------------

/// Every record is served, retired ones included, each with its stored lifecycle and its views.
///
/// A read that hid a superseded or invalidated record would make *"what did we once believe and no
/// longer do"* unanswerable at exactly the moment it becomes the question — so all four stored
/// statuses are asserted present, and `superseded` and `invalidated` are asserted **different**.
#[test]
fn every_record_is_listed_with_its_stored_lifecycle_and_its_derived_views() {
    let (_dir, _root, session) = common::served_memory();
    let value = session.json("/api/memory?limit=200");

    assert_eq!(value["result_kind"], "memory_records");
    assert_eq!(
        value["records_in_repository"],
        common::MEMORY_WORLD_IDS.len()
    );
    assert_eq!(value["records_matching"], common::MEMORY_WORLD_IDS.len());
    assert_eq!(ids(&value), common::MEMORY_WORLD_IDS.to_vec());
    assert_eq!(value["absence_statement"], Value::Null);

    // The four stored statuses, and the two retirements kept apart.
    let statuses: Vec<&str> = value["records"]
        .as_array()
        .unwrap()
        .iter()
        .map(|record| record["status"].as_str().unwrap())
        .collect();
    for expected in ["proposed", "active", "superseded", "invalidated"] {
        assert!(statuses.contains(&expected), "{expected} is not served");
    }
    let superseded = record(&value, "m4");
    let invalidated = record(&value, "m6");
    assert_ne!(superseded["status"], invalidated["status"]);
    assert_eq!(superseded["superseded_by_memory_id"], "m5");
    assert_eq!(superseded["superseded_by_is_derived"], true);
    assert_eq!(superseded["invalidated_at"], Value::Null);
    assert_eq!(invalidated["superseded_by_memory_id"], Value::Null);
    assert!(invalidated["invalidated_at"].is_string());
    assert_eq!(invalidated["invalidation_reason"], common::HOSTILE_REASON);
    assert_eq!(record(&value, "m5")["supersedes_memory_id"], "m4");

    // Stored and derived are different kinds, under different keys, and the views are measured
    // rather than constant: two records share a subject, a scope and a claim key and are reported
    // conflicted; a third shares none of them and carries no view at all.
    let conflicted: Vec<&str> = record(&value, "m1")["views"]
        .as_array()
        .unwrap()
        .iter()
        .map(|view| view["view"].as_str().unwrap())
        .collect();
    assert!(conflicted.contains(&"conflicted"), "{conflicted:?}");
    assert!(conflicted.contains(&"multiple_active"), "{conflicted:?}");
    assert_eq!(record(&value, "m1")["views_are_derived"], true);
    assert_eq!(record(&value, "m3")["views"], serde_json::json!([]));
    for view in record(&value, "m1")["views"].as_array().unwrap() {
        assert!(
            view["note"].as_str().unwrap().len() > 20,
            "a view arrived without its own sentence: {view}"
        );
    }
    // And no view is ever spelled as a status.
    for record in value["records"].as_array().unwrap() {
        assert!(
            !nerve_server::api::memory::view_vocabulary()
                .contains(&record["status"].as_str().unwrap()),
            "a derived view was served as a stored status: {record}"
        );
    }

    // The record outlives what it is about: the subject is a snapshot, and what it reaches now is a
    // reported verdict rather than a pointer.
    let subject = &record(&value, "m1")["subject"];
    assert_eq!(subject["path"], common::HOSTILE_FILE);
    assert_eq!(record(&value, "m1")["subject_resolution"], "resolved");
    assert!(record(&value, "m1")["subject_resolution_note"].is_string());
    assert_eq!(
        record(&value, "m1")["subject_live_entity_ids"]
            .as_array()
            .unwrap()
            .len(),
        1
    );

    // The boundary is on the answer rather than in a document, and it names commands.
    assert_eq!(value["boundary"]["read_only"], true);
    assert_eq!(
        value["boundary"]["commands"],
        serde_json::json!(nerve_server::api::memory::BOUNDARY_COMMANDS)
    );
    assert!(value["limitations"]["no_delete_verb"]
        .as_str()
        .unwrap()
        .contains("invalidate"));
}

/// The four filters narrow, and the id list they produce is a subset rather than a re-ordering.
#[test]
fn each_filter_narrows_and_a_filter_that_matches_nothing_says_which_absence_it_is() {
    let (_dir, _root, session) = common::served_memory();

    let by_scope = session.json("/api/memory?scope=process");
    assert_eq!(ids(&by_scope), vec!["m3"]);
    assert_eq!(by_scope["records_matching"], 1);
    // The denominator survives the filter, so an answer can never be read as "this is everything".
    assert_eq!(
        by_scope["records_in_repository"],
        common::MEMORY_WORLD_IDS.len()
    );
    assert_eq!(by_scope["requested"]["scope"], "process");

    let by_status = session.json("/api/memory?status=proposed");
    assert_eq!(ids(&by_status), vec!["m3", "m5"]);

    let by_subject = session.json("/api/memory?subject=src%2Fmath.ts");
    assert_eq!(ids(&by_subject), vec!["m3", "m4", "m5", "m6"]);
    // The resolver said which stage matched, like every other endpoint that takes a selector.
    assert!(by_subject["selectors"]["subject"]["matched_by"].is_string());

    let by_query = session.json("/api/memory?q=retry%20budget");
    assert_eq!(ids(&by_query), vec!["m6"]);

    // Two filters compose, and the narrower one wins without either being ignored.
    let both = session.json("/api/memory?subject=src%2Fmath.ts&status=proposed");
    assert_eq!(ids(&both), vec!["m3", "m5"]);
    let none = session.json("/api/memory?subject=src%2Fmath.ts&scope=implementation");
    assert_eq!(ids(&none), Vec::<String>::new());
    assert_eq!(none["result_kind"], "no_memory_matches");
    assert!(none["absence_statement"]
        .as_str()
        .unwrap()
        .contains("accepted"));

    // And the other absence is a different answer, on a repository where nothing was ever written.
    let (_empty_dir, _empty_root, empty) = common::served_without_memory();
    let nothing = empty.json("/api/memory");
    assert_eq!(nothing["result_kind"], "no_memory_recorded");
    assert_eq!(nothing["records_in_repository"], 0);
    assert_ne!(nothing["result_kind"], none["result_kind"]);
    assert_ne!(nothing["absence_statement"], none["absence_statement"]);
}

/// `absence is not zero`, on the surface that would otherwise answer it.
///
/// Both halves: a misspelling is refused with the admitted set, and a legal value that matches
/// nothing is not. A route that refused everything would pass the first on its own.
#[test]
fn an_unknown_scope_or_status_is_refused_by_name_rather_than_answered_with_an_empty_list() {
    let (_dir, _root, session) = common::served_memory();

    let response = session.get("/api/memory?scope=opertions");
    assert_eq!(response.status, 400);
    let error = response.parse_json();
    assert_eq!(error["error"]["code"], "unknown_scope");
    assert_eq!(error["error"]["detail"]["this_is_not_an_empty_list"], true);
    assert_eq!(
        error["error"]["detail"]["allowed"],
        serde_json::json!(nerve_server::api::memory::scope_vocabulary())
    );
    assert!(error["records"].is_null(), "a refusal carried records");

    // A derived view asked for as a status is refused **and named as one**, which is the case an
    // empty list would have answered "nothing is stale" to.
    for view in nerve_server::api::memory::view_vocabulary() {
        let response = session.get(&format!("/api/memory?status={view}"));
        assert_eq!(response.status, 400, "{view}");
        let error = response.parse_json();
        assert_eq!(error["error"]["code"], "unknown_status", "{view}");
        assert_eq!(
            error["error"]["detail"]["named_a_derived_view"], true,
            "{view}"
        );
        assert_eq!(
            error["error"]["detail"]["derived_views"],
            serde_json::json!(nerve_server::api::memory::view_vocabulary())
        );
    }
    let unknown = session.get("/api/memory?status=banana").parse_json();
    assert_eq!(unknown["error"]["detail"]["named_a_derived_view"], false);

    // And every admitted value really is admitted, so the refusals above are about the vocabulary
    // rather than about the parameter being unusable.
    let mut answered = 0;
    for scope in nerve_server::api::memory::scope_vocabulary() {
        assert_eq!(
            session.get(&format!("/api/memory?scope={scope}")).status,
            200
        );
        answered += 1;
    }
    for status in nerve_server::api::memory::status_vocabulary() {
        assert_eq!(
            session.get(&format!("/api/memory?status={status}")).status,
            200
        );
        answered += 1;
    }
    assert_eq!(answered, 8, "the admitted sets were not driven");
}

/// A bounded page, with truncation as a comparison rather than the guess `returned == limit`.
#[test]
fn the_list_is_bounded_and_the_page_continues_exactly() {
    let (_dir, _root, session) = common::served_memory();
    let total = common::MEMORY_WORLD_IDS.len();

    let cut = session.json("/api/memory?limit=2");
    assert_eq!(ids(&cut), vec!["m1", "m2"]);
    assert_eq!(cut["truncation"]["truncated"], true);
    assert_eq!(cut["truncation"]["total"], total);
    assert_eq!(cut["continuation"]["supported"], true);
    assert_eq!(cut["continuation"]["next_offset"], 2);

    let next = session.json("/api/memory?limit=2&offset=2");
    assert_eq!(ids(&next), vec!["m3", "m4"]);

    // The case `returned == limit` gets wrong: a page ending exactly on the total is not a cut.
    let exact = session.json(&format!("/api/memory?limit={total}"));
    assert_eq!(exact["truncation"]["returned"], total);
    assert_eq!(exact["truncation"]["truncated"], false);
    assert_eq!(exact["continuation"]["next_offset"], Value::Null);
}

// ---- one record --------------------------------------------------------------------------------

/// One record whole, with its citations and its complete audit history.
#[test]
fn one_record_is_served_with_every_citation_and_every_event_it_has() {
    let (_dir, _root, session) = common::served_memory();
    let value = session.json("/api/memory/record?memory_id=m1");

    assert_eq!(value["result_kind"], "memory_record");
    assert_eq!(value["records_matching"], 1);
    assert_eq!(value["continuation"]["supported"], false);
    assert!(value["continuation"]["statement"].is_string());

    let note = record(&value, "m1");
    assert_eq!(note["content"], common::HOSTILE_NOTE);
    assert_eq!(note["author_label"], common::HOSTILE_AUTHOR);
    assert_eq!(note["author_label_is_an_identity"], false);
    assert_eq!(note["claim_key"], common::HOSTILE_CLAIM_KEY);
    assert_eq!(note["scope"], "implementation");
    assert!(note["scope_note"].is_string());

    // The whole history, oldest first, including the one event that changed no status.
    let operations: Vec<&str> = note["events"]
        .as_array()
        .unwrap()
        .iter()
        .map(|event| event["operation"].as_str().unwrap())
        .collect();
    assert_eq!(operations, vec!["proposed", "confirmed", "cited"]);
    let proposed = &note["events"][0];
    assert_eq!(proposed["from_status"], Value::Null);
    assert_eq!(proposed["to_status"], "proposed");
    let confirmed = &note["events"][1];
    assert_eq!(confirmed["note"], common::HOSTILE_EVENT_NOTE);
    assert_eq!(confirmed["changes_status"], true);
    let cited = &note["events"][2];
    assert_eq!(cited["changes_status"], false);
    assert_eq!(cited["from_status"], cited["to_status"]);

    assert_eq!(note["citations"][0]["cited_path"], common::HOSTILE_FILE);
    assert_eq!(note["citations"][0]["cited_span"], "1:2");

    // The predecessor's history survived its retirement — the row's own property, on this surface.
    let retired = session.json("/api/memory/record?memory_id=m4");
    let operations: Vec<&str> = record(&retired, "m4")["events"]
        .as_array()
        .unwrap()
        .iter()
        .map(|event| event["operation"].as_str().unwrap())
        .collect();
    assert_eq!(operations, vec!["proposed", "confirmed", "superseded"]);
}

/// **Every field a person typed is served as an escaped string, and none of it is lost.** T7, and
/// the widest instance of it in the product.
///
/// Slice 14d puts these values on a page. A memory record is the one thing in this database a human
/// wrote, so the note itself, the author label, the claim key, the reason it ended, an event's note,
/// a citation's path and every field of the subject snapshot are all free text — and on a project
/// that accepts contributions, all of it is attacker-influencable. `respond::to_json_bytes` escapes
/// `<`, `>` and `&` on the way out, which is what makes the bytes safe to hand to a renderer, and
/// this asserts it for **this family of routes** rather than trusting the general test: a route
/// that assembled its body some other way would pass that one and fail here.
///
/// Both halves are asserted, because either alone is worthless. The bytes carry no raw `<` — and
/// the payload still decodes to exactly what was typed, which is what makes the escaping a
/// rendering decision rather than a lossy filter that quietly damages a person's note.
#[test]
fn every_field_a_person_typed_is_served_escaped_and_decodes_back_unchanged() {
    let (_dir, _root, session) = common::served_memory();

    for target in [
        "/api/memory?limit=200",
        "/api/memory/record?memory_id=m1",
        "/api/memory/record?memory_id=m6",
    ] {
        let response = session.get(target);
        assert_eq!(response.status, 200, "{target}: {}", response.body);
        for character in ['<', '>', '&'] {
            assert!(
                !response.body.contains(character),
                "{target} served a raw {character:?}:\n{}",
                response.body
            );
        }
        // Anti-vacuity. A body with nothing hostile in it would satisfy the three assertions above
        // by carrying no repository text at all, which is the way this check goes quietly empty.
        assert!(
            response.body.contains("\\u003c"),
            "{target} carries no escaped payload, so the check above proved nothing:\n{}",
            response.body
        );
    }

    // And nothing was damaged on the way. Seven fields, each one text somebody typed, each read
    // back through a JSON decode and compared byte for byte against what was written.
    let listed = session.json("/api/memory?limit=200");
    let note = record(&listed, "m1");
    assert_eq!(note["content"], common::HOSTILE_NOTE);
    assert_eq!(note["author_label"], common::HOSTILE_AUTHOR);
    assert_eq!(note["claim_key"], common::HOSTILE_CLAIM_KEY);
    assert_eq!(note["subject"]["path"], common::HOSTILE_FILE);
    assert_eq!(note["subject"]["selector"], common::HOSTILE_FILE);
    assert_eq!(note["citations"][0]["cited_path"], common::HOSTILE_FILE);
    assert_eq!(note["events"][1]["note"], common::HOSTILE_EVENT_NOTE);
    assert_eq!(
        record(&listed, "m6")["invalidation_reason"],
        common::HOSTILE_REASON
    );

    // The prompt-injection prose is carried as a **string value** and never as a key, a vocabulary
    // member or a code, which is the property that keeps it renderable as text. Asserted by
    // walking the whole answer rather than by inspecting the fields above.
    fn keys(value: &Value, into: &mut Vec<String>) {
        match value {
            Value::Object(map) => {
                for (key, item) in map {
                    into.push(key.clone());
                    keys(item, into);
                }
            }
            Value::Array(items) => items.iter().for_each(|item| keys(item, into)),
            _ => {}
        }
    }
    let mut every_key = Vec::new();
    keys(&listed, &mut every_key);
    assert!(every_key.len() > 100, "the walk found {}", every_key.len());
    for key in &every_key {
        assert!(
            !key.contains('<') && !key.contains("DISREGARD"),
            "repository text reached a JSON key: {key}"
        );
    }
}

/// A record that is not here is a refusal, never an empty one.
#[test]
fn an_unknown_memory_id_is_a_refusal_rather_than_an_empty_record() {
    let (_dir, _root, session) = common::served_memory();
    let response = session.get("/api/memory/record?memory_id=nope");
    assert_eq!(response.status, 404);
    let error = response.parse_json();
    assert_eq!(error["error"]["code"], "memory_record_not_found");
    assert_eq!(
        error["error"]["detail"]["this_is_not_an_empty_record"],
        true
    );
    // The refusal knows there *are* records, which is what tells it apart from an empty repository.
    assert_eq!(
        error["error"]["detail"]["records_in_repository"],
        common::MEMORY_WORLD_IDS.len()
    );

    let missing = session.get("/api/memory/record");
    assert_eq!(missing.status, 400);
}

// ---- read-only ---------------------------------------------------------------------------------

/// Row 14 §5 and acceptance criterion 7, on the bytes.
///
/// Every write verb is refused **before routing**, so there is no memory route that could be
/// reached by a forged form submission — and the database is byte-identical across a session that
/// includes all four attempts as well as every read.
#[test]
fn no_memory_route_is_reachable_by_a_write_verb_and_the_database_never_changes() {
    let (_dir, root, session) = common::served_memory();
    let db_path = nerve_index::config::db_path(&root);
    let before = content_hash(&std::fs::read(&db_path).unwrap());

    let mut answered = 0;
    for target in [
        "/api/memory",
        "/api/memory?limit=200",
        "/api/memory?scope=implementation",
        "/api/memory?status=active",
        "/api/memory?subject=src%2Fmath.ts",
        "/api/memory?q=audited",
        "/api/memory/record?memory_id=m1",
        "/api/memory/record?memory_id=m6",
    ] {
        let value = session.json(target);
        assert_eq!(value["ok"], true, "{target}");
        answered += 1;
    }
    // Anti-vacuity: the reads really produced answers, so "unchanged" is not "nothing ran".
    assert_eq!(answered, 8);
    assert_eq!(
        session.json("/api/memory?limit=200")["records"]
            .as_array()
            .unwrap()
            .len(),
        common::MEMORY_WORLD_IDS.len()
    );

    let mut refused = 0;
    for target in memory_targets() {
        for method in ["POST", "PUT", "DELETE", "PATCH"] {
            let response = session.raw(
                method,
                &target,
                &[
                    ("Host", &session.host()),
                    (nerve_server::token::TOKEN_HEADER, session.token()),
                ],
            );
            assert_eq!(response.status, 405, "{method} {target}");
            assert_eq!(response.parse_json()["error"]["code"], "method_not_allowed");
            refused += 1;
        }
    }
    // Two routes, four verbs each. A `memory_targets()` that silently found nothing would make the
    // loop above pass by iterating an empty list.
    assert_eq!(refused, 8, "the write verbs were not driven");

    assert_eq!(
        before,
        content_hash(&std::fs::read(&db_path).unwrap()),
        "a memory session wrote to the index"
    );
}

/// The two memory routes are advertised, gated and dispatched like every other route.
#[test]
fn the_memory_routes_are_advertised_and_gated() {
    let (_dir, _root, session) = common::served_memory();
    let mut checked = 0;
    for route in ["/api/memory", "/api/memory/record"] {
        assert!(
            nerve_server::router::ROUTES.contains(&route),
            "{route} is served and not advertised"
        );
        // No token: refused before dispatch, like every other `/api/*` route.
        let response = session.raw("GET", route, &[("Host", &session.host())]);
        assert_eq!(response.status, 401, "{route}");
        checked += 1;
    }
    assert_eq!(checked, 2);
}

// ---- the two surfaces answer the same question ---------------------------------------------

/// Read the CLI's memory renderer out of its own source and take the keys it emits.
///
/// `crates/nerve-cli/tests/history_wording.rs` already scans this crate's source from the other
/// side, for the same reason: the two surfaces render one answer, and a scan that fails loudly is
/// worth more than a convention nobody rechecks. Reading the source rather than running the binary
/// keeps this a `cargo test` away from a developer, and `CARGO_BIN_EXE_nerve` is not defined for
/// this crate's tests in any case.
fn cli_json_keys(function: &str) -> Vec<String> {
    let source = std::fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../nerve-cli/src/main.rs"),
    )
    .expect("the CLI source must be readable");
    let start = source
        .find(&format!("fn {function}("))
        .unwrap_or_else(|| panic!("{function} is not in the CLI source"));
    let end = source[start..]
        .find("\n}\n")
        .unwrap_or_else(|| panic!("{function} has no end"));
    let mut keys: Vec<String> = source[start..start + end]
        .lines()
        .filter_map(|line| {
            let line = line.trim_start();
            let rest = line.strip_prefix('"')?;
            let (key, tail) = rest.split_once('"')?;
            tail.trim_start().starts_with(':').then(|| key.to_string())
        })
        .collect();
    keys.sort();
    keys.dedup();
    // Anti-vacuity: a scrape that matched nothing would compare two empty sets and pass.
    assert!(keys.len() >= 5, "{function} scraped only {keys:?}");
    keys
}

fn served_keys(value: &Value) -> Vec<String> {
    let mut keys: Vec<String> = value
        .as_object()
        .unwrap_or_else(|| panic!("not an object: {value}"))
        .keys()
        .cloned()
        .collect();
    keys.sort();
    keys
}

/// The record this surface serves is the record `nerve memory show --json` prints, field for field.
///
/// Not "both have a content field" but "the two key sets are equal", for the record and for each of
/// its three nested shapes. A field added to one and forgotten in the other leaves a client reading
/// a different answer depending on where it asked, which is the drift ARCHITECTURE.md invariant 3
/// exists to prevent — and the same cross-surface agreement 13d-ii asserted for a link's freshness.
#[test]
fn the_served_record_carries_exactly_the_fields_the_command_line_prints() {
    let (_dir, _root, session) = common::served_memory();
    let value = session.json("/api/memory/record?memory_id=m1");
    let note = record(&value, "m1");

    for (function, served) in [
        ("memory_json", note),
        ("memory_subject_json", &note["subject"]),
        ("memory_citation_json", &note["citations"][0]),
        ("memory_event_json", &note["events"][0]),
    ] {
        assert_eq!(
            served_keys(served),
            cli_json_keys(function),
            "{function} and the served shape disagree"
        );
    }

    // And the values that can be compared without a running binary are the stored row itself,
    // which is what makes the key comparison above a comparison of the same answer.
    assert_eq!(note["memory_id"], "m1");
    assert_eq!(note["status"], "active");
    assert_eq!(note["content"], common::HOSTILE_NOTE);
    assert_eq!(note["events"][0]["operation"], "proposed");
    assert_eq!(note["citations"][0]["cited_span"], "1:2");
}

/// Every `nerve memory` verb the CLI declares is named by the boundary, and nothing else is.
///
/// The boundary's whole job is to be the thing a reader who cannot press a button is given
/// instead, so a command named here that does not exist is worse than no list at all — and a verb
/// the CLI grew that this list never learned is a capability the surface hides.
#[test]
fn the_boundary_names_every_memory_verb_the_command_line_declares() {
    let source = std::fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../nerve-cli/src/main.rs"),
    )
    .expect("the CLI source must be readable");
    let start = source
        .find("enum MemoryCommand {")
        .expect("the CLI must declare a memory command family");
    let end = source[start..].find("\n}\n").expect("the enum must end");

    let mut declared: Vec<String> = source[start..start + end]
        .lines()
        .filter_map(|line| {
            let variant = line.strip_prefix("    ")?.strip_suffix(" {")?;
            variant
                .chars()
                .all(|character| character.is_ascii_alphabetic())
                .then(|| variant.to_ascii_lowercase())
        })
        .collect();
    declared.sort();
    // Anti-vacuity: the scrape found the family rather than nothing.
    assert!(declared.len() >= 8, "scraped only {declared:?}");
    assert!(!declared.contains(&"delete".to_string()), "{declared:?}");

    let mut named: Vec<String> = nerve_server::api::memory::BOUNDARY_COMMANDS
        .iter()
        .map(|command| {
            command
                .split_whitespace()
                .nth(2)
                .unwrap_or_default()
                .to_string()
        })
        .collect();
    named.sort();
    assert_eq!(
        named, declared,
        "the boundary and the command family disagree about which verbs exist"
    );
}
