//! Traversal and evidence assembly, over a graph built directly in SQL.
//!
//! These tests deliberately do not go through the indexer. The point is to pin the semantics
//! of `find_paths` and `explain` — ordering, bounds, filters, direction, freshness — against a
//! graph small enough to reason about completely.

use std::collections::BTreeMap;

use nerve_core::vocab::Relation;
use nerve_store::{
    explain, find_paths, migrate, open_in_memory, rebuild_assertion_state, resolve_selector,
    Connection, Direction, EdgeDirection, FileProbe, FileProber, Freshness, PathQuery, Selection,
    WhyDirection, WhyQuery,
};

const STATE: &str = "state-1";

/// A prober that answers from a table, so freshness can be tested without a filesystem.
struct StubProber {
    answers: BTreeMap<String, FileProbe>,
}

impl FileProber for StubProber {
    fn probe(&self, rel_path: &str) -> FileProbe {
        self.answers
            .get(rel_path)
            .cloned()
            .unwrap_or(FileProbe::Missing)
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
         VALUES (?1, ?2, ?3, 0, 1, ?4, 0, ?4, 1, 'hash-of-file')",
        rusqlite::params![format!("occ-{id}"), id, file, line],
    )
    .unwrap();
}

fn edge(conn: &Connection, source: &str, relation: Relation, target: &str, file: &str, line: i64) {
    let assertion_id = format!("a-{source}-{}-{target}", relation.as_str());
    conn.execute(
        "INSERT INTO assertion (assertion_id, repo_id, source_entity_id, relation,
                                target_entity_id)
         VALUES (?1, 'repo', ?2, ?3, ?4)",
        rusqlite::params![assertion_id, source, relation.as_str(), target],
    )
    .unwrap();
    // The observation names no state: it names the run, and the run names the state (ADR-0006).
    conn.execute(
        "INSERT INTO observation (assertion_id, extractor_run_id, evidence_source_type,
                                  directness, extractor_id, extractor_version,
                                  file_path, start_line, end_line, content_hash, details,
                                  created_at)
         VALUES (?1, 1, 'AST_RESOLVED', 'RESOLVED', 'test-extractor', '9.9.9', ?2, ?3, ?3,
                 'hash-of-file', '{\"why\":\"because\"}', '2026-07-31T00:00:00Z')",
        rusqlite::params![assertion_id, file, line],
    )
    .unwrap();
}

/// ```text
///  A --CALLS--> B --CALLS--> C
///  A --REFERENCES----------> C
///               B --IMPORTS--> U   (U is an Unresolved entity)
///  D is isolated
/// ```
fn fixture() -> Connection {
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

    entity(&conn, "A", "function", "alpha", "src/a.ts", 1);
    entity(&conn, "B", "function", "bravo", "src/b.ts", 2);
    entity(&conn, "C", "function", "charlie", "src/c.ts", 3);
    entity(&conn, "D", "function", "delta", "src/d.ts", 4);
    entity(&conn, "U", "unresolved", "unknown", "src/b.ts", 5);

    edge(&conn, "A", Relation::Calls, "B", "src/a.ts", 10);
    edge(&conn, "B", Relation::Calls, "C", "src/b.ts", 20);
    edge(&conn, "A", Relation::References, "C", "src/a.ts", 11);
    edge(&conn, "B", Relation::Imports, "U", "src/b.ts", 21);

    rebuild_assertion_state(&conn).unwrap();
    conn
}

fn hop_counts(paths: &[nerve_store::GraphPath]) -> Vec<usize> {
    paths.iter().map(|path| path.length()).collect()
}

fn relations(path: &nerve_store::GraphPath) -> Vec<String> {
    path.hops.iter().map(|hop| hop.relation.clone()).collect()
}

#[test]
fn paths_come_back_shortest_first() {
    let conn = fixture();
    let report = find_paths(&conn, "A", "C", &PathQuery::default()).unwrap();
    assert_eq!(hop_counts(&report.paths), vec![1, 2]);
    assert_eq!(relations(&report.paths[0]), vec!["REFERENCES"]);
    assert_eq!(relations(&report.paths[1]), vec!["CALLS", "CALLS"]);
    assert!(!report.truncated);
}

#[test]
fn the_same_question_gives_byte_identical_answers() {
    let conn = fixture();
    let first = find_paths(&conn, "A", "C", &PathQuery::default()).unwrap();
    let second = find_paths(&conn, "A", "C", &PathQuery::default()).unwrap();
    assert_eq!(first, second);
}

#[test]
fn hops_carry_the_derived_state_and_a_representative_observation() {
    let conn = fixture();
    let report = find_paths(&conn, "A", "B", &PathQuery::default()).unwrap();
    let hop = &report.paths[0].hops[0];
    assert_eq!(hop.relation, "CALLS");
    assert_eq!(hop.from.name, "alpha");
    assert_eq!(hop.to.name, "bravo");
    assert_eq!(hop.status.as_deref(), Some("SUPPORTED"));
    assert_eq!(hop.strongest_source_type.as_deref(), Some("AST_RESOLVED"));
    assert_eq!(hop.observation_count, 1);
    assert_eq!(hop.location(), "src/a.ts:10");
    assert!(!hop.traversed_backwards);
}

#[test]
fn the_relation_filter_removes_edges_from_the_walk() {
    let conn = fixture();
    let query = PathQuery {
        relations: vec![Relation::Calls],
        ..PathQuery::default()
    };
    let report = find_paths(&conn, "A", "C", &query).unwrap();
    assert_eq!(hop_counts(&report.paths), vec![2]);
}

#[test]
fn max_depth_bounds_the_walk() {
    let conn = fixture();
    let query = PathQuery {
        max_depth: 1,
        relations: vec![Relation::Calls],
        ..PathQuery::default()
    };
    let report = find_paths(&conn, "A", "C", &query).unwrap();
    assert!(report.paths.is_empty(), "{:?}", report.paths);
    assert!(
        !report.truncated,
        "an exhausted search is not a truncated one"
    );
}

#[test]
fn limit_caps_the_number_of_paths() {
    let conn = fixture();
    let query = PathQuery {
        limit: 1,
        ..PathQuery::default()
    };
    let report = find_paths(&conn, "A", "C", &query).unwrap();
    assert_eq!(hop_counts(&report.paths), vec![1]);
}

#[test]
fn forward_is_directed_and_any_is_not() {
    let conn = fixture();
    let forward = find_paths(&conn, "C", "A", &PathQuery::default()).unwrap();
    assert!(forward.paths.is_empty());

    let query = PathQuery {
        direction: Direction::Any,
        ..PathQuery::default()
    };
    let any = find_paths(&conn, "C", "A", &query).unwrap();
    assert_eq!(hop_counts(&any.paths), vec![1, 2]);
    assert!(
        any.paths[0].hops[0].traversed_backwards,
        "an edge walked against its recorded direction must say so"
    );
}

#[test]
fn unresolved_edges_are_included_by_default_and_marked() {
    let conn = fixture();
    let report = find_paths(&conn, "A", "U", &PathQuery::default()).unwrap();
    assert_eq!(hop_counts(&report.paths), vec![2]);
    assert!(report.paths[0].traverses_unresolved());
    assert!(report.paths[0].hops[1].is_unresolved);
    assert_eq!(
        report.paths[0].hops[1].status.as_deref(),
        Some("UNRESOLVED")
    );
}

#[test]
fn resolved_only_excludes_unresolved_edges() {
    let conn = fixture();
    let query = PathQuery {
        resolved_only: true,
        ..PathQuery::default()
    };
    let report = find_paths(&conn, "A", "U", &query).unwrap();
    assert!(report.paths.is_empty());
}

#[test]
fn an_unreachable_target_is_an_empty_answer_not_an_error() {
    let conn = fixture();
    let report = find_paths(&conn, "A", "D", &PathQuery::default()).unwrap();
    assert!(report.paths.is_empty());
    assert!(!report.truncated);
    assert_eq!(report.to.name, "delta");
}

#[test]
fn an_entity_reaches_itself_in_zero_hops() {
    let conn = fixture();
    let report = find_paths(&conn, "A", "A", &PathQuery::default()).unwrap();
    assert_eq!(hop_counts(&report.paths), vec![0]);
}

// ---- why -----------------------------------------------------------------------------------

#[test]
fn explain_reports_both_sides_of_an_entity_in_a_stable_order() {
    let conn = fixture();
    let files = prober(&[
        ("src/a.ts", FileProbe::Hash("hash-of-file".into())),
        ("src/b.ts", FileProbe::Hash("hash-of-file".into())),
    ]);
    let report = explain(&conn, "B", None, &WhyQuery::default(), &files).unwrap();

    let summary: Vec<(String, &str)> = report
        .assertions
        .iter()
        .map(|assertion| (assertion.relation.clone(), assertion.direction.as_str()))
        .collect();
    assert_eq!(
        summary,
        vec![
            ("CALLS".to_string(), "outgoing"),
            ("CALLS".to_string(), "incoming"),
            ("IMPORTS".to_string(), "outgoing"),
        ]
    );
    assert_eq!(report.files_probed, 2);
}

#[test]
fn explain_between_two_entities_reports_only_that_pair() {
    let conn = fixture();
    let files = prober(&[("src/a.ts", FileProbe::Hash("hash-of-file".into()))]);
    let report = explain(&conn, "A", Some("B"), &WhyQuery::default(), &files).unwrap();
    assert_eq!(report.assertions.len(), 1);
    assert_eq!(report.assertions[0].relation, "CALLS");
    assert_eq!(report.assertions[0].direction, EdgeDirection::Outgoing);
    assert_eq!(report.object.as_ref().unwrap().name, "bravo");
}

#[test]
fn explain_carries_the_whole_evidence_profile() {
    let conn = fixture();
    let files = prober(&[("src/a.ts", FileProbe::Hash("hash-of-file".into()))]);
    let report = explain(&conn, "A", Some("B"), &WhyQuery::default(), &files).unwrap();
    let observation = &report.assertions[0].observations[0];
    assert_eq!(observation.evidence_source_type, "AST_RESOLVED");
    assert_eq!(observation.directness, "RESOLVED");
    assert_eq!(observation.extractor_id, "test-extractor");
    assert_eq!(observation.extractor_version, "9.9.9");
    assert_eq!(observation.location(), "src/a.ts:10");
    assert_eq!(observation.state_id, STATE);
    assert_eq!(observation.content_hash, "hash-of-file");
    assert_eq!(observation.match_quality, None);
    assert_eq!(observation.environment, None);
    assert_eq!(observation.details.as_deref(), Some(r#"{"why":"because"}"#));
    assert_eq!(observation.freshness, Freshness::Fresh);
}

#[test]
fn freshness_is_computed_from_the_file_not_read_from_the_row() {
    let conn = fixture();
    let files = prober(&[
        ("src/a.ts", FileProbe::Hash("something-else".into())),
        ("src/b.ts", FileProbe::Hash("hash-of-file".into())),
    ]);
    let report = explain(&conn, "B", None, &WhyQuery::default(), &files).unwrap();
    for assertion in &report.assertions {
        for observation in &assertion.observations {
            let expected = if observation.file_path == "src/a.ts" {
                Freshness::Stale
            } else {
                Freshness::Fresh
            };
            assert_eq!(
                observation.freshness,
                expected,
                "{}",
                observation.location()
            );
        }
    }
}

#[test]
fn a_refused_or_missing_file_is_reported_as_such_not_as_stale() {
    let conn = fixture();
    let files = prober(&[
        ("src/a.ts", FileProbe::Refused),
        ("src/b.ts", FileProbe::Missing),
    ]);
    let report = explain(&conn, "B", None, &WhyQuery::default(), &files).unwrap();
    let observed: Vec<Freshness> = report
        .assertions
        .iter()
        .flat_map(|assertion| assertion.observations.iter())
        .map(|observation| observation.freshness)
        .collect();
    assert!(observed.contains(&Freshness::Refused));
    assert!(observed.contains(&Freshness::FileMissing));
    assert!(!observed.contains(&Freshness::Stale));
}

#[test]
fn the_direction_and_relation_filters_narrow_the_answer() {
    let conn = fixture();
    let files = prober(&[
        ("src/a.ts", FileProbe::Hash("hash-of-file".into())),
        ("src/b.ts", FileProbe::Hash("hash-of-file".into())),
    ]);

    let incoming = explain(
        &conn,
        "B",
        None,
        &WhyQuery {
            direction: WhyDirection::Incoming,
            relations: Vec::new(),
        },
        &files,
    )
    .unwrap();
    assert_eq!(incoming.assertions.len(), 1);
    assert_eq!(incoming.assertions[0].source.name, "alpha");

    let imports = explain(
        &conn,
        "B",
        None,
        &WhyQuery {
            direction: WhyDirection::Both,
            relations: vec![Relation::Imports],
        },
        &files,
    )
    .unwrap();
    assert_eq!(imports.assertions.len(), 1);
    assert!(imports.assertions[0].is_unresolved);
}

// ---- selectors -------------------------------------------------------------------------------

#[test]
fn a_unique_name_resolves_and_a_shared_one_does_not() {
    let conn = fixture();
    match resolve_selector(&conn, "alpha").unwrap() {
        Selection::Resolved { entity, .. } => assert_eq!(entity.entity_id, "A"),
        other => panic!("expected a single match, got {other:?}"),
    }

    // A second entity with the same name makes the selector ambiguous, never a coin toss.
    entity(&conn, "A2", "function", "alpha", "src/other.ts", 1);
    match resolve_selector(&conn, "alpha").unwrap() {
        Selection::Ambiguous { candidates, .. } => {
            let ids: Vec<String> = candidates.into_iter().map(|c| c.entity_id).collect();
            assert_eq!(ids, vec!["A".to_string(), "A2".to_string()]);
        }
        other => panic!("expected ambiguity, got {other:?}"),
    }
}

#[test]
fn an_exact_entity_id_wins_over_every_other_stage() {
    let conn = fixture();
    match resolve_selector(&conn, "A").unwrap() {
        Selection::Resolved { entity, matched_by } => {
            assert_eq!(entity.entity_id, "A");
            assert_eq!(matched_by.as_str(), "entity_id");
        }
        other => panic!("expected a single match, got {other:?}"),
    }
}

#[test]
fn a_selector_matching_nothing_carries_suggestions() {
    let conn = fixture();
    match resolve_selector(&conn, "alp").unwrap() {
        Selection::NotFound { suggestions } => {
            assert!(
                suggestions.iter().any(|hit| hit.name == "alpha"),
                "{suggestions:?}"
            );
        }
        other => panic!("expected no match, got {other:?}"),
    }

    // A typo in the last characters finds nothing by prefix, so the search steps back.
    match resolve_selector(&conn, "alphb").unwrap() {
        Selection::NotFound { suggestions } => {
            assert!(
                suggestions.iter().any(|hit| hit.name == "alpha"),
                "{suggestions:?}"
            );
        }
        other => panic!("expected no match, got {other:?}"),
    }

    // Nothing remotely similar yields no suggestions rather than an arbitrary list.
    match resolve_selector(&conn, "zzzzznope").unwrap() {
        Selection::NotFound { suggestions } => assert!(suggestions.is_empty(), "{suggestions:?}"),
        other => panic!("expected no match, got {other:?}"),
    }
}

// ---- neighbourhood (Slice 4a) ----------------------------------------------------------------

use nerve_store::{
    entity_relation_counts, neighbourhood, occurrences_of, path_is_indexed, unresolved_entities,
    NeighbourhoodQuery,
};

fn node_ids(report: &nerve_store::NeighbourhoodReport) -> Vec<String> {
    report
        .nodes
        .iter()
        .map(|node| node.entity.entity_id.clone())
        .collect()
}

#[test]
fn a_depth_one_neighbourhood_is_the_focus_and_its_immediate_edges() {
    let conn = fixture();
    let report = neighbourhood(&conn, "B", &NeighbourhoodQuery::default()).unwrap();

    assert_eq!(report.focus.entity_id, "B");
    assert_eq!(report.nodes[0].depth, 0);
    assert_eq!(report.nodes[0].entity.entity_id, "B");
    // A (calls B), C (called by B), U (imported by B).
    assert_eq!(node_ids(&report), vec!["B", "A", "C", "U"]);
    assert_eq!(report.edges.len(), 3);
    assert_eq!(report.omitted_nodes, 0);
    assert!(
        !report.truncated,
        "a complete answer is not a truncated one"
    );
    assert!(report.frontier_nodes > 0, "there is more to expand");
}

#[test]
fn direction_forward_follows_only_outgoing_edges() {
    let conn = fixture();
    let report = neighbourhood(
        &conn,
        "B",
        &NeighbourhoodQuery {
            direction: Direction::Forward,
            ..NeighbourhoodQuery::default()
        },
    )
    .unwrap();
    assert_eq!(node_ids(&report), vec!["B", "C", "U"]);
    for edge in &report.edges {
        assert_eq!(edge.source_entity_id, "B");
    }
}

#[test]
fn depth_two_reaches_further_and_the_edges_stay_closed() {
    let conn = fixture();
    let report = neighbourhood(
        &conn,
        "A",
        &NeighbourhoodQuery {
            max_depth: 2,
            ..NeighbourhoodQuery::default()
        },
    )
    .unwrap();
    let ids = node_ids(&report);
    assert!(ids.contains(&"U".to_string()), "{ids:?}");

    // Every reported edge names two reported nodes.
    for edge in &report.edges {
        assert!(ids.contains(&edge.source_entity_id), "{edge:?}");
        assert!(ids.contains(&edge.target_entity_id), "{edge:?}");
    }
}

#[test]
fn the_node_budget_bites_and_says_how_much_it_left_out() {
    let conn = fixture();
    let report = neighbourhood(
        &conn,
        "B",
        &NeighbourhoodQuery {
            max_nodes: 2,
            ..NeighbourhoodQuery::default()
        },
    )
    .unwrap();
    assert_eq!(report.nodes.len(), 2);
    assert!(report.truncated);
    assert_eq!(
        report.omitted_nodes, 2,
        "two neighbours were refused a slot: {report:?}"
    );
}

#[test]
fn a_relation_filter_and_resolved_only_apply_to_the_neighbourhood() {
    let conn = fixture();
    let imports = neighbourhood(
        &conn,
        "B",
        &NeighbourhoodQuery {
            relations: vec![Relation::Imports],
            ..NeighbourhoodQuery::default()
        },
    )
    .unwrap();
    assert_eq!(node_ids(&imports), vec!["B", "U"]);

    let resolved = neighbourhood(
        &conn,
        "B",
        &NeighbourhoodQuery {
            relations: vec![Relation::Imports],
            resolved_only: true,
            ..NeighbourhoodQuery::default()
        },
    )
    .unwrap();
    assert_eq!(
        node_ids(&resolved),
        vec!["B"],
        "the unresolved edge is gone"
    );
    assert!(resolved.edges.is_empty());
}

#[test]
fn an_isolated_entity_has_a_neighbourhood_of_exactly_itself() {
    let conn = fixture();
    let report = neighbourhood(&conn, "D", &NeighbourhoodQuery::default()).unwrap();
    assert_eq!(node_ids(&report), vec!["D"]);
    assert!(report.edges.is_empty());
    assert!(!report.truncated);
    assert_eq!(report.omitted_nodes, 0);
    assert_eq!(report.frontier_nodes, 0);
}

#[test]
fn the_same_neighbourhood_question_gives_a_byte_identical_answer() {
    let conn = fixture();
    let query = NeighbourhoodQuery {
        max_depth: 2,
        ..NeighbourhoodQuery::default()
    };
    let first = neighbourhood(&conn, "A", &query).unwrap();
    for _ in 0..8 {
        assert_eq!(neighbourhood(&conn, "A", &query).unwrap(), first);
    }
}

#[test]
fn occurrences_come_back_in_a_stable_order() {
    let conn = fixture();
    let occurrences = occurrences_of(&conn, "B").unwrap();
    assert_eq!(occurrences.len(), 1);
    assert_eq!(occurrences[0].file_path, "src/b.ts");
    assert_eq!(occurrences[0].start_line, 2);
    assert_eq!(occurrences[0].content_hash, "hash-of-file");
    assert!(occurrences_of(&conn, "nope").unwrap().is_empty());
}

#[test]
fn relation_counts_split_by_side() {
    let conn = fixture();
    let counts = entity_relation_counts(&conn, "B").unwrap();
    assert_eq!(counts.outgoing.get("CALLS"), Some(&1));
    assert_eq!(counts.outgoing.get("IMPORTS"), Some(&1));
    assert_eq!(counts.incoming.get("CALLS"), Some(&1));
    assert_eq!(counts.incoming.get("IMPORTS"), None);
}

#[test]
fn the_unresolved_list_reports_how_often_each_target_was_wanted() {
    let conn = fixture();
    let rows = unresolved_entities(&conn, 10, 0).unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].entity_id, "U");
    assert_eq!(rows[0].name, "unknown");
    assert_eq!(rows[0].referencing_assertions, 1);
    assert!(unresolved_entities(&conn, 10, 1).unwrap().is_empty());
}

#[test]
fn a_path_is_indexed_only_if_an_occurrence_names_it() {
    let conn = fixture();
    assert!(path_is_indexed(&conn, "src/b.ts").unwrap());
    assert!(!path_is_indexed(&conn, "src/never.ts").unwrap());
    assert!(!path_is_indexed(&conn, "../../etc/passwd").unwrap());
    assert!(!path_is_indexed(&conn, "").unwrap());
}
