//! Human-confirmed memory against the **real indexer** (schema v9, Slice 14a).
//!
//! The `nerve-store` tests build their fixtures out of literal `INSERT`s, which is what makes them
//! precise — and also what makes them blind to anything the real pipeline does that the literals do
//! not reproduce. Slice 3b learned that the expensive way: the v2 data-destruction bug passed every
//! unit-level test and was only ever caught by one that ran the actual indexer and compared the
//! whole database afterwards.
//!
//! Two properties are asserted here and cannot be asserted anywhere else.
//!
//! 1. **A note survives its subject being pruned by a real re-index.** `prune_orphans` issues
//!    `DELETE FROM entity` on every incremental run, so this is the ordinary path rather than a
//!    contrived one: write a note about a file, delete the file, re-index, and the note is still
//!    readable with subject resolution `missing`. A foreign key would have made this test either
//!    fail at the delete (a human note blocking re-indexing) or fail at the read (a routine
//!    re-index destroying the note).
//! 2. **The machine tables are byte-identical across every memory operation**, hashed with the same
//!    BLAKE3 the rest of the product uses for content identity.

mod common;

use std::path::Path;

use common::{named_fixture_copy, open_db, TEST_PROJECT_ID};

use nerve_core::vocab::{MemoryStatus, MemorySubjectResolution};
use nerve_store::memory::{insert_memory, read_memory, MemoryRow, MemorySubject};
use nerve_store::Connection;

fn index(root: &Path) -> nerve_index::IndexOutcome {
    nerve_index::index_repository(root).unwrap()
}

fn indexed_incremental_fixture() -> (tempfile::TempDir, std::path::PathBuf) {
    let (dir, root) = named_fixture_copy("ts-incremental");
    nerve_index::init_with_project_id(&root, Some(TEST_PROJECT_ID)).unwrap();
    nerve_index::index_repository(&root).unwrap();
    (dir, root)
}

fn scalar(conn: &Connection, sql: &str) -> i64 {
    conn.query_row(sql, [], |row| row.get(0)).unwrap()
}

fn repo_id(conn: &Connection) -> String {
    nerve_store::repository(conn)
        .unwrap()
        .expect("a repository")
        .repo_id
}

/// The `entity_id` and the current `state_id` of a file the indexer really produced.
fn file_subject(conn: &Connection, path: &str) -> (String, String) {
    let entity_id = conn
        .query_row(
            "SELECT e.entity_id
               FROM entity e
               JOIN occurrence o ON o.entity_id = e.entity_id
              WHERE e.kind = 'file' AND o.file_path = ?1",
            [path],
            |row| row.get::<_, String>(0),
        )
        .unwrap_or_else(|error| panic!("no file entity for {path}: {error}"));
    let state_id = conn
        .query_row(
            "SELECT state_id FROM extractor_run ORDER BY run_id DESC LIMIT 1",
            [],
            |row| row.get::<_, String>(0),
        )
        .unwrap();
    (entity_id, state_id)
}

fn note(memory_id: &str, entity_id: &str, state_id: &str, path: &str, content: &str) -> MemoryRow {
    MemoryRow {
        memory_id: memory_id.to_string(),
        subject: MemorySubject {
            entity_id: entity_id.to_string(),
            kind: "file".to_string(),
            name: path.rsplit('/').next().unwrap_or(path).to_string(),
            path: path.to_string(),
            selector: format!("file:{path}"),
        },
        anchor_state_id: state_id.to_string(),
        scope: "file".to_string(),
        claim_key: None,
        content: content.to_string(),
        author_label: "krish".to_string(),
        created_at: String::new(),
        status: MemoryStatus::Active,
        supersedes_memory_id: None,
        invalidated_at: None,
        invalidation_reason: None,
    }
}

/// A BLAKE3 digest over every row of the four tables memory may never touch.
///
/// Hashed rather than eyeballed, and hashed over a canonical serialisation so that "unchanged"
/// means every column of every row rather than a count.
fn evidence_digest(conn: &Connection) -> String {
    let mut hasher = blake3::Hasher::new();
    for sql in [
        "SELECT assertion_id || '|' || repo_id || '|' || source_entity_id || '|' || relation
             || '|' || target_entity_id
           FROM assertion ORDER BY assertion_id",
        "SELECT occurrence_id || '|' || entity_id || '|' || file_path || '|' || start_byte
             || '|' || end_byte || '|' || content_hash
           FROM occurrence ORDER BY occurrence_id",
        "SELECT observation_id || '|' || assertion_id || '|' || evidence_source_type || '|'
             || directness || '|' || extractor_id || '|' || extractor_version || '|' || file_path
             || '|' || start_line || '|' || end_line || '|' || content_hash
           FROM observation ORDER BY observation_id",
        "SELECT assertion_id || '|' || status || '|' || strongest_source_type || '|'
             || source_type_mask || '|' || observation_count || '|' || is_unresolved
           FROM assertion_state ORDER BY assertion_id",
    ] {
        let mut stmt = conn.prepare(sql).unwrap();
        let rows = stmt.query_map([], |row| row.get::<_, String>(0)).unwrap();
        for row in rows {
            hasher.update(row.unwrap().as_bytes());
            hasher.update(b"\n");
        }
        hasher.update(b"--\n");
    }
    hasher.finalize().to_hex().to_string()
}

/// **A human's note survives a re-index that prunes its subject, and says `missing` rather than
/// nothing.**
///
/// This is the property row 14 was rewritten for, asserted end to end. `prune_orphans` issues
/// `DELETE FROM entity` (`prune.rs:376`, and again scoped at `:440`) and
/// `deleting_a_file_removes_its_entities_assertions_and_observations` pins that as required
/// behaviour, so with `PRAGMA foreign_keys=ON` a foreign key from `memory` into `entity` would give
/// exactly one of two failures — and this test would fail at a different line for each of them:
///
/// - **refused delete**: `index()` below would return an error, or the entity would still be there,
///   which means a note about a file blocks re-indexing that file;
/// - **cascade**: `read_memory` would return `None`, which means a routine re-index silently
///   destroyed the human's note.
#[test]
fn a_note_survives_the_re_index_that_prunes_its_subject() {
    let (_dir, root) = indexed_incremental_fixture();

    let (entity_id, state_id, repo) = {
        let conn = open_db(&root);
        let (entity_id, state_id) = file_subject(&conn, "src/impl.ts");
        (entity_id, state_id, repo_id(&conn))
    };

    {
        let conn = open_db(&root);
        insert_memory(
            &conn,
            &repo,
            &note(
                "m1",
                &entity_id,
                &state_id,
                "src/impl.ts",
                "the helper here is deliberately not exported",
            ),
        )
        .unwrap();

        // The control: it resolves while the file is still indexed.
        let report = read_memory(&conn, &repo, "m1")
            .unwrap()
            .expect("the record");
        assert_eq!(report.subject.resolution, MemorySubjectResolution::Resolved);
        assert_eq!(
            report.subject.live_entity_ids,
            std::slice::from_ref(&entity_id)
        );
        assert!(report.views.is_empty(), "{:?}", report.views);
    }

    // Delete the file and re-index. This is the ordinary incremental path, and it is what runs
    // `prune_orphans_scoped` over the rows the deletion orphaned.
    std::fs::remove_file(root.join("src/impl.ts")).unwrap();
    let outcome = index(&root);
    assert_eq!(outcome.incremental.files_removed, 1);
    assert!(
        outcome.incremental.entities_removed > 0,
        "the re-index pruned no entity, so the test would prove nothing"
    );

    let conn = open_db(&root);
    assert_eq!(
        scalar(
            &conn,
            &format!("SELECT count(*) FROM entity WHERE entity_id = '{entity_id}'")
        ),
        0,
        "the subject entity survived the prune; a foreign key refused the delete"
    );

    let report = read_memory(&conn, &repo, "m1")
        .unwrap()
        .expect("the note was destroyed by a routine re-index");
    assert_eq!(report.subject.resolution, MemorySubjectResolution::Missing);
    assert!(report.subject.live_entity_ids.is_empty());

    // Still readable, and still able to name what it was written about — which is the entire
    // reason the subject is a snapshot rather than a pointer.
    assert_eq!(
        report.row.content,
        "the helper here is deliberately not exported"
    );
    assert_eq!(report.row.subject.selector, "file:src/impl.ts");
    assert_eq!(report.row.subject.name, "impl.ts");
    assert_eq!(report.row.subject.entity_id, entity_id);

    // The re-index moved the repository on, so the note is also reported as unchecked against it.
    assert!(report
        .views
        .contains(&nerve_core::vocab::MemoryView::PotentiallyStale));
}

/// **A memory operation moves no byte of `assertion`, `observation`, `occurrence` or
/// `assertion_state`**, on a database the real indexer built.
///
/// Hashed rather than inspected. The store-level test asserts the same invariant over a hand-built
/// graph; this one asserts it over the several hundred rows an actual index produces, where a stray
/// trigger, a cascade or an incidental `rebuild_assertion_state` would have something to disturb.
#[test]
fn memory_operations_leave_the_indexer_s_evidence_byte_identical() {
    let (_dir, root) = indexed_incremental_fixture();
    let conn = open_db(&root);
    let repo = repo_id(&conn);
    let (entity_id, state_id) = file_subject(&conn, "src/app.ts");

    let before = evidence_digest(&conn);
    // Anti-vacuity: there is a real graph here, so "unchanged" is a measurement.
    assert!(scalar(&conn, "SELECT count(*) FROM assertion") > 20);
    assert!(scalar(&conn, "SELECT count(*) FROM observation") > 20);
    assert!(scalar(&conn, "SELECT count(*) FROM assertion_state") > 20);

    insert_memory(
        &conn,
        &repo,
        &note("m1", &entity_id, &state_id, "src/app.ts", "entry point"),
    )
    .unwrap();
    let mut successor = note(
        "m2",
        &entity_id,
        &state_id,
        "src/app.ts",
        "entry point, and it is generated",
    );
    successor.supersedes_memory_id = Some("m1".to_string());
    nerve_store::memory::supersede_memory(&conn, &repo, &successor, "supersede", None).unwrap();
    nerve_store::memory::read_memory_all(&conn, &repo).unwrap();

    assert_eq!(
        evidence_digest(&conn),
        before,
        "a memory operation moved the evidence tables"
    );
    assert_eq!(scalar(&conn, "SELECT count(*) FROM memory"), 2);
    assert_eq!(scalar(&conn, "SELECT count(*) FROM memory_event"), 1);
}

/// A re-index that changes nothing leaves the note resolving exactly as it did.
///
/// The complement to the deletion test: an ordinary re-index must not disturb a record, and the
/// anchor it was written against must still be the current state, so nothing is reported stale.
#[test]
fn a_re_index_that_changes_nothing_leaves_a_note_resolved_and_unqualified() {
    let (_dir, root) = indexed_incremental_fixture();
    let (entity_id, state_id, repo) = {
        let conn = open_db(&root);
        let (entity_id, state_id) = file_subject(&conn, "src/app.ts");
        (entity_id, state_id, repo_id(&conn))
    };
    {
        let conn = open_db(&root);
        insert_memory(
            &conn,
            &repo,
            &note("m1", &entity_id, &state_id, "src/app.ts", "entry point"),
        )
        .unwrap();
    }

    index(&root);

    let conn = open_db(&root);
    let report = read_memory(&conn, &repo, "m1")
        .unwrap()
        .expect("the record");
    assert_eq!(report.subject.resolution, MemorySubjectResolution::Resolved);
    assert_eq!(
        report.views,
        Vec::new(),
        "an unchanged tree re-indexed to the same state must not make a note look stale"
    );
    assert_eq!(report.current_state_id.as_deref(), Some(state_id.as_str()));
}
