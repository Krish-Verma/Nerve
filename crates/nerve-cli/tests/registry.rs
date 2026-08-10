//! `nerve repo` at the command line (Slice 13a-ii).
//!
//! The surface's own job is narrow — render, and map an outcome to an exit code — so what is
//! asserted here is narrow too: that a refusal reaches the exit code and the JSON with its reason
//! *named*, that a retired entry is still printed, and that a neighbour's database is byte-identical
//! after every command that touches it. The derivations these commands render are asserted at the
//! layer that owns them, in `crates/nerve-index/tests/registry.rs`.
//!
//! Every repository here is `nerve init`-ed with its own directory, so no two share a `repo_id`:
//! `repo_id` derives from the `project_id` that `init` generates, and two checkouts with the same
//! identity would make the moved-target assertions pass for the wrong reason.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn binary() -> &'static str {
    env!("CARGO_BIN_EXE_nerve")
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

fn stderr(output: &Output) -> String {
    String::from_utf8(output.stderr.clone()).unwrap()
}

fn json(output: &Output) -> serde_json::Value {
    serde_json::from_str(&stdout(output))
        .unwrap_or_else(|err| panic!("--json output did not parse: {err}\n{}", stdout(output)))
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

/// Copy a fixture to `<base>/<name>`, `nerve init` it, and optionally index it.
fn repository(base: &Path, name: &str, fixture: &str, index: bool) -> PathBuf {
    let source = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures")
        .join(fixture)
        .canonicalize()
        .unwrap();
    let root = base.join(name);
    copy_tree(&source, &root);
    let path = root.to_str().unwrap();
    assert_eq!(code(&run(&["init", path])), 0, "init {name}");
    if index {
        assert_eq!(code(&run(&["index", path])), 0, "index {name}");
    }
    root
}

/// A temporary directory holding an indexed `a` and an indexed `b`.
fn two_repositories() -> (tempfile::TempDir, PathBuf, PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    let a = repository(dir.path(), "a", "ts-basic", true);
    let b = repository(dir.path(), "b", "ts-resolution", true);
    (dir, a, b)
}

fn as_str(path: &Path) -> &str {
    path.to_str().unwrap()
}

/// The whole round trip: register, list, retire, and see the retired entry still listed.
#[test]
fn a_neighbour_is_registered_listed_and_retired_without_its_row_disappearing() {
    let (_dir, a, b) = two_repositories();
    let a = as_str(&a);

    let added = run(&["repo", "add", as_str(&b), "--path", a, "--json"]);
    assert_eq!(code(&added), 0, "{}", stderr(&added));
    let entry = &json(&added)["entries"][0];
    for key in [
        "registry_id",
        "expected_repository_id",
        "display_name",
        "local_path",
        "added_at",
        "status",
        "status_note",
        "availability",
        "availability_statement",
        "refusal",
        "freshness",
        "freshness_note",
        "last_seen_state",
        "availability_checked_at",
    ] {
        assert!(
            entry.get(key).is_some(),
            "the response must carry {key}: {entry}"
        );
    }
    assert_eq!(entry["status"], "active");
    assert_eq!(entry["availability"], "available");
    assert_eq!(entry["freshness"], serde_json::Value::Null);
    assert_eq!(entry["refusal"], serde_json::Value::Null);
    let registry_id = entry["registry_id"].as_str().unwrap().to_string();

    let listed = json(&run(&["repo", "list", "--path", a, "--json"]));
    assert_eq!(listed["entries"].as_array().unwrap().len(), 1);
    assert_eq!(listed["entries"][0]["registry_id"], registry_id.as_str());

    // Retiring keeps the row. That is the whole reason `registry_entry_removed` is reportable.
    let removed = run(&["repo", "remove", &registry_id, "--path", a, "--json"]);
    assert_eq!(code(&removed), 0, "{}", stderr(&removed));
    assert_eq!(json(&removed)["entries"][0]["status"], "tombstoned");

    let after = json(&run(&["repo", "list", "--path", a, "--json"]));
    let entries = after["entries"].as_array().unwrap();
    assert_eq!(entries.len(), 1, "a retired entry is still listed: {after}");
    assert_eq!(entries[0]["status"], "tombstoned");
    assert_eq!(entries[0]["availability"], "entry_removed");
    assert_eq!(entries[0]["freshness"], "registry_entry_removed");
    assert!(entries[0]["withdrawn_at"].is_string());

    // And the human surface marks it rather than printing it like any other row.
    let text = stdout(&run(&["repo", "list", "--path", a]));
    assert!(text.contains("tombstoned"), "{text}");
    assert!(text.contains("registry_entry_removed"), "{text}");

    // Removing twice is refused rather than re-dating the ending.
    let again = run(&["repo", "remove", &registry_id, "--path", a, "--json"]);
    assert_eq!(code(&again), 10);
    assert_eq!(json(&again)["refusal"], "registry_entry_tombstoned");
}

/// Every refusal reaches the exit code and the JSON with its own name.
#[test]
fn each_registration_refusal_names_itself_and_exits_ten() {
    let (dir, a, b) = two_repositories();
    let a_path = as_str(&a);

    let file = dir.path().join("a-file");
    std::fs::write(&file, b"not a repository").unwrap();
    let bare = dir.path().join("bare");
    std::fs::create_dir_all(&bare).unwrap();
    let link = dir.path().join("link-to-b");
    std::os::unix::fs::symlink(&b, &link).unwrap();

    for (target, expected) in [
        (dir.path().join("nowhere"), "path_does_not_exist"),
        (file, "path_is_not_a_directory"),
        (bare, "no_nerve_database"),
        (link, "symlink_escape"),
        (a.clone(), "same_repository"),
    ] {
        let output = run(&["repo", "add", as_str(&target), "--path", a_path, "--json"]);
        assert_eq!(code(&output), 10, "{expected}: {}", stdout(&output));
        let body = json(&output);
        assert_eq!(body["ok"], false);
        assert_eq!(body["refusal"], expected);
        assert!(
            body["refusal_statement"].as_str().unwrap().len() > 40,
            "{expected} has no statement"
        );
        // Nothing was half-done: the registry is still empty.
        let listed = json(&run(&["repo", "list", "--path", a_path, "--json"]));
        assert!(listed["entries"].as_array().unwrap().is_empty());
    }

    // And the same command with a real neighbour succeeds, so the refusals above are decisions
    // rather than a command that never works.
    assert_eq!(
        code(&run(&["repo", "add", as_str(&b), "--path", a_path])),
        0
    );
    let twice = run(&["repo", "add", as_str(&b), "--path", a_path, "--json"]);
    assert_eq!(code(&twice), 10);
    assert_eq!(json(&twice)["refusal"], "already_registered");
}

/// **Relocation is refused when the new path holds a different repository.**
///
/// Both halves, because the refusal alone is satisfied by a `relocate` that never works.
#[test]
fn relocate_refuses_a_path_holding_a_different_repository_and_accepts_the_recorded_one() {
    let (dir, a, b) = two_repositories();
    let elsewhere = repository(dir.path(), "elsewhere", "ts-resolution", true);
    let a_path = as_str(&a);
    assert_eq!(
        code(&run(&[
            "repo",
            "add",
            as_str(&b),
            "--path",
            a_path,
            "--id",
            "neighbour"
        ])),
        0
    );

    let refused = run(&[
        "repo",
        "relocate",
        "neighbour",
        as_str(&elsewhere),
        "--path",
        a_path,
        "--json",
    ]);
    assert_eq!(code(&refused), 10, "{}", stdout(&refused));
    assert_eq!(json(&refused)["refusal"], "target_repository_moved");

    // The entry did not move.
    let listed = json(&run(&["repo", "list", "--path", a_path, "--json"]));
    assert_eq!(
        listed["entries"][0]["local_path"],
        b.canonicalize().unwrap().to_string_lossy().as_ref()
    );

    // The same repository, genuinely moved, is accepted.
    let moved = dir.path().join("b-moved");
    std::fs::rename(&b, &moved).unwrap();
    let accepted = run(&[
        "repo",
        "relocate",
        "neighbour",
        as_str(&moved),
        "--path",
        a_path,
        "--json",
    ]);
    assert_eq!(code(&accepted), 0, "{}", stderr(&accepted));
    assert_eq!(json(&accepted)["entries"][0]["availability"], "available");
    assert_eq!(
        json(&accepted)["entries"][0]["local_path"],
        moved.canonicalize().unwrap().to_string_lossy().as_ref()
    );
}

/// **T12 control 2 at the surface.** The neighbour's database is byte-identical after every command.
#[test]
fn every_repo_command_leaves_the_neighbours_database_byte_identical() {
    let (dir, a, b) = two_repositories();
    let a_path = as_str(&a);
    let moved = dir.path().join("b-moved");
    let target_db = b.join(".nerve/nerve.db");
    let before = std::fs::read(&target_db).expect("the neighbour's database must exist");

    for args in [
        vec!["repo", "add", as_str(&b), "--path", a_path, "--id", "n"],
        vec!["repo", "list", "--path", a_path],
        vec!["repo", "list", "--path", a_path, "--json"],
        // `links` probes the neighbour for its *current* state, which is a second read of somebody
        // else's database and belongs under this assertion for that reason.
        vec!["repo", "links", "--path", a_path],
        vec!["repo", "links", "--path", a_path, "--json"],
    ] {
        assert_eq!(code(&run(&args)), 0, "{args:?}");
    }
    assert_eq!(
        std::fs::read(&target_db).unwrap(),
        before,
        "a registry command changed the neighbour's database"
    );

    // Relocation reads it too, and so does the listing afterwards.
    std::fs::rename(&b, &moved).unwrap();
    let moved_db = moved.join(".nerve/nerve.db");
    assert_eq!(
        std::fs::read(&moved_db).unwrap(),
        before,
        "renaming the directory must not have changed the file"
    );
    for args in [
        vec!["repo", "relocate", "n", as_str(&moved), "--path", a_path],
        vec!["repo", "list", "--path", a_path],
        vec!["repo", "links", "--path", a_path],
        vec!["repo", "remove", "n", "--path", a_path],
        vec!["repo", "list", "--path", a_path],
        vec!["repo", "links", "--path", a_path],
    ] {
        assert_eq!(code(&run(&args)), 0, "{args:?}");
    }
    assert_eq!(
        std::fs::read(&moved_db).unwrap(),
        before,
        "a registry command changed the neighbour's database"
    );

    // Anti-vacuity: the reads really produced answers, so "unchanged" is not "nothing happened".
    let listed = json(&run(&["repo", "list", "--path", a_path, "--json"]));
    assert_eq!(listed["entries"].as_array().unwrap().len(), 1);
}

/// **T12 control 1 at the surface.** A sibling checkout is never registered on its own.
#[test]
fn a_sibling_checkout_is_never_registered_by_itself() {
    let (dir, a, b) = two_repositories();
    let _sibling = repository(dir.path(), "sibling", "ts-resolution", true);
    let a_path = as_str(&a);

    let empty = run(&["repo", "list", "--path", a_path, "--json"]);
    assert_eq!(code(&empty), 0);
    assert!(
        json(&empty)["entries"].as_array().unwrap().is_empty(),
        "a sibling directory was registered without anybody naming it"
    );
    // The human surface says why it is empty rather than printing nothing.
    let text = stdout(&run(&["repo", "list", "--path", a_path]));
    assert!(text.contains("No registered neighbours"), "{text}");
    assert!(text.contains("`nerve repo add` named it"), "{text}");

    // And after registering exactly one, exactly one is listed.
    assert_eq!(
        code(&run(&["repo", "add", as_str(&b), "--path", a_path])),
        0
    );
    let listed = json(&run(&["repo", "list", "--path", a_path, "--json"]));
    assert_eq!(listed["entries"].as_array().unwrap().len(), 1);
}

/// **T12 control 5 at the surface.** A hostile display name cannot forge a line of Nerve's output.
///
/// The name is stored verbatim — that is the store's correctness — and rendered inert, on exactly
/// the terms a commit summary already established.
#[test]
fn a_hostile_display_name_is_rendered_inert_on_both_surfaces() {
    let (_dir, a, b) = two_repositories();
    let a_path = as_str(&a);
    let hostile = "b\u{1b}[2K\n    availability   available";

    let added = run(&[
        "repo",
        "add",
        as_str(&b),
        "--path",
        a_path,
        "--id",
        "n",
        "--name",
        hostile,
    ]);
    assert_eq!(code(&added), 0, "{}", stderr(&added));

    for text in [
        stdout(&added),
        stdout(&run(&["repo", "list", "--path", a_path])),
    ] {
        assert!(
            text.contains("\\u{1b}"),
            "the escape was not made visible: {text:?}"
        );
        assert!(
            !text.contains('\u{1b}'),
            "a raw ESC reached the terminal: {text:?}"
        );
        // The forged *line* would have read as a second `availability` row. Exactly one line
        // begins that way, and it is Nerve's — the payload survives as visible text on the display
        // name's own line, which is the point: it is shown, not obeyed.
        assert_eq!(
            text.lines()
                .filter(|line| line.starts_with("    availability   "))
                .count(),
            1,
            "the display name forged a line of output: {text}"
        );
        assert!(
            text.lines()
                .any(|line| line.contains("\\u{1b}[2K\\u{0a}    availability")),
            "the payload must still be visible, as text, on the name's own line: {text}"
        );
    }

    // The same escaping in `--json`, because emitting the raw byte would hand the problem to
    // whatever reads the JSON next.
    let listed = json(&run(&["repo", "list", "--path", a_path, "--json"]));
    let name = listed["entries"][0]["display_name"].as_str().unwrap();
    assert!(name.contains("\\u{1b}"), "{name:?}");
    assert!(!name.contains('\u{1b}') && !name.contains('\n'), "{name:?}");

    // And the store kept the original, because neutralising it there would make the database
    // disagree with the disk.
    let conn = nerve_store::open(&a.join(".nerve/nerve.db")).unwrap();
    let repo_id = nerve_store::repository(&conn).unwrap().unwrap().repo_id;
    let stored = nerve_store::list_registry_entries(&conn, &repo_id).unwrap();
    assert_eq!(stored[0].display_name, hostile);
}

/// A repository with no index answers with the "no index" code, not with an empty registry.
#[test]
fn the_registry_commands_need_an_initialised_repository() {
    let dir = tempfile::tempdir().unwrap();
    let bare = dir.path().join("bare");
    std::fs::create_dir_all(&bare).unwrap();
    let bare = as_str(&bare);
    assert_eq!(code(&run(&["repo", "list", "--path", bare])), 2);
    assert_eq!(code(&run(&["repo", "add", bare, "--path", bare])), 2);
    assert_eq!(code(&run(&["repo", "remove", "x", "--path", bare])), 2);
}

/// A neighbour that was initialised and never indexed is *unknown*, not *fine*.
#[test]
fn a_never_indexed_neighbour_is_reported_as_partially_indexed() {
    let dir = tempfile::tempdir().unwrap();
    let a = repository(dir.path(), "a", "ts-basic", true);
    let b = repository(dir.path(), "b", "ts-resolution", false);
    let a_path = as_str(&a);

    let added = run(&["repo", "add", as_str(&b), "--path", a_path, "--json"]);
    assert_eq!(code(&added), 0, "{}", stderr(&added));
    let entry = &json(&added)["entries"][0];
    assert_eq!(entry["availability"], "partially_indexed");
    assert_eq!(entry["freshness"], "target_partially_indexed");
    // Looked at, and nothing was there to see. The two are recorded as different facts.
    assert!(entry["availability_checked_at"].is_string());
    assert_eq!(entry["last_seen_state"], serde_json::Value::Null);
}

/// A registered path that no longer exists, and one that now holds another repository, stay apart.
#[test]
fn a_missing_neighbour_and_a_swapped_one_are_two_different_answers() {
    let (dir, a, b) = two_repositories();
    let gone = repository(dir.path(), "gone", "ts-resolution", true);
    let a_path = as_str(&a);
    assert_eq!(
        code(&run(&[
            "repo",
            "add",
            as_str(&b),
            "--path",
            a_path,
            "--id",
            "swapped"
        ])),
        0
    );
    assert_eq!(
        code(&run(&[
            "repo",
            "add",
            as_str(&gone),
            "--path",
            a_path,
            "--id",
            "gone"
        ])),
        0
    );

    std::fs::remove_dir_all(&gone).unwrap();
    std::fs::remove_dir_all(&b).unwrap();
    repository(dir.path(), "b", "ts-basic", true);

    let listed = json(&run(&["repo", "list", "--path", a_path, "--json"]));
    let by_id = |id: &str| {
        listed["entries"]
            .as_array()
            .unwrap()
            .iter()
            .find(|entry| entry["registry_id"] == id)
            .unwrap_or_else(|| panic!("{id} must be listed"))
            .clone()
    };
    let gone_entry = by_id("gone");
    let swapped_entry = by_id("swapped");

    assert_eq!(gone_entry["availability"], "missing");
    assert_eq!(gone_entry["freshness"], "target_repository_missing");
    assert_eq!(gone_entry["refusal"], "path_does_not_exist");

    assert_eq!(swapped_entry["availability"], "moved");
    assert_eq!(swapped_entry["freshness"], "target_repository_moved");
    assert!(swapped_entry["observed_repository_id"].is_string());
    assert_ne!(
        swapped_entry["observed_repository_id"], swapped_entry["expected_repository_id"],
        "a swapped checkout must be reported against the recorded identity"
    );

    assert_ne!(gone_entry["freshness"], swapped_entry["freshness"]);
}

// ---- `nerve repo links` (Slice 13d-ii) ---------------------------------------------------------
//
// The command reads stored links back with the standing `nerve_index::contract_report` decided for
// them, which is the call `/api/contracts` and the `nerve_contracts` MCP tool make. What is asserted
// here is what this surface owns: that the verdict reaches the answer, that the two ways of having
// no links stay apart, that the bound is a fact, and that repository text arrives inert. That the
// verdict *agrees* with HTTP's for the same link needs a live server, and is asserted in
// `scripts/final_acceptance.sh`.

/// A `host` with `pkg-map` registered and scanned.
///
/// `contracts-exports` rather than `contracts-npm`, because it is the only fixture producing a C2
/// link: an import specifier resolved through the neighbour's own export map to a file inside it.
/// That is the only link carrying a target snapshot, and a snapshot is what a freshness verdict is
/// read against.
fn a_scanned_pair(display_name: Option<&str>) -> (tempfile::TempDir, PathBuf, PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    let host = repository(dir.path(), "host", "contracts-exports/host", true);
    let neighbour = repository(dir.path(), "pkg-map", "contracts-exports/pkg-map", true);
    let mut args = vec![
        "repo",
        "add",
        as_str(&neighbour),
        "--path",
        as_str(&host),
        "--id",
        "pkg-map",
    ];
    if let Some(name) = display_name {
        args.push("--name");
        args.push(name);
    }
    let added = run(&args);
    assert_eq!(code(&added), 0, "{}", stderr(&added));
    let scanned = run(&["repo", "scan", "--path", as_str(&host)]);
    assert_eq!(code(&scanned), 0, "{}", stderr(&scanned));
    (dir, host, neighbour)
}

/// One link out of an answer, by the identity its manifest declared.
fn link_named(answer: &serde_json::Value, identity: &str) -> serde_json::Value {
    answer["links"]
        .as_array()
        .unwrap_or_else(|| panic!("the answer must carry a link list: {answer}"))
        .iter()
        .find(|link| link["contract_identity"] == identity)
        .unwrap_or_else(|| panic!("{identity} must be listed: {answer}"))
        .clone()
}

/// The verdict reaches the command, and it follows the neighbour rather than the last scan.
///
/// The second half is the whole point of the slice. `repo scan` reports what a scan wrote, so a
/// repository whose neighbour moved on *after* the scan has nothing in the scan to report it with;
/// this command re-decides against the world as it is now, with no second scan and no write.
#[test]
fn a_stored_link_is_read_back_with_its_freshness_and_the_verdict_follows_the_neighbour() {
    let (_dir, host, neighbour) = a_scanned_pair(None);
    let host_path = as_str(&host);

    let answer = json(&run(&[
        "repo", "links", "--path", host_path, "--limit", "500", "--json",
    ]));
    assert_eq!(answer["result_kind"], "contract_links");
    assert!(answer["links_total"].as_u64().unwrap() >= 2, "{answer}");
    assert_eq!(answer["links_without_registry_entry"], 0);

    let resolved = link_named(&answer, "pkg-map/sub");
    // Every column a consumer needs to tell current from moved, on the answer rather than in a
    // document. These are `/api/contracts`'s keys, so the same row read two ways is one row.
    for key in [
        "link_id",
        "relation_semantics",
        "contract_kind",
        "contract_identity",
        "resolution_method",
        "resolution_method_note",
        "expected_contract_version",
        "observed_contract_version",
        "source_path",
        "source_span",
        "source_manifest_present",
        "target_state_at_resolution",
        "target_current_state",
        "target_entity_id",
        "target_path_snapshot",
        "ambiguity",
        "unsupported_reason",
        "status",
        "status_note",
        "freshness",
        "freshness_note",
        "is_current",
        "registry_entry",
    ] {
        assert!(
            resolved.get(key).is_some(),
            "the answer must carry {key}: {resolved}"
        );
    }
    assert_eq!(resolved["freshness"], serde_json::Value::Null);
    assert_eq!(resolved["is_current"], true);
    assert_eq!(resolved["resolution_method"], "export_map_resolved");
    assert_eq!(resolved["relation_semantics"], "REFERENCES");
    assert_eq!(resolved["target_path_snapshot"], "src/sub.ts");
    assert_eq!(resolved["registry_entry"]["availability"], "available");
    assert_eq!(resolved["status"], "active");

    // A file the neighbour has and has never indexed is *unknown*, and it is neither stale nor
    // missing. Reporting it as either would be a claim the evidence does not carry.
    let unindexed = link_named(&answer, "pkg-map/data");
    assert_eq!(unindexed["freshness"], "target_partially_indexed");
    assert_eq!(unindexed["is_current"], false);
    assert_eq!(unindexed["target_entity_id"], serde_json::Value::Null);
    assert_ne!(unindexed["freshness"], resolved["freshness"]);

    // The neighbour moves on. Nothing is scanned again, and nothing here writes.
    std::fs::write(
        neighbour.join("src/added.ts"),
        "export function added(): number { return 3; }\n",
    )
    .unwrap();
    assert_eq!(code(&run(&["index", as_str(&neighbour)])), 0);

    let after = json(&run(&[
        "repo", "links", "--path", host_path, "--limit", "500", "--json",
    ]));
    let moved = link_named(&after, "pkg-map/sub");
    assert_eq!(moved["freshness"], "target_changed");
    assert_eq!(moved["is_current"], false);
    assert!(
        moved["freshness_note"].as_str().unwrap().len() > 20,
        "{moved}"
    );
    assert_ne!(
        moved["target_state_at_resolution"],
        moved["target_current_state"]
    );
    // And the human answer says it too, rather than only the machine one.
    let human = stdout(&run(&[
        "repo", "links", "--path", host_path, "--limit", "500",
    ]));
    assert!(human.contains("target_changed"), "{human}");
}

/// Two absences, and neither may be read as the other.
///
/// Nothing is ever discovered here, so "no neighbour was ever named" and "neighbours were named and
/// nothing declares a path into one" have different next steps. An answer that let one stand for the
/// other would report *we never looked* as *we looked and there is nothing*.
#[test]
fn no_neighbour_and_no_declaration_are_two_different_absences() {
    let (_dir, a, b) = two_repositories();
    let a_path = as_str(&a);

    let unregistered = json(&run(&["repo", "links", "--path", a_path, "--json"]));
    let unregistered_human = stdout(&run(&["repo", "links", "--path", a_path]));
    assert_eq!(unregistered["result_kind"], "no_registered_neighbours");
    assert_eq!(unregistered["registry_entries_total"], 0);
    assert_eq!(unregistered["links_total"], 0);
    assert_eq!(unregistered["links"], serde_json::json!([]));

    assert_eq!(
        code(&run(&[
            "repo",
            "add",
            as_str(&b),
            "--path",
            a_path,
            "--id",
            "n"
        ])),
        0
    );
    let declared = json(&run(&["repo", "links", "--path", a_path, "--json"]));
    let declared_human = stdout(&run(&["repo", "links", "--path", a_path]));
    assert_eq!(declared["result_kind"], "no_contract_links");
    assert_eq!(declared["registry_entries_total"], 1);
    assert_eq!(declared["links_total"], 0);

    assert_ne!(unregistered["result_kind"], declared["result_kind"]);
    assert_ne!(
        unregistered_human, declared_human,
        "the two absences print the same words, so a reader cannot tell them apart"
    );
    // The second one names the command that would change the answer; the first cannot, because
    // scanning a repository with no registered neighbour would find nothing to resolve to.
    assert!(
        declared_human.contains("nerve repo scan"),
        "{declared_human}"
    );
    assert!(
        unregistered_human.contains("nerve repo add"),
        "{unregistered_human}"
    );
}

/// The list is bounded, and `truncated` is a comparison rather than the guess `returned == limit`.
#[test]
fn the_link_list_is_bounded_and_truncation_is_a_fact() {
    let (_dir, host, _neighbour) = a_scanned_pair(None);
    let host_path = as_str(&host);

    let all = json(&run(&[
        "repo", "links", "--path", host_path, "--limit", "500", "--json",
    ]));
    let total = all["links_total"].as_u64().unwrap();
    assert!(total >= 2, "the fixture must produce links to bound: {all}");
    assert_eq!(all["truncated"], false);
    assert_eq!(all["returned"], total);

    let cut = json(&run(&[
        "repo", "links", "--path", host_path, "--limit", "1", "--json",
    ]));
    assert_eq!(cut["returned"], 1);
    assert_eq!(cut["links_total"], total, "the total survives the cut");
    assert_eq!(cut["truncated"], true);
    assert_eq!(cut["links"].as_array().unwrap().len(), 1);
    assert!(stdout(&run(&[
        "repo", "links", "--path", host_path, "--limit", "1"
    ]))
    .contains("more beyond the limit"));

    // The case `returned == limit` gets wrong: a page ending exactly on the total is not a cut.
    let exact = json(&run(&[
        "repo",
        "links",
        "--path",
        host_path,
        "--limit",
        &total.to_string(),
        "--json",
    ]));
    assert_eq!(exact["returned"], total);
    assert_eq!(exact["truncated"], false);

    assert_eq!(
        code(&run(&[
            "repo", "links", "--path", host_path, "--limit", "0"
        ])),
        10,
        "a limit of zero is a usage error, not an empty answer"
    );
}

/// Repository-derived text reaches the answer, and reaches it inert.
///
/// A neighbour is a checkout that may have been cloned from anywhere, so its display name is
/// untrusted content. A raw control character in it would forge a line of Nerve's own output.
#[test]
fn repository_text_reaches_the_link_answer_inert() {
    let (_dir, host, _neighbour) = a_scanned_pair(Some("first\u{7}second"));
    let host_path = as_str(&host);

    let answer = run(&[
        "repo", "links", "--path", host_path, "--limit", "500", "--json",
    ]);
    let raw = stdout(&answer);
    let parsed: serde_json::Value = serde_json::from_str(&raw).unwrap();
    let link = link_named(&parsed, "pkg-map/sub");
    // Anti-vacuity: the hostile name really reached the answer, or the scan below proves nothing.
    assert_eq!(
        link["registry_entry"]["display_name"], "first\\u{07}second",
        "{link}"
    );
    assert!(
        !raw.contains('\u{7}'),
        "a raw control character reached the JSON answer"
    );
    assert!(
        !stdout(&run(&[
            "repo", "links", "--path", host_path, "--limit", "500"
        ]))
        .contains('\u{7}'),
        "a raw control character reached the human answer"
    );
}

/// Every `nerve repo` verb the CLI has is named by the boundary every contract answer carries.
///
/// The list is what a reader who cannot press a button is given instead, so a verb missing from it
/// is a capability that exists and is not discoverable from the surfaces that cannot perform it.
/// Checked in **both** directions: a name in the list with no verb behind it is a promise the
/// product does not keep.
#[test]
fn the_boundary_names_every_repo_verb_and_only_verbs_that_exist() {
    let help = stdout(&run(&["repo", "--help"]));
    let mut verbs: Vec<String> = help
        .lines()
        .skip_while(|line| !line.starts_with("Commands:"))
        .skip(1)
        .take_while(|line| !line.trim().is_empty())
        .filter_map(|line| line.split_whitespace().next())
        .filter(|name| *name != "help")
        .map(str::to_string)
        .collect();
    verbs.sort();
    // Anti-vacuity: the help really was parsed, and reached the verb this slice adds.
    assert!(verbs.len() >= 6, "parsed only {verbs:?} from {help}");
    assert!(verbs.contains(&"links".to_string()), "{verbs:?}");

    let mut named: Vec<String> = nerve_server::api::contracts::BOUNDARY_COMMANDS
        .iter()
        .map(|command| {
            command
                .strip_prefix("nerve repo ")
                .unwrap_or_else(|| panic!("{command} is not a `nerve repo` verb"))
                .split_whitespace()
                .next()
                .unwrap()
                .to_string()
        })
        .collect();
    named.sort();
    assert_eq!(
        verbs, named,
        "the CLI's repo verbs and the boundary's command list have drifted"
    );

    // And each named verb answers rather than merely existing.
    let dir = tempfile::tempdir().unwrap();
    let root = repository(dir.path(), "solo", "ts-basic", true);
    for verb in ["list", "links"] {
        let output = run(&["repo", verb, "--path", as_str(&root), "--json"]);
        assert_eq!(code(&output), 0, "repo {verb}: {}", stderr(&output));
        assert_eq!(json(&output)["ok"], true);
    }
}
