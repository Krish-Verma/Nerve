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
    let source = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures/ts-basic")
        .canonicalize()
        .unwrap();
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("repo");
    copy_tree(&source, &root);
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
    assert_eq!(init["schema_version"], 1);

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
    assert_eq!(status["schema_version"], 1);
    assert_eq!(status["state_id"], index["state_id"]);
    assert!(status["database_bytes"].as_u64().unwrap() > 0);
}
