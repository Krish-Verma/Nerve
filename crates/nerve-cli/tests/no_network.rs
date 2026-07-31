//! Offline-first is enforced structurally, not by intention.
//!
//! CLAUDE.md §2 and SECURITY.md require that Nerve makes no network calls. The strongest
//! check available at test time is that no networking crate is reachable in the dependency
//! graph at all — including dev-dependencies and build-dependencies, because a test or build
//! script could otherwise reach the network during CI.

use std::process::Command;

/// Crates that would give any part of this workspace a network stack.
const FORBIDDEN: [&str; 8] = [
    "tokio",
    "reqwest",
    "hyper",
    "ureq",
    "curl",
    "native-tls",
    "rustls",
    "socket2",
];

fn cargo() -> String {
    std::env::var("CARGO").unwrap_or_else(|_| "cargo".to_string())
}

fn metadata() -> serde_json::Value {
    let manifest = concat!(env!("CARGO_MANIFEST_DIR"), "/../../Cargo.toml");
    let output = Command::new(cargo())
        .args([
            "metadata",
            "--format-version",
            "1",
            "--manifest-path",
            manifest,
        ])
        .output()
        .expect("cargo metadata must run");
    assert!(
        output.status.success(),
        "cargo metadata failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).expect("cargo metadata must emit JSON")
}

#[test]
fn no_networking_crate_is_in_the_dependency_tree() {
    let metadata = metadata();
    let packages = metadata["packages"].as_array().expect("packages array");
    let names: Vec<&str> = packages
        .iter()
        .filter_map(|package| package["name"].as_str())
        .collect();
    assert!(!names.is_empty(), "cargo metadata returned no packages");

    let found: Vec<&str> = names
        .iter()
        .copied()
        .filter(|name| FORBIDDEN.contains(name))
        .collect();
    assert!(
        found.is_empty(),
        "networking crate(s) reachable from this workspace: {found:?}"
    );
}

#[test]
fn the_workspace_declares_no_telemetry_dependency() {
    let metadata = metadata();
    let packages = metadata["packages"].as_array().unwrap();
    for package in packages {
        let name = package["name"].as_str().unwrap_or_default();
        assert!(
            !name.contains("telemetry") && !name.contains("analytics") && !name.contains("sentry"),
            "telemetry-shaped dependency: {name}"
        );
    }
}

/// Slice 1 uses no async runtime, which is both a performance-simplicity and an offline
/// posture decision: there is nothing to await because there is nothing remote.
#[test]
fn no_async_runtime_is_present() {
    let metadata = metadata();
    let names: Vec<&str> = metadata["packages"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|package| package["name"].as_str())
        .collect();
    for runtime in ["tokio", "async-std", "smol", "futures-executor"] {
        assert!(
            !names.contains(&runtime),
            "async runtime present: {runtime}"
        );
    }
}
