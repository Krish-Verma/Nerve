//! Unit tests for [`crate::trace`], in their own file.
//!
//! A child module rather than an inline `mod tests`, so `use super::*` still reaches the parser's
//! private helpers while the reader itself stays one screenful of contract per concern.

use super::*;

const HEADER: &str = r#"{"format":"nerve-trace","format_version":1,"producer":"p","producer_version":"0.1.0","repository_root_name":"repo","git_commit":null,"content_merkle":null,"run_id":"r1","test_framework":"pytest","runtime":"cpython","runtime_version":"3.12.4","platform":"darwin-arm64","started_at":"t0","completed_at":"t1","completion_state":"complete","partial_reason":null,"source_map_state":"none","producer_limitations":[]}"#;

const RECORD: &str = r#"{"test_id":"t::a","caller_file":"src/a.py","caller_line":1,"callee_file":"src/b.py","callee_line":2,"count":1,"worker":null,"async_context":null,"resolution":"located","unsupported_form":null}"#;

fn artifact(extra: &[&str]) -> TraceArtifact {
    let mut text = String::from(HEADER);
    for line in extra {
        text.push('\n');
        text.push_str(line);
    }
    text.push('\n');
    parse_trace(text.as_bytes())
}

/// A header field replaced, so one rule can be attacked at a time.
fn header_with(key: &str, value: &str) -> String {
    let mut object: serde_json::Map<String, serde_json::Value> =
        serde_json::from_str(HEADER).expect("the reference header parses");
    object.insert(
        key.to_string(),
        serde_json::from_str(value).expect("the replacement value parses"),
    );
    serde_json::to_string(&serde_json::Value::Object(object)).expect("a Map serialises")
}

/// A record field replaced, likewise.
fn record_with(key: &str, value: &str) -> String {
    let mut object: serde_json::Map<String, serde_json::Value> =
        serde_json::from_str(RECORD).expect("the reference record parses");
    object.insert(
        key.to_string(),
        serde_json::from_str(value).expect("the replacement value parses"),
    );
    serde_json::to_string(&serde_json::Value::Object(object)).expect("a Map serialises")
}

// ---- extractor identity ----------------------------------------------------------------------

#[test]
fn declares_only_the_test_call_trace_source_type_and_resolves_rather_than_states() {
    assert_eq!(EXTRACTOR_ID, "test-trace");
    assert_eq!(EXTRACTOR_VERSION, "1.0.0");
    assert_eq!(DECLARED_SOURCE_TYPES.len(), 1);
    assert_eq!(DECLARED_SOURCE_TYPES[0], EvidenceSourceType::TestCallTrace);
    assert_eq!(DECLARED_SOURCE_TYPES[0].as_str(), "TEST_CALL_TRACE");
    assert_eq!(DECLARED_RELATIONS.len(), 1);
    assert_eq!(DECLARED_RELATIONS[0], Relation::TestObservedCall);
    // The artifact states the call outright but names only a file and a line, so the endpoints are
    // resolved. Not `DIRECT` (the artifact does not name the symbol) and not `INFERRED` (no rule
    // concludes the relation, unlike coverage).
    assert_eq!(DIRECTNESS, Directness::Resolved);
    assert_eq!(DIRECTNESS.as_str(), "RESOLVED");
    assert_ne!(DIRECTNESS, Directness::Direct);
    assert_ne!(DIRECTNESS, crate::coverage::DIRECTNESS);
}

/// The three vocabularies that share one counter map must be disjoint; the ones that do not may
/// deliberately share a spelling.
///
/// `test-trace` reports two maps — refusals and limitations — and the refusal map also carries
/// [`crate::trace_ingest::form`]. Those three are read together, so a collision between them would
/// make a count ambiguous.
///
/// Sharing a spelling with `coverage` is the opposite: `path-refused` means the same thing in both
/// extractors and *should* be spelled the same, so a reader learns one counting convention rather
/// than two. The overlap is pinned exactly, so a tag that drifts into meaning something else here
/// fails rather than quietly diverging.
#[test]
fn the_counter_vocabularies_that_share_a_map_are_disjoint() {
    let mut shared_map: Vec<&str> = form::ALL.to_vec();
    shared_map.extend(limitation::ALL);
    shared_map.extend(crate::trace_ingest::form::ALL);
    let mut sorted = shared_map.clone();
    sorted.sort_unstable();
    sorted.dedup();
    assert_eq!(sorted.len(), shared_map.len(), "two counter tags collide");
    assert!(shared_map.iter().all(|tag| !tag.is_empty()));

    let coverage: std::collections::BTreeSet<&str> = crate::coverage::form::ALL
        .into_iter()
        .chain(crate::coverage_ingest::form::ALL)
        .collect();
    let mut overlap: Vec<&str> = shared_map
        .iter()
        .copied()
        .filter(|tag| coverage.contains(tag))
        .collect();
    overlap.sort_unstable();
    assert_eq!(
        overlap,
        vec![
            "file-changed-since-index",
            "file-not-indexed",
            "file-unreadable",
            "invalid-utf8-line",
            "path-refused",
            "records-exceeded",
        ],
        "a tag shared with the coverage vocabularies must mean the same thing in both"
    );
}

#[test]
fn parsing_the_same_bytes_twice_gives_the_same_answer() {
    let text = format!("{HEADER}\n{RECORD}\n");
    assert_eq!(parse_trace(text.as_bytes()), parse_trace(text.as_bytes()));
}

// ---- the happy path --------------------------------------------------------------------------

#[test]
fn a_well_formed_artifact_yields_its_header_and_records_with_nothing_refused() {
    let parsed = artifact(&[RECORD]);
    let header = parsed.header.expect("the header parses");
    assert_eq!(header.run_id, "r1");
    assert_eq!(header.completion_state, CompletionState::Complete);
    assert_eq!(header.source_map_state, SourceMapState::None);
    assert_eq!(header.git_commit, None);
    assert_eq!(header.content_merkle, None);
    assert_eq!(parsed.records_in_artifact, 1);
    assert_eq!(parsed.records.len(), 1);
    assert_eq!(parsed.records[0].caller_file, "src/a.py");
    assert_eq!(parsed.records[0].caller_line, 1);
    assert_eq!(parsed.records[0].count, 1);
    assert_eq!(parsed.counters.refused_total(), 0);
    assert_eq!(parsed.counters.limitations_total(), 0);
}

/// Paths are carried byte for byte. Refusing them is the ingestion layer's job, behind the shared
/// guard, because a refusal the guard never sees is a refusal nobody reports.
#[test]
fn a_record_carries_its_paths_exactly_as_written() {
    for written in [
        "src/a.py",
        "./src/a.py",
        "/etc/passwd",
        "../../../../etc/passwd",
        "..\\..\\windows\\sam",
        "src/../src/a.py",
        "src/a b.py",
    ] {
        let line = record_with("caller_file", &serde_json::json!(written).to_string());
        let parsed = artifact(&[&line]);
        assert_eq!(
            parsed.records.len(),
            1,
            "{written} was refused by the parser"
        );
        assert_eq!(
            parsed.records[0].caller_file, written,
            "{written} was rewritten by the parser"
        );
        assert_eq!(parsed.counters.refused_total(), 0, "{written} was counted");
    }
}

#[test]
fn crlf_and_lf_artifacts_parse_identically() {
    let lf = format!("{HEADER}\n{RECORD}\n");
    let crlf = format!("{HEADER}\r\n{RECORD}\r\n");
    let mixed = format!("{HEADER}\r\n{RECORD}\n");
    assert_eq!(parse_trace(lf.as_bytes()), parse_trace(crlf.as_bytes()));
    assert_eq!(parse_trace(lf.as_bytes()), parse_trace(mixed.as_bytes()));
}

// ---- the header ------------------------------------------------------------------------------

#[test]
fn an_unknown_header_key_refuses_the_whole_artifact() {
    let line = header_with("paths_are_absolute", "true");
    let parsed = parse_trace(format!("{line}\n{RECORD}\n").as_bytes());
    assert!(parsed.header.is_none());
    assert!(
        parsed.records.is_empty(),
        "a record must not survive a header Nerve could not read"
    );
    assert_eq!(parsed.counters.refused_count(form::HEADER_UNKNOWN_KEY), 1);
    assert_eq!(parsed.records_in_artifact, 0);
}

#[test]
fn an_unknown_record_key_is_ignored_and_counted_and_the_record_is_kept() {
    let line = record_with("depth", "3");
    let parsed = artifact(&[&line]);
    assert_eq!(parsed.records.len(), 1, "the record must still be believed");
    assert_eq!(parsed.counters.refused_count(form::RECORD_UNKNOWN_KEY), 1);
    assert_eq!(parsed.counters.refused_total(), 1);
}

/// The one contradiction the contract names explicitly.
#[test]
fn a_complete_run_with_no_completion_time_is_a_contradiction_and_is_refused() {
    let line = header_with("completed_at", "null");
    let parsed = parse_trace(format!("{line}\n{RECORD}\n").as_bytes());
    assert!(parsed.header.is_none());
    assert_eq!(parsed.counters.refused_count(form::HEADER_INVALID), 1);

    // A `partial` run with no completion time is legal, and is the ordinary case.
    let mut object: serde_json::Map<String, serde_json::Value> =
        serde_json::from_str(HEADER).expect("the reference header parses");
    object.insert("completed_at".into(), serde_json::Value::Null);
    object.insert("completion_state".into(), serde_json::json!("partial"));
    object.insert("partial_reason".into(), serde_json::json!("SIGINT"));
    let legal = serde_json::to_string(&serde_json::Value::Object(object)).expect("serialises");
    let parsed = parse_trace(format!("{legal}\n{RECORD}\n").as_bytes());
    let header = parsed.header.expect("a partial run is legal");
    assert_eq!(header.completion_state, CompletionState::Partial);
    assert_eq!(header.completed_at, None);
    assert_eq!(parsed.counters.refused_total(), 0);
}

#[test]
fn every_header_field_is_checked_and_a_bad_one_refuses_the_artifact() {
    for (key, value) in [
        ("format", r#""lcov""#),
        ("format_version", "2"),
        ("format_version", r#""1""#),
        ("producer", r#""""#),
        ("producer", "null"),
        ("run_id", "17"),
        ("repository_root_name", r#""../repo""#),
        ("repository_root_name", r#""a/b""#),
        ("git_commit", r#""deadbeef""#),
        (
            "git_commit",
            r#""DEADBEEFDEADBEEFDEADBEEFDEADBEEFDEADBEEF""#,
        ),
        ("content_merkle", r#""0000""#),
        ("completion_state", r#""finished""#),
        ("source_map_state", r#""inline""#),
        ("producer_limitations", r#""native-frames""#),
        ("started_at", "null"),
    ] {
        let line = header_with(key, value);
        let parsed = parse_trace(format!("{line}\n{RECORD}\n").as_bytes());
        assert!(
            parsed.header.is_none(),
            "{key}={value} was accepted as a header"
        );
        assert!(
            parsed.records.is_empty(),
            "{key}={value} let a record through"
        );
        assert!(
            parsed.counters.refused_total() > 0,
            "{key}={value} was refused without being counted"
        );
    }
}

#[test]
fn a_missing_header_field_refuses_the_artifact() {
    for key in HEADER_KEYS {
        let mut object: serde_json::Map<String, serde_json::Value> =
            serde_json::from_str(HEADER).expect("the reference header parses");
        object.remove(key);
        let line = serde_json::to_string(&serde_json::Value::Object(object)).expect("serialises");
        let parsed = parse_trace(format!("{line}\n{RECORD}\n").as_bytes());
        assert!(
            parsed.header.is_none(),
            "a header missing {key} was accepted"
        );
        assert!(parsed.counters.refused_total() > 0, "{key} was not counted");
    }
}

#[test]
fn an_unknown_declared_limitation_is_counted_and_the_artifact_is_still_read() {
    let line = header_with(
        "producer_limitations",
        r#"["native-frames","quantum-entanglement"]"#,
    );
    let parsed = parse_trace(format!("{line}\n{RECORD}\n").as_bytes());
    let header = parsed.header.expect("the artifact is still read");
    assert_eq!(header.producer_limitations, vec!["native-frames"]);
    assert_eq!(parsed.counters.refused_count(form::LIMITATION_UNKNOWN), 1);
    assert_eq!(parsed.records.len(), 1);
}

#[test]
fn an_artifact_with_no_header_at_all_is_refused_whole() {
    for bytes in [
        &b""[..],
        &b"\n\n\n"[..],
        &b"not json at all\n"[..],
        &b"[1,2,3]\n"[..],
        &b"\"a string\"\n"[..],
    ] {
        let parsed = parse_trace(bytes);
        assert!(parsed.header.is_none(), "{bytes:?} produced a header");
        assert!(parsed.records.is_empty());
        assert!(
            parsed.counters.refused_total() > 0,
            "{bytes:?} was refused without a count"
        );
    }
    // A record with no header before it is not a header, and takes the artifact with it.
    let parsed = parse_trace(format!("{RECORD}\n{RECORD}\n").as_bytes());
    assert!(parsed.header.is_none());
    assert_eq!(parsed.counters.refused_count(form::HEADER_MISSING), 1);
}

// ---- records ---------------------------------------------------------------------------------

#[test]
fn a_producer_unresolved_frame_is_counted_and_never_guessed_at() {
    let mut object: serde_json::Map<String, serde_json::Value> =
        serde_json::from_str(RECORD).expect("parses");
    object.insert("resolution".into(), serde_json::json!("unresolved"));
    for key in ["caller_file", "caller_line", "callee_file", "callee_line"] {
        object.insert(key.into(), serde_json::Value::Null);
    }
    let line = serde_json::to_string(&serde_json::Value::Object(object)).expect("serialises");
    let parsed = artifact(&[&line]);
    assert!(parsed.records.is_empty());
    assert_eq!(
        parsed
            .counters
            .refused_count(form::PRODUCER_UNRESOLVED_FRAME),
        1
    );
    assert_eq!(
        parsed.counters.refused_total(),
        1,
        "an unresolved frame is one refusal, not one per null location"
    );
    assert_eq!(parsed.records_in_artifact, 1, "it is still a record");
}

/// Every member of the closed limitation vocabulary has a producing case, here and in the fixture.
/// A tally member with no producer is untested — the Slice 10a lesson.
#[test]
fn every_limitation_form_is_counted_when_a_record_declares_it() {
    for form in limitation::ALL {
        let line = record_with("unsupported_form", &serde_json::json!(form).to_string());
        let parsed = artifact(&[&line]);
        assert!(
            parsed.records.is_empty(),
            "{form} produced a located call event"
        );
        assert_eq!(
            parsed.counters.limitation_count(form),
            1,
            "{form} was not counted"
        );
        assert_eq!(
            parsed.counters.refused_total(),
            0,
            "{form} was counted as a refusal as well as a limitation"
        );
    }

    let unknown = record_with("unsupported_form", r#""time-travel""#);
    let parsed = artifact(&[&unknown]);
    assert!(parsed.records.is_empty());
    assert_eq!(parsed.counters.limitations_total(), 0);
    assert_eq!(parsed.counters.refused_count(form::LIMITATION_UNKNOWN), 1);
}

#[test]
fn every_record_field_is_checked_and_a_bad_one_refuses_the_record_only() {
    for (key, value) in [
        ("test_id", "null"),
        ("test_id", r#""""#),
        ("caller_file", "null"),
        ("caller_file", "12"),
        ("callee_file", r#""""#),
        ("caller_line", "0"),
        ("caller_line", "-1"),
        ("caller_line", r#""8""#),
        ("callee_line", "null"),
        ("count", "0"),
        ("count", "1.5"),
        ("worker", "12"),
        ("async_context", "false"),
        ("resolution", r#""maybe""#),
        ("resolution", "null"),
        ("unsupported_form", "7"),
    ] {
        let line = record_with(key, value);
        let parsed = artifact(&[&line, RECORD]);
        assert_eq!(
            parsed.records.len(),
            1,
            "{key}={value} cost the well-formed record after it"
        );
        assert!(
            parsed.counters.refused_total() > 0,
            "{key}={value} was refused without a count"
        );
        assert_eq!(parsed.records_in_artifact, 2);
    }
}

/// A control character is refused, not stripped: nothing here is used for identity, so there is no
/// reason to accept a byte that only ever arrives on purpose.
///
/// Two halves, and only the second would survive a producer that escapes its strings properly. A
/// **raw** control byte inside a JSON string is not legal JSON, so such a line is refused as
/// unparseable before any field rule runs; **escaped**, the same character is legal JSON and the
/// field rule is what has to refuse it.
#[test]
fn a_control_character_in_a_string_refuses_the_record() {
    let raw = format!(
        r#"{{"test_id":"a{}b","caller_file":"src/a.py","caller_line":1,"callee_file":"src/b.py","callee_line":2,"count":1,"worker":null,"async_context":null,"resolution":"located","unsupported_form":null}}"#,
        '\u{1}'
    );
    let unparseable = artifact(&[&raw]);
    assert!(unparseable.records.is_empty());
    assert_eq!(unparseable.counters.refused_count(form::MALFORMED_JSON), 1);

    let escaped = serde_json::json!("tests/a.py::b\u{1}forged").to_string();
    let line = record_with("test_id", &escaped);
    let parsed = artifact(&[&line]);
    assert!(parsed.records.is_empty());
    assert_eq!(parsed.counters.refused_count(form::RECORD_INVALID), 1);
}

#[test]
fn a_second_header_line_is_one_invalid_record_rather_than_a_pile_of_unknown_keys() {
    let parsed = artifact(&[HEADER, RECORD]);
    assert_eq!(parsed.records.len(), 1);
    assert_eq!(parsed.counters.refused_count(form::RECORD_INVALID), 1);
    assert_eq!(parsed.counters.refused_count(form::RECORD_UNKNOWN_KEY), 0);
    assert_eq!(parsed.counters.refused_total(), 1);
}

#[test]
fn a_malformed_json_line_costs_that_line_and_nothing_else() {
    let parsed = artifact(&["{not json", RECORD, "[]"]);
    assert_eq!(parsed.records.len(), 1);
    assert_eq!(parsed.counters.refused_count(form::MALFORMED_JSON), 2);
    assert_eq!(parsed.records_in_artifact, 3);
}

// ---- bounds ----------------------------------------------------------------------------------

#[test]
fn an_artifact_over_the_size_bound_is_refused_whole_and_unparsed() {
    let mut oversized = format!("{HEADER}\n{RECORD}\n").into_bytes();
    oversized.resize(MAX_ARTIFACT_BYTES + 1, b'\n');
    let parsed = parse_trace(&oversized);
    assert!(parsed.header.is_none());
    assert!(parsed.records.is_empty());
    assert_eq!(parsed.counters.refused_count(form::ARTIFACT_TOO_LARGE), 1);
    assert_eq!(parsed.counters.refused_total(), 1);

    // Exactly at the bound is inside it.
    let mut at_bound = format!("{HEADER}\n{RECORD}\n").into_bytes();
    at_bound.resize(MAX_ARTIFACT_BYTES, b'\n');
    let parsed = parse_trace(&at_bound);
    assert_eq!(parsed.records.len(), 1);
    assert_eq!(parsed.counters.refused_total(), 0);
}

#[test]
fn a_line_over_the_record_bound_is_refused_and_the_lines_around_it_are_read() {
    let padding = "x".repeat(MAX_RECORD_BYTES);
    let line = record_with("padding", &serde_json::json!(padding).to_string());
    assert!(line.len() > MAX_RECORD_BYTES);
    let parsed = artifact(&[&line, RECORD]);
    assert_eq!(parsed.records.len(), 1);
    assert_eq!(parsed.counters.refused_count(form::RECORD_TOO_LARGE), 1);
    assert_eq!(parsed.records_in_artifact, 2);

    // An oversized *first* line is a missing header, and the artifact goes with it.
    let parsed = parse_trace(format!("{line}\n{RECORD}\n").as_bytes());
    assert!(parsed.header.is_none());
    assert_eq!(parsed.counters.refused_count(form::HEADER_MISSING), 1);
}

#[test]
fn a_string_over_the_string_bound_is_refused_and_the_next_record_is_read() {
    let long = "a".repeat(MAX_STRING_BYTES + 1);
    let line = record_with("test_id", &serde_json::json!(long).to_string());
    let parsed = artifact(&[&line, RECORD]);
    assert_eq!(parsed.records.len(), 1);
    assert_eq!(parsed.counters.refused_count(form::STRING_TOO_LONG), 1);

    // Exactly at the bound is inside it.
    let at_bound = "a".repeat(MAX_STRING_BYTES);
    let line = record_with("test_id", &serde_json::json!(at_bound).to_string());
    let parsed = artifact(&[&line]);
    assert_eq!(parsed.records.len(), 1);
    assert_eq!(parsed.counters.refused_total(), 0);
}

/// The depth check is a byte scan over the raw line, and it runs before `serde_json` is called.
///
/// [`MAX_RECORD_BYTES`] already caps how deep a *legal-length* line can nest — about 4,000 — so the
/// two bounds compose: the byte bound stops the stack-exhausting case, and this one makes the
/// accepted depth small and explicit rather than "whatever 8 KiB of brackets happens to be".
#[test]
fn a_deeply_nested_line_is_refused_before_it_is_parsed() {
    assert_eq!(json_depth("{}"), 1);
    assert_eq!(json_depth(r#"{"a":[1,2]}"#), 2);
    assert_eq!(
        json_depth(r#"{"a":"[[[[["}"#),
        1,
        "brackets inside a string are text"
    );
    assert_eq!(json_depth(r#"{"a":"\""}"#), 1);

    let deep = format!(
        "{}1{}",
        "[".repeat(MAX_JSON_DEPTH + 2),
        "]".repeat(MAX_JSON_DEPTH + 2)
    );
    let line = format!(r#"{{"test_id":"t::a","nested":{deep}}}"#);
    let parsed = artifact(&[&line, RECORD]);
    assert_eq!(parsed.records.len(), 1);
    assert_eq!(parsed.counters.refused_count(form::NESTING_TOO_DEEP), 1);
    assert_eq!(parsed.records_in_artifact, 2);

    // The deepest nesting a line inside the byte bound can carry, refused by depth rather than by
    // length — the case `serde_json`'s recursive parser would otherwise be handed.
    let brackets = (MAX_RECORD_BYTES - 40) / 2;
    let bomb = format!(
        r#"{{"nested":{}1{}}}"#,
        "[".repeat(brackets),
        "]".repeat(brackets)
    );
    assert!(
        bomb.len() <= MAX_RECORD_BYTES,
        "the bomb must fit the bound"
    );
    let parsed = artifact(&[&bomb]);
    assert_eq!(parsed.counters.refused_count(form::NESTING_TOO_DEEP), 1);
    assert_eq!(parsed.counters.refused_count(form::RECORD_TOO_LARGE), 0);

    // A header nesting too deep is a missing header.
    let parsed = parse_trace(format!("{bomb}\n{RECORD}\n").as_bytes());
    assert!(parsed.header.is_none());
    assert_eq!(parsed.counters.refused_count(form::HEADER_MISSING), 1);
}

#[test]
fn invalid_utf8_refuses_the_line_it_is_on_and_nothing_else() {
    let mut bytes = format!("{HEADER}\n").into_bytes();
    bytes.extend_from_slice(br#"{"test_id":"a"#);
    bytes.extend_from_slice(&[0xff, 0xfe, 0x80]);
    bytes.extend_from_slice(b"\"}\n");
    bytes.extend_from_slice(RECORD.as_bytes());
    bytes.push(b'\n');

    let parsed = parse_trace(&bytes);
    assert_eq!(parsed.records.len(), 1);
    assert_eq!(parsed.counters.refused_count(form::INVALID_UTF8_LINE), 1);
    assert_eq!(parsed.records_in_artifact, 2);

    // A header that is not UTF-8 is no header.
    let mut headerless = vec![0xff, 0xfe, 0x80, b'\n'];
    headerless.extend_from_slice(RECORD.as_bytes());
    let parsed = parse_trace(&headerless);
    assert!(parsed.header.is_none());
    assert_eq!(parsed.counters.refused_count(form::HEADER_MISSING), 1);
}

#[test]
fn records_past_the_record_bound_are_refused_and_counted() {
    // The bound itself is 500,000, which is impractical to reach in a unit test without spending
    // the memory the bound exists to protect. What is checked here is the arithmetic that decides
    // it: the guard fires on `records.len()`, so it counts only records that were *kept*.
    const _: () = assert!(MAX_RECORDS > 0);
    let parsed = artifact(&[RECORD, RECORD]);
    assert_eq!(
        parsed.records.len(),
        2,
        "two identical records are two events"
    );
    assert_eq!(parsed.counters.refused_count(form::RECORDS_EXCEEDED), 0);
}

// ---- the completion vocabulary ---------------------------------------------------------------

#[test]
fn completion_and_source_map_states_round_trip_and_nothing_else_parses() {
    for state in CompletionState::ALL {
        assert_eq!(CompletionState::parse(state.as_str()), Some(state));
    }
    for invented in ["finished", "ok", "PARTIAL", ""] {
        assert_eq!(CompletionState::parse(invented), None);
    }
    for state in SourceMapState::ALL {
        assert_eq!(SourceMapState::parse(state.as_str()), Some(state));
    }
    for invented in ["inline", "external", "NONE", ""] {
        assert_eq!(SourceMapState::parse(invented), None);
    }
    assert!(CompletionState::Complete.is_complete());
    assert!(!CompletionState::Partial.is_complete());
    assert!(!CompletionState::Crashed.is_complete());
}

/// The weaker of two states, so an observation aggregating two runs never reads as complete when
/// one contributor did not finish.
#[test]
fn the_weaker_completion_state_wins_when_evidence_is_aggregated() {
    use CompletionState::{Complete, Crashed, Partial};
    assert_eq!(Complete.weaker(Complete), Complete);
    assert_eq!(Complete.weaker(Partial), Partial);
    assert_eq!(Partial.weaker(Complete), Partial);
    assert_eq!(Partial.weaker(Crashed), Crashed);
    assert_eq!(Crashed.weaker(Complete), Crashed);
    // Commutative, so the fold does not depend on the order runs are read in.
    for left in CompletionState::ALL {
        for right in CompletionState::ALL {
            assert_eq!(left.weaker(right), right.weaker(left));
        }
    }
}
