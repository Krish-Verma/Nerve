//! Human-confirmed memory at the storage boundary (schema v9, Slice 14a).
//!
//! Five properties are asserted repeatedly here, because they are what the sub-slice is:
//!
//! - **A record outlives its subject.** Every resolution test deletes the subject entity and then
//!   reads the record back, because the whole reason the subject is a snapshot is that
//!   `prune_orphans` deletes entities on every re-index.
//! - **A shared subject is not a contradiction.** The negative test is the load-bearing one: two
//!   notes about one file with no `claim_key` must be reported `multiple_active` and must **not** be
//!   reported `conflicted`, because as first drafted the rule would have manufactured a
//!   disagreement out of two unrelated English sentences.
//! - **Nothing is derived that is also stored.** `potentially_stale`, `conflicted` and
//!   `multiple_active` never appear in a column, and a stored status never appears as a view.
//! - **Supersession has one writable direction.** A source scan proves no second column holds the
//!   inverse, because a test that only exercised the API would pass against a schema that stored
//!   both.
//! - **The machine tables never move.** `assertion`, `observation`, `occurrence` and
//!   `assertion_state` are compared byte for byte across every operation in this file.
//!
//! Every fixture holds two repositories. All three v9 tables carry `repo_id`, and a read that forgot
//! to scope by it would still pass against a database holding one.

use nerve_core::vocab::{MemoryStatus, MemorySubjectResolution, MemoryView};
use nerve_store::memory::{
    append_memory_event, insert_memory, insert_memory_citation, list_memory, memory,
    memory_citations, memory_events, memory_for_subject, memory_in_scope, read_memory,
    read_memory_all, read_memory_for_subject, read_memory_in_scope, resolve_memory_subject,
    supersede_memory, superseded_by, MemoryCitationRow, MemoryEventRow, MemoryRow, MemorySubject,
};
use nerve_store::{migrate, open_in_memory, Connection};

/// A migrated database with two repositories, a state and an entity in each, and one extractor run.
///
/// The extractor run is what makes a repository state *current*: since ADR-0006 no graph row carries
/// a state, so the read model takes it from the most recent run exactly as
/// `history_freshness` does. Without one, every subject verdict is
/// `repository_state_unavailable` — which is a real state, and it has its own test.
fn store() -> Connection {
    let conn = open_in_memory().unwrap();
    migrate(&conn).unwrap();
    conn.execute_batch(
        "INSERT INTO repository VALUES ('r','p','/tmp/a','t');
         INSERT INTO repository VALUES ('r2','p','/tmp/b','t');
         INSERT INTO repository_state VALUES ('s','r','content',NULL,'m','t');
         INSERT INTO repository_state VALUES ('s-later','r','content',NULL,'m-later','t');
         INSERT INTO repository_state VALUES ('s2','r2','content',NULL,'m2','t');
         INSERT INTO entity VALUES ('local1','r','file','app.ts','src','typescript',NULL);
         INSERT INTO entity VALUES ('local2','r2','file','other.ts','src','typescript',NULL);
         INSERT INTO extractor_run
             VALUES (1,'r','s','ts-js-structural','1.1.0','t','t',1,0,'complete');
         INSERT INTO extractor_run
             VALUES (2,'r2','s2','ts-js-structural','1.1.0','t','t',1,0,'complete');",
    )
    .unwrap();
    conn
}

/// Move repository `r` on to a later state, as a re-index would.
fn reindex_to_later_state(conn: &Connection) {
    conn.execute(
        "INSERT INTO extractor_run
             VALUES (3,'r','s-later','ts-js-structural','1.1.0','t','t',1,0,'complete')",
        [],
    )
    .unwrap();
}

/// A well-formed record about `local1`, anchored at `s`.
fn note(memory_id: &str, claim_key: Option<&str>, content: &str) -> MemoryRow {
    MemoryRow {
        memory_id: memory_id.to_string(),
        subject: MemorySubject {
            entity_id: "local1".to_string(),
            kind: "file".to_string(),
            name: "app.ts".to_string(),
            path: "src/app.ts".to_string(),
            selector: "file:src/app.ts".to_string(),
        },
        anchor_state_id: "s".to_string(),
        scope: "file".to_string(),
        claim_key: claim_key.map(str::to_string),
        content: content.to_string(),
        author_label: "krish".to_string(),
        // Overwritten by the writer, which stamps it.
        created_at: String::new(),
        status: MemoryStatus::Active,
        supersedes_memory_id: None,
        invalidated_at: None,
        invalidation_reason: None,
    }
}

/// Every row of the four machine tables, in one deterministic string.
///
/// A full serialisation rather than a digest of one, which is strictly stronger: a digest says
/// *that* something moved and this says *what*. It needs no new dependency, which matters because
/// `nerve-store` has three and adding a hasher for a test would be one more.
fn machine_tables(conn: &Connection) -> String {
    let mut out = String::new();
    for (table, sql) in [
        (
            "entity",
            "SELECT entity_id || '|' || repo_id || '|' || kind || '|' || name || '|' || scope_path
               FROM entity ORDER BY entity_id",
        ),
        (
            "assertion",
            "SELECT assertion_id || '|' || repo_id || '|' || source_entity_id || '|' || relation
                 || '|' || target_entity_id
               FROM assertion ORDER BY assertion_id",
        ),
        (
            "occurrence",
            "SELECT occurrence_id || '|' || entity_id || '|' || file_path || '|' || start_byte
                 || '|' || end_byte
               FROM occurrence ORDER BY occurrence_id",
        ),
        (
            "observation",
            "SELECT observation_id || '|' || assertion_id || '|' || evidence_source_type || '|'
                 || directness || '|' || extractor_id || '|' || extractor_version || '|'
                 || file_path
               FROM observation ORDER BY observation_id",
        ),
        (
            "assertion_state",
            "SELECT assertion_id || '|' || status || '|' || strongest_source_type || '|'
                 || source_type_mask || '|' || observation_count || '|' || is_unresolved
               FROM assertion_state ORDER BY assertion_id",
        ),
    ] {
        out.push_str(table);
        out.push('\n');
        let mut stmt = conn.prepare(sql).unwrap();
        let rows = stmt.query_map([], |row| row.get::<_, String>(0)).unwrap();
        for row in rows {
            out.push_str(&row.unwrap());
            out.push('\n');
        }
    }
    out
}

/// A graph with a real assertion, occurrence, observation and derived state in it.
///
/// Without rows there would be nothing for [`machine_tables`] to find moving, and the invariant test
/// would pass by comparing two empty strings.
fn seed_graph(conn: &Connection) {
    conn.execute_batch(
        "INSERT INTO entity VALUES ('e1','r','function','add','src/app.ts','typescript',NULL);
         INSERT INTO assertion VALUES ('a1','r','local1','DEFINES','e1');
         INSERT INTO occurrence VALUES ('o1','e1','src/app.ts',0,10,1,0,1,10,'h');
         INSERT INTO observation
             (assertion_id, extractor_run_id, evidence_source_type, directness, extractor_id,
              extractor_version, file_path, start_line, end_line, content_hash, created_at)
         VALUES ('a1',1,'AST_DIRECT','DIRECT','ts-js-structural','1.1.0','src/app.ts',1,1,'h','t');",
    )
    .unwrap();
    nerve_store::rebuild_assertion_state(conn).unwrap();
}

fn scalar(conn: &Connection, sql: &str) -> i64 {
    conn.query_row(sql, [], |row| row.get(0)).unwrap()
}

// ---- writing and reading back --------------------------------------------------------------

/// A record round-trips through every column, and `created_at` is stamped rather than supplied.
#[test]
fn a_record_round_trips_with_every_column_intact() {
    let conn = store();
    let written =
        insert_memory(&conn, "r", &note("m1", Some("retry-policy"), "budget is 3")).unwrap();

    assert!(
        !written.created_at.is_empty(),
        "created_at must be stamped by the writer, not supplied"
    );
    let read = memory(&conn, "r", "m1").unwrap().expect("the record");
    assert_eq!(read, written);
    assert_eq!(read.subject.entity_id, "local1");
    assert_eq!(read.subject.selector, "file:src/app.ts");
    assert_eq!(read.claim_key.as_deref(), Some("retry-policy"));
    assert_eq!(read.status, MemoryStatus::Active);
    assert_eq!(read.author_label, "krish");
    assert!(read.supersedes_memory_id.is_none());

    // Scoped by repository. The second repository has no record of this id, and a read that
    // dropped the scope would find one.
    assert!(memory(&conn, "r2", "m1").unwrap().is_none());
    assert!(memory(&conn, "r", "absent").unwrap().is_none());
}

/// Lists are scoped by repository, by subject and by scope, and hold retired records too.
///
/// A retired record is listed like any other. Filtering one out would make *"what did we once
/// believe and no longer do"* unanswerable at exactly the moment it becomes the answer.
#[test]
fn lists_are_scoped_and_include_retired_records() {
    let conn = store();
    insert_memory(&conn, "r", &note("m1", None, "one")).unwrap();
    let mut second = note("m2", None, "two");
    second.scope = "repository".to_string();
    insert_memory(&conn, "r", &second).unwrap();
    let mut retired = note("m3", None, "three");
    retired.status = MemoryStatus::Invalidated;
    retired.invalidated_at = Some("2026-02-01T00:00:00.000Z".to_string());
    retired.invalidation_reason = Some("the module was removed".to_string());
    insert_memory(&conn, "r", &retired).unwrap();

    let mut elsewhere = note("m9", None, "another repository");
    elsewhere.subject.entity_id = "local2".to_string();
    elsewhere.anchor_state_id = "s2".to_string();
    insert_memory(&conn, "r2", &elsewhere).unwrap();

    let ids = |rows: Vec<MemoryRow>| -> Vec<String> {
        rows.into_iter().map(|row| row.memory_id).collect()
    };

    assert_eq!(ids(list_memory(&conn, "r").unwrap()), ["m1", "m2", "m3"]);
    assert_eq!(ids(list_memory(&conn, "r2").unwrap()), ["m9"]);
    assert_eq!(
        ids(memory_for_subject(&conn, "r", "local1").unwrap()),
        ["m1", "m2", "m3"],
        "the invalidated record must still be listed"
    );
    assert!(memory_for_subject(&conn, "r", "local2").unwrap().is_empty());
    assert_eq!(
        ids(memory_in_scope(&conn, "r", "file").unwrap()),
        ["m1", "m3"]
    );
    assert_eq!(
        ids(memory_in_scope(&conn, "r", "repository").unwrap()),
        ["m2"]
    );
}

/// Citations round-trip, including the one that names a place and no thing.
#[test]
fn citations_round_trip_and_may_name_only_a_place() {
    let conn = store();
    insert_memory(&conn, "r", &note("m1", None, "one")).unwrap();

    let citation =
        |entity: Option<&str>, kind: Option<&str>, name: Option<&str>| MemoryCitationRow {
            citation_id: None,
            memory_id: "m1".to_string(),
            cited_entity_id: entity.map(str::to_string),
            cited_kind: kind.map(str::to_string),
            cited_name: name.map(str::to_string),
            cited_path: "src/app.ts".to_string(),
            cited_span: Some("3:9".to_string()),
            cited_at_state: "s".to_string(),
            created_at: String::new(),
        };

    insert_memory_citation(
        &conn,
        "r",
        &citation(Some("local1"), Some("file"), Some("app.ts")),
    )
    .unwrap();
    insert_memory_citation(&conn, "r", &citation(None, None, None)).unwrap();

    let rows = memory_citations(&conn, "r", "m1").unwrap();
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0].cited_entity_id.as_deref(), Some("local1"));
    assert_eq!(rows[0].cited_name.as_deref(), Some("app.ts"));
    assert!(rows[1].cited_entity_id.is_none());
    assert_eq!(rows[1].cited_path, "src/app.ts");
    assert!(rows.iter().all(|row| !row.created_at.is_empty()));

    // The citation survives the deletion of the entity it names, for the reason the subject does.
    conn.execute("DELETE FROM entity WHERE entity_id = 'local1'", [])
        .unwrap();
    let after = memory_citations(&conn, "r", "m1").unwrap();
    assert_eq!(after.len(), 2);
    assert_eq!(after[0].cited_name.as_deref(), Some("app.ts"));
}

/// Events append in order and nothing removes one.
#[test]
fn events_append_in_order_and_are_never_removed() {
    let conn = store();
    let mut proposed = note("m1", None, "one");
    proposed.status = MemoryStatus::Proposed;
    insert_memory(&conn, "r", &proposed).unwrap();

    let event = |operation: &str, from: Option<MemoryStatus>, to: MemoryStatus| MemoryEventRow {
        event_id: None,
        memory_id: "m1".to_string(),
        at: String::new(),
        operation: operation.to_string(),
        from_status: from,
        to_status: to,
        note: None,
    };

    append_memory_event(&conn, "r", &event("propose", None, MemoryStatus::Proposed)).unwrap();
    append_memory_event(
        &conn,
        "r",
        &event(
            "confirm",
            Some(MemoryStatus::Proposed),
            MemoryStatus::Active,
        ),
    )
    .unwrap();
    // A status-preserving event is legitimate: a citation added to an active record changed no
    // status and still happened.
    append_memory_event(
        &conn,
        "r",
        &event("cite", Some(MemoryStatus::Active), MemoryStatus::Active),
    )
    .unwrap();

    let events = memory_events(&conn, "r", "m1").unwrap();
    assert_eq!(events.len(), 3);
    assert_eq!(
        events
            .iter()
            .map(|e| e.operation.as_str())
            .collect::<Vec<_>>(),
        ["propose", "confirm", "cite"]
    );
    assert!(
        events[0].from_status.is_none(),
        "the creating event has no prior status"
    );
    assert_eq!(events[1].from_status, Some(MemoryStatus::Proposed));
    assert_eq!(events[1].to_status, MemoryStatus::Active);
    assert!(events.iter().all(|e| !e.at.is_empty()));
    assert!(events.iter().all(|e| e.event_id.is_some()));

    assert!(memory_events(&conn, "r2", "m1").unwrap().is_empty());
}

// ---- supersession ----------------------------------------------------------------------------

/// **The status change and the event append are one transaction, and one direction is stored.**
#[test]
fn superseding_flips_the_status_and_appends_the_event_together() {
    let conn = store();
    insert_memory(&conn, "r", &note("m1", Some("owner"), "team A owns this")).unwrap();

    let mut successor = note("m2", Some("owner"), "team B owns this");
    successor.supersedes_memory_id = Some("m1".to_string());
    let written = supersede_memory(&conn, "r", &successor, "supersede", Some("handover")).unwrap();

    assert_eq!(written.memory_id, "m2");
    assert_eq!(
        memory(&conn, "r", "m1").unwrap().unwrap().status,
        MemoryStatus::Superseded
    );
    assert_eq!(
        memory(&conn, "r", "m2").unwrap().unwrap().status,
        MemoryStatus::Active
    );

    // The event exists, and it says what changed and why.
    let events = memory_events(&conn, "r", "m1").unwrap();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].operation, "supersede");
    assert_eq!(events[0].from_status, Some(MemoryStatus::Active));
    assert_eq!(events[0].to_status, MemoryStatus::Superseded);
    assert_eq!(events[0].note.as_deref(), Some("handover"));

    // The content of the superseded record is untouched: superseding rewrites nothing.
    assert_eq!(
        memory(&conn, "r", "m1").unwrap().unwrap().content,
        "team A owns this"
    );

    // One direction is stored; the inverse is a query.
    assert_eq!(
        memory(&conn, "r", "m2")
            .unwrap()
            .unwrap()
            .supersedes_memory_id,
        Some("m1".to_string())
    );
    assert_eq!(
        superseded_by(&conn, "r", "m1").unwrap(),
        Some("m2".to_string())
    );
    assert_eq!(superseded_by(&conn, "r", "m2").unwrap(), None);
}

/// A supersession that cannot be completed writes **nothing at all** — not even the successor.
///
/// The transaction is the point. Without it the successor row would land, the predecessor would stay
/// active, and the database would hold a record claiming to replace one that was never retired.
#[test]
fn a_refused_supersession_leaves_neither_half_applied() {
    let conn = store();
    let mut invalidated = note("m1", None, "one");
    invalidated.status = MemoryStatus::Invalidated;
    invalidated.invalidated_at = Some("2026-02-01T00:00:00.000Z".to_string());
    invalidated.invalidation_reason = Some("the module was removed".to_string());
    insert_memory(&conn, "r", &invalidated).unwrap();

    let mut successor = note("m2", None, "two");
    successor.supersedes_memory_id = Some("m1".to_string());
    let err = supersede_memory(&conn, "r", &successor, "supersede", None).unwrap_err();
    assert!(
        matches!(err, nerve_store::StoreError::Memory(_)),
        "expected a memory refusal, got {err}"
    );

    assert!(
        memory(&conn, "r", "m2").unwrap().is_none(),
        "the successor landed even though the supersession was refused"
    );
    assert_eq!(
        memory(&conn, "r", "m1").unwrap().unwrap().status,
        MemoryStatus::Invalidated,
        "the invalidated record was retired a second time"
    );
    assert!(memory_events(&conn, "r", "m1").unwrap().is_empty());

    // A successor that names no predecessor is not a supersession, and writes nothing either.
    let orphan = note("m3", None, "three");
    let err = supersede_memory(&conn, "r", &orphan, "supersede", None).unwrap_err();
    assert!(matches!(err, nerve_store::StoreError::Memory(_)), "{err}");
    assert!(memory(&conn, "r", "m3").unwrap().is_none());

    // And one naming a record in another repository finds nothing to retire.
    let mut cross = note("m4", None, "four");
    cross.supersedes_memory_id = Some("m1".to_string());
    assert!(supersede_memory(&conn, "r2", &cross, "supersede", None).is_err());
    assert!(memory(&conn, "r2", "m4").unwrap().is_none());

    assert_eq!(scalar(&conn, "SELECT count(*) FROM memory"), 1);
    assert_eq!(scalar(&conn, "SELECT count(*) FROM memory_event"), 0);
}

// ---- subject resolution ------------------------------------------------------------------------

/// The subject is in the index: the record attaches to it directly.
#[test]
fn a_present_subject_resolves() {
    let conn = store();
    insert_memory(&conn, "r", &note("m1", None, "one")).unwrap();

    let report = read_memory(&conn, "r", "m1").unwrap().expect("the record");
    assert_eq!(report.subject.resolution, MemorySubjectResolution::Resolved);
    assert_eq!(report.subject.live_entity_ids, ["local1"]);

    assert_eq!(
        resolve_memory_subject(&conn, "r", "local1")
            .unwrap()
            .resolution,
        MemorySubjectResolution::Resolved
    );
}

/// **The record survives the deletion of its subject and says so: `missing`, never nothing.**
#[test]
fn a_pruned_subject_reports_missing_and_the_record_is_still_readable() {
    let conn = store();
    insert_memory(
        &conn,
        "r",
        &note("m1", None, "the retry budget here is deliberate"),
    )
    .unwrap();

    // The control: it resolves before the subject is deleted.
    assert_eq!(
        read_memory(&conn, "r", "m1")
            .unwrap()
            .unwrap()
            .subject
            .resolution,
        MemorySubjectResolution::Resolved
    );

    conn.execute("DELETE FROM entity WHERE entity_id = 'local1'", [])
        .expect("a memory record must not block the deletion of its subject");

    let report = read_memory(&conn, "r", "m1").unwrap().expect("the record");
    assert_eq!(report.subject.resolution, MemorySubjectResolution::Missing);
    assert!(report.subject.live_entity_ids.is_empty());
    // Still readable, and still able to name what it was about.
    assert_eq!(report.row.content, "the retry budget here is deliberate");
    assert_eq!(report.row.subject.selector, "file:src/app.ts");
    assert_eq!(report.row.subject.name, "app.ts");
}

/// **A moved subject re-attaches only because an `identity_link` says so.**
#[test]
fn a_moved_subject_resolves_through_the_link_that_records_the_move() {
    let conn = store();
    insert_memory(&conn, "r", &note("m1", None, "one")).unwrap();

    conn.execute_batch(
        "INSERT INTO entity VALUES ('moved1','r','file','renamed.ts','src','typescript',NULL);
         DELETE FROM entity WHERE entity_id = 'local1';
         INSERT INTO identity_link
             (repo_id, left_entity_id, right_entity_id, link_kind, evidence, created_at)
         VALUES ('r','local1','moved1','moved_file','{}','t');",
    )
    .unwrap();

    let report = read_memory(&conn, "r", "m1").unwrap().expect("the record");
    assert_eq!(
        report.subject.resolution,
        MemorySubjectResolution::ResolvedThroughIdentityLink
    );
    assert_eq!(report.subject.live_entity_ids, ["moved1"]);
}

/// A link whose successor is itself gone reaches nothing, and nothing is guessed.
///
/// The bound is stated rather than hidden: exactly one link is followed. A subject moved twice
/// across two indexing runs reports `missing`, which is a true statement about the snapshot — the id
/// genuinely is not in the index — rather than an identity assembled out of a chain of proposals
/// whose combined strength nothing here measures.
#[test]
fn a_link_to_a_pruned_successor_reaches_nothing_and_a_chain_is_not_chased() {
    let conn = store();
    insert_memory(&conn, "r", &note("m1", None, "one")).unwrap();

    conn.execute_batch(
        "INSERT INTO entity VALUES ('moved2','r','file','twice.ts','src','typescript',NULL);
         DELETE FROM entity WHERE entity_id = 'local1';
         INSERT INTO identity_link
             (repo_id, left_entity_id, right_entity_id, link_kind, evidence, created_at)
         VALUES ('r','local1','moved1','moved_file','{}','t');
         INSERT INTO identity_link
             (repo_id, left_entity_id, right_entity_id, link_kind, evidence, created_at)
         VALUES ('r','moved1','moved2','moved_file','{}','t');",
    )
    .unwrap();

    let report = read_memory(&conn, "r", "m1").unwrap().expect("the record");
    assert_eq!(
        report.subject.resolution,
        MemorySubjectResolution::Missing,
        "a two-hop chain must not be chased"
    );
    assert!(report.subject.live_entity_ids.is_empty());

    // The control: the second hop is genuinely reachable on its own, so the `missing` above is the
    // one-hop bound rather than a broken query.
    assert_eq!(
        resolve_memory_subject(&conn, "r", "moved1")
            .unwrap()
            .resolution,
        MemorySubjectResolution::ResolvedThroughIdentityLink
    );
}

/// Two live successors: every candidate is reported and none is promoted.
#[test]
fn two_linked_successors_are_ambiguous_and_neither_is_chosen() {
    let conn = store();
    insert_memory(&conn, "r", &note("m1", None, "one")).unwrap();

    conn.execute_batch(
        "INSERT INTO entity VALUES ('split-a','r','file','a.ts','src','typescript',NULL);
         INSERT INTO entity VALUES ('split-b','r','file','b.ts','src','typescript',NULL);
         DELETE FROM entity WHERE entity_id = 'local1';
         INSERT INTO identity_link
             (repo_id, left_entity_id, right_entity_id, link_kind, evidence, created_at)
         VALUES ('r','local1','split-a','moved_file','{}','t');
         INSERT INTO identity_link
             (repo_id, left_entity_id, right_entity_id, link_kind, evidence, created_at)
         VALUES ('r','local1','split-b','moved_file','{}','t');",
    )
    .unwrap();

    let report = read_memory(&conn, "r", "m1").unwrap().expect("the record");
    assert_eq!(
        report.subject.resolution,
        MemorySubjectResolution::Ambiguous
    );
    assert_eq!(report.subject.live_entity_ids, ["split-a", "split-b"]);
}

/// **Nothing indexed is unknown, not missing.**
///
/// Reporting a subject as gone when nothing was ever looked at would claim a deletion nothing
/// observed — the `Stale` / `Unverified` separation Slice 7c-i made, one table over.
#[test]
fn an_unindexed_repository_reports_unknown_rather_than_missing() {
    let conn = store();
    conn.execute("DELETE FROM extractor_run WHERE repo_id = 'r'", [])
        .unwrap();
    insert_memory(&conn, "r", &note("m1", None, "one")).unwrap();

    let report = read_memory(&conn, "r", "m1").unwrap().expect("the record");
    assert_eq!(
        report.subject.resolution,
        MemorySubjectResolution::RepositoryStateUnavailable,
        "the subject is present in `entity`, and there is still no state to check it against"
    );
    assert!(report.subject.live_entity_ids.is_empty());
    assert!(report.current_state_id.is_none());
    // And with no state to compare against, nothing is reported as stale.
    assert!(report.views.is_empty());
}

/// Resolution is scoped by repository: a subject id present in the *other* repository is not a hit.
#[test]
fn subject_resolution_is_scoped_by_repository() {
    let conn = store();
    let mut elsewhere = note("m1", None, "one");
    elsewhere.subject.entity_id = "local2".to_string();
    insert_memory(&conn, "r", &elsewhere).unwrap();

    let report = read_memory(&conn, "r", "m1").unwrap().expect("the record");
    assert_eq!(
        report.subject.resolution,
        MemorySubjectResolution::Missing,
        "an entity in another repository resolved a subject in this one"
    );
    assert_eq!(
        resolve_memory_subject(&conn, "r2", "local2")
            .unwrap()
            .resolution,
        MemorySubjectResolution::Resolved
    );
}

// ---- the derived views ---------------------------------------------------------------------

/// **Two notes about one subject with no claim key are `multiple_active` and NOT `conflicted`.**
///
/// The negative half is the load-bearing one, and it is asserted first. As row 14 first drafted the
/// rule, `conflicted` fired on any two active records sharing a subject "whose content the resolver
/// cannot order" — and the content is free prose, so the resolver can never order it, so every
/// second note about a file became a contradiction. That is a claim manufactured by a rule out of
/// two unrelated English sentences, which is what `ADR_DESCRIBES_COMPONENT` was refused for.
#[test]
fn two_notes_on_one_subject_without_a_claim_key_are_not_a_conflict() {
    let conn = store();
    insert_memory(
        &conn,
        "r",
        &note("m1", None, "the retry budget here is deliberate"),
    )
    .unwrap();
    insert_memory(
        &conn,
        "r",
        &note("m2", None, "this file is generated on release"),
    )
    .unwrap();

    for report in read_memory_for_subject(&conn, "r", "local1").unwrap() {
        assert!(
            !report.views.contains(&MemoryView::Conflicted),
            "{} was reported as conflicting with a note it shares no claim with",
            report.row.memory_id
        );
        assert!(
            report.views.contains(&MemoryView::MultipleActive),
            "{} did not report the several-notes fact it should",
            report.row.memory_id
        );
    }

    // A single note about a subject is neither.
    let mut alone = note("m3", None, "alone");
    alone.scope = "repository".to_string();
    insert_memory(&conn, "r", &alone).unwrap();
    let only = read_memory(&conn, "r", "m3").unwrap().unwrap();
    assert!(only.views.is_empty(), "{:?}", only.views);
}

/// A conflict requires a **shared claim key**, and both facts are reported when both hold.
#[test]
fn a_shared_claim_key_is_what_makes_two_records_conflict() {
    let conn = store();
    insert_memory(&conn, "r", &note("m1", Some("owner"), "team A owns this")).unwrap();
    insert_memory(&conn, "r", &note("m2", Some("owner"), "team B owns this")).unwrap();
    // A third record with a *different* claim key competes with neither.
    insert_memory(
        &conn,
        "r",
        &note("m3", Some("deprecation"), "not deprecated"),
    )
    .unwrap();

    let reports = read_memory_for_subject(&conn, "r", "local1").unwrap();
    let views = |memory_id: &str| -> Vec<MemoryView> {
        reports
            .iter()
            .find(|report| report.row.memory_id == memory_id)
            .expect("the record")
            .views
            .clone()
    };

    assert_eq!(
        views("m1"),
        vec![MemoryView::Conflicted, MemoryView::MultipleActive],
        "both facts are true and both are reported"
    );
    assert_eq!(
        views("m2"),
        vec![MemoryView::Conflicted, MemoryView::MultipleActive]
    );
    assert_eq!(
        views("m3"),
        vec![MemoryView::MultipleActive],
        "a different claim key is not a disagreement"
    );

    // A claim key in a different *scope* is a different claim.
    let mut other_scope = note("m4", Some("owner"), "the repository is owned by team C");
    other_scope.scope = "repository".to_string();
    insert_memory(&conn, "r", &other_scope).unwrap();
    assert_eq!(
        read_memory(&conn, "r", "m4").unwrap().unwrap().views,
        Vec::<MemoryView>::new()
    );
}

/// Only an **active** record is qualified. A retired one is not a statement anyone is making.
#[test]
fn a_retired_record_carries_no_views() {
    let conn = store();
    insert_memory(&conn, "r", &note("m1", Some("owner"), "team A")).unwrap();

    let mut proposed = note("m2", Some("owner"), "team B");
    proposed.status = MemoryStatus::Proposed;
    insert_memory(&conn, "r", &proposed).unwrap();

    let mut invalidated = note("m3", Some("owner"), "team C");
    invalidated.status = MemoryStatus::Invalidated;
    invalidated.invalidated_at = Some("2026-02-01T00:00:00.000Z".to_string());
    insert_memory(&conn, "r", &invalidated).unwrap();

    reindex_to_later_state(&conn);

    // The only active record: stale against the new state, and competing with nothing, because the
    // other two are not active and are counted into no group.
    assert_eq!(
        read_memory(&conn, "r", "m1").unwrap().unwrap().views,
        vec![MemoryView::PotentiallyStale]
    );
    for retired in ["m2", "m3"] {
        assert!(
            read_memory(&conn, "r", retired)
                .unwrap()
                .unwrap()
                .views
                .is_empty(),
            "{retired} was qualified even though nobody is making the statement"
        );
    }
}

/// `potentially_stale` is the anchor state against the current one, and nothing else.
#[test]
fn a_record_anchored_to_an_older_state_is_potentially_stale() {
    let conn = store();
    insert_memory(&conn, "r", &note("m1", None, "one")).unwrap();
    assert!(
        read_memory(&conn, "r", "m1")
            .unwrap()
            .unwrap()
            .views
            .is_empty(),
        "the anchor is the current state; nothing is stale yet"
    );

    reindex_to_later_state(&conn);

    let report = read_memory(&conn, "r", "m1").unwrap().expect("the record");
    assert_eq!(report.views, vec![MemoryView::PotentiallyStale]);
    assert_eq!(report.current_state_id.as_deref(), Some("s-later"));
    // The stored row is untouched: the qualification is computed and never written.
    assert_eq!(report.row.anchor_state_id, "s");
    assert_eq!(report.row.status, MemoryStatus::Active);
}

/// **No derived view is ever stored, and no stored status is ever a view.**
///
/// A probe: after every operation this file performs, the `status` column holds only the four
/// stored values. A writer that put `potentially_stale` in a column would fail here by name.
#[test]
fn no_derived_view_ever_reaches_the_status_column() {
    let conn = store();
    insert_memory(&conn, "r", &note("m1", Some("owner"), "team A")).unwrap();
    let mut successor = note("m2", Some("owner"), "team B");
    successor.supersedes_memory_id = Some("m1".to_string());
    supersede_memory(&conn, "r", &successor, "supersede", None).unwrap();
    reindex_to_later_state(&conn);

    let mut stmt = conn
        .prepare("SELECT DISTINCT status FROM memory ORDER BY 1")
        .unwrap();
    let stored: Vec<String> = stmt
        .query_map([], |row| row.get::<_, String>(0))
        .unwrap()
        .map(|row| row.unwrap())
        .collect();
    assert_eq!(stored, ["active", "superseded"]);
    for value in &stored {
        value
            .parse::<MemoryStatus>()
            .unwrap_or_else(|_| panic!("`{value}` is not a stored MemoryStatus"));
        assert!(
            value.parse::<MemoryView>().is_err(),
            "`{value}` is a derived view and it is in a column"
        );
    }

    // And the views that *are* reported were computed, not read: none of them is in any column.
    let report = read_memory(&conn, "r", "m2").unwrap().unwrap();
    assert_eq!(report.views, vec![MemoryView::PotentiallyStale]);
    let text: String = conn
        .query_row(
            "SELECT group_concat(status || '|' || coalesce(claim_key,'') || '|' || scope)
               FROM memory",
            [],
            |row| row.get(0),
        )
        .unwrap();
    for view in MemoryView::ALL {
        assert!(
            !text.contains(view.as_str()),
            "`{view}` was written into a column"
        );
    }
}

/// **The database refuses a derived view in the status column — not only this crate's writers.**
///
/// The test above proves `nerve-store`'s own functions never store a view. That is a property of
/// *this code*, and the invariant is stronger than that: `potentially_stale`, `conflicted` and
/// `multiple_active` are derived and **must not be storable at all**. A vocabulary closed only in
/// Rust leaves raw SQL free to write one, which would then fail on the next read through
/// `MemoryStatus::FromStr` — loudly, but after the row is on disk, which is a repair job rather than
/// a refusal.
///
/// So `V9` enumerates the four stored statuses in a `CHECK`, following v7's
/// `git_rename_hypothesis`, and this asserts the constraint rather than the caller. Each of the
/// three views is refused **by name**, and the anti-vacuity half matters as much: all four real
/// statuses must still be accepted, or a constraint that refused everything would pass this.
#[test]
fn the_database_itself_refuses_a_derived_view_in_the_status_column() {
    let conn = store();

    for view in MemoryView::ALL {
        let mut row = note("v1", None, "anything");
        row.status = MemoryStatus::Active;
        insert_memory(&conn, "r", &row).unwrap();
        conn.execute("DELETE FROM memory WHERE memory_id = 'v1'", [])
            .unwrap();

        // Raw SQL, deliberately bypassing every Rust type in this crate.
        let refused = conn.execute(
            "INSERT INTO memory
                 (memory_id, repo_id, subject_entity_id_snapshot, subject_kind_snapshot,
                  subject_name_snapshot, subject_path_snapshot, subject_selector_snapshot,
                  anchor_state_id, scope, claim_key, content, author_label, created_at, status,
                  supersedes_memory_id, invalidated_at, invalidation_reason)
             VALUES ('probe', 'r', 'e1', 'file', 'a.ts', 'a.ts', 'a.ts',
                     'state-1', 'anything', NULL, 'a sentence', 'local', '2026-01-01T00:00:00Z', ?1,
                     NULL, NULL, NULL)",
            rusqlite::params![view.as_str()],
        );
        let error = refused
            .expect_err(&format!("`{view}` is a derived view and was stored"))
            .to_string();
        assert!(
            error.contains("CHECK constraint failed"),
            "`{view}` was refused, but not by the CHECK: {error}"
        );
    }

    // Anti-vacuity: the four stored statuses are still accepted. Without this, a `CHECK` that
    // refused every value would satisfy the loop above.
    for status in MemoryStatus::ALL {
        let mut row = note(&format!("ok-{status}"), None, "anything");
        row.status = status;
        if status == MemoryStatus::Invalidated {
            row.invalidated_at = Some("2026-01-02T00:00:00Z".to_string());
        }
        insert_memory(&conn, "r", &row).unwrap_or_else(|error| {
            panic!("`{status}` is a stored status and was refused: {error}")
        });
    }
}

/// The list readers agree with the single reader, record for record.
#[test]
fn the_list_readers_agree_with_the_single_reader() {
    let conn = store();
    insert_memory(&conn, "r", &note("m1", Some("owner"), "team A")).unwrap();
    insert_memory(&conn, "r", &note("m2", Some("owner"), "team B")).unwrap();
    reindex_to_later_state(&conn);

    let all = read_memory_all(&conn, "r").unwrap();
    assert_eq!(all.len(), 2);
    for report in &all {
        let single = read_memory(&conn, "r", &report.row.memory_id)
            .unwrap()
            .unwrap();
        assert_eq!(&single, report);
    }
    assert_eq!(read_memory_for_subject(&conn, "r", "local1").unwrap(), all);
    assert_eq!(read_memory_in_scope(&conn, "r", "file").unwrap(), all);
    assert!(read_memory_in_scope(&conn, "r", "repository")
        .unwrap()
        .is_empty());
    assert!(read_memory_all(&conn, "r2").unwrap().is_empty());
}

// ---- the invariant that separates memory from evidence ---------------------------------------

/// **`assertion`, `observation`, `occurrence` and `assertion_state` are byte-identical across every
/// memory operation.**
///
/// Memory is offered *beside* evidence and never mixed into it. A memory record is a statement about
/// one subject rather than a relation between two entities, and `assertion_state` is defined as a
/// pure function of machine observations — a human's sentence in that table would be the
/// silent-truth failure arrived at through the schema instead of through a feature.
///
/// The comparison is a full serialisation rather than a digest, which is strictly stronger: it says
/// what moved as well as that something did.
#[test]
fn no_memory_operation_moves_a_single_byte_of_the_evidence_tables() {
    let conn = store();
    seed_graph(&conn);

    // Anti-vacuity: the tables have rows in them, so "unchanged" is a measurement.
    let before = machine_tables(&conn);
    assert!(before.contains("AST_DIRECT"), "{before}");
    assert!(before.contains("DEFINES"), "{before}");
    assert!(before.contains("SUPPORTED"), "{before}");
    assert!(before.lines().count() > 8, "{before}");

    insert_memory(&conn, "r", &note("m1", Some("owner"), "team A owns this")).unwrap();
    insert_memory_citation(
        &conn,
        "r",
        &MemoryCitationRow {
            citation_id: None,
            memory_id: "m1".to_string(),
            cited_entity_id: Some("e1".to_string()),
            cited_kind: Some("function".to_string()),
            cited_name: Some("add".to_string()),
            cited_path: "src/app.ts".to_string(),
            cited_span: Some("1:1".to_string()),
            cited_at_state: "s".to_string(),
            created_at: String::new(),
        },
    )
    .unwrap();
    append_memory_event(
        &conn,
        "r",
        &MemoryEventRow {
            event_id: None,
            memory_id: "m1".to_string(),
            at: String::new(),
            operation: "cite".to_string(),
            from_status: Some(MemoryStatus::Active),
            to_status: MemoryStatus::Active,
            note: None,
        },
    )
    .unwrap();
    let mut successor = note("m2", Some("owner"), "team B owns this");
    successor.supersedes_memory_id = Some("m1".to_string());
    supersede_memory(&conn, "r", &successor, "supersede", None).unwrap();

    // And the reads, which must not write either.
    read_memory_all(&conn, "r").unwrap();
    read_memory(&conn, "r", "m1").unwrap();
    resolve_memory_subject(&conn, "r", "local1").unwrap();
    memory_events(&conn, "r", "m1").unwrap();
    memory_citations(&conn, "r", "m1").unwrap();
    superseded_by(&conn, "r", "m1").unwrap();

    assert_eq!(
        machine_tables(&conn),
        before,
        "a memory operation moved the evidence tables"
    );
    // The memory tables did fill up, so the invariant above is not holding because nothing happened.
    assert_eq!(scalar(&conn, "SELECT count(*) FROM memory"), 2);
    assert_eq!(scalar(&conn, "SELECT count(*) FROM memory_citation"), 1);
    assert_eq!(scalar(&conn, "SELECT count(*) FROM memory_event"), 2);
}

/// The control for the test above: a probe that *does* write evidence is caught.
///
/// Without this, a serialisation that silently returned the same string for every database would
/// make the invariant hold by measuring nothing.
#[test]
fn the_evidence_comparison_catches_a_probe_that_writes_evidence() {
    let conn = store();
    seed_graph(&conn);
    let before = machine_tables(&conn);

    conn.execute(
        "INSERT INTO assertion_state
             VALUES ('probe','SUPPORTED','DOCUMENT_STATED',256,1,0)",
        [],
    )
    .unwrap_or(0);
    conn.execute(
        "INSERT INTO entity VALUES ('probe','r','function','probe','src/app.ts',NULL,NULL)",
        [],
    )
    .unwrap();

    assert_ne!(
        machine_tables(&conn),
        before,
        "the comparison did not notice a row written into the evidence tables"
    );
}

// ---- the source scans --------------------------------------------------------------------------

fn store_source(file: &str) -> String {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("src")
        .join(file);
    std::fs::read_to_string(&path)
        .unwrap_or_else(|error| panic!("{} could not be read: {error}", path.display()))
}

/// The same source with every comment line removed.
///
/// The scans below look for names that must not be *used*, and this module's own documentation
/// names several of them precisely in order to say they are absent. Scanning the prose as though it
/// were code would make a file that explains its constraint indistinguishable from one that
/// violates it.
fn code_only(source: &str) -> String {
    source
        .lines()
        .filter(|line| {
            let trimmed = line.trim_start();
            !(trimmed.starts_with("//") || trimmed.starts_with('*'))
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// **No second column stores the inverse of supersession.**
///
/// A test that only exercised the API would pass against a schema holding both directions, so the
/// claim is made against the DDL itself: `supersedes_memory_id` appears and `superseded_by` does
/// not. Two independently writable directions of one fact can disagree with nothing to notice.
#[test]
fn supersession_has_exactly_one_writable_direction_in_the_schema() {
    let schema = code_only(&store_source("schema.rs"));
    let start = schema
        .find("const V9: &str")
        .expect("the v9 migration must exist");
    let ddl = &schema[start..];

    assert!(
        ddl.contains("supersedes_memory_id"),
        "the scan found no supersession column at all"
    );
    assert!(
        !ddl.contains("superseded_by"),
        "the v9 schema stores the inverse of supersession in a second column"
    );

    // And the module derives it rather than reading a column.
    let module = store_source("memory.rs");
    assert!(
        module.contains("pub fn superseded_by"),
        "the derived inverse is gone"
    );
    assert!(
        !module.contains("SET superseded_by"),
        "the module writes an inverse column"
    );
}

/// **`memory_event` is append-only, and the enforcement is that no writer exists.**
///
/// Asserted by scanning every source file in the workspace for a `DELETE` or an `UPDATE` against the
/// table. A trigger was considered and declined — it can be dropped by a later migration, whereas a
/// scan cannot be satisfied by anything except the code not existing.
#[test]
fn nothing_in_the_workspace_deletes_or_updates_a_memory_event() {
    let crates = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .expect("the workspace root")
        .join("crates");

    let mut scanned = 0;
    let mut offenders = Vec::new();
    let mut stack = vec![crates.clone()];
    while let Some(dir) = stack.pop() {
        for entry in std::fs::read_dir(&dir).expect("a readable directory") {
            let path = entry.expect("a readable entry").path();
            if path.is_dir() {
                stack.push(path);
                continue;
            }
            if path.extension().is_none_or(|ext| ext != "rs") {
                continue;
            }
            scanned += 1;
            let source = std::fs::read_to_string(&path).expect("a readable source file");
            for (offset, line) in source.lines().enumerate() {
                let lowered = line.to_lowercase();
                let forbidden = lowered.contains("delete from memory_event")
                    || lowered.contains("update memory_event");
                // This file names both forms in order to forbid them.
                let is_this_test = path.ends_with("tests/memory.rs");
                if forbidden && !is_this_test {
                    offenders.push(format!("{}:{}", path.display(), offset + 1));
                }
            }
        }
    }

    // Anti-vacuity: a walk that found nothing would pass by scanning nothing.
    assert!(scanned >= 40, "the scan read only {scanned} source files");
    assert!(
        offenders.is_empty(),
        "memory_event is append-only; these statements would rewrite an audit history:\n  {}",
        offenders.join("\n  ")
    );
}

/// **Memory is not evidence, and the module says nothing that would make it one.**
///
/// No `EvidenceSourceType`, no `Relation`, no statement touching `assertion_state`. Inventing a
/// relation for a human sentence about one subject (`HUMAN_NOTED_ABOUT`) is what
/// `ADR_DESCRIBES_COMPONENT` was refused for, and a memory row in `assertion_state` would be a human
/// sentence inside a table defined as a pure function of machine observations.
#[test]
fn the_memory_module_contains_no_statement_touching_the_evidence_tables() {
    let module = code_only(&store_source("memory.rs"));
    for table in ["assertion_state", "observation", "occurrence"] {
        for verb in ["INSERT INTO", "UPDATE", "DELETE FROM"] {
            let needle = format!("{verb} {table}");
            assert!(
                !module.contains(&needle),
                "`{needle}` is in the memory module"
            );
        }
    }
    assert!(
        !module.contains("INSERT INTO assertion"),
        "the memory module writes an assertion"
    );
    assert!(
        !module.contains("EvidenceSourceType"),
        "the memory module reaches for an evidence source type"
    );
    assert!(
        !module.contains("HUMAN_NOTED"),
        "a relation was invented for a statement about one subject"
    );
    // Anti-vacuity: the scan is looking at the right file.
    assert!(module.contains("pub fn insert_memory"));
    assert!(
        module.contains("FROM entity"),
        "the read model does resolve subjects"
    );
}
