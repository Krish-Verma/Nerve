//! The measured gate for `test-trace`, over `fixtures/trace-basic` and `fixtures/trace-hostile`.
//!
//! Written by the orchestrator after the implementation agent was terminated mid-slice by an
//! infrastructure error, at the line where it was about to write this file. Every assertion here is
//! read out of `fixtures/trace-basic/expected.json`, which was written **before** the ingestion
//! existed.
//!
//! The load-bearing test is [`a_nested_call_is_attributed_to_its_real_caller_and_never_to_the_test`].
//! `docs/plans/slice-11a-trace-ingestion.md` §2.1 exists because the Slice 11 plan made the test
//! function the source of every observed call, which for a stack `test_basic → parse → tokenize`
//! either asserts a call the test never made or throws away everything below depth 1.

mod common;

use std::collections::BTreeMap;
use std::path::Path;

use common::{named_fixture_copy, open_db, TEST_PROJECT_ID};

const FIXTURE: &str = "trace-basic";

fn expected() -> serde_json::Value {
    let path = common::named_fixture_root(FIXTURE).join("expected.json");
    let text = std::fs::read_to_string(&path).expect("expected.json must be readable");
    serde_json::from_str(&text).expect("expected.json must be valid JSON")
}

/// Index the fixture and substitute the real content merkle into each artifact.
///
/// The artifacts ship with `__CONTENT_MERKLE__` because the merkle is a property of the bytes the
/// index observed, and hard-coding one would make the fixture fail the moment a `.py` file changed
/// by a byte. Substituting it is what lets `bound` mean *bound*.
fn indexed(artifact: &str) -> (tempfile::TempDir, std::path::PathBuf, String) {
    let (dir, root) = named_fixture_copy(FIXTURE);
    nerve_index::init_with_project_id(&root, Some(TEST_PROJECT_ID)).unwrap();
    nerve_index::index_repository(&root).unwrap();

    let merkle: String = {
        let conn = open_db(&root);
        conn.query_row(
            "SELECT content_merkle FROM repository_state ORDER BY rowid DESC LIMIT 1",
            [],
            |row| row.get(0),
        )
        .unwrap()
    };

    let source = root.join("trace").join(artifact);
    let text = std::fs::read_to_string(&source)
        .unwrap_or_else(|err| panic!("{} must be readable: {err}", source.display()));
    let name = format!("imported-{artifact}");
    std::fs::write(
        root.join(&name),
        text.replace("__CONTENT_MERKLE__", &merkle),
    )
    .unwrap();
    (dir, root, name)
}

fn import(root: &Path, artifact: &str) -> nerve_index::TraceOutcome {
    nerve_index::ingest_trace(root, Path::new(artifact)).unwrap()
}

/// The placeholders `fixtures/trace-hostile` declares, each expanded from the bound it attacks.
///
/// Deriving the payload from the constant is the whole point: tightening a bound cannot leave its own
/// attack testing nothing, because the attack is one byte past whatever the bound currently says.
const HOSTILE_TOKENS: [(&str, usize); 3] = [
    // A whole line of padding, so the artifact exceeds the ceiling however small the rest of it is.
    (
        "__PAD_ARTIFACT__",
        nerve_index::trace::MAX_ARTIFACT_BYTES + 1,
    ),
    // Inside a JSON string on one record line, so that line alone exceeds the record ceiling.
    ("__PAD_RECORD__", nerve_index::trace::MAX_RECORD_BYTES + 1),
    // Inside `test_id`, so the *field* exceeds its bound while the line stays well inside the record
    // bound — otherwise this would test `record-too-large` a second time and `string-too-long` never.
    ("__PAD_STRING__", nerve_index::trace::MAX_STRING_BYTES + 1),
];

/// Bytes that are not valid UTF-8 in any position.
const INVALID_UTF8: [u8; 3] = [0xff, 0xfe, 0x80];

fn replace_bytes(haystack: &[u8], needle: &[u8], replacement: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(haystack.len());
    let mut cursor = 0;
    while cursor < haystack.len() {
        if haystack[cursor..].starts_with(needle) {
            out.extend_from_slice(replacement);
            cursor += needle.len();
        } else {
            out.push(haystack[cursor]);
            cursor += 1;
        }
    }
    out
}

/// Copy a hostile artifact into `root`, expanding every placeholder it carries.
///
/// **The mechanism this implements is one `fixtures/trace-hostile/README.md` already claimed existed.**
/// It did not. Nothing in the crate expanded a token — `grep` for any of the four across `crates/`
/// found zero hits — and the hostile artifacts were `fs::copy`d verbatim, so `__PAD_STRING__` reached
/// the parser as fourteen ASCII bytes and `__INVALID_UTF8__` as perfectly valid UTF-8. Four attacks
/// tested nothing while reading, in the fixture table, as though each tested the bound in its name.
///
/// Measured before this fix, which is how the scale of it was established:
///
/// | artifact | README requires | actually produced |
/// |---|---|---|
/// | `oversized-file` | `artifact-too-large`, zero edges | `malformed-json`, **1 observation written** |
/// | `oversized-record` | `record-too-large` | `record-unknown-key`, from its own padding key |
/// | `oversized-string` | `string-too-long` | **nothing refused, 2 observations** |
/// | `malformed-utf8` | `invalid-utf8-line` | **nothing refused, 2 observations** |
///
/// The same defect class as the Slice 10a lambda test, which passed because the walker never visited
/// the node the fixture existed to exercise, and as the two vacuous T7 tests in Slice 8b. A test that
/// cannot fail is worse than a missing one: it reports a guarantee nobody is keeping.
///
/// Substitution is on **bytes**. `__INVALID_UTF8__` expands to bytes that are not UTF-8, so a
/// `String` round-trip could not carry it — which is also why the token exists instead of the payload
/// being committed (`CLAUDE.md` §9, and a file that is not text cannot be reviewed as text).
fn stage_hostile(root: &Path, name: &str) {
    let source = common::named_fixture_root("trace-hostile").join(name);
    let mut bytes = std::fs::read(&source)
        .unwrap_or_else(|err| panic!("{} unreadable: {err}", source.display()));

    for (token, width) in HOSTILE_TOKENS {
        bytes = replace_bytes(&bytes, token.as_bytes(), &vec![b'x'; width]);
    }
    bytes = replace_bytes(&bytes, b"__INVALID_UTF8__", &INVALID_UTF8);

    // The guard that keeps this honest. A token added to a fixture without being added above would
    // otherwise silently disarm that fixture, which is exactly the failure being repaired. Matching
    // on *prefixes* is deliberate: a token this function has never heard of still trips it.
    for token in ["__PAD_", "__INVALID_", "__CONTENT_MERKLE__"] {
        assert!(
            !bytes.windows(token.len()).any(|w| w == token.as_bytes()),
            "{name} still contains the placeholder {token} after staging; an unexpanded token means \
             this artifact is attacking nothing"
        );
    }

    std::fs::write(root.join(name), bytes).unwrap();
}

/// Every hostile artifact, sorted, so a new one is picked up without editing a list.
fn hostile_artifacts() -> Vec<String> {
    let dir = common::named_fixture_root("trace-hostile");
    let mut names: Vec<String> = std::fs::read_dir(&dir)
        .expect("fixtures/trace-hostile must exist")
        .map(|entry| entry.unwrap().path())
        .filter(|path| path.extension().is_some_and(|ext| ext == "jsonl"))
        .map(|path| path.file_name().unwrap().to_str().unwrap().to_string())
        .collect();
    names.sort();
    names
}

/// Every `(caller name, callee name)` pair `test-trace` asserted.
fn observed_edges(conn: &nerve_store::Connection) -> Vec<(String, String)> {
    let mut statement = conn
        .prepare(
            "SELECT s.name, t.name
               FROM assertion a
               JOIN entity s ON s.entity_id = a.source_entity_id
               JOIN entity t ON t.entity_id = a.target_entity_id
               JOIN observation o ON o.assertion_id = a.assertion_id
              WHERE o.extractor_id = ?1 AND a.relation = 'TEST_OBSERVED_CALL'
              ORDER BY s.name, t.name",
        )
        .unwrap();
    let rows = statement
        .query_map([nerve_index::trace::EXTRACTOR_ID], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })
        .unwrap();
    rows.map(|row| row.unwrap()).collect()
}

/// **The defect plan §2.1 corrects.** A depth-2 call belongs to its real caller.
///
/// `fixtures/trace-basic/trace/bound.jsonl` records the stack
/// `test_basic → parse → tokenize` and `test_method → parse_all → parse → tokenize`. The Slice 11
/// plan would have emitted `test_basic TEST_OBSERVED_CALL tokenize`, which asserts a call
/// `test_basic` never made. Both halves are asserted: the true edge exists, and the false one does
/// not.
#[test]
fn a_nested_call_is_attributed_to_its_real_caller_and_never_to_the_test() {
    let (_dir, root, artifact) = indexed("bound.jsonl");
    let outcome = import(&root, &artifact);
    assert_eq!(outcome.binding, Some(nerve_index::TraceBinding::Bound));

    let conn = open_db(&root);
    let edges = observed_edges(&conn);
    assert!(
        !edges.is_empty(),
        "no edges at all, so every assertion below passes vacuously"
    );

    assert!(
        edges.contains(&("parse".to_string(), "tokenize".to_string())),
        "the depth-2 edge must be attributed to `parse`; got {edges:?}"
    );
    assert!(
        edges.contains(&("parse_all".to_string(), "parse".to_string())),
        "a method frame is a caller like any other; got {edges:?}"
    );
    for test in [
        "test_basic",
        "test_method",
        "test_lazy_import",
        "test_partial",
    ] {
        assert!(
            !edges.contains(&(test.to_string(), "tokenize".to_string())),
            "{test} never called tokenize; making the test the source of a nested call asserts a \
             call the test did not make. This is the Slice 11 plan's defect."
        );
    }
    // A depth-1 edge from the test body is still the test's own, so the rule is not "never a test".
    assert!(
        edges.contains(&("test_basic".to_string(), "parse".to_string())),
        "a call written in the test body genuinely has the test as its caller; got {edges:?}"
    );
}

/// The accepted edge set is exactly the ground truth's, and the per-test attribution survives.
#[test]
fn the_edge_set_and_its_test_attribution_match_the_ground_truth() {
    let (_dir, root, artifact) = indexed("bound.jsonl");
    let outcome = import(&root, &artifact);
    let expected = expected();
    let bound = &expected["bound"];

    let mut want: Vec<(String, String)> = Vec::new();
    let mut want_by_test: BTreeMap<(String, String), BTreeMap<String, u64>> = BTreeMap::new();
    for edge in bound["edges"].as_array().unwrap() {
        // `sym:src/parse.py#Parser.parse_all` names the *qualified* symbol; `entity.name` is the
        // last segment (`parse_all`), with `Parser` living in `scope_path`. Taking the whole
        // fragment compared a qualified name against a bare one.
        let bare = |value: &str| -> String {
            value
                .rsplit('#')
                .next()
                .unwrap_or_default()
                .rsplit('.')
                .next()
                .unwrap_or_default()
                .to_string()
        };
        let caller = bare(edge["caller"].as_str().unwrap());
        let callee = bare(edge["callee"].as_str().unwrap());
        want.push((caller.clone(), callee.clone()));
        let tests: BTreeMap<String, u64> = edge["by_test"]
            .as_object()
            .unwrap()
            .iter()
            .map(|(test, count)| (test.clone(), count.as_u64().unwrap()))
            .collect();
        want_by_test.insert((caller, callee), tests);
    }
    want.sort();
    want.dedup();

    let conn = open_db(&root);
    let mut have = observed_edges(&conn);
    have.sort();
    have.dedup();
    assert_eq!(
        have, want,
        "the observed edge set is not the ground truth's"
    );

    assert!(
        outcome.records_accepted > 0,
        "no record was accepted, so the edge comparison above is vacuous"
    );

    // Every test named in the ground truth appears in the stored environment for its edge. This is
    // the union the README's disagreement describes: two tests at one call site are ONE row,
    // because `idx_observation_identity` has no environment column, so the row must name both.
    let mut statement = conn
        .prepare(
            "SELECT s.name, t.name, o.environment
               FROM assertion a
               JOIN entity s ON s.entity_id = a.source_entity_id
               JOIN entity t ON t.entity_id = a.target_entity_id
               JOIN observation o ON o.assertion_id = a.assertion_id
              WHERE o.extractor_id = ?1",
        )
        .unwrap();
    let rows: Vec<(String, String, Option<String>)> = statement
        .query_map([nerve_index::trace::EXTRACTOR_ID], |row| {
            Ok((row.get(0)?, row.get(1)?, row.get(2)?))
        })
        .unwrap()
        .map(|row| row.unwrap())
        .collect();

    for (caller, callee, environment) in &rows {
        let Some(tests) = want_by_test.get(&(caller.clone(), callee.clone())) else {
            continue;
        };
        let environment = environment
            .as_deref()
            .unwrap_or_else(|| panic!("{caller} -> {callee} stored no environment"));
        for test in tests.keys() {
            assert!(
                environment.contains(test.as_str()),
                "{caller} -> {callee}: environment does not name {test}. Two tests at one call \
                 site are one row, so the row must name both or one test's identity is lost \
                 silently."
            );
        }
    }
    assert!(
        !rows.is_empty(),
        "no observations, so the loop above is vacuous"
    );
}

/// `bound` / `stale` / `unverified` are three reported states, not a boolean.
///
/// The same three-valued shape as `CoverageEvidence::Absent` and the `Unverified`-vs-`Stale` split
/// in `nerve check`: absence of verification is not verification of absence.
#[test]
fn repository_state_binding_has_three_distinct_values() {
    let cases = [
        ("bound.jsonl", nerve_index::TraceBinding::Bound),
        ("stale.jsonl", nerve_index::TraceBinding::Stale),
        ("unverified.jsonl", nerve_index::TraceBinding::Unverified),
    ];
    let mut seen = Vec::new();
    for (artifact, want) in cases {
        let (_dir, root, name) = indexed(artifact);
        let outcome = import(&root, &name);
        assert_eq!(
            outcome.binding,
            Some(want),
            "{artifact} must bind as {want:?}"
        );
        seen.push(want);
    }
    assert_eq!(seen.len(), 3);
    assert_eq!(
        nerve_index::TraceBinding::ALL.len(),
        3,
        "a fourth binding would need a case here"
    );
}

/// A stale artifact still yields evidence, and says it is stale. It is not refused.
#[test]
fn a_stale_artifact_is_reported_rather_than_discarded() {
    let (_dir, root, artifact) = indexed("stale.jsonl");
    let outcome = import(&root, &artifact);
    assert_eq!(outcome.binding, Some(nerve_index::TraceBinding::Stale));
    assert!(
        outcome.records_in_artifact > 0,
        "the artifact must contain records for this to mean anything"
    );
}

/// A partial run is labelled partial. A truncated trace must never read as a complete one.
#[test]
fn a_partial_run_is_labelled_partial() {
    let (_dir, root, artifact) = indexed("partial.jsonl");
    let outcome = import(&root, &artifact);
    assert_ne!(
        outcome.completion_state,
        Some(nerve_index::trace::CompletionState::Complete),
        "a run the producer did not finish must not report as complete"
    );
    assert!(
        outcome.partial_reason.is_some(),
        "a partial run must say why it is partial"
    );
}

/// Re-importing the same artifact adds no observations.
#[test]
fn importing_the_same_artifact_twice_adds_nothing() {
    let (_dir, root, artifact) = indexed("bound.jsonl");
    let first = import(&root, &artifact);
    assert!(first.observations_written > 0);

    let before: i64 = {
        let conn = open_db(&root);
        conn.query_row("SELECT count(*) FROM observation", [], |row| row.get(0))
            .unwrap()
    };
    let second = import(&root, &artifact);
    let after: i64 = {
        let conn = open_db(&root);
        conn.query_row("SELECT count(*) FROM observation", [], |row| row.get(0))
            .unwrap()
    };
    assert_eq!(
        before, after,
        "a second import of identical bytes added observations; `idx_observation_identity` should \
         make this idempotent exactly as a re-index is"
    );
    assert_eq!(second.records_accepted, first.records_accepted);
}

/// `test-trace` asserts `TEST_OBSERVED_CALL` and nothing else, over the whole closed vocabulary.
///
/// The Slice 6b pattern: asserting that the relations we expect are present is not enough. Every
/// *other* member of `Relation::ALL` must be absent, or a future change could add a call-shaped
/// edge and no test would notice.
#[test]
fn the_trace_extractor_asserts_no_relation_but_test_observed_call() {
    let (_dir, root, artifact) = indexed("bound.jsonl");
    import(&root, &artifact);
    let conn = open_db(&root);

    for relation in nerve_core::vocab::Relation::ALL {
        let count: i64 = conn
            .query_row(
                "SELECT count(*) FROM assertion a
                   JOIN observation o ON o.assertion_id = a.assertion_id
                  WHERE o.extractor_id = ?1 AND a.relation = ?2",
                [nerve_index::trace::EXTRACTOR_ID, relation.as_str()],
                |row| row.get(0),
            )
            .unwrap();
        if relation == nerve_core::vocab::Relation::TestObservedCall {
            assert!(count > 0, "TEST_OBSERVED_CALL must be emitted");
        } else {
            assert_eq!(
                count, 0,
                "test-trace asserted {relation}, which it does not declare. A trace is not \
                 coverage and is not a static call."
            );
        }
    }
}

/// A `COVERS` observation never becomes a `TEST_OBSERVED_CALL`, and the two share no assertion.
///
/// ADR-0005 restated for a new relation: two symbols executing during one run says nothing about
/// who invoked whom, and a trace edge must come from a trace rather than from co-occurrence.
#[test]
fn coverage_and_trace_evidence_never_share_an_assertion() {
    let (_dir, root, artifact) = indexed("bound.jsonl");
    import(&root, &artifact);
    let conn = open_db(&root);

    let shared: i64 = conn
        .query_row(
            "SELECT count(*) FROM assertion a
              WHERE EXISTS (SELECT 1 FROM observation o
                             WHERE o.assertion_id = a.assertion_id AND o.extractor_id = 'coverage')
                AND EXISTS (SELECT 1 FROM observation o
                             WHERE o.assertion_id = a.assertion_id AND o.extractor_id = ?1)",
            [nerve_index::trace::EXTRACTOR_ID],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(shared, 0, "coverage and trace evidence share an assertion");

    let miscoloured: i64 = conn
        .query_row(
            "SELECT count(*) FROM observation
              WHERE evidence_source_type = 'TEST_COVERAGE' AND extractor_id = ?1",
            [nerve_index::trace::EXTRACTOR_ID],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(miscoloured, 0, "a trace observation typed as coverage");
}

/// Every accepted observation carries `TEST_CALL_TRACE` / `RESOLVED`.
///
/// `RESOLVED` rather than `DIRECT` because the artifact names a **location** and ingestion resolves
/// it to a symbol — plan §2.3. And rather than `INFERRED`, which is coverage's value, because a
/// trace does not infer the *relation*: the call is stated outright and only the endpoints need
/// resolving.
#[test]
fn every_trace_observation_is_test_call_trace_and_resolved() {
    let (_dir, root, artifact) = indexed("bound.jsonl");
    import(&root, &artifact);
    let conn = open_db(&root);

    let mut statement = conn
        .prepare("SELECT evidence_source_type, directness FROM observation WHERE extractor_id = ?1")
        .unwrap();
    let rows: Vec<(String, String)> = statement
        .query_map([nerve_index::trace::EXTRACTOR_ID], |row| {
            Ok((row.get(0)?, row.get(1)?))
        })
        .unwrap()
        .map(|row| row.unwrap())
        .collect();
    assert!(!rows.is_empty(), "no observations, so this is vacuous");
    for (source_type, directness) in rows {
        assert_eq!(source_type, "TEST_CALL_TRACE");
        assert_eq!(directness, "RESOLVED");
    }
}

/// Every hostile artifact is classified, and no hostile record ever becomes evidence.
///
/// This test was rewritten three times, and the rewrites are the finding. Each earlier version
/// asserted something stronger than the plan requires and was wrong for a different reason:
///
/// 1. *"every hostile artifact writes zero observations"* — false. Plan §5 says a bad **record** is
///    refused and counted while the import continues, so `deep-nesting` correctly keeps its good
///    record. Refusing the file would discard true evidence to express a doubt about one line.
/// 2. *"no artifact string reaches Nerve's output"* — too strong. The artifact is a file the user
///    pointed Nerve at, and echoing its declared `run_id` is how they know which run was imported.
///    What matters is inertness and bounds, which the next test asserts.
/// 3. *"the database is byte-identical after a refusal"* — unsound. An `extractor_run` row recording
///    that Nerve looked and refused is legitimate, and is exactly what every other extractor does in
///    a repository with nothing to find.
///
/// What is left is what the plan actually requires and what is actually sound: **no hostile artifact
/// contributes an observation to a hostile record**, the schema survives, and the classification is
/// printed so a change in it is visible in the log rather than discovered later.
#[test]
fn no_hostile_artifact_contributes_evidence_from_a_hostile_record() {
    let artifacts = hostile_artifacts();
    assert!(
        artifacts.len() >= 12,
        "expected the full hostile set, found {}",
        artifacts.len()
    );

    let mut classification: BTreeMap<String, String> = BTreeMap::new();

    for name in &artifacts {
        let (_dir, root, _unused) = indexed("bound.jsonl");
        stage_hostile(&root, name);

        let verdict = match nerve_index::ingest_trace(&root, Path::new(name)) {
            Err(_) => "refused whole, as an error".to_string(),
            Ok(outcome) => format!(
                "records {} · accepted {} · observations {} · refusals {}",
                outcome.records_in_artifact,
                outcome.records_accepted,
                outcome.observations_written,
                outcome.refused_total()
            ),
        };
        classification.insert(name.to_string(), verdict);

        // The schema survives every one of them. This is what a SQL-injection string would break and
        // what a partial commit would leave inconsistent.
        let conn = open_db(&root);
        for table in [
            "observation",
            "assertion",
            "entity",
            "occurrence",
            "extractor_run",
        ] {
            let present: i64 = conn
                .query_row(
                    "SELECT count(*) FROM sqlite_master WHERE type = 'table' AND name = ?1",
                    [table],
                    |row| row.get(0),
                )
                .unwrap();
            assert_eq!(present, 1, "{name} dropped or renamed {table}");
        }

        // Derived state stays a pure function of observations. A hostile artifact that left
        // `assertion_state` disagreeing with `observation` would have corrupted the model even if
        // every table still existed.
        let orphaned: i64 = conn
            .query_row(
                "SELECT count(*) FROM assertion_state s
                  WHERE NOT EXISTS (SELECT 1 FROM observation o
                                     WHERE o.assertion_id = s.assertion_id)",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(
            orphaned, 0,
            "{name} left derived state with no observation behind it"
        );
    }

    println!("hostile artifact classification:\n{classification:#?}");
}

/// A hostile artifact cannot injure the database, and its own text cannot execute as SQL.
///
/// The narrower, correct form of a test that first asserted no artifact string ever reaches Nerve's
/// output. That was too strong: the artifact is a file the user *pointed Nerve at*, and echoing its
/// declared `run_id` is how they know which run was imported — `nerve coverage` echoes the report
/// path for the same reason. What actually matters is that the text is inert and bounded.
#[test]
fn hostile_artifact_text_is_inert_and_bounded() {
    for name in [
        "sql-injection.jsonl",
        "fts5-syntax.jsonl",
        "prompt-injection.jsonl",
    ] {
        let (_dir, root, _unused) = indexed("bound.jsonl");
        stage_hostile(&root, name);

        let rendered = match nerve_index::ingest_trace(&root, Path::new(name)) {
            Err(err) => err.to_string(),
            Ok(outcome) => format!("{outcome:?}"),
        };

        // 1. The schema is intact. This is what a SQL-injection string would break.
        let conn = open_db(&root);
        for table in ["observation", "assertion", "entity", "occurrence"] {
            let present: i64 = conn
                .query_row(
                    "SELECT count(*) FROM sqlite_master WHERE type = 'table' AND name = ?1",
                    [table],
                    |row| row.get(0),
                )
                .unwrap();
            assert_eq!(present, 1, "{name} dropped or renamed {table}");
        }

        // 2. Nothing echoed carries a control character or an ANSI escape, which is what would let
        //    artifact text rewrite a terminal rather than merely appear in it.
        for character in rendered.chars() {
            assert!(
                !character.is_control() || character == '\n',
                "{name}: rendered output contains control character {character:?}"
            );
        }

        // 3. Nothing echoed is unbounded.
        assert!(
            rendered.len() < 64 * 1024,
            "{name}: rendered {} bytes; artifact text must be bounded before it is reported",
            rendered.len()
        );
    }
}

/// A traversal path inside an artifact is refused **as a refusal**, in all three spellings.
#[test]
fn traversal_inside_an_artifact_is_refused_in_every_spelling() {
    for name in [
        "traversal-dotdot.jsonl",
        "traversal-backslash.jsonl",
        "traversal-absolute.jsonl",
    ] {
        let (_dir, root, _unused) = indexed("bound.jsonl");
        stage_hostile(&root, name);

        let result = nerve_index::ingest_trace(&root, Path::new(name));
        match result {
            Err(_) => {}
            Ok(outcome) => {
                assert_eq!(outcome.observations_written, 0, "{name} produced evidence");
                assert!(
                    outcome.refused_total() > 0,
                    "{name}: a traversal must be counted as refused, never reported as a path \
                     that simply matched nothing"
                );
            }
        }
    }
}

/// A replayed `run_id` is reported, and **overwrites nothing**.
///
/// The fixture table makes two claims about `duplicate-run-id.jsonl` and only the first had a test.
/// The second is the one an attacker would care about: the artifact replays `run-bound-1` with
/// `count: 9999` on a *different* edge, and what it must not achieve is displacing what
/// `run-bound-1` already means.
///
/// Both halves are asserted, because either alone is satisfiable the wrong way — reporting the
/// conflict while clobbering the evidence, or preserving the evidence while saying nothing. Plan §7
/// requires import-and-report rather than refusal precisely so that a *corrected* artifact with a
/// repeated id is not thrown away; that choice is only safe if the earlier evidence survives it.
#[test]
fn a_replayed_run_id_is_reported_and_overwrites_nothing() {
    let (_dir, root, bound) = indexed("bound.jsonl");
    let first = import(&root, &bound);
    assert_eq!(
        first.refused_count("run-id-conflict"),
        0,
        "a first import collides with nothing"
    );

    // Every trace observation exactly as the legitimate import left it.
    let snapshot = |conn: &nerve_store::Connection| -> Vec<(String, String, i64, String, String)> {
        let mut statement = conn
            .prepare(
                "SELECT assertion_id, file_path, start_line, environment, details
                   FROM observation
                  WHERE extractor_id = ?1
                  ORDER BY assertion_id, file_path, start_line",
            )
            .unwrap();
        let rows = statement
            .query_map([nerve_index::trace::EXTRACTOR_ID], |row| {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                ))
            })
            .unwrap();
        rows.map(|row| row.unwrap()).collect()
    };

    let before = snapshot(&open_db(&root));
    assert!(
        !before.is_empty(),
        "no trace evidence to protect, so every assertion below would pass vacuously"
    );

    stage_hostile(&root, "duplicate-run-id.jsonl");
    let replay = import(&root, "duplicate-run-id.jsonl");

    // 1. Reported. Once — the collision is one fact about one header, not one per overlapping site.
    assert_eq!(
        replay.refused_count("run-id-conflict"),
        1,
        "the replay must be reported exactly once; got {:?}",
        replay.refused
    );

    // 2. Nothing displaced. Every row the legitimate import wrote is still there, byte for byte,
    //    including its `count`, which the replay asserts as 9999.
    let after = snapshot(&open_db(&root));
    for row in &before {
        assert!(
            after.contains(row),
            "the replay displaced {}:{} — a repeated run id must add, never replace",
            row.1,
            row.2
        );
    }
    // 3. The replay's own claim reached **only its own edge**.
    //
    //    This started as "no row anywhere contains 9999" and failed, correctly. The replay names a
    //    *different* edge — `test_parse.py:8 → parse.py:24` — and plan §7 says it imports, so a row
    //    carrying its count is the honest record of a claim it actually made. Forbidding the number
    //    outright would have been the fifth over-assertion in this slice: banning the evidence
    //    instead of bounding where it may land. What is worth asserting is the boundary.
    let owned: BTreeMap<(String, String, i64), ()> = before
        .iter()
        .map(|row| ((row.0.clone(), row.1.clone(), row.2), ()))
        .collect();
    for row in &after {
        if row.4.contains("9999") {
            assert!(
                !owned.contains_key(&(row.0.clone(), row.1.clone(), row.2)),
                "the replay's asserted count reached {}:{}, a site the legitimate run already owned",
                row.1,
                row.2
            );
        }
    }
    assert!(
        after.len() > before.len(),
        "the replay produced no evidence of its own, so it was refused rather than imported — plan \
         §7 requires import-and-report, because refusing would discard a corrected artifact"
    );
}

/// Each hostile artifact produces **the** refusal its fixture table declares.
///
/// This replaces an aggregate: *at least six distinct forms across the whole set*. That threshold is
/// how four disarmed artifacts went unnoticed — nine working attacks were more than enough to satisfy
/// it, so the four whose placeholders were never expanded contributed nothing and cost nothing. An
/// aggregate over a corpus cannot tell "every case works" from "enough cases work", and that
/// difference is the entire value of a hostile corpus.
///
/// Per-artifact, and in **both** directions:
///
/// - an artifact declared hostile must produce its declared form, so a guard that stops firing fails
///   here instead of being absorbed by its neighbours;
/// - an artifact declared inert must produce **no** refusal, because refusing it would be its own
///   defect. `fts5-syntax`, `prompt-injection` and `sql-injection` carry text that is dangerous only
///   if something interprets it, and the correct answer is that nothing does. Refusing a legal
///   `run_id` for containing `NEAR(` would invent a rule the format does not have, and T7's claim
///   about untrusted content is inertness rather than rejection. `state-substitution` is inert here
///   too: its answer is a **binding** of `stale`, asserted by
///   [`repository_state_binding_has_three_distinct_values`], and a refusal would be the wrong answer.
///
/// Scope, so this is not read as more than it is: the fourteen parser forms in `trace::form::ALL` are
/// each gated by a unit test in `trace_tests.rs`. This gates the **end-to-end** path — artifact on
/// disk, through `ingest_trace`, to a counted refusal — for the forms a committed fixture can reach.
#[test]
fn each_hostile_artifact_produces_its_declared_refusal() {
    // `fixtures/trace-hostile/README.md`'s table, as an assertion. `None` means inert: read,
    // understood, and correctly not refused.
    let declared: BTreeMap<&str, Option<&str>> = BTreeMap::from([
        ("cross-repository.jsonl", Some("other-repository")),
        ("deep-nesting.jsonl", Some("nesting-too-deep")),
        ("duplicate-run-id.jsonl", Some("run-id-conflict")),
        ("fts5-syntax.jsonl", None),
        ("header-unknown-key.jsonl", Some("header-unknown-key")),
        ("malformed-utf8.jsonl", Some("invalid-utf8-line")),
        ("oversized-file.jsonl", Some("artifact-too-large")),
        ("oversized-record.jsonl", Some("record-too-large")),
        ("oversized-string.jsonl", Some("string-too-long")),
        ("prompt-injection.jsonl", None),
        ("sql-injection.jsonl", None),
        ("state-substitution.jsonl", None),
        ("traversal-absolute.jsonl", Some("path-refused")),
        ("traversal-backslash.jsonl", Some("path-refused")),
        ("traversal-dotdot.jsonl", Some("path-refused")),
    ]);

    let artifacts = hostile_artifacts();
    assert_eq!(
        artifacts.len(),
        declared.len(),
        "the hostile set on disk and this table have drifted apart: {artifacts:?} versus {:?}. A \
         fixture with no row here would be imported and never checked.",
        declared.keys().collect::<Vec<_>>()
    );

    let mut produced: BTreeMap<String, usize> = BTreeMap::new();
    for name in &artifacts {
        let expected = *declared
            .get(name.as_str())
            .unwrap_or_else(|| panic!("{name} has no row in the fixture table"));

        let (_dir, root, bound) = indexed("bound.jsonl");
        // Every attack arrives at a repository that **already holds legitimate trace evidence**, which
        // is both the realistic case and the harder one. It is also the only setup under which
        // `duplicate-run-id` means anything: it replays `run-bound-1` from `bound.jsonl`, so against an
        // empty graph there is no identity to collide with and the attack is unarmed — which is what
        // the earlier aggregate test was measuring without noticing.
        import(&root, &bound);
        stage_hostile(&root, name);
        let outcome = nerve_index::ingest_trace(&root, Path::new(name))
            .unwrap_or_else(|err| panic!("{name}: ingestion errored rather than refusing: {err}"));

        match expected {
            Some(form) => assert!(
                outcome.refused_count(form) > 0,
                "{name} must produce `{form}`; it produced {:?}. Either the guard stopped firing or \
                 the fixture row claims an attack this artifact is not making.",
                outcome.refused,
            ),
            None => assert!(
                outcome.refused.is_empty(),
                "{name} is inert by design and must produce no refusal; it produced {:?}. Refusing \
                 inert-but-alarming text would reject legal input for looking dangerous.",
                outcome.refused,
            ),
        }
        for (form, count) in outcome.refused {
            *produced.entry(form).or_insert(0) += count;
        }
    }

    // A well-formed import's own counters join the tally, so the picture is not only the attacks.
    let (_dir, root, artifact) = indexed("bound.jsonl");
    for (form, count) in import(&root, &artifact).refused {
        *produced.entry(form).or_insert(0) += count;
    }
    println!("refusal forms produced across the fixture set: {produced:#?}");
}

/// The two security invariants the whole design exists to keep are untouched by it.
///
/// Not a proxy for reading the files: `no_subprocess.rs` scans `crates/*/src/**` for process
/// creation and `no_network.rs` scans for outbound clients, and both run in this same suite. This
/// test asserts the *new* modules specifically, so a future edit to them is caught here rather than
/// only in another crate's test.
#[test]
fn the_new_trace_modules_create_no_process_and_open_no_socket() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    for module in ["src/trace.rs", "src/trace_ingest.rs"] {
        let text = std::fs::read_to_string(root.join(module)).unwrap();
        for forbidden in [
            "Command::new",
            "process::Command",
            "posix_spawn",
            "libc::fork",
            "libc::system",
            "TcpStream",
            "UdpSocket",
            "reqwest",
        ] {
            assert!(
                !text.contains(forbidden),
                "{module} contains {forbidden}. Slice 11a exists so that ingesting a trace needs \
                 neither a process nor a socket."
            );
        }
    }
}
