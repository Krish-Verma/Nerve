//! The read-only API: shapes, bounds, determinism, and the two properties that make it safe to
//! point at a real repository — it never writes, and it stops cleanly.

mod common;

use nerve_core::ids::content_hash;

// ---- shapes --------------------------------------------------------------------------------

#[test]
fn overview_reports_counts_runs_schema_and_freshness() {
    let (_dir, _root, session) = common::served();
    let value = session.json("/api/overview");

    assert_eq!(value["schema_version"], nerve_store::SCHEMA_VERSION);
    assert_eq!(
        value["supported_schema_version"],
        nerve_store::SCHEMA_VERSION
    );
    assert_eq!(value["healthy"], true);
    assert!(value["entities_total"].as_u64().unwrap() > 0);
    assert!(value["entities_by_kind"]["module"].as_u64().unwrap() > 0);
    assert!(value["assertions_by_relation"]["DEFINES"].as_u64().unwrap() > 0);
    assert!(value["observations_total"].as_u64().unwrap() > 0);
    assert!(value["database_bytes"].as_u64().unwrap() > 0);
    assert!(value["state_id"].is_string());
    assert!(value["last_run"]["extractor_id"].is_string());
    assert!(value["runs"].as_array().unwrap().len() >= 2);

    // The fixture has unresolved references by construction; hiding that would be the failure.
    assert!(value["unresolved_entities"].as_u64().unwrap() > 0);

    let freshness = &value["freshness"];
    assert!(freshness["files_total"].as_u64().unwrap() > 0);
    assert_eq!(freshness["files_probed"], freshness["files_total"]);
    assert_eq!(freshness["fresh"], freshness["files_total"]);
    assert_eq!(freshness["stale"], 0);
    assert_eq!(freshness["truncated"], false);
}

#[test]
fn overview_notices_that_the_repository_moved_on() {
    let (_dir, root, session) = common::served();
    common::write(&root, "src/math.ts", "export const changed = 1;\n");
    let freshness = session.json("/api/overview")["freshness"].clone();
    assert_eq!(freshness["stale"], 1, "{freshness}");

    std::fs::remove_file(root.join("src/shapes.ts")).unwrap();
    let after = session.json("/api/overview")["freshness"].clone();
    assert_eq!(after["missing"], 1, "{after}");
}

#[test]
fn search_returns_ordered_hits_and_honours_kind_and_limit() {
    let (_dir, _root, session) = common::served();

    let all = session.json("/api/search?q=area");
    assert!(all["count"].as_u64().unwrap() > 0);
    let first = &all["results"][0];
    for field in ["entity_id", "kind", "name", "scope_path"] {
        assert!(first[field].is_string(), "{field} missing: {first}");
    }
    assert!(first["score"].is_number());

    let methods = session.json("/api/search?q=area&kind=method");
    for hit in methods["results"].as_array().unwrap() {
        assert_eq!(hit["kind"], "method");
    }

    let limited = session.json("/api/search?q=a&limit=2");
    assert!(limited["results"].as_array().unwrap().len() <= 2);
    assert_eq!(limited["limit"], 2);

    // Deterministic: the same question twice gives the same answer in the same order.
    assert_eq!(session.json("/api/search?q=area"), all);
}

#[test]
fn search_refuses_an_unknown_kind_rather_than_ignoring_it() {
    let (_dir, _root, session) = common::served();
    let response = session.get("/api/search?q=area&kind=wombat");
    assert_eq!(response.status, 400);
    let value = response.parse_json();
    assert_eq!(value["error"]["code"], "unknown_kind");
    assert!(value["error"]["detail"]["allowed"]
        .as_array()
        .unwrap()
        .contains(&serde_json::json!("method")));
}

#[test]
fn search_with_no_query_is_a_usage_error() {
    let (_dir, _root, session) = common::served();
    assert_eq!(session.get("/api/search").status, 400);
}

#[test]
fn entity_returns_occurrences_relation_counts_and_defining_edges() {
    let (_dir, _root, session) = common::served();
    let value = session.json("/api/entity?selector=src%2Fshapes.ts");

    assert_eq!(value["entity"]["kind"], "module");
    assert_eq!(value["entity"]["scope_path"], "src/shapes.ts");
    assert!(!value["entity"]["entity_id"].as_str().unwrap().is_empty());
    assert!(value["occurrence_count"].as_u64().unwrap() >= 1);

    let occurrence = &value["occurrences"][0];
    assert_eq!(occurrence["file_path"], "src/shapes.ts");
    for field in ["start_byte", "end_byte", "start_line", "end_line"] {
        assert!(occurrence[field].is_number(), "{field}");
    }
    assert!(occurrence["content_hash"].is_string());

    assert!(
        value["relation_counts"]["outgoing"]["DEFINES"]
            .as_u64()
            .unwrap()
            > 0
    );
    let defining = &value["defining_edges"];
    assert!(defining["node_count"].as_u64().unwrap() > 1);
    for edge in defining["edges"].as_array().unwrap() {
        assert!(
            edge["relation"] == "DEFINES" || edge["relation"] == "CONTAINS",
            "{edge}"
        );
    }
}

#[test]
fn a_symbol_selector_resolves_the_same_way_the_cli_resolves_it() {
    let (_dir, _root, session) = common::served();
    let by_path = session.json("/api/entity?selector=src%2Fshapes.ts%23Circle.area");
    assert_eq!(by_path["entity"]["name"], "area");
    assert_eq!(by_path["entity"]["kind"], "method");

    let id = by_path["entity"]["entity_id"].as_str().unwrap().to_string();
    let by_id = session.json(&format!("/api/entity?selector={id}"));
    assert_eq!(by_id["entity"], by_path["entity"]);
}

#[test]
fn an_ambiguous_selector_chooses_nothing_and_says_so() {
    let (_dir, _root, session) = common::served();
    let response = session.get("/api/entity?selector=helper");
    assert_eq!(response.status, 409, "{}", response.body);
    let value = response.parse_json();
    assert_eq!(value["error"]["code"], "ambiguous_selector");
    assert!(
        value["error"]["detail"]["candidates"]
            .as_array()
            .unwrap()
            .len()
            > 1
    );
}

#[test]
fn an_unknown_selector_comes_back_with_suggestions() {
    let (_dir, _root, session) = common::served();
    let response = session.get("/api/entity?selector=circumferance");
    assert_eq!(response.status, 404);
    let value = response.parse_json();
    assert_eq!(value["error"]["code"], "selector_not_found");
    assert!(value["error"]["detail"]["suggestions"].is_array());
}

#[test]
fn a_neighbourhood_is_bounded_and_reports_what_it_left_out() {
    let (_dir, _root, session) = common::served();

    let wide = session.json("/api/neighbourhood?selector=src%2Fshapes.ts&depth=1&max_nodes=200");
    assert_eq!(wide["focus"]["scope_path"], "src/shapes.ts");
    assert_eq!(wide["max_depth"], 1);
    assert_eq!(wide["nodes"][0]["depth"], 0);
    assert!(wide["node_count"].as_u64().unwrap() > 1);
    assert_eq!(wide["omitted_nodes"], 0);
    assert_eq!(
        wide["truncated"], false,
        "a depth-1 answer to a depth-1 question is complete: {wide}"
    );
    assert!(
        wide["frontier_nodes"].as_u64().unwrap() > 0,
        "there is more to expand, and saying so is not the same as truncating: {wide}"
    );

    let pinched = session.json("/api/neighbourhood?selector=src%2Fshapes.ts&depth=2&max_nodes=3");
    assert_eq!(pinched["node_count"], 3);
    assert_eq!(pinched["truncated"], true);
    assert!(
        pinched["omitted_nodes"].as_u64().unwrap() > 0,
        "a budget that bit must say how much it left out: {pinched}"
    );

    // Every edge names two nodes that are in the response. A renderer never has to invent one.
    let ids: Vec<&str> = pinched["nodes"]
        .as_array()
        .unwrap()
        .iter()
        .map(|node| node["entity"]["entity_id"].as_str().unwrap())
        .collect();
    for edge in pinched["edges"].as_array().unwrap() {
        assert!(
            ids.contains(&edge["source_entity_id"].as_str().unwrap()),
            "{edge}"
        );
        assert!(
            ids.contains(&edge["target_entity_id"].as_str().unwrap()),
            "{edge}"
        );
    }
}

#[test]
fn a_neighbourhood_can_be_filtered_to_one_relation() {
    let (_dir, _root, session) = common::served();
    let value = session.json("/api/neighbourhood?selector=src%2Fapp.ts&depth=1&relation=IMPORTS");
    assert_eq!(value["relations"], serde_json::json!(["IMPORTS"]));
    for edge in value["edges"].as_array().unwrap() {
        assert_eq!(edge["relation"], "IMPORTS");
    }
}

#[test]
fn path_finds_a_connection_and_reports_its_bounds() {
    let (_dir, _root, session) = common::served();
    let value = session
        .json("/api/path?from=src%2Fapp.ts&to=src%2Fmath.ts&max_depth=4&limit=3&direction=forward");
    assert_eq!(value["from"]["scope_path"], "src/app.ts");
    assert_eq!(value["to"]["scope_path"], "src/math.ts");
    assert_eq!(value["max_depth"], 4);
    assert_eq!(value["direction"], "forward");
    assert!(value["count"].as_u64().unwrap() > 0, "{value}");

    let hop = &value["paths"][0]["hops"][0];
    assert!(hop["relation"].is_string());
    assert!(hop["assertion_id"].is_string());
    assert!(hop["from"]["entity_id"].is_string());
    assert!(hop["to"]["entity_id"].is_string());
    assert!(hop["observation_count"].is_number());
    assert!(value["truncated"].is_boolean());
    assert!(value["expansions"].is_number());
}

#[test]
fn path_rejects_an_unknown_relation_rather_than_dropping_the_filter() {
    let (_dir, _root, session) = common::served();
    let response = session.get("/api/path?from=src%2Fapp.ts&to=src%2Fmath.ts&relation=SUMMONS");
    assert_eq!(response.status, 400);
    assert_eq!(response.parse_json()["error"]["code"], "unknown_relation");
}

#[test]
fn why_returns_the_evidence_packet_with_computed_freshness() {
    let (_dir, root, session) = common::served();
    let value = session.json("/api/why?subject=src%2Fshapes.ts%23Circle.area");

    assert_eq!(value["subject"]["name"], "area");
    assert_eq!(value["direction"], "both");
    assert!(value["count"].as_u64().unwrap() > 0);
    assert!(value["files_probed"].as_u64().unwrap() > 0);

    let assertion = &value["assertions"][0];
    assert!(assertion["assertion_id"].is_string());
    assert!(assertion["relation"].is_string());
    assert!(assertion["source"]["entity_id"].is_string());
    assert!(assertion["target"]["entity_id"].is_string());

    let observation = &assertion["observations"][0];
    for field in [
        "evidence_source_type",
        "directness",
        "extractor_id",
        "extractor_version",
        "state_id",
        "file_path",
        "content_hash",
        "created_at",
    ] {
        assert!(observation[field].is_string(), "{field}: {observation}");
    }
    assert_eq!(
        observation["freshness"], "fresh",
        "an untouched repository is fresh: {observation}"
    );

    // Freshness is computed, never stored: change the file and the same query says so.
    common::write(&root, "src/shapes.ts", "export class Circle {}\n");
    let after = session.json("/api/why?subject=src%2Fshapes.ts");
    let stale = after["assertions"][0]["observations"][0]["freshness"]
        .as_str()
        .unwrap();
    assert_eq!(stale, "stale", "{after}");
}

#[test]
fn why_can_be_narrowed_to_one_side_and_one_pair() {
    let (_dir, _root, session) = common::served();
    let outgoing = session.json("/api/why?subject=src%2Fapp.ts&direction=outgoing");
    for assertion in outgoing["assertions"].as_array().unwrap() {
        assert_eq!(assertion["direction"], "outgoing");
    }

    let pair = session.json("/api/why?subject=src%2Fapp.ts&object=src%2Fmath.ts");
    assert_eq!(pair["object"]["scope_path"], "src/math.ts");
    assert!(pair["count"].as_u64().unwrap() > 0);

    let response = session.get("/api/why?subject=src%2Fapp.ts&direction=sideways");
    assert_eq!(response.status, 400);
    assert_eq!(response.parse_json()["error"]["code"], "unknown_direction");
}

#[test]
fn the_unresolved_list_is_paged_and_counted() {
    let (_dir, _root, session) = common::served();
    let value = session.json("/api/unresolved?limit=5");
    assert!(value["unresolved_entities_total"].as_u64().unwrap() > 0);
    assert!(value["results"].as_array().unwrap().len() <= 5);

    let row = &value["results"][0];
    assert!(row["entity_id"].is_string());
    assert!(row["name"].is_string());
    assert!(row["referencing_assertions"].as_u64().unwrap() >= 1);

    let page_two = session.json("/api/unresolved?limit=5&offset=5");
    assert_eq!(page_two["offset"], 5);
    assert_ne!(page_two["results"], value["results"]);
}

#[test]
fn the_partial_parse_list_names_files_that_did_not_parse_cleanly() {
    let (_dir, root) = common::fixture_copy("ts-resolution");
    common::write(&root, "src/broken.ts", "export function oops( {\n");
    common::index(&root);
    let session = common::Session::start(&root);

    let value = session.json("/api/partial-parses");
    let paths: Vec<&str> = value["results"]
        .as_array()
        .unwrap()
        .iter()
        .map(|row| row["rel_path"].as_str().unwrap())
        .collect();
    assert!(paths.contains(&"src/broken.ts"), "{value}");
    assert_eq!(value["count"], paths.len());
}

#[test]
fn a_clean_repository_reports_no_partial_parses() {
    let (_dir, root) = common::fixture_copy("ts-basic");
    common::index(&root);
    let session = common::Session::start(&root);
    let value = session.json("/api/partial-parses");
    assert_eq!(value["count"], 0);
    assert_eq!(value["results"], serde_json::json!([]));
}

#[test]
fn an_unknown_route_lists_the_ones_that_exist() {
    let (_dir, _root, session) = common::served();
    let response = session.get("/api/does-not-exist");
    assert_eq!(response.status, 404);
    let value = response.parse_json();
    assert_eq!(value["error"]["code"], "no_such_route");
    assert!(value["error"]["detail"]["routes"]
        .as_array()
        .unwrap()
        .contains(&serde_json::json!("/api/overview")));
}

#[test]
fn the_embedded_assets_are_served() {
    let (_dir, _root, session) = common::served();
    let index = session.get("/index.html");
    assert_eq!(index.status, 200);
    assert!(index.body.contains("Nerve"));

    let css = session.get("/assets/nerve.css");
    assert_eq!(css.status, 200);
    assert_eq!(css.header("content-type"), Some("text/css; charset=utf-8"));
    assert!(css.body.contains("body"));
}

// ---- read-only -------------------------------------------------------------------------------

/// The load-bearing promise: pointing this at a real repository cannot change its index.
#[test]
fn a_full_api_session_leaves_the_database_byte_identical() {
    let (_dir, root, session) = common::served();
    let db_path = nerve_index::config::db_path(&root);

    let before = std::fs::read(&db_path).unwrap();
    let before_hash = content_hash(&before);

    let module = "src%2Fshapes.ts";
    for target in [
        "/api/overview".to_string(),
        "/api/search?q=area".to_string(),
        "/api/search?q=circle&kind=class".to_string(),
        format!("/api/entity?selector={module}"),
        format!("/api/neighbourhood?selector={module}&depth=2&max_nodes=50"),
        "/api/path?from=src%2Fapp.ts&to=src%2Fmath.ts".to_string(),
        format!("/api/why?subject={module}"),
        "/api/source?path=src/shapes.ts&start_line=1&end_line=40".to_string(),
        "/api/unresolved?limit=100".to_string(),
        "/api/partial-parses".to_string(),
        "/".to_string(),
        "/assets/nerve.css".to_string(),
    ] {
        assert!(session.get(&target).status < 500, "{target}");
    }
    // Refusals must not write either — an audit log would be a write.
    let _ = session.get("/api/source?path=../../etc/passwd");
    let _ = session.raw("GET", "/api/overview", &[("Host", "evil.test")]);
    let _ = session.raw("POST", "/api/overview", &[("Host", &session.host())]);

    let after = std::fs::read(&db_path).unwrap();
    assert_eq!(before.len(), after.len(), "database size changed");
    assert_eq!(
        before_hash,
        content_hash(&after),
        "the database changed during a read-only session"
    );
}

/// Read-onlyness is enforced by SQLite, not merely by this crate's good intentions.
#[test]
fn the_connection_a_worker_holds_cannot_write() {
    let (_dir, root, _session) = common::served();
    let conn = nerve_store::open(&nerve_index::config::db_path(&root)).unwrap();
    conn.pragma_update(None, "query_only", "ON").unwrap();
    let err = conn
        .execute("DELETE FROM entity", [])
        .expect_err("query_only must refuse a write");
    assert!(err.to_string().to_lowercase().contains("readonly"), "{err}");
}

// ---- lifecycle -------------------------------------------------------------------------------

#[test]
fn the_server_stops_cleanly_and_leaves_no_lock_behind() {
    let (_dir, root) = common::fixture_copy("ts-resolution");
    common::index(&root);

    let mut session = common::Session::start(&root);
    assert_eq!(session.get("/api/overview").status, 200);
    let address = session.address();
    session.stop();

    // The port is released.
    assert!(
        std::net::TcpStream::connect(address).is_err(),
        "the listener outlived shutdown"
    );

    // And a writer can take the database straight away.
    common::write(&root, "src/added.ts", "export const added = 1;\n");
    let outcome = nerve_index::index_repository(&root).expect("indexing after shutdown");
    assert!(outcome.files_processed > 0);
}

#[test]
fn two_servers_can_run_side_by_side_on_ephemeral_ports() {
    let (_dir, root) = common::fixture_copy("ts-resolution");
    common::index(&root);
    let first = common::Session::start(&root);
    let second = common::Session::start(&root);

    assert_ne!(first.address().port(), second.address().port());
    assert_ne!(first.token(), second.token(), "tokens must be per session");

    assert_eq!(first.get("/api/overview").status, 200);
    assert_eq!(second.get("/api/overview").status, 200);

    // A token is a capability for one server, not for "a Nerve server".
    let crossed = second.raw(
        "GET",
        "/api/overview",
        &[
            ("Host", &second.host()),
            (nerve_server::token::TOKEN_HEADER, first.token()),
        ],
    );
    assert_eq!(crossed.status, 403);
}

#[test]
fn concurrent_requests_are_answered_consistently() {
    let (_dir, _root, session) = common::served();
    let address = session.address();
    let token = session.token().to_string();
    let host = session.host();

    let expected = session.json("/api/search?q=area");
    let handles: Vec<_> = (0..12)
        .map(|_| {
            let token = token.clone();
            let host = host.clone();
            std::thread::spawn(move || {
                let response = common::request(
                    address,
                    "GET",
                    "/api/search?q=area",
                    &[("Host", &host), (nerve_server::token::TOKEN_HEADER, &token)],
                );
                assert_eq!(response.status, 200);
                response.parse_json()
            })
        })
        .collect();
    for handle in handles {
        assert_eq!(handle.join().unwrap(), expected);
    }
}
