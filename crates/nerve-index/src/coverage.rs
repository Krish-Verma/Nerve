//! The `coverage` extractor's LCOV reader: bytes in, a structured result out.
//!
//! This module is the whole of Slice 6a. It **parses**, and does nothing else. It does not open a
//! file, spawn a process, run a test, resolve a path, touch the store, or construct a single
//! graph record — those belong to Slice 6b, behind the same path guard everything else in this
//! crate uses. The signature is the proof: [`parse_lcov`] takes `&[u8]` and returns a
//! [`CoverageReport`], so there is nowhere for a syscall to hide.
//!
//! # What LCOV can and cannot say
//!
//! `docs/decisions/ADR-0008-coverage-evidence.md` records the empirical finding that gates this
//! slice: an LCOV report is **aggregate**. Its `TN:` — the format's own *test name* field — is
//! emitted empty by the runtimes that produce these reports, there is one record set per source
//! file for a whole run, and concatenating one report per test does not recover the attribution
//! because every record's `TN:` is still blank. So:
//!
//! - The source endpoint of a coverage edge is a [`nerve_core::vocab::EntityKind::CoverageRun`],
//!   never a test. It is impossible to state "test X covers symbol Y" because no such endpoint
//!   exists to state it with.
//! - Directness is [`Directness::Inferred`]: a line hit is not a statement that a symbol is
//!   covered. A mapping step — line to enclosing symbol — concludes it.
//!
//! This module still counts a non-empty `TN:` under [`form::TEST_NAME_PRESENT`], so that if a
//! producer ever does populate it the finding is contradicted by a number rather than by memory.
//!
//! # `DA:<line>,0` is evidence
//!
//! A hit count of zero says the line was instrumented and **not** executed. That is the gap
//! question's raw material and it is preserved, not filtered out. [`LineHit::is_covered`] is the
//! only place the distinction is collapsed, and it is a query rather than a discard.
//!
//! # Paths are carried exactly as written
//!
//! An `SF:` value is repository content, and repository content is attacker-controlled
//! (THREAT-MODEL.md, A1 and T9). This module does not normalise, resolve, canonicalise or clean
//! it — not even to strip a control byte — for the same reason [`crate::markdown`] carries
//! control bytes through to the guard: a refusal the guard never sees is a refusal nobody
//! reports. Slice 6b routes every path through [`crate::discover::canonical_child`].
//!
//! # Bounds and malformed input
//!
//! Every bound refuses and counts; nothing is silently truncated, and no input produces a panic.
//! The counter vocabulary is closed ([`form::ALL`]) so a reader can enumerate every way this
//! parser can decline to believe a report.

use std::collections::btree_map::Entry;
use std::collections::BTreeMap;

use nerve_core::vocab::{Directness, EvidenceSourceType};

/// Extractor identity, recorded on every observation and on its `extractor_run` row.
pub const EXTRACTOR_ID: &str = "coverage";

/// Extractor version. A change here re-states every coverage claim, by design.
pub const EXTRACTOR_VERSION: &str = "1.0.0";

/// The only evidence source type this extractor may emit (ADR-0003, ADR-0005, ADR-0008).
///
/// `TEST_COVERAGE` has existed at ordinal 5 since Slice 1 and has been emitted by nothing. It is
/// kept permanently distinct from `TEST_CALL_TRACE` and `RUNTIME_CALL_TRACE`: coverage records
/// *that* code ran, never *who invoked it*.
pub const DECLARED_SOURCE_TYPES: [EvidenceSourceType; 1] = [EvidenceSourceType::TestCoverage];

/// How directly a coverage report states that a symbol is covered.
///
/// `Inferred`, and this is not a formality. The report states that line `n` of a file executed.
/// Concluding that a *symbol* is covered requires mapping that line onto a symbol's extent — a
/// rule concluding something the artifact did not say, which is exactly ADR-0003's definition of
/// `INFERRED`. Recording it as `DIRECT` would repeat the defect Slice 5d-i corrected.
pub const DIRECTNESS: Directness = Directness::Inferred;

// ---- resource bounds -------------------------------------------------------------------------
//
// Three bounds, all of them refusing and counting rather than truncating. A silently truncated
// report is the worst possible outcome here: missing `DA:` lines read as "these lines were never
// executed", which is a false negative claim in a store that only holds positive evidence.

/// Bytes of an LCOV report this parser is willing to read.
///
/// The whole report is held in memory while it is parsed, so this is also the parser's memory
/// ceiling. A `DA:` line costs roughly 13 bytes, so 32 MiB is on the order of 2.5 million
/// instrumented lines — far past any single project Nerve indexes, and small enough that
/// ingesting a report never becomes the largest allocation in the process. A report over the
/// bound is refused **whole**: parsing its prefix would mean reporting partial coverage as if it
/// were complete.
pub const MAX_REPORT_BYTES: usize = 32 * 1024 * 1024;

/// Records — that is, source files — a single report may contribute.
///
/// Each accepted record becomes an entity and a set of edges in Slice 6b, so this bounds the
/// graph a single attacker-controlled file can create. A repository with more than 100,000
/// instrumented source files is beyond anything Nerve indexes as one project.
pub const MAX_RECORDS: usize = 100_000;

/// Distinct instrumented lines a single record may contribute.
///
/// Derived rather than guessed: [`crate::config::DEFAULT_MAX_FILE_BYTES`] is 2 MiB and a line
/// with any content at all costs at least two bytes including its terminator, so a file Nerve
/// will read has at most ~1,048,576 lines. A record claiming more is describing a file that could
/// not have been indexed, so the remainder is refused and counted rather than stored.
pub const MAX_LINES_PER_RECORD: usize = 1_000_000;

/// Form tags used in [`CoverageCounters::refused`]. Closed, so a reader can enumerate them.
pub mod form {
    /// The report is larger than [`super::MAX_REPORT_BYTES`]. Nothing in it was parsed.
    pub const REPORT_TOO_LARGE: &str = "report-too-large";
    /// A record past [`super::MAX_RECORDS`]. Counted once per refused record.
    pub const RECORDS_EXCEEDED: &str = "records-exceeded";
    /// A `DA:` line past [`super::MAX_LINES_PER_RECORD`]. Counted once per refused line.
    pub const LINES_EXCEEDED: &str = "lines-exceeded";
    /// A line that is not valid UTF-8. Refused on its own; the rest of the report is still read.
    pub const INVALID_UTF8_LINE: &str = "invalid-utf8-line";
    /// The report ended with a record that no `end_of_record` closed. The record is dropped.
    ///
    /// Dropped rather than kept, because `end_of_record` is the format's own statement that the
    /// record is complete. Without it there is no way to tell a finished record from a truncated
    /// one, and a truncated `DA:` list reads as "those lines never executed".
    pub const UNTERMINATED_RECORD: &str = "unterminated-record";
    /// A record closed without an `SF:` naming a source file. There is nothing to attribute to.
    pub const RECORD_WITHOUT_SOURCE_FILE: &str = "record-without-source-file";
    /// An `SF:` with an empty value. Refused; the record then has no source file either.
    pub const SOURCE_FILE_PATH_EMPTY: &str = "source-file-path-empty";
    /// A second `SF:` inside one record. The first is kept and the second refused.
    pub const REPEATED_SOURCE_FILE_IN_RECORD: &str = "repeated-source-file-in-record";
    /// A second record naming a path an earlier record already named. The two are merged.
    ///
    /// Expected rather than exotic: concatenating per-test reports produces exactly this.
    pub const DUPLICATE_SOURCE_FILE: &str = "duplicate-source-file";
    /// A second `DA:` for a line the same record already stated. The two are merged.
    pub const DUPLICATE_LINE: &str = "duplicate-line";
    /// A line number that is not a 1-based line number — non-numeric, negative, zero, or past
    /// `u64`. Lines are 1-based everywhere in Nerve, so `DA:0,…` names no line and is not
    /// coerced into naming the first one.
    pub const LINE_NUMBER_UNPARSED: &str = "line-number-unparsed";
    /// A hit count that is not a non-negative integer — negative, non-numeric, or past `u64`.
    pub const HIT_COUNT_UNPARSED: &str = "hit-count-unparsed";
    /// A known record type whose fields are not the shape the format defines.
    pub const MALFORMED_RECORD: &str = "malformed-record";
    /// A line whose prefix is not a record type this parser knows.
    pub const UNKNOWN_RECORD: &str = "unknown-record";
    /// A `TN:` with a non-empty value.
    ///
    /// Counted, never used. ADR-0008 rests on the finding that this field is emitted empty; if a
    /// producer populates it, the finding is contradicted by a number rather than by memory.
    pub const TEST_NAME_PRESENT: &str = "test-name-present";

    /// Every tag in this module, in declaration order.
    pub const ALL: [&str; 15] = [
        REPORT_TOO_LARGE,
        RECORDS_EXCEEDED,
        LINES_EXCEEDED,
        INVALID_UTF8_LINE,
        UNTERMINATED_RECORD,
        RECORD_WITHOUT_SOURCE_FILE,
        SOURCE_FILE_PATH_EMPTY,
        REPEATED_SOURCE_FILE_IN_RECORD,
        DUPLICATE_SOURCE_FILE,
        DUPLICATE_LINE,
        LINE_NUMBER_UNPARSED,
        HIT_COUNT_UNPARSED,
        MALFORMED_RECORD,
        UNKNOWN_RECORD,
        TEST_NAME_PRESENT,
    ];
}

/// What the parse refused, and how often.
///
/// The same shape as [`crate::markdown::ScanCounters`], deliberately: one counting convention for
/// every reader of attacker-controlled repository content.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CoverageCounters {
    /// Refusals by form tag. Every key is from [`form`].
    pub refused: BTreeMap<String, usize>,
}

impl CoverageCounters {
    fn count(&mut self, tag: &'static str) {
        *self.refused.entry(tag.to_string()).or_insert(0) += 1;
    }

    /// How many times `tag` was counted.
    pub fn get(&self, tag: &str) -> usize {
        self.refused.get(tag).copied().unwrap_or(0)
    }

    /// Total refusals across every form.
    pub fn total(&self) -> usize {
        self.refused.values().sum()
    }
}

/// One instrumented line and the number of times the run executed it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct LineHit {
    /// 1-based line number, exactly as the report stated it.
    pub line: u64,
    /// Executions the run recorded. **Zero is a value, not an absence.**
    pub hits: u64,
}

impl LineHit {
    /// Whether the run executed this line at least once.
    ///
    /// The only place the hit/not-hit distinction is collapsed, and it is a question rather than
    /// a discard: `hits == 0` survives in [`LineHit::hits`] for callers that need it.
    pub fn is_covered(self) -> bool {
        self.hits > 0
    }
}

/// Everything one report said about one source file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileCoverage {
    /// The `SF:` value **exactly as written**. Not normalised, not resolved, not canonicalised.
    ///
    /// Repository content; inert here. Slice 6b puts it through the path guard.
    pub raw_path: String,
    /// Instrumented lines, ascending by line number, one entry per distinct line.
    pub lines: Vec<LineHit>,
}

impl FileCoverage {
    /// How many of this file's instrumented lines the run executed at least once.
    pub fn covered_count(&self) -> usize {
        self.lines.iter().filter(|hit| hit.is_covered()).count()
    }
}

/// The result of parsing one LCOV report.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CoverageReport {
    /// One entry per distinct `SF:` value, ascending by that value.
    ///
    /// Ordering is by the raw path so that the same bytes always produce the same result: Slice
    /// 6b's full-versus-incremental equivalence check needs the parse to be a pure function.
    pub files: Vec<FileCoverage>,
    /// What the parse refused.
    pub counters: CoverageCounters,
}

/// One record under construction, between an `SF:` and its `end_of_record`.
#[derive(Default)]
struct Record {
    path: Option<String>,
    lines: BTreeMap<u64, u64>,
    /// Any non-blank line seen since the last `end_of_record`. Distinguishes "the report ended
    /// on a record boundary" from "the report was cut off mid-record".
    dirty: bool,
}

/// Split on `\n`, dropping one trailing `\r` so that CRLF and LF reports parse identically.
///
/// A CR anywhere else in the line is kept: it is content, and content is carried through to the
/// guard rather than cleaned up here.
fn lines(bytes: &[u8]) -> impl Iterator<Item = &[u8]> {
    bytes
        .split(|byte| *byte == b'\n')
        .map(|line| match line.split_last() {
            Some((&b'\r', head)) => head,
            _ => line,
        })
}

/// Parse a 1-based line number. Zero, negative, non-numeric and past-`u64` are all refusals.
fn line_number(text: &str) -> Option<u64> {
    text.parse::<u64>().ok().filter(|value| *value >= 1)
}

/// Parse an execution count. Negative, non-numeric and past-`u64` are all refusals.
fn hit_count(text: &str) -> Option<u64> {
    text.parse::<u64>().ok()
}

/// Parse an LCOV report.
///
/// **Total**: every input produces a [`CoverageReport`]. There is no error return and no panic
/// path — a report this parser cannot believe comes back with fewer files and more counters, and
/// a report it cannot read at all comes back empty with the reason counted.
///
/// **Pure**: the only input is `bytes` and the only output is the returned value. Nothing here
/// reads the filesystem, spawns a process or opens a socket.
pub fn parse_lcov(bytes: &[u8]) -> CoverageReport {
    let mut counters = CoverageCounters::default();

    if bytes.len() > MAX_REPORT_BYTES {
        counters.count(form::REPORT_TOO_LARGE);
        return CoverageReport {
            files: Vec::new(),
            counters,
        };
    }

    let mut merged: BTreeMap<String, BTreeMap<u64, u64>> = BTreeMap::new();
    let mut accepted_records = 0usize;
    let mut record = Record::default();

    for raw in lines(bytes) {
        if raw.is_empty() {
            // A blank line carries no claim. Every report ends with one, so counting them would
            // report a refusal on every well-formed input.
            continue;
        }
        let Ok(line) = std::str::from_utf8(raw) else {
            counters.count(form::INVALID_UTF8_LINE);
            record.dirty = true;
            continue;
        };
        record.dirty = true;

        if line == "end_of_record" {
            close_record(
                &mut record,
                &mut merged,
                &mut accepted_records,
                &mut counters,
            );
            continue;
        }

        if let Some(value) = line.strip_prefix("TN:") {
            // The field ADR-0008 turns on. Empty is the expected and only observed value.
            if !value.is_empty() {
                counters.count(form::TEST_NAME_PRESENT);
            }
        } else if let Some(value) = line.strip_prefix("SF:") {
            if value.is_empty() {
                counters.count(form::SOURCE_FILE_PATH_EMPTY);
            } else if record.path.is_some() {
                counters.count(form::REPEATED_SOURCE_FILE_IN_RECORD);
            } else {
                // Verbatim. No trimming, no normalising, no control-byte cleanup.
                record.path = Some(value.to_string());
            }
        } else if let Some(value) = line.strip_prefix("DA:") {
            read_da(value, &mut record, &mut counters);
        } else if let Some(value) = line.strip_prefix("FNDA:") {
            // `<count>,<name>`. A function name may itself contain commas, so the split is at
            // the first one only. Recognised and validated; not carried — Slice 6a yields line
            // coverage, and function coverage would be a second data model nothing consumes yet.
            match value.split_once(',') {
                Some((count, name)) if hit_count(count).is_some() && !name.is_empty() => {}
                _ => counters.count(form::MALFORMED_RECORD),
            }
        } else if let Some(value) = line.strip_prefix("FN:") {
            // `<line>,<name>`. Some producers write `<start>,<end>,<name>`; the leading field is
            // a line number in both, and nothing past the first comma is interpreted.
            match value.split_once(',') {
                Some((start, rest)) if line_number(start).is_some() && !rest.is_empty() => {}
                _ => counters.count(form::MALFORMED_RECORD),
            }
        } else if let Some(value) = line.strip_prefix("BRDA:") {
            read_brda(value, &mut counters);
        } else if let Some(value) = line
            .strip_prefix("FNF:")
            .or_else(|| line.strip_prefix("FNH:"))
            .or_else(|| line.strip_prefix("BRF:"))
            .or_else(|| line.strip_prefix("BRH:"))
            .or_else(|| line.strip_prefix("LH:"))
            .or_else(|| line.strip_prefix("LF:"))
        {
            // Summary totals. Recognised and shape-checked, and deliberately not used as a
            // cross-check against the `DA:` lines: a disagreement between a summary and the data
            // it summarises has no correct resolution at parse time, and picking one would mean
            // believing a number over the evidence or the reverse, without grounds for either.
            if hit_count(value).is_none() {
                counters.count(form::MALFORMED_RECORD);
            }
        } else {
            counters.count(form::UNKNOWN_RECORD);
        }
    }

    if record.dirty {
        counters.count(form::UNTERMINATED_RECORD);
    }

    let files = merged
        .into_iter()
        .map(|(raw_path, hits_by_line)| FileCoverage {
            raw_path,
            lines: hits_by_line
                .into_iter()
                .map(|(line, hits)| LineHit { line, hits })
                .collect(),
        })
        .collect();

    CoverageReport { files, counters }
}

/// Read one `DA:<line>,<count>[,<checksum>]`.
fn read_da(value: &str, record: &mut Record, counters: &mut CoverageCounters) {
    let mut fields = value.splitn(3, ',');
    let (Some(raw_line), Some(raw_hits)) = (fields.next(), fields.next()) else {
        // No comma at all: the line was cut off, or was never a `DA:` record.
        counters.count(form::MALFORMED_RECORD);
        return;
    };
    let Some(line) = line_number(raw_line) else {
        counters.count(form::LINE_NUMBER_UNPARSED);
        return;
    };
    let Some(hits) = hit_count(raw_hits) else {
        counters.count(form::HIT_COUNT_UNPARSED);
        return;
    };

    // Read before the entry borrows the map. A line the record already states costs no new
    // storage, so the bound applies to new lines only.
    let at_capacity = record.lines.len() >= MAX_LINES_PER_RECORD;
    match record.lines.entry(line) {
        Entry::Occupied(mut existing) => {
            counters.count(form::DUPLICATE_LINE);
            let slot = existing.get_mut();
            *slot = (*slot).max(hits);
        }
        Entry::Vacant(slot) => {
            if at_capacity {
                counters.count(form::LINES_EXCEEDED);
                return;
            }
            slot.insert(hits);
        }
    }
}

/// Read one `BRDA:<line>,<block>,<branch>,<taken>`.
///
/// Branch data is recognised and shape-checked, and not modelled. Branch coverage answers "did
/// both arms run", which is a different question from "was this symbol entered", and inventing a
/// mapping from it would be a claim this slice has no evidence for.
fn read_brda(value: &str, counters: &mut CoverageCounters) {
    let fields: Vec<&str> = value.split(',').collect();
    let well_formed = fields.len() == 4
        && line_number(fields[0]).is_some()
        && hit_count(fields[1]).is_some()
        && hit_count(fields[2]).is_some()
        // `-` means the branch was never reached, which is the format's own way of saying so.
        && (fields[3] == "-" || hit_count(fields[3]).is_some());
    if !well_formed {
        counters.count(form::MALFORMED_RECORD);
    }
}

/// Close a record at its `end_of_record` and merge it into the report.
fn close_record(
    record: &mut Record,
    merged: &mut BTreeMap<String, BTreeMap<u64, u64>>,
    accepted: &mut usize,
    counters: &mut CoverageCounters,
) {
    let finished = std::mem::take(record);
    let Some(path) = finished.path else {
        counters.count(form::RECORD_WITHOUT_SOURCE_FILE);
        return;
    };
    if *accepted >= MAX_RECORDS {
        counters.count(form::RECORDS_EXCEEDED);
        return;
    }
    *accepted += 1;

    match merged.entry(path) {
        Entry::Occupied(mut existing) => {
            counters.count(form::DUPLICATE_SOURCE_FILE);
            let target = existing.get_mut();
            for (line, hits) in finished.lines {
                let slot = target.entry(line).or_insert(0);
                // The maximum, not the sum. LCOV carries no run identity — `TN:` is empty — so
                // two records for one path may be two runs or one run recorded twice, and there
                // is no way to tell. The maximum is the weakest claim consistent with both: it
                // never invents an execution that no record stated.
                *slot = (*slot).max(hits);
            }
        }
        Entry::Vacant(slot) => {
            slot.insert(finished.lines);
        }
    }
}

#[cfg(test)]
mod tests {
    use std::fmt::Write as _;

    use super::*;

    impl CoverageReport {
        /// The `(line, hits)` pairs recorded for `raw_path`, for readable assertions.
        fn get_lines(&self, raw_path: &str) -> Vec<(u64, u64)> {
            self.files
                .iter()
                .find(|file| file.raw_path == raw_path)
                .map(|file| file.lines.iter().map(|hit| (hit.line, hit.hits)).collect())
                .unwrap_or_default()
        }
    }

    /// The parse of a well-formed report, as a `(path, [(line, hits)])` list.
    fn files_of(report: &CoverageReport) -> Vec<(String, Vec<(u64, u64)>)> {
        report
            .files
            .iter()
            .map(|file| {
                (
                    file.raw_path.clone(),
                    file.lines
                        .iter()
                        .map(|hit| (hit.line, hit.hits))
                        .collect::<Vec<_>>(),
                )
            })
            .collect()
    }

    // ---- extractor identity ------------------------------------------------------------------

    #[test]
    fn declares_only_the_test_coverage_source_type_and_infers_nothing_more() {
        assert_eq!(EXTRACTOR_ID, "coverage");
        assert_eq!(EXTRACTOR_VERSION, "1.0.0");
        assert_eq!(DECLARED_SOURCE_TYPES.len(), 1);
        assert_eq!(DECLARED_SOURCE_TYPES[0], EvidenceSourceType::TestCoverage);
        assert_eq!(DECLARED_SOURCE_TYPES[0].as_str(), "TEST_COVERAGE");
        // A line hit is not a statement that a symbol is covered; a mapping step concludes it.
        assert_eq!(DIRECTNESS, Directness::Inferred);
        assert_eq!(DIRECTNESS.as_str(), "INFERRED");
    }

    #[test]
    fn every_form_tag_is_distinct() {
        let mut sorted = form::ALL.to_vec();
        sorted.sort_unstable();
        sorted.dedup();
        assert_eq!(sorted.len(), form::ALL.len(), "two form tags collide");
        assert!(form::ALL.iter().all(|tag| !tag.is_empty()));
    }

    /// The parse is a pure function of its bytes, which is what makes Slice 6b's
    /// full-versus-incremental equivalence checkable at all.
    #[test]
    fn parsing_the_same_bytes_twice_gives_the_same_answer() {
        let report = b"TN:\nSF:src/a.js\nDA:1,1\nDA:2,0\nend_of_record\n";
        assert_eq!(parse_lcov(report), parse_lcov(report));
    }

    // ---- one test per record type ------------------------------------------------------------

    /// `TN:` is the field ADR-0008 turns on. Empty is the observed value and produces nothing.
    #[test]
    fn tn_is_read_and_an_empty_test_name_is_not_a_refusal() {
        let report = parse_lcov(b"TN:\nSF:src/a.js\nDA:1,1\nend_of_record\n");
        assert_eq!(report.counters.total(), 0);
        assert_eq!(report.get_lines("src/a.js"), vec![(1, 1)]);
    }

    /// A populated `TN:` is counted, so the finding this slice rests on can be contradicted by a
    /// measurement rather than defended from memory.
    #[test]
    fn tn_with_a_value_is_counted_because_it_would_contradict_adr_0008() {
        let report = parse_lcov(b"TN:alpha test\nSF:src/a.js\nDA:1,1\nend_of_record\n");
        assert_eq!(report.counters.get(form::TEST_NAME_PRESENT), 1);
        // Counted, never used: the record is still attributed to the run, not to `alpha test`.
        assert_eq!(report.get_lines("src/a.js"), vec![(1, 1)]);
    }

    /// The path is carried byte for byte. Normalisation is Slice 6b's job, behind the guard.
    #[test]
    fn sf_carries_the_path_exactly_as_written() {
        for written in [
            "src/a.js",
            "./src/a.js",
            "/abs/olute/src/a.js",
            "../../../../etc/passwd",
            "src\\windows\\a.js",
            "src/a b.js",
            "src/../src/a.js",
        ] {
            let report = parse_lcov(format!("SF:{written}\nDA:1,1\nend_of_record\n").as_bytes());
            assert_eq!(
                report.files.len(),
                1,
                "{written} did not produce exactly one record"
            );
            assert_eq!(
                report.files[0].raw_path, written,
                "{written} was rewritten by the parser"
            );
            assert_eq!(report.counters.total(), 0, "{written} was refused");
        }
    }

    #[test]
    fn fn_is_recognised_and_a_malformed_one_is_counted() {
        let good = parse_lcov(b"SF:src/a.js\nFN:12,alphaCovered\nDA:12,1\nend_of_record\n");
        assert_eq!(good.counters.total(), 0);
        assert_eq!(good.get_lines("src/a.js"), vec![(12, 1)]);

        // A name containing a comma is a name, not a third field.
        let comma = parse_lcov(b"SF:src/a.js\nFN:12,f<a,b>\nend_of_record\n");
        assert_eq!(comma.counters.total(), 0);

        for malformed in ["FN:alphaCovered", "FN:,name", "FN:12,", "FN:-1,name"] {
            let report =
                parse_lcov(format!("SF:src/a.js\n{malformed}\nend_of_record\n").as_bytes());
            assert_eq!(
                report.counters.get(form::MALFORMED_RECORD),
                1,
                "{malformed} was not counted"
            );
        }
    }

    #[test]
    fn fnda_is_recognised_and_a_malformed_one_is_counted() {
        let good = parse_lcov(b"SF:src/a.js\nFNDA:3,alphaCovered\nDA:1,1\nend_of_record\n");
        assert_eq!(good.counters.total(), 0);
        // A function executed three times says nothing about which lines ran; only `DA:` does.
        assert_eq!(good.get_lines("src/a.js"), vec![(1, 1)]);

        // Zero executions is a legitimate value: the function was instrumented and never called.
        let zero = parse_lcov(b"SF:src/a.js\nFNDA:0,neverCalled\nend_of_record\n");
        assert_eq!(zero.counters.total(), 0);

        for malformed in [
            "FNDA:alphaCovered",
            "FNDA:-1,name",
            "FNDA:3,",
            "FNDA:x,name",
        ] {
            let report =
                parse_lcov(format!("SF:src/a.js\n{malformed}\nend_of_record\n").as_bytes());
            assert_eq!(
                report.counters.get(form::MALFORMED_RECORD),
                1,
                "{malformed} was not counted"
            );
        }
    }

    #[test]
    fn fnf_is_recognised_and_a_malformed_one_is_counted() {
        let good = parse_lcov(b"SF:src/a.js\nFNF:2\nDA:1,1\nend_of_record\n");
        assert_eq!(good.counters.total(), 0);
        assert_eq!(good.get_lines("src/a.js"), vec![(1, 1)]);

        let bad = parse_lcov(b"SF:src/a.js\nFNF:two\nend_of_record\n");
        assert_eq!(bad.counters.get(form::MALFORMED_RECORD), 1);
    }

    #[test]
    fn fnh_is_recognised_and_a_malformed_one_is_counted() {
        let good = parse_lcov(b"SF:src/a.js\nFNF:2\nFNH:1\nDA:1,1\nend_of_record\n");
        assert_eq!(good.counters.total(), 0);

        let bad = parse_lcov(b"SF:src/a.js\nFNH:-1\nend_of_record\n");
        assert_eq!(bad.counters.get(form::MALFORMED_RECORD), 1);
    }

    #[test]
    fn brda_is_recognised_and_never_modelled() {
        let good = parse_lcov(b"SF:src/a.js\nBRDA:5,0,0,3\nBRDA:5,0,1,-\nDA:5,3\nend_of_record\n");
        assert_eq!(good.counters.total(), 0);
        // Branch data contributes no lines: `DA:` is the only source of a line hit.
        assert_eq!(good.get_lines("src/a.js"), vec![(5, 3)]);

        for malformed in [
            "BRDA:5,0,0",
            "BRDA:5,0,0,3,9",
            "BRDA:x,0,0,3",
            "BRDA:5,0,0,x",
        ] {
            let report =
                parse_lcov(format!("SF:src/a.js\n{malformed}\nend_of_record\n").as_bytes());
            assert_eq!(
                report.counters.get(form::MALFORMED_RECORD),
                1,
                "{malformed} was not counted"
            );
        }
    }

    #[test]
    fn brf_is_recognised_and_a_malformed_one_is_counted() {
        let good = parse_lcov(b"SF:src/a.js\nBRF:4\nDA:1,1\nend_of_record\n");
        assert_eq!(good.counters.total(), 0);

        let bad = parse_lcov(b"SF:src/a.js\nBRF:\nend_of_record\n");
        assert_eq!(bad.counters.get(form::MALFORMED_RECORD), 1);
    }

    #[test]
    fn brh_is_recognised_and_a_malformed_one_is_counted() {
        let good = parse_lcov(b"SF:src/a.js\nBRF:4\nBRH:2\nDA:1,1\nend_of_record\n");
        assert_eq!(good.counters.total(), 0);

        let bad = parse_lcov(b"SF:src/a.js\nBRH:two\nend_of_record\n");
        assert_eq!(bad.counters.get(form::MALFORMED_RECORD), 1);
    }

    /// The record the whole slice is about — and the zero that must survive it.
    #[test]
    fn da_records_both_hit_and_explicitly_unhit_lines() {
        let report =
            parse_lcov(b"TN:\nSF:src/alpha.js\nDA:1,1\nDA:2,4\nDA:6,0\nDA:7,0\nend_of_record\n");
        assert_eq!(report.counters.total(), 0);
        assert_eq!(
            report.get_lines("src/alpha.js"),
            vec![(1, 1), (2, 4), (6, 0), (7, 0)],
            "DA:<line>,0 is evidence of an uncovered line and must survive the parse"
        );

        let file = &report.files[0];
        assert_eq!(file.covered_count(), 2);
        assert!(file.lines[0].is_covered());
        assert!(!file.lines[2].is_covered());
        assert_eq!(file.lines[2].hits, 0);
    }

    /// The optional third field is the line checksum. Recognised, ignored, never a refusal.
    #[test]
    fn da_accepts_the_optional_checksum_field() {
        let report = parse_lcov(b"SF:src/a.js\nDA:1,1,cfd7fcd5b3e1c0d9\nend_of_record\n");
        assert_eq!(report.counters.total(), 0);
        assert_eq!(report.get_lines("src/a.js"), vec![(1, 1)]);
    }

    #[test]
    fn lh_is_recognised_and_a_malformed_one_is_counted() {
        let good = parse_lcov(b"SF:src/a.js\nDA:1,1\nDA:2,0\nLH:1\nLF:2\nend_of_record\n");
        assert_eq!(good.counters.total(), 0);
        assert_eq!(good.get_lines("src/a.js"), vec![(1, 1), (2, 0)]);

        // A summary that disagrees with the data is not resolved either way; only its shape is
        // checked, and a well-formed disagreement is not a refusal.
        let disagrees = parse_lcov(b"SF:src/a.js\nDA:1,1\nLH:99\nend_of_record\n");
        assert_eq!(disagrees.counters.total(), 0);
        assert_eq!(disagrees.get_lines("src/a.js"), vec![(1, 1)]);

        let bad = parse_lcov(b"SF:src/a.js\nLH:one\nend_of_record\n");
        assert_eq!(bad.counters.get(form::MALFORMED_RECORD), 1);
    }

    #[test]
    fn lf_is_recognised_and_a_malformed_one_is_counted() {
        let good = parse_lcov(b"SF:src/a.js\nDA:1,1\nLF:1\nend_of_record\n");
        assert_eq!(good.counters.total(), 0);

        let bad = parse_lcov(b"SF:src/a.js\nLF:-2\nend_of_record\n");
        assert_eq!(bad.counters.get(form::MALFORMED_RECORD), 1);
    }

    /// `end_of_record` is the format's statement that a record is complete, and the only thing
    /// that commits one. Two records in one report are two source files.
    #[test]
    fn end_of_record_closes_a_record_and_starts_the_next() {
        let report = parse_lcov(
            b"TN:\nSF:src/alpha.js\nDA:1,1\nend_of_record\n\
              TN:\nSF:src/beta.js\nDA:1,0\nend_of_record\n",
        );
        assert_eq!(report.counters.total(), 0);
        assert_eq!(
            files_of(&report),
            vec![
                ("src/alpha.js".to_string(), vec![(1, 1)]),
                ("src/beta.js".to_string(), vec![(1, 0)]),
            ]
        );
    }

    /// A bare `end_of_record` closes a record that named no source file.
    #[test]
    fn a_lone_end_of_record_is_a_record_without_a_source_file() {
        let report = parse_lcov(b"end_of_record\n");
        assert!(report.files.is_empty());
        assert_eq!(report.counters.get(form::RECORD_WITHOUT_SOURCE_FILE), 1);
    }

    /// An unknown record type is counted, never guessed at.
    #[test]
    fn an_unknown_record_type_is_counted() {
        let report = parse_lcov(b"SF:src/a.js\nVER:9\nDA:1,1\nend_of_record\n");
        assert_eq!(report.counters.get(form::UNKNOWN_RECORD), 1);
        assert_eq!(report.get_lines("src/a.js"), vec![(1, 1)]);
    }

    // ---- malformed input never panics --------------------------------------------------------

    /// Cut off mid-`DA:`. The partial line is refused and the record it was in is dropped.
    #[test]
    fn a_report_truncated_mid_record_is_refused_rather_than_half_believed() {
        let report = parse_lcov(b"TN:\nSF:src/a.js\nDA:1,1\nDA:2");
        assert!(
            report.files.is_empty(),
            "a truncated record must not contribute coverage"
        );
        assert_eq!(report.counters.get(form::MALFORMED_RECORD), 1);
        assert_eq!(report.counters.get(form::UNTERMINATED_RECORD), 1);
    }

    /// Every line well-formed, but the report stops before `end_of_record`.
    ///
    /// Dropped, not kept: without the terminator there is no way to tell a finished record from a
    /// truncated one, and a truncated `DA:` list reads as "those lines never executed".
    #[test]
    fn a_record_with_no_end_of_record_at_eof_is_dropped_and_counted() {
        let report = parse_lcov(b"TN:\nSF:src/a.js\nDA:1,1\nDA:2,0\n");
        assert!(report.files.is_empty());
        assert_eq!(report.counters.get(form::UNTERMINATED_RECORD), 1);

        // The complete record before it survives; only the unterminated tail is lost.
        let partial = parse_lcov(b"SF:src/a.js\nDA:1,1\nend_of_record\nSF:src/b.js\nDA:1,1\n");
        assert_eq!(
            files_of(&partial),
            vec![("src/a.js".to_string(), vec![(1, 1)])]
        );
        assert_eq!(partial.counters.get(form::UNTERMINATED_RECORD), 1);
    }

    #[test]
    fn a_record_with_no_source_file_is_refused_and_counted() {
        let report = parse_lcov(b"TN:\nDA:1,1\nDA:2,0\nLH:1\nLF:2\nend_of_record\n");
        assert!(report.files.is_empty());
        assert_eq!(report.counters.get(form::RECORD_WITHOUT_SOURCE_FILE), 1);
    }

    #[test]
    fn a_second_source_file_inside_one_record_is_refused_and_the_first_kept() {
        let report = parse_lcov(b"SF:src/a.js\nSF:src/b.js\nDA:1,1\nend_of_record\n");
        assert_eq!(report.counters.get(form::REPEATED_SOURCE_FILE_IN_RECORD), 1);
        assert_eq!(
            files_of(&report),
            vec![("src/a.js".to_string(), vec![(1, 1)])]
        );
    }

    /// A line number past `u64`, and every other thing that is not a 1-based line number.
    #[test]
    fn absurd_line_numbers_are_refused_and_counted() {
        let report = parse_lcov(
            b"SF:src/a.js\n\
              DA:999999999999999999999,1\n\
              DA:18446744073709551616,1\n\
              DA:-1,1\n\
              DA:0,1\n\
              DA:1.5,1\n\
              DA:1,1\n\
              end_of_record\n",
        );
        assert_eq!(
            report.counters.get(form::LINE_NUMBER_UNPARSED),
            5,
            "each absurd line number must be counted, not clamped"
        );
        assert_eq!(
            report.get_lines("src/a.js"),
            vec![(1, 1)],
            "only the one real line survives"
        );

        // The largest line number a `u64` holds is a number, and is not refused for being large:
        // the bound this parser enforces is on how many lines a record may state, not on how big
        // a line number may be. Mapping it to a symbol is Slice 6b's problem and it will fail
        // there, visibly, against a real file's extents.
        let biggest = parse_lcov(b"SF:src/a.js\nDA:18446744073709551615,1\nend_of_record\n");
        assert_eq!(biggest.counters.total(), 0);
        assert_eq!(
            biggest.get_lines("src/a.js"),
            vec![(18_446_744_073_709_551_615, 1)]
        );
    }

    #[test]
    fn negative_and_non_numeric_hit_counts_are_refused_and_counted() {
        let report = parse_lcov(
            b"SF:src/a.js\n\
              DA:1,-1\n\
              DA:2,many\n\
              DA:3,\n\
              DA:4,1e3\n\
              DA:5,18446744073709551616\n\
              DA:6,0\n\
              end_of_record\n",
        );
        assert_eq!(report.counters.get(form::HIT_COUNT_UNPARSED), 5);
        assert_eq!(
            report.get_lines("src/a.js"),
            vec![(6, 0)],
            "a refused count must not be coerced to zero — that would invent an uncovered line"
        );
    }

    /// Concatenating one report per test produces exactly this shape (ADR-0008 §A.1, probe 3).
    #[test]
    fn duplicate_source_file_records_merge_and_are_counted() {
        let report = parse_lcov(
            b"TN:\nSF:src/a.js\nDA:1,1\nDA:2,0\nend_of_record\n\
              TN:\nSF:src/a.js\nDA:2,3\nDA:9,0\nend_of_record\n",
        );
        assert_eq!(report.counters.get(form::DUPLICATE_SOURCE_FILE), 1);
        assert_eq!(
            files_of(&report),
            vec![(
                "src/a.js".to_string(),
                // The maximum, never the sum: with `TN:` empty there is no way to tell two runs
                // from one run recorded twice, and the maximum invents no executions.
                vec![(1, 1), (2, 3), (9, 0)]
            )]
        );
    }

    #[test]
    fn duplicate_da_lines_for_one_line_merge_and_are_counted() {
        let report = parse_lcov(b"SF:src/a.js\nDA:1,0\nDA:1,7\nDA:1,2\nend_of_record\n");
        assert_eq!(report.counters.get(form::DUPLICATE_LINE), 2);
        assert_eq!(
            report.get_lines("src/a.js"),
            vec![(1, 7)],
            "a later zero must not erase an execution an earlier record stated"
        );
    }

    #[test]
    fn crlf_and_lf_line_endings_parse_identically_even_when_mixed() {
        let lf = parse_lcov(b"TN:\nSF:src/a.js\nDA:1,1\nDA:2,0\nend_of_record\n");
        let crlf = parse_lcov(b"TN:\r\nSF:src/a.js\r\nDA:1,1\r\nDA:2,0\r\nend_of_record\r\n");
        let mixed = parse_lcov(b"TN:\r\nSF:src/a.js\nDA:1,1\r\nDA:2,0\nend_of_record\r\n");
        assert_eq!(lf, crlf);
        assert_eq!(lf, mixed);
        assert_eq!(lf.counters.total(), 0);
        assert_eq!(lf.get_lines("src/a.js"), vec![(1, 1), (2, 0)]);
    }

    #[test]
    fn invalid_utf8_refuses_the_line_it_is_on_and_nothing_else() {
        let mut report = Vec::new();
        report.extend_from_slice(b"SF:src/a.js\nDA:1,1\nSF:");
        report.extend_from_slice(&[0xff, 0xfe, 0x80]);
        report.extend_from_slice(b"\nDA:2,0\nend_of_record\n");

        let parsed = parse_lcov(&report);
        assert_eq!(parsed.counters.get(form::INVALID_UTF8_LINE), 1);
        assert_eq!(
            parsed.get_lines("src/a.js"),
            vec![(1, 1), (2, 0)],
            "one unreadable line must not cost the rest of the record"
        );

        // A record whose only `SF:` is unreadable names no source file at all.
        let mut orphan = Vec::new();
        orphan.extend_from_slice(b"SF:");
        orphan.extend_from_slice(&[0xff]);
        orphan.extend_from_slice(b"\nDA:1,1\nend_of_record\n");
        let parsed = parse_lcov(&orphan);
        assert!(parsed.files.is_empty());
        assert_eq!(parsed.counters.get(form::INVALID_UTF8_LINE), 1);
        assert_eq!(parsed.counters.get(form::RECORD_WITHOUT_SOURCE_FILE), 1);
    }

    #[test]
    fn an_empty_report_is_empty_rather_than_an_error() {
        let report = parse_lcov(b"");
        assert!(report.files.is_empty());
        assert_eq!(report.counters.total(), 0);
        assert_eq!(report, CoverageReport::default());
    }

    #[test]
    fn a_report_of_only_newlines_is_empty_rather_than_an_error() {
        for bytes in [
            &b"\n"[..],
            &b"\n\n\n\n"[..],
            &b"\r\n\r\n"[..],
            &b"\n\r\n\n"[..],
        ] {
            let report = parse_lcov(bytes);
            assert!(report.files.is_empty());
            assert_eq!(
                report.counters.total(),
                0,
                "a blank line carries no claim and is not a refusal"
            );
        }
    }

    #[test]
    fn an_empty_source_file_path_is_refused_and_counted() {
        let report = parse_lcov(b"TN:\nSF:\nDA:1,1\nend_of_record\n");
        assert!(report.files.is_empty());
        assert_eq!(report.counters.get(form::SOURCE_FILE_PATH_EMPTY), 1);
        // Having refused the only `SF:`, the record names no source file either. Both are true
        // and both are stated.
        assert_eq!(report.counters.get(form::RECORD_WITHOUT_SOURCE_FILE), 1);
    }

    // ---- resource bounds, each refusing and counting ------------------------------------------

    #[test]
    fn a_report_over_the_size_bound_is_refused_whole_and_counted() {
        let mut oversized = b"SF:src/a.js\nDA:1,1\nend_of_record\n".to_vec();
        oversized.resize(MAX_REPORT_BYTES + 1, b'\n');
        let report = parse_lcov(&oversized);
        assert!(
            report.files.is_empty(),
            "an oversized report must not be parsed as far as the bound and then stopped"
        );
        assert_eq!(report.counters.get(form::REPORT_TOO_LARGE), 1);
        assert_eq!(report.counters.total(), 1);

        // Exactly at the bound is inside it.
        let mut at_bound = b"SF:src/a.js\nDA:1,1\nend_of_record\n".to_vec();
        at_bound.resize(MAX_REPORT_BYTES, b'\n');
        let report = parse_lcov(&at_bound);
        assert_eq!(report.get_lines("src/a.js"), vec![(1, 1)]);
        assert_eq!(report.counters.total(), 0);
    }

    #[test]
    fn records_past_the_record_bound_are_refused_and_counted() {
        let mut source = String::new();
        for index in 0..MAX_RECORDS + 3 {
            writeln!(source, "SF:src/f{index}.js\nDA:1,1\nend_of_record").expect("a String write");
        }
        let report = parse_lcov(source.as_bytes());
        assert_eq!(report.files.len(), MAX_RECORDS);
        assert_eq!(report.counters.get(form::RECORDS_EXCEEDED), 3);
        assert_eq!(report.counters.total(), 3);
    }

    #[test]
    fn lines_past_the_per_record_bound_are_refused_and_counted() {
        let mut source = String::from("SF:src/a.js\n");
        for line in 1..=MAX_LINES_PER_RECORD as u64 + 2 {
            writeln!(source, "DA:{line},1").expect("a String write");
        }
        source.push_str("end_of_record\n");

        let report = parse_lcov(source.as_bytes());
        assert_eq!(report.files.len(), 1);
        assert_eq!(report.files[0].lines.len(), MAX_LINES_PER_RECORD);
        assert_eq!(report.counters.get(form::LINES_EXCEEDED), 2);
        assert_eq!(report.counters.total(), 2);
        // Refused, not truncated silently: the last line kept is the bound-th one stated.
        assert_eq!(
            report.files[0].lines[MAX_LINES_PER_RECORD - 1].line,
            MAX_LINES_PER_RECORD as u64
        );
    }

    // ---- a whole report, end to end ----------------------------------------------------------

    /// The exact shape ADR-0008 §A.1 recorded from `node --test --experimental-test-coverage
    /// --test-reporter=lcov`: an empty `TN:`, one record set per source file, no test dimension.
    #[test]
    fn the_reference_report_from_adr_0008_parses_to_what_it_says() {
        let report = parse_lcov(
            b"TN:\n\
              SF:src/alpha.js\n\
              FN:1,alphaCovered\n\
              FNDA:1,alphaCovered\n\
              FNF:2\n\
              FNH:1\n\
              BRDA:3,0,0,1\n\
              BRF:1\n\
              BRH:1\n\
              DA:1,1\n\
              DA:2,1\n\
              DA:3,1\n\
              DA:6,0\n\
              DA:7,0\n\
              LH:3\n\
              LF:5\n\
              end_of_record\n\
              TN:\n\
              SF:src/beta.js\n\
              DA:1,0\n\
              LH:0\n\
              LF:1\n\
              end_of_record\n",
        );
        assert_eq!(report.counters.total(), 0);
        assert_eq!(
            files_of(&report),
            vec![
                (
                    "src/alpha.js".to_string(),
                    vec![(1, 1), (2, 1), (3, 1), (6, 0), (7, 0)]
                ),
                ("src/beta.js".to_string(), vec![(1, 0)]),
            ]
        );
        assert_eq!(report.files[0].covered_count(), 3);
        assert_eq!(report.files[1].covered_count(), 0);
        // Nothing in the parse names a test, because nothing in the report does.
        assert_eq!(report.counters.get(form::TEST_NAME_PRESENT), 0);
    }
}
