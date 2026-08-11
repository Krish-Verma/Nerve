//! ARCHITECTURE.md invariant 3, enforced rather than asserted in a comment.
//!
//! `nerve-server` is a surface. It may parse arguments, call the application layer, shape JSON
//! and map outcomes to status codes. Traversal, SQL, evidence assembly, identity computation and
//! path resolution live in `nerve-store` and `nerve-index`, so the CLI, this server and the
//! Slice 8 MCP tools answer the same question the same way.
//!
//! The check mirrors `crates/nerve-cli/tests/cli.rs`. It is deliberately crude: a grep that
//! fails loudly is worth more than a convention nobody rechecks.

use std::path::Path;

/// Every `.rs` file under `src/`, read at run time rather than listed here.
///
/// This was a hand-maintained `include_str!` array until a new module under `src/mcp/` was one
/// commit away from evading all four scans below — not only the SQL one, but the loopback-binding
/// and CORS ones too. A file nobody remembered to add would have been silently exempt from the
/// invariants this file exists to enforce.
///
/// `crates/nerve-cli/tests/cli.rs` already reads its crate's `src/` directory for exactly this
/// reason, with the comment "scanning one file would let it be escaped by adding a second one" —
/// Slice 7c-ii widened it there after the guard had only ever looked at `main.rs`. The server's
/// copy kept the older shape. This is that correction, one crate over, and it recurses because
/// `nerve-server/src/` has an `mcp/` subdirectory where the CLI's has none.
///
/// The returned text is already stripped to product code. A unit test proving `Host: 0.0.0.0` is
/// refused necessarily contains the string `0.0.0.0`, and a test proving no CORS header is
/// emitted necessarily names one, so scanning test code for the very strings the tests exist to
/// refuse would make the two checks mutually exclusive.
fn sources() -> Vec<(String, String)> {
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut found = Vec::new();
    collect(&src, &src, &mut found);
    found.sort();

    // Anti-vacuity. A walk that silently found nothing would make every scan below pass by
    // iterating an empty list, which is the failure mode a directory read introduces and the
    // hand-written array could not have.
    //
    // 17 is the measured count, not the 16 the old array listed — the discrepancy is the defect
    // this function was written to close. A floor rather than an equality so the crate may grow,
    // but tight enough that deleting a module has to be deliberate.
    assert!(
        found.len() >= 17,
        "expected to scan the whole crate, found {} files: {:?}",
        found.len(),
        found.iter().map(|(name, _)| name).collect::<Vec<_>>()
    );
    found
}

fn collect(base: &Path, dir: &Path, found: &mut Vec<(String, String)>) {
    let entries = std::fs::read_dir(dir)
        .unwrap_or_else(|error| panic!("{} must be readable: {error}", dir.display()));
    for entry in entries {
        let path = entry.expect("a readable directory entry").path();
        if path.is_dir() {
            collect(base, &path, found);
        } else if path.extension().is_some_and(|ext| ext == "rs") {
            let text = std::fs::read_to_string(&path)
                .unwrap_or_else(|error| panic!("{} must be readable: {error}", path.display()));
            let name = path
                .strip_prefix(base)
                .expect("every collected path is under src/")
                .to_string_lossy()
                .into_owned();
            found.push((name, product_code(&text).to_owned()));
        }
    }
}

fn product_code(source: &str) -> &str {
    match source.find("#[cfg(test)]") {
        Some(index) => &source[..index],
        None => source,
    }
}

#[test]
fn the_server_contains_no_sql_and_no_traversal() {
    for (name, source) in sources() {
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
///
/// The file-opening half of this scan exempts `token.rs`, and the exemption is written here
/// rather than being a consequence of a list nobody maintained. Replacing the hand-written
/// source array with a directory read revealed that the array held 16 entries while `src/` holds
/// 17 `.rs` files: **`token.rs` was never scanned at all**, so it was also exempt from the SQL,
/// loopback-binding and CORS scans, which it has no business being exempt from. It is now
/// scanned by all of them, and its one legitimate `File::open` — the randomness source, whose
/// narrowness `the_only_file_the_server_opens_itself_is_the_randomness_source` pins separately —
/// is the single named exception.
#[test]
fn the_server_computes_no_graph_of_its_own() {
    /// The one file permitted to open a file, and the only exemption in this module.
    const OPENS_THE_RANDOMNESS_SOURCE: &str = "token.rs";

    let mut exempted = 0;
    for (name, source) in sources() {
        for forbidden in [
            "fn traverse",
            "fn walk",
            "fn expand",
            "adjacency",
            "breadth_first",
            "content_hash(",
        ] {
            assert!(
                !source.contains(forbidden),
                "nerve-server/{name} must not contain {forbidden:?}"
            );
        }

        if name == OPENS_THE_RANDOMNESS_SOURCE {
            exempted += 1;
            continue;
        }
        for forbidden in ["canonicalize(", "std::fs::read", "File::open"] {
            assert!(
                !source.contains(forbidden),
                "nerve-server/{name} must not contain {forbidden:?}"
            );
        }
    }

    // The exemption must apply to a file that exists. A renamed `token.rs` would otherwise turn
    // this into an exemption for nothing while the real module went unscanned.
    assert_eq!(
        exempted, 1,
        "expected to exempt exactly {OPENS_THE_RANDOMNESS_SOURCE}"
    );
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

/// Every lifecycle writer in `nerve-store`'s memory module, by the name it is called by.
///
/// Row 14 §1's control is a **surface boundary**, not an identity check, and this is the half of it
/// that is enforceable. Nerve is offline and has no accounts, so an agent invoking `nerve memory
/// confirm` at a local shell is byte-indistinguishable from a human invoking it; what can be made
/// true and kept true is that *the code path is absent from the agent surface rather than gated on
/// it*. These are the eight functions that would have to appear for it to stop being absent.
const MEMORY_LIFECYCLE_WRITERS: [&str; 8] = [
    "propose_memory",
    "confirm_memory",
    "invalidate_memory",
    "supersede_memory",
    "cite_memory",
    "insert_memory",
    "insert_memory_citation",
    "append_memory_event",
];

/// Acceptance criterion 2: **no memory write is reachable from the MCP surface.**
///
/// Two scans, because one of them alone would be escapable:
///
/// 1. `src/mcp.rs` and everything under `src/mcp/` — the surface the criterion names, with its own
///    anti-vacuity floor so a walk that found no files could not pass by iterating nothing.
/// 2. The **whole crate**, because an MCP tool calls the application layer: a writer added to
///    `src/api/memory.rs` and invoked from a tool would satisfy the first scan and defeat the
///    property. `nerve serve` opens `query_only` and every route on it is a `GET`, so no file here
///    has any business naming one of these.
///
/// The failure message names the function, so a probe that adds one fails by name rather than with
/// a count. This is the same shape as the scans above and is deliberately just as crude: a grep
/// that fails loudly is worth more than a convention nobody rechecks.
#[test]
fn no_memory_lifecycle_write_is_reachable_from_the_mcp_surface() {
    let all = sources();
    let mcp: Vec<&(String, String)> = all
        .iter()
        .filter(|(name, _)| name == "mcp.rs" || name.starts_with("mcp/"))
        .collect();

    // Anti-vacuity, twice over: the filter found the surface rather than nothing, and the walk
    // found the crate. A renamed directory would otherwise turn this into a scan of an empty list.
    assert!(
        mcp.len() >= 10,
        "expected to scan the whole MCP surface, found {} files: {:?}",
        mcp.len(),
        mcp.iter().map(|(name, _)| name).collect::<Vec<_>>()
    );
    assert!(
        mcp.iter().any(|(name, _)| name == "mcp/memory.rs"),
        "the memory tool is not in the scanned set: {:?}",
        mcp.iter().map(|(name, _)| name).collect::<Vec<_>>()
    );
    // And the scan really can see a call: the tool it is about does reach the read model, so the
    // files below are not empty of `nerve_store` calls in general.
    assert!(
        mcp.iter()
            .any(|(name, source)| name == "mcp/memory.rs" && source.contains("api::memory")),
        "the memory tool does not reach the application layer, so this scan proves nothing"
    );

    for (name, source) in &mcp {
        for writer in MEMORY_LIFECYCLE_WRITERS {
            assert!(
                !source.contains(writer),
                "nerve-server/{name} reaches {writer:?}; the MCP surface must not be able to write \
                 a memory record, and the boundary is that the path is absent rather than gated"
            );
        }
    }
    for (name, source) in &all {
        for writer in MEMORY_LIFECYCLE_WRITERS {
            assert!(
                !source.contains(writer),
                "nerve-server/{name} reaches {writer:?}; this server is read-only, and a writer in \
                 the application layer would be reachable from every surface on it"
            );
        }
    }
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
    for (name, source) in sources() {
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
    for (name, source) in sources() {
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
