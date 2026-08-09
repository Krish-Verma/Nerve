//! The cross-repository registry at the storage boundary (schema v8, Slice 13a-i).
//!
//! Three properties are asserted repeatedly here, because they are what the sub-slice is:
//!
//! - **Nothing is ever deleted.** Every retirement test asserts the row is still readable
//!   afterwards *and* that its `registry_id` survives, because `registry_entry_removed` is a report
//!   made from the kept row and a deleted row cannot report its own ending.
//! - **The target end has no foreign key, and that is proved rather than described.** A contract
//!   link names an entity in another database, so emptying this database's `entity` table must
//!   leave the link intact — the whole reason `contract_link` exists instead of an `assertion`.
//! - **Every fixture holds two repositories.** `repo_id` is on both tables, and a read that forgot
//!   to scope by it would still pass against a single-repository fixture.
//!
//! Nothing here touches the filesystem or opens a second database. Registration-time path
//! validation and the read-only open of the target are Slice 13a-ii's trust boundary, and a test
//! in this file that reached for either would be testing a guard that has not been written.

use nerve_core::vocab::{ContractLinkStatus, ContractResolutionMethod, RegistryEntryStatus};
use nerve_store::registry::{
    contract_links_for_registry_entry, insert_contract_link, insert_registry_entry,
    list_contract_links, list_registry_entries, registry_entry, relocate_registry_entry,
    tombstone_registry_entry, withdraw_contract_link, withdraw_links_for_registry_entry,
    ContractLinkRow,
};
use nerve_store::{migrate, open_in_memory, Connection};

/// A migrated database with two repositories and one local entity in each.
///
/// Two repositories, always: both v8 tables carry `repo_id`, and a read that dropped the scope
/// would still pass against a database holding one.
fn store() -> Connection {
    let conn = open_in_memory().unwrap();
    migrate(&conn).unwrap();
    conn.execute_batch(
        "INSERT INTO repository VALUES ('r','p','/tmp/a','t');
         INSERT INTO repository VALUES ('r2','p','/tmp/b','t');
         INSERT INTO repository_state VALUES ('s','r','content',NULL,'m','t');
         INSERT INTO repository_state VALUES ('s2','r2','content',NULL,'m2','t');
         INSERT INTO entity VALUES ('local1','r','file','app.ts','src','typescript',NULL);
         INSERT INTO entity VALUES ('local2','r2','file','other.ts','src','typescript',NULL);",
    )
    .unwrap();
    conn
}

/// A link whose target is a fully snapshotted entity in a repository this database never indexed.
///
/// `target_entity_id` and `target_state_at_resolution` name rows that exist in **no** local table,
/// which is the shape every real cross-repository link has. A fixture that used a local entity id
/// would prove the opposite of what this table is for.
fn link(registry_entry_id: &str, identity: &str, span: &str) -> ContractLinkRow {
    ContractLinkRow {
        link_id: None,
        source_repository_id: "r".to_string(),
        source_state_at_resolution: "s".to_string(),
        source_entity_id: Some("local1".to_string()),
        source_kind_snapshot: Some("file".to_string()),
        source_path: "package.json".to_string(),
        source_span: span.to_string(),
        registry_entry_id: registry_entry_id.to_string(),
        expected_target_repository_id: "repo-b".to_string(),
        target_state_at_resolution: Some("b-state-77".to_string()),
        target_entity_id: Some("b-entity-42".to_string()),
        target_kind_snapshot: Some("file".to_string()),
        target_name_snapshot: Some("sub.ts".to_string()),
        target_path_snapshot: Some("src/sub.ts".to_string()),
        target_span_snapshot: Some("1:40".to_string()),
        relation_semantics: "REFERENCES".to_string(),
        contract_kind: "npm_package_export".to_string(),
        contract_identity: identity.to_string(),
        expected_contract_version: Some("^1.0.0".to_string()),
        observed_contract_version: Some("1.2.0".to_string()),
        resolution_method: ContractResolutionMethod::ExportMapResolved,
        extractor_id: "ts-contract".to_string(),
        extractor_version: "1.0.0".to_string(),
        evidence_details: Some("{}".to_string()),
        ambiguity: None,
        unsupported_reason: None,
        // Overwritten by the writer, which stamps both from one `strftime`.
        first_seen_at: String::new(),
        last_seen_at: String::new(),
        withdrawn_at: None,
        status: ContractLinkStatus::Active,
    }
}

fn scalar(conn: &Connection, sql: &str) -> i64 {
    conn.query_row(sql, [], |row| row.get(0)).unwrap()
}

// ---- registry entries --------------------------------------------------------------------------

/// A registered neighbour comes back exactly as it was written, and only for its own repository.
#[test]
fn a_registered_neighbour_is_readable_and_scoped_to_its_repository() {
    let conn = store();

    let written =
        insert_registry_entry(&conn, "r", "reg1", "repo-b", "Neighbour B", "/checkouts/b").unwrap();
    assert_eq!(written.registry_id, "reg1");
    assert_eq!(written.expected_repository_id, "repo-b");
    assert_eq!(written.display_name, "Neighbour B");
    assert_eq!(written.local_path, "/checkouts/b");
    assert_eq!(written.status, RegistryEntryStatus::Active);
    assert_eq!(written.withdrawn_at, None);
    assert!(
        !written.added_at.is_empty(),
        "added_at is stamped by the writer, not supplied"
    );
    // A new entry has observed nothing yet, and says so as two absences rather than one.
    assert_eq!(written.last_seen_state, None);
    assert_eq!(written.last_seen_at, None);
    assert_eq!(written.availability_checked_at, None);

    // The other repository has its own registry, and registering the *same* id there is a different
    // entry rather than a conflict.
    insert_registry_entry(&conn, "r2", "reg1", "repo-c", "Neighbour C", "/checkouts/c").unwrap();

    assert_eq!(registry_entry(&conn, "r", "reg1").unwrap(), Some(written));
    assert_eq!(
        registry_entry(&conn, "r", "absent").unwrap(),
        None,
        "an unregistered id is None, not an error"
    );
    assert_eq!(
        registry_entry(&conn, "r2", "reg1")
            .unwrap()
            .expect("r2 has its own entry")
            .expected_repository_id,
        "repo-c",
        "the read is not scoped by repository"
    );

    let listed = list_registry_entries(&conn, "r").unwrap();
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].registry_id, "reg1");
    assert_eq!(list_registry_entries(&conn, "r2").unwrap().len(), 1);
}

/// Registering the same id twice is an error, not a silent overwrite and not a silent ignore.
///
/// A second registration of one name is a *different* statement — another path, another expected
/// repository — and quietly keeping either one leaves the user's registry saying something they did
/// not ask for. `INSERT OR IGNORE` would keep the first and `INSERT OR REPLACE` the second; both are
/// a decision made on the caller's behalf without telling it.
#[test]
fn registering_the_same_id_twice_is_refused() {
    let conn = store();
    insert_registry_entry(&conn, "r", "reg1", "repo-b", "B", "/checkouts/b").unwrap();

    let err = insert_registry_entry(&conn, "r", "reg1", "repo-c", "C", "/elsewhere/c").unwrap_err();
    assert!(
        err.to_string().to_lowercase().contains("unique")
            || err.to_string().to_lowercase().contains("constraint"),
        "expected a primary-key refusal, got {err}"
    );

    // The first registration is untouched: the refusal did not half-apply.
    let entry = registry_entry(&conn, "r", "reg1").unwrap().unwrap();
    assert_eq!(entry.expected_repository_id, "repo-b");
    assert_eq!(entry.local_path, "/checkouts/b");
    assert_eq!(scalar(&conn, "SELECT count(*) FROM repo_registry"), 1);
}

/// **Tombstoning keeps the row, and that is what makes `registry_entry_removed` answerable.**
///
/// The assertion that matters is the one after the removal: the entry is still readable, still
/// carries its `registry_id` and its recorded `expected_repository_id`, and now says *when* it
/// stopped counting. A `DELETE` would pass a test that only checked the entry no longer appears in
/// an active listing, and would leave nothing for a link to name.
#[test]
fn tombstoning_keeps_the_entry_and_its_identity() {
    let conn = store();
    insert_registry_entry(&conn, "r", "reg1", "repo-b", "Neighbour B", "/checkouts/b").unwrap();
    insert_registry_entry(&conn, "r", "reg2", "repo-c", "Neighbour C", "/checkouts/c").unwrap();

    assert!(tombstone_registry_entry(&conn, "r", "reg1").unwrap());

    // The row is still there, in the table, readable by the id it was registered under.
    assert_eq!(
        scalar(
            &conn,
            "SELECT count(*) FROM repo_registry WHERE repo_id = 'r'"
        ),
        2,
        "a tombstone is not a deletion"
    );
    let entry = registry_entry(&conn, "r", "reg1")
        .unwrap()
        .expect("a tombstoned entry is still readable, or registry_entry_removed is unanswerable");
    assert_eq!(entry.registry_id, "reg1");
    assert_eq!(
        entry.expected_repository_id, "repo-b",
        "the recorded identity is what a later check is made against; losing it loses the check"
    );
    assert_eq!(entry.local_path, "/checkouts/b");
    assert_eq!(entry.status, RegistryEntryStatus::Tombstoned);
    let withdrawn = entry
        .withdrawn_at
        .clone()
        .expect("a tombstone says when it happened");
    assert!(!withdrawn.is_empty());

    // A listing returns it too. Which entries to show is a decision a surface states, not one the
    // store makes silently on the way out.
    let listed = list_registry_entries(&conn, "r").unwrap();
    assert_eq!(listed.len(), 2);
    assert_eq!(
        listed
            .iter()
            .filter(|e| e.status == RegistryEntryStatus::Tombstoned)
            .count(),
        1
    );

    // Re-tombstoning changes nothing and does not re-date the ending. The date something ended is
    // not re-datable by asking again.
    assert!(!tombstone_registry_entry(&conn, "r", "reg1").unwrap());
    assert_eq!(
        registry_entry(&conn, "r", "reg1")
            .unwrap()
            .unwrap()
            .withdrawn_at,
        Some(withdrawn)
    );

    // The sibling entry is untouched, and so is the other repository's registry.
    assert_eq!(
        registry_entry(&conn, "r", "reg2").unwrap().unwrap().status,
        RegistryEntryStatus::Active
    );
    assert!(
        !tombstone_registry_entry(&conn, "r2", "reg1").unwrap(),
        "the update is not scoped by repository"
    );
}

/// Relocation moves the path and nothing else.
///
/// `expected_repository_id` is deliberately not a parameter. Relocation says *the same repository is
/// now somewhere else*; letting the expected identity be rewritten in the same call would make
/// relocation the silent re-pointing `target_repository_moved` exists to catch, performed by Nerve
/// itself on request. The identity check that must precede a real relocation is Slice 13a-ii's — it
/// needs a read-only open of the target, which this layer does not do.
#[test]
fn relocating_moves_the_path_and_leaves_the_recorded_identity_alone() {
    let conn = store();
    insert_registry_entry(&conn, "r", "reg1", "repo-b", "Neighbour B", "/checkouts/b").unwrap();
    conn.execute(
        "UPDATE repo_registry SET availability_checked_at = '2026-01-01T00:00:00.000Z'
          WHERE repo_id = 'r' AND registry_id = 'reg1'",
        [],
    )
    .unwrap();

    assert!(relocate_registry_entry(&conn, "r", "reg1", "/moved/b").unwrap());

    let entry = registry_entry(&conn, "r", "reg1").unwrap().unwrap();
    assert_eq!(entry.local_path, "/moved/b");
    assert_eq!(
        entry.expected_repository_id, "repo-b",
        "relocation must not re-point the entry at a different repository"
    );
    assert_eq!(entry.registry_id, "reg1");
    assert_eq!(entry.status, RegistryEntryStatus::Active);
    assert_eq!(
        entry.availability_checked_at, None,
        "a check of the old path says nothing about the new one"
    );

    // A tombstoned entry stays where it was when it was retired.
    assert!(tombstone_registry_entry(&conn, "r", "reg1").unwrap());
    assert!(
        !relocate_registry_entry(&conn, "r", "reg1", "/moved/again").unwrap(),
        "a retired entry must not be relocated"
    );
    assert_eq!(
        registry_entry(&conn, "r", "reg1")
            .unwrap()
            .unwrap()
            .local_path,
        "/moved/b"
    );

    // And an id nobody registered changes nothing rather than creating something.
    assert!(!relocate_registry_entry(&conn, "r", "absent", "/nowhere").unwrap());
    assert_eq!(scalar(&conn, "SELECT count(*) FROM repo_registry"), 1);
}

// ---- contract links ------------------------------------------------------------------------------

/// A recorded link comes back with every field it was given, and its timestamps stamped.
#[test]
fn a_recorded_link_round_trips_through_the_store() {
    let conn = store();
    insert_registry_entry(&conn, "r", "reg1", "repo-b", "Neighbour B", "/checkouts/b").unwrap();

    let id = insert_contract_link(&conn, "r", &link("reg1", "pkg-b/sub", "3:3")).unwrap();
    assert!(id > 0);

    let links = list_contract_links(&conn, "r").unwrap();
    assert_eq!(links.len(), 1);
    let row = &links[0];
    assert_eq!(row.link_id, Some(id));
    assert_eq!(row.registry_entry_id, "reg1");
    assert_eq!(row.expected_target_repository_id, "repo-b");
    assert_eq!(row.contract_identity, "pkg-b/sub");
    assert_eq!(
        row.resolution_method,
        ContractResolutionMethod::ExportMapResolved
    );
    assert_eq!(row.status, ContractLinkStatus::Active);
    assert_eq!(row.withdrawn_at, None);
    // Two versions, not one: a mismatch is a disagreement between two recorded numbers.
    assert_eq!(row.expected_contract_version.as_deref(), Some("^1.0.0"));
    assert_eq!(row.observed_contract_version.as_deref(), Some("1.2.0"));
    // The target snapshot, in full. Without it a renamed target has nothing left to name it.
    assert_eq!(row.target_entity_id.as_deref(), Some("b-entity-42"));
    assert_eq!(row.target_kind_snapshot.as_deref(), Some("file"));
    assert_eq!(row.target_name_snapshot.as_deref(), Some("sub.ts"));
    assert_eq!(row.target_path_snapshot.as_deref(), Some("src/sub.ts"));
    assert_eq!(
        row.target_state_at_resolution.as_deref(),
        Some("b-state-77")
    );
    // Stamped by the writer from one `strftime`, so the schema's ordering CHECK holds by
    // construction on a new row.
    assert!(!row.first_seen_at.is_empty());
    assert_eq!(row.first_seen_at, row.last_seen_at);

    // Scoped: the other repository has no links, and asking for them is empty rather than wrong.
    assert!(list_contract_links(&conn, "r2").unwrap().is_empty());
    assert_eq!(
        contract_links_for_registry_entry(&conn, "r", "reg1")
            .unwrap()
            .len(),
        1
    );
    assert!(contract_links_for_registry_entry(&conn, "r", "reg2")
        .unwrap()
        .is_empty());
}

/// **A contract link survives when the local `entity` table is emptied.**
///
/// This is the correction the whole sub-slice exists to honour, asserted the only way it can be:
/// `assertion.target_entity_id` is `NOT NULL REFERENCES entity(entity_id)` with
/// `PRAGMA foreign_keys=ON`, so an assertion could not name a foreign target at all. A contract
/// link's target is a snapshot of a row in *another* database and therefore has no foreign key — and
/// the proof is that deleting every local entity leaves the link, its target id and its whole
/// snapshot readable.
///
/// The control comes first: an ordinary assertion into the same emptied table *is* refused, so the
/// survival below is the absence of a constraint rather than the absence of enforcement.
#[test]
fn a_contract_link_outlives_the_local_entity_table() {
    let conn = store();
    insert_registry_entry(&conn, "r", "reg1", "repo-b", "Neighbour B", "/checkouts/b").unwrap();
    insert_contract_link(&conn, "r", &link("reg1", "pkg-b/sub", "3:3")).unwrap();

    // The control: the foreign key on `assertion` is real and enforced on this connection.
    let err = conn
        .execute(
            "INSERT INTO assertion VALUES ('a-foreign','r','local1','REFERENCES','b-entity-42')",
            [],
        )
        .unwrap_err();
    assert!(
        err.to_string().to_lowercase().contains("foreign key"),
        "an assertion naming a foreign target was accepted; the contrast this test rests on is \
         gone: {err}"
    );

    // The link's *source* end is local and is a foreign key, so it has to be released first — which
    // is itself the asymmetry being demonstrated.
    conn.execute(
        "UPDATE contract_link SET source_entity_id = NULL, source_kind_snapshot = NULL",
        [],
    )
    .unwrap();
    conn.execute("DELETE FROM entity", []).unwrap();
    assert_eq!(scalar(&conn, "SELECT count(*) FROM entity"), 0);

    // And the link is still there, still naming what it named.
    let links = list_contract_links(&conn, "r").unwrap();
    assert_eq!(
        links.len(),
        1,
        "a contract link must not depend on a local entity row for its target"
    );
    assert_eq!(links[0].target_entity_id.as_deref(), Some("b-entity-42"));
    assert_eq!(links[0].target_path_snapshot.as_deref(), Some("src/sub.ts"));
    assert_eq!(links[0].target_name_snapshot.as_deref(), Some("sub.ts"));
    assert_eq!(links[0].expected_target_repository_id, "repo-b");

    // Stated as the acceptance criterion states it: no `entity` row anywhere matches the target.
    assert_eq!(
        scalar(
            &conn,
            "SELECT count(*) FROM contract_link cl
               JOIN entity e ON e.entity_id = cl.target_entity_id"
        ),
        0,
        "a foreign target must never have a local entity row"
    );
}

/// Withdrawing a link keeps it, and re-withdrawing does not re-date its ending.
#[test]
fn withdrawing_a_link_keeps_the_row() {
    let conn = store();
    insert_registry_entry(&conn, "r", "reg1", "repo-b", "Neighbour B", "/checkouts/b").unwrap();
    let id = insert_contract_link(&conn, "r", &link("reg1", "pkg-b/sub", "3:3")).unwrap();
    let other = insert_contract_link(&conn, "r", &link("reg1", "pkg-b/other", "9:9")).unwrap();

    assert!(withdraw_contract_link(&conn, "r", id).unwrap());

    assert_eq!(
        scalar(&conn, "SELECT count(*) FROM contract_link"),
        2,
        "a withdrawal is not a deletion"
    );
    let links = list_contract_links(&conn, "r").unwrap();
    let withdrawn = links
        .iter()
        .find(|row| row.link_id == Some(id))
        .expect("a withdrawn link is still listed, or contract_deleted is unreportable");
    assert_eq!(withdrawn.status, ContractLinkStatus::Withdrawn);
    let when = withdrawn
        .withdrawn_at
        .clone()
        .expect("a withdrawal says when it happened");
    assert_eq!(withdrawn.contract_identity, "pkg-b/sub");

    // The sibling link is untouched.
    assert_eq!(
        links
            .iter()
            .find(|row| row.link_id == Some(other))
            .unwrap()
            .status,
        ContractLinkStatus::Active
    );

    assert!(!withdraw_contract_link(&conn, "r", id).unwrap());
    assert_eq!(
        list_contract_links(&conn, "r")
            .unwrap()
            .into_iter()
            .find(|row| row.link_id == Some(id))
            .unwrap()
            .withdrawn_at,
        Some(when)
    );

    // Scoped by repository: the other repository cannot withdraw this one's link.
    assert!(!withdraw_contract_link(&conn, "r2", other).unwrap());
    assert_eq!(
        scalar(
            &conn,
            "SELECT count(*) FROM contract_link WHERE status = 'active'"
        ),
        1
    );
}

/// Retiring an entry and withdrawing its links are two calls, and both keep every row.
///
/// They are separate on purpose: a link withdrawn without its entry being retired, and an entry
/// retired with its links left as they were, are both things a caller may legitimately want to
/// record, so the store never decides on the caller's behalf that a link ended.
#[test]
fn retiring_an_entry_and_withdrawing_its_links_keeps_both() {
    let conn = store();
    insert_registry_entry(&conn, "r", "reg1", "repo-b", "Neighbour B", "/checkouts/b").unwrap();
    insert_registry_entry(&conn, "r", "reg2", "repo-c", "Neighbour C", "/checkouts/c").unwrap();
    insert_contract_link(&conn, "r", &link("reg1", "pkg-b/sub", "3:3")).unwrap();
    insert_contract_link(&conn, "r", &link("reg1", "pkg-b/other", "9:9")).unwrap();
    let untouched = insert_contract_link(&conn, "r", &link("reg2", "pkg-c/sub", "3:3")).unwrap();

    let tx = conn.unchecked_transaction().unwrap();
    assert!(tombstone_registry_entry(&tx, "r", "reg1").unwrap());
    assert_eq!(
        withdraw_links_for_registry_entry(&tx, "r", "reg1").unwrap(),
        2
    );
    tx.commit().unwrap();

    // Nothing was destroyed: three links and two entries, exactly as before.
    assert_eq!(scalar(&conn, "SELECT count(*) FROM contract_link"), 3);
    assert_eq!(scalar(&conn, "SELECT count(*) FROM repo_registry"), 2);

    for row in contract_links_for_registry_entry(&conn, "r", "reg1").unwrap() {
        assert_eq!(row.status, ContractLinkStatus::Withdrawn);
        assert!(row.withdrawn_at.is_some());
        // The link still names the entry it resolved through, and the entry still exists to be
        // named — which together are what `registry_entry_removed` is reported from.
        assert_eq!(row.registry_entry_id, "reg1");
    }
    let entry = registry_entry(&conn, "r", "reg1").unwrap().unwrap();
    assert_eq!(entry.status, RegistryEntryStatus::Tombstoned);

    // A link through a different entry is untouched by either call.
    let other = list_contract_links(&conn, "r")
        .unwrap()
        .into_iter()
        .find(|row| row.link_id == Some(untouched))
        .unwrap();
    assert_eq!(other.status, ContractLinkStatus::Active);
    assert_eq!(other.withdrawn_at, None);

    // Re-running the sweep withdraws nothing further.
    assert_eq!(
        withdraw_links_for_registry_entry(&conn, "r", "reg1").unwrap(),
        0
    );
}

/// The store's writers use a plain `INSERT`, so a violated constraint is an error and not a silence.
///
/// Slice 3b lost graph rows to an `INSERT OR IGNORE` that swallowed `NOT NULL` violations and exited
/// zero. Here the same swallow would make a link through an unregistered entry — a link Nerve drew
/// without being asked to look at anything — indistinguishable from a repository with no contracts.
#[test]
fn a_link_through_an_unregistered_entry_is_an_error_rather_than_a_silence() {
    let conn = store();
    insert_registry_entry(&conn, "r", "reg1", "repo-b", "Neighbour B", "/checkouts/b").unwrap();

    let err =
        insert_contract_link(&conn, "r", &link("never-registered", "pkg-z", "3:3")).unwrap_err();
    assert!(
        err.to_string().to_lowercase().contains("foreign key"),
        "expected a foreign-key refusal, got {err}"
    );
    assert!(
        list_contract_links(&conn, "r").unwrap().is_empty(),
        "the refused row must not have landed"
    );

    // And an entry registered in the *other* repository is not this repository's entry.
    insert_registry_entry(&conn, "r2", "reg9", "repo-d", "D", "/checkouts/d").unwrap();
    let err = insert_contract_link(&conn, "r", &link("reg9", "pkg-d", "3:3")).unwrap_err();
    assert!(
        err.to_string().to_lowercase().contains("foreign key"),
        "a registry entry belonging to another repository was accepted: {err}"
    );

    // The control: the well-formed link lands, so the refusals above are the constraint doing its
    // job rather than the statement being malformed.
    insert_contract_link(&conn, "r", &link("reg1", "pkg-b/sub", "3:3")).unwrap();
    assert_eq!(list_contract_links(&conn, "r").unwrap().len(), 1);
}
