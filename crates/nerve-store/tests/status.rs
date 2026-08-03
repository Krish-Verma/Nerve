//! The status aggregates, and the one of them that is easy to get quietly wrong.
//!
//! `entities_total` counts rows in `entity`. `symbols_total` counts the subset of those rows the
//! vocabulary calls a symbol. The two were once the same number in the interface — the navigation
//! rail printed `entities_total` beside the word "Symbols" — which meant every repository, every
//! directory, every file, every module, every document, every section, every unresolved reference
//! and, after Slice 6a, every ingested coverage report was being reported to the user as a symbol.
//!
//! The property that makes that impossible is not "the number looks right on a fixture". It is
//! that **adding a non-symbol entity must not move `symbols_total`**, and the tests below assert
//! it over every non-symbol kind in the vocabulary rather than over a chosen example.

use nerve_core::vocab::EntityKind;
use nerve_store::{migrate, open_in_memory, status, Connection};

/// An empty, migrated database with the one repository row every entity is scoped by.
fn empty_repository() -> Connection {
    let conn = open_in_memory().unwrap();
    migrate(&conn).unwrap();
    conn.execute(
        "INSERT INTO repository (repo_id, project_id, root_path, created_at)
         VALUES ('repo', 'project', '/repo', 'now')",
        [],
    )
    .unwrap();
    conn
}

/// One bare entity of the given kind. No occurrence: the counts are over `entity` alone.
fn add_entity(conn: &Connection, id: &str, kind: EntityKind) {
    conn.execute(
        "INSERT INTO entity (entity_id, repo_id, kind, name, scope_path, language)
         VALUES (?1, 'repo', ?2, ?3, '', NULL)",
        rusqlite::params![id, kind.as_str(), id],
    )
    .unwrap();
}

/// The invariant, over every kind the vocabulary does **not** call a symbol.
///
/// Both halves are load-bearing. Without the `entities_total` assertion the test would pass on a
/// query that returned a constant — including the constant `0` — and would therefore not notice
/// the bug it exists to catch. Without the `symbols_total` assertion it is not a test of anything.
#[test]
fn a_non_symbol_entity_never_increases_symbols_total() {
    let conn = empty_repository();

    let before = status(&conn).unwrap();
    assert_eq!(before.symbols_total, 0);
    assert_eq!(before.entities_total, 0);

    let mut expected_entities = 0;
    for kind in EntityKind::ALL.iter().filter(|kind| !kind.is_symbol()) {
        add_entity(&conn, &format!("E_{kind}"), *kind);
        expected_entities += 1;

        let after = status(&conn).unwrap();
        assert_eq!(
            after.symbols_total, 0,
            "a {kind} is not a symbol and must not be counted as one"
        );
        assert_eq!(
            after.entities_total, expected_entities,
            "the {kind} row was inserted, so entities_total must have moved"
        );
    }

    // Nine non-symbol kinds are now in the table and the symbol count has never left zero.
    // `Endpoint` joined them in Slice 10a: a route is a declaration about code, not code, so
    // counting it would make the interface print a number of "symbols" the repository lacks.
    assert_eq!(expected_entities, 9);
    assert_eq!(status(&conn).unwrap().symbols_total, 0);
}

/// The other direction: a symbol entity moves both counts, by one each.
///
/// Stated separately because "never increases" is satisfied by a count that never increases at
/// all, and a `symbols_total` frozen at zero would be a different lie told in the same place.
#[test]
fn a_symbol_entity_increases_both_counts_by_one() {
    let conn = empty_repository();

    let mut expected = 0;
    for kind in EntityKind::ALL.iter().filter(|kind| kind.is_symbol()) {
        add_entity(&conn, &format!("S_{kind}"), *kind);
        expected += 1;

        let after = status(&conn).unwrap();
        assert_eq!(after.symbols_total, expected, "{kind} is a symbol");
        assert_eq!(after.entities_total, expected);
    }
    assert_eq!(expected, 4);
}

/// On a mixed database the two numbers must differ, and `symbols_total` must be the smaller.
///
/// This is the shape of every real repository — no repository consists solely of functions — so
/// the interface binding one figure to the other is always wrong, never merely sometimes wrong.
#[test]
fn symbols_total_is_strictly_below_entities_total_on_a_mixed_database() {
    let conn = empty_repository();
    for kind in EntityKind::ALL {
        add_entity(&conn, &format!("X_{kind}"), kind);
    }

    let report = status(&conn).unwrap();
    assert_eq!(report.entities_total, 13);
    assert_eq!(report.symbols_total, 4);
    assert!(report.symbols_total < report.entities_total);

    // And it agrees with the per-kind breakdown, which is computed by a different statement.
    let summed: i64 = report
        .entities_by_kind
        .iter()
        .filter(|(kind, _)| {
            kind.parse::<EntityKind>()
                .map(EntityKind::is_symbol)
                .unwrap_or(false)
        })
        .map(|(_, count)| *count)
        .sum();
    assert_eq!(report.symbols_total, summed);
}

/// An unmigrated database reports the default, not a query error.
#[test]
fn an_unmigrated_database_reports_no_symbols() {
    let conn = open_in_memory().unwrap();
    let report = status(&conn).unwrap();
    assert_eq!(report.schema_version, None);
    assert_eq!(report.symbols_total, 0);
    assert_eq!(report.entities_total, 0);
}
