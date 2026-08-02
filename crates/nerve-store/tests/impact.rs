//! Reverse dependency closure, over graphs built directly in SQL.
//!
//! These tests do not go through the indexer, for the same reason `tests/graph.rs` does not: the
//! point is to pin the semantics — closure membership, depth, cycles, ordering, bounds, the
//! relation set, and the unresolved account — against graphs small enough to reason about
//! completely. The end-to-end behaviour on a real repository is `crates/nerve-cli/tests/cli.rs`.

use std::collections::BTreeMap;

use nerve_core::vocab::Relation;
use nerve_store::{
    impact, migrate, open_in_memory, rebuild_assertion_state, Connection, FileProbe, FileProber,
    Freshness, ImpactQuery, ImpactReport, DEFAULT_IMPACT_RELATIONS,
};

const STATE: &str = "state-1";
const HASH: &str = "hash-of-file";

/// A prober that answers from a table, so freshness can be tested without a filesystem.
struct StubProber {
    answers: BTreeMap<String, FileProbe>,
}

impl FileProber for StubProber {
    fn probe(&self, rel_path: &str) -> FileProbe {
        self.answers
            .get(rel_path)
            .cloned()
            .unwrap_or(FileProbe::Hash(HASH.to_string()))
    }
}

/// Everything is fresh unless a test says otherwise.
fn fresh() -> StubProber {
    StubProber {
        answers: BTreeMap::new(),
    }
}

fn prober(pairs: &[(&str, FileProbe)]) -> StubProber {
    StubProber {
        answers: pairs
            .iter()
            .map(|(path, probe)| ((*path).to_string(), probe.clone()))
            .collect(),
    }
}

fn skeleton() -> Connection {
    let conn = open_in_memory().unwrap();
    migrate(&conn).unwrap();
    conn.execute(
        "INSERT INTO repository (repo_id, project_id, root_path, created_at)
         VALUES ('repo', 'project', '/repo', 'now')",
        [],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO repository_state (state_id, repo_id, kind, content_merkle, created_at)
         VALUES (?1, 'repo', 'working-tree', 'merkle', 'now')",
        rusqlite::params![STATE],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO extractor_run (run_id, repo_id, state_id, extractor_id, extractor_version,
                                    started_at, status)
         VALUES (1, 'repo', ?1, 'test-extractor', '9.9.9', 'now', 'complete')",
        rusqlite::params![STATE],
    )
    .unwrap();
    conn
}

fn entity(conn: &Connection, id: &str, kind: &str, name: &str, file: &str, line: i64) {
    conn.execute(
        "INSERT INTO entity (entity_id, repo_id, kind, name, scope_path, language)
         VALUES (?1, 'repo', ?2, ?3, '', 'typescript')",
        rusqlite::params![id, kind, name],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO occurrence (occurrence_id, entity_id, file_path, start_byte,
                                 end_byte, start_line, start_col, end_line, end_col, content_hash)
         VALUES (?1, ?2, ?3, 0, 1, ?4, 0, ?4, 1, ?5)",
        rusqlite::params![format!("occ-{id}"), id, file, line, HASH],
    )
    .unwrap();
}

/// An `Unresolved` entity carrying the category its identity tuple was built from.
fn unresolved(conn: &Connection, id: &str, name: &str, file: &str, category: &str) {
    conn.execute(
        "INSERT INTO entity (entity_id, repo_id, kind, name, scope_path, language, meta)
         VALUES (?1, 'repo', 'unresolved', ?2, ?3, NULL, ?4)",
        rusqlite::params![
            id,
            name,
            file,
            format!(r#"{{"category":"{category}","importer":"{file}"}}"#)
        ],
    )
    .unwrap();
}

/// One assertion with `sites` observations behind it — a site is an observation, not an edge.
fn edge_with_sites(
    conn: &Connection,
    source: &str,
    relation: Relation,
    target: &str,
    file: &str,
    line: i64,
    sites: i64,
) {
    let assertion_id = format!("a-{source}-{}-{target}", relation.as_str());
    conn.execute(
        "INSERT INTO assertion (assertion_id, repo_id, source_entity_id, relation,
                                target_entity_id)
         VALUES (?1, 'repo', ?2, ?3, ?4)",
        rusqlite::params![assertion_id, source, relation.as_str(), target],
    )
    .unwrap();
    for site in 0..sites {
        conn.execute(
            "INSERT INTO observation (assertion_id, extractor_run_id, evidence_source_type,
                                      directness, extractor_id, extractor_version,
                                      file_path, start_line, end_line, content_hash, created_at)
             VALUES (?1, 1, 'AST_RESOLVED', 'RESOLVED', 'test-extractor', '9.9.9', ?2, ?3, ?3,
                     ?4, '2026-08-02T00:00:00Z')",
            rusqlite::params![assertion_id, file, line + site, HASH],
        )
        .unwrap();
    }
}

fn edge(conn: &Connection, source: &str, relation: Relation, target: &str, file: &str, line: i64) {
    edge_with_sites(conn, source, relation, target, file, line, 1);
}

/// ```text
///   E --CALLS--> D --CALLS--> B --CALLS--> LEAF
///                C --CALLS--> B
///                    SUB --EXTENDS--> BASE --CALLS--> LEAF
///   FILE --CONTAINS--> LEAF        (structural: must never appear in an impact set)
///   DIR  --CONTAINS--> FILE
///   MOD  --DEFINES-->  LEAF
///   IMPORTER --IMPORTS--> MOD
///   RUN  --COVERS-->   LEAF
///   ORPHAN is isolated
/// ```
fn fixture() -> Connection {
    let conn = skeleton();

    entity(&conn, "LEAF", "function", "leaf", "src/leaf.ts", 1);
    entity(&conn, "B", "function", "bravo", "src/b.ts", 1);
    entity(&conn, "C", "function", "charlie", "src/c.ts", 1);
    entity(&conn, "D", "function", "delta", "src/d.ts", 1);
    entity(&conn, "E", "function", "echo", "src/e.ts", 1);
    entity(&conn, "BASE", "class", "Base", "src/base.ts", 1);
    entity(&conn, "SUB", "class", "Sub", "src/sub.ts", 1);
    entity(&conn, "ORPHAN", "function", "orphan", "src/orphan.ts", 1);

    entity(&conn, "FILE", "file", "leaf.ts", "src/leaf.ts", 1);
    entity(&conn, "DIR", "directory", "src", "src", 1);
    entity(&conn, "MOD", "module", "leaf", "src/leaf.ts", 1);
    entity(&conn, "IMPORTER", "module", "importer", "src/imp.ts", 1);
    entity(&conn, "RUN", "coverage_run", "lcov.info", "lcov.info", 1);

    edge(&conn, "B", Relation::Calls, "LEAF", "src/b.ts", 10);
    edge(&conn, "BASE", Relation::Calls, "LEAF", "src/base.ts", 10);
    edge(&conn, "C", Relation::Calls, "B", "src/c.ts", 10);
    edge(&conn, "D", Relation::Calls, "B", "src/d.ts", 10);
    edge(&conn, "E", Relation::Calls, "D", "src/e.ts", 10);
    edge(&conn, "SUB", Relation::Extends, "BASE", "src/sub.ts", 10);

    // Structural and coverage edges the default set must refuse.
    edge(&conn, "FILE", Relation::Contains, "LEAF", "src/leaf.ts", 1);
    edge(&conn, "DIR", Relation::Contains, "FILE", "src", 1);
    edge(&conn, "MOD", Relation::Defines, "LEAF", "src/leaf.ts", 1);
    edge(&conn, "IMPORTER", Relation::Imports, "MOD", "src/imp.ts", 1);
    edge(&conn, "RUN", Relation::Covers, "LEAF", "lcov.info", 1);

    rebuild_assertion_state(&conn).unwrap();
    conn
}

fn names(report: &ImpactReport) -> Vec<String> {
    report
        .results
        .iter()
        .map(|row| row.entity.name.clone())
        .collect()
}

fn depths(report: &ImpactReport) -> Vec<(String, usize)> {
    report
        .results
        .iter()
        .map(|row| (row.entity.name.clone(), row.depth))
        .collect()
}

fn ask(conn: &Connection, subject: &str) -> ImpactReport {
    impact(conn, subject, &ImpactQuery::default(), &fresh()).unwrap()
}

// ---- closure ---------------------------------------------------------------------------------

/// The whole point: transitive, not one hop.
#[test]
fn the_closure_is_transitive_and_carries_the_depth_it_was_reached_at() {
    let conn = fixture();
    let report = ask(&conn, "LEAF");

    assert_eq!(
        depths(&report),
        vec![
            ("Base".to_string(), 1),
            ("bravo".to_string(), 1),
            ("Sub".to_string(), 2),
            ("charlie".to_string(), 2),
            ("delta".to_string(), 2),
            ("echo".to_string(), 3),
        ]
    );
    assert_eq!(report.totals.entities, 6);
    assert_eq!(report.totals.by_depth.get(&1), Some(&2));
    assert_eq!(report.totals.by_depth.get(&2), Some(&3));
    assert_eq!(report.totals.by_depth.get(&3), Some(&1));
    assert!(!report.truncated);
    assert_eq!(report.subject.name, "leaf");
}

/// The subject is the question, not an answer, and it is never listed as depending on itself.
#[test]
fn the_subject_is_never_in_its_own_impact_set() {
    let conn = fixture();
    let report = ask(&conn, "LEAF");
    assert!(
        !report
            .results
            .iter()
            .any(|row| row.entity.entity_id == "LEAF"),
        "{:?}",
        names(&report)
    );
    for row in &report.results {
        assert!(row.depth >= 1, "a reported entity is at least one hop away");
    }
}

/// Nothing depends on it is a finding, not an error, and not an empty struct either.
#[test]
fn an_entity_nothing_depends_on_has_an_empty_closure_and_a_full_report() {
    let conn = fixture();
    let report = ask(&conn, "ORPHAN");
    assert!(report.results.is_empty());
    assert_eq!(report.results_total, 0);
    assert_eq!(report.totals.entities, 0);
    assert!(!report.truncated);
    // And the caveat is still there. This is the most dangerous answer to report bare.
    assert_eq!(report.unresolved.sites, 0);
    assert_eq!(report.relations, DEFAULT_IMPACT_RELATIONS.to_vec());
}

/// Every row says how it was reached: relation, direction, evidence and freshness.
#[test]
fn every_row_carries_the_edge_that_reached_it() {
    let conn = fixture();
    let report = ask(&conn, "LEAF");

    let bravo = report
        .results
        .iter()
        .find(|row| row.entity.name == "bravo")
        .expect("bravo calls leaf");
    assert_eq!(bravo.relation, "CALLS");
    assert_eq!(bravo.reached_entity_id, "LEAF");
    assert_eq!(bravo.direction.as_str(), "outgoing");
    assert_eq!(bravo.assertion_id, "a-B-CALLS-LEAF");
    assert_eq!(bravo.status.as_deref(), Some("SUPPORTED"));
    assert_eq!(bravo.strongest_source_type.as_deref(), Some("AST_RESOLVED"));
    assert_eq!(bravo.observation_count, 1);
    assert!(!bravo.is_unresolved);
    assert_eq!(bravo.location(), "src/b.ts:10");
    assert_eq!(bravo.evidence_freshness, Some(Freshness::Fresh));

    let sub = report
        .results
        .iter()
        .find(|row| row.entity.name == "Sub")
        .expect("Sub extends Base, which calls leaf");
    assert_eq!(sub.relation, "EXTENDS");
    assert_eq!(sub.reached_entity_id, "BASE");
    assert_eq!(sub.depth, 2);
}

/// The direction is invariant, and that invariance is the claim being made.
///
/// A reverse closure admits an entity only through an edge that entity itself asserts, so every
/// row is `outgoing` when read against its own entity. Reporting it keeps a row that has been
/// lifted out of the list from being mistaken for a forward dependency.
#[test]
fn every_reached_edge_is_outgoing_from_the_entity_that_asserts_it() {
    let conn = fixture();
    let report = ask(&conn, "LEAF");
    assert!(!report.results.is_empty());
    for row in &report.results {
        assert_eq!(row.direction.as_str(), "outgoing", "{}", row.entity.name);
    }
}

/// Freshness is computed by re-hashing at query time, never read from the row.
#[test]
fn evidence_freshness_is_computed_from_the_file() {
    let conn = fixture();
    let files = prober(&[
        ("src/b.ts", FileProbe::Hash("moved-on".into())),
        ("src/c.ts", FileProbe::Missing),
    ]);
    let report = impact(&conn, "LEAF", &ImpactQuery::default(), &files).unwrap();

    let by_name: BTreeMap<&str, Option<Freshness>> = report
        .results
        .iter()
        .map(|row| (row.entity.name.as_str(), row.evidence_freshness))
        .collect();
    assert_eq!(by_name["bravo"], Some(Freshness::Stale));
    assert_eq!(by_name["charlie"], Some(Freshness::FileMissing));
    assert_eq!(by_name["delta"], Some(Freshness::Fresh));
    assert_eq!(
        report.totals.stale, 2,
        "stale and missing are both not-fresh"
    );
    assert!(report.files_probed > 0);
}

// ---- bounds ----------------------------------------------------------------------------------

#[test]
fn the_depth_bound_is_respected() {
    let conn = fixture();
    for (max_depth, expected) in [
        (1, vec!["Base", "bravo"]),
        (2, vec!["Base", "bravo", "Sub", "charlie", "delta"]),
        (3, vec!["Base", "bravo", "Sub", "charlie", "delta", "echo"]),
    ] {
        let query = ImpactQuery {
            max_depth,
            ..ImpactQuery::default()
        };
        let report = impact(&conn, "LEAF", &query, &fresh()).unwrap();
        assert_eq!(names(&report), expected, "at depth {max_depth}");
        assert_eq!(report.max_depth, max_depth);
        assert_eq!(report.totals.entities, expected.len());
    }
}

/// The cap cuts rows. It never touches the tallies, and it admits that it cut.
#[test]
fn truncation_caps_rows_while_the_totals_stay_exact() {
    let conn = fixture();
    let query = ImpactQuery {
        limit: 2,
        ..ImpactQuery::default()
    };
    let report = impact(&conn, "LEAF", &query, &fresh()).unwrap();

    assert_eq!(report.results.len(), 2);
    assert!(report.truncated);
    assert_eq!(report.results_total, 6);
    assert_eq!(report.limit, 2);
    assert_eq!(
        report.totals.entities, 6,
        "the tally is exact whatever the cap cuts"
    );
    assert_eq!(report.totals.by_depth.values().sum::<usize>(), 6);
    assert_eq!(report.totals.by_relation.get("CALLS"), Some(&5));
    assert_eq!(report.totals.by_relation.get("EXTENDS"), Some(&1));
    // Nearest first, so a cap keeps the closest dependents rather than an arbitrary two.
    assert_eq!(names(&report), vec!["Base", "bravo"]);

    let uncapped = ask(&conn, "LEAF");
    assert!(!uncapped.truncated);
    assert_eq!(uncapped.results.len(), uncapped.results_total);
}

#[test]
fn the_same_question_gives_a_byte_identical_answer() {
    let conn = fixture();
    let first = ask(&conn, "LEAF");
    for _ in 0..8 {
        assert_eq!(ask(&conn, "LEAF"), first);
    }
}

// ---- relations -------------------------------------------------------------------------------

/// A function's impact is not its file, its directory, its module or the repository.
///
/// This is the pushback in `docs/plans/slice-07b-impact.md` §1 made into a test. With `CONTAINS`
/// or `DEFINES` in the default set every symbol reaches the repository itself, which is true and
/// useless, and it buries the edges that carry the actual answer.
#[test]
fn the_default_relation_set_excludes_containment_definition_imports_and_coverage() {
    let conn = fixture();
    let report = ask(&conn, "LEAF");

    for refused in ["leaf.ts", "src", "leaf", "importer", "lcov.info"] {
        assert!(
            !names(&report).contains(&refused.to_string()),
            "{refused} reached the impact set of a function: {:?}",
            names(&report)
        );
    }
    for kind in ["file", "directory", "module", "repository", "coverage_run"] {
        assert_eq!(
            report.totals.by_kind.get(kind),
            None,
            "a {kind} is in a symbol's impact set: {:?}",
            report.totals.by_kind
        );
    }
    assert_eq!(report.totals.by_kind.get("function"), Some(&4));
    assert_eq!(report.totals.by_kind.get("class"), Some(&2));
}

/// `--relation` replaces the default with a set from the closed vocabulary.
#[test]
fn an_explicit_relation_set_replaces_the_default() {
    let conn = fixture();

    let extends_only = impact(
        &conn,
        "BASE",
        &ImpactQuery {
            relations: vec![Relation::Extends],
            ..ImpactQuery::default()
        },
        &fresh(),
    )
    .unwrap();
    assert_eq!(names(&extends_only), vec!["Sub"]);
    assert_eq!(extends_only.relations, vec![Relation::Extends]);

    // The conservative module closure is available on request, and only on request.
    let imports = impact(
        &conn,
        "MOD",
        &ImpactQuery {
            relations: vec![Relation::Imports],
            ..ImpactQuery::default()
        },
        &fresh(),
    )
    .unwrap();
    assert_eq!(names(&imports), vec!["importer"]);
    assert!(
        ask(&conn, "MOD").results.is_empty(),
        "the default set does not follow IMPORTS"
    );
}

/// An empty list is the default four, not "every relation".
#[test]
fn an_empty_relation_list_does_not_open_the_walk_to_containment() {
    let conn = fixture();
    let report = impact(
        &conn,
        "LEAF",
        &ImpactQuery {
            relations: Vec::new(),
            ..ImpactQuery::default()
        },
        &fresh(),
    )
    .unwrap();
    assert_eq!(report.relations, DEFAULT_IMPACT_RELATIONS.to_vec());
    assert_eq!(report.totals.by_kind.get("file"), None);
    assert_eq!(names(&report), names(&ask(&conn, "LEAF")));
}

// ---- cycles ----------------------------------------------------------------------------------

/// A cycle terminates, and every entity in it appears exactly once.
///
/// `find_paths` keeps paths *simple* because it enumerates alternative routes. A closure cannot
/// do that: without a global visited set the walk revisits `X → Y → Z → X` forever, and with a
/// per-path one it would report the same entity several times at several depths.
#[test]
fn a_cycle_terminates_and_each_entity_appears_exactly_once() {
    let conn = skeleton();
    entity(&conn, "X", "function", "ex", "src/x.ts", 1);
    entity(&conn, "Y", "function", "why", "src/y.ts", 1);
    entity(&conn, "Z", "function", "zed", "src/z.ts", 1);
    entity(&conn, "OUT", "function", "outside", "src/out.ts", 1);
    // X -> Y -> Z -> X, plus a self-loop, plus one dependent hanging off the ring.
    edge(&conn, "X", Relation::Calls, "Y", "src/x.ts", 10);
    edge(&conn, "Y", Relation::Calls, "Z", "src/y.ts", 10);
    edge(&conn, "Z", Relation::Calls, "X", "src/z.ts", 10);
    edge(&conn, "Y", Relation::Calls, "Y", "src/y.ts", 11);
    edge(&conn, "OUT", Relation::Calls, "Z", "src/out.ts", 10);
    rebuild_assertion_state(&conn).unwrap();

    let query = ImpactQuery {
        max_depth: 32,
        ..ImpactQuery::default()
    };
    let report = impact(&conn, "X", &query, &fresh()).unwrap();

    assert_eq!(
        depths(&report),
        vec![
            ("zed".to_string(), 1),
            ("outside".to_string(), 2),
            ("why".to_string(), 2),
        ]
    );
    assert!(
        !report.results.iter().any(|row| row.entity.name == "ex"),
        "the subject must not come back round the ring as its own dependent"
    );
    let mut ids: Vec<&str> = report
        .results
        .iter()
        .map(|row| row.entity.entity_id.as_str())
        .collect();
    let before = ids.len();
    ids.sort_unstable();
    ids.dedup();
    assert_eq!(ids.len(), before, "an entity appeared twice");
    assert_eq!(report.totals.entities, 3);
}

/// A re-export chain: the dependent is several modules away from the definition.
///
/// `plus` is `add` re-exported through a barrel and then star-exported again. Nerve resolves the
/// call site through the chain, so the caller lands in the closure of the *definition* — which is
/// exactly the case grep gets wrong, because the caller never writes the name `add`.
#[test]
fn a_re_export_chain_is_followed_to_the_original_definition() {
    let conn = skeleton();
    entity(&conn, "ADD", "function", "add", "src/math.ts", 1);
    entity(&conn, "BARREL", "module", "barrel", "src/barrel.ts", 1);
    entity(&conn, "INDEX", "module", "index", "src/index.ts", 1);
    entity(&conn, "APP", "function", "viaBarrel", "src/app.ts", 1);
    entity(
        &conn,
        "OUTER",
        "function",
        "callsViaBarrel",
        "src/outer.ts",
        1,
    );

    // The re-export hops are module-level; the resolved call jumps straight to the definition.
    edge(
        &conn,
        "BARREL",
        Relation::Exports,
        "ADD",
        "src/barrel.ts",
        1,
    );
    edge(&conn, "INDEX", Relation::Exports, "ADD", "src/index.ts", 1);
    edge(&conn, "APP", Relation::Calls, "ADD", "src/app.ts", 9);
    edge(&conn, "OUTER", Relation::Calls, "APP", "src/outer.ts", 3);
    rebuild_assertion_state(&conn).unwrap();

    let report = ask(&conn, "ADD");
    assert_eq!(
        depths(&report),
        vec![
            ("viaBarrel".to_string(), 1),
            ("callsViaBarrel".to_string(), 2),
        ]
    );
    // The barrel modules are `EXPORTS`, which is not a dependency on the symbol: re-exporting a
    // function does not break when its body changes.
    assert!(!names(&report).contains(&"barrel".to_string()));
    assert!(!names(&report).contains(&"index".to_string()));
}

// ---- the unresolved account ------------------------------------------------------------------

/// The account counts **sites**, not edges, and splits by what kind of thing failed to resolve.
#[test]
fn the_unresolved_account_counts_sites_and_splits_by_category() {
    let conn = skeleton();
    entity(&conn, "TARGET", "function", "target", "src/t.ts", 1);
    entity(&conn, "CALLER", "function", "caller", "src/caller.ts", 1);
    entity(
        &conn,
        "TYPED",
        "function",
        "typedParameter",
        "src/typed.ts",
        1,
    );
    entity(&conn, "DOC", "document", "notes", "docs/notes.md", 1);
    unresolved(&conn, "U_AREA", "area", "src/typed.ts", "value");
    unresolved(&conn, "U_LOG", "console.log", "src/typed.ts", "value");
    unresolved(
        &conn,
        "U_LINK",
        "./gone.md",
        "docs/notes.md",
        "document_link",
    );
    unresolved(&conn, "U_PKG", "lodash", "src/typed.ts", "module");

    edge(
        &conn,
        "CALLER",
        Relation::Calls,
        "TARGET",
        "src/caller.ts",
        5,
    );
    // Three call sites on one unresolvable receiver method collapse to one assertion.
    edge_with_sites(
        &conn,
        "TYPED",
        Relation::Calls,
        "U_AREA",
        "src/typed.ts",
        5,
        3,
    );
    edge_with_sites(
        &conn,
        "TYPED",
        Relation::Calls,
        "U_LOG",
        "src/typed.ts",
        20,
        2,
    );
    edge(
        &conn,
        "DOC",
        Relation::References,
        "U_LINK",
        "docs/notes.md",
        4,
    );
    // An unresolved module specifier: reached only when the walk follows IMPORTS.
    edge(
        &conn,
        "TYPED",
        Relation::Imports,
        "U_PKG",
        "src/typed.ts",
        1,
    );
    rebuild_assertion_state(&conn).unwrap();

    let report = ask(&conn, "TARGET");
    assert_eq!(names(&report), vec!["caller"]);

    let account = &report.unresolved;
    assert_eq!(account.sites, 6, "3 + 2 call sites, plus one document link");
    assert_eq!(account.assertions, 3);
    assert_eq!(account.targets, 3);
    assert_eq!(account.by_category.get("value"), Some(&5));
    assert_eq!(account.by_category.get("document_link"), Some(&1));
    assert_eq!(
        account.by_category.get("module"),
        None,
        "an unresolved IMPORTS specifier is outside the relations this walk followed"
    );
    assert!(!account.is_empty());

    // Asking for IMPORTS changes which silence is relevant, and the account follows.
    let imports = impact(
        &conn,
        "TARGET",
        &ImpactQuery {
            relations: vec![Relation::Imports],
            ..ImpactQuery::default()
        },
        &fresh(),
    )
    .unwrap();
    assert_eq!(imports.unresolved.by_category.get("module"), Some(&1));
    assert_eq!(imports.unresolved.by_category.get("value"), None);
    assert_eq!(imports.unresolved.sites, 1);
}

/// The easy bug: omitting the account when there is nothing to say.
///
/// Zero is a measurement — "every reference site in this repository resolved, so nothing is
/// hidden from this answer by a failed resolution" — and it is the case where a reader most needs
/// to be told, because an empty impact set with no caveat reads as *safe to change*.
#[test]
fn the_unresolved_account_is_present_when_it_is_zero() {
    let conn = fixture();
    for subject in ["LEAF", "ORPHAN", "B"] {
        let report = ask(&conn, subject);
        assert!(report.unresolved.is_empty(), "{subject}");
        assert_eq!(report.unresolved.sites, 0, "{subject}");
        assert_eq!(report.unresolved.assertions, 0, "{subject}");
        assert_eq!(report.unresolved.targets, 0, "{subject}");
        assert!(report.unresolved.by_category.is_empty(), "{subject}");
    }
}

/// An unresolved-heavy repository: the visible answer is small and the caveat is not.
///
/// This is the shape the whole slice exists for. Two resolved dependents, sixteen sites that
/// resolved to nothing — and no attempt anywhere to guess which of the sixteen might be a call to
/// the subject. Nothing in the report compares a name to a name.
#[test]
fn an_unresolved_heavy_repository_reports_a_small_answer_beside_a_large_caveat() {
    let conn = skeleton();
    entity(
        &conn,
        "PARSE",
        "function",
        "parseConfig",
        "src/config.ts",
        1,
    );
    entity(&conn, "KNOWN", "function", "known", "src/known.ts", 1);
    entity(&conn, "ALSO", "function", "alsoKnown", "src/also.ts", 1);
    edge(&conn, "KNOWN", Relation::Calls, "PARSE", "src/known.ts", 5);
    edge(&conn, "ALSO", Relation::Calls, "KNOWN", "src/also.ts", 5);

    // Eight callers, each with two sites on a receiver whose type Nerve cannot infer. One of them
    // is literally named `parseConfig` — and it is still not offered as a suggestion.
    for index in 0..8 {
        let caller = format!("BLIND{index}");
        let target = format!("U{index}");
        let file = format!("src/blind{index}.ts");
        entity(&conn, &caller, "function", &caller, &file, 1);
        let name = if index == 0 {
            "parseConfig"
        } else {
            "somethingElse"
        };
        unresolved(&conn, &target, name, &file, "value");
        edge_with_sites(&conn, &caller, Relation::Calls, &target, &file, 5, 2);
    }
    rebuild_assertion_state(&conn).unwrap();

    let report = ask(&conn, "PARSE");
    assert_eq!(
        depths(&report),
        vec![("known".to_string(), 1), ("alsoKnown".to_string(), 2)]
    );
    assert_eq!(report.totals.entities, 2);
    assert_eq!(report.unresolved.sites, 16);
    assert_eq!(report.unresolved.assertions, 8);
    assert_eq!(report.unresolved.targets, 8);
    assert_eq!(report.unresolved.by_category.get("value"), Some(&16));

    // The name-coincident site is counted as a site and named as nothing.
    for row in &report.results {
        assert_ne!(row.entity.entity_id, "BLIND0");
        assert!(!row.entity.kind.contains("unresolved"));
    }
}

/// A subject that is itself unresolved is answerable, and its edges say so.
#[test]
fn an_unresolved_subject_reports_the_edges_that_point_at_it_as_unresolved() {
    let conn = skeleton();
    entity(&conn, "CALLER", "function", "caller", "src/caller.ts", 1);
    unresolved(&conn, "U", "console.log", "src/caller.ts", "value");
    edge(&conn, "CALLER", Relation::Calls, "U", "src/caller.ts", 5);
    rebuild_assertion_state(&conn).unwrap();

    let report = ask(&conn, "U");
    assert_eq!(names(&report), vec!["caller"]);
    assert!(report.results[0].is_unresolved);
    assert_eq!(report.results[0].status.as_deref(), Some("UNRESOLVED"));
}
