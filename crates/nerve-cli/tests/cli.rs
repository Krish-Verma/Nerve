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
            "documents_processed",
            "adr_documents",
            "document_sections",
            "unsupported_markdown",
            "unsupported_markdown_by_form",
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
    assert_eq!(status["last_run"]["extractor_id"], "md-structural");

    // Every run for the current state is reported, not only the last.
    let runs = status["runs"].as_array().unwrap();
    assert_eq!(runs.len(), 4, "one run per extractor");
    assert_eq!(runs[0]["extractor_id"], "fs-structural");
    assert_eq!(runs[0]["extractor_version"], "1.0.0");
    assert_eq!(runs[1]["extractor_id"], "ts-js-structural");
    assert_eq!(runs[1]["extractor_version"], "1.1.0");
    assert_eq!(runs[2]["extractor_id"], "ts-js-reference");
    assert_eq!(runs[2]["extractor_version"], "1.0.0");
    assert_eq!(runs[3]["extractor_id"], "md-structural");
    // Slice 5d-ii: `SUPERSEDES` edges from the four explicit supersession fields. The version
    // moved with the behaviour and in the same commit as it, because that is what makes every
    // document re-scan once on this build rather than keep a graph the current rules would not
    // produce.
    assert_eq!(runs[3]["extractor_version"], "1.2.0");
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

// ---- coverage ---------------------------------------------------------------------------------

/// The whole workflow a user runs: index, hand Nerve a report, ask why a symbol is covered.
#[test]
fn coverage_ingests_the_named_report_and_why_reports_it() {
    let (_dir, root) = indexed_fixture("ts-coverage");
    let root = root.to_str().unwrap();

    let ingest = run(&["coverage", "coverage/lcov.info", root]);
    assert_eq!(code(&ingest), 0, "{}", stdout(&ingest));
    let rendered = stdout(&ingest);
    assert!(
        rendered.contains("6 covered (4 fully, 2 partially)"),
        "{rendered}"
    );
    // The product language says "the test suite", never "this test" (ADR-0008).
    assert!(
        rendered.contains("Coverage is not a call graph"),
        "{rendered}"
    );

    let value = json(&run(&["coverage", "coverage/lcov.info", root, "--json"]));
    require_keys(
        &value,
        &[
            "command",
            "ok",
            "exit_code",
            "root",
            "report_path",
            "report_content_hash",
            "coverage_run_entity_id",
            "state_id",
            "status",
            "files_in_report",
            "files_ingested",
            "files_refused",
            "symbols_covered",
            "symbols_fully_covered",
            "symbols_partially_covered",
            "covered_lines",
            "uncovered_lines",
            "refused",
            "refused_total",
            "rows_written",
            "duration_ms",
            "per_test_attribution",
        ],
    );
    assert_eq!(value["command"], "coverage");
    assert_eq!(value["status"], "complete");
    assert_eq!(value["symbols_covered"], 6);
    assert_eq!(value["symbols_partially_covered"], 2);
    assert_eq!(value["files_ingested"], 2);
    assert_eq!(
        value["per_test_attribution"], false,
        "LCOV carries none, and the output says so rather than leaving it to be assumed"
    );
    assert!(value["coverage_run_entity_id"]
        .as_str()
        .unwrap()
        .starts_with("cov_"));

    // `nerve why` on a covered symbol reports the edge, its evidence profile and its freshness.
    let why = json(&run(&[
        "why",
        "src/math.ts#clamp",
        "--relation",
        "COVERS",
        "--path",
        root,
        "--json",
    ]));
    assert_eq!(why["count"], 1);
    let assertion = &why["assertions"][0];
    assert_eq!(assertion["relation"], "COVERS");
    assert_eq!(assertion["source"]["kind"], "coverage_run");
    assert_eq!(assertion["target"]["name"], "clamp");
    let observation = &assertion["observations"][0];
    assert_eq!(observation["evidence_source_type"], "TEST_COVERAGE");
    assert_eq!(observation["directness"], "INFERRED");
    assert_eq!(observation["extractor_id"], "coverage");
    assert_eq!(observation["freshness"], "fresh");
    assert_eq!(observation["details"]["coverage"], "partial");
    assert_eq!(observation["file_path"], "src/math.ts");
}

/// Coverage needs an index, and says which step is missing rather than failing obscurely.
#[test]
fn coverage_without_an_index_exits_two() {
    let (_dir, root) = named_fixture_copy("ts-coverage");
    let root = root.to_str().unwrap();

    let uninitialized = run(&["coverage", "coverage/lcov.info", root]);
    assert_eq!(code(&uninitialized), 2);
    assert!(String::from_utf8_lossy(&uninitialized.stderr).contains("nerve init"));

    assert_eq!(code(&run(&["init", root])), 0);
    let unindexed = run(&["coverage", "coverage/lcov.info", root]);
    assert_eq!(code(&unindexed), 2);
    assert!(String::from_utf8_lossy(&unindexed.stderr).contains("nerve index"));
}

/// A report outside the repository is a wrong argument, not an internal failure.
#[test]
fn coverage_refuses_a_report_outside_the_repository_with_exit_ten() {
    let (dir, root) = indexed_fixture("ts-coverage");
    std::fs::write(
        dir.path().join("outside.info"),
        "SF:src/math.ts\nDA:1,1\nend_of_record\n",
    )
    .unwrap();
    let root = root.to_str().unwrap();

    for named in ["../outside.info", "/etc/passwd"] {
        let refused = run(&["coverage", named, root]);
        assert_eq!(code(&refused), 10, "{named}: {}", stdout(&refused));
    }
}

/// A report Nerve could only partly believe exits 3, exactly as a partial index does.
#[test]
fn a_partly_refused_report_exits_three() {
    let (_dir, root) = indexed_fixture("ts-coverage");
    std::fs::write(
        root.join("coverage/lcov.info"),
        "TN:\nSF:src/math.ts\nDA:1,1\nend_of_record\n\
         TN:\nSF:../../../../etc/passwd\nDA:1,1\nend_of_record\n",
    )
    .unwrap();
    let root = root.to_str().unwrap();

    let partial = run(&["coverage", "coverage/lcov.info", root]);
    assert_eq!(code(&partial), 3, "{}", stdout(&partial));

    let value = json(&run(&["coverage", "coverage/lcov.info", root, "--json"]));
    assert_eq!(value["status"], "partial");
    assert_eq!(value["exit_code"], 3);
    assert_eq!(value["files_refused"], 1);
    assert_eq!(value["refused"]["path-refused"], 1);
    // The refused path is counted and never echoed.
    assert!(!serde_json::to_string(&value).unwrap().contains("passwd"));
}

// ---- gaps -------------------------------------------------------------------------------------

/// Copy, index and ingest `fixtures/ts-coverage`'s report in one step.
fn covered_fixture() -> (tempfile::TempDir, PathBuf) {
    let (dir, root) = indexed_fixture("ts-coverage");
    assert_eq!(
        code(&run(&[
            "coverage",
            "coverage/lcov.info",
            root.to_str().unwrap()
        ])),
        0
    );
    (dir, root)
}

/// The failure the whole command exists to avoid.
///
/// `fixtures/ts-basic` has 24 symbols and no coverage. The naive implementation reports 24 gaps
/// and reads as "your tests cover nothing". The truth is that Nerve has been told nothing, and
/// the command must say that instead — with `totals` **null** rather than a row of zeroes, so a
/// script cannot read the absence of evidence as evidence of coverage.
#[test]
fn a_repository_with_no_coverage_says_the_question_is_unanswerable() {
    let (_dir, root) = indexed_fixture("ts-basic");
    let root = root.to_str().unwrap();

    let plain = run(&["gaps", root]);
    assert_eq!(code(&plain), 0, "reporting a fact is not a failure");
    let rendered = stdout(&plain);
    assert!(rendered.contains("No coverage evidence in"), "{rendered}");
    assert!(rendered.contains("unanswerable"), "{rendered}");
    assert!(rendered.contains("nerve coverage"), "{rendered}");

    let value = json(&run(&["gaps", root, "--json"]));
    assert_eq!(value["coverage"], "absent");
    assert_eq!(value["answerable"], false);
    assert_eq!(value["totals"], serde_json::Value::Null);
    assert_eq!(value["count"], 0);
    assert_eq!(value["results_total"], 0);
    assert_eq!(value["results"], serde_json::json!([]));
    assert_eq!(value["runs"], serde_json::json!([]));
    assert!(
        value["symbols_in_scope"].as_u64().unwrap() > 0,
        "the symbols are still counted, so the message can say how much is unanswered"
    );
}

/// With coverage ingested, every one of the four states is reported and none is rounded away.
#[test]
fn an_ingested_report_makes_the_gap_question_answerable() {
    let (_dir, root) = covered_fixture();
    let root = root.to_str().unwrap();

    let plain = run(&["gaps", root]);
    assert_eq!(code(&plain), 0, "gaps found is not a failure");
    let rendered = stdout(&plain);
    assert!(rendered.contains("Coverage gaps in"), "{rendered}");
    assert!(rendered.contains("neverRun"), "{rendered}");

    let value = json(&run(&["gaps", root, "--json"]));
    require_keys(
        &value,
        &[
            "command",
            "ok",
            "exit_code",
            "root",
            "coverage",
            "answerable",
            "under",
            "kind",
            "include_partial",
            "limit",
            "runs",
            "symbols_in_scope",
            "totals",
            "count",
            "results_total",
            "truncated",
            "files_probed",
            "results",
        ],
    );
    assert_eq!(value["coverage"], "present");
    assert_eq!(value["answerable"], true);
    assert_eq!(value["exit_code"], 0);

    // `fixtures/ts-coverage/expected.json` is the hand-read ground truth: 8 symbols, 6 edges,
    // 4 fully covered, 2 partial, and exactly two symbols with no edge at all.
    let totals = &value["totals"];
    assert_eq!(value["symbols_in_scope"], 8);
    assert_eq!(totals["covered"], 4);
    assert_eq!(totals["partial"], 2);
    assert_eq!(totals["uncovered"], 2);
    assert_eq!(totals["unmeasured"], 0);
    assert_eq!(totals["gaps"], 2);
    assert_eq!(totals["stale"], 0);
    assert_eq!(totals["measured_files"], 2);

    let names: Vec<&str> = value["results"]
        .as_array()
        .unwrap()
        .iter()
        .map(|row| row["entity"]["name"].as_str().unwrap())
        .collect();
    assert_eq!(names, vec!["neverRun", "Shape"]);
    for row in value["results"].as_array().unwrap() {
        assert_eq!(row["state"], "uncovered");
        assert_eq!(row["coverage_freshness"], "fresh");
    }

    // The answer names the run it is relative to.
    let run_row = &value["runs"][0];
    assert_eq!(run_row["report_path"], "coverage/lcov.info");
    assert_eq!(run_row["freshness"], "fresh");
    assert_eq!(run_row["source_files_in_report"], 2);
}

/// A partially covered symbol is not a gap, is counted anyway, and is listed on request.
#[test]
fn partial_is_surfaced_rather_than_rounded() {
    let (_dir, root) = covered_fixture();
    let root = root.to_str().unwrap();

    let default = json(&run(&["gaps", root, "--json"]));
    assert!(default["results"]
        .as_array()
        .unwrap()
        .iter()
        .all(|row| row["state"] != "partial"));

    let widened = json(&run(&["gaps", root, "--include-partial", "--json"]));
    assert_eq!(widened["totals"]["partial"], 2);
    let partial: Vec<&str> = widened["results"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|row| row["state"] == "partial")
        .map(|row| row["entity"]["name"].as_str().unwrap())
        .collect();
    assert_eq!(partial, vec!["clamp", "perimeter"]);
    let clamp = widened["results"]
        .as_array()
        .unwrap()
        .iter()
        .find(|row| row["entity"]["name"] == "clamp")
        .unwrap();
    assert_eq!(clamp["covered_lines"], 5);
    assert_eq!(clamp["instrumented_lines"], 6);
    assert_eq!(
        clamp["covered_by"],
        serde_json::json!(["coverage/lcov.info"])
    );
}

/// Editing a covered file makes every answer about it stale, gaps included.
#[test]
fn a_gap_computed_from_stale_coverage_is_labelled_stale() {
    let (_dir, root) = covered_fixture();
    let source = root.join("src/math.ts");
    let text = std::fs::read_to_string(&source).unwrap();
    std::fs::write(
        &source,
        format!("{text}\nexport function later() {{ return 1; }}\n"),
    )
    .unwrap();
    let root = root.to_str().unwrap();
    assert_eq!(code(&run(&["index", root])), 0);

    let value = json(&run(&["gaps", root, "--include-partial", "--json"]));
    assert_eq!(value["coverage"], "present");
    assert_eq!(value["totals"]["stale_files"], 1);
    assert!(value["totals"]["stale"].as_u64().unwrap() >= 3);

    let never_run = value["results"]
        .as_array()
        .unwrap()
        .iter()
        .find(|row| row["entity"]["name"] == "neverRun")
        .expect("the measured gap survives the edit");
    assert_eq!(
        never_run["coverage_freshness"], "stale",
        "a gap computed from coverage that no longer matches the file is a stale gap"
    );

    // The symbol the edit added was never in any report, and its file is measured, so it is a
    // measured gap rather than an unmeasured one.
    let later = value["results"]
        .as_array()
        .unwrap()
        .iter()
        .find(|row| row["entity"]["name"] == "later")
        .expect("the new symbol is a gap");
    assert_eq!(later["state"], "uncovered");
}

/// The two absence states are different answers, and both surfaces say which.
#[test]
fn a_file_no_report_named_is_unmeasured_rather_than_uncovered() {
    let (_dir, root) = covered_fixture();
    std::fs::write(
        root.join("src/elsewhere.ts"),
        "export function untouched(): number { return 7; }\n",
    )
    .unwrap();
    let root = root.to_str().unwrap();
    assert_eq!(code(&run(&["index", root])), 0);

    let value = json(&run(&["gaps", root, "--json"]));
    let untouched = value["results"]
        .as_array()
        .unwrap()
        .iter()
        .find(|row| row["entity"]["name"] == "untouched")
        .expect("a symbol in a file no report named is a gap");
    assert_eq!(
        untouched["state"], "unmeasured",
        "no coverage evidence names this file, so its absence is silence, not a measurement"
    );
    assert_eq!(untouched["coverage_freshness"], serde_json::Value::Null);
    assert_eq!(value["totals"]["unmeasured"], 1);
    assert_eq!(value["totals"]["uncovered"], 2);
}

/// Truncation is reported, and the tallies stay exact when the rows are cut.
#[test]
fn gaps_truncation_is_honest() {
    let (_dir, root) = covered_fixture();
    let root = root.to_str().unwrap();

    let full = json(&run(&["gaps", root, "--json"]));
    assert_eq!(full["truncated"], false);
    assert_eq!(full["count"], 2);

    let capped = json(&run(&["gaps", root, "--limit", "1", "--json"]));
    assert_eq!(capped["truncated"], true);
    assert_eq!(capped["count"], 1);
    assert_eq!(capped["results_total"], 2);
    assert_eq!(
        capped["totals"]["gaps"], 2,
        "the tally is exact whatever the cap cuts"
    );
    assert!(stdout(&run(&["gaps", root, "--limit", "1"])).contains("Listed 1 of 2"));
}

/// Filters narrow the question without changing what the states mean.
#[test]
fn gaps_filters_scope_by_path_and_by_kind() {
    let (_dir, root) = covered_fixture();
    let root = root.to_str().unwrap();

    let scoped = json(&run(&["gaps", root, "--under", "src/shapes.ts", "--json"]));
    assert_eq!(scoped["under"], "src/shapes.ts");
    assert_eq!(scoped["symbols_in_scope"], 5);
    assert_eq!(scoped["count"], 1);
    assert_eq!(scoped["results"][0]["entity"]["name"], "Shape");

    let by_kind = json(&run(&["gaps", root, "--kind", "interface", "--json"]));
    assert_eq!(by_kind["kind"], "interface");
    assert_eq!(by_kind["count"], 1);
    assert_eq!(by_kind["results"][0]["entity"]["name"], "Shape");

    // A coverage gap is about a symbol; a non-symbol kind is a wrong argument, not an empty
    // answer that would read as "nothing of that kind is uncovered".
    let refused = run(&["gaps", root, "--kind", "file"]);
    assert_eq!(code(&refused), 10);
    assert!(stderr(&refused).contains("a coverage gap is about a symbol"));
    assert_eq!(code(&run(&["gaps", root, "--kind", "nonsense"])), 10);
    assert_eq!(code(&run(&["gaps", root, "--limit", "0"])), 10);
}

/// Identical repository, identical bytes on stdout.
#[test]
fn gaps_output_is_byte_identical_across_runs() {
    let (_dir, root) = covered_fixture();
    let root = root.to_str().unwrap();

    let first = stdout(&run(&["gaps", root, "--include-partial", "--json"]));
    for _ in 0..4 {
        assert_eq!(
            stdout(&run(&["gaps", root, "--include-partial", "--json"])),
            first
        );
    }
    let rendered = stdout(&run(&["gaps", root, "--include-partial"]));
    assert_eq!(stdout(&run(&["gaps", root, "--include-partial"])), rendered);
}

/// The CLI and the HTTP API must give the **same answer**, asserted rather than assumed.
///
/// Both call one query in `nerve-store` (ARCHITECTURE.md invariant 3). This compares the whole
/// payload — states, tallies, freshness, run list, truncation — field by field, after removing
/// the two keys that are legitimately per-surface: the CLI's envelope and its `root`, which the
/// API does not repeat because a client already knows which server it is talking to.
#[cfg(unix)]
#[test]
fn the_cli_and_the_api_answer_the_gap_question_identically() {
    use std::io::{Read, Write};
    use std::process::Stdio;

    let (_dir, root) = covered_fixture();
    let root_arg = root.to_str().unwrap().to_string();

    /// Kills the spawned server however this test leaves — including by panicking.
    ///
    /// Without this the `kill` at the end of the body is only reached when every assertion
    /// passes. A failing assertion would leave `nerve serve` running, and because the test
    /// harness inherits its piped stdout, the orphan holds that pipe open and `cargo test`
    /// blocks forever collecting output. The suite would hang instead of reporting the failure,
    /// which is the worst possible way for a regression to present in CI.
    ///
    /// Observed, not theorised: an orchestrator mutation probe against the gap query hung the
    /// whole workspace run for over ten minutes at this exact test until the orphan was killed
    /// by hand. `crates/nerve-server/tests/common/mod.rs` already guards its `Session` this way.
    struct Reaper(std::process::Child);
    impl Drop for Reaper {
        fn drop(&mut self) {
            let _ = Command::new("kill")
                .args(["-TERM", &self.0.id().to_string()])
                .status();
            let _ = self.0.wait();
        }
    }

    let mut spawned = Command::new(binary())
        .args(["serve", &root_arg, "--json", "--port", "0"])
        .stdout(Stdio::piped())
        .spawn()
        .expect("nerve serve must start");
    let mut out = spawned.stdout.take().unwrap();
    let child = Reaper(spawned);
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
    let address = announcement["address"].as_str().unwrap().to_string();
    let token = announcement["token"].as_str().unwrap().to_string();
    assert!(
        announcement["routes"]
            .as_array()
            .unwrap()
            .iter()
            .any(|route| route == "/api/gaps"),
        "the endpoint must be advertised"
    );

    let api = |target: &str| -> serde_json::Value {
        let mut socket = std::net::TcpStream::connect(&address).expect("connect");
        socket
            .write_all(
                format!(
                    "GET {target} HTTP/1.1\r\nHost: {address}\r\nX-Nerve-Token: {token}\r\n\
                     Connection: close\r\n\r\n"
                )
                .as_bytes(),
            )
            .unwrap();
        let mut response = String::new();
        socket.read_to_string(&mut response).unwrap();
        assert!(response.starts_with("HTTP/1.1 200"), "{response}");
        let body = response.split_once("\r\n\r\n").expect("a body").1;
        serde_json::from_str(body).expect("the API must answer JSON")
    };

    for (cli_args, target) in [
        (vec!["gaps", &root_arg, "--json"], "/api/gaps?limit=50"),
        (
            vec!["gaps", &root_arg, "--include-partial", "--json"],
            "/api/gaps?limit=50&include_partial=1",
        ),
        (
            vec!["gaps", &root_arg, "--limit", "1", "--json"],
            "/api/gaps?limit=1",
        ),
        (
            vec!["gaps", &root_arg, "--under", "src/math.ts", "--json"],
            "/api/gaps?limit=50&under=src%2Fmath.ts",
        ),
    ] {
        let mut from_cli = json(&run(&cli_args));
        let mut from_api = api(target);
        for key in ["command", "ok", "exit_code", "root"] {
            from_cli.as_object_mut().unwrap().remove(key);
        }
        from_api.as_object_mut().unwrap().remove("ok");
        assert_eq!(
            from_cli, from_api,
            "the CLI and the API disagreed for {target}"
        );
    }

    // And the unanswerable state crosses the wire unchanged, which is the one that matters.
    let (_basic_dir, basic_root) = indexed_fixture("ts-basic");
    let basic = basic_root.to_str().unwrap();
    let mut from_cli = json(&run(&["gaps", basic, "--json"]));
    for key in ["command", "ok", "exit_code", "root"] {
        from_cli.as_object_mut().unwrap().remove(key);
    }
    assert_eq!(from_cli["coverage"], "absent");
    assert_eq!(from_cli["totals"], serde_json::Value::Null);

    // `Reaper::drop` sends the TERM and reaps, on this path and on every panicking one.
    drop(child);
}
