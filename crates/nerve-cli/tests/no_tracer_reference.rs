//! Nerve's product code does not know the reference tracer exists. Enforced structurally.
//!
//! Slice 11b adds `tracers/python/nerve_trace/`, a Python package that lives in this repository and
//! is **not part of the Nerve product**. `docs/plans/slice-11b-python-tracer.md` §1 states why the
//! boundary needs a test rather than a convention:
//!
//! > `crates/nerve-cli/tests/no_subprocess.rs` scans `crates/*/src/**` and its module documentation
//! > names *"no test runners"* as what it exists to refuse. A Python package inside Nerve's
//! > repository is exactly the thing that could quietly erode that.
//!
//! So: **no Rust source under `crates/*/src/**` may name the tracer, `pytest`, or the `tracers/`
//! directory.** If product code ever learns the tracer's name, it is one step from learning how to
//! launch it — and `nerve trace-tests` would follow, which is the command this project has twice
//! refused to add (`docs/plans/slice-11-test-observed-calls.md` §1,
//! `docs/plans/slice-11a-trace-ingestion.md` §1).
//!
//! This complements `no_subprocess.rs` rather than duplicating it. That file forbids the *mechanism*
//! — `Command::new` and its relatives. This one forbids the *intent*, which is the earlier and more
//! recoverable failure: a constant naming the tracer would pass `no_subprocess.rs` and would be the
//! first line of the change that does not.
//!
//! # What is exempt, and why
//!
//! **Documentation is exempt.** A `//` comment that names `pytest` is a module explaining what a
//! `test_id` looks like — `crates/nerve-index/src/trace.rs:138` does exactly that — and a scan that
//! could not tell prose from code would force the invariant to go unexplained.
//! `no_subprocess.rs::product_code_contains_no_process_spawning_api` strips comments for the same
//! reason and this file uses the same rule.
//!
//! **`*_tests.rs` is exempt**, because in this workspace that suffix means a `#[cfg(test)]` module
//! that happens to live under `src/`: `crates/nerve-index/src/trace_tests.rs` is included by
//! `trace.rs` through `#[cfg(test)] #[path = "trace_tests.rs"]`, and its fixture header string
//! contains `"test_framework":"pytest"` because that is what the contract says. `no_subprocess.rs`
//! grants test code the same exemption and grants it by directory; that is not enough here, so
//! [`every_excluded_file_really_is_a_test_module`] proves the exemption is not a loophole.

use std::path::{Path, PathBuf};

/// Strings that would mean product code knows about the tracer.
///
/// `pytest` is here and not only `nerve_trace` because the harm is knowing about a test *runner* at
/// all. A constant naming `pytest` in product code is a constant that exists to be passed to
/// something.
const FORBIDDEN: [&str; 3] = ["nerve_trace", "tracers/", "pytest"];

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .expect("workspace root")
}

/// Every `.rs` file under `crates/*/src`, minus the `#[cfg(test)]` modules that live there.
fn product_sources() -> Vec<PathBuf> {
    let mut out = Vec::new();
    let crates = workspace_root().join("crates");
    let mut crate_dirs: Vec<PathBuf> = std::fs::read_dir(&crates)
        .expect("crates/ must be readable")
        .filter_map(|entry| entry.ok().map(|entry| entry.path()))
        .filter(|path| path.is_dir())
        .collect();
    crate_dirs.sort();
    for crate_dir in crate_dirs {
        collect_rs(&crate_dir.join("src"), &mut out);
    }
    out.retain(|path| !is_test_module(path));
    out.sort();
    out
}

fn collect_rs(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.filter_map(|entry| entry.ok()) {
        let path = entry.path();
        if path.is_dir() {
            collect_rs(&path, out);
        } else if path.extension().is_some_and(|ext| ext == "rs") {
            out.push(path);
        }
    }
}

fn is_test_module(path: &Path) -> bool {
    path.file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| name.ends_with("_tests.rs"))
}

fn excluded_files() -> Vec<PathBuf> {
    let mut all = Vec::new();
    let crates = workspace_root().join("crates");
    let mut crate_dirs: Vec<PathBuf> = std::fs::read_dir(&crates)
        .expect("crates/ must be readable")
        .filter_map(|entry| entry.ok().map(|entry| entry.path()))
        .filter(|path| path.is_dir())
        .collect();
    crate_dirs.sort();
    for crate_dir in crate_dirs {
        collect_rs(&crate_dir.join("src"), &mut all);
    }
    all.retain(|path| is_test_module(path));
    all.sort();
    all
}

#[test]
fn product_code_never_names_the_tracer_or_a_test_runner() {
    let sources = product_sources();
    assert!(
        sources.len() > 20,
        "expected to scan the whole product, found {} files",
        sources.len()
    );

    let mut offenders: Vec<String> = Vec::new();
    for path in &sources {
        let text = std::fs::read_to_string(path).expect("source must be readable");
        for (index, line) in text.lines().enumerate() {
            // A line that only *documents* the invariant is not a violation. `///`, `//!` and `//`
            // all vanish here, which is what lets `trace.rs` explain what a `pytest` node id is.
            let code = line.split("//").next().unwrap_or("");
            for needle in FORBIDDEN {
                if code.contains(needle) {
                    let relative = path
                        .strip_prefix(workspace_root())
                        .unwrap_or(path)
                        .display();
                    offenders.push(format!("{relative}:{}: {needle}", index + 1));
                }
            }
        }
    }

    assert!(
        offenders.is_empty(),
        "product code named the reference tracer or a test runner.\n\
         `tracers/python/` is not part of the Nerve product: `nerve` cannot start it, must not \
         reference it, and does not run anyone's test suite.\n\
         If a legitimate exception is ever needed it requires a documented amendment to \
         docs/plans/slice-11b-python-tracer.md §1, not an edit to this list.\n{}",
        offenders.join("\n")
    );
}

/// The exemption above must be an exemption for test code, not a hole any file can fall through.
#[test]
fn every_excluded_file_really_is_a_test_module() {
    for path in excluded_files() {
        let name = path
            .file_name()
            .and_then(|n| n.to_str())
            .expect("file name");
        let directory = path.parent().expect("a file under src/ has a parent");
        let mut siblings = Vec::new();
        collect_rs(directory, &mut siblings);
        let included_under_cfg_test = siblings.iter().any(|sibling| {
            if sibling == &path {
                return false;
            }
            let Ok(text) = std::fs::read_to_string(sibling) else {
                return false;
            };
            text.contains("#[cfg(test)]") && text.contains(&format!("#[path = \"{name}\"]"))
        });
        assert!(
            included_under_cfg_test,
            "{} is excluded from the scan on the strength of its name, but no sibling includes it \
             under #[cfg(test)]. Either it is product code — in which case the exclusion is a hole \
             — or the convention changed and this test should change with it.",
            path.display()
        );
    }
}

/// The scan must be guarding something that exists.
///
/// A test that passes because the tracer was deleted, renamed or never landed proves nothing, and
/// this is the same reason `no_subprocess.rs` asserts it scanned more than twenty files.
#[test]
fn the_tracer_this_test_exists_for_is_present() {
    let package = workspace_root().join("tracers/python/nerve_trace");
    assert!(
        package.join("__init__.py").is_file(),
        "the reference tracer is missing; this test would then be guarding nothing"
    );
    for module in [
        "frames.py",
        "monitoring_backend.py",
        "paths.py",
        "pytest_plugin.py",
        "record.py",
        "settrace_backend.py",
    ] {
        assert!(
            package.join(module).is_file(),
            "the reference tracer is missing {module}"
        );
    }
}

/// The producer and the consumer must agree on the contract's identifiers.
///
/// Not a substitute for the end-to-end run — `scripts/trace_python_e2e.sh` is that, and it needs
/// pytest — but the cheap half of it is hermetic: if `trace.rs` renamed the format or moved the
/// version, the tracer would emit a header `read_header` counts as `header-missing` and every record
/// after it would be unattributable. Catching that here costs nothing and needs no Python.
#[test]
fn the_producer_and_the_reader_agree_on_the_format_stamp() {
    let reader = std::fs::read_to_string(workspace_root().join("crates/nerve-index/src/trace.rs"))
        .expect("the reader must be readable");
    let producer =
        std::fs::read_to_string(workspace_root().join("tracers/python/nerve_trace/record.py"))
            .expect("the producer must be readable");

    assert!(reader.contains(r#"pub const FORMAT: &str = "nerve-trace";"#));
    assert!(producer.contains(r#"FORMAT = "nerve-trace""#));
    assert!(reader.contains("pub const FORMAT_VERSION: u64 = 1;"));
    assert!(producer.contains("FORMAT_VERSION = 1"));

    for bound in [
        ("MAX_STRING_BYTES: usize = 512", "MAX_STRING_BYTES = 512"),
        (
            "MAX_RECORD_BYTES: usize = 8 * 1024",
            "MAX_RECORD_BYTES = 8 * 1024",
        ),
        ("MAX_RECORDS: usize = 500_000", "MAX_RECORDS = 500_000"),
        (
            "MAX_ARTIFACT_BYTES: usize = 32 * 1024 * 1024",
            "MAX_ARTIFACT_BYTES = 32 * 1024 * 1024",
        ),
    ] {
        assert!(
            reader.contains(bound.0),
            "the reader's bound moved: {}. The producer respects these bounds so that it never \
             emits a record the reader will refuse; if one changes, both change.",
            bound.0
        );
        assert!(
            producer.contains(bound.1),
            "the producer's bound moved: {}",
            bound.1
        );
    }
}
