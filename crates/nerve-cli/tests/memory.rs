//! `nerve memory` at the command line (Slice 14b-ii).
//!
//! The surface's own job is narrow — parse, render, and map an outcome to an exit code — so what is
//! asserted here is narrow too. The lifecycle itself is asserted at the layer that owns it, in
//! `crates/nerve-store/tests/memory.rs`; what this file pins is everything that would still be
//! wrong if that layer were perfect:
//!
//! - **A refusal is not an empty result.** `--scope opertions` exits `10` naming the four admitted
//!   values, rather than exiting `0` with no records — which would read as *"there are no notes"*
//!   when what is true is *"there is no such scope"*. Both halves are checked, because a command
//!   that refused everything would pass the first on its own.
//! - **A read writes nothing.** The database is hashed around every read verb. `PRAGMA query_only`
//!   is what makes that true by construction, and this is what would notice if a read verb were
//!   ever opened writable by mistake.
//! - **The two kinds stay apart.** `status` is stored and `views` are derived, they appear under
//!   different keys, and no derived value is ever printed as though a column held it.
//! - **The export is exact.** Byte-identical twice, no timestamp of its own, no derived field, no
//!   absolute path. It is the only artefact here a human could lose their notes to.
//! - **There is no delete verb**, asserted rather than left to discipline.
//!
//! Every repository is `nerve init`-ed and indexed in its own directory: a memory record needs a
//! repository state to anchor to, and a repository nothing has indexed is a case with its own test.

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

/// A copy of `ts-basic`, `nerve init`-ed and — unless `index` is false — indexed.
fn repository(index: bool) -> (tempfile::TempDir, PathBuf) {
    let source = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures/ts-basic")
        .canonicalize()
        .unwrap();
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("repo");
    copy_tree(&source, &root);
    let path = root.to_str().unwrap();
    assert_eq!(code(&run(&["init", path])), 0, "init");
    if index {
        assert_eq!(code(&run(&["index", path])), 0, "index");
    }
    (dir, root)
}

fn as_str(path: &Path) -> &str {
    path.to_str().unwrap()
}

/// The whole database file hashed, so *"a read wrote nothing"* is a fact about bytes rather than
/// about rows. The same hash `cli.rs` uses for the identical proof, and no new dependency.
fn database_digest(root: &Path) -> String {
    let bytes = std::fs::read(root.join(".nerve/nerve.db")).expect("the database must be readable");
    nerve_core::ids::content_hash(&bytes)
}

/// Write one note about `src/math.ts` and return the command's output.
fn propose(root: &Path, id: &str, content: &str, claim_key: Option<&str>) -> Output {
    let mut args = vec![
        "memory",
        "propose",
        "--subject",
        "file:src/math.ts",
        "--scope",
        "implementation",
        "--content",
        content,
        "--id",
        id,
        "--path",
        as_str(root),
    ];
    if let Some(key) = claim_key {
        args.push("--claim-key");
        args.push(key);
    }
    run(&args)
}

/// The record with this id, out of any command's `--json` answer.
fn record<'a>(answer: &'a serde_json::Value, memory_id: &str) -> &'a serde_json::Value {
    answer["records"]
        .as_array()
        .unwrap_or_else(|| panic!("no records array in {answer}"))
        .iter()
        .find(|record| record["memory_id"] == memory_id)
        .unwrap_or_else(|| panic!("no record {memory_id} in {answer}"))
}

// ---- the lifecycle, through the surface --------------------------------------------------------

/// **Every verb's happy path, and the property the row is about: nothing earlier is lost.**
///
/// The record is proposed, confirmed, cited, superseded and — through its successor — invalidated,
/// and at the end the *first* record still carries all four of its events and its original content.
/// Superseding rewrites nothing and deletes nothing, so what was once believed stays readable.
#[test]
fn the_whole_lifecycle_runs_and_every_earlier_event_survives() {
    let (_dir, root) = repository(true);
    let root = root.as_path();

    let proposed = propose(root, "m1", "the add helper is the only export", Some("api"));
    assert_eq!(code(&proposed), 0, "{}", stderr(&proposed));
    assert!(
        stdout(&proposed).contains("nerve memory confirm m1"),
        "a proposal must print the command that settles it: {}",
        stdout(&proposed)
    );

    for (verb, extra) in [
        ("confirm", vec!["--note", "checked against the tests"]),
        ("cite", vec!["--file", "src/math.ts", "--span", "1:12"]),
    ] {
        let mut args = vec!["memory", verb, "m1"];
        args.extend(extra);
        args.extend(["--path", as_str(root)]);
        let output = run(&args);
        assert_eq!(code(&output), 0, "memory {verb}: {}", stderr(&output));
    }

    let superseded = run(&[
        "memory",
        "supersede",
        "m1",
        "--content",
        "add and mul are both exported now",
        "--id",
        "m2",
        "--note",
        "the module grew",
        "--path",
        as_str(root),
        "--json",
    ]);
    assert_eq!(code(&superseded), 0, "{}", stderr(&superseded));
    let answer = json(&superseded);
    // Both records are reported, in the order the change happened.
    let old = record(&answer, "m1");
    let new = record(&answer, "m2");
    assert_eq!(old["status"], "superseded");
    assert_eq!(
        old["content"], "the add helper is the only export",
        "superseding rewrote the predecessor's content"
    );
    assert_eq!(
        old["superseded_by_memory_id"], "m2",
        "the inverse of supersession is derived and must be reported"
    );
    assert_eq!(old["superseded_by_is_derived"], true);
    assert_eq!(new["supersedes_memory_id"], "m1");
    assert_eq!(
        new["status"], "proposed",
        "`active` is reachable only through confirm"
    );
    assert_eq!(
        new["scope"], old["scope"],
        "a successor inherits the facet it replaces"
    );
    assert_eq!(new["claim_key"], old["claim_key"]);
    assert_eq!(
        new["subject"], old["subject"],
        "a replacement about a different subject would not be a replacement"
    );

    // Every one of the predecessor's events is still there, in order, including the two that
    // preceded its retirement.
    let events = run(&["memory", "events", "m1", "--path", as_str(root), "--json"]);
    assert_eq!(code(&events), 0, "{}", stderr(&events));
    let listed = record(&json(&events), "m1")["events"].clone();
    let operations: Vec<&str> = listed
        .as_array()
        .unwrap()
        .iter()
        .map(|event| event["operation"].as_str().unwrap())
        .collect();
    assert_eq!(
        operations,
        ["proposed", "confirmed", "cited", "superseded"],
        "an earlier event was lost: {listed}"
    );
    // The citation changed no status, and its event says so on both sides.
    let cited = &listed.as_array().unwrap()[2];
    assert_eq!(cited["from_status"], cited["to_status"]);
    assert_eq!(cited["changes_status"], false);

    // Invalidation is a different fact from supersession, and both are reachable.
    let invalidated = run(&[
        "memory",
        "invalidate",
        "m2",
        "--reason",
        "the module was deleted",
        "--path",
        as_str(root),
        "--json",
    ]);
    assert_eq!(code(&invalidated), 0, "{}", stderr(&invalidated));
    let after_invalidation = json(&invalidated);
    let ended = record(&after_invalidation, "m2");
    assert_eq!(ended["status"], "invalidated");
    assert_eq!(ended["invalidation_reason"], "the module was deleted");
    assert!(
        ended["invalidated_at"].is_string(),
        "an ending is a status and a moment"
    );
    assert!(
        ended["superseded_by_memory_id"].is_null(),
        "nothing replaced it — that is what invalidated means"
    );

    // And the two retirements stay distinguishable in the listing.
    let listed = json(&run(&["memory", "list", "--path", as_str(root), "--json"]));
    assert_eq!(record(&listed, "m1")["status"], "superseded");
    assert_eq!(record(&listed, "m2")["status"], "invalidated");
}

/// A record created by `supersede` can say that it was created.
///
/// **This test previously asserted the opposite, and the defect it documented is now fixed.**
/// 14b-ii found that `supersede_memory` reached the raw `insert_memory`, so a successor arrived
/// with an *empty* audit history while an otherwise identical record born by `propose` arrived
/// with a `proposed` event — `nerve memory events <successor>` printed nothing, and the record
/// could not say it had ever been written. §4's *"every mutating lifecycle operation appends a
/// typed event"* carves out no exception for the operation that creates a record, so the store
/// now appends the successor's creating event inside the same transaction. Two records change in
/// a supersession, so two events are recorded, and reading either record's history tells the
/// whole of what happened to it.
#[test]
fn a_record_created_by_supersede_reports_its_own_creation() {
    let (_dir, root) = repository(true);
    let root = root.as_path();
    assert_eq!(code(&propose(root, "m1", "the first note", None)), 0);
    assert_eq!(
        code(&run(&[
            "memory",
            "supersede",
            "m1",
            "--content",
            "the second note",
            "--id",
            "m2",
            "--path",
            as_str(root),
        ])),
        0
    );

    let events = run(&["memory", "events", "m2", "--path", as_str(root)]);
    assert_eq!(code(&events), 0, "{}", stderr(&events));
    let text = stdout(&events);
    assert!(
        text.contains("1 event(s) for m2"),
        "a superseding record must record its own creation: {text}"
    );
    assert!(text.contains("superseded"), "{text}");

    // And the predecessor keeps its own retirement event — the successor's creating event is an
    // addition, not a relabelling of the one that was already there.
    let predecessor = run(&["memory", "events", "m1", "--path", as_str(root)]);
    assert_eq!(code(&predecessor), 0, "{}", stderr(&predecessor));
    let text = stdout(&predecessor);
    assert!(
        text.contains("2 event(s) for m1"),
        "the predecessor keeps its proposal and its retirement: {text}"
    );
}

// ---- refusals ----------------------------------------------------------------------------------

/// **An unknown scope or status is refused, and a known one that matches nothing is not.**
///
/// The second half is what makes the first mean anything: a surface that refused every value would
/// satisfy the refusal assertion on its own, and would have hidden the real behaviour — that an
/// empty answer to a *legal* question is a legitimate zero.
#[test]
fn an_unknown_scope_or_status_is_refused_rather_than_answered_with_nothing() {
    let (_dir, root) = repository(true);
    let root = root.as_path();
    assert_eq!(code(&propose(root, "m1", "a note", None)), 0);

    for (flag, value, admitted) in [
        (
            "--scope",
            "opertions",
            "implementation, interface, operations, process",
        ),
        (
            "--status",
            "potentially_stale",
            "proposed, active, superseded, invalidated",
        ),
    ] {
        let refused = run(&[
            "memory",
            "list",
            flag,
            value,
            "--path",
            as_str(root),
            "--json",
        ]);
        assert_eq!(
            code(&refused),
            10,
            "{flag} {value} was not refused: {}",
            stdout(&refused)
        );
        let answer = json(&refused);
        assert_eq!(answer["ok"], false);
        let message = answer["error"].as_str().unwrap();
        assert!(
            message.contains(admitted),
            "the refusal must name the admitted set: {message}"
        );
        assert!(
            answer.get("records").is_none(),
            "a refusal must not carry a result: {answer}"
        );
    }

    // A derived view is named as such rather than merely being missing from the list.
    let refused = run(&[
        "memory",
        "list",
        "--status",
        "potentially_stale",
        "--path",
        as_str(root),
    ]);
    assert!(
        stderr(&refused).contains("derived at read time"),
        "{}",
        stderr(&refused)
    );

    // Anti-vacuity: a legal scope holding no record is a zero, not a refusal.
    let empty = run(&[
        "memory",
        "list",
        "--scope",
        "interface",
        "--path",
        as_str(root),
        "--json",
    ]);
    assert_eq!(code(&empty), 0, "{}", stderr(&empty));
    assert_eq!(json(&empty)["count"], 0);
}

/// **A note anchored to nothing is refused.**
///
/// Staleness is derived from the anchor at read time, so a record written against a repository
/// nothing has indexed could never be qualified. The refusal names the remedy, and nothing is
/// written — asserted on the bytes, because "refused" and "written and then reported as refused"
/// look identical from the exit code alone.
#[test]
fn proposing_into_an_unindexed_repository_is_refused_and_writes_nothing() {
    let (_dir, root) = repository(false);
    let root = root.as_path();
    let before = database_digest(root);

    let refused = run(&[
        "memory",
        "propose",
        "--subject",
        "file:src/math.ts",
        "--scope",
        "process",
        "--content",
        "a note nobody can date",
        "--path",
        as_str(root),
        "--json",
    ]);
    assert_eq!(code(&refused), 2, "{}", stdout(&refused));
    let answer = json(&refused);
    assert_eq!(answer["ok"], false);
    assert_eq!(answer["reason"], "repository_not_indexed");
    assert!(answer["anchor_state_id"].is_null());
    assert_eq!(
        database_digest(root),
        before,
        "a refused proposal wrote to the database"
    );
}

/// The store's own refusals reach the caller with the reason it gave, and at exit `10`.
///
/// The two directions are the pair that keeps `superseded` and `invalidated` from collapsing into
/// each other, so the messages are checked rather than only the codes: a refusal that said only
/// "not allowed" would send the reader back to the database to find out which fact they had hit.
#[test]
fn a_refused_transition_carries_the_stores_reason() {
    let (_dir, root) = repository(true);
    let root = root.as_path();

    assert_eq!(code(&propose(root, "m1", "the first note", None)), 0);
    assert_eq!(
        code(&run(&[
            "memory",
            "invalidate",
            "m1",
            "--path",
            as_str(root)
        ])),
        0
    );

    // Invalidated, so nothing replaced it: superseding it now would turn one claim into the other.
    let refused = run(&[
        "memory",
        "supersede",
        "m1",
        "--content",
        "a successor that must not exist",
        "--id",
        "m2",
        "--path",
        as_str(root),
    ]);
    assert_eq!(code(&refused), 10, "{}", stdout(&refused));
    assert!(
        stderr(&refused).contains("was invalidated"),
        "{}",
        stderr(&refused)
    );
    assert!(
        !stdout(&run(&["memory", "list", "--path", as_str(root)])).contains("m2"),
        "the refused successor was written anyway"
    );

    // The mirror: a superseded record may not then be said to have ended with no successor.
    assert_eq!(code(&propose(root, "m3", "another note", None)), 0);
    assert_eq!(
        code(&run(&[
            "memory",
            "supersede",
            "m3",
            "--content",
            "its replacement",
            "--id",
            "m4",
            "--path",
            as_str(root),
        ])),
        0
    );
    let refused = run(&["memory", "invalidate", "m3", "--path", as_str(root)]);
    assert_eq!(code(&refused), 10, "{}", stdout(&refused));
    assert!(
        stderr(&refused).contains("superseded"),
        "{}",
        stderr(&refused)
    );

    // Confirming twice is refused with the status the record is actually in, not a bare "no".
    assert_eq!(
        code(&run(&["memory", "confirm", "m4", "--path", as_str(root)])),
        0
    );
    let refused = run(&["memory", "confirm", "m4", "--path", as_str(root)]);
    assert_eq!(code(&refused), 10);
    assert!(
        stderr(&refused).contains("is active"),
        "{}",
        stderr(&refused)
    );

    // And an id that is not here is a refusal rather than a silent no-op.
    let missing = run(&["memory", "confirm", "nope", "--path", as_str(root)]);
    assert_eq!(code(&missing), 10, "{}", stdout(&missing));
    assert!(stderr(&missing).contains("no memory record"));
}

/// A citation names a place inside this repository, or it is refused by name.
#[test]
fn a_citation_outside_the_repository_or_with_a_bad_span_is_refused() {
    let (_dir, root) = repository(true);
    let root = root.as_path();
    assert_eq!(code(&propose(root, "m1", "a note", None)), 0);

    for (file, span, expected) in [
        ("/etc/passwd", None, "repository-relative"),
        ("../outside.ts", None, "outside the repository"),
        ("src/math.ts", Some("12"), "not START:END"),
        ("src/math.ts", Some("12:3"), "does not name a range"),
    ] {
        let mut args = vec!["memory", "cite", "m1", "--file", file];
        if let Some(span) = span {
            args.extend(["--span", span]);
        }
        args.extend(["--path", as_str(root)]);
        let refused = run(&args);
        assert_eq!(code(&refused), 10, "{file} {span:?}: {}", stdout(&refused));
        assert!(
            stderr(&refused).contains(expected),
            "{file} {span:?}: {}",
            stderr(&refused)
        );
    }

    // Anti-vacuity: the legal form is accepted, so the loop above is not refusing everything.
    assert_eq!(
        code(&run(&[
            "memory",
            "cite",
            "m1",
            "--file",
            "src/math.ts",
            "--span",
            "3:9",
            "--path",
            as_str(root),
        ])),
        0
    );
}

/// **There is no `nerve memory delete`.**
///
/// A delete verb is how *"history preserved"* stops being true, so its absence is asserted rather
/// than left to discipline — the same standing `nerve affected` and `nerve trace-tests` are held
/// to. Both halves: the verb is refused when typed, and it is not offered in the help.
#[test]
fn there_is_no_delete_verb() {
    let (_dir, root) = repository(true);
    let root = root.as_path();
    assert_eq!(code(&propose(root, "m1", "a note", None)), 0);

    for verb in ["delete", "remove", "purge", "forget"] {
        let refused = run(&["memory", verb, "m1", "--path", as_str(root)]);
        assert_eq!(
            code(&refused),
            10,
            "`nerve memory {verb}` exists: {}",
            stdout(&refused)
        );
        assert!(
            stderr(&refused).contains("unrecognized subcommand"),
            "`nerve memory {verb}` was understood: {}",
            stderr(&refused)
        );
    }

    let help = run(&["memory", "--help"]);
    assert_eq!(code(&help), 0);
    let verbs: Vec<String> = stdout(&help)
        .lines()
        .filter_map(|line| line.strip_prefix("  "))
        .filter(|line| !line.starts_with(' ') && !line.starts_with('-'))
        .filter_map(|line| line.split_whitespace().next().map(str::to_string))
        .collect();
    assert!(
        verbs.contains(&"invalidate".to_string()),
        "the help was not read: {verbs:?}"
    );
    for forbidden in ["delete", "remove", "purge", "forget"] {
        assert!(
            !verbs.contains(&forbidden.to_string()),
            "`{forbidden}` is offered as a memory verb: {verbs:?}"
        );
    }

    // The record is still here, which is the point of all of the above.
    assert_eq!(
        code(&run(&["memory", "show", "m1", "--path", as_str(root)])),
        0
    );
}

// ---- reading -----------------------------------------------------------------------------------

/// **Every read verb leaves the database byte-identical.**
///
/// `PRAGMA query_only` is what makes this true by construction. The hash is taken around all five
/// readers at once, and the answers are checked afterwards so that "unchanged" is not "nothing
/// ran".
#[test]
fn every_read_leaves_the_database_byte_identical() {
    let (_dir, root) = repository(true);
    let root = root.as_path();
    assert_eq!(code(&propose(root, "m1", "a note worth reading", None)), 0);
    assert_eq!(
        code(&run(&["memory", "confirm", "m1", "--path", as_str(root)])),
        0
    );

    let before = database_digest(root);
    let reads = [
        vec!["memory", "list", "--path", as_str(root)],
        vec!["memory", "show", "m1", "--path", as_str(root)],
        vec!["memory", "search", "worth reading", "--path", as_str(root)],
        vec!["memory", "events", "m1", "--path", as_str(root)],
        vec!["memory", "export", "--path", as_str(root)],
    ];
    for read in &reads {
        let output = run(read);
        assert_eq!(code(&output), 0, "{read:?}: {}", stderr(&output));
        assert!(
            !stdout(&output).is_empty(),
            "{read:?} answered nothing, so the hash below proves nothing"
        );
    }
    assert_eq!(
        database_digest(root),
        before,
        "a read command wrote to the database"
    );
}

/// The human and the JSON renderings carry the same facts, and neither prints a derived value as a
/// stored one.
#[test]
fn the_human_and_json_readings_agree() {
    let (_dir, root) = repository(true);
    let root = root.as_path();
    assert_eq!(
        code(&propose(
            root,
            "m1",
            "the retry budget is three",
            Some("retry-policy")
        )),
        0
    );
    assert_eq!(
        code(&run(&["memory", "confirm", "m1", "--path", as_str(root)])),
        0
    );
    assert_eq!(
        code(&run(&[
            "memory",
            "cite",
            "m1",
            "--file",
            "src/math.ts",
            "--path",
            as_str(root)
        ])),
        0
    );

    let human = stdout(&run(&["memory", "show", "m1", "--path", as_str(root)]));
    let answer = json(&run(&[
        "memory",
        "show",
        "m1",
        "--path",
        as_str(root),
        "--json",
    ]));
    let shown = record(&answer, "m1");

    // Every field the row plan requires a read to expose, present in both renderings.
    for key in [
        "memory_id",
        "status",
        "status_note",
        "views",
        "subject",
        "subject_resolution",
        "subject_resolution_note",
        "scope",
        "scope_note",
        "claim_key",
        "anchor_state_id",
        "current_state_id",
        "content",
        "author_label",
        "created_at",
        "supersedes_memory_id",
        "superseded_by_memory_id",
        "invalidated_at",
        "invalidation_reason",
        "citations",
        "events",
    ] {
        assert!(shown.get(key).is_some(), "{key} is missing from {shown}");
    }
    for text in [
        shown["memory_id"].as_str().unwrap(),
        shown["status"].as_str().unwrap(),
        shown["scope"].as_str().unwrap(),
        shown["claim_key"].as_str().unwrap(),
        shown["content"].as_str().unwrap(),
        shown["author_label"].as_str().unwrap(),
        shown["anchor_state_id"].as_str().unwrap(),
        shown["subject_resolution"].as_str().unwrap(),
        shown["subject"]["path"].as_str().unwrap(),
        shown["subject"]["selector"].as_str().unwrap(),
        shown["citations"][0]["cited_path"].as_str().unwrap(),
        shown["events"][0]["operation"].as_str().unwrap(),
    ] {
        assert!(
            human.contains(text),
            "the human reading omits {text:?}:\n{human}"
        );
    }
    // The author is displayed with what it is, because a field named `author` in a product with no
    // accounts invites being read as authentication.
    assert!(human.contains("a local label, not an identity"), "{human}");
    assert_eq!(shown["author_label_is_an_identity"], false);
    // A derived view is never rendered as a stored status.
    assert_eq!(shown["views_are_derived"], true);
    assert!(shown["views"].as_array().unwrap().is_empty());
    assert_eq!(shown["status"], "active");
}

/// **A note outlives the file it is about, and says so.**
///
/// The subject is a snapshot rather than a foreign key precisely so that `prune_orphans` cannot
/// take a human's note with it. The record stays readable, its subject reports `missing`, and
/// nothing in the record itself changed.
#[test]
fn a_note_survives_the_deletion_of_its_subject_and_reports_it() {
    let (_dir, root) = repository(true);
    let root = root.as_path();
    assert_eq!(code(&propose(root, "m1", "a note about math.ts", None)), 0);
    assert_eq!(
        code(&run(&["memory", "confirm", "m1", "--path", as_str(root)])),
        0
    );
    let before = json(&run(&[
        "memory",
        "show",
        "m1",
        "--path",
        as_str(root),
        "--json",
    ]));
    assert_eq!(record(&before, "m1")["subject_resolution"], "resolved");

    std::fs::remove_file(root.join("src/math.ts")).unwrap();
    let reindexed = run(&["index", as_str(root)]);
    assert!(
        [0, 3].contains(&code(&reindexed)),
        "re-index: {}",
        stderr(&reindexed)
    );

    let after = json(&run(&[
        "memory",
        "show",
        "m1",
        "--path",
        as_str(root),
        "--json",
    ]));
    let shown = record(&after, "m1");
    assert_eq!(shown["subject_resolution"], "missing");
    assert!(shown["subject_resolution_note"].as_str().unwrap().len() > 20);
    assert_eq!(
        shown["subject"],
        record(&before, "m1")["subject"],
        "the snapshot is what makes the record still nameable"
    );
    assert_eq!(shown["content"], "a note about math.ts");
    // The tree moved on, so the note is qualified — derived, and reported beside the status rather
    // than written into it.
    let views: Vec<&str> = shown["views"]
        .as_array()
        .unwrap()
        .iter()
        .map(|view| view["view"].as_str().unwrap())
        .collect();
    assert!(
        views.contains(&"potentially_stale"),
        "the anchor is no longer the current state: {shown}"
    );
    assert_eq!(shown["status"], "active", "a view is not a status");
}

/// Search is a literal substring over what the human wrote, and finds nothing by path.
#[test]
fn search_matches_content_and_claim_key_and_never_the_subject_path() {
    let (_dir, root) = repository(true);
    let root = root.as_path();
    assert_eq!(
        code(&propose(
            root,
            "m1",
            "the retry budget is 100% of three",
            Some("retry-policy")
        )),
        0
    );
    assert_eq!(code(&propose(root, "m2", "an unrelated sentence", None)), 0);

    for (query, expected) in [
        ("RETRY BUDGET", vec!["m1"]),
        ("retry-policy", vec!["m1"]),
        ("unrelated", vec!["m2"]),
        // A wildcard is a character here. Unescaped it would match every record.
        ("%", vec!["m1"]),
        ("src/math.ts", vec![]),
    ] {
        let found = run(&["memory", "search", query, "--path", as_str(root), "--json"]);
        assert_eq!(code(&found), 0, "{query}: {}", stderr(&found));
        let answer = json(&found);
        let ids: Vec<&str> = answer["records"]
            .as_array()
            .unwrap()
            .iter()
            .map(|record| record["memory_id"].as_str().unwrap())
            .collect();
        assert_eq!(ids, expected, "search {query:?} answered {answer}");
    }
}

/// `--subject` filters by a selector resolved against the live index, and `--scope` by facet.
#[test]
fn list_filters_by_subject_scope_and_status() {
    let (_dir, root) = repository(true);
    let root = root.as_path();
    assert_eq!(code(&propose(root, "m1", "about math", None)), 0);
    assert_eq!(
        code(&run(&[
            "memory",
            "propose",
            "--subject",
            "file:src/shapes.ts",
            "--scope",
            "process",
            "--content",
            "about shapes",
            "--id",
            "m2",
            "--path",
            as_str(root),
        ])),
        0
    );
    assert_eq!(
        code(&run(&["memory", "confirm", "m2", "--path", as_str(root)])),
        0
    );

    let ids = |args: &[&str]| -> Vec<String> {
        let mut full = vec!["memory", "list"];
        full.extend(args);
        full.extend(["--path", as_str(root), "--json"]);
        let output = run(&full);
        assert_eq!(code(&output), 0, "{args:?}: {}", stderr(&output));
        json(&output)["records"]
            .as_array()
            .unwrap()
            .iter()
            .map(|record| record["memory_id"].as_str().unwrap().to_string())
            .collect()
    };

    assert_eq!(ids(&[]), ["m1", "m2"]);
    assert_eq!(ids(&["--subject", "file:src/shapes.ts"]), ["m2"]);
    assert_eq!(ids(&["--scope", "implementation"]), ["m1"]);
    assert_eq!(ids(&["--status", "active"]), ["m2"]);
    assert_eq!(
        ids(&["--scope", "process", "--status", "proposed"]).len(),
        0
    );

    // A selector that names nothing indexed is a refusal, not an empty list.
    let missing = run(&[
        "memory",
        "list",
        "--subject",
        "file:src/nowhere.ts",
        "--path",
        as_str(root),
    ]);
    assert_ne!(code(&missing), 0, "{}", stdout(&missing));
}

// ---- export ------------------------------------------------------------------------------------

/// **The same database exports byte-identically twice, and carries the record just written.**
#[test]
fn the_export_is_deterministic_and_contains_what_was_written() {
    let (_dir, root) = repository(true);
    let root = root.as_path();
    assert_eq!(
        code(&propose(root, "m1", "the note to export", Some("owner"))),
        0
    );
    assert_eq!(
        code(&run(&["memory", "confirm", "m1", "--path", as_str(root)])),
        0
    );
    assert_eq!(
        code(&run(&[
            "memory",
            "cite",
            "m1",
            "--file",
            "src/math.ts",
            "--span",
            "1:4",
            "--path",
            as_str(root)
        ])),
        0
    );

    let first = run(&["memory", "export", "--path", as_str(root)]);
    assert_eq!(code(&first), 0, "{}", stderr(&first));
    let second = run(&["memory", "export", "--path", as_str(root)]);
    assert_eq!(
        stdout(&first),
        stdout(&second),
        "two exports of one database differ"
    );

    let document: serde_json::Value = serde_json::from_str(&stdout(&first)).unwrap();
    assert_eq!(document["format"], "nerve-memory-export");
    assert_eq!(document["format_version"], 1);
    assert_eq!(document["schema_version"], 10);
    assert_eq!(document["record_count"], 1);
    assert!(document["repo_id"].as_str().unwrap().starts_with("repo_"));

    let exported = &document["records"][0];
    assert_eq!(exported["memory_id"], "m1");
    assert_eq!(exported["content"], "the note to export");
    assert_eq!(exported["claim_key"], "owner");
    assert_eq!(exported["status"], "active");
    assert_eq!(exported["subject"]["path"], "src/math.ts");
    assert_eq!(exported["citations"][0]["cited_span"], "1:4");
    let operations: Vec<&str> = exported["events"]
        .as_array()
        .unwrap()
        .iter()
        .map(|event| event["operation"].as_str().unwrap())
        .collect();
    assert_eq!(operations, ["proposed", "confirmed", "cited"]);

    // `--out` writes the same bytes, so the two ways of getting the document cannot disagree.
    let file = root.join("memory-export.json");
    let written = run(&[
        "memory",
        "export",
        "--out",
        as_str(&file),
        "--path",
        as_str(root),
    ]);
    assert_eq!(code(&written), 0, "{}", stderr(&written));
    assert_eq!(std::fs::read_to_string(&file).unwrap(), stdout(&first));
}

/// **The export carries no timestamp of its own, no derived state and no absolute path.**
///
/// Each omission is deliberate. An `exported_at` would break the determinism above on the only
/// field nobody needed; a derived view in a file is a query-time verdict presented as stored truth;
/// an absolute path records one machine's layout inside a document meant to outlive it.
#[test]
fn the_export_omits_the_three_things_that_would_make_it_a_claim() {
    let (_dir, root) = repository(true);
    let root = root.as_path();
    assert_eq!(code(&propose(root, "m1", "a note", Some("owner"))), 0);
    assert_eq!(
        code(&run(&["memory", "confirm", "m1", "--path", as_str(root)])),
        0
    );
    // A second active record on the same subject and claim, so `conflicted` and `multiple_active`
    // are both genuinely true right now — without this the scan below would prove nothing.
    assert_eq!(
        code(&propose(root, "m2", "a competing note", Some("owner"))),
        0
    );
    assert_eq!(
        code(&run(&["memory", "confirm", "m2", "--path", as_str(root)])),
        0
    );
    let shown = json(&run(&[
        "memory",
        "show",
        "m1",
        "--path",
        as_str(root),
        "--json",
    ]));
    let views: Vec<&str> = record(&shown, "m1")["views"]
        .as_array()
        .unwrap()
        .iter()
        .map(|view| view["view"].as_str().unwrap())
        .collect();
    assert!(
        views.contains(&"conflicted") && views.contains(&"multiple_active"),
        "the read reports no view, so the export scan below is vacuous: {views:?}"
    );

    let text = stdout(&run(&["memory", "export", "--path", as_str(root)]));
    for forbidden in [
        "exported_at",
        "potentially_stale",
        "conflicted",
        "multiple_active",
        "current_state_id",
        "subject_resolution",
        "superseded_by",
    ] {
        assert!(
            !text.contains(forbidden),
            "the export carries {forbidden:?}, which is derived or dated:\n{text}"
        );
    }
    assert!(!text.contains("/Users/"), "the export carries a home path");
    assert!(
        !text.contains(as_str(root)),
        "the export carries the repository root"
    );

    // Key order is sorted, which is what makes the byte-identity above a property of the data
    // rather than of one build's map iteration order.
    let keys: Vec<&str> = text
        .lines()
        .filter_map(|line| line.trim().strip_prefix('"'))
        .filter_map(|line| line.split('"').next())
        .collect();
    let top = [
        "format",
        "format_version",
        "record_count",
        "records",
        "repo_id",
        "schema_version",
    ];
    let found: Vec<&str> = keys
        .iter()
        .copied()
        .filter(|key| top.contains(key))
        .collect();
    assert_eq!(found, top, "the document's keys are not in sorted order");
}
