//! Offline-first is enforced structurally, not by intention.
//!
//! # What this asserts, precisely
//!
//! Since Slice 4a, Nerve **does** have a network stack: `tiny_http` is an inbound HTTP listener
//! bound to `127.0.0.1`. Claiming "no networking crates" would be false, and this file used to
//! say so. The accurate and load-bearing invariant is narrower:
//!
//! > Nerve has **no outbound network client**. It listens locally; it never dials out.
//!
//! So the forbidden list below is not "anything that touches a socket" — it is HTTP/TLS *client*
//! stacks, async runtimes that would bring one, telemetry, analytics, update checkers and crash
//! reporters. Dev-dependencies and build-dependencies are included, because a test or build
//! script could otherwise reach the network during CI.
//!
//! `tiny_http` is deliberately **not** forbidden, and its presence is what makes the distinction
//! necessary rather than pedantic. See `docs/SECURITY.md` and `docs/THREAT-MODEL.md`.

use std::process::Command;

/// Crates that would give any part of this workspace an **outbound** network capability,
/// telemetry, analytics, update checking or crash reporting.
///
/// Inbound-only listeners are not here by design: see the module documentation.
const FORBIDDEN: [&str; 20] = [
    // async runtimes that exist to drive network clients
    "tokio",
    "async-std",
    "smol",
    // HTTP and RPC clients
    "reqwest",
    "hyper",
    "hyper-util",
    "ureq",
    "curl",
    "isahc",
    "attohttpc",
    "tonic",
    // TLS, which only an outbound client needs here
    "native-tls",
    "rustls",
    "openssl",
    // raw socket plumbing
    "socket2",
    // telemetry, analytics, update checking, crash reporting
    "sentry",
    "opentelemetry",
    "tracing-opentelemetry",
    "self_update",
    "posthog-rs",
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
fn no_outbound_network_client_is_in_the_dependency_tree() {
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
        "outbound network client / telemetry crate(s) reachable from this workspace: {found:?}"
    );
}

/// The inbound listener is expected, and naming it here keeps the distinction honest: a future
/// reader must not "fix" the test above by concluding Nerve has no network stack at all.
#[test]
fn the_only_network_crate_is_the_inbound_listener() {
    let metadata = metadata();
    let names: Vec<&str> = metadata["packages"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|package| package["name"].as_str())
        .collect();
    assert!(
        names.contains(&"tiny_http"),
        "tiny_http is the local server; if it is gone, revisit this test and SECURITY.md"
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
