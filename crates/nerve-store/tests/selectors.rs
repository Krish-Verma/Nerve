//! The selector rules, over a graph small enough to reason about completely.
//!
//! `crates/nerve-index/tests/selectors.rs` checks the rules against what the pipeline actually
//! writes. This file checks the rules themselves, including the cases the pipeline cannot
//! currently produce — which is the point: the resolver must not depend on the pipeline's
//! present-day guarantees for its refusals to hold.
//!
//! **A `Module` and a `Document` cannot today share one path.** `FileKind::from_extension`
//! returns exactly one kind per extension and `Language::from_extension`'s set is disjoint from
//! `DOCUMENT_EXTENSIONS`, so one file becomes one or the other and never both. That is a fact
//! about `nerve-index`, not about `nerve-store`, and it can change with one new extension. The
//! resolver therefore treats two content entities at one path as **ambiguous**, and
//! [`two_content_entities_at_one_path_are_ambiguous`] is what holds it to that.

use nerve_store::{
    migrate, open_in_memory, resolve_selector, Connection, InvalidSelector, Selection,
    SelectorRefusal,
};

const STATE: &str = "state-1";

fn fixture() -> Connection {
    let conn = open_in_memory().unwrap();
    migrate(&conn).unwrap();
    conn.execute(
        "INSERT INTO repository (repo_id, project_id, root_path, created_at)
         VALUES ('repo', 'project', '/tmp/repo', '2026-01-01T00:00:00Z')",
        [],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO repository_state (state_id, repo_id, kind, content_merkle, created_at)
         VALUES (?1, 'repo', 'INDEX', 'merkle', '2026-01-01T00:00:00Z')",
        rusqlite::params![STATE],
    )
    .unwrap();
    conn
}

/// One entity, with an occurrence so that it has a location to print.
fn entity(conn: &Connection, id: &str, kind: &str, name: &str, scope_path: &str) {
    entity_with_meta(conn, id, kind, name, scope_path, None);
}

fn entity_with_meta(
    conn: &Connection,
    id: &str,
    kind: &str,
    name: &str,
    scope_path: &str,
    meta: Option<&str>,
) {
    conn.execute(
        "INSERT INTO entity (entity_id, repo_id, kind, name, scope_path, language, meta)
         VALUES (?1, 'repo', ?2, ?3, ?4, 'typescript', ?5)",
        rusqlite::params![id, kind, name, scope_path, meta],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO occurrence (occurrence_id, entity_id, file_path, start_byte,
                                 end_byte, start_line, start_col, end_line, end_col, content_hash)
         VALUES (?1, ?2, 'src/app.ts', 0, 1, 1, 0, 1, 1, 'hash')",
        rusqlite::params![format!("occ-{id}"), id],
    )
    .unwrap();
}

fn resolved_id(conn: &Connection, selector: &str) -> String {
    match resolve_selector(conn, selector).unwrap() {
        Selection::Resolved { entity, .. } => entity.entity_id,
        other => panic!("{selector} must resolve, got {other:?}"),
    }
}

// ---- the two-tier path rule -------------------------------------------------------------------

/// One content entity and one container entity: the rule fires, and it says that it fired.
#[test]
fn a_path_resolves_to_its_content_and_reports_the_container_it_passed_over() {
    let conn = fixture();
    entity(&conn, "mod", "module", "app", "src/app.ts");
    entity(&conn, "file", "file", "app.ts", "src");

    match resolve_selector(&conn, "src/app.ts").unwrap() {
        Selection::Resolved {
            entity,
            matched_by,
            alternatives,
        } => {
            assert_eq!(entity.entity_id, "mod");
            assert_eq!(matched_by.as_str(), "path");
            assert_eq!(alternatives.len(), 1);
            assert_eq!(alternatives[0].entity_id, "file");
        }
        other => panic!("expected a resolution, got {other:?}"),
    }
}

/// Nothing in the content tier: the container tier decides, and passes over nothing.
#[test]
fn a_path_with_only_a_container_resolves_to_it() {
    let conn = fixture();
    entity(&conn, "file", "file", "notes.txt", "docs");

    match resolve_selector(&conn, "docs/notes.txt").unwrap() {
        Selection::Resolved {
            entity,
            alternatives,
            ..
        } => {
            assert_eq!(entity.entity_id, "file");
            assert!(alternatives.is_empty(), "{alternatives:?}");
        }
        other => panic!("expected a resolution, got {other:?}"),
    }
}

/// **The case the plan asked to be verified rather than assumed.**
///
/// A `Module` and a `Document` at one path are two content entities. The rule chooses between
/// *tiers*, never between members of one, so this is ambiguous and nothing is chosen — exactly
/// as two functions named `parse` are. `nerve-index` cannot currently produce this state (see
/// the module header); the resolver does not rely on that.
#[test]
fn two_content_entities_at_one_path_are_ambiguous() {
    let conn = fixture();
    entity(&conn, "mod", "module", "app", "src/app.ts");
    entity(&conn, "doc", "document", "app", "src/app.ts");
    entity(&conn, "file", "file", "app.ts", "src");

    match resolve_selector(&conn, "src/app.ts").unwrap() {
        Selection::Ambiguous {
            candidates,
            matched_by,
        } => {
            let ids: Vec<&str> = candidates.iter().map(|c| c.entity_id.as_str()).collect();
            assert_eq!(ids, vec!["doc", "mod"], "the deciding tier, and only it");
            assert_eq!(matched_by.as_str(), "path");
        }
        other => panic!("expected ambiguity, got {other:?}"),
    }

    // And the way out is a qualifier, which is the caller's decision rather than Nerve's.
    assert_eq!(resolved_id(&conn, "module:src/app.ts"), "mod");
    assert_eq!(resolved_id(&conn, "document:src/app.ts"), "doc");
    assert_eq!(resolved_id(&conn, "file:src/app.ts"), "file");
}

/// Two container entities at one path are ambiguous for the same reason.
#[test]
fn two_container_entities_at_one_path_are_ambiguous() {
    let conn = fixture();
    entity(&conn, "file", "file", "thing", "src");
    entity(&conn, "dir", "directory", "thing", "src");

    match resolve_selector(&conn, "src/thing").unwrap() {
        Selection::Ambiguous { candidates, .. } => assert_eq!(candidates.len(), 2),
        other => panic!("expected ambiguity, got {other:?}"),
    }
}

/// A `Section` also stores a repository path in `scope_path`, and is not at that path.
///
/// This is why the path stage's kind lists are generated from `EntityKind::path_role` rather
/// than being "whatever has a path-shaped scope": every heading in a document would otherwise be
/// a candidate, and every document path would be permanently ambiguous.
#[test]
fn a_section_is_not_an_entity_at_the_document_path() {
    let conn = fixture();
    entity(&conn, "doc", "document", "architecture", "docs/a.md");
    entity(&conn, "sect", "section", "Overview", "docs/a.md");

    match resolve_selector(&conn, "docs/a.md").unwrap() {
        Selection::Resolved {
            entity,
            alternatives,
            ..
        } => {
            assert_eq!(entity.entity_id, "doc");
            assert!(alternatives.is_empty(), "{alternatives:?}");
        }
        other => panic!("expected the document, got {other:?}"),
    }
}

// ---- qualifiers -------------------------------------------------------------------------------

/// A qualifier constrains every stage, not only the path stage.
#[test]
fn a_qualifier_constrains_each_stage() {
    let conn = fixture();
    entity(&conn, "mod", "module", "thing", "src/thing.ts");
    entity(&conn, "fn", "function", "thing", "");

    // Stage 1: an entity id, with a qualifier that does not admit it.
    assert!(matches!(
        resolve_selector(&conn, "function:mod").unwrap(),
        Selection::NotFound { .. }
    ));
    assert_eq!(resolved_id(&conn, "module:mod"), "mod");

    // Stage 4: a name two kinds share.
    match resolve_selector(&conn, "thing").unwrap() {
        Selection::Ambiguous { candidates, .. } => assert_eq!(candidates.len(), 2),
        other => panic!("expected ambiguity, got {other:?}"),
    }
    assert_eq!(resolved_id(&conn, "symbol:thing"), "fn");
    assert_eq!(resolved_id(&conn, "module:thing"), "mod");
}

/// The `adr` alias reads `meta`, which means `json_extract` on the SQLite build that ships.
#[test]
fn the_adr_alias_reads_the_metadata_the_indexer_writes() {
    let conn = fixture();
    entity_with_meta(
        &conn,
        "adr1",
        "document",
        "ADR-0001-header-status",
        "docs/decisions/ADR-0001-header-status.md",
        Some(r#"{"adr": true, "adr_id": "ADR-0001", "status": "Accepted"}"#),
    );
    entity_with_meta(
        &conn,
        "plain",
        "document",
        "notes",
        "docs/notes.md",
        Some(r#"{"adr": false, "adr_id": null}"#),
    );
    // A non-document carrying a lookalike `meta` must not be reachable through the alias.
    entity_with_meta(
        &conn,
        "impostor",
        "function",
        "adrLike",
        "",
        Some(r#"{"adr": true, "adr_id": "ADR-0001"}"#),
    );

    assert_eq!(resolved_id(&conn, "adr:ADR-0001"), "adr1");
    assert_eq!(resolved_id(&conn, "adr:ADR-0001-header-status"), "adr1");
    assert!(matches!(
        resolve_selector(&conn, "adr:notes").unwrap(),
        Selection::NotFound { .. }
    ));
    assert!(matches!(
        resolve_selector(&conn, "adr:adrLike").unwrap(),
        Selection::NotFound { .. }
    ));

    // Unqualified, `ADR-0001` is not a name anywhere, so the widening is the alias's alone.
    assert!(matches!(
        resolve_selector(&conn, "ADR-0001").unwrap(),
        Selection::NotFound { .. }
    ));
}

/// A qualified miss carries what the qualifier excluded, from the stage that would have matched.
#[test]
fn a_qualified_miss_carries_what_it_ruled_out() {
    let conn = fixture();
    entity(&conn, "doc", "document", "architecture", "docs/a.md");
    entity(&conn, "file", "file", "a.md", "docs");

    match resolve_selector(&conn, "module:docs/a.md").unwrap() {
        Selection::NotFound {
            qualifier,
            excluded,
            suggestions,
        } => {
            assert_eq!(qualifier.map(|q| q.as_str()), Some("module"));
            let ids: Vec<&str> = excluded.iter().map(|e| e.entity_id.as_str()).collect();
            assert_eq!(ids, vec!["doc", "file"], "both readings, neither admitted");
            // Suggestions are narrowed to the qualifier's kind: offering a document to someone
            // who asked for a module would answer a question they did not ask.
            assert!(
                suggestions.iter().all(|hit| hit.kind == "module"),
                "{suggestions:?}"
            );
        }
        other => panic!("expected a qualified miss, got {other:?}"),
    }

    // An unqualified miss has nothing to exclude.
    match resolve_selector(&conn, "docs/nothing.md").unwrap() {
        Selection::NotFound {
            qualifier,
            excluded,
            ..
        } => {
            assert!(qualifier.is_none());
            assert!(excluded.is_empty(), "{excluded:?}");
        }
        other => panic!("expected a miss, got {other:?}"),
    }
}

// ---- the two refusals that are not misses -----------------------------------------------------

#[test]
fn a_malformed_selector_is_invalid_and_a_traversal_one_is_refused() {
    let conn = fixture();
    entity(&conn, "mod", "module", "app", "src/app.ts");

    for (selector, reason) in [
        ("banana:foo", InvalidSelector::UnknownQualifier),
        (":foo", InvalidSelector::EmptyQualifier),
        ("module:", InvalidSelector::EmptyBody),
        ("", InvalidSelector::EmptyBody),
    ] {
        match resolve_selector(&conn, selector).unwrap() {
            Selection::Invalid { reason: actual } => assert_eq!(actual, reason, "{selector}"),
            other => panic!("{selector} must be invalid, got {other:?}"),
        }
    }

    for selector in ["../../etc/passwd", "/etc/passwd", "file:../x", "./../x"] {
        match resolve_selector(&conn, selector).unwrap() {
            Selection::Refused { reason } => {
                assert_eq!(reason, SelectorRefusal::Traversal, "{selector}")
            }
            other => panic!("{selector} must be refused, got {other:?}"),
        }
    }
}

// ---- nothing the caller types reaches SQL as text ---------------------------------------------

/// Every stage binds its body as a parameter, qualifier or no qualifier.
///
/// The only text ever interpolated into a statement here comes from the closed compile-time
/// vocabulary. A selector that is SQL is resolved as a *name*, and the table it names survives.
#[test]
fn a_selector_that_looks_like_sql_is_bound_rather_than_interpolated() {
    let conn = fixture();
    let hostile = "'; DROP TABLE entity; --";
    entity(&conn, "sneaky", "module", hostile, "src/sneaky.ts");

    assert_eq!(resolved_id(&conn, hostile), "sneaky");
    assert_eq!(resolved_id(&conn, &format!("module:{hostile}")), "sneaky");
    for selector in [
        "' OR 1=1 --",
        "symbol:' OR 1=1 --",
        "adr:' OR 1=1 --",
        "src/x.ts#' OR 1=1 --",
    ] {
        let outcome = resolve_selector(&conn, selector).unwrap();
        assert!(
            matches!(outcome, Selection::NotFound { .. }),
            "{selector}: {outcome:?}"
        );
    }

    let entities: i64 = conn
        .query_row("SELECT count(*) FROM entity", [], |row| row.get(0))
        .unwrap();
    assert_eq!(entities, 1, "the entity table is still there, intact");
}
