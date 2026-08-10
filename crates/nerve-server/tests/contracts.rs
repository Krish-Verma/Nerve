//! `/api/contracts*` as behaviour: what a link says, what a registry says, and what neither may do.
//!
//! The endpoints are read-only, so the properties worth asserting are the ones a second renderer
//! would quietly lose:
//!
//! - a link carries **every** column a reader needs to tell a current link from one whose target
//!   moved, including the target snapshot that makes `contract_deleted`, `target_changed` and
//!   `contract_file_missing` distinguishable at all;
//! - a registry entry carries the verdict `nerve_index::availability_of` produced, and the
//!   **sentence that verdict owns**, rather than a phrasing invented here;
//! - the closed vocabularies are answerable, because §9.1 requires an unsupported form to be
//!   reported *with its form named* and a surface has to be able to name one;
//! - nothing mutates, and the neighbour's database is byte-identical after every read.
//!
//! A negative is never asserted alone. "No link changed" is satisfied by an endpoint that answers
//! nothing, so every byte-equality assertion is paired with an assertion that the read produced a
//! link in the first place.

mod common;

use serde_json::Value;

/// Every link this world records, as `/api/contracts` returns them.
fn links(session: &common::Session) -> Vec<Value> {
    let value = session.json("/api/contracts?limit=500");
    value["links"].as_array().cloned().unwrap_or_default()
}

/// One link, found by the pair that identifies it: its rule and the identity it was declared under.
///
/// Both are needed, because `pkg-map` is declared twice under one name and by two different rules —
/// once as a `file:` dependency between two repositories, and once as an import specifier resolved
/// through that repository's export map to a file inside it.
fn of<'a>(rows: &'a [Value], kind: &str, identity: &str) -> &'a Value {
    rows.iter()
        .find(|row| row["contract_kind"] == kind && row["contract_identity"] == identity)
        .unwrap_or_else(|| panic!("no {kind} link for {identity}"))
}

#[test]
fn a_recorded_link_carries_every_column_a_reader_needs() {
    let (world, session) = common::served_contracts(common::HOSTILE_DISPLAY_NAME);
    let rows = links(&session);
    assert_eq!(rows.len(), common::CONTRACT_WORLD_LINKS, "{rows:?}");

    // The C2 link the whole row exists for: a file in this repository names a file in another one,
    // through that other repository's own declared export map.
    let row = of(&rows, "npm_export_resolution", "pkg-map/sub");

    // What the declaration says.
    assert_eq!(row["relation_semantics"], "REFERENCES");
    assert_eq!(row["resolution_method"], "export_map_resolved");
    assert!(row["resolution_method_note"]
        .as_str()
        .unwrap()
        .contains("declared export"));

    // Both version columns exist as keys on every row, and neither produces a verdict.
    assert!(row.get("expected_contract_version").is_some());
    assert_eq!(row["observed_contract_version"], "3.1.0");

    // The local end.
    assert_eq!(row["source_path"], "src/app.ts");
    assert!(row["source_span"].as_str().unwrap().contains(':'));
    assert_eq!(row["source_manifest_present"], true);
    assert!(row["source_entity_id"].is_string());
    assert!(row["source_state_at_resolution"].is_string());

    // The far end, as a **snapshot**: what the neighbour looked like when the link was resolved,
    // so that a target which later moves, changes kind or vanishes is still nameable.
    assert!(row["expected_target_repository_id"]
        .as_str()
        .unwrap()
        .starts_with("repo_"));
    assert!(row["target_state_at_resolution"].is_string());
    assert!(row["target_entity_id"].is_string());
    assert_eq!(row["target_kind_snapshot"], "module");
    assert_eq!(row["target_path_snapshot"], "src/sub.ts");
    assert!(row["target_name_snapshot"].is_string());
    assert!(row["target_span_snapshot"].is_string());

    // Lifecycle.
    assert_eq!(row["status"], "active");
    assert!(row["status_note"].as_str().unwrap().contains("declaration"));
    assert!(row["first_seen_at"].is_string());
    assert!(row["last_seen_at"].is_string());
    assert_eq!(row["withdrawn_at"], Value::Null);

    // Ambiguity and the unsupported column are keys rather than omissions.
    assert!(row.get("ambiguity").is_some());
    assert!(row.get("unsupported_reason").is_some());

    // Freshness. Nothing has moved, so the verdict is *no qualification* — and `is_current` says
    // so rather than leaving a null to be read as "unknown".
    assert_eq!(row["freshness"], Value::Null);
    assert_eq!(row["is_current"], true);

    // The entry it came through, in full. A link without it is a claim about a repository the
    // reader cannot identify.
    let entry = &row["registry_entry"];
    assert_eq!(entry["registry_id"], "pkg-map");
    assert_eq!(entry["display_name"], common::HOSTILE_DISPLAY_NAME);
    assert_eq!(entry["availability"], "available");
    assert!(entry["availability_statement"]
        .as_str()
        .unwrap()
        .contains("re-checked"));
    assert_eq!(entry["freshness"], Value::Null);
    assert_eq!(entry["usable"], true);

    // The same neighbour, reached by the other rule: a repository-to-repository dependency, which
    // names no file at either end and says so by carrying no target snapshot at all.
    let dependency = of(&rows, "npm_local_dependency", "pkg-map");
    assert_eq!(dependency["relation_semantics"], "DEPENDS_ON");
    assert_eq!(dependency["resolution_method"], "manifest_declared");
    assert_eq!(dependency["source_path"], "package.json");
    assert_eq!(dependency["target_entity_id"], Value::Null);
    assert_eq!(dependency["target_path_snapshot"], Value::Null);
    assert_eq!(dependency["is_current"], true);

    drop(session);
    drop(world);
}

/// A file the neighbour has and never indexed is **unknown**, never *changed* and never *missing*.
///
/// Slice 7c-i's `Stale` / `Unverified` distinction, in the place row 13 puts it: the export entry
/// names `src/data.json`, which is really in the neighbour and which the neighbour's index has
/// never looked at because `.json` is not an indexed extension. The link is recorded **with** the
/// path and **without** a target entity id, and that pair is the whole evidence for the verdict.
#[test]
fn a_target_the_neighbour_never_indexed_is_reported_as_unknown_rather_than_stale() {
    let (world, session) = common::served_contracts("pkg-map");
    let rows = links(&session);
    let row = of(&rows, "npm_export_resolution", "pkg-map/data");

    assert_eq!(row["target_path_snapshot"], "src/data.json");
    assert_eq!(row["target_entity_id"], Value::Null);
    assert_eq!(row["freshness"], "target_partially_indexed");
    assert_eq!(row["is_current"], false);
    assert!(row["freshness_note"]
        .as_str()
        .unwrap()
        .contains("never indexed"));
    // And it is not the neighbour that is unavailable: the entry it came through is fine.
    assert_eq!(row["registry_entry"]["availability"], "available");

    drop(session);
    drop(world);
}

/// One identity declared twice is evidence, and neither declaration is promoted over the other.
#[test]
fn an_ambiguous_declaration_marks_every_row_and_promotes_none() {
    let (world, session) = common::served_contracts("pkg-map");
    let rows = links(&session);

    let twins: Vec<&Value> = rows
        .iter()
        .filter(|row| {
            row["contract_kind"] == "npm_export_resolution"
                && row["contract_identity"] == "pkg-twin"
        })
        .collect();
    assert_eq!(twins.len(), 2, "{twins:?}");
    let mut targets: Vec<&str> = twins
        .iter()
        .map(|row| row["registry_entry"]["registry_id"].as_str().unwrap())
        .collect();
    targets.sort_unstable();
    assert_eq!(targets, vec!["twin-a", "twin-b"]);
    for row in &twins {
        assert_eq!(row["ambiguity"], "conflicting_targets");
    }

    // The weaker form: one target, declared twice, with the declarations agreeing.
    let agreed = of(&rows, "npm_export_resolution", "pkg-string");
    assert_eq!(agreed["ambiguity"], "declared_more_than_once");

    drop(session);
    drop(world);
}

/// A link that is current and one whose target moved are different answers, and both are readable.
#[test]
fn a_link_whose_target_moved_is_distinguishable_from_a_current_one() {
    let (world, session) = common::served_contracts("pkg-map");
    let before = links(&session);
    let current = before
        .iter()
        .filter(|row| row["is_current"] == true)
        .count();
    assert!(current > 0, "no link was current to begin with");

    // The neighbour moves on: a new file, re-indexed, so its state id changes.
    common::write(
        &world.map,
        "src/added.ts",
        "export function added(): number {\n  return 3;\n}\n",
    );
    nerve_index::index_repository(&world.map).unwrap();

    // **Only the links through the neighbour that moved are qualified.** The others did not move,
    // so they are still current — the property that would be lost if freshness were a
    // repository-level verdict rather than a per-link one.
    let after = links(&session);
    let mut moved = 0;
    let mut untouched = 0;
    for row in &after {
        let through_the_mover = row["registry_entry"]["registry_id"] == "pkg-map";
        // The one link through `pkg-map` that was already qualified stays qualified by the
        // stronger answer: part of the target was never looked at, which outranks "it moved on".
        if through_the_mover && row["contract_identity"] != "pkg-map/data" {
            assert_eq!(row["freshness"], "target_changed", "{row}");
            assert_eq!(row["is_current"], false);
            assert!(row["freshness_note"]
                .as_str()
                .unwrap()
                .contains("far end has moved on"));
            assert_ne!(
                row["target_state_at_resolution"],
                row["target_current_state"]
            );
            moved += 1;
        } else if !through_the_mover {
            assert_eq!(row["freshness"], Value::Null, "{row}");
            assert_eq!(row["is_current"], true);
            untouched += 1;
        }
    }
    assert!(moved >= 4, "{moved}");
    assert!(untouched >= 5, "{untouched}");

    drop(session);
    drop(world);
}

/// A retired entry is still listed, and the link through it says the entry is why it ended.
#[test]
fn a_retired_entry_is_listed_and_its_links_report_the_entry_rather_than_a_deletion() {
    let (world, session) = common::served_contracts("pkg-map");
    assert!(!links(&session).is_empty());

    let conn = nerve_store::open(&nerve_index::config::db_path(&world.host)).unwrap();
    let repo_id = nerve_store::repository(&conn).unwrap().unwrap().repo_id;
    nerve_index::remove_registry_target(&conn, &repo_id, "pkg-map").unwrap();
    drop(conn);

    let registry = session.json("/api/contracts/registry");
    let entries = registry["entries"].as_array().unwrap();
    assert_eq!(
        entries.len(),
        common::CONTRACT_WORLD_NEIGHBOURS.len(),
        "a tombstone must still be listed"
    );
    let retired = entries
        .iter()
        .find(|entry| entry["registry_id"] == "pkg-map")
        .expect("the retired entry must still be listed");
    assert_eq!(retired["status"], "tombstoned");
    assert_eq!(retired["availability"], "entry_removed");
    assert_eq!(retired["freshness"], "registry_entry_removed");
    assert!(retired["withdrawn_at"].is_string());
    assert!(retired["links_through_this_entry"].as_u64().unwrap() > 0);
    // The other entries are untouched: retiring one neighbour says nothing about another.
    let kept = entries
        .iter()
        .find(|entry| entry["registry_id"] == "pkg-legacy")
        .expect("the other entries must be unaffected");
    assert_eq!(kept["status"], "active");
    assert_eq!(kept["availability"], "available");

    let mut withdrawn = 0;
    for row in links(&session) {
        if row["registry_entry"]["registry_id"] != "pkg-map" {
            assert_eq!(row["status"], "active", "{row}");
            continue;
        }
        assert_eq!(row["status"], "withdrawn");
        // `registry_entry_removed` outranks `contract_deleted`: the entry being retired is *why*
        // the link ended, and the more specific answer is the one with a remedy.
        assert_eq!(row["freshness"], "registry_entry_removed");
        assert_eq!(row["is_current"], false);
        withdrawn += 1;
    }
    assert!(withdrawn >= 5, "{withdrawn}");

    drop(session);
    drop(world);
}

/// An empty registry is an absence with a reason, never an empty list with no explanation.
#[test]
fn a_repository_with_no_neighbour_says_so_rather_than_answering_nothing() {
    let (_dir, _root, session) = common::served_without_contracts();
    let registry = session.json("/api/contracts/registry");
    assert_eq!(registry["result_kind"], "no_registered_neighbours");
    assert_eq!(registry["entries"], serde_json::json!([]));
    assert_eq!(registry["nothing_is_auto_registered"], true);
    assert_eq!(registry["registry_entries_total"], 0);

    let value = session.json("/api/contracts");
    assert_eq!(value["result_kind"], "no_contract_links");
    assert_eq!(value["links"], serde_json::json!([]));
    assert_eq!(value["links_total"], 0);
    // And the boundary is on the answer that has nothing else on it, because that is exactly the
    // answer a reader needs the next command from.
    assert_eq!(value["boundary"]["read_only"], true);
    assert!(value["boundary"]["commands"]
        .as_array()
        .unwrap()
        .contains(&serde_json::json!("nerve repo scan")));
}

/// Nothing is auto-registered: an indexed sibling that nobody named produces no entry and no link.
#[test]
fn an_adjacent_indexed_repository_that_nobody_named_is_not_registered() {
    let (world, session) = common::served_contracts("pkg-map");
    let registry = session.json("/api/contracts/registry");
    let mut ids: Vec<&str> = registry["entries"]
        .as_array()
        .unwrap()
        .iter()
        .map(|entry| entry["registry_id"].as_str().unwrap())
        .collect();
    ids.sort_unstable();
    let mut expected = common::CONTRACT_WORLD_NEIGHBOURS.to_vec();
    expected.sort_unstable();
    assert_eq!(ids, expected);
    assert!(
        !ids.contains(&"pkg-unregistered"),
        "an adjacent checkout registered itself: {ids:?}"
    );
    // Anti-vacuity: the directory really is there, really is a Nerve repository, and is really
    // named by this repository's own manifest — so "no entry" is a decision rather than an absence.
    assert!(world.dir.path().join("pkg-unregistered/.nerve").is_dir());
    assert!(links(&session)
        .iter()
        .all(|row| row["registry_entry"]["registry_id"] != "pkg-unregistered"));

    drop(session);
    drop(world);
}

/// Every closed vocabulary is answerable, and the declined forms are named.
#[test]
fn the_vocabulary_names_every_form_including_the_ones_nerve_declines() {
    let (_dir, _root, session) = common::served_without_contracts();
    let value = session.json("/api/contracts/vocabulary");
    assert_eq!(value["result_kind"], "vocabulary");
    let vocabulary = &value["vocabulary"];

    let names = |key: &str| -> Vec<String> {
        vocabulary[key]
            .as_array()
            .unwrap_or_else(|| panic!("{key} is absent"))
            .iter()
            .map(|term| term["name"].as_str().unwrap().to_string())
            .collect()
    };

    assert_eq!(names("freshness").len(), 12);
    assert!(!names("freshness").contains(&"generated_client_stale".to_string()));
    assert_eq!(names("rules").len(), 3);
    assert_eq!(names("resolution_methods").len(), 4);
    assert_eq!(names("unsupported_forms").len(), 23);
    assert_eq!(names("unresolved_reasons").len(), 10);
    assert_eq!(names("supported_forms").len(), 15);
    assert_eq!(names("availability").len(), 6);
    assert_eq!(names("registry_refusals").len(), 12);

    // Named rather than counted only: the forms a reader would otherwise never see are here.
    for form in [
        "npm_registry_range",
        "npm_git_specifier",
        "python_workspace_source",
        "npm_export_wildcard_subpath",
    ] {
        assert!(
            names("unsupported_forms").contains(&form.to_string()),
            "{form}"
        );
    }
    // Each declined form says which rule read it, so a reader can tell an npm refusal from a
    // Python one without matching on a prefix.
    for term in vocabulary["unsupported_forms"].as_array().unwrap() {
        assert!(term["rule"].is_string(), "{term}");
    }
    // Every freshness value carries its own sentence.
    for term in vocabulary["freshness"].as_array().unwrap() {
        assert!(term["note"].as_str().unwrap().len() > 40, "{term}");
    }

    // A build constant returned whole: nothing was cut, and there is no page after it.
    assert_eq!(value["continuation"]["supported"], false);
    assert!(value["continuation"]["statement"]
        .as_str()
        .unwrap()
        .contains("build constant"));
    assert_eq!(value["truncation"], Value::Null);
}

/// Bounds: the window is honoured, truncation is a comparison, and the offset is the next page.
#[test]
fn the_link_list_is_bounded_and_its_truncation_is_a_fact() {
    let (world, session) = common::served_contracts("pkg-map");
    let all = links(&session);
    assert_eq!(all.len(), common::CONTRACT_WORLD_LINKS, "{all:?}");

    let first = session.json("/api/contracts?limit=1");
    assert_eq!(first["links"].as_array().unwrap().len(), 1);
    assert_eq!(first["truncation"]["returned"], 1);
    assert_eq!(first["truncation"]["total"], all.len());
    assert_eq!(first["truncation"]["truncated"], true);
    assert_eq!(first["continuation"]["supported"], true);
    assert_eq!(first["continuation"]["next_offset"], 1);

    // The next page is the next rows rather than a re-run of the bound.
    let second = session.json("/api/contracts?limit=1&offset=1");
    assert_eq!(second["links"][0]["link_id"], all[1]["link_id"]);

    // A page that ends exactly on the boundary is not truncated — the case `len() == limit` gets
    // wrong, and the reason truncation is a comparison rather than a guess.
    let exact = session.json(&format!("/api/contracts?limit={}", all.len()));
    assert_eq!(exact["truncation"]["truncated"], false);
    assert_eq!(exact["continuation"]["next_offset"], Value::Null);

    // The filter is an exact registry id, and an id nothing matches is an empty page rather than
    // an error — the entry is real, the narrowing is real, and zero is the honest answer.
    let filtered = session.json("/api/contracts?registry_id=pkg-legacy&limit=500");
    let matched = filtered["links_matching_filter"].as_u64().unwrap();
    assert!(matched > 0 && matched < all.len() as u64, "{matched}");
    let none = session.json("/api/contracts?registry_id=no-such-entry");
    assert_eq!(none["links_matching_filter"], 0);
    assert_eq!(none["links"], serde_json::json!([]));
    assert_eq!(none["links_total"], all.len());

    drop(session);
    drop(world);
}

/// Read-only, on the bytes: neither database changes across a full sweep of every contract route.
#[test]
fn every_contract_route_leaves_both_databases_byte_identical() {
    let (world, session) = common::served_contracts("pkg-map");
    let host_db = nerve_index::config::db_path(&world.host);
    let map_db = nerve_index::target_database_path(&world.map);
    let before = (common::digest(&host_db), common::digest(&map_db));

    let mut answered = 0;
    for route in [
        "/api/contracts",
        "/api/contracts?limit=2&offset=1",
        "/api/contracts?registry_id=pkg-map",
        "/api/contracts/registry",
        "/api/contracts/vocabulary",
    ] {
        let value = session.json(route);
        assert_eq!(value["ok"], true, "{route}");
        answered += 1;
    }
    // Anti-vacuity: the reads really produced answers, so "unchanged" is not "nothing ran".
    assert_eq!(answered, 5);
    assert!(!links(&session).is_empty());

    assert_eq!(
        before,
        (common::digest(&host_db), common::digest(&map_db)),
        "a database changed while the contract routes were read"
    );

    // And a write verb is refused before routing, so no contract route can be reached by one.
    let response = session.raw(
        "POST",
        "/api/contracts",
        &[
            ("Host", &session.host()),
            (nerve_server::token::TOKEN_HEADER, session.token()),
        ],
    );
    assert_eq!(response.status, 405);
    assert_eq!(response.parse_json()["error"]["code"], "method_not_allowed");
    assert_eq!(
        before,
        (common::digest(&host_db), common::digest(&map_db)),
        "a refused POST changed a database"
    );

    drop(session);
    drop(world);
}

/// The three contract routes are advertised, gated and dispatched like every other route.
#[test]
fn the_contract_routes_are_advertised_and_gated() {
    let (world, session) = common::served_contracts("pkg-map");
    for route in [
        "/api/contracts",
        "/api/contracts/registry",
        "/api/contracts/vocabulary",
    ] {
        assert!(
            nerve_server::router::ROUTES.contains(&route),
            "{route} is served and not advertised"
        );
        // No token: refused before dispatch, like every other `/api/*` route.
        let response = session.raw("GET", route, &[("Host", &session.host())]);
        assert_eq!(response.status, 401, "{route}");
    }
    drop(session);
    drop(world);
}
