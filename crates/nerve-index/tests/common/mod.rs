//! Shared helpers for the integration tests.
//!
//! Each integration test binary compiles this module independently and uses only part of it,
//! so unused-code warnings here are an artefact of the test layout, not real dead code.

#![allow(dead_code)]

use std::path::{Path, PathBuf};

/// Project id used by every test that compares ids, so that comparisons are meaningful.
pub const TEST_PROJECT_ID: &str = "00000000000000000000000000000001";

/// Path to a committed fixture tree.
pub fn named_fixture_root(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures")
        .join(name)
        .canonicalize()
        .unwrap_or_else(|err| panic!("fixtures/{name} must exist ({err})"))
}

/// Path to the committed `ts-basic` fixture.
pub fn fixture_root() -> PathBuf {
    named_fixture_root("ts-basic")
}

/// Recursively copy a tree, skipping any existing `.nerve` index.
pub fn copy_tree(source: &Path, destination: &Path) {
    std::fs::create_dir_all(destination).unwrap();
    let mut entries: Vec<_> = std::fs::read_dir(source)
        .unwrap()
        .map(|entry| entry.unwrap())
        .collect();
    entries.sort_by_key(|entry| entry.file_name());
    for entry in entries {
        let name = entry.file_name();
        if name == ".nerve" {
            continue;
        }
        let from = entry.path();
        let to = destination.join(&name);
        if from.is_dir() {
            copy_tree(&from, &to);
        } else {
            std::fs::copy(&from, &to).unwrap();
        }
    }
}

/// Copy the fixture into `<temp>/repo` and return the temp directory plus that root.
///
/// The copy always lands in a directory named `repo` so that nothing observable depends on
/// the randomly generated temporary directory name.
pub fn fixture_copy() -> (tempfile::TempDir, PathBuf) {
    named_fixture_copy("ts-basic")
}

/// Copy a named fixture into `<temp>/repo`. The committed fixture is never mutated.
pub fn named_fixture_copy(name: &str) -> (tempfile::TempDir, PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("repo");
    copy_tree(&named_fixture_root(name), &root);
    (dir, root)
}

/// Copy the fixture, initialize with the fixed project id, and index it.
pub fn indexed_fixture() -> (tempfile::TempDir, PathBuf) {
    indexed_named_fixture("ts-basic").0
}

/// Copy, initialize and index a named fixture, returning the run outcome too.
pub fn indexed_named_fixture(
    name: &str,
) -> ((tempfile::TempDir, PathBuf), nerve_index::IndexOutcome) {
    let (dir, root) = named_fixture_copy(name);
    nerve_index::init_with_project_id(&root, Some(TEST_PROJECT_ID)).unwrap();
    let outcome = nerve_index::index_repository(&root).unwrap();
    ((dir, root), outcome)
}

/// Open the database for an indexed root.
pub fn open_db(root: &Path) -> nerve_store::Connection {
    nerve_store::open(&nerve_index::config::db_path(root)).unwrap()
}

/// Parse a committed JSON file, naming it in the panic so a missing fixture is not a mystery.
pub fn read_json(path: &Path) -> serde_json::Value {
    let text = std::fs::read_to_string(path)
        .unwrap_or_else(|err| panic!("{} must be readable: {err}", path.display()));
    serde_json::from_str(&text)
        .unwrap_or_else(|err| panic!("{} must be valid JSON: {err}", path.display()))
}

/// What Git says a `fixtures/history-*` fixture contains.
pub fn history_inventory(name: &str) -> serde_json::Value {
    read_json(&named_fixture_root(name).join("inventory.json"))
}

/// Copy a `fixtures/history-*` fixture's `gitdir/` into a temporary tree as `.git`, and initialize.
///
/// The committed directory is **not** called `.git`, because Git will not track files inside a nested
/// `.git` — so the rename is the fixture's contract rather than a convenience. `init` runs because
/// history is written into `.nerve/nerve.db`; it does **not** index, because history resolves nothing
/// against the graph.
pub fn history_fixture(name: &str) -> (tempfile::TempDir, PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("repo");
    std::fs::create_dir_all(&root).unwrap();
    copy_tree(&named_fixture_root(name).join("gitdir"), &root.join(".git"));
    nerve_index::init_with_project_id(&root, Some(TEST_PROJECT_ID)).unwrap();
    (dir, root)
}

/// Adler-32, as the zlib wrapper requires it.
fn adler32(data: &[u8]) -> u32 {
    let mut low = 1u32;
    let mut high = 0u32;
    for byte in data {
        low = (low + u32::from(*byte)) % 65_521;
        high = (high + low) % 65_521;
    }
    (high << 16) | low
}

/// A zlib stream built from **stored** deflate blocks only.
///
/// Test-only, and it exists so a hostile or over-large Git object can be constructed as *real bytes*
/// without adding a compressor to anything. Stored blocks are the deflate format's escape hatch — a
/// one-byte block header, then `LEN`, then `!LEN`, then the bytes verbatim — so this is a legal zlib
/// stream that `flate2` inflates like any other, checksum included.
pub fn zlib_stored(payload: &[u8]) -> Vec<u8> {
    // CMF=0x78 (deflate, 32 KiB window), FLG=0x01: 0x7801 is divisible by 31, as the header check
    // requires.
    let mut out = vec![0x78, 0x01];
    let chunks: Vec<&[u8]> = if payload.is_empty() {
        vec![&[][..]]
    } else {
        payload.chunks(0xffff).collect()
    };
    for (index, chunk) in chunks.iter().enumerate() {
        let final_block = index + 1 == chunks.len();
        out.push(u8::from(final_block));
        let length = u16::try_from(chunk.len()).expect("chunks are at most 0xffff");
        out.extend_from_slice(&length.to_le_bytes());
        out.extend_from_slice(&(!length).to_le_bytes());
        out.extend_from_slice(chunk);
    }
    out.extend_from_slice(&adler32(payload).to_be_bytes());
    out
}

/// Write a loose Git object at `oid`, whatever its content.
///
/// **The reader deliberately does not verify content against its object id** — a stated non-check,
/// because verifying would mean adding SHA-1 to detect corruption `git fsck` exists for. That is what
/// makes a synthetic repository possible here without a hash implementation: the oid is a name, and
/// these tests choose it. Every synthetic case says so where it is used.
pub fn write_loose_object(git_dir: &Path, oid: &str, kind: &str, payload: &[u8]) {
    let mut raw = format!("{kind} {}\0", payload.len()).into_bytes();
    raw.extend_from_slice(payload);
    let path = git_dir.join("objects").join(&oid[..2]).join(&oid[2..]);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, zlib_stored(&raw)).unwrap();
}

/// Serialize tree entries as Git does: `<octal mode> <name>\0<20 raw oid bytes>`, repeated.
pub fn tree_object(entries: &[(&str, &[u8], &str)]) -> Vec<u8> {
    let mut out = Vec::new();
    for (mode, name, oid) in entries {
        out.extend_from_slice(mode.as_bytes());
        out.push(b' ');
        out.extend_from_slice(name);
        out.push(0);
        for index in 0..20 {
            let byte = u8::from_str_radix(&oid[index * 2..index * 2 + 2], 16)
                .unwrap_or_else(|_| panic!("{oid} must be 40 hex characters"));
            out.push(byte);
        }
    }
    out
}

/// Serialize a commit object with a fixed synthetic identity and clock.
pub fn commit_object(tree: &str, parents: &[&str], summary: &str) -> Vec<u8> {
    let mut text = format!("tree {tree}\n");
    for parent in parents {
        text.push_str(&format!("parent {parent}\n"));
    }
    text.push_str("author Nerve Test <test@nerve.invalid> 1767225600 +0000\n");
    text.push_str("committer Nerve Test <test@nerve.invalid> 1767225600 +0000\n");
    text.push_str(&format!("\n{summary}\n"));
    text.into_bytes()
}

/// An empty, initialized repository whose `.git` these tests fill by hand.
///
/// Used only for the cases no committed fixture can hold: a bound that needs 100 001 tree entries, a
/// tree nested past the depth bound, and the entry names the format reader lets through so the path
/// guard has something to refuse.
pub fn synthetic_repo() -> (tempfile::TempDir, PathBuf, PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("repo");
    let git_dir = root.join(".git");
    std::fs::create_dir_all(git_dir.join("objects")).unwrap();
    std::fs::create_dir_all(git_dir.join("refs/heads")).unwrap();
    std::fs::write(git_dir.join("HEAD"), "ref: refs/heads/main\n").unwrap();
    nerve_index::init_with_project_id(&root, Some(TEST_PROJECT_ID)).unwrap();
    (dir, root, git_dir)
}

/// Point `refs/heads/main` at `oid`.
pub fn set_main(git_dir: &Path, oid: &str) {
    std::fs::write(git_dir.join("refs/heads/main"), format!("{oid}\n")).unwrap();
}

/// Count rows in a table.
pub fn count(conn: &nerve_store::Connection, table: &str) -> i64 {
    conn.query_row(&format!("SELECT count(*) FROM {table}"), [], |row| {
        row.get(0)
    })
    .unwrap()
}
