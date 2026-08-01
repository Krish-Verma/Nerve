//! ARCHITECTURE.md invariant 3, enforced rather than asserted in a comment.
//!
//! `nerve-server` is a surface. It may parse arguments, call the application layer, shape JSON
//! and map outcomes to status codes. Traversal, SQL, evidence assembly, identity computation and
//! path resolution live in `nerve-store` and `nerve-index`, so the CLI, this server and the
//! Slice 8 MCP tools answer the same question the same way.
//!
//! The check mirrors `crates/nerve-cli/tests/cli.rs`. It is deliberately crude: a grep that
//! fails loudly is worth more than a convention nobody rechecks.

const SOURCES: [(&str, &str); 9] = [
    ("lib.rs", include_str!("../src/lib.rs")),
    ("api.rs", include_str!("../src/api.rs")),
    ("assets.rs", include_str!("../src/assets.rs")),
    ("error.rs", include_str!("../src/error.rs")),
    ("guard.rs", include_str!("../src/guard.rs")),
    ("request.rs", include_str!("../src/request.rs")),
    ("respond.rs", include_str!("../src/respond.rs")),
    ("router.rs", include_str!("../src/router.rs")),
    ("shapes.rs", include_str!("../src/shapes.rs")),
];

/// Product code only.
///
/// A unit test that proves `Host: 0.0.0.0` is refused necessarily contains the string
/// `0.0.0.0`, and a test that proves no CORS header is emitted necessarily names one. Scanning
/// test code for the very strings the tests exist to refuse would make the two checks mutually
/// exclusive, so the scan stops at the test module.
fn product_code(source: &str) -> &str {
    match source.find("#[cfg(test)]") {
        Some(index) => &source[..index],
        None => source,
    }
}

#[test]
fn the_server_contains_no_sql_and_no_traversal() {
    for (name, source) in SOURCES.map(|(name, source)| (name, product_code(source))) {
        for forbidden in [
            "SELECT ",
            "INSERT INTO",
            "UPDATE ",
            "DELETE FROM",
            "FROM assertion",
            "FROM entity",
            "ORDER BY",
            ".prepare(",
            "query_map",
            "query_row",
            "VecDeque",
            "BinaryHeap",
        ] {
            assert!(
                !source.contains(forbidden),
                "nerve-server/{name} must not contain {forbidden:?}"
            );
        }
    }
}

/// The graph is walked in one place. A second walker is a second set of answers.
#[test]
fn the_server_computes_no_graph_of_its_own() {
    for (name, source) in SOURCES.map(|(name, source)| (name, product_code(source))) {
        for forbidden in [
            "fn traverse",
            "fn walk",
            "fn expand",
            "adjacency",
            "breadth_first",
            "content_hash(",
            "canonicalize(",
            "std::fs::read",
            "File::open",
        ] {
            assert!(
                !source.contains(forbidden),
                "nerve-server/{name} must not contain {forbidden:?}"
            );
        }
    }
}

/// One exception, deliberately narrow: `token.rs` opens `/dev/urandom`. Reading the operating
/// system's randomness source is not a repository read and has no path-safety dimension, so it
/// does not belong behind the repository prober.
#[test]
fn the_only_file_the_server_opens_itself_is_the_randomness_source() {
    const TOKEN: &str = include_str!("../src/token.rs");
    assert!(TOKEN.contains("/dev/urandom"));
    assert!(!TOKEN.contains("root"));
    assert!(!TOKEN.contains("rel_path"));
}

/// The storage layer is reached through `nerve-store`'s own API, never by linking SQLite here.
#[test]
fn the_server_does_not_depend_on_a_database_or_a_parser() {
    const MANIFEST: &str = include_str!("../Cargo.toml");
    for forbidden in [
        "rusqlite",
        "blake3",
        "tree-sitter",
        "tokio",
        "axum",
        "hyper",
    ] {
        assert!(
            !MANIFEST.contains(forbidden),
            "nerve-server must not depend on {forbidden:?}"
        );
    }
}

/// No route may bind anything but loopback, and no code path may construct another address.
#[test]
fn nothing_in_the_crate_can_bind_a_non_loopback_address() {
    for (name, source) in SOURCES.map(|(name, source)| (name, product_code(source))) {
        for forbidden in ["0.0.0.0", "UNSPECIFIED", "Ipv4Addr::new", "to_socket_addrs"] {
            assert!(
                !source.contains(forbidden),
                "nerve-server/{name} must not contain {forbidden:?}"
            );
        }
    }
    const LIB: &str = include_str!("../src/lib.rs");
    assert!(LIB.contains("Ipv4Addr::LOCALHOST"));
}

/// No CORS header may be *emitted* by any module.
///
/// The match requires the opening quote of a string literal, because a header this crate could
/// send has to be written as one. Prose that explains why the header is absent — which several
/// modules carry, deliberately — is not a header.
#[test]
fn no_module_can_emit_a_cors_header() {
    for (name, source) in SOURCES.map(|(name, source)| (name, product_code(source))) {
        let lowered = source.to_ascii_lowercase();
        for forbidden in ["\"access-control", "'access-control", "b\"access-control"] {
            assert!(
                !lowered.contains(forbidden),
                "nerve-server/{name} must not contain {forbidden:?}"
            );
        }
    }
    // And the fixed header table names nothing of the sort.
    for (field, _) in nerve_server::respond::SECURITY_HEADERS {
        assert!(!field.to_ascii_lowercase().starts_with("access-control"));
    }
}
