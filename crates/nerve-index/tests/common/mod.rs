//! Shared helpers for the integration tests.
//!
//! Each integration test binary compiles this module independently and uses only part of it,
//! so unused-code warnings here are an artefact of the test layout, not real dead code.

#![allow(dead_code)]

use std::path::{Path, PathBuf};

/// Project id used by every test that compares ids, so that comparisons are meaningful.
pub const TEST_PROJECT_ID: &str = "00000000000000000000000000000001";

/// Path to the committed `ts-basic` fixture.
pub fn fixture_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures/ts-basic")
        .canonicalize()
        .expect("fixtures/ts-basic must exist")
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
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("repo");
    copy_tree(&fixture_root(), &root);
    (dir, root)
}

/// Copy the fixture, initialize with the fixed project id, and index it.
pub fn indexed_fixture() -> (tempfile::TempDir, PathBuf) {
    let (dir, root) = fixture_copy();
    nerve_index::init_with_project_id(&root, Some(TEST_PROJECT_ID)).unwrap();
    nerve_index::index_repository(&root).unwrap();
    (dir, root)
}

/// Open the database for an indexed root.
pub fn open_db(root: &Path) -> nerve_store::Connection {
    nerve_store::open(&nerve_index::config::db_path(root)).unwrap()
}

/// Count rows in a table.
pub fn count(conn: &nerve_store::Connection, table: &str) -> i64 {
    conn.query_row(&format!("SELECT count(*) FROM {table}"), [], |row| {
        row.get(0)
    })
    .unwrap()
}
