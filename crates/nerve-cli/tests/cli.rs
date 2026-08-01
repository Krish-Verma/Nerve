//! End-to-end CLI smoke tests and the `--json` output contract.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn binary() -> &'static str {
    env!("CARGO_BIN_EXE_nerve")
}

fn copy_tree(source: &Path, destination: &Path) {
    std::fs::create_dir_all(destination).unwrap();
    let mut entries: Vec<_> = std::fs::read_dir(source)
        .unwrap()
        .map(|entry| entry.unwrap())
        .collect();
    entries.sort_by_key(|entry| entry.file_name());
    for entry in entries {
        if entry.file_name() == ".nerve" {
            continue;
        }
        let from = entry.path();
        let to = destination.join(entry.file_name());
        if from.is_dir() {
            copy_tree(&from, &to);
        } else {
            std::fs::copy(&from, &to).unwrap();
        }
    }
}

fn fixture_copy() -> (tempfile::TempDir, PathBuf) {
    named_fixture_copy("ts-basic")
}

/// Copy a committed fixture into a temporary directory. The fixture itself is never touched.
fn named_fixture_copy(name: &str) -> (tempfile::TempDir, PathBuf) {
    let source = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures")
        .join(name)
        .canonicalize()
        .unwrap();
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("repo");
    copy_tree(&source, &root);
    (dir, root)
}

/// Copy, initialize and index a fixture, returning the temp directory and its root.
fn indexed_fixture(name: &str) -> (tempfile::TempDir, PathBuf) {
    let (dir, root) = named_fixture_copy(name);
    let path = root.to_str().unwrap();
    assert_eq!(code(&run(&["init", path])), 0);
    assert_eq!(code(&run(&["index", path])), 0);
    (dir, root)
}

fn run(args: &[&str]) -> Output {
    Command::new(binary())
        .args(args)
        .output()
        .expect("nerve binary must run")
}

fn code(output: &Output) -> i32 {
    output.status.code().expect("process exited with a code")
}

fn stdout(output: &Output) -> String {
    String::from_utf8(output.stdout.clone()).unwrap()
}

fn json(output: &Output) -> serde_json::Value {
    serde_json::from_str(&stdout(output))
        .unwrap_or_else(|err| panic!("--json output did not parse: {err}\n{}", stdout(output)))
}

fn require_keys(value: &serde_json::Value, keys: &[&str]) {
    let object = value.as_object().expect("--json output must be an object");
    for key in keys {
        assert!(
            object.contains_key(*key),
            "missing key {key:?} in {}",
            serde_json::to_string_pretty(value).unwrap()
        );
    }
}

#[test]
fn init_index_status_search_end_to_end() {
    let (_dir, root) = fixture_copy();
    let root = root.to_str().unwrap();

    let init = run(&["init", root]);
    assert_eq!(code(&init), 0, "{}", String::from_utf8_lossy(&init.stderr));
    assert!(stdout(&init).contains("Initialized Nerve index"));
    assert!(Path::new(root).join(".nerve/nerve.db").is_file());
    assert!(Path::new(root).join(".nerve/cache").is_dir());
    assert!(Path::new(root).join(".nerve/logs").is_dir());

    let index = run(&["index", root]);
    assert_eq!(
        code(&index),
        0,
        "{}",
        String::from_utf8_lossy(&index.stderr)
    );
    assert!(stdout(&index).contains("Indexed"));

    let status = run(&["status", "--path", root]);
    assert_eq!(code(&status), 0);
    assert!(stdout(&status).contains("healthy        yes"));

    let search = run(&["search", "area", "--path", root]);
    assert_eq!(code(&search), 0);
    assert!(stdout(&search).contains("Circle.area"));
}

#[test]
fn init_is_idempotent_from_the_command_line() {
    let (_dir, root) = fixture_copy();
    let root = root.to_str().unwrap();
    assert_eq!(code(&run(&["init", root])), 0);
    let second = run(&["init", root]);
    assert_eq!(code(&second), 0);
    assert!(stdout(&second).contains("already initialized"));
}

#[test]
fn commands_that_need_an_index_exit_two_without_one() {
    let (_dir, root) = fixture_copy();
    let root = root.to_str().unwrap();
    assert_eq!(code(&run(&["index", root])), 2);
    assert_eq!(code(&run(&["status", "--path", root])), 2);
    assert_eq!(code(&run(&["search", "x", "--path", root])), 2);
}

#[test]
fn a_partial_index_exits_three() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    std::fs::create_dir_all(root.join("src")).unwrap();
    std::fs::write(root.join("src/a.ts"), "export const a = 1;\n").unwrap();
    std::fs::write(root.join("src/broken.ts"), [0xff, 0xfe, 0x41]).unwrap();
    let root = root.to_str().unwrap();

    assert_eq!(code(&run(&["init", root])), 0);
    let index = run(&["index", root]);
    assert_eq!(code(&index), 3, "{}", stdout(&index));

    let value = json(&run(&["index", root, "--json"]));
    assert_eq!(value["status"], "partial");
    assert_eq!(value["files_failed"], 1);
    assert_eq!(value["exit_code"], 3);
}

#[test]
fn an_unusable_root_exits_ten() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("not-a-directory.txt");
    std::fs::write(&file, "x").unwrap();
    assert_eq!(code(&run(&["init", file.to_str().unwrap()])), 10);
}

#[test]
fn a_bad_argument_exits_ten() {
    assert_eq!(code(&run(&["nonexistent-subcommand"])), 10);
    assert_eq!(code(&run(&["search"])), 10, "missing required query");
}

#[test]
fn an_unknown_kind_filter_exits_ten() {
    let (_dir, root) = fixture_copy();
    let root = root.to_str().unwrap();
    run(&["init", root]);
    run(&["index", root]);
    let output = run(&["search", "area", "--kind", "widget", "--path", root]);
    assert_eq!(code(&output), 10);
    assert!(String::from_utf8_lossy(&output.stderr).contains("unknown --kind"));
}

#[test]
fn help_and_version_exit_zero() {
    assert_eq!(code(&run(&["--help"])), 0);
    assert_eq!(code(&run(&["--version"])), 0);
    assert_eq!(code(&run(&[])), 0, "bare invocation prints help");
}

#[test]
fn an_empty_result_set_is_a_success() {
    let (_dir, root) = fixture_copy();
    let root = root.to_str().unwrap();
    run(&["init", root]);
    run(&["index", root]);

    let output = run(&["search", "zzzznosuchsymbol", "--path", root]);
    assert_eq!(code(&output), 0);

    let value = json(&run(&[
        "search",
        "zzzznosuchsymbol",
        "--path",
        root,
        "--json",
    ]));
    assert_eq!(value["count"], 0);
    assert_eq!(value["results"].as_array().unwrap().len(), 0);
}

#[test]
fn quiet_suppresses_human_output_but_not_json() {
    let (_dir, root) = fixture_copy();
    let root = root.to_str().unwrap();
    let quiet_init = run(&["init", root, "--quiet"]);
    assert_eq!(code(&quiet_init), 0);
    assert!(stdout(&quiet_init).is_empty());

    run(&["index", root]);
    let quiet_json = run(&["status", "--path", root, "--quiet", "--json"]);
    assert!(!stdout(&quiet_json).is_empty());
    json(&quiet_json);
}

#[test]
fn no_color_is_accepted_and_output_carries_no_ansi_escapes() {
    let (_dir, root) = fixture_copy();
    let root = root.to_str().unwrap();
    run(&["init", root]);
    let output = run(&["index", root, "--no-color"]);
    assert_eq!(code(&output), 0);
    assert!(!stdout(&output).contains('\u{1b}'));
}

// ---- JSON contract -----------------------------------------------------------------------

#[test]
fn every_json_output_parses_and_carries_its_required_keys() {
    let (_dir, root) = fixture_copy();
    let root = root.to_str().unwrap();

    let init = json(&run(&["init", root, "--json"]));
    require_keys(
        &init,
        &[
            "command",
            "ok",
            "exit_code",
            "root",
            "nerve_dir",
            "database_path",
            "project_id",
            "schema_version",
            "created",
        ],
    );
    assert_eq!(init["command"], "init");
    assert_eq!(init["ok"], true);
    assert_eq!(init["schema_version"], nerve_store::SCHEMA_VERSION);

    let index = json(&run(&["index", root, "--json"]));
    require_keys(
        &index,
        &[
            "command",
            "ok",
            "exit_code",
            "root",
            "state_id",
            "git_commit",
            "status",
            "files_processed",
            "files_failed",
            "files_with_syntax_errors",
            "skipped_unsupported",
            "skipped_symlinks",
            "denied_secrets",
            "dynamic_imports_without_specifier",
            "unmodelled_call_sites",
            "unmodelled_by_form",
            "entities_total",
            "entities_by_kind",
            "assertions_total",
            "assertions_by_relation",
            "observations_total",
            "unresolved_entities",
            "unresolved_assertions",
            "duration_ms",
        ],
    );
    assert_eq!(index["command"], "index");
    assert_eq!(index["status"], "complete");
    assert_eq!(index["files_processed"], 8);
    assert_eq!(
        index["unresolved_entities"], 6,
        "2 unresolved module specifiers plus 4 unresolved call targets"
    );
    assert_eq!(index["dynamic_imports_without_specifier"], 1);
    assert_eq!(
        index["unmodelled_call_sites"], 2,
        "one require() and one import() in ts-basic"
    );
    assert_eq!(index["assertions_by_relation"]["CALLS"], 12);
    assert_eq!(index["assertions_by_relation"]["REFERENCES"], 5);
    assert_eq!(index["assertions_by_relation"]["IMPLEMENTS"], 2);
    assert_eq!(
        index["denied_secrets"].as_array().unwrap(),
        &vec![serde_json::Value::from(".env")]
    );

    let status = json(&run(&["status", "--path", root, "--json"]));
    require_keys(
        &status,
        &[
            "command",
            "ok",
            "exit_code",
            "healthy",
            "database_path",
            "database_bytes",
            "schema_version",
            "project_id",
            "root_path",
            "state_id",
            "git_commit",
            "entities_total",
            "entities_by_kind",
            "assertions_total",
            "assertions_by_relation",
            "occurrences_total",
            "observations_total",
            "assertion_states_total",
            "unresolved_entities",
            "unresolved_assertions",
            "last_run",
            "runs",
        ],
    );
    assert_eq!(status["command"], "status");
    assert_eq!(status["healthy"], true);
    require_keys(
        &status["last_run"],
        &[
            "run_id",
            "state_id",
            "extractor_id",
            "extractor_version",
            "started_at",
            "finished_at",
            "files_processed",
            "files_failed",
            "status",
        ],
    );
    assert_eq!(status["last_run"]["extractor_id"], "ts-js-reference");

    // Every run for the current state is reported, not only the last.
    let runs = status["runs"].as_array().unwrap();
    assert_eq!(runs.len(), 2, "one run per extractor");
    assert_eq!(runs[0]["extractor_id"], "ts-js-structural");
    assert_eq!(runs[0]["extractor_version"], "1.1.0");
    assert_eq!(runs[1]["extractor_id"], "ts-js-reference");
    assert_eq!(runs[1]["extractor_version"], "1.0.0");
    require_keys(&runs[0], &["run_id", "state_id", "status"]);

    let search = json(&run(&["search", "area", "--path", root, "--json"]));
    require_keys(
        &search,
        &[
            "command",
            "ok",
            "exit_code",
            "query",
            "kind",
            "limit",
            "count",
            "results",
        ],
    );
    assert_eq!(search["command"], "search");
    assert!(search["count"].as_u64().unwrap() >= 2);
    require_keys(
        &search["results"][0],
        &[
            "entity_id",
            "kind",
            "name",
            "scope_path",
            "language",
            "file_path",
            "start_line",
            "end_line",
            "score",
        ],
    );
}

#[test]
fn json_failures_are_objects_too() {
    let (_dir, root) = fixture_copy();
    let root = root.to_str().unwrap();
    let output = run(&["status", "--path", root, "--json"]);
    assert_eq!(code(&output), 2);
    let value = json(&output);
    require_keys(&value, &["command", "ok", "exit_code", "error"]);
    assert_eq!(value["command"], "status");
    assert_eq!(value["ok"], false);
    assert_eq!(value["exit_code"], 2);
}

#[test]
fn search_respects_the_kind_filter_and_limit() {
    let (_dir, root) = fixture_copy();
    let root = root.to_str().unwrap();
    run(&["init", root]);
    run(&["index", root]);

    let methods = json(&run(&[
        "search", "area", "--kind", "method", "--path", root, "--json",
    ]));
    let results = methods["results"].as_array().unwrap();
    assert!(!results.is_empty());
    assert!(results.iter().all(|hit| hit["kind"] == "method"));

    let limited = json(&run(&[
        "search", "a", "--limit", "1", "--path", root, "--json",
    ]));
    assert!(limited["results"].as_array().unwrap().len() <= 1);
}

#[test]
fn status_reports_the_schema_version_and_state() {
    let (_dir, root) = fixture_copy();
    let root_str = root.to_str().unwrap();
    run(&["init", root_str]);
    let index = json(&run(&["index", root_str, "--json"]));
    let status = json(&run(&["status", "--path", root_str, "--json"]));
    assert_eq!(status["schema_version"], nerve_store::SCHEMA_VERSION);
    assert_eq!(status["state_id"], index["state_id"]);
    assert!(status["database_bytes"].as_u64().unwrap() > 0);
}

/// `init`, `index` and `status` all take the repository positionally. `nerve status .` erroring
/// while `nerve index .` worked was an inconsistency found in the Slice 3 product review.
#[test]
fn status_accepts_the_repository_positionally_and_via_path() {
    let (_dir, root) = fixture_copy();
    let root_str = root.to_str().unwrap();
    run(&["init", root_str]);
    run(&["index", root_str]);

    let positional = json(&run(&["status", root_str, "--json"]));
    let flagged = json(&run(&["status", "--path", root_str, "--json"]));
    assert_eq!(positional["state_id"], flagged["state_id"]);
    assert_eq!(positional["healthy"], serde_json::Value::Bool(true));
}

// ---- graph query surface (Slice 2b) --------------------------------------------------------

fn stderr(output: &Output) -> String {
    String::from_utf8(output.stderr.clone()).unwrap()
}

#[test]
fn path_finds_a_known_multi_hop_route_and_pins_the_hop_count() {
    let (_dir, root) = indexed_fixture("ts-resolution");
    let root = root.to_str().unwrap();

    // viaBarrel calls `plus`, which is `add` re-exported through index.ts and barrel.ts;
    // `add` then calls `normalize` inside math.ts. Two hops, and only two.
    let output = run(&["path", "viaBarrel", "normalize", "--path", root]);
    assert_eq!(code(&output), 0, "{}", stderr(&output));

    let value = json(&run(&[
        "path",
        "viaBarrel",
        "normalize",
        "--path",
        root,
        "--json",
    ]));
    assert_eq!(value["count"], 1);
    assert_eq!(value["paths"][0]["length"], 2);
    assert_eq!(value["paths"][0]["traverses_unresolved"], false);
    let hops = value["paths"][0]["hops"].as_array().unwrap();
    let relations: Vec<&str> = hops
        .iter()
        .map(|hop| hop["relation"].as_str().unwrap())
        .collect();
    assert_eq!(relations, vec!["CALLS", "CALLS"]);
    assert_eq!(hops[0]["from"]["name"], "viaBarrel");
    assert_eq!(hops[0]["to"]["name"], "add");
    assert_eq!(hops[0]["file_path"], "src/app.ts");
    assert_eq!(hops[0]["start_line"], 10);
    assert_eq!(hops[0]["strongest_source_type"], "AST_RESOLVED");
    assert_eq!(hops[1]["to"]["name"], "normalize");
    assert_eq!(hops[1]["file_path"], "src/math.ts");
    assert_eq!(hops[1]["start_line"], 4);
}

#[test]
fn path_with_no_route_exits_zero_and_says_so() {
    let (_dir, root) = indexed_fixture("ts-resolution");
    let root = root.to_str().unwrap();

    // The call graph runs one way: normalize never reaches back up to viaBarrel.
    let output = run(&["path", "normalize", "viaBarrel", "--path", root]);
    assert_eq!(code(&output), 0, "absence of a path is not an error");
    assert!(
        stdout(&output).contains("No path found within depth 6"),
        "{}",
        stdout(&output)
    );

    let value = json(&run(&[
        "path",
        "normalize",
        "viaBarrel",
        "--path",
        root,
        "--json",
    ]));
    assert_eq!(value["count"], 0);
    assert_eq!(value["truncated"], false);
    assert_eq!(value["max_depth"], 6);
}

#[test]
fn path_json_carries_its_contract_and_marks_unresolved_hops() {
    let (_dir, root) = indexed_fixture("ts-resolution");
    let root = root.to_str().unwrap();

    // `console` is a host global, so the call leaves what Nerve can see. It is still a path.
    let value = json(&run(&[
        "path",
        "report",
        "console.log",
        "--path",
        root,
        "--json",
    ]));
    require_keys(
        &value,
        &[
            "command",
            "ok",
            "exit_code",
            "from",
            "to",
            "max_depth",
            "limit",
            "direction",
            "relations",
            "resolved_only",
            "truncated",
            "expansions",
            "count",
            "paths",
        ],
    );
    assert_eq!(value["command"], "path");
    assert_eq!(value["direction"], "forward");
    assert_eq!(value["count"], 1);
    assert_eq!(value["paths"][0]["traverses_unresolved"], true);
    let hop = &value["paths"][0]["hops"][0];
    require_keys(
        hop,
        &[
            "relation",
            "assertion_id",
            "from",
            "to",
            "traversed_backwards",
            "is_unresolved",
            "status",
            "strongest_source_type",
            "observation_count",
            "file_path",
            "start_line",
        ],
    );
    assert_eq!(hop["is_unresolved"], true);
    assert_eq!(hop["status"], "UNRESOLVED");
    require_keys(
        &value["from"],
        &[
            "entity_id",
            "kind",
            "name",
            "scope_path",
            "qualified_name",
            "language",
            "file_path",
            "start_line",
            "end_line",
        ],
    );

    // The same route disappears when unresolved edges are excluded, and that is still exit 0.
    let filtered = json(&run(&[
        "path",
        "report",
        "console.log",
        "--resolved-only",
        "--path",
        root,
        "--json",
    ]));
    assert_eq!(filtered["count"], 0);
    assert_eq!(filtered["resolved_only"], true);
}

#[test]
fn why_prints_every_observation_with_extractor_version_and_location() {
    let (_dir, root) = indexed_fixture("ts-resolution");
    let root = root.to_str().unwrap();

    let human = run(&["why", "add", "normalize", "--path", root]);
    assert_eq!(code(&human), 0, "{}", stderr(&human));
    let text = stdout(&human);
    assert!(text.contains("CALLS"), "{text}");
    assert!(text.contains("ts-js-reference 1.0.0"), "{text}");
    assert!(text.contains("src/math.ts:4"), "{text}");
    assert!(text.contains("freshness fresh"), "{text}");

    let value = json(&run(&["why", "add", "normalize", "--path", root, "--json"]));
    require_keys(
        &value,
        &[
            "command",
            "ok",
            "exit_code",
            "subject",
            "object",
            "direction",
            "relations",
            "files_probed",
            "count",
            "assertions",
        ],
    );
    assert_eq!(value["command"], "why");
    assert_eq!(value["count"], 1);
    let assertion = &value["assertions"][0];
    require_keys(
        assertion,
        &[
            "assertion_id",
            "relation",
            "direction",
            "source",
            "target",
            "status",
            "is_unresolved",
            "observation_count",
            "strongest_source_type",
            "observations",
        ],
    );
    assert_eq!(assertion["relation"], "CALLS");
    assert_eq!(assertion["direction"], "outgoing");
    assert_eq!(assertion["status"], "SUPPORTED");
    assert_eq!(assertion["is_unresolved"], false);
    assert_eq!(assertion["observation_count"], 1);
    assert_eq!(assertion["strongest_source_type"], "AST_RESOLVED");

    let observation = &assertion["observations"][0];
    require_keys(
        observation,
        &[
            "observation_id",
            "evidence_source_type",
            "directness",
            "extractor_id",
            "extractor_version",
            "match_quality",
            "state_id",
            "file_path",
            "start_line",
            "end_line",
            "content_hash",
            "environment",
            "details",
            "created_at",
            "freshness",
        ],
    );
    assert_eq!(observation["evidence_source_type"], "AST_RESOLVED");
    assert_eq!(observation["directness"], "RESOLVED");
    assert_eq!(observation["extractor_id"], "ts-js-reference");
    assert_eq!(observation["extractor_version"], "1.0.0");
    assert_eq!(observation["file_path"], "src/math.ts");
    assert_eq!(observation["start_line"], 4);
    assert_eq!(observation["freshness"], "fresh");
    assert_eq!(observation["match_quality"], serde_json::Value::Null);
}

#[test]
fn why_reports_stale_only_for_the_file_that_changed() {
    let (_dir, root) = indexed_fixture("ts-resolution");
    let math = root.join("src/math.ts");
    let root = root.to_str().unwrap();

    let before = json(&run(&[
        "why",
        "add",
        "--relation",
        "CALLS",
        "--path",
        root,
        "--json",
    ]));
    for assertion in before["assertions"].as_array().unwrap() {
        for observation in assertion["observations"].as_array().unwrap() {
            assert_eq!(observation["freshness"], "fresh");
        }
    }

    let original = std::fs::read_to_string(&math).unwrap();
    std::fs::write(&math, format!("{original}\n// touched after indexing\n")).unwrap();

    let after = json(&run(&[
        "why",
        "add",
        "--relation",
        "CALLS",
        "--path",
        root,
        "--json",
    ]));
    let mut stale = 0;
    let mut fresh = 0;
    for assertion in after["assertions"].as_array().unwrap() {
        for observation in assertion["observations"].as_array().unwrap() {
            let expected = if observation["file_path"] == "src/math.ts" {
                stale += 1;
                "stale"
            } else {
                fresh += 1;
                "fresh"
            };
            assert_eq!(
                observation["freshness"],
                expected,
                "{}",
                serde_json::to_string(observation).unwrap()
            );
        }
    }
    assert!(stale > 0, "the mutated file must produce a stale reading");
    assert!(fresh > 0, "untouched files must stay fresh");

    // Freshness is derived, so restoring the bytes restores the answer without re-indexing.
    std::fs::write(&math, &original).unwrap();
    let restored = json(&run(&[
        "why",
        "add",
        "--relation",
        "CALLS",
        "--path",
        root,
        "--json",
    ]));
    assert_eq!(restored, before);
}

#[test]
fn why_hashes_each_file_once_however_many_observations_quote_it() {
    let (_dir, root) = indexed_fixture("ts-resolution");
    let root = root.to_str().unwrap();
    let value = json(&run(&["why", "add", "--path", root, "--json"]));

    let mut files = std::collections::BTreeSet::new();
    let mut observations = 0;
    for assertion in value["assertions"].as_array().unwrap() {
        for observation in assertion["observations"].as_array().unwrap() {
            files.insert(observation["file_path"].as_str().unwrap().to_string());
            observations += 1;
        }
    }
    assert!(observations > files.len(), "the test needs repeated files");
    assert_eq!(value["files_probed"], files.len());
}

#[test]
fn an_ambiguous_selector_exits_ten_and_lists_every_candidate() {
    let (_dir, root) = indexed_fixture("ts-resolution");
    let root = root.to_str().unwrap();

    let output = run(&["path", "area", "normalize", "--path", root]);
    assert_eq!(code(&output), 10);
    let text = stderr(&output);
    assert!(text.contains("Rectangle.area"), "{text}");
    assert!(text.contains("Circle.area"), "{text}");
    assert!(
        text.contains("meth_"),
        "candidate ids must be printed: {text}"
    );

    let value = json(&run(&[
        "path",
        "area",
        "normalize",
        "--path",
        root,
        "--json",
    ]));
    assert_eq!(value["exit_code"], 10);
    assert_eq!(value["ok"], false);
    assert_eq!(value["selector"], "area");
    assert_eq!(value["selector_role"], "from");
    assert_eq!(value["matched_by"], "name");
    let candidates = value["candidates"].as_array().unwrap();
    assert_eq!(candidates.len(), 3);
    assert!(candidates.iter().all(|candidate| candidate["entity_id"]
        .as_str()
        .unwrap()
        .starts_with("meth_")));

    // `why` refuses in exactly the same way.
    assert_eq!(code(&run(&["why", "area", "--path", root])), 10);

    // Naming the file disambiguates it.
    let resolved = json(&run(&[
        "why",
        "src/shapes.ts#Rectangle.area",
        "--path",
        root,
        "--json",
    ]));
    assert_eq!(resolved["subject"]["qualified_name"], "Rectangle.area");
}

#[test]
fn a_selector_that_matches_nothing_exits_two_with_suggestions() {
    let (_dir, root) = indexed_fixture("ts-resolution");
    let root = root.to_str().unwrap();

    let output = run(&["path", "viaBarrel", "normalise", "--path", root]);
    assert_eq!(code(&output), 2);
    assert!(stderr(&output).contains("matches no indexed entity"));

    let value = json(&run(&[
        "path",
        "viaBarrel",
        "normalise",
        "--path",
        root,
        "--json",
    ]));
    assert_eq!(value["exit_code"], 2);
    assert_eq!(value["selector"], "normalise");
    assert_eq!(value["selector_role"], "to");
    let suggestions = value["suggestions"].as_array().unwrap();
    assert!(
        suggestions.iter().any(|hit| hit["name"] == "normalize"),
        "{suggestions:?}"
    );

    assert_eq!(code(&run(&["why", "nosuchsymbolatall", "--path", root])), 2);
}

#[test]
fn bad_graph_arguments_exit_ten() {
    let (_dir, root) = indexed_fixture("ts-resolution");
    let root = root.to_str().unwrap();
    assert_eq!(
        code(&run(&[
            "path",
            "add",
            "normalize",
            "--relation",
            "SUMMONS",
            "--path",
            root
        ])),
        10
    );
    assert_eq!(
        code(&run(&[
            "path",
            "add",
            "normalize",
            "--max-depth",
            "0",
            "--path",
            root
        ])),
        10
    );
    assert_eq!(
        code(&run(&[
            "path",
            "add",
            "normalize",
            "--limit",
            "0",
            "--path",
            root
        ])),
        10
    );
    assert_eq!(
        code(&run(&[
            "why",
            "add",
            "--incoming",
            "--outgoing",
            "--path",
            root
        ])),
        10,
        "--incoming and --outgoing contradict each other"
    );
}

#[test]
fn graph_commands_need_an_index() {
    let (_dir, root) = named_fixture_copy("ts-resolution");
    let root = root.to_str().unwrap();
    assert_eq!(code(&run(&["path", "a", "b", "--path", root])), 2);
    assert_eq!(code(&run(&["why", "a", "--path", root])), 2);
}

/// ARCHITECTURE.md invariant 3: surfaces contain no business logic.
///
/// The CLI may parse arguments, render, and map exit codes. Traversal, evidence assembly and
/// SQL live in `nerve-store`, so the Slice 4 server and Slice 8 MCP tools reuse them unchanged.
#[test]
fn the_cli_contains_no_traversal_or_query_logic() {
    const MAIN: &str = include_str!("../src/main.rs");
    for forbidden in [
        "SELECT ",
        "INSERT INTO",
        "FROM assertion",
        "JOIN ",
        "ORDER BY",
        "prepare(",
        "query_row",
        "rusqlite",
        "VecDeque",
        "BinaryHeap",
        "blake3",
    ] {
        assert!(
            !MAIN.contains(forbidden),
            "nerve-cli must not contain {forbidden:?}"
        );
    }

    const MANIFEST: &str = include_str!("../Cargo.toml");
    for forbidden in ["rusqlite", "blake3", "tree-sitter"] {
        assert!(
            !MANIFEST.contains(forbidden),
            "nerve-cli must not depend on {forbidden:?}"
        );
    }
}

// ---- serve ---------------------------------------------------------------------------------

#[test]
fn serve_refuses_a_directory_with_no_index() {
    let (_dir, root) = named_fixture_copy("ts-resolution");
    let serve = run(&["serve", root.to_str().unwrap()]);
    assert_eq!(
        code(&serve),
        2,
        "{}",
        String::from_utf8_lossy(&serve.stderr)
    );
    assert!(String::from_utf8_lossy(&serve.stderr).contains("no Nerve index"));
}

#[test]
fn serve_refuses_a_path_that_does_not_exist() {
    let serve = run(&["serve", "/nerve/definitely/not/here"]);
    assert_eq!(code(&serve), 10);
}

/// The whole `nerve serve` lifecycle from the command line: it binds loopback, prints a URL
/// carrying the session token, answers only with that token, and stops on SIGTERM leaving the
/// index writable.
#[cfg(unix)]
#[test]
fn serve_prints_a_usable_url_and_stops_on_a_signal() {
    use std::io::{Read, Write};
    use std::process::Stdio;

    let (_dir, root) = indexed_fixture("ts-resolution");
    let mut child = Command::new(binary())
        .args(["serve", root.to_str().unwrap(), "--json", "--port", "0"])
        .stdout(Stdio::piped())
        .spawn()
        .expect("nerve serve must start");

    // `serve` prints its banner and then blocks, so read until what has arrived parses.
    let mut out = child.stdout.take().unwrap();
    let mut buffer = Vec::new();
    let mut chunk = [0u8; 512];
    let announcement = loop {
        let read = out.read(&mut chunk).expect("reading serve output");
        assert!(read > 0, "serve exited without announcing itself");
        buffer.extend_from_slice(&chunk[..read]);
        if let Ok(value) = serde_json::from_slice::<serde_json::Value>(&buffer) {
            break value;
        }
    };

    require_keys(
        &announcement,
        &[
            "command",
            "ok",
            "address",
            "port",
            "base_url",
            "url",
            "token",
            "token_header",
            "routes",
        ],
    );
    let address = announcement["address"].as_str().unwrap().to_string();
    let token = announcement["token"].as_str().unwrap().to_string();
    assert!(address.starts_with("127.0.0.1:"), "{address}");
    assert_eq!(token.len(), 64);
    assert!(announcement["url"].as_str().unwrap().contains(&token));
    assert_eq!(announcement["token_header"], "X-Nerve-Token");

    let get = |headers: String| -> String {
        let mut socket = std::net::TcpStream::connect(&address).expect("connect");
        socket
            .write_all(
                format!("GET /api/overview HTTP/1.1\r\nHost: {address}\r\n{headers}Connection: close\r\n\r\n")
                    .as_bytes(),
            )
            .unwrap();
        let mut response = String::new();
        socket.read_to_string(&mut response).unwrap();
        response
    };

    assert!(
        get(format!("X-Nerve-Token: {token}\r\n")).starts_with("HTTP/1.1 200"),
        "the printed token must work"
    );
    assert!(
        get(String::new()).starts_with("HTTP/1.1 401"),
        "no token must be refused"
    );

    // SIGTERM, not SIGKILL: the point is that the graceful path works.
    let terminated = Command::new("kill")
        .args(["-TERM", &child.id().to_string()])
        .status()
        .expect("kill must run");
    assert!(terminated.success());
    let status = child.wait().expect("serve must exit");
    assert_eq!(status.code(), Some(0), "serve must exit cleanly");

    // Nothing is left holding the index.
    assert!(std::net::TcpStream::connect(&address).is_err());
    assert_eq!(code(&run(&["index", root.to_str().unwrap()])), 0);
}
