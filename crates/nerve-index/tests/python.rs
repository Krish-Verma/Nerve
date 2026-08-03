//! Slice 9a: Python structure, measured against `fixtures/py-basic/expected.json`.
//!
//! The ground truth was written **before** the resolver, so what follows is a specification the
//! implementation had to satisfy rather than a description of what it happened to produce. Every
//! set comparison here is an equality, not a containment: an edge the corpus produces and the
//! ground truth does not name is a failure in the same way a missing one is.
//!
//! The nine acceptance criteria in `docs/plans/slice-09-python.md` map onto the tests below.
//! Criterion 6 (incremental equivalence) lives in `incremental.rs`, extending the harness that
//! already exists rather than starting a second one, and criterion 7 (no repository code is
//! executed) lives in `nerve-cli/tests/no_subprocess.rs`, next to the T1 loop it belongs to.

mod common;

use std::collections::{BTreeMap, BTreeSet};

use common::{indexed_named_fixture, named_fixture_copy, open_db, TEST_PROJECT_ID};

const FIXTURE: &str = "py-basic";

fn expected() -> serde_json::Value {
    let path = common::named_fixture_root(FIXTURE).join("expected.json");
    let text = std::fs::read_to_string(&path)
        .unwrap_or_else(|err| panic!("{} is unreadable: {err}", path.display()));
    serde_json::from_str(&text)
        .unwrap_or_else(|err| panic!("{} is not valid JSON: {err}", path.display()))
}

fn array<'a>(value: &'a serde_json::Value, key: &str) -> &'a Vec<serde_json::Value> {
    value[key]
        .as_array()
        .unwrap_or_else(|| panic!("expected.json has no `{key}` array"))
}

fn text(value: &serde_json::Value, key: &str) -> String {
    value[key]
        .as_str()
        .unwrap_or_else(|| panic!("expected.json entry {value} has no string `{key}`"))
        .to_string()
}

/// `<rel_path>#<scope_path>.<name>` for every symbol, and `<rel_path>` for every module.
fn selectors(conn: &nerve_store::Connection) -> BTreeMap<String, String> {
    let mut candidates: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    let mut stmt = conn
        .prepare(
            "SELECT e.entity_id, e.kind, e.name, e.scope_path, o.file_path
               FROM entity e
               JOIN occurrence o ON o.entity_id = e.entity_id
              ORDER BY e.entity_id, o.file_path",
        )
        .unwrap();
    let rows = stmt
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, String>(4)?,
            ))
        })
        .unwrap();
    for row in rows {
        let (entity_id, kind, name, scope_path, file_path) = row.unwrap();
        let selector = match kind.as_str() {
            "module" => file_path.clone(),
            "function" | "method" | "class" | "interface" => {
                if scope_path.is_empty() {
                    format!("{file_path}#{name}")
                } else {
                    format!("{file_path}#{scope_path}.{name}")
                }
            }
            _ => continue,
        };
        candidates.entry(selector).or_default().insert(entity_id);
    }
    let mut resolved = BTreeMap::new();
    for (selector, ids) in candidates {
        assert_eq!(ids.len(), 1, "selector {selector:?} is ambiguous: {ids:?}");
        resolved.insert(selector, ids.into_iter().next().unwrap());
    }
    resolved
}

/// Every `py-structural` symbol as `<selector>|<kind>`, excluding the malformed file.
fn python_symbols(conn: &nerve_store::Connection, skip_path: &str) -> BTreeSet<String> {
    let mut stmt = conn
        .prepare(
            "SELECT DISTINCT o.file_path, e.kind, e.scope_path, e.name
               FROM entity e
               JOIN occurrence o ON o.entity_id = e.entity_id
              WHERE e.language = 'python' AND e.kind IN ('function', 'method', 'class')",
        )
        .unwrap();
    stmt.query_map([], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, String>(2)?,
            row.get::<_, String>(3)?,
        ))
    })
    .unwrap()
    .map(|row| row.unwrap())
    .filter(|(file_path, _, _, _)| file_path != skip_path)
    .map(|(file_path, kind, scope_path, name)| {
        if scope_path.is_empty() {
            format!("{file_path}#{name}|{kind}")
        } else {
            format!("{file_path}#{scope_path}.{name}|{kind}")
        }
    })
    .collect()
}

/// `IMPORTS` edges whose target is a `Module`, as `<importer> -> <target path>`.
fn resolved_imports(conn: &nerve_store::Connection) -> BTreeSet<String> {
    let mut stmt = conn
        .prepare(
            "SELECT s.scope_path, t.scope_path, o.evidence_source_type
               FROM assertion a
               JOIN entity s ON s.entity_id = a.source_entity_id
               JOIN entity t ON t.entity_id = a.target_entity_id
               JOIN observation o ON o.assertion_id = a.assertion_id
              WHERE a.relation = 'IMPORTS' AND t.kind = 'module'
                AND o.extractor_id = 'py-structural'",
        )
        .unwrap();
    stmt.query_map([], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, String>(2)?,
        ))
    })
    .unwrap()
    .map(|row| {
        let (from, to, source_type) = row.unwrap();
        assert_eq!(
            source_type, "AST_RESOLVED",
            "{from} -> {to} survived resolution but is not labelled as having been resolved"
        );
        format!("{from} -> {to}")
    })
    .collect()
}

/// `IMPORTS` edges whose target is an `Unresolved`, as `<importer> -> <category>:<name>`.
fn unresolved_imports(conn: &nerve_store::Connection) -> BTreeMap<String, String> {
    let mut stmt = conn
        .prepare(
            "SELECT s.scope_path, t.name, t.meta, o.evidence_source_type
               FROM assertion a
               JOIN entity s ON s.entity_id = a.source_entity_id
               JOIN entity t ON t.entity_id = a.target_entity_id
               JOIN observation o ON o.assertion_id = a.assertion_id
              WHERE a.relation = 'IMPORTS' AND t.kind = 'unresolved'
                AND o.extractor_id = 'py-structural'",
        )
        .unwrap();
    stmt.query_map([], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, String>(2)?,
            row.get::<_, String>(3)?,
        ))
    })
    .unwrap()
    .map(|row| {
        let (from, name, meta, source_type) = row.unwrap();
        assert_eq!(
            source_type, "AST_DIRECT",
            "{from} -> {name} resolved nothing, so it must not claim a resolution step ran"
        );
        let meta: serde_json::Value = serde_json::from_str(&meta).unwrap();
        let category = meta["category"].as_str().unwrap().to_string();
        let reason = meta["reason"].as_str().unwrap().to_string();
        (format!("{from} -> {category}:{name}"), reason)
    })
    .collect()
}

// ---- criterion 1: entities and spans -------------------------------------------------------

/// **Criterion 1.** Every `.py` file is a `Module`, and every function, class and method is an
/// entity with a span that points at the declaration.
#[test]
fn python_modules_functions_classes_and_methods_are_indexed_with_correct_spans() {
    let ((_dir, root), _outcome) = indexed_named_fixture(FIXTURE);
    let conn = open_db(&root);
    let expected = expected();

    let modules: BTreeMap<String, bool> = conn
        .prepare(
            "SELECT e.scope_path, e.meta FROM entity e
              WHERE e.kind = 'module' AND e.language = 'python'",
        )
        .unwrap()
        .query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })
        .unwrap()
        .map(|row| {
            let (path, meta) = row.unwrap();
            let meta: serde_json::Value = serde_json::from_str(&meta).unwrap();
            (path, meta["package"].as_bool().unwrap())
        })
        .collect();
    let expected_modules: BTreeMap<String, bool> = array(&expected, "modules")
        .iter()
        .map(|entry| (text(entry, "path"), entry["package"].as_bool().unwrap()))
        .collect();
    assert_eq!(
        modules, expected_modules,
        "the Module set, or a package flag, differs from the ground truth"
    );

    let actual = python_symbols(&conn, "broken.py");
    let expected_symbols: BTreeSet<String> = array(&expected, "symbols")
        .iter()
        .map(|entry| format!("{}|{}", text(entry, "selector"), text(entry, "kind")))
        .collect();
    assert_eq!(
        actual,
        expected_symbols,
        "declared symbols differ from the ground truth\nmissing: {:?}\nunexpected: {:?}",
        expected_symbols.difference(&actual).collect::<Vec<_>>(),
        actual.difference(&expected_symbols).collect::<Vec<_>>()
    );

    // Spans point at the declaration, decorators included. `tune` is decorated, so its span must
    // start at the `@` rather than at the `def`.
    let names = selectors(&conn);
    let span = |selector: &str| -> (i64, i64) {
        conn.query_row(
            "SELECT start_line, end_line FROM occurrence WHERE entity_id = ?1",
            [names.get(selector).unwrap_or_else(|| panic!("{selector}"))],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap()
    };
    let source = std::fs::read_to_string(root.join("pkg/core.py")).unwrap();
    let line = |number: i64| {
        source
            .lines()
            .nth((number - 1) as usize)
            .unwrap()
            .to_string()
    };
    let (tune_start, tune_end) = span("pkg/core.py#tune");
    assert!(
        line(tune_start).starts_with("@functools.lru_cache"),
        "a decorated function's span must start at the decorator, got {:?}",
        line(tune_start)
    );
    assert!(tune_end > tune_start);
    let (inner_start, _) = span("pkg/core.py#tune.inner");
    assert!(line(inner_start).trim_start().starts_with("def inner"));
    assert!(
        inner_start > tune_start && inner_start < tune_end,
        "a nested function must lie inside its parent"
    );
    let (init_start, _) = span("pkg/core.py#Engine.__init__");
    assert!(line(init_start).trim_start().starts_with("def __init__"));
}

/// Decorators are structural metadata on the decorated symbol. They are never a call edge, and
/// `@app.route`-style framework meaning is Slice 10's, not this slice's.
#[test]
fn decorators_are_metadata_and_never_an_edge() {
    let ((_dir, root), _outcome) = indexed_named_fixture(FIXTURE);
    let conn = open_db(&root);
    let names = selectors(&conn);
    let expected = expected();

    for entry in array(&expected, "symbols") {
        let Some(decorators) = entry.get("decorators") else {
            continue;
        };
        let selector = text(entry, "selector");
        let meta: String = conn
            .query_row(
                "SELECT meta FROM entity WHERE entity_id = ?1",
                [names.get(&selector).unwrap()],
                |row| row.get(0),
            )
            .unwrap();
        let meta: serde_json::Value = serde_json::from_str(&meta).unwrap();
        assert_eq!(&meta["decorators"], decorators, "decorators of {selector}");
    }

    let async_start: String = conn
        .query_row(
            "SELECT meta FROM entity WHERE entity_id = ?1",
            [names.get("pkg/core.py#Engine.start").unwrap()],
            |row| row.get(0),
        )
        .unwrap();
    let async_start: serde_json::Value = serde_json::from_str(&async_start).unwrap();
    assert_eq!(async_start["async"], true, "`async def` is a syntax fact");

    // No relation outside the four this slice declares. In particular no CALLS from a decorator
    // and no EXTENDS from `class Turbo(Engine)`.
    let relations: Vec<String> = conn
        .prepare(
            "SELECT DISTINCT a.relation FROM assertion a
               JOIN observation o ON o.assertion_id = a.assertion_id
              WHERE o.extractor_id = 'py-structural' ORDER BY a.relation",
        )
        .unwrap()
        .query_map([], |row| row.get(0))
        .unwrap()
        .map(|row| row.unwrap())
        .collect();
    assert_eq!(
        relations,
        vec!["DEFINES", "EXPORTS", "IMPORTS"],
        "py-structural emitted a relation outside what Slice 9a declares"
    );
}

// ---- criterion 2: resolution ---------------------------------------------------------------

/// **Criterion 2.** Relative and in-repo absolute imports resolve, to exactly the expected set.
#[test]
fn relative_and_absolute_in_repo_imports_resolve_to_the_expected_edge_set() {
    let ((_dir, root), _outcome) = indexed_named_fixture(FIXTURE);
    let conn = open_db(&root);
    let expected = expected();

    let actual = resolved_imports(&conn);
    let want: BTreeSet<String> = array(&expected, "imports")
        .iter()
        .map(|entry| format!("{} -> {}", text(entry, "from"), text(entry, "to")))
        .collect();
    // The forbidden list is checked **first**, and deliberately. The equality below already
    // catches every one of these — a forbidden edge is by construction an unexpected one — but it
    // reports a set difference, while this reports the named wrong answer and the reason it is
    // wrong. Running the weaker check first is what makes it ever fire at all; behind the
    // equality it would be unreachable, which is how a test comes to be trusted without having
    // been exercised.
    for entry in array(&expected, "forbidden") {
        let edge = format!("{} -> {}", text(entry, "from"), text(entry, "to"));
        assert!(
            !actual.contains(&edge),
            "forbidden edge present: {edge} — {}",
            text(entry, "why")
        );
    }
    assert_eq!(
        actual,
        want,
        "resolved IMPORTS edges differ from the ground truth\nmissing: {:?}\nunexpected: {:?}",
        want.difference(&actual).collect::<Vec<_>>(),
        actual.difference(&want).collect::<Vec<_>>()
    );

    // `__all__` is the only statement of a public surface Python has, so it is the only source of
    // an EXPORTS edge — and only for names the module itself declares.
    let names = selectors(&conn);
    let exports: BTreeSet<String> = conn
        .prepare(
            "SELECT s.scope_path, a.target_entity_id FROM assertion a
               JOIN entity s ON s.entity_id = a.source_entity_id
               JOIN observation o ON o.assertion_id = a.assertion_id
              WHERE a.relation = 'EXPORTS' AND o.extractor_id = 'py-structural'",
        )
        .unwrap()
        .query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })
        .unwrap()
        .map(|row| {
            let (from, target) = row.unwrap();
            format!("{from} -> {target}")
        })
        .collect();
    let want_exports: BTreeSet<String> = array(&expected, "exports")
        .iter()
        .map(|entry| {
            let selector = text(entry, "to");
            format!(
                "{} -> {}",
                text(entry, "from"),
                names.get(&selector).unwrap_or_else(|| panic!("{selector}"))
            )
        })
        .collect();
    assert_eq!(
        exports, want_exports,
        "an __all__ entry naming an imported binding, or naming nothing at all, must not become \
         an EXPORTS edge"
    );
}

// ---- criterion 3: unsupported forms are values ----------------------------------------------

/// **Criterion 3.** Every unsupported form is an `Unresolved` entity carrying a reason.
///
/// Asserted as an equality over the whole set, so that neither a missing refusal nor an invented
/// one can pass. The reason is checked by keyword rather than by exact prose: the wording is
/// documentation, the distinction it draws is the contract.
#[test]
fn every_unsupported_form_is_an_unresolved_entity_with_a_reason() {
    let ((_dir, root), _outcome) = indexed_named_fixture(FIXTURE);
    let conn = open_db(&root);
    let expected = expected();

    let actual = unresolved_imports(&conn);
    let want: BTreeMap<String, String> = array(&expected, "unresolved")
        .iter()
        .map(|entry| {
            (
                format!(
                    "{} -> {}:{}",
                    text(entry, "from"),
                    text(entry, "category"),
                    text(entry, "name")
                ),
                text(entry, "reason"),
            )
        })
        .collect();

    let actual_keys: BTreeSet<&String> = actual.keys().collect();
    let want_keys: BTreeSet<&String> = want.keys().collect();
    assert_eq!(
        actual_keys,
        want_keys,
        "unresolved edges differ from the ground truth\nmissing: {:?}\nunexpected: {:?}",
        want_keys.difference(&actual_keys).collect::<Vec<_>>(),
        actual_keys.difference(&want_keys).collect::<Vec<_>>()
    );

    // Each ground-truth `reason` is the distinguishing phrase the recorded reason must contain.
    // "namespace package" and "names no indexed module" are different findings with different
    // fixes, and a refusal that collapsed them would be less useful than no refusal at all.
    for (key, keyword) in &want {
        let recorded = &actual[key];
        assert!(
            recorded.contains(keyword),
            "{key} recorded {recorded:?}, which does not state {keyword:?}"
        );
    }

    // The two categories are load-bearing, not decorative: `pkg.util` the module specifier and
    // `wildcard:pkg.util` the unknowable binding set are different things about one statement.
    assert!(actual
        .keys()
        .any(|key| key.contains("value:wildcard:pkg.util")));
    assert!(actual.keys().any(|key| key.contains("module:pkg.util")));
}

// ---- criterion 4: the 5d-i invariant, restated ----------------------------------------------

/// **Criterion 4.** A Python-only repository produces **zero** `ts-js-*` observations.
///
/// This is Slice 5d-i's invariant said again for a new language. Directory containment was once
/// stamped `ts-js-structural` in a tree holding no TypeScript; an observation names what produced
/// it, and nothing about a `.py` file was produced by a TypeScript extractor.
#[test]
fn a_python_only_repository_produces_no_ts_js_observations() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("repo");
    std::fs::create_dir_all(root.join("pkg")).unwrap();
    std::fs::write(root.join("pkg/__init__.py"), "\"\"\"pkg.\"\"\"\n").unwrap();
    std::fs::write(
        root.join("pkg/core.py"),
        "from .util import scale\n\n\ndef tune(v):\n    return scale(v)\n",
    )
    .unwrap();
    std::fs::write(
        root.join("pkg/util.py"),
        "def scale(v):\n    return v * 2\n",
    )
    .unwrap();
    nerve_index::init_with_project_id(&root, Some(TEST_PROJECT_ID)).unwrap();
    let outcome = nerve_index::index_repository(&root).unwrap();
    assert_eq!(outcome.files_processed, 3);

    let conn = open_db(&root);
    let by_extractor: Vec<(String, i64)> = conn
        .prepare("SELECT extractor_id, count(*) FROM observation GROUP BY 1 ORDER BY 1")
        .unwrap()
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
        .unwrap()
        .map(|row| row.unwrap())
        .collect();
    let ts_js: Vec<&(String, i64)> = by_extractor
        .iter()
        .filter(|(id, _)| id.starts_with("ts-js-"))
        .collect();
    assert!(
        ts_js.is_empty(),
        "a repository with no TypeScript in it produced {ts_js:?}; observations by extractor were \
         {by_extractor:?}"
    );
    assert!(
        by_extractor
            .iter()
            .any(|(id, count)| id == "py-structural" && *count > 0),
        "py-structural wrote nothing, so the assertion above would pass vacuously: {by_extractor:?}"
    );

    // The run rows say the same thing from the other side: the TypeScript extractors ran, looked
    // at nothing, and said so. A row that appeared only when a `.ts` file existed would make its
    // absence ambiguous.
    let report = nerve_store::status(&conn).unwrap();
    for run in &report.runs {
        if run.extractor_id.starts_with("ts-js-") {
            assert_eq!(
                run.files_processed, 0,
                "{} claims to have read {} file(s) in a repository with no TypeScript",
                run.extractor_id, run.files_processed
            );
        }
    }
    let python_run = report
        .runs
        .iter()
        .find(|run| run.extractor_id == "py-structural")
        .expect("py-structural must have a run row");
    assert_eq!(python_run.files_processed, 3);
    assert_eq!(python_run.extractor_version, "1.0.0");
}

/// The converse, over the TypeScript corpus: `py-structural` runs, reads nothing, and writes
/// nothing. Without this the criterion above could be satisfied by an extractor that fires on
/// everything.
#[test]
fn a_typescript_only_repository_produces_no_python_observations() {
    let ((_dir, root), _outcome) = indexed_named_fixture("ts-basic");
    let conn = open_db(&root);
    let python: i64 = conn
        .query_row(
            "SELECT count(*) FROM observation WHERE extractor_id = 'py-structural'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(python, 0);
    let report = nerve_store::status(&conn).unwrap();
    let python_run = report
        .runs
        .iter()
        .find(|run| run.extractor_id == "py-structural")
        .expect("the run row is recorded even when there is no Python");
    assert_eq!(python_run.files_processed, 0);
}

// ---- criterion 5: malformed input ------------------------------------------------------------

/// **Criterion 5.** A syntax error degrades to a partial parse, does not abort the index, and is
/// counted in parse health.
#[test]
fn a_malformed_file_degrades_to_a_partial_parse_and_is_counted() {
    let ((_dir, root), outcome) = indexed_named_fixture(FIXTURE);
    let expected = expected();
    let partial = &expected["partial_parse"];
    let path = text(partial, "path");

    assert_eq!(
        outcome.files_with_syntax_errors, 1,
        "exactly one fixture file is malformed"
    );
    assert_eq!(
        outcome.status,
        nerve_index::RunStatus::Complete,
        "a partial parse is not a failed read; the run still completes"
    );
    assert_eq!(outcome.files_failed, 0);

    let repo_id = nerve_core::ids::repository_id(TEST_PROJECT_ID);
    let conn = open_db(&root);
    let reported = nerve_index::partial_parses(&conn, &repo_id).unwrap();
    let paths: Vec<&str> = reported.iter().map(|p| p.rel_path.as_str()).collect();
    assert_eq!(paths, vec![path.as_str()], "parse health names the file");
    assert_eq!(reported[0].language, "python");

    // What could be read survives. A file that produced nothing would be indistinguishable from
    // one that was never indexed.
    let names = selectors(&conn);
    for selector in partial["must_contain"].as_array().unwrap() {
        let selector = selector.as_str().unwrap();
        assert!(
            names.contains_key(selector),
            "{selector} did not survive the partial parse; the file yielded {:?}",
            names
                .keys()
                .filter(|key| key.starts_with(&path))
                .collect::<Vec<_>>()
        );
    }
}

// ---- criterion 8: determinism ----------------------------------------------------------------

/// **Criterion 8.** Two indexes of the same tree produce byte-identical graphs.
#[test]
fn two_indexes_of_the_same_python_tree_are_byte_identical() {
    let dump = |root: &std::path::Path| -> String {
        nerve_store::canonical_dump(&open_db(root))
            .unwrap()
            .to_canonical_json()
            .unwrap()
    };

    let (_first_dir, first) = named_fixture_copy(FIXTURE);
    nerve_index::init_with_project_id(&first, Some(TEST_PROJECT_ID)).unwrap();
    nerve_index::index_repository(&first).unwrap();

    let (_second_dir, second) = named_fixture_copy(FIXTURE);
    nerve_index::init_with_project_id(&second, Some(TEST_PROJECT_ID)).unwrap();
    nerve_index::index_repository(&second).unwrap();

    assert_eq!(
        dump(&first),
        dump(&second),
        "two from-scratch indexes of one tree disagree"
    );

    // And re-indexing in place changes nothing either.
    let before = dump(&first);
    nerve_index::index_repository(&first).unwrap();
    assert_eq!(
        before,
        dump(&first),
        "a second run over the same tree moved"
    );
}

// ---- the evidence labels ----------------------------------------------------------------------

/// ADR-0003: an edge produced by resolution says `AST_RESOLVED`; one the tree literally states
/// says `AST_DIRECT`. Checked over the whole Python batch, not per emission site.
#[test]
fn python_evidence_labels_distinguish_read_from_resolved() {
    let ((_dir, root), _outcome) = indexed_named_fixture(FIXTURE);
    let conn = open_db(&root);

    let undeclared: i64 = conn
        .query_row(
            "SELECT count(*) FROM observation
              WHERE extractor_id = 'py-structural'
                AND evidence_source_type NOT IN ('AST_DIRECT', 'AST_RESOLVED')",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(undeclared, 0);

    let mismatched: i64 = conn
        .query_row(
            "SELECT count(*) FROM observation
              WHERE extractor_id = 'py-structural'
                AND (evidence_source_type = 'AST_RESOLVED') != (directness = 'RESOLVED')",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(mismatched, 0);

    // A resolved import must not still be labelled AST_DIRECT, and an unresolved one must not
    // claim a resolution step ran.
    let mislabelled: i64 = conn
        .query_row(
            "SELECT count(*) FROM observation o
               JOIN assertion a ON a.assertion_id = o.assertion_id
               JOIN entity t ON t.entity_id = a.target_entity_id
              WHERE o.extractor_id = 'py-structural' AND a.relation = 'IMPORTS'
                AND ((t.kind = 'module') != (o.evidence_source_type = 'AST_RESOLVED'))",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(mislabelled, 0);

    // Every observation carries the extractor that produced it and its version — the field 5d-i
    // was a corrective slice for.
    let versions: Vec<String> = conn
        .prepare(
            "SELECT DISTINCT extractor_version FROM observation WHERE extractor_id = 'py-structural'",
        )
        .unwrap()
        .query_map([], |row| row.get(0))
        .unwrap()
        .map(|row| row.unwrap())
        .collect();
    assert_eq!(versions, vec![nerve_index::PYTHON_EXTRACTOR_VERSION]);
}

/// A `sys.path` rewrite is recorded on the module and refuses only the absolute specifiers.
///
/// Both halves matter. Refusing everything would throw away relative imports that `sys.path`
/// cannot affect; refusing nothing would let a module that has moved its own import roots claim
/// its absolute specifiers name repository files.
#[test]
fn a_sys_path_rewrite_is_recorded_and_refuses_only_absolute_specifiers() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("repo");
    std::fs::create_dir_all(root.join("pkg")).unwrap();
    std::fs::write(root.join("pkg/__init__.py"), "\"\"\"pkg.\"\"\"\n").unwrap();
    std::fs::write(root.join("pkg/util.py"), "def scale(v):\n    return v\n").unwrap();
    std::fs::write(
        root.join("pkg/widened.py"),
        "import sys\n\nsys.path.insert(0, 'vendor')\n\nfrom pkg.util import scale\nfrom .util import scale as also\n",
    )
    .unwrap();
    nerve_index::init_with_project_id(&root, Some(TEST_PROJECT_ID)).unwrap();
    nerve_index::index_repository(&root).unwrap();

    let conn = open_db(&root);
    let meta: String = conn
        .query_row(
            "SELECT meta FROM entity WHERE kind = 'module' AND scope_path = 'pkg/widened.py'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    let meta: serde_json::Value = serde_json::from_str(&meta).unwrap();
    assert_eq!(meta["sys_path_mutated"], true);

    let edges: BTreeSet<String> = conn
        .prepare(
            "SELECT t.kind || ':' || t.name FROM assertion a
               JOIN entity s ON s.entity_id = a.source_entity_id
               JOIN entity t ON t.entity_id = a.target_entity_id
              WHERE a.relation = 'IMPORTS' AND s.scope_path = 'pkg/widened.py'",
        )
        .unwrap()
        .query_map([], |row| row.get::<_, String>(0))
        .unwrap()
        .map(|row| row.unwrap())
        .collect();
    assert!(
        edges.contains("module:util"),
        "a relative import is resolved from __package__, not from sys.path: {edges:?}"
    );
    assert!(
        edges.contains("unresolved:pkg.util"),
        "the absolute specifier must be refused: {edges:?}"
    );
    assert!(
        edges.contains("unresolved:sys"),
        "`import sys` is absolute too: {edges:?}"
    );
}
