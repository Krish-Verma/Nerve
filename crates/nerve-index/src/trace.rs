//! The `test-trace` extractor's artifact reader: bytes in, a structured result out.
//!
//! This module **parses**, and does nothing else. It does not open a file, spawn a process, run a
//! test, resolve a path, touch the store or construct a graph record — those belong to
//! [`crate::trace_ingest`]. The signature is the proof: [`parse_trace`] takes `&[u8]` and returns a
//! [`TraceArtifact`], so there is nowhere for a syscall to hide. It is the same split
//! [`crate::coverage`] and [`crate::coverage_ingest`] use, for the same reason.
//!
//! # Nerve does not run your tests
//!
//! `crates/nerve-cli/tests/no_subprocess.rs` forbids process creation anywhere under
//! `crates/*/src/**`, and its own module documentation names *"no test runners"* as the thing it
//! exists to refuse. So `nerve trace-tests` does not exist and is not deferred:
//! `docs/plans/slice-11-test-observed-calls.md` §1 records the decision and
//! `docs/plans/slice-11a-trace-ingestion.md` §1 reinforces it. The user runs their own tests under
//! their own tracer; Nerve reads the artifact.
//!
//! # What the artifact can and cannot say
//!
//! A record names a **file and a line** on each side of one call. It carries no symbol *name*, on
//! purpose: a name would invite resolving by name, which this project forbids and which
//! `docs/plans/slice-11a-trace-ingestion.md` §6 rules out explicitly. It also carries no argument
//! value, return value, local, exception value, source text or per-call timing — a trace that
//! captured arguments would capture credentials, and there is no redaction scheme that is safe by
//! construction.
//!
//! So the endpoints of the assertion are the two frames of the call, never the test. For a stack
//! `test_x → parse → lex` a tracer records two call events; naming `test_x` as the source of the
//! second would assert a call `test_x` never made. Which test observed an edge is **provenance** and
//! belongs on the observation (ADR-0003), which is where [`crate::trace_ingest`] puts it.
//!
//! # `count` is frequency, never importance
//!
//! [`TraceRecord::count`] is how many times the producer saw this edge under this test. Nothing
//! ranks by it, sorts by it, or thresholds on it. It is reported because it is a measurement.
//!
//! # The asymmetry on unknown fields
//!
//! **Unknown keys in the header reject the artifact; unknown keys in a record are ignored and
//! counted.** Deliberate: a header key Nerve does not understand may change the meaning of the whole
//! file — imagine `"paths_are_absolute": true` — whereas an unknown record key is at worst one extra
//! datum about one edge. The count is reported, so a systematically-ignored field is visible rather
//! than silent.
//!
//! # Bounds, and why the depth check precedes the parse
//!
//! Every bound refuses and counts; nothing is silently truncated, and no input produces a panic.
//! [`MAX_JSON_DEPTH`] is measured on the **raw bytes**, before `serde_json` is handed the line, and
//! that ordering is the point: `serde_json`'s parser is recursive, so a deeply nested line could
//! exhaust the stack and abort the process before any check inside the deserialiser could fire. A
//! refusal that runs after the crash is not a refusal.
//!
//! The counter vocabularies are closed — [`form::ALL`] for what Nerve refused and
//! [`limitation::ALL`] for what the producer said it could not see — so a reader can enumerate every
//! way this parser declines to believe an artifact, and every way a producer admits to a gap.

use std::collections::BTreeMap;

use nerve_core::vocab::{Directness, EvidenceSourceType, Relation};

/// Extractor identity, recorded on every observation and on its `extractor_run` row.
pub const EXTRACTOR_ID: &str = "test-trace";

/// Extractor version. A change here re-states every trace claim, by design.
pub const EXTRACTOR_VERSION: &str = "1.0.0";

/// The artifact format this reader understands, and the only value `format` may hold.
pub const FORMAT: &str = "nerve-trace";

/// The artifact format version this reader understands.
pub const FORMAT_VERSION: u64 = 1;

/// The only evidence source type this extractor may emit.
///
/// `TEST_CALL_TRACE` has existed at ordinal 6 since Slice 1 and has been emitted by nothing;
/// Slice 11a is its first emitter, exactly as Slice 10a was the first emitter of `FRAMEWORK_RULE`.
/// No member is added on that axis and no `source_type_mask` ordinal moves.
///
/// It is kept permanently distinct from `TEST_COVERAGE` and `RUNTIME_CALL_TRACE`: coverage records
/// *that* code ran and never who invoked it (ADR-0005, ADR-0008), and a production trace is a
/// different environment from a test run.
pub const DECLARED_SOURCE_TYPES: [EvidenceSourceType; 1] = [EvidenceSourceType::TestCallTrace];

/// The only relation this extractor may emit.
pub const DECLARED_RELATIONS: [Relation; 1] = [Relation::TestObservedCall];

/// How directly a trace artifact states that one symbol called another.
///
/// **`Resolved`.** `docs/plans/slice-11a-trace-ingestion.md` §2.3 settles this, and the difference
/// from coverage is real rather than a formality:
///
/// - `Direct` means *"the artifact literally states it"* ([`Directness::Direct`]). The artifact does
///   not state `nerve_index::pystruct::extract_module`; it states a file and a line. Recording
///   `DIRECT` would repeat the defect Slice 5d-i corrected.
/// - `Inferred` means *a rule concluded the relation*, which is what [`crate::coverage`] does: a
///   line hit does not say "covered", and a mapping step concludes it. **A trace infers no relation
///   at all** — the call is stated outright. Only the *endpoints* need resolving.
///
/// `Resolved` is *"derived through a resolution step"*, which is exactly that, and is the value
/// `AST_RESOLVED` uses when import resolution names a target.
pub const DIRECTNESS: Directness = Directness::Resolved;

// ---- resource bounds -------------------------------------------------------------------------
//
// Every bound refuses and counts rather than truncating, for the reason the coverage reader gives:
// a silently truncated artifact reads as "these calls never happened", which is a false negative
// claim in a store that only holds positive evidence.

/// Bytes of an artifact this parser is willing to read.
///
/// The same ceiling as [`crate::coverage::MAX_REPORT_BYTES`] and for the same reason: the whole
/// artifact is held in memory while it is parsed, so this is also the parser's memory ceiling, and
/// ingesting one must never become the largest allocation in the process. A record costs roughly
/// 250 bytes, so 32 MiB is on the order of 130,000 observed edges. An artifact over the bound is
/// refused **whole** — parsing its prefix would report part of a run as if it were all of it, which
/// is exactly the lie [`CompletionState::Partial`] exists to avoid telling by accident.
pub const MAX_ARTIFACT_BYTES: usize = 32 * 1024 * 1024;

/// Bytes one NDJSON line may occupy.
///
/// Derived rather than guessed: a record has ten fields, each string field is bounded by
/// [`MAX_STRING_BYTES`], so a well-formed record cannot exceed about 5.5 KiB. 8 KiB leaves room for
/// a producer's unknown keys while keeping one line from being a memory problem on its own. A line
/// over the bound is refused; the lines around it are still read, because NDJSON's whole point is
/// that one broken line is not a broken file.
pub const MAX_RECORD_BYTES: usize = 8 * 1024;

/// Records a single artifact may contribute.
///
/// [`MAX_ARTIFACT_BYTES`] binds first in practice — 500,000 records at the minimum plausible size is
/// well past 32 MiB — so this is a second, independent ceiling rather than the operative one. It
/// exists so that a pathologically small record shape cannot make the record count unbounded while
/// the byte count stays legal.
pub const MAX_RECORDS: usize = 500_000;

/// Bytes any single string field may occupy, in the header and in a record alike.
///
/// A `pytest` node id — `tests/test_x.py::TestClass::test_y[param-1-2]` — is the longest string the
/// contract carries in practice, and 512 bytes is generous for one. The bound matters because these
/// strings are stored: an unbounded `test_id` would let an artifact write an arbitrary amount of
/// text into `observation.environment` for every edge it names.
pub const MAX_STRING_BYTES: usize = 512;

/// JSON nesting depth a line may reach.
///
/// A header is one object whose deepest value is an array of strings (depth 2); a record is a flat
/// object (depth 1). 8 leaves room for a producer to nest an unknown key without permitting a
/// nesting bomb. Measured on the raw bytes **before** `serde_json` sees the line — see the module
/// header for why the ordering is the security property rather than a detail.
pub const MAX_JSON_DEPTH: usize = 8;

/// Form tags for what this parser refused. Closed, so a reader can enumerate them.
pub mod form {
    /// The artifact is larger than [`super::MAX_ARTIFACT_BYTES`]. Nothing in it was parsed.
    pub const ARTIFACT_TOO_LARGE: &str = "artifact-too-large";
    /// A line longer than [`super::MAX_RECORD_BYTES`]. That line only.
    pub const RECORD_TOO_LARGE: &str = "record-too-large";
    /// A record past [`super::MAX_RECORDS`]. Counted once per refused record.
    pub const RECORDS_EXCEEDED: &str = "records-exceeded";
    /// A line that is not valid UTF-8. Refused on its own; the rest of the artifact is still read.
    pub const INVALID_UTF8_LINE: &str = "invalid-utf8-line";
    /// A line that is not a JSON object.
    pub const MALFORMED_JSON: &str = "malformed-json";
    /// A line nesting past [`super::MAX_JSON_DEPTH`], measured before the line is parsed.
    pub const NESTING_TOO_DEEP: &str = "nesting-too-deep";
    /// A string field longer than [`super::MAX_STRING_BYTES`].
    pub const STRING_TOO_LONG: &str = "string-too-long";
    /// The artifact has no first line, or its first line is not a usable header.
    ///
    /// Without a header there is no run identity, no repository binding and no completion state, so
    /// there is nothing to attribute a record to. The artifact is refused whole.
    pub const HEADER_MISSING: &str = "header-missing";
    /// A top-level header key this reader does not know. The artifact is refused **whole**.
    ///
    /// The asymmetry against [`RECORD_UNKNOWN_KEY`] is the point: an unknown header key may change
    /// how the whole file must be read.
    pub const HEADER_UNKNOWN_KEY: &str = "header-unknown-key";
    /// A header field missing, of the wrong type, or outside its stated vocabulary or range.
    ///
    /// Includes the one contradiction the contract names explicitly: `completion_state: "complete"`
    /// with a null `completed_at`. A producer that claims completion must say when.
    pub const HEADER_INVALID: &str = "header-invalid";
    /// A record field missing, of the wrong type, or outside its vocabulary or range.
    pub const RECORD_INVALID: &str = "record-invalid";
    /// A key in a record this reader does not know. **Ignored and counted; the record is kept.**
    pub const RECORD_UNKNOWN_KEY: &str = "record-unknown-key";
    /// A record whose `resolution` is `unresolved`: the producer could not locate the frame.
    ///
    /// Counted, never guessed at. A frame the producer could not place is not evidence of a call
    /// between two symbols, and inventing one from the surrounding records would be exactly the
    /// fuzzy attribution ADR-0002's tuples exist to prevent.
    pub const PRODUCER_UNRESOLVED_FRAME: &str = "producer-unresolved-frame";
    /// A `producer_limitations` entry, or a record `unsupported_form`, outside
    /// [`super::limitation::ALL`].
    ///
    /// Counted and **not** echoed, and the artifact is still read. A limitation value Nerve cannot
    /// name is still a producer saying *something was missed*, which is what a limitation means;
    /// refusing the artifact for it would make the vocabulary brittle across producer versions
    /// while buying no correctness. Contrast [`HEADER_UNKNOWN_KEY`], where an unknown *key* changes
    /// how the file is read rather than adding one more admission of a gap.
    pub const LIMITATION_UNKNOWN: &str = "limitation-unknown";

    /// Every tag in this module, in declaration order.
    pub const ALL: [&str; 14] = [
        ARTIFACT_TOO_LARGE,
        RECORD_TOO_LARGE,
        RECORDS_EXCEEDED,
        INVALID_UTF8_LINE,
        MALFORMED_JSON,
        NESTING_TOO_DEEP,
        STRING_TOO_LONG,
        HEADER_MISSING,
        HEADER_UNKNOWN_KEY,
        HEADER_INVALID,
        RECORD_INVALID,
        RECORD_UNKNOWN_KEY,
        PRODUCER_UNRESOLVED_FRAME,
        LIMITATION_UNKNOWN,
    ];
}

/// What a producer may declare it could not observe. Closed, and counted by form.
///
/// `docs/plans/slice-11-test-observed-calls.md` §4 lists these as the limitations of tracing and
/// requires each to be **counted rather than described** — following Slice 9b's gate 7, so a
/// silently growing set of unobservable forms fails the build rather than accumulating in prose.
///
/// A form appears in two places: in a header's `producer_limitations`, where it says *this run could
/// not see this class of thing at all*, and on a record's `unsupported_form`, where it says *this
/// particular frame was one*. The second is the stronger statement and is counted per occurrence.
pub mod limitation {
    /// An `async`/`await` continuation: the frame that resumed a coroutine is not the frame that
    /// suspended it, and a tracer cannot generally recover the logical caller.
    pub const ASYNC_CONTINUATIONS: &str = "async-continuations";
    /// A frame on a thread other than the one the tracer was installed on.
    pub const THREADS: &str = "threads";
    /// A frame in a `multiprocessing` child, which has its own interpreter and its own tracer.
    pub const MULTIPROCESSING_CHILDREN: &str = "multiprocessing-children";
    /// A native or C-extension frame. It has no source file and line to name.
    pub const NATIVE_FRAMES: &str = "native-frames";
    /// A call reached through a dynamic import, where the module was named at run time.
    pub const DYNAMIC_IMPORTS: &str = "dynamic-imports";
    /// A frame in code generated at run time — `exec`, `eval`, a template compiler.
    pub const GENERATED_CODE: &str = "generated-code";
    /// A framework wrapper whose own frame stands between the caller and the callee.
    pub const FRAMEWORK_WRAPPERS: &str = "framework-wrappers";
    /// A gap left by a sampling tracer: an interval in which no event was recorded.
    pub const SAMPLING_GAP: &str = "sampling-gap";
    /// A test that raised out of the tracer's reach, so its stack was unwound unobserved.
    pub const CRASHED_TEST: &str = "crashed-test";
    /// A run that did not finish. Distinct from a crashed test: the *run* stopped, not one test.
    pub const INTERRUPTED_RUN: &str = "interrupted-run";
    /// Several tests sharing one interpreter, so which test a frame belongs to is not certain.
    pub const PARALLEL_TESTS_SHARED_PROCESS: &str = "parallel-tests-shared-process";
    /// Another profiler holding the interpreter's tracing hook, so events were lost.
    pub const PROFILER_CONTENTION: &str = "profiler-contention";

    /// Every limitation, in declaration order.
    pub const ALL: [&str; 12] = [
        ASYNC_CONTINUATIONS,
        THREADS,
        MULTIPROCESSING_CHILDREN,
        NATIVE_FRAMES,
        DYNAMIC_IMPORTS,
        GENERATED_CODE,
        FRAMEWORK_WRAPPERS,
        SAMPLING_GAP,
        CRASHED_TEST,
        INTERRUPTED_RUN,
        PARALLEL_TESTS_SHARED_PROCESS,
        PROFILER_CONTENTION,
    ];

    /// Whether `value` is a member of this closed vocabulary.
    pub fn is_known(value: &str) -> bool {
        ALL.contains(&value)
    }
}

/// Whether the traced run finished, as the producer states it.
///
/// **Not** `extractor_run.status`, which is a statement about Nerve's own file processing. Two
/// different partialities must not share one column: an interrupted test run that Nerve read from end
/// to end would otherwise report `complete`
/// (`docs/plans/slice-11a-trace-ingestion.md` §2.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum CompletionState {
    /// A run whose process died.
    Crashed,
    /// A run that stopped early, with a stated reason.
    Partial,
    /// A run that reached the end of the suite. The producer must also say when.
    Complete,
}

impl CompletionState {
    /// Every state, in declaration order — **weakest claim first**, which is what makes
    /// [`CompletionState::weaker`] a `min` over the derived `Ord`.
    pub const ALL: [CompletionState; 3] = [
        CompletionState::Crashed,
        CompletionState::Partial,
        CompletionState::Complete,
    ];

    /// Canonical name, as it appears in `environment`, in `details` and in `--json` output.
    pub fn as_str(self) -> &'static str {
        match self {
            CompletionState::Complete => "complete",
            CompletionState::Partial => "partial",
            CompletionState::Crashed => "crashed",
        }
    }

    /// Read the canonical name. Nothing outside the vocabulary parses.
    pub fn parse(text: &str) -> Option<CompletionState> {
        CompletionState::ALL
            .into_iter()
            .find(|state| state.as_str() == text)
    }

    /// Whether the run reached the end of the suite.
    pub fn is_complete(self) -> bool {
        self == CompletionState::Complete
    }

    /// The weaker of two states, `Crashed < Partial < Complete`.
    ///
    /// Used when one observation aggregates evidence from more than one run: if any contributor did
    /// not finish, the row must say so, because a partial trace must never read as a complete one.
    /// Each run's own state survives exactly, in `environment.runs[]`.
    pub fn weaker(self, other: CompletionState) -> CompletionState {
        self.min(other)
    }
}

/// What the producer did about source maps.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SourceMapState {
    /// No source map was involved. The recorded locations are the executed locations.
    None,
    /// A source map was applied, so the locations are of the original source.
    Applied,
    /// A source map was needed and was not available. Locations may be of generated code.
    Unavailable,
}

impl SourceMapState {
    /// Every state, in declaration order.
    pub const ALL: [SourceMapState; 3] = [
        SourceMapState::None,
        SourceMapState::Applied,
        SourceMapState::Unavailable,
    ];

    /// Canonical name.
    pub fn as_str(self) -> &'static str {
        match self {
            SourceMapState::None => "none",
            SourceMapState::Applied => "applied",
            SourceMapState::Unavailable => "unavailable",
        }
    }

    /// Read the canonical name. Nothing outside the vocabulary parses.
    pub fn parse(text: &str) -> Option<SourceMapState> {
        SourceMapState::ALL
            .into_iter()
            .find(|state| state.as_str() == text)
    }
}

/// What the parse refused, and what the producer admitted, in two closed vocabularies.
///
/// Two maps rather than one, because they say different things. A **refusal** is Nerve declining to
/// believe part of an artifact. A **limitation** is the producer saying it could not see something.
/// Folding them together would let a well-behaved producer's honesty read as Nerve rejecting its
/// output.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TraceCounters {
    /// Refusals by form tag. Every key is from [`form`].
    pub refused: BTreeMap<String, usize>,
    /// Producer-declared limitations by form tag. Every key is from [`limitation`].
    pub limitations: BTreeMap<String, usize>,
}

impl TraceCounters {
    fn refuse(&mut self, tag: &'static str) {
        *self.refused.entry(tag.to_string()).or_insert(0) += 1;
    }

    fn limit(&mut self, tag: &str) {
        *self.limitations.entry(tag.to_string()).or_insert(0) += 1;
    }

    /// How many times `tag` was refused.
    pub fn refused_count(&self, tag: &str) -> usize {
        self.refused.get(tag).copied().unwrap_or(0)
    }

    /// How many records declared `tag`.
    pub fn limitation_count(&self, tag: &str) -> usize {
        self.limitations.get(tag).copied().unwrap_or(0)
    }

    /// Total refusals across every form.
    pub fn refused_total(&self) -> usize {
        self.refused.values().sum()
    }

    /// Total declared limitations across every form.
    pub fn limitations_total(&self) -> usize {
        self.limitations.values().sum()
    }
}

/// The header, once every field has been read and checked.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TraceHeader {
    /// Producer identity, as the producer states it. Never used to decide anything.
    pub producer: String,
    /// Producer version.
    pub producer_version: String,
    /// Final path segment of the repository root the run was made in.
    pub repository_root_name: String,
    /// Git HEAD at run time, 40 lowercase hex, or `None` when the producer had none.
    pub git_commit: Option<String>,
    /// Content merkle at run time, 64 lowercase hex, or `None` when the producer had none.
    pub content_merkle: Option<String>,
    /// Producer-chosen identifier, unique per run.
    pub run_id: String,
    /// Test framework the run was driven by.
    pub test_framework: String,
    /// Language runtime.
    pub runtime: String,
    /// Runtime version.
    pub runtime_version: String,
    /// Platform the run happened on.
    pub platform: String,
    /// When the run started.
    pub started_at: String,
    /// When it finished, or `None` for a run that did not.
    pub completed_at: Option<String>,
    /// Whether the run finished.
    pub completion_state: CompletionState,
    /// Why it did not, when it did not.
    pub partial_reason: Option<String>,
    /// What the producer did about source maps.
    pub source_map_state: SourceMapState,
    /// Limitations the producer declares for the whole run. Members of [`limitation::ALL`] only;
    /// anything else was counted under [`form::LIMITATION_UNKNOWN`] and dropped.
    pub producer_limitations: Vec<String>,
}

/// One located call event: two frames, and which test was executing.
///
/// Every field is present and checked. A record whose producer could not locate a frame, or that
/// declared an unsupported form, never becomes one of these — it is counted and dropped, because a
/// frame that cannot be placed is not evidence of a call between two symbols.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct TraceRecord {
    /// The executing test, as the framework names it. Provenance only: **never resolved**, and never
    /// compared against a symbol name (`docs/plans/slice-11a-trace-ingestion.md` §6).
    pub test_id: String,
    /// Path of the calling frame, **exactly as written**. Not normalised here.
    pub caller_file: String,
    /// 1-based line of the calling frame.
    pub caller_line: u64,
    /// Path of the called frame, exactly as written.
    pub callee_file: String,
    /// 1-based line of the called frame.
    pub callee_line: u64,
    /// How many times the producer saw this edge under this test.
    ///
    /// **Evidence of frequency, never of importance.** Nothing ranks, sorts or thresholds on it.
    pub count: u64,
    /// Worker the frame ran on, as the producer names it.
    pub worker: Option<String>,
    /// Async context the frame ran in, when the producer knows one.
    pub async_context: Option<String>,
}

/// The result of parsing one artifact.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TraceArtifact {
    /// The header, or `None` when the artifact was refused whole.
    ///
    /// `None` together with a non-empty [`TraceArtifact::records`] is not a state this parser can
    /// produce: without a header there is no run to attribute a record to.
    pub header: Option<TraceHeader>,
    /// Located records, in the order the artifact stated them.
    pub records: Vec<TraceRecord>,
    /// Lines after the header, whether or not they were believed. The denominator.
    pub records_in_artifact: usize,
    /// What the parse refused and what the producer admitted.
    pub counters: TraceCounters,
}

/// Split on `\n`, dropping one trailing `\r` so CRLF and LF artifacts parse identically.
fn lines(bytes: &[u8]) -> impl Iterator<Item = &[u8]> {
    bytes
        .split(|byte| *byte == b'\n')
        .map(|line| match line.split_last() {
            Some((&b'\r', head)) => head,
            _ => line,
        })
}

/// The deepest `{`/`[` nesting in `line`, ignoring brackets inside strings.
///
/// A byte scan, not a parse, and it runs **before** `serde_json` is called. `serde_json`'s parser is
/// recursive: a line nesting thousands of arrays could exhaust the stack and abort the process, and
/// a check inside the deserialiser would never get to run. Escapes are honoured only as far as is
/// needed to know whether a quote closes a string, which is all that matters here.
fn json_depth(line: &str) -> usize {
    let mut depth = 0usize;
    let mut deepest = 0usize;
    let mut in_string = false;
    let mut escaped = false;
    for byte in line.bytes() {
        if in_string {
            if escaped {
                escaped = false;
            } else if byte == b'\\' {
                escaped = true;
            } else if byte == b'"' {
                in_string = false;
            }
            continue;
        }
        match byte {
            b'"' => in_string = true,
            b'{' | b'[' => {
                depth += 1;
                deepest = deepest.max(depth);
            }
            b'}' | b']' => depth = depth.saturating_sub(1),
            _ => {}
        }
    }
    deepest
}

/// Every key the header may carry. Anything else refuses the artifact.
const HEADER_KEYS: [&str; 18] = [
    "format",
    "format_version",
    "producer",
    "producer_version",
    "repository_root_name",
    "git_commit",
    "content_merkle",
    "run_id",
    "test_framework",
    "runtime",
    "runtime_version",
    "platform",
    "started_at",
    "completed_at",
    "completion_state",
    "partial_reason",
    "source_map_state",
    "producer_limitations",
];

/// Every key a record may carry. Anything else is ignored and counted.
const RECORD_KEYS: [&str; 10] = [
    "test_id",
    "caller_file",
    "caller_line",
    "callee_file",
    "callee_line",
    "count",
    "worker",
    "async_context",
    "resolution",
    "unsupported_form",
];

/// A string field: present, a string, non-empty, within the byte bound, and control-free.
///
/// Control characters are **refused rather than stripped**, unlike heading text
/// ([`nerve_core::ids::strip_control`]), because nothing here is used for identity and there is no
/// reason to accept a byte that only ever arrives on purpose. It is the same choice
/// [`crate::discover::canonical_child`] makes for a path: refuse at the door, and nothing downstream
/// has to be careful.
fn string_field(
    object: &serde_json::Map<String, serde_json::Value>,
    key: &str,
    counters: &mut TraceCounters,
    too_long: &'static str,
    invalid: &'static str,
) -> Option<String> {
    let Some(serde_json::Value::String(text)) = object.get(key) else {
        counters.refuse(invalid);
        return None;
    };
    if text.len() > MAX_STRING_BYTES {
        counters.refuse(too_long);
        return None;
    }
    if text.is_empty() || text.chars().any(|c| (c as u32) < 0x20) {
        counters.refuse(invalid);
        return None;
    }
    Some(text.clone())
}

/// An optional string field: `null`, or a string meeting the same rules.
fn optional_string_field(
    object: &serde_json::Map<String, serde_json::Value>,
    key: &str,
    counters: &mut TraceCounters,
    too_long: &'static str,
    invalid: &'static str,
) -> Result<Option<String>, ()> {
    match object.get(key) {
        Some(serde_json::Value::Null) => Ok(None),
        Some(serde_json::Value::String(_)) => {
            match string_field(object, key, counters, too_long, invalid) {
                Some(text) => Ok(Some(text)),
                None => Err(()),
            }
        }
        _ => {
            counters.refuse(invalid);
            Err(())
        }
    }
}

/// A lowercase-hex digest of exactly `width` characters, or `null`.
fn optional_hex_field(
    object: &serde_json::Map<String, serde_json::Value>,
    key: &str,
    width: usize,
    counters: &mut TraceCounters,
) -> Result<Option<String>, ()> {
    let text = optional_string_field(
        object,
        key,
        counters,
        form::STRING_TOO_LONG,
        form::HEADER_INVALID,
    )?;
    let Some(text) = text else { return Ok(None) };
    let well_formed = text.len() == width
        && text
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte));
    if !well_formed {
        counters.refuse(form::HEADER_INVALID);
        return Err(());
    }
    Ok(Some(text))
}

/// A 1-based line number, or a positive count. Zero, negative and non-integer are all refusals.
fn positive_u64(
    object: &serde_json::Map<String, serde_json::Value>,
    key: &str,
    counters: &mut TraceCounters,
) -> Option<u64> {
    match object.get(key).and_then(serde_json::Value::as_u64) {
        Some(value) if value >= 1 => Some(value),
        _ => {
            counters.refuse(form::RECORD_INVALID);
            None
        }
    }
}

/// Read and check the header. `None` means the artifact is refused whole, and why is counted.
fn read_header(line: &str, counters: &mut TraceCounters) -> Option<TraceHeader> {
    let Ok(serde_json::Value::Object(object)) = serde_json::from_str::<serde_json::Value>(line)
    else {
        counters.refuse(form::HEADER_MISSING);
        return None;
    };

    // The format stamp first, and a failure here is `HEADER_MISSING` rather than `HEADER_INVALID`.
    // The distinction is not pedantry: *this is not a nerve-trace header* and *this is a
    // nerve-trace header I cannot fully read* are different facts, and an artifact whose first line
    // is a record — the commonest way to hand Nerve a headerless file — must not be reported as
    // eighteen unknown header keys.
    if object.get("format").and_then(serde_json::Value::as_str) != Some(FORMAT)
        || object
            .get("format_version")
            .and_then(serde_json::Value::as_u64)
            != Some(FORMAT_VERSION)
    {
        counters.refuse(form::HEADER_MISSING);
        return None;
    }

    // Then unknown keys, which refuse the whole artifact. A key Nerve does not understand may change
    // what every record in the file means.
    let unknown = object
        .keys()
        .filter(|key| !HEADER_KEYS.contains(&key.as_str()))
        .count();
    if unknown > 0 {
        for _ in 0..unknown {
            counters.refuse(form::HEADER_UNKNOWN_KEY);
        }
        return None;
    }

    let required = |key: &str, counters: &mut TraceCounters| {
        string_field(
            &object,
            key,
            counters,
            form::STRING_TOO_LONG,
            form::HEADER_INVALID,
        )
    };
    let producer = required("producer", counters)?;
    let producer_version = required("producer_version", counters)?;
    let repository_root_name = required("repository_root_name", counters)?;
    let run_id = required("run_id", counters)?;
    let test_framework = required("test_framework", counters)?;
    let runtime = required("runtime", counters)?;
    let runtime_version = required("runtime_version", counters)?;
    let platform = required("platform", counters)?;
    let started_at = required("started_at", counters)?;

    // A root name is a single path segment, never a path. The binding check would refuse a header
    // naming `../../etc` anyway, but a segment is what the field means and saying so here keeps the
    // comparison in the binding check a comparison rather than a path decision.
    if repository_root_name.contains('/') || repository_root_name.contains('\\') {
        counters.refuse(form::HEADER_INVALID);
        return None;
    }

    let git_commit = optional_hex_field(&object, "git_commit", 40, counters).ok()?;
    let content_merkle = optional_hex_field(&object, "content_merkle", 64, counters).ok()?;
    let completed_at = optional_string_field(
        &object,
        "completed_at",
        counters,
        form::STRING_TOO_LONG,
        form::HEADER_INVALID,
    )
    .ok()?;
    let partial_reason = optional_string_field(
        &object,
        "partial_reason",
        counters,
        form::STRING_TOO_LONG,
        form::HEADER_INVALID,
    )
    .ok()?;

    let Some(completion_state) = object
        .get("completion_state")
        .and_then(serde_json::Value::as_str)
        .and_then(CompletionState::parse)
    else {
        counters.refuse(form::HEADER_INVALID);
        return None;
    };
    let Some(source_map_state) = object
        .get("source_map_state")
        .and_then(serde_json::Value::as_str)
        .and_then(SourceMapState::parse)
    else {
        counters.refuse(form::HEADER_INVALID);
        return None;
    };

    // The one contradiction the contract names: a producer that claims completion must say when.
    if completion_state.is_complete() && completed_at.is_none() {
        counters.refuse(form::HEADER_INVALID);
        return None;
    }

    let Some(serde_json::Value::Array(declared)) = object.get("producer_limitations") else {
        counters.refuse(form::HEADER_INVALID);
        return None;
    };
    let mut producer_limitations = Vec::new();
    for value in declared {
        match value.as_str() {
            Some(text) if limitation::is_known(text) => producer_limitations.push(text.to_string()),
            // Counted and dropped, never echoed. See `form::LIMITATION_UNKNOWN`.
            _ => counters.refuse(form::LIMITATION_UNKNOWN),
        }
    }
    producer_limitations.sort();
    producer_limitations.dedup();

    Some(TraceHeader {
        producer,
        producer_version,
        repository_root_name,
        git_commit,
        content_merkle,
        run_id,
        test_framework,
        runtime,
        runtime_version,
        platform,
        started_at,
        completed_at,
        completion_state,
        partial_reason,
        source_map_state,
        producer_limitations,
    })
}

/// Read one record. `None` means it contributed no located call event, and why is counted.
fn read_record(line: &str, counters: &mut TraceCounters) -> Option<TraceRecord> {
    let Ok(serde_json::Value::Object(object)) = serde_json::from_str::<serde_json::Value>(line)
    else {
        counters.refuse(form::MALFORMED_JSON);
        return None;
    };

    // An unsupported form is checked first, and is the only thing checked on such a record: the
    // producer has already said it could not model this frame, so validating locations it never
    // claimed to have would count the same admission twice.
    match object.get("unsupported_form") {
        Some(serde_json::Value::String(text)) => {
            if limitation::is_known(text) {
                counters.limit(text);
            } else {
                counters.refuse(form::LIMITATION_UNKNOWN);
            }
            return None;
        }
        Some(serde_json::Value::Null) => {}
        _ => {
            counters.refuse(form::RECORD_INVALID);
            return None;
        }
    }

    match object.get("resolution").and_then(serde_json::Value::as_str) {
        Some("located") => {}
        Some("unresolved") => {
            counters.refuse(form::PRODUCER_UNRESOLVED_FRAME);
            return None;
        }
        _ => {
            counters.refuse(form::RECORD_INVALID);
            return None;
        }
    }

    let test_id = string_field(
        &object,
        "test_id",
        counters,
        form::STRING_TOO_LONG,
        form::RECORD_INVALID,
    )?;
    let caller_file = string_field(
        &object,
        "caller_file",
        counters,
        form::STRING_TOO_LONG,
        form::RECORD_INVALID,
    )?;
    let callee_file = string_field(
        &object,
        "callee_file",
        counters,
        form::STRING_TOO_LONG,
        form::RECORD_INVALID,
    )?;
    let caller_line = positive_u64(&object, "caller_line", counters)?;
    let callee_line = positive_u64(&object, "callee_line", counters)?;
    let count = positive_u64(&object, "count", counters)?;
    let worker = optional_string_field(
        &object,
        "worker",
        counters,
        form::STRING_TOO_LONG,
        form::RECORD_INVALID,
    )
    .ok()?;
    let async_context = optional_string_field(
        &object,
        "async_context",
        counters,
        form::STRING_TOO_LONG,
        form::RECORD_INVALID,
    )
    .ok()?;

    // Unknown record keys are counted **after** the record is known to be well formed, so a second
    // header line — which is a record with eight unknown keys — is one `record-invalid` rather than
    // a pile of ignored-key counts.
    for key in object.keys() {
        if !RECORD_KEYS.contains(&key.as_str()) {
            counters.refuse(form::RECORD_UNKNOWN_KEY);
        }
    }

    Some(TraceRecord {
        test_id,
        caller_file,
        caller_line,
        callee_file,
        callee_line,
        count,
        worker,
        async_context,
    })
}

/// Parse one `nerve-trace/v1` artifact.
///
/// **Total**: every input produces a [`TraceArtifact`]. There is no error return and no panic path —
/// an artifact this parser cannot believe comes back with fewer records and more counters, and one it
/// cannot read at all comes back with no header and the reason counted.
///
/// **Pure**: the only input is `bytes` and the only output is the returned value. Nothing here reads
/// the filesystem, spawns a process or opens a socket.
pub fn parse_trace(bytes: &[u8]) -> TraceArtifact {
    let mut counters = TraceCounters::default();

    if bytes.len() > MAX_ARTIFACT_BYTES {
        counters.refuse(form::ARTIFACT_TOO_LARGE);
        return TraceArtifact {
            header: None,
            records: Vec::new(),
            records_in_artifact: 0,
            counters,
        };
    }

    let mut header: Option<TraceHeader> = None;
    let mut records = Vec::new();
    let mut records_in_artifact = 0usize;

    for raw in lines(bytes) {
        if raw.is_empty() {
            // Every artifact ends with one, so counting blank lines would report a refusal on every
            // well-formed input.
            continue;
        }
        // Each guard below refuses the line, and refuses the *artifact* when the line in question is
        // the header: a run with no identity, no binding and no completion state has nothing a
        // record could be attributed to.
        if raw.len() > MAX_RECORD_BYTES {
            // Refused before it is decoded, let alone parsed: the bound exists to keep one line from
            // being a memory problem, and decoding it first would spend exactly that memory.
            if header.is_none() {
                counters.refuse(form::HEADER_MISSING);
                break;
            }
            records_in_artifact += 1;
            counters.refuse(form::RECORD_TOO_LARGE);
            continue;
        }
        let Ok(line) = std::str::from_utf8(raw) else {
            counters.refuse(form::INVALID_UTF8_LINE);
            if header.is_none() {
                counters.refuse(form::HEADER_MISSING);
                break;
            }
            records_in_artifact += 1;
            continue;
        };
        if json_depth(line) > MAX_JSON_DEPTH {
            counters.refuse(form::NESTING_TOO_DEEP);
            if header.is_none() {
                counters.refuse(form::HEADER_MISSING);
                break;
            }
            records_in_artifact += 1;
            continue;
        }

        if header.is_none() {
            match read_header(line, &mut counters) {
                Some(read) => header = Some(read),
                None => break,
            }
            continue;
        }

        records_in_artifact += 1;
        if records.len() >= MAX_RECORDS {
            counters.refuse(form::RECORDS_EXCEEDED);
            continue;
        }
        if let Some(record) = read_record(line, &mut counters) {
            records.push(record);
        }
    }

    if header.is_none() {
        // A wholly empty artifact has no line to fail on, so the reason is counted here; every other
        // route counted it at the point the reason was found.
        if counters.refused_total() == 0 {
            counters.refuse(form::HEADER_MISSING);
        }
        records.clear();
        records_in_artifact = 0;
    }

    TraceArtifact {
        header,
        records,
        records_in_artifact,
        counters,
    }
}

#[cfg(test)]
#[path = "trace_tests.rs"]
mod tests;
