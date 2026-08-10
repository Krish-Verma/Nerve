//! The second-repository trust boundary (Slice 13a-ii), attacked rather than described.
//!
//! `docs/THREAT-MODEL.md` T12 lists seven controls. Six of them are asserted here and the seventh —
//! *no user-specific absolute path is tracked by Git* — is in
//! `crates/nerve-cli/tests/registry_guards.rs`, because it is a scan over the whole repository
//! rather than a property of one function.
//!
//! Two habits run through the file.
//!
//! **Every fixture is two repositories with two different `project_id`s.** `repo_id` derives from
//! `project_id` (`nerve_core::ids::repository_id`), so a test that initialised both halves with the
//! shared `TEST_PROJECT_ID` would give them the same identity — and every identity assertion below
//! would then pass for the wrong reason, including the one that proves a swapped checkout is
//! detected.
//!
//! **A negative is never asserted alone.** *"No sibling was registered"* is satisfied by a registry
//! that cannot register anything, so the sibling test registers a neighbour explicitly in the same
//! breath; *"the bytes did not change"* is satisfied by a read that read nothing, so the byte tests
//! assert the read produced an answer first.

mod common;

use std::path::{Path, PathBuf};

use common::copy_tree;
use nerve_core::vocab::{ContractFreshness, RegistryEntryStatus};
use nerve_index::registry::{
    add_registry_target, availability_of, list_registry, probe_target, relocate_registry_target,
    remove_registry_target, target_database_path, RegistryAvailability, RegistryOutcome,
    RegistryRefusal,
};

/// A distinct project id per repository, so two checkouts never share a `repo_id`.
fn project_id(seed: u8) -> String {
    format!("{:032x}", 0xa0000000u64 + u64::from(seed))
}

/// Copy a fixture to `<base>/<name>`, `nerve init` it with its own project id, and index it.
fn repository(base: &Path, name: &str, fixture: &str, seed: u8, index: bool) -> PathBuf {
    let root = base.join(name);
    copy_tree(&common::named_fixture_root(fixture), &root);
    nerve_index::init_with_project_id(&root, Some(&project_id(seed))).unwrap();
    if index {
        nerve_index::index_repository(&root).unwrap();
    }
    root
}

/// A temporary directory holding `a` (the registry's owner) and `b` (its neighbour), both indexed.
fn two_repositories() -> (tempfile::TempDir, PathBuf, PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    let a = repository(dir.path(), "a", "ts-basic", 1, true);
    let b = repository(dir.path(), "b", "ts-resolution", 2, true);
    (dir, a, b)
}

fn repo_id_of(root: &Path) -> String {
    let conn = nerve_store::open(&nerve_index::config::db_path(root)).unwrap();
    nerve_store::repository(&conn).unwrap().unwrap().repo_id
}

/// BLAKE3 of a file, so "unchanged" is a hash comparison rather than a length comparison.
fn digest(path: &Path) -> String {
    let bytes = std::fs::read(path).unwrap_or_else(|err| panic!("{}: {err}", path.display()));
    nerve_core::ids::content_hash(&bytes)
}

fn conn_for(root: &Path) -> nerve_store::Connection {
    nerve_store::open(&nerve_index::config::db_path(root)).unwrap()
}

// ---- control 1: registration is explicit -------------------------------------------------------

/// **T12 control 1.** A sibling checkout beside a registered one is never registered on its own.
///
/// The positive half is in the same test on purpose: a registry that registered *nothing* would
/// satisfy the negative for free, which is the trap this project has recorded twice.
#[test]
fn a_sibling_checkout_is_never_discovered_and_only_a_named_one_is_registered() {
    let dir = tempfile::tempdir().unwrap();
    let a = repository(dir.path(), "a", "ts-basic", 1, true);
    let b = repository(dir.path(), "b", "ts-resolution", 2, true);
    // A third checkout, sitting in the same parent directory and never named by any command.
    let sibling = repository(dir.path(), "sibling", "ts-resolution", 3, true);

    let conn = conn_for(&a);
    let repo_id = repo_id_of(&a);

    // Before anything is registered, the registry is empty even though two neighbours are adjacent.
    assert!(list_registry(&conn, &repo_id).unwrap().is_empty());

    // Registering one does not register the other.
    let outcome = add_registry_target(&conn, &repo_id, &b, None, None).unwrap();
    assert!(matches!(outcome, RegistryOutcome::Done(_)), "{outcome:?}");

    let listed = list_registry(&conn, &repo_id).unwrap();
    assert_eq!(listed.len(), 1, "exactly the neighbour that was named");
    assert_eq!(
        listed[0].entry.expected_repository_id,
        repo_id_of(&b),
        "and it is that neighbour rather than another"
    );
    assert!(
        listed
            .iter()
            .all(|view| view.entry.expected_repository_id != repo_id_of(&sibling)),
        "the sibling checkout was registered without anybody naming it"
    );
}

// ---- control 2: the target opens read-only and its bytes do not move ---------------------------

/// **T12 control 2.** Every read of a neighbour leaves its database byte-identical.
///
/// The anti-vacuity half is the `assert` on the probe's contents: a read that failed would leave the
/// bytes alone too, and would prove nothing.
#[test]
fn every_read_of_a_neighbour_leaves_its_database_byte_identical() {
    let (_dir, a, b) = two_repositories();
    let target_db = target_database_path(&b);
    let before = digest(&target_db);

    let target = probe_target(&b).expect("the neighbour must be readable");
    assert_eq!(target.repository_id, repo_id_of(&b));
    assert!(
        target.entities_total > 0,
        "the probe must have read something"
    );
    assert!(target.state_id.is_some());

    let conn = conn_for(&a);
    let repo_id = repo_id_of(&a);
    add_registry_target(&conn, &repo_id, &b, None, None).unwrap();
    let listed = list_registry(&conn, &repo_id).unwrap();
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].availability, RegistryAvailability::Available);

    // And again, several times, because a per-read side effect would be invisible in one pass.
    for _ in 0..3 {
        let _ = availability_of(&listed[0].entry);
    }

    assert_eq!(
        digest(&target_db),
        before,
        "a read of the neighbour changed its database"
    );
}

/// The connection Nerve opens on a neighbour refuses a write, at SQLite rather than at review.
#[test]
fn the_connection_opened_on_a_neighbour_is_query_only_and_refuses_a_write() {
    let (_dir, _a, b) = two_repositories();
    let target_db = target_database_path(&b);
    let before = digest(&target_db);
    {
        let conn = nerve_store::open_read_only(&target_db).unwrap();

        let query_only: i64 = conn
            .query_row("PRAGMA query_only", [], |row| row.get(0))
            .unwrap();
        assert_eq!(query_only, 1, "the neighbour is opened query_only");

        // Anti-vacuity: the connection can read, so the refusals below are about writing.
        let entities: i64 = conn
            .query_row("SELECT count(*) FROM entity", [], |row| row.get(0))
            .unwrap();
        assert!(entities > 0);

        for write in [
            "DELETE FROM entity",
            "INSERT INTO repository VALUES ('x','y','z','t')",
            "UPDATE repository SET project_id = 'hijacked'",
        ] {
            assert!(
                conn.execute_batch(write).is_err(),
                "a write into a neighbour's database must be refused: {write}"
            );
        }
        let still_there: i64 = conn
            .query_row("SELECT count(*) FROM entity", [], |row| row.get(0))
            .unwrap();
        assert_eq!(still_there, entities);
    }
    assert_eq!(
        digest(&target_db),
        before,
        "opening a neighbour changed its database file"
    );
}

// ---- control 3: the row is untrusted input, re-validated at every use --------------------------

/// **T12 control 3.** A checkout swapped underneath a registered entry is detected on the next use.
///
/// This is the whole reason identity is compared against the recorded repository id rather than
/// against the path: the path is identical before and after, and only the id changes.
#[test]
fn a_checkout_swapped_underneath_an_entry_is_reported_as_moved_and_not_as_available() {
    let (dir, a, b) = two_repositories();
    let conn = conn_for(&a);
    let repo_id = repo_id_of(&a);
    let expected = repo_id_of(&b);

    add_registry_target(&conn, &repo_id, &b, Some("neighbour"), None).unwrap();
    assert_eq!(
        list_registry(&conn, &repo_id).unwrap()[0].availability,
        RegistryAvailability::Available
    );

    // The path stays the same; what is at it does not.
    std::fs::remove_dir_all(&b).unwrap();
    let replacement = repository(dir.path(), "b", "ts-basic", 9, true);
    assert_eq!(replacement, b, "the swap must reuse the registered path");
    let intruder = repo_id_of(&b);
    assert_ne!(intruder, expected, "the swap must change the identity");

    let listed = list_registry(&conn, &repo_id).unwrap();
    assert_eq!(
        listed[0].availability,
        RegistryAvailability::Moved {
            observed_repository_id: intruder.clone()
        }
    );
    assert_eq!(
        listed[0].availability.freshness(),
        Some(ContractFreshness::TargetRepositoryMoved)
    );
    // The row still records the repository that was registered, not the one that turned up.
    assert_eq!(listed[0].entry.expected_repository_id, expected);
}

// ---- control 4: the database, and nothing else -------------------------------------------------

/// **T12 control 4.** Registering a neighbour does not index it, and modifies no file inside it.
///
/// Asserted over the whole tree rather than as a spot check on the database, because "Nerve wrote
/// something over there" includes a stray lock file, a log line or a cache entry.
///
/// **Two sidecars are allowed, named, and nothing else is.** A Nerve index runs in WAL mode, and a
/// read-only connection to a WAL database makes SQLite create `nerve.db-shm` and a zero-length
/// `nerve.db-wal` if they are absent — a read-only connection cannot delete them again either. That
/// is measured, it is the reason `crates/nerve-index/src/registry.rs` explains why `immutable=1` was
/// not taken, and it is stated in `docs/THREAT-MODEL.md` T12. This test pins the residual to exactly
/// those two paths so that a third one, or any change to an existing file, fails.
#[test]
fn registering_a_neighbour_indexes_nothing_and_modifies_no_file_inside_it() {
    let (_dir, a, b) = two_repositories();
    let before = tree_digest(&b);
    let target_counts = index_counts(&b);
    assert!(
        target_counts.0 > 0 && target_counts.1 > 0,
        "{target_counts:?}"
    );

    let conn = conn_for(&a);
    let repo_id = repo_id_of(&a);
    let outcome = add_registry_target(&conn, &repo_id, &b, None, None).unwrap();
    assert!(matches!(outcome, RegistryOutcome::Done(_)));
    let listed = list_registry(&conn, &repo_id).unwrap();
    assert_eq!(listed.len(), 1, "the read must have produced an answer");

    let after = tree_digest(&b);
    for (path, hash) in &before {
        assert_eq!(
            after.iter().find(|(name, _)| name == path).map(|(_, h)| h),
            Some(hash),
            "{path} was changed or removed inside the neighbour"
        );
    }
    let appeared: Vec<&String> = after
        .iter()
        .map(|(name, _)| name)
        .filter(|name| !before.iter().any(|(existing, _)| existing == *name))
        .collect();
    for name in &appeared {
        assert!(
            name.as_str() == ".nerve/nerve.db-wal" || name.as_str() == ".nerve/nerve.db-shm",
            "reading a neighbour created {name} inside it"
        );
    }

    // The literal control: no extractor run, no new entity. Nerve did not index the target.
    assert_eq!(index_counts(&b), target_counts);
}

/// `(entities, extractor runs)` in a repository's own index.
fn index_counts(root: &Path) -> (i64, i64) {
    let conn = nerve_store::open_read_only(&target_database_path(root)).unwrap();
    (
        conn.query_row("SELECT count(*) FROM entity", [], |row| row.get(0))
            .unwrap(),
        conn.query_row("SELECT count(*) FROM extractor_run", [], |row| row.get(0))
            .unwrap(),
    )
}

/// Every file under `root`, as `(relative path, content hash)`, sorted.
fn tree_digest(root: &Path) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        for entry in std::fs::read_dir(&dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                stack.push(path);
            } else {
                out.push((
                    path.strip_prefix(root)
                        .unwrap()
                        .to_string_lossy()
                        .into_owned(),
                    digest(&path),
                ));
            }
        }
    }
    out.sort();
    assert!(!out.is_empty(), "the tree walk must find files");
    out
}

// ---- control 5: everything read out is untrusted repository content ----------------------------

/// **T12 control 5.** A hostile directory name is stored verbatim and interpreted never.
///
/// Storing it raw is the correct half of T7: neutralising meaning at the store would make the
/// database disagree with the disk. Rendering it inert is the surface's job and
/// `crates/nerve-cli/tests/registry.rs` asserts that end.
#[test]
fn a_hostile_directory_name_is_stored_verbatim_and_never_interpreted() {
    let dir = tempfile::tempdir().unwrap();
    let a = repository(dir.path(), "a", "ts-basic", 1, true);
    let hostile = "b\u{1b}[2K\nshallow        false";
    let b = repository(dir.path(), hostile, "ts-resolution", 2, true);

    let target = probe_target(&b).expect("a hostile name is not a refusal");
    assert_eq!(target.default_display_name, hostile);

    let conn = conn_for(&a);
    let repo_id = repo_id_of(&a);
    // The default *id* is reduced to a local identifier — it is a key and an argument — while the
    // default *name* is not, because a name is content and reducing it would misquote the disk.
    let outcome = add_registry_target(&conn, &repo_id, &b, None, None).unwrap();
    let RegistryOutcome::Done(entry) = outcome else {
        panic!("{outcome:?}");
    };
    assert_eq!(entry.display_name, hostile);
    assert!(
        !entry.registry_id.contains('\n') && !entry.registry_id.contains('\u{1b}'),
        "the derived id carried a control character: {:?}",
        entry.registry_id
    );
}

// ---- control 6: a symlink out of the user's control is refused, not followed -------------------

/// **T12 control 6, half one.** The registered path itself is a symlink.
#[test]
fn a_registered_path_that_is_a_symlink_is_refused_rather_than_followed() {
    let (dir, _a, b) = two_repositories();
    let link = dir.path().join("link-to-b");
    std::os::unix::fs::symlink(&b, &link).unwrap();

    assert_eq!(probe_target(&link), Err(RegistryRefusal::SymlinkEscape));
    // The control: the same repository through its real path is not refused, so the guard is
    // deciding rather than refusing everything.
    assert!(probe_target(&b).is_ok());
}

/// **T12 control 6, half two.** The `.nerve` directory is a symlink leading out of the root.
///
/// This is the escape that matters: the registered path is an ordinary directory, and the database
/// Nerve would open belongs to somebody else entirely.
#[test]
fn a_nerve_directory_symlinked_out_of_the_target_root_is_refused() {
    let (dir, _a, b) = two_repositories();
    let decoy = dir.path().join("decoy");
    let decoy_root = repository(&decoy, "elsewhere", "ts-basic", 7, true);

    let hollow = dir.path().join("hollow");
    std::fs::create_dir_all(&hollow).unwrap();
    std::os::unix::fs::symlink(decoy_root.join(".nerve"), hollow.join(".nerve")).unwrap();

    assert_eq!(probe_target(&hollow), Err(RegistryRefusal::SymlinkEscape));
    // And the decoy is readable through its own path, so the refusal is about the escape and not
    // about the decoy being unreadable.
    assert!(probe_target(&decoy_root).is_ok());
    assert!(probe_target(&b).is_ok());
}

/// A directory with no `.nerve/nerve.db` is *missing an index*, not a symlink escape.
///
/// The two arrive at `canonical_child` as the same error, because it cannot canonicalize a path that
/// does not exist. Reporting the first as the second would be a security control claiming a hit it
/// never got.
#[test]
fn a_directory_with_no_index_is_refused_as_an_absent_index_and_not_as_an_escape() {
    let dir = tempfile::tempdir().unwrap();
    let bare = dir.path().join("bare");
    std::fs::create_dir_all(bare.join("src")).unwrap();
    assert_eq!(probe_target(&bare), Err(RegistryRefusal::NoNerveDatabase));
}

// ---- the remaining named refusals --------------------------------------------------------------

/// Each refusal `nerve repo add` can produce, produced.
#[test]
fn every_registration_refusal_has_its_own_reason() {
    let (dir, a, b) = two_repositories();
    let conn = conn_for(&a);
    let repo_id = repo_id_of(&a);

    let refuse =
        |path: &Path, id: Option<&str>| match add_registry_target(&conn, &repo_id, path, id, None)
            .unwrap()
        {
            RegistryOutcome::Refused(reason) => reason,
            other => panic!("expected a refusal, got {other:?}"),
        };

    assert_eq!(
        refuse(&dir.path().join("nowhere"), None),
        RegistryRefusal::PathDoesNotExist
    );

    let file = dir.path().join("a-file");
    std::fs::write(&file, b"not a repository").unwrap();
    assert_eq!(refuse(&file, None), RegistryRefusal::PathIsNotADirectory);

    assert_eq!(refuse(&a, None), RegistryRefusal::SameRepository);

    // Registered once, then twice — under the same name and under a different one, because they are
    // different mistakes and both must be refused.
    assert!(matches!(
        add_registry_target(&conn, &repo_id, &b, Some("neighbour"), None).unwrap(),
        RegistryOutcome::Done(_)
    ));
    assert_eq!(
        refuse(&b, Some("neighbour")),
        RegistryRefusal::AlreadyRegistered
    );
    assert_eq!(
        refuse(&b, Some("another-name")),
        RegistryRefusal::AlreadyRegistered
    );

    // A directory whose name reduces to no usable identifier, with no `--id` to fall back on. The
    // id is refused rather than generated: it is a key the user will type back at Nerve later, and
    // inventing one would be inventing the argument.
    let unnameable = repository(dir.path(), "++", "ts-resolution", 4, true);
    assert!(
        unnameable.is_dir(),
        "the awkwardly named fixture must have been created"
    );
    assert_eq!(
        refuse(&unnameable, None),
        RegistryRefusal::UnusableRegistryId
    );
    // And an unusable id given explicitly is refused too, rather than falling back to the default.
    assert_eq!(
        refuse(&unnameable, Some("  ")),
        RegistryRefusal::UnusableRegistryId
    );
}

/// A neighbour whose database records a schema version this build cannot support is refused.
///
/// Refused rather than migrated: migrating is a write, and Nerve has never written into a repository
/// it was not pointed at.
#[test]
fn a_neighbour_whose_schema_is_newer_than_this_build_is_refused_rather_than_migrated() {
    let (_dir, _a, b) = two_repositories();
    let target_db = target_database_path(&b);
    {
        let conn = nerve_store::open(&target_db).unwrap();
        conn.execute(
            "INSERT INTO schema_version (version, applied_at, description)
             VALUES (?1, 't', 'from a future build')",
            [nerve_store::SCHEMA_VERSION + 1],
        )
        .unwrap();
    }
    assert_eq!(probe_target(&b), Err(RegistryRefusal::TargetSchemaTooNew));
}

// ---- relocate: the check that makes it not a re-pointing ---------------------------------------

/// Relocation refuses a path holding a different repository, and accepts the recorded one.
///
/// Both halves in one test, because the refusal alone is satisfied by a relocate that never works.
#[test]
fn relocation_verifies_the_recorded_identity_before_accepting_a_new_path() {
    let (dir, a, b) = two_repositories();
    let elsewhere = repository(dir.path(), "elsewhere", "ts-resolution", 5, true);
    let conn = conn_for(&a);
    let repo_id = repo_id_of(&a);
    add_registry_target(&conn, &repo_id, &b, Some("neighbour"), None).unwrap();

    // A different repository at the new path is refused with the reason named.
    assert_eq!(
        relocate_registry_target(&conn, &repo_id, "neighbour", &elsewhere).unwrap(),
        RegistryOutcome::Refused(RegistryRefusal::TargetRepositoryMoved)
    );
    assert_eq!(
        nerve_store::registry_entry(&conn, &repo_id, "neighbour")
            .unwrap()
            .unwrap()
            .local_path,
        b.canonicalize().unwrap().to_string_lossy(),
        "a refused relocation must not have moved the entry"
    );

    // The same repository, genuinely moved, is accepted.
    let moved = dir.path().join("b-moved");
    std::fs::rename(&b, &moved).unwrap();
    let outcome = relocate_registry_target(&conn, &repo_id, "neighbour", &moved).unwrap();
    let RegistryOutcome::Done(entry) = outcome else {
        panic!("{outcome:?}");
    };
    assert_eq!(
        entry.local_path,
        moved.canonicalize().unwrap().to_string_lossy()
    );
    assert_eq!(availability_of(&entry), RegistryAvailability::Available);

    // An unknown id and a retired one are two different refusals.
    assert_eq!(
        relocate_registry_target(&conn, &repo_id, "never-registered", &moved).unwrap(),
        RegistryOutcome::Refused(RegistryRefusal::NoSuchRegistryEntry)
    );
    remove_registry_target(&conn, &repo_id, "neighbour").unwrap();
    assert_eq!(
        relocate_registry_target(&conn, &repo_id, "neighbour", &moved).unwrap(),
        RegistryOutcome::Refused(RegistryRefusal::RegistryEntryTombstoned)
    );
}

// ---- remove: a tombstone, and the links that rested on it --------------------------------------

/// Removing an entry retires it, withdraws its links, and deletes nothing.
#[test]
fn removing_an_entry_retires_it_and_withdraws_its_links_without_deleting_a_row() {
    let (_dir, a, b) = two_repositories();
    let conn = conn_for(&a);
    let repo_id = repo_id_of(&a);
    add_registry_target(&conn, &repo_id, &b, Some("neighbour"), None).unwrap();

    // One link resting on the entry, so "its links are withdrawn" is not vacuous.
    let state_id: String = conn
        .query_row(
            "SELECT state_id FROM repository_state WHERE repo_id = ?1 LIMIT 1",
            [&repo_id],
            |row| row.get(0),
        )
        .unwrap();
    nerve_store::insert_contract_link(
        &conn,
        &repo_id,
        &nerve_store::ContractLinkRow {
            link_id: None,
            source_repository_id: repo_id.clone(),
            source_state_at_resolution: state_id,
            source_entity_id: None,
            source_kind_snapshot: None,
            source_path: "package.json".into(),
            source_span: "1:1".into(),
            registry_entry_id: "neighbour".into(),
            expected_target_repository_id: repo_id_of(&b),
            target_state_at_resolution: None,
            target_entity_id: None,
            target_kind_snapshot: None,
            target_name_snapshot: None,
            target_path_snapshot: None,
            target_span_snapshot: None,
            relation_semantics: "REFERENCES".into(),
            contract_kind: "npm_local_dependency".into(),
            contract_identity: "pkg-b".into(),
            expected_contract_version: None,
            observed_contract_version: None,
            resolution_method: nerve_core::vocab::ContractResolutionMethod::ManifestDeclared,
            extractor_id: "test".into(),
            extractor_version: "1.0.0".into(),
            evidence_details: None,
            ambiguity: None,
            unsupported_reason: None,
            first_seen_at: String::new(),
            last_seen_at: String::new(),
            withdrawn_at: None,
            status: nerve_core::vocab::ContractLinkStatus::Active,
        },
    )
    .unwrap();

    let outcome = remove_registry_target(&conn, &repo_id, "neighbour").unwrap();
    let RegistryOutcome::Done(entry) = outcome else {
        panic!("{outcome:?}");
    };
    assert_eq!(entry.status, RegistryEntryStatus::Tombstoned);
    assert!(entry.withdrawn_at.is_some());

    // The row is still there, and still listed.
    let listed = list_registry(&conn, &repo_id).unwrap();
    assert_eq!(listed.len(), 1, "a retired entry stays listed");
    assert_eq!(listed[0].availability, RegistryAvailability::EntryRemoved);
    assert_eq!(
        listed[0].availability.freshness(),
        Some(ContractFreshness::RegistryEntryRemoved)
    );

    // And so is its link, withdrawn rather than gone.
    let links =
        nerve_store::contract_links_for_registry_entry(&conn, &repo_id, "neighbour").unwrap();
    assert_eq!(links.len(), 1, "a withdrawn link is kept");
    assert_eq!(
        links[0].status,
        nerve_core::vocab::ContractLinkStatus::Withdrawn
    );
    assert!(links[0].withdrawn_at.is_some());

    // Removing twice is refused rather than re-dating the ending.
    assert_eq!(
        remove_registry_target(&conn, &repo_id, "neighbour").unwrap(),
        RegistryOutcome::Refused(RegistryRefusal::RegistryEntryTombstoned)
    );
    assert_eq!(
        remove_registry_target(&conn, &repo_id, "never-registered").unwrap(),
        RegistryOutcome::Refused(RegistryRefusal::NoSuchRegistryEntry)
    );
}

// ---- the freshness states this slice can actually produce --------------------------------------

/// **Four of the twelve, each produced from a fixture. The other eight are not claimed.**
///
/// The eight absentees are properties of a *contract link* — a source that moved on, a version that
/// disagrees, a manifest that no longer holds the declaration — and there are no contract links
/// until 13b/13c extracts one. A state that cannot be produced is not required, which is the
/// correction 12c-i-b already had to make when `WalkBudgetExhausted` turned out to be unreachable.
#[test]
fn the_registry_alone_produces_exactly_four_of_the_twelve_freshness_states() {
    let dir = tempfile::tempdir().unwrap();
    let a = repository(dir.path(), "a", "ts-basic", 1, true);
    let indexed = repository(dir.path(), "indexed", "ts-resolution", 2, true);
    let unindexed = repository(dir.path(), "unindexed", "ts-resolution", 3, false);
    let doomed = repository(dir.path(), "doomed", "ts-resolution", 4, true);
    let swapped = repository(dir.path(), "swapped", "ts-resolution", 5, true);
    let retired = repository(dir.path(), "retired", "ts-resolution", 6, true);

    let conn = conn_for(&a);
    let repo_id = repo_id_of(&a);
    for (id, path) in [
        ("indexed", &indexed),
        ("unindexed", &unindexed),
        ("doomed", &doomed),
        ("swapped", &swapped),
        ("retired", &retired),
    ] {
        assert!(matches!(
            add_registry_target(&conn, &repo_id, path, Some(id), None).unwrap(),
            RegistryOutcome::Done(_)
        ));
    }

    // Produce each situation for real.
    std::fs::remove_dir_all(&doomed).unwrap();
    std::fs::remove_dir_all(&swapped).unwrap();
    repository(dir.path(), "swapped", "ts-basic", 7, true);
    remove_registry_target(&conn, &repo_id, "retired").unwrap();

    let mut seen: std::collections::BTreeMap<String, Option<ContractFreshness>> =
        std::collections::BTreeMap::new();
    for view in list_registry(&conn, &repo_id).unwrap() {
        seen.insert(
            view.entry.registry_id.clone(),
            view.availability.freshness(),
        );
    }

    assert_eq!(seen["indexed"], None, "a healthy neighbour is unqualified");
    assert_eq!(
        seen["unindexed"],
        Some(ContractFreshness::TargetPartiallyIndexed)
    );
    assert_eq!(
        seen["doomed"],
        Some(ContractFreshness::TargetRepositoryMissing)
    );
    assert_eq!(
        seen["swapped"],
        Some(ContractFreshness::TargetRepositoryMoved)
    );
    assert_eq!(
        seen["retired"],
        Some(ContractFreshness::RegistryEntryRemoved)
    );

    // `missing` and `moved` are the pair a first draft collapses, and they are distinct here on the
    // same fixture rather than in two tests that could each be right about a different thing.
    assert_ne!(seen["doomed"], seen["swapped"]);
    // And `partially_indexed` is not `target_changed`: nothing was observed to change over there.
    assert_ne!(seen["unindexed"], Some(ContractFreshness::TargetChanged));

    let produced: std::collections::BTreeSet<ContractFreshness> =
        seen.values().flatten().copied().collect();
    assert_eq!(
        produced,
        [
            ContractFreshness::TargetRepositoryMissing,
            ContractFreshness::TargetRepositoryMoved,
            ContractFreshness::TargetPartiallyIndexed,
            ContractFreshness::RegistryEntryRemoved,
        ]
        .into_iter()
        .collect::<std::collections::BTreeSet<_>>()
    );

    // The eight this slice cannot reach are named, so a later slice adding one has to delete it from
    // here rather than quietly widening a set.
    for unreachable in [
        ContractFreshness::SourceChanged,
        ContractFreshness::TargetChanged,
        ContractFreshness::BothChanged,
        ContractFreshness::ContractVersionMismatch,
        ContractFreshness::ContractFileMissing,
        ContractFreshness::DuplicateContractIdentity,
        ContractFreshness::ConflictingDefinitions,
        ContractFreshness::ContractDeleted,
    ] {
        assert!(
            !produced.contains(&unreachable),
            "{unreachable} was claimed without a contract link to carry it"
        );
    }
    assert_eq!(produced.len() + 8, ContractFreshness::ALL.len());
}

/// Availability is stamped when the target is read, and a target with no index says so as an
/// absence rather than as a state nobody observed.
#[test]
fn reading_a_neighbour_records_when_it_was_read_and_what_state_it_was_in() {
    let dir = tempfile::tempdir().unwrap();
    let a = repository(dir.path(), "a", "ts-basic", 1, true);
    let indexed = repository(dir.path(), "indexed", "ts-resolution", 2, true);
    let unindexed = repository(dir.path(), "unindexed", "ts-resolution", 3, false);
    let conn = conn_for(&a);
    let repo_id = repo_id_of(&a);

    add_registry_target(&conn, &repo_id, &indexed, Some("indexed"), None).unwrap();
    add_registry_target(&conn, &repo_id, &unindexed, Some("unindexed"), None).unwrap();

    let rows = nerve_store::list_registry_entries(&conn, &repo_id).unwrap();
    let indexed_row = rows.iter().find(|r| r.registry_id == "indexed").unwrap();
    let unindexed_row = rows.iter().find(|r| r.registry_id == "unindexed").unwrap();

    assert!(indexed_row.availability_checked_at.is_some());
    assert!(indexed_row.last_seen_state.is_some());
    assert!(indexed_row.last_seen_at.is_some());

    // Looked at, and there was nothing to see. The two are different observations and the schema's
    // CHECK requires the pair to move together.
    assert!(unindexed_row.availability_checked_at.is_some());
    assert_eq!(unindexed_row.last_seen_state, None);
    assert_eq!(unindexed_row.last_seen_at, None);
}
