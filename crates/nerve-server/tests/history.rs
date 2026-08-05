//! `/api/history*` — the derived historical questions over HTTP.
//!
//! Every assertion here is about **content**. A status code establishes that a route dispatched and
//! nothing else, and four of this project's five recorded vacuity traps passed a green suite; so
//! wherever a test asserts an absence — that no answer says "created", that no field is named
//! `related` — a nonzero tally is asserted beside it, so the absence is about wording rather than
//! about an empty answer.
//!
//! **The numbers come from `inventory.json`, which is Git's own answer.** Each `fixtures/history-*`
//! carries one, generated from that fixture's own object store by `git` itself. A disagreement
//! between an endpoint and an inventory is a Nerve defect, never a stale expectation, which is why
//! nothing below is a hardcoded count.

mod common;

use nerve_core::ids::content_hash;

/// The phrases only `FirstObservedKind::CreatedInVisibleHistory` may produce.
///
/// Checked as phrases rather than as the substring `creat`, because a refusal sentence has to stay
/// free to name the claim it is refusing — `may_claim_created_note` says "not permitted" and would
/// match a naive scan.
const CREATION_PHRASES: [&str; 4] = [
    "the path was created at this change",
    "was created here",
    "first ever",
    "the file was created",
];

/// Every history route, with arguments where one is required.
///
/// Taken from `router::ROUTES` rather than retyped, so a route added later is gated by the
/// authentication test below without this file being edited.
fn history_targets() -> Vec<String> {
    nerve_server::router::ROUTES
        .iter()
        .filter(|route| route.starts_with("/api/history"))
        .map(|route| match *route {
            "/api/history/commit" => format!("{route}?commit=abc"),
            "/api/history/path" | "/api/history/cochange" => format!("{route}?path=README.md"),
            "/api/history/diff" => format!("{route}?from=abc&to=def"),
            other => other.to_string(),
        })
        .collect()
}

// ---- availability ----------------------------------------------------------------------------

#[test]
fn availability_reports_the_boundary_git_itself_declares() {
    let (_dir, _root, session) = common::served_history("history-shallow");
    let inventory = common::history_inventory("history-shallow");
    let value = session.json("/api/history");

    assert_eq!(value["result_kind"], "availability");
    assert_eq!(value["history_ingested"], true);
    assert_eq!(value["shallow"], true);
    // Git's own boundary oids, not Nerve's idea of them.
    assert_eq!(
        value["shallow_boundary"], inventory["shallow"]["boundary_oids"],
        "Nerve and Git disagree about where the boundary is"
    );
    assert_eq!(
        value["commits_recorded"],
        serde_json::json!(inventory["commits"].as_array().unwrap().len()),
        "Nerve and Git disagree about how many commits are visible"
    );
    assert_eq!(value["walk_terminated_by"], "shallow_boundary");
    assert!(value["walk_terminated_note"]
        .as_str()
        .unwrap()
        .contains("unavailable to this repository"));
    assert_eq!(value["limitations"]["earlier_changes_may_exist"], true);
    assert_eq!(value["ingest_head_oid"], inventory["head_oid"]);
    assert_eq!(
        value["current_repository_state"]["git_commit"],
        inventory["head_oid"]
    );
    assert_eq!(value["freshness"], "current");
    assert!(value["freshness_note"]
        .as_str()
        .unwrap()
        .contains("indexed"));

    // The tallies are Git's too: every change row every visible commit declares.
    let declared: usize = inventory["commits"]
        .as_array()
        .unwrap()
        .iter()
        .map(|commit| commit["changes"].as_array().map_or(0, Vec::len))
        .sum();
    assert!(declared > 0, "the fixture must declare changes");
    assert_eq!(value["totals"]["changes"], serde_json::json!(declared));

    // The boundary is never described as the start of the project, whatever else is said.
    assert!(
        !session.get("/api/history").body.contains("begins here"),
        "a shallow boundary was described as the start of history"
    );
}

/// **An un-ingested history is an absence, not a failure, and not an empty history.**
#[test]
fn an_un_ingested_repository_answers_with_an_absence_rather_than_a_zero() {
    let (_dir, _root, session) = common::served_without_history("history-basic");
    let value = session.json("/api/history");

    assert_eq!(value["result_kind"], "no_history_ingested");
    assert_eq!(value["history_ingested"], false);
    assert_eq!(value["freshness"], "no_history_ingested");
    assert!(value["freshness_note"]
        .as_str()
        .unwrap()
        .contains("an absence, and not a failure"));
    // `null`, never `0`. A zero tally would read as "this project has no history".
    for absent in [
        "totals",
        "shallow",
        "commits_recorded",
        "walk_terminated_by",
        "reader_version",
    ] {
        assert!(value[absent].is_null(), "{absent}: {}", value[absent]);
    }
    assert!(value["limitations"]["earlier_changes_may_exist"].is_null());

    // The anti-vacuity half: the same fixture, ingested, answers the opposite — so the nulls above
    // are about this repository's state rather than about fields that are always null.
    let (_ingested_dir, _ingested_root, ingested) = common::served_history("history-basic");
    let after = ingested.json("/api/history");
    assert_eq!(after["history_ingested"], true);
    assert!(after["totals"]["commits"].as_u64().unwrap() > 0);
    assert_eq!(after["result_kind"], "availability");
}

// ---- the commit log --------------------------------------------------------------------------

#[test]
fn the_commit_log_carries_gits_commits_and_bounds_its_page() {
    let (_dir, _root, session) = common::served_history("history-basic");
    let inventory = common::history_inventory("history-basic");
    let declared = inventory["commits"].as_array().unwrap().len();
    assert!(declared > 2, "the fixture must declare a history to page");

    let all = session.json("/api/history/commits");
    assert_eq!(all["result_kind"], "commit_log");
    assert_eq!(
        all["commits"].as_array().unwrap().len(),
        declared,
        "the API and Git disagree about how many commits there are"
    );
    assert_eq!(all["truncation"]["total"], serde_json::json!(declared));
    // The page holds everything, so it is not truncated even though a naive reader might expect it.
    assert_eq!(all["truncation"]["truncated"], false);
    assert_eq!(all["continuation"]["supported"], true);
    assert!(all["continuation"]["next_offset"].is_null());

    // Every commit carries the state that qualifies its change count. A count alone would read as
    // "nothing changed" in three of the four cases.
    for commit in all["commits"].as_array().unwrap() {
        assert!(commit["changes"].is_number(), "{commit}");
        assert!(commit["changes_enumerated"].is_string(), "{commit}");
        assert!(commit["changes_enumerated_note"].is_string(), "{commit}");
        assert!(
            commit["may_claim_history_begins_here"].is_boolean(),
            "{commit}"
        );
    }
    // Exactly one commit may claim it, and Git says which: the parentless one.
    let claiming: Vec<&serde_json::Value> = all["commits"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|commit| commit["may_claim_history_begins_here"] == true)
        .collect();
    let roots: Vec<&serde_json::Value> = inventory["commits"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|commit| commit["parent_oids"].as_array().unwrap().is_empty())
        .collect();
    assert_eq!(claiming.len(), roots.len());
    assert_eq!(claiming[0]["commit_oid"], roots[0]["oid"]);

    // Bounded, with truncation as a fact and a continuation the query honours.
    let page = session.json("/api/history/commits?limit=1");
    assert_eq!(page["commits"].as_array().unwrap().len(), 1);
    assert_eq!(page["truncation"]["truncated"], true);
    assert_eq!(page["truncation"]["limit"], 1);
    assert_eq!(page["continuation"]["next_offset"], 1);

    let second = session.json("/api/history/commits?limit=1&offset=1");
    assert_eq!(second["continuation"]["offset"], 1);
    assert_ne!(
        second["commits"][0]["commit_oid"], page["commits"][0]["commit_oid"],
        "the offset must move the page rather than repeat it"
    );
    assert_eq!(second["commits"][0], all["commits"][1]);

    // The boundary case `len() == limit` gets wrong: a full page that is also the whole answer.
    let exact = session.json(&format!("/api/history/commits?limit={declared}"));
    assert_eq!(exact["commits"].as_array().unwrap().len(), declared);
    assert_eq!(
        exact["truncation"]["truncated"], false,
        "a page that happens to fill the limit is not truncated"
    );
}

// ---- one commit ------------------------------------------------------------------------------

#[test]
fn one_commits_changes_are_gits_and_a_boundary_enumerates_none_with_its_reason() {
    let (_dir, _root, session) = common::served_history("history-shallow");
    let inventory = common::history_inventory("history-shallow");

    let head = inventory["head_oid"].as_str().unwrap();
    let declared = inventory["commits"]
        .as_array()
        .unwrap()
        .iter()
        .find(|commit| commit["oid"] == head)
        .expect("the inventory names HEAD");
    let declared_paths: Vec<&str> = declared["changes"]
        .as_array()
        .unwrap()
        .iter()
        .map(|change| change["path"].as_str().unwrap())
        .collect();
    assert!(!declared_paths.is_empty(), "HEAD must change something");

    let value = session.json(&format!("/api/history/commit?commit={head}"));
    assert_eq!(value["result_kind"], "commit_changes");
    assert_eq!(value["requested_subject"]["commit"], head);
    let served: Vec<&str> = value["changes"]
        .as_array()
        .unwrap()
        .iter()
        .map(|change| change["path"].as_str().unwrap())
        .collect();
    assert_eq!(
        served, declared_paths,
        "the API and Git disagree about HEAD"
    );
    assert_eq!(
        value["truncation"]["total"],
        serde_json::json!(declared_paths.len())
    );
    assert_eq!(value["truncation"]["truncated"], false);

    // The boundary: zero change rows, and which of the four silences it is.
    let boundary = inventory["shallow"]["boundary_oids"][0].as_str().unwrap();
    let value = session.json(&format!("/api/history/commit?commit={boundary}"));
    assert_eq!(value["changes"], serde_json::json!([]));
    assert_eq!(value["commit"]["changes_enumerated"], "parent_unavailable");
    assert!(value["commit"]["changes_enumerated_note"]
        .as_str()
        .unwrap()
        .contains("not an empty commit"));
    assert_eq!(value["commit"]["parent_completeness"], "shallow_boundary");
    assert_eq!(value["commit"]["may_claim_history_begins_here"], false);
    // Git says the boundary names a parent it does not have. Nerve must not have dropped it.
    assert_eq!(
        value["commit"]["parent_oids"],
        inventory["shallow"]["absent_parent_oids_named_by_boundary"]
    );

    // A bound, on a list that has no total of its own until it is counted.
    let bounded = session.json(&format!("/api/history/commit?commit={head}&limit=1"));
    assert_eq!(bounded["changes"].as_array().unwrap().len(), 1);
    assert_eq!(bounded["truncation"]["truncated"], true);
    assert_eq!(
        bounded["truncation"]["total"],
        serde_json::json!(declared_paths.len())
    );
}

#[test]
fn an_unrecorded_commit_is_a_refusal_never_an_empty_change_list() {
    let (_dir, _root, session) = common::served_history("history-shallow");
    let response =
        session.get("/api/history/commit?commit=0000000000000000000000000000000000000000");
    assert_eq!(response.status, 404);
    let value = response.parse_json();
    assert_eq!(value["error"]["code"], "commit_not_recorded");
    assert_eq!(
        value["error"]["detail"]["this_is_not_an_empty_commit"],
        true
    );
    // Nothing diff-shaped is served beside the refusal, so it cannot be read as an empty commit.
    assert!(value["changes"].is_null(), "{value}");

    // Anti-vacuity: a commit that *is* recorded answers, so the refusal is about this oid.
    let head = common::history_inventory("history-shallow")["head_oid"]
        .as_str()
        .unwrap()
        .to_string();
    assert!(
        !session.json(&format!("/api/history/commit?commit={head}"))["changes"]
            .as_array()
            .unwrap()
            .is_empty()
    );
}

// ---- first and last observed -------------------------------------------------------------------

/// **The headline gate, at this surface.** Only one answer of six may say *created*.
#[test]
fn a_boundary_path_is_the_earliest_visible_change_and_is_never_a_creation() {
    let (_dir, _root, session) = common::served_history("history-shallow");
    let path = common::inventory_changed_paths("history-shallow")
        .into_iter()
        .next()
        .expect("the shallow fixture declares a changed path");

    let response = session.get(&format!("/api/history/path?path={path}"));
    assert_eq!(response.status, 200, "{}", response.body);
    let value = response.parse_json();
    let observed = &value["first_observed"];

    // Anti-vacuity: the path really has a change row, so the refusals below are about wording
    // rather than about an empty answer.
    assert!(
        observed["changes_in_visible_history"].as_u64().unwrap() > 0,
        "the visible commit must touch {path}"
    );
    assert_eq!(observed["kind"], "earliest_visible_change");
    assert_eq!(observed["may_claim_created"], false);
    assert_eq!(observed["earlier_history_unavailable"], "shallow_boundary");
    assert!(observed["earlier_history_unavailable_note"]
        .as_str()
        .unwrap()
        .contains("expected, and not a fault"));
    assert_eq!(observed["earlier_changes_may_exist"], true);
    assert_eq!(observed["shallow"], true);
    assert!(observed["kind_note"]
        .as_str()
        .unwrap()
        .contains("earliest change Nerve can see"));
    assert!(observed["may_claim_created_note"]
        .as_str()
        .unwrap()
        .starts_with("not permitted"));

    for phrase in CREATION_PHRASES {
        assert!(
            !response.body.contains(phrase),
            "an earliest-visible change on a shallow clone claimed {phrase:?}: {}",
            response.body
        );
    }
}

#[test]
fn the_one_licensed_answer_says_created_and_names_gits_own_root_commit() {
    let (_dir, _root, session) = common::served_history("history-basic");
    let inventory = common::history_inventory("history-basic");
    let root = inventory["commits"]
        .as_array()
        .unwrap()
        .iter()
        .find(|commit| commit["parent_oids"].as_array().unwrap().is_empty())
        .expect("the fixture declares a root commit");

    let response = session.get("/api/history/path?path=README.md");
    let observed = response.parse_json()["first_observed"].clone();
    assert_eq!(observed["kind"], "created_in_visible_history");
    assert_eq!(observed["may_claim_created"], true);
    assert_eq!(observed["additions_recorded"], 1);
    assert!(observed["earlier_history_unavailable"].is_null());
    assert_eq!(
        observed["first"]["commit"]["commit_oid"], root["oid"],
        "the API and Git disagree about where README.md first appears"
    );
    assert!(
        response
            .body
            .contains("the path was created at this change"),
        "the one licensed answer must actually say it: {}",
        response.body
    );
    assert_eq!(
        observed["may_claim_created_note"],
        "permitted — this is the one answer of six that licenses it"
    );
}

/// The four states §11 requires to stay distinct, each reached and each different.
#[test]
fn the_four_silences_a_path_can_be_in_stay_distinct() {
    let mut seen: Vec<(String, bool)> = Vec::new();

    // Ingested, path in the current tree, zero change rows — the common case on a shallow clone,
    // and the one a happy-path draft reports as "this file has no history".
    let (_dir, _root, shallow) = common::served_history("history-shallow");
    let present = shallow.json("/api/history/path?path=README.md")["first_observed"].clone();
    assert_eq!(present["kind"], "present_before_visible_history");
    assert_eq!(present["changes_in_visible_history"], 0);
    assert!(
        present["current_tree"]["entities_at_path"]
            .as_u64()
            .unwrap()
            > 0,
        "the path must be in the current tree for this answer to mean anything"
    );
    assert_eq!(present["current_tree"]["basis"], "entity_table");
    seen.push((
        present["kind"].as_str().unwrap().to_string(),
        present["may_claim_created"].as_bool().unwrap(),
    ));

    // Ingested, path unknown to the current tree and to history.
    let absent = shallow.json("/api/history/path?path=no/such/file.txt")["first_observed"].clone();
    assert_eq!(absent["kind"], "absent_from_visible_history");
    assert_eq!(absent["current_tree"]["index_exists"], true);
    assert_eq!(absent["current_tree"]["entities_at_path"], 0);
    seen.push((
        absent["kind"].as_str().unwrap().to_string(),
        absent["may_claim_created"].as_bool().unwrap(),
    ));

    // Never ingested. Not a failure, and not "this path has no history".
    let (_never_dir, _never_root, never) = common::served_without_history("history-shallow");
    let response = never.get("/api/history/path?path=README.md");
    assert_eq!(response.status, 200, "an absent history is not a failure");
    let unread = response.parse_json()["first_observed"].clone();
    assert_eq!(unread["kind"], "no_history_ingested");
    assert!(unread["walk_terminated_by"].is_null());
    seen.push((
        unread["kind"].as_str().unwrap().to_string(),
        unread["may_claim_created"].as_bool().unwrap(),
    ));

    // Known, with changes, cut away by the bound — the fourth state, and it is a truncation rather
    // than a silence about the path.
    let (_basic_dir, _basic_root, basic) = common::served_history("history-basic");
    // The most-changed path, taken from the frequency endpoint rather than counted here: two
    // implementations of "which path changed most" is the duplication this row exists to avoid,
    // even in a test.
    let busy = basic.json("/api/history/frequency?limit=1")["rows"][0]["path"]
        .as_str()
        .expect("the fixture changed something")
        .to_string();
    let truncated = basic.json(&format!("/api/history/path?path={busy}&limit=1"));
    assert_eq!(truncated["commits"].as_array().unwrap().len(), 1);
    assert_eq!(truncated["truncation"]["truncated"], true);
    assert!(
        truncated["first_observed"]["changes_in_visible_history"]
            .as_u64()
            .unwrap()
            > 1,
        "the bound must be cutting something for this state to be the fourth one"
    );

    // Three different answers, and none of them licenses "created".
    let kinds: std::collections::BTreeSet<&str> =
        seen.iter().map(|(kind, _)| kind.as_str()).collect();
    assert_eq!(kinds.len(), 3, "{seen:?}");
    assert!(seen.iter().all(|(_, created)| !*created), "{seen:?}");
}

// ---- the symbol refusal ------------------------------------------------------------------------

/// A symbol-shaped path is refused with its reason, and the containing path is **never** guessed.
#[test]
fn every_path_taking_endpoint_refuses_a_symbol_selector_and_guesses_nothing() {
    let (_dir, _root, session) = common::served_history("history-basic");

    let mut refused = 0;
    for route in ["/api/history/path", "/api/history/cochange"] {
        for (encoded, decoded) in [
            ("README.md%23parse", "README.md#parse"),
            ("function%3Aparse", "function:parse"),
            ("method%3ACircle.area", "method:Circle.area"),
            ("symbol%3Aparse", "symbol:parse"),
        ] {
            let response = session.get(&format!("{route}?path={encoded}"));
            assert_eq!(response.status, 400, "{route}?path={encoded}");
            let value = response.parse_json();
            assert_eq!(value["error"]["code"], "refused_history_path");
            assert_eq!(
                value["error"]["detail"]["reason"],
                "symbol_selector_refused"
            );
            assert_eq!(value["error"]["detail"]["path_guessed"], false);
            assert_eq!(value["error"]["detail"]["nothing_was_looked_up"], true);
            assert!(value["error"]["detail"]["reason_statement"]
                .as_str()
                .unwrap()
                .contains("PathRole::None"));
            // The refusal echoes the argument, which is what was refused, and nothing else. Every
            // field a successful answer would carry is absent, so a client cannot mistake this for
            // a path with no history — and no containing path is offered in its place.
            assert_eq!(value["error"]["detail"]["argument"], decoded);
            for absent in [
                "first_observed",
                "commits",
                "rows",
                "truncation",
                "result_kind",
                "history_ingested",
            ] {
                assert!(value[absent].is_null(), "{route} {encoded}: {value}");
            }
            assert!(!value.to_string().contains("earliest_visible_change"));
            refused += 1;
        }
    }
    // Anti-vacuity, both halves: the loop really ran, and a plain path is not refused.
    assert_eq!(refused, 8);
    assert_eq!(
        session.get("/api/history/path?path=README.md").status,
        200,
        "a plain path must still be answered, or the refusal proves nothing"
    );
    // A colon below the root is part of a path, not a qualifier.
    assert_eq!(
        session.get("/api/history/path?path=docs%2Fa%3Ab.md").status,
        200
    );
}

/// **A residual, pinned rather than left to be discovered.** An *unencoded* `#` never reaches the
/// refusal, because it never reaches the handler.
///
/// `Target::parse` drops everything after the first raw `#` in a query string
/// (`crates/nerve-server/src/request.rs:58`), which mirrors HTTP itself: a fragment is a
/// client-side construct and a conformant client never transmits one. So
/// `?path=README.md#parse` arrives as `path=README.md` and is answered for `README.md`.
///
/// This is written down because it is exactly the shape of a vacuous test. A refusal test using a
/// raw `#` would pass against a handler with **no refusal at all** — the argument would never have
/// been symbol-shaped by the time anything looked at it. `%23` is the only form that reaches the
/// gate, which is why every case above uses it.
#[test]
fn an_unencoded_fragment_is_dropped_before_the_refusal_can_see_it() {
    let (_dir, _root, session) = common::served_history("history-basic");

    let plain = session.json("/api/history/path?path=README.md");
    let fragmented = session.json("/api/history/path?path=README.md#parse");
    assert_eq!(
        fragmented["path"], "README.md",
        "the fragment must have been dropped before parsing"
    );
    assert_eq!(fragmented["first_observed"], plain["first_observed"]);

    // And the encoded form of the same string *is* refused, which is what makes the two
    // distinguishable rather than a single lenient path.
    let encoded = session.get("/api/history/path?path=README.md%23parse");
    assert_eq!(encoded.status, 400);
    assert_eq!(
        encoded.parse_json()["error"]["detail"]["reason"],
        "symbol_selector_refused"
    );
}

// ---- bounds -------------------------------------------------------------------------------------

#[test]
fn every_history_list_is_bounded_and_reports_truncation_as_a_measured_fact() {
    let (_dir, _root, session) = common::served_history("history-basic");

    // Frequency: bounded, with the total it was cut against.
    let all = session.json("/api/history/frequency");
    let paths_total = all["paths_total"].as_u64().unwrap();
    assert!(paths_total > 1, "the fixture must change several paths");
    assert_eq!(all["truncation"]["truncated"], false);
    assert_eq!(
        all["rows"].as_array().unwrap().len() as u64,
        paths_total.min(50)
    );
    // Deterministically ordered: count descending, then path ascending.
    let rows = all["rows"].as_array().unwrap();
    for pair in rows.windows(2) {
        let (left, right) = (&pair[0], &pair[1]);
        let (lc, rc) = (
            left["commits"].as_i64().unwrap(),
            right["commits"].as_i64().unwrap(),
        );
        assert!(
            lc > rc || (lc == rc && left["path"].as_str() <= right["path"].as_str()),
            "{left} then {right}"
        );
    }

    let cut = session.json("/api/history/frequency?limit=1");
    assert_eq!(cut["rows"].as_array().unwrap().len(), 1);
    assert_eq!(cut["truncation"]["truncated"], true);
    assert_eq!(cut["truncation"]["total"], serde_json::json!(paths_total));
    assert_eq!(cut["continuation"]["supported"], false);
    assert!(cut["continuation"]["statement"].as_str().unwrap().len() > 40);

    // The ceiling clamps rather than trusting the caller, and a non-number is refused rather than
    // silently defaulted.
    let clamped = session.json("/api/history/frequency?limit=100000");
    assert_eq!(
        clamped["truncation"]["limit"],
        serde_json::json!(nerve_server::api::history::MAX_HISTORY_FREQUENCY_LIMIT)
    );
    for target in [
        "/api/history/frequency?limit=abc",
        "/api/history/commits?limit=0",
        "/api/history/path?path=README.md&limit=abc",
    ] {
        let response = session.get(target);
        assert_eq!(response.status, 400, "{target}: {}", response.body);
        assert_eq!(response.parse_json()["error"]["code"], "bad_request");
    }

    // A path's own history has no total, so truncation is established by fetching one row past the
    // limit. Both directions are checked: cut, and exactly full.
    let busy = session.json("/api/history/frequency?limit=1")["rows"][0]["path"]
        .as_str()
        .unwrap()
        .to_string();
    let commits = session.json(&format!("/api/history/path?path={busy}"))["commits"]
        .as_array()
        .unwrap()
        .len();
    assert!(commits > 1, "{busy} must have been changed more than once");
    let cut = session.json(&format!("/api/history/path?path={busy}&limit=1"));
    assert_eq!(cut["truncation"]["truncated"], true);
    let exact = session.json(&format!("/api/history/path?path={busy}&limit={commits}"));
    assert_eq!(exact["commits"].as_array().unwrap().len(), commits);
    assert_eq!(
        exact["truncation"]["truncated"], false,
        "a page that exactly fills the limit is not truncated"
    );
}

// ---- co-change ------------------------------------------------------------------------------

/// Co-change is an **observation**. The label is enforced, not merely intended.
#[test]
fn cochange_is_labelled_an_observation_and_never_a_dependency() {
    let (_dir, _root, session) = common::served_history("history-shallow");
    let path = common::inventory_changed_paths("history-shallow")
        .into_iter()
        .next()
        .expect("a changed path");

    let response = session.get(&format!("/api/history/cochange?path={path}"));
    assert_eq!(response.status, 200, "{}", response.body);
    let value = response.parse_json();

    // Anti-vacuity: there really are pairs, so the forbidden-word check below is about naming.
    let rows = value["rows"].as_array().unwrap();
    assert!(!rows.is_empty(), "the fixture's commit changes two paths");
    assert!(value["pairs_total"].as_u64().unwrap() > 0);
    for row in rows {
        assert!(row["cochange_observations"].as_i64().unwrap() > 0, "{row}");
        assert!(row["path_a"].as_str().unwrap() < row["path_b"].as_str().unwrap());
    }

    // The store's sentence, byte for byte, not a paraphrase written on this surface.
    assert_eq!(
        value["disclaimer"],
        nerve_store::COCHANGE_IS_NOT_A_DEPENDENCY
    );
    assert!(value["disclaimer"]
        .as_str()
        .unwrap()
        .contains("an observation, not a dependency"));

    // And the words that would invite the inference are absent from the whole answer.
    let body = value.to_string();
    for forbidden in [
        "related_paths",
        "\"related\"",
        "coupled",
        "coupling_score",
        "depends",
        "affinity",
    ] {
        assert!(!body.contains(forbidden), "{forbidden} in {body}");
    }

    // Nothing was written. Co-change exists in a response and nowhere else, so no relation names
    // it — asserted by a count over what the graph actually holds, not by inspection.
    let overview = session.json("/api/overview");
    let relations = overview["assertions_by_relation"].as_object().unwrap();
    for name in relations.keys() {
        let lowered = name.to_ascii_lowercase();
        assert!(
            !lowered.contains("cochange") && !lowered.contains("co_change"),
            "a relation was emitted for co-change: {name}"
        );
    }
    // Anti-vacuity: the graph does hold relations, so the absence above is about co-change.
    assert!(
        overview["assertions_total"].as_u64().unwrap() > 0,
        "{overview}"
    );
    assert!(!relations.is_empty());
}

// ---- the state diff --------------------------------------------------------------------------

#[test]
fn the_state_diff_answers_by_ancestry_and_refuses_rather_than_returning_an_empty_diff() {
    let (_dir, _root, session) = common::served_history("history-basic");
    let inventory = common::history_inventory("history-basic");
    let head = inventory["head_oid"].as_str().unwrap();
    let root = inventory["commits"]
        .as_array()
        .unwrap()
        .iter()
        .find(|commit| commit["parent_oids"].as_array().unwrap().is_empty())
        .expect("a root commit")["oid"]
        .as_str()
        .unwrap();

    // Ancestry, forwards.
    let value = session.json(&format!("/api/history/diff?from={root}&to={head}"));
    assert_eq!(value["result_kind"], "diff");
    assert_eq!(value["ancestry_not_a_time_range"], true);
    let in_range = value["commits_in_range"].as_u64().unwrap();
    assert!(in_range > 0, "{value}");
    assert_eq!(
        in_range as usize,
        inventory["commits"].as_array().unwrap().len() - 1,
        "`from` is excluded, so the range is every commit but the root"
    );
    assert!(!value["changes"].as_array().unwrap().is_empty());
    assert!(value["merges_in_range"].is_number());
    assert!(value["changes_enumerated"]["enumerated"].as_u64().unwrap() > 0);
    assert_eq!(value["truncation"]["truncated"], false);

    // Backwards: not an ancestor, and **not** an empty diff.
    let value = session.json(&format!("/api/history/diff?from={head}&to={root}"));
    assert_eq!(value["result_kind"], "not_an_ancestor");
    assert_eq!(value["this_is_not_an_empty_diff"], true);
    assert!(value["commits_walked"].as_u64().is_some());
    // The diff-shaped keys are present and `null`: no range was computed, which is a different
    // fact from a range that holds nothing.
    for key in [
        "commits",
        "changes",
        "commits_in_range",
        "changes_truncated",
    ] {
        assert!(value[key].is_null(), "{key}: {value}");
        assert!(value.get(key).is_some(), "{key} must be present");
    }

    // An oid Nerve never read is named as such, and which end it was.
    let value = session.json(&format!(
        "/api/history/diff?from=0000000000000000000000000000000000000000&to={head}"
    ));
    assert_eq!(value["result_kind"], "state_not_recorded");
    assert_eq!(value["from_recorded"], false);
    assert_eq!(value["to_recorded"], true);
    assert!(value["commits"].is_null());

    // Bounded, with truncation as a fact.
    let value = session.json(&format!("/api/history/diff?from={root}&to={head}&limit=1"));
    assert_eq!(value["commits"].as_array().unwrap().len(), 1);
    assert_eq!(value["commits_truncated"], true);
    assert_eq!(value["truncation"]["truncated"], true);
    assert_eq!(
        value["changes_truncated"], true,
        "a cut commit list is necessarily a cut diff"
    );
}

// ---- the guard -------------------------------------------------------------------------------

/// Authentication, `Host` and `Origin` gate the history routes too — verified, not assumed.
///
/// The guard runs before dispatch, so this *should* hold automatically. "Should" is what this test
/// exists to replace: the asset relaxation in `router::resolve` lets a request through on a
/// `MissingToken` verdict when the path resolves in the asset table, and nothing but this check
/// establishes that no `/api/history*` path does.
#[test]
fn no_history_route_answers_without_the_token_or_from_another_host_or_origin() {
    let (_dir, _root, session) = common::served_history("history-shallow");
    let targets = history_targets();
    assert_eq!(targets.len(), 7, "{targets:?}");

    for target in &targets {
        // No token at all.
        let response = session.raw("GET", target, &[("Host", &session.host())]);
        assert_eq!(response.status, 401, "{target}: {}", response.body);
        assert_eq!(response.parse_json()["error"]["code"], "token_required");

        // A token that is not this session's.
        let response = session.raw(
            "GET",
            target,
            &[
                ("Host", &session.host()),
                (nerve_server::token::TOKEN_HEADER, &"0".repeat(64)),
            ],
        );
        assert_eq!(response.status, 403, "{target}");
        assert_eq!(response.parse_json()["error"]["code"], "token_invalid");

        // A forged `Host`, which is the DNS-rebinding defence.
        let response = session.raw(
            "GET",
            target,
            &[
                ("Host", "nerve.evil.test"),
                (nerve_server::token::TOKEN_HEADER, session.token()),
            ],
        );
        assert_eq!(response.status, 403, "{target}");
        assert_eq!(response.parse_json()["error"]["code"], "host_not_allowed");

        // Another document's origin.
        let response = session.raw(
            "GET",
            target,
            &[
                ("Host", &session.host()),
                ("Origin", "https://evil.test"),
                (nerve_server::token::TOKEN_HEADER, session.token()),
            ],
        );
        assert_eq!(response.status, 403, "{target}");
        assert_eq!(response.parse_json()["error"]["code"], "origin_not_allowed");

        // And nothing historical leaked into the refusal.
        let body = response.body.as_str();
        assert!(!body.contains("commit_oid"), "{target}: {body}");
        assert!(!body.contains("shallow_boundary"), "{target}: {body}");
    }

    // Anti-vacuity: with the token, the same targets answer. Four refusals each of a route that
    // does not exist would otherwise be a green test over nothing.
    for target in &targets {
        let response = session.get(target);
        assert_ne!(response.status, 401, "{target}");
        assert_ne!(response.status, 403, "{target}");
        if response.status != 200 {
            assert_ne!(
                response.parse_json()["error"]["code"],
                "no_such_route",
                "{target}"
            );
        }
    }
}

/// Only `GET` is routed, so no history endpoint can be reached by a forged form submission.
#[test]
fn no_history_route_is_reachable_by_any_method_but_get() {
    let (_dir, _root, session) = common::served_history("history-shallow");
    for target in history_targets() {
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
        }
    }
}

// ---- read-only -------------------------------------------------------------------------------

/// The load-bearing promise, for this family too: reading history cannot change the index.
#[test]
fn a_full_history_session_leaves_the_database_byte_identical() {
    let (_dir, root, session) = common::served_history("history-shallow");
    let db_path = nerve_index::config::db_path(&root);
    let inventory = common::history_inventory("history-shallow");
    let head = inventory["head_oid"].as_str().unwrap();
    let boundary = inventory["shallow"]["boundary_oids"][0].as_str().unwrap();
    let path = common::inventory_changed_paths("history-shallow")
        .into_iter()
        .next()
        .unwrap();

    let before = std::fs::read(&db_path).unwrap();
    let before_hash = content_hash(&before);

    let mut answered = 0;
    for target in [
        "/api/history".to_string(),
        "/api/history/commits?limit=2".to_string(),
        format!("/api/history/commit?commit={head}"),
        format!("/api/history/commit?commit={boundary}"),
        format!("/api/history/path?path={path}"),
        "/api/history/path?path=README.md".to_string(),
        format!("/api/history/diff?from={boundary}&to={head}"),
        "/api/history/frequency?limit=10".to_string(),
        format!("/api/history/cochange?path={path}"),
    ] {
        let response = session.get(&target);
        assert_eq!(response.status, 200, "{target}: {}", response.body);
        answered += 1;
    }
    assert_eq!(
        answered, 9,
        "every history endpoint must have been exercised"
    );

    // Refusals must not write either — an audit log would be a write.
    let _ = session.get("/api/history/path?path=x%23y");
    let _ = session.get("/api/history/commit?commit=deadbeef");
    let _ = session.raw("GET", "/api/history", &[("Host", "evil.test")]);
    let _ = session.raw("POST", "/api/history", &[("Host", &session.host())]);

    let after = std::fs::read(&db_path).unwrap();
    assert_eq!(before.len(), after.len(), "database size changed");
    assert_eq!(
        before_hash,
        content_hash(&after),
        "the database changed during a read-only history session"
    );
}

// ---- repository text ---------------------------------------------------------------------------

/// A commit summary is repository prose. It is carried as data and never as vocabulary.
#[test]
fn a_hostile_commit_summary_is_carried_as_a_string_and_never_as_a_field_name() {
    let (_dir, _root, session) = common::served_history("history-hostile");
    let response = session.get("/api/history/commits?limit=200");
    assert_eq!(response.status, 200, "{}", response.body);
    let value = response.parse_json();

    let commits = value["commits"].as_array().unwrap();
    assert!(!commits.is_empty());
    let hostile: Vec<&serde_json::Value> = commits
        .iter()
        .filter(|commit| {
            let summary = commit["summary"].as_str().unwrap_or_default();
            summary.contains('<') || summary.contains("IGNORE ALL PREVIOUS")
        })
        .collect();
    // Anti-vacuity: the fixture really does carry hostile prose, so the properties below are about
    // how it is served rather than about a fixture that happens to be tame.
    assert!(
        !hostile.is_empty(),
        "the hostile fixture must carry a hostile summary: {commits:?}"
    );

    for commit in &hostile {
        // A JSON string value, never a key and never a vocabulary field.
        assert!(commit["summary"].is_string());
        for vocabulary in [
            "parent_completeness",
            "changes_enumerated",
            "commit_oid",
            "tree_oid",
        ] {
            assert_ne!(commit[vocabulary], commit["summary"], "{vocabulary}");
        }
        assert!(commit.as_object().unwrap().keys().all(|key| key
            .chars()
            .all(|character| character.is_ascii_lowercase() || character == '_')));
    }

    // And the served bytes cannot open markup, whatever the summary says.
    assert!(
        !response.body.contains("<script"),
        "a raw `<` reached the wire: {}",
        response.body
    );
    assert!(
        response.body.contains("\\u003c"),
        "the escaping that makes this safe must be visible in the bytes"
    );
}
