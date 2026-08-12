//! The commands that are **absent by decision**, asserted as a gate rather than as a convention.
//!
//! # Why this file exists
//!
//! Row 14's plan §7.6 and `docs/CONTINUATION.md` both stated that a CLI-surface test already held
//! `nerve affected` and `nerve trace-tests` refused and would fail if a forbidden command
//! appeared. Slice 14b-ii went to reuse that mechanism for `nerve memory delete` and found **there
//! was nothing to reuse**: `affected` appeared in no test under `crates/nerve-cli/tests/` at all,
//! and `trace-tests` only in doc comments and in `scripts/final_acceptance.sh`.
//!
//! So the two most load-bearing refusals this product makes — one of them settled by an ADR — were
//! held only by an acceptance script. A script is something a developer may never run; it is not a
//! gate. `cargo test` is. This file is the gate the documentation assumed already existed.
//!
//! # What each refusal costs, and why it is not a gap
//!
//! - **`nerve affected`** — *"which tests would my change affect?"* is unanswerable from coverage
//!   evidence, because LCOV carries no per-test attribution (`docs/decisions/ADR-0008` §A.2). The
//!   command is absent rather than shipped with the attribution guessed, and the same reasoning
//!   forbids deriving it from aggregate coverage and calling the result test attribution.
//! - **`nerve trace-tests`** — Nerve must not run a repository's test runner.
//!   `crates/nerve-cli/tests/no_subprocess.rs`'s own module doc names *"no test runners"* as what
//!   it exists to refuse, so this command would need an exception to a security boundary. Tracing
//!   is **ingest-only**: an external tracer writes an artifact and `nerve trace import` reads it.
//!
//! A future slice that believes it has the evidence for either one must delete a test here and say
//! so in the same commit, which is the whole point: the decision becomes visible instead of
//! drifting.

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

/// Every top-level subcommand the binary offers, read out of `--help`.
///
/// Parsed from the help rather than from a list in this file, because a list here would be a
/// second copy of the command surface and could agree with itself while disagreeing with the
/// binary. The caller asserts something known-present before trusting the result, so a parse that
/// silently returned nothing cannot make an absence assertion pass.
fn subcommands() -> Vec<String> {
    let help = run(&["--help"]);
    assert_eq!(code(&help), 0, "`nerve --help` must succeed");
    stdout(&help)
        .lines()
        .filter_map(|line| line.strip_prefix("  "))
        .filter(|line| !line.starts_with(' ') && !line.starts_with('-'))
        .filter_map(|line| line.split_whitespace().next().map(str::to_string))
        .collect()
}

/// The two refusals, both halves each: not accepted when typed, and not offered in the help.
///
/// **The anti-vacuity assertion is the first one.** Without it this test passes just as happily
/// against a binary that refuses *everything*, or against a help output this parser failed to
/// read — which is the failure mode that let four hostile fixtures attack nothing in Slice 11a
/// while a green suite reported them passing.
#[test]
fn the_two_refused_commands_are_absent_and_the_surface_they_are_absent_from_is_real() {
    let offered = subcommands();

    // Anti-vacuity: this is a real command surface, not an empty parse.
    for present in ["index", "search", "why", "impact", "trace", "memory"] {
        assert!(
            offered.contains(&present.to_string()),
            "`nerve {present}` should exist, so this help was not read correctly: {offered:?}"
        );
    }
    assert!(
        offered.len() >= 14,
        "expected the whole command surface, found {}: {offered:?}",
        offered.len()
    );

    for (refused, why) in [
        (
            "affected",
            "ADR-0008 §A.2: LCOV carries no per-test attribution",
        ),
        (
            "trace-tests",
            "THREAT-MODEL T1 / no_subprocess.rs: Nerve must not run a repository's test runner",
        ),
    ] {
        assert!(
            !offered.contains(&refused.to_string()),
            "`nerve {refused}` is offered in the help, and it must not exist — {why}"
        );

        let attempted = run(&[refused]);
        assert_ne!(
            code(&attempted),
            0,
            "`nerve {refused}` was accepted, and it must not exist — {why}"
        );
        assert!(
            stderr(&attempted).contains("unrecognized subcommand"),
            "`nerve {refused}` was understood rather than rejected as unknown — {why}. stderr: {}",
            stderr(&attempted)
        );
    }
}

/// `trace-tests` is absent **while `trace` is present**, which is the distinction it exists to make.
///
/// A test asserting only that `trace-tests` is missing would also pass against a build with no
/// tracing at all, and would then be reporting a shipped feature's absence as a security property.
/// Nerve *does* have tracing — it is ingest-only, so the artifact is produced by a tracer the user
/// runs and `nerve trace import` reads it, and no process is spawned.
#[test]
fn tracing_exists_and_it_is_ingest_only() {
    let offered = subcommands();
    assert!(
        offered.contains(&"trace".to_string()),
        "`nerve trace` should exist: {offered:?}"
    );
    assert!(
        !offered.contains(&"trace-tests".to_string()),
        "`nerve trace-tests` must not exist: {offered:?}"
    );

    // And the surviving surface really is ingestion rather than execution.
    let help = run(&["trace", "--help"]);
    assert_eq!(code(&help), 0, "`nerve trace --help` must succeed");
    let text = stdout(&help);
    assert!(
        text.contains("import"),
        "`nerve trace` should offer `import`: {text}"
    );
    for spawning in ["run", "exec", "pytest", "execute"] {
        assert!(
            // `split_whitespace` already skips leading whitespace, so the help's indentation
            // needs no trimming first — clippy's `trim_split_whitespace` catches the redundancy.
            !text
                .lines()
                .any(|line| line.split_whitespace().next().is_some_and(|verb| verb == spawning)),
            "`nerve trace {spawning}` is offered, which would need an exception to no_subprocess.rs"
        );
    }
}
