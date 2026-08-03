//! Nerve never spawns a process. Enforced structurally, not by code review.
//!
//! `docs/THREAT-MODEL.md` T1 is the single most important indexing invariant: repository content
//! is untrusted, and Nerve must parse it rather than run it. No package scripts, no build tools,
//! no compilers, no test runners, no `git` binary — Git HEAD is read from `.git/HEAD` directly
//! for exactly this reason.
//!
//! Until this file existed the invariant was guaranteed only by inspection, which is precisely
//! the kind of guarantee that rots. A future extractor shelling out to `tsc` for type resolution
//! would be a natural-looking change that silently breaks T1; this test is what refuses it.
//!
//! # What is and is not forbidden
//!
//! Forbidden in **product code** (`crates/*/src/**`): anything that creates a new process —
//! `Command`, `exec*`, `fork`, `posix_spawn`, `system`.
//!
//! Not forbidden: `std::process::exit` and `std::process::abort`. They terminate the current
//! process and create nothing; the CLI uses `exit` to map outcomes to exit codes.
//!
//! **Test code is exempt** and deliberately so: `no_network.rs` runs `cargo metadata`, and
//! `cli.rs` runs the `nerve` binary end to end. Those are the harness, not the product.

use std::path::{Path, PathBuf};
use std::process::Command;

/// Substrings that indicate process creation.
const FORBIDDEN: [&str; 8] = [
    "Command::new",
    "process::Command",
    "posix_spawn",
    "libc::fork",
    "libc::execv",
    "libc::execl",
    "libc::execve",
    "libc::system",
];

/// Crates whose entire purpose is running other programs.
const FORBIDDEN_CRATES: [&str; 6] = [
    "duct",
    "subprocess",
    "command-group",
    "shell-words",
    "execute",
    "run_script",
];

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .expect("workspace root")
}

/// Every `.rs` file under `crates/*/src`. Test directories are deliberately excluded.
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

#[test]
fn product_code_contains_no_process_spawning_api() {
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
            // A line that only *names* the invariant (documentation) is not a violation.
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
        "THREAT-MODEL T1 violated — product code must never spawn a process.\n\
         Nerve parses repository content; it does not run it.\n\
         If a legitimate exception is ever needed it requires a documented threat-model \
         amendment, not an edit to this list.\n{}",
        offenders.join("\n")
    );
}

#[test]
fn no_process_running_crate_is_a_dependency() {
    let manifest = concat!(env!("CARGO_MANIFEST_DIR"), "/../../Cargo.toml");
    let cargo = std::env::var("CARGO").unwrap_or_else(|_| "cargo".to_string());
    let output = Command::new(cargo)
        .args([
            "metadata",
            "--format-version",
            "1",
            "--manifest-path",
            manifest,
        ])
        .output()
        .expect("cargo metadata must run");
    let metadata: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("cargo metadata must emit JSON");
    let names: Vec<&str> = metadata["packages"]
        .as_array()
        .expect("packages array")
        .iter()
        .filter_map(|package| package["name"].as_str())
        .collect();
    assert!(!names.is_empty());

    let found: Vec<&str> = names
        .iter()
        .copied()
        .filter(|name| FORBIDDEN_CRATES.contains(name))
        .collect();
    assert!(
        found.is_empty(),
        "process-running crate(s) present: {found:?}"
    );
}

/// `std::process::exit` is how the CLI returns an exit code. It creates nothing, and the scan
/// above must not be tightened into rejecting it.
#[test]
fn terminating_the_current_process_is_still_allowed() {
    let main = workspace_root().join("crates/nerve-cli/src/main.rs");
    let text = std::fs::read_to_string(main).expect("main.rs must be readable");
    assert!(
        text.contains("std::process::exit"),
        "the CLI maps outcomes to exit codes; if that changed, this test should change with it"
    );
}

/// The end-to-end proof: index a repository whose content would execute if anything ran it.
///
/// A package manifest with a hostile `postinstall`, and a source file that writes a marker on
/// import. If either ever runs, the marker exists and this fails.
#[test]
fn indexing_a_repository_never_executes_its_contents() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let marker = root.join("EXECUTED");
    let marker_arg = marker.display().to_string();

    std::fs::write(
        root.join("package.json"),
        format!(
            r#"{{
  "name": "hostile",
  "scripts": {{
    "postinstall": "touch {marker_arg}",
    "prepare": "touch {marker_arg}",
    "build": "touch {marker_arg}"
  }}
}}"#
        ),
    )
    .unwrap();
    std::fs::create_dir_all(root.join("src")).unwrap();
    std::fs::write(
        root.join("src/evil.ts"),
        format!(
            "import {{ writeFileSync }} from 'fs';\n\
             writeFileSync({marker_arg:?}, 'executed');\n\
             export function looksHarmless(): number {{ return 1; }}\n"
        ),
    )
    .unwrap();

    let binary = env!("CARGO_BIN_EXE_nerve");
    let path = root.to_str().unwrap();
    // `check` is in this loop because it reads repository bytes of its own: the untracked-file
    // walk opens every path the index has no row for, which on this tree is the hostile file.
    for args in [
        ["init", path],
        ["index", path],
        ["status", path],
        ["check", path],
    ] {
        Command::new(binary)
            .args(args)
            .output()
            .expect("nerve must run");
    }

    assert!(
        !marker.exists(),
        "repository content executed during indexing — THREAT-MODEL T1 violated"
    );

    // `mcp` is in the loop too, and separately, because it is the one command whose *caller* is
    // an agent that may itself have been prompt-injected by this very repository (A4). It reads
    // the same hostile tree, through the same reader, and must run nothing either.
    let mut child = Command::new(binary)
        .args(["mcp", path])
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .expect("nerve mcp must run");
    {
        use std::io::Write;
        let stdin = child.stdin.as_mut().expect("stdin is piped");
        for message in [
            r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}"#,
            r#"{"jsonrpc":"2.0","id":2,"method":"tools/list"}"#,
            r#"{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"nerve_investigate","arguments":{"selector":"src/evil.ts"}}}"#,
            r#"{"jsonrpc":"2.0","id":4,"method":"tools/call","params":{"name":"nerve_investigate","arguments":{"selector":"looksHarmless"}}}"#,
            // Every tool reads the same hostile tree through the same reader (Slice 8b-ii).
            r#"{"jsonrpc":"2.0","id":5,"method":"tools/call","params":{"name":"nerve_search","arguments":{"query":"looksHarmless"}}}"#,
            r#"{"jsonrpc":"2.0","id":6,"method":"tools/call","params":{"name":"nerve_path","arguments":{"from":"src/evil.ts","to":"looksHarmless"}}}"#,
            r#"{"jsonrpc":"2.0","id":7,"method":"tools/call","params":{"name":"nerve_impact","arguments":{"selector":"looksHarmless"}}}"#,
            r#"{"jsonrpc":"2.0","id":8,"method":"tools/call","params":{"name":"nerve_gaps","arguments":{}}}"#,
        ] {
            stdin.write_all(message.as_bytes()).unwrap();
            stdin.write_all(b"\n").unwrap();
        }
    }
    let session = child.wait_with_output().expect("nerve mcp must finish");
    assert!(
        !String::from_utf8_lossy(&session.stdout).is_empty(),
        "the session must actually have answered, or this proves nothing"
    );

    assert!(
        !marker.exists(),
        "repository content executed while serving MCP — THREAT-MODEL T1 violated"
    );
}
