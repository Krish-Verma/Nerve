//! Selector resolution against a **real index**, not a hand-built graph.
//!
//! `crates/nerve-store/tests/selectors.rs` pins the rules; this file pins that the rules match
//! what the indexer actually writes. The defect Slice 8b-i exists for was invisible to a
//! hand-built test precisely because it was a mismatch between the resolver's idea of where a
//! path lives (`kind = 'module' AND scope_path = ?`) and where the pipeline puts it — a `File`'s
//! path is `scope_path` joined to `name`, and a `Document` is not a `Module`. So every
//! acceptance criterion that talks about `docs/architecture.md` is checked here, against
//! `fixtures/md-docs` indexed by the real pipeline.

mod common;

use std::path::PathBuf;

use common::{indexed_named_fixture, open_db};

use nerve_store::{
    resolve_selector, Connection, EntityRef, InvalidSelector, Selection, SelectorRefusal,
};

fn indexed() -> ((tempfile::TempDir, PathBuf), Connection) {
    let (held, _) = indexed_named_fixture("md-docs");
    let conn = open_db(&held.1);
    (held, conn)
}

/// Resolve, or fail with what came back instead.
fn resolved(conn: &Connection, selector: &str) -> (EntityRef, &'static str, Vec<EntityRef>) {
    match resolve_selector(conn, selector).unwrap() {
        Selection::Resolved {
            entity,
            matched_by,
            alternatives,
        } => (*entity, matched_by.as_str(), alternatives),
        other => panic!("{selector} must resolve, got {other:?}"),
    }
}

// ---- criterion 1: a document is named by its path ---------------------------------------------

/// `docs/architecture.md` resolves to the `Document`, with the `File` as an alternative.
///
/// Before Slice 8b-i this was `nerve why: "docs/architecture.md" matches no indexed entity` — a
/// document was reachable only as the bare stem `architecture`, and only while nothing else in
/// the repository shared that stem.
#[test]
fn a_document_is_named_by_its_path_and_the_file_is_the_alternative() {
    let (_held, conn) = indexed();

    let (entity, matched_by, alternatives) = resolved(&conn, "docs/architecture.md");
    assert_eq!(entity.kind, "document");
    assert_eq!(entity.name, "architecture");
    assert_eq!(matched_by, "path");

    assert_eq!(alternatives.len(), 1, "{alternatives:?}");
    assert_eq!(alternatives[0].kind, "file");
    assert_eq!(alternatives[0].name, "architecture.md");
    // The passed-over entity is addressable, and the string that addresses it is one the answer
    // itself can print.
    assert_eq!(
        alternatives[0].repository_path().as_deref(),
        Some("docs/architecture.md")
    );
}

/// A document at the repository root, where a container entity's `scope_path` is empty.
#[test]
fn a_document_at_the_root_is_named_by_its_path_too() {
    let (_held, conn) = indexed();
    let (entity, matched_by, alternatives) = resolved(&conn, "README.md");
    assert_eq!(entity.kind, "document");
    assert_eq!(matched_by, "path");
    assert_eq!(alternatives.len(), 1);
    assert_eq!(alternatives[0].kind, "file");
    assert_eq!(alternatives[0].scope_path, "", "root files have no parent");
}

// ---- criterion 2: the container stays addressable ---------------------------------------------

#[test]
fn a_file_is_addressable_by_the_qualifier_the_alternative_names() {
    let (_held, conn) = indexed();
    let (entity, matched_by, alternatives) = resolved(&conn, "file:docs/architecture.md");
    assert_eq!(entity.kind, "file");
    assert_eq!(entity.name, "architecture.md");
    assert_eq!(matched_by, "path");
    assert!(
        alternatives.is_empty(),
        "the qualifier removed the other tier from contention, so nothing was passed over"
    );
}

#[test]
fn a_directory_is_named_by_its_path() {
    let (_held, conn) = indexed();
    for (selector, name) in [("docs", "docs"), ("docs/decisions", "decisions")] {
        let (entity, matched_by, alternatives) = resolved(&conn, selector);
        assert_eq!(entity.kind, "directory", "{selector}");
        assert_eq!(entity.name, name);
        assert_eq!(matched_by, "path");
        assert!(alternatives.is_empty(), "{alternatives:?}");
    }
}

// ---- criterion 3: a module wins, and the answer says so ---------------------------------------

#[test]
fn a_source_path_resolves_to_the_module_and_reports_the_file_it_passed_over() {
    let (_held, conn) = indexed();

    let (entity, matched_by, alternatives) = resolved(&conn, "src/app.ts");
    assert_eq!(entity.kind, "module");
    assert_eq!(matched_by, "path");
    assert_eq!(alternatives.len(), 1, "{alternatives:?}");
    assert_eq!(alternatives[0].kind, "file");
    assert_eq!(
        alternatives[0].repository_path().as_deref(),
        Some("src/app.ts")
    );

    // And the alternative resolves to exactly the entity that was passed over.
    let (file, _, _) = resolved(&conn, "file:src/app.ts");
    assert_eq!(file.entity_id, alternatives[0].entity_id);
}

// ---- criterion 4: a qualifier that excludes everything says what is there ---------------------

#[test]
fn a_wrong_qualifier_is_a_miss_that_names_what_is_actually_there() {
    let (_held, conn) = indexed();
    match resolve_selector(&conn, "module:docs/architecture.md").unwrap() {
        Selection::NotFound {
            qualifier,
            excluded,
            ..
        } => {
            assert_eq!(qualifier.map(|q| q.as_str()), Some("module"));
            let kinds: Vec<&str> = excluded.iter().map(|e| e.kind.as_str()).collect();
            assert!(
                kinds.contains(&"document"),
                "the refusal must say a document is there: {kinds:?}"
            );
        }
        other => panic!("expected a qualified miss, got {other:?}"),
    }
}

// ---- criterion 5: malformed is not missing ----------------------------------------------------

#[test]
fn an_unknown_qualifier_is_invalid_rather_than_not_found() {
    let (_held, conn) = indexed();
    assert!(matches!(
        resolve_selector(&conn, "banana:foo").unwrap(),
        Selection::Invalid {
            reason: InvalidSelector::UnknownQualifier
        }
    ));
    // And the resolver never falls back to treating it as a bare name, even when a matching
    // entity called `banana:foo` could not exist but the body alone would resolve.
    assert!(matches!(
        resolve_selector(&conn, "banana:architecture").unwrap(),
        Selection::Invalid { .. }
    ));
}

// ---- criterion 6: an ADR by its identifier ----------------------------------------------------

#[test]
fn an_adr_resolves_by_the_identifier_that_lives_in_its_metadata() {
    let (_held, conn) = indexed();

    let (entity, matched_by, _) = resolved(&conn, "adr:ADR-0001");
    assert_eq!(entity.kind, "document");
    assert_eq!(entity.name, "ADR-0001-header-status");
    assert_eq!(matched_by, "name");

    // Every ADR in the fixture, so the alias is not one lucky row.
    for id in ["ADR-0001", "ADR-0002", "ADR-0003"] {
        let (entity, _, _) = resolved(&conn, &format!("adr:{id}"));
        assert!(entity.name.starts_with(id), "{id} -> {}", entity.name);
    }

    // A document that is not an ADR is not reachable through the alias, and `plain-note` — an
    // ADR-shaped document with no id — is not reachable by an id it does not have.
    for selector in ["adr:architecture", "adr:ADR-0009", "adr:README"] {
        assert!(
            matches!(
                resolve_selector(&conn, selector).unwrap(),
                Selection::NotFound { .. }
            ),
            "{selector} must not resolve"
        );
    }
    // `plain-note` *is* flagged as an ADR, so the alias reaches it by name.
    let (note, _, _) = resolved(&conn, "adr:plain-note");
    assert_eq!(note.name, "plain-note");
}

// ---- criterion 7: `symbol:` excludes what is not a symbol ------------------------------------

#[test]
fn the_symbol_alias_takes_a_module_or_document_out_of_contention() {
    let (_held, conn) = indexed();

    // `app` is the module's name; unqualified it resolves to the module.
    let (module, _, _) = resolved(&conn, "app");
    assert_eq!(module.kind, "module");

    // Qualified, the module is not a candidate at all — and the fixture has no symbol by that
    // name, so the answer is a miss that says which qualifier was applied.
    match resolve_selector(&conn, "symbol:app").unwrap() {
        Selection::NotFound {
            qualifier,
            excluded,
            ..
        } => {
            assert_eq!(qualifier.map(|q| q.as_str()), Some("symbol"));
            assert!(excluded.iter().any(|e| e.kind == "module"), "{excluded:?}");
        }
        other => panic!("expected a qualified miss, got {other:?}"),
    }

    // A real symbol still resolves through the alias.
    let (entity, _, _) = resolved(&conn, "symbol:run");
    assert!(
        entity
            .kind
            .parse::<nerve_core::EntityKind>()
            .unwrap()
            .is_symbol(),
        "{entity:?}"
    );
}

// ---- criterion 8: real ambiguity still refuses ------------------------------------------------

/// `describe` is a function in `src/util.ts` and a method on `Describer`. Nothing is chosen.
///
/// This is the case the two-tier rule must **not** reach: two entities the tool genuinely cannot
/// tell apart, in one stage, with no rule that distinguishes them.
#[test]
fn two_symbols_with_one_name_remain_ambiguous() {
    let (_held, conn) = indexed();
    match resolve_selector(&conn, "describe").unwrap() {
        Selection::Ambiguous {
            candidates,
            matched_by,
        } => {
            assert!(candidates.len() >= 2, "{candidates:?}");
            assert_eq!(matched_by.as_str(), "name");
        }
        other => panic!("expected ambiguity, got {other:?}"),
    }
    // Qualifying by kind is the way out, and it is the caller's decision rather than Nerve's.
    let (function, _, _) = resolved(&conn, "function:describe");
    assert_eq!(function.kind, "function");
    let (method, _, _) = resolved(&conn, "method:describe");
    assert_eq!(method.kind, "method");
    // The alias covers both, so it does not disambiguate — and must not pretend to.
    assert!(matches!(
        resolve_selector(&conn, "symbol:describe").unwrap(),
        Selection::Ambiguous { .. }
    ));
}

// ---- criterion 9 and 10: refusal, and what is not refused ------------------------------------

#[test]
fn a_traversal_selector_is_refused_by_the_store_rather_than_missed() {
    let (_held, conn) = indexed();
    for selector in [
        "../../etc/passwd",
        "/etc/passwd",
        "./../x",
        "src/../../../etc/passwd#Circle.area",
        "file:/etc/passwd",
    ] {
        assert!(
            matches!(
                resolve_selector(&conn, selector).unwrap(),
                Selection::Refused {
                    reason: SelectorRefusal::Traversal
                }
            ),
            "{selector} must be refused"
        );
    }
}

#[test]
fn a_unicode_path_and_a_dotted_segment_are_resolved_rather_than_refused() {
    let (_held, conn) = indexed();
    // Neither is in the fixture, so neither resolves — but neither is *refused*, which is the
    // distinction the criterion is about.
    for selector in ["docs/архитектура.md", "docs/a..b.md", "architecture"] {
        let outcome = resolve_selector(&conn, selector).unwrap();
        assert!(
            !matches!(outcome, Selection::Refused { .. }),
            "{selector} must not be refused: {outcome:?}"
        );
    }
}

// ---- criterion 11: every suggestion can be typed back ----------------------------------------

/// A suggestion is offered as something to type, so typing it must reach an entity.
///
/// The two private copies of the scope fold printed `docs.architecture.md` beside the word
/// *document* and `file` beside `docs.architecture.md`; both were copied verbatim by the person
/// reading them and both came back "matches no indexed entity". This walks every suggestion the
/// resolver produces for a set of near-miss selectors and re-resolves it.
#[test]
fn every_suggestion_a_miss_offers_resolves_when_it_is_typed_back() {
    let (_held, conn) = indexed();

    let mut checked = 0;
    for selector in [
        "architectur",
        "architecutre",
        "ADR-000",
        "READ",
        "app.t",
        "decision",
    ] {
        let Selection::NotFound { suggestions, .. } = resolve_selector(&conn, selector).unwrap()
        else {
            continue;
        };
        for hit in &suggestions {
            let typed = hit.qualified_name();
            let outcome = resolve_selector(&conn, &typed).unwrap();
            assert!(
                matches!(
                    outcome,
                    Selection::Resolved { .. } | Selection::Ambiguous { .. }
                ),
                "suggestion {typed:?} ({}) offered for {selector:?} does not resolve: {outcome:?}",
                hit.kind
            );
            checked += 1;
        }
    }
    assert!(checked >= 5, "only {checked} suggestions were exercised");
}

/// The specific string the defect report quoted, and the one it should have been.
#[test]
fn a_document_suggestion_is_its_own_name_rather_than_a_dotted_path() {
    let (_held, conn) = indexed();
    let Selection::NotFound { suggestions, .. } = resolve_selector(&conn, "architectur").unwrap()
    else {
        panic!("`architectur` must miss");
    };
    let document = suggestions
        .iter()
        .find(|hit| hit.kind == "document")
        .expect("the document must be suggested");
    assert_eq!(document.qualified_name(), "architecture");
    assert_ne!(
        document.qualified_name(),
        "docs/architecture.md.architecture"
    );

    let file = suggestions.iter().find(|hit| hit.kind == "file");
    if let Some(file) = file {
        assert_eq!(file.qualified_name(), "architecture.md");
        assert_ne!(file.qualified_name(), "docs.architecture.md");
    }
}

// ---- what did not change ----------------------------------------------------------------------

/// The three stages Slice 8b-i did not touch still answer exactly as they did.
#[test]
fn the_other_stages_are_unchanged() {
    let (_held, conn) = indexed();

    // An entity id wins over everything.
    let (document, _, _) = resolved(&conn, "docs/architecture.md");
    let (by_id, matched_by, _) = resolved(&conn, &document.entity_id);
    assert_eq!(matched_by, "entity_id");
    assert_eq!(by_id.entity_id, document.entity_id);

    // `<rel_path>#<qualified_name>` still finds a symbol inside a file.
    let (symbol, matched_by, _) = resolved(&conn, "src/app.ts#run");
    assert_eq!(matched_by, "path_qualified");
    assert_eq!(symbol.name, "run");

    // A bare unique name still resolves.
    let (named, matched_by, _) = resolved(&conn, "run");
    assert_eq!(matched_by, "name");
    assert_eq!(named.entity_id, symbol.entity_id);
}
