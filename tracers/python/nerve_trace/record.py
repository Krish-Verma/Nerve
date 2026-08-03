"""The `nerve-trace/v1` artifact writer, and the contract it writes to.

`docs/plans/slice-11a-trace-ingestion.md` §4 is the contract and
`crates/nerve-index/src/trace.rs` is the consumer that enforces it. Both key lists below are
copied from that consumer deliberately rather than derived: `HEADER_KEYS` and `RECORD_KEYS` are
closed there, an unknown *header* key refuses the whole artifact, and an unknown *record* key is
counted as `record-unknown-key`. A producer that emitted a field the reader has never heard of
would be a producer whose output is quietly discarded, so the lists are asserted against the
contract in `tests/test_record.py` and any drift fails there.

# Why NDJSON, and why the header is rewritten in place

A tracer streams and may be killed. NDJSON means a truncated file still has valid lines above the
break, which is what makes an honest `partial` possible — and every record line is flushed as it is
written, so a `kill -9` costs at most the test in progress.

That collides with the header being line 1: `completion_state` and `completed_at` are only known at
the *end* of the run. So the header is written twice. The first write says
`completion_state: "partial"` with a null `completed_at` and a reason that says so — a killed run
therefore reads as partial, which is true — and [`ArtifactWriter.close`] seeks back to offset 0 and
overwrites it with the final answer. To make that safe the line is padded with trailing spaces to a
fixed width, which JSON permits after the closing brace and which
`crates/nerve-index/src/trace.rs`'s `read_header` accepts because `serde_json::from_str` skips
trailing whitespace. Padding is the reason a rewrite cannot corrupt the file: the replacement is
exactly as long as what it replaces.

The alternative — buffer everything and write the header last — was rejected because a killed run
would then leave nothing at all, which is the property NDJSON was chosen for.

# The producer respects the reader's bounds

`MAX_ARTIFACT_BYTES` refuses an artifact **whole**, so a producer that sailed past it would hand
Nerve a file it will not read one line of. This writer therefore stops writing records before the
bound and reports `partial` with a reason, which is a smaller and honest artifact rather than a
large and useless one. `MAX_RECORD_BYTES`, `MAX_RECORDS` and `MAX_STRING_BYTES` are respected for
the same reason: a record Nerve will refuse is a record not worth emitting, and dropping it here
keeps the refusal counters in `nerve trace import` meaningful.
"""

from __future__ import annotations

import json
import os

#: Producer version, as it appears in the artifact header. A change re-states every claim.
VERSION = "0.1.0"

#: Producer identity. Never used by Nerve to decide anything; recorded as provenance.
PRODUCER = "nerve-trace-python"

#: The artifact format stamp. Must equal `crates/nerve-index/src/trace.rs::FORMAT`.
FORMAT = "nerve-trace"

#: The artifact format version. Must equal `crates/nerve-index/src/trace.rs::FORMAT_VERSION`.
FORMAT_VERSION = 1

#: The framework whose node ids populate `test_id`.
TEST_FRAMEWORK = "pytest"

#: Every key the header may carry, in the order the contract lists them. Anything else refuses the
#: artifact whole, so this tuple is exhaustive on purpose.
HEADER_KEYS = (
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
)

#: Every key a record may carry. Anything else is ignored and counted as `record-unknown-key`.
RECORD_KEYS = (
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
)

#: Bytes any single string field may occupy. Mirrors `trace.rs::MAX_STRING_BYTES`.
MAX_STRING_BYTES = 512

#: Bytes one NDJSON line may occupy. Mirrors `trace.rs::MAX_RECORD_BYTES`.
MAX_RECORD_BYTES = 8 * 1024

#: Records one artifact may contribute. Mirrors `trace.rs::MAX_RECORDS`.
MAX_RECORDS = 500_000

#: Bytes of artifact the reader is willing to read **at all**. Mirrors
#: `trace.rs::MAX_ARTIFACT_BYTES`. Exceeding it refuses the artifact whole, so this writer stops
#: short of it rather than crossing it.
MAX_ARTIFACT_BYTES = 32 * 1024 * 1024

#: Room left below `MAX_ARTIFACT_BYTES` so that the final header rewrite and a last flush cannot
#: push a legal artifact over the reader's ceiling.
ARTIFACT_BYTE_MARGIN = 64 * 1024

#: Spare bytes reserved after the provisional header line, so the final header — which gains a
#: `completed_at` timestamp and a longer `completion_state` — still fits in the same span.
HEADER_SLACK = 256

#: What the first header write says. A run killed before `close` keeps exactly this, and it is true.
PROVISIONAL_REASON = "the run had not finished when this header was written"

#: The three legal values of `completion_state` (`trace.rs::CompletionState`). `interrupted` is
#: **not** among them: an interrupted run is `partial` with a reason.
COMPLETION_STATES = ("complete", "partial", "crashed")

#: The three legal values of `source_map_state`. Python has no source maps, so this producer always
#: says `none`.
SOURCE_MAP_STATES = ("none", "applied", "unavailable")


def is_clean_string(value, limit=MAX_STRING_BYTES):
    """Whether `value` is a string Nerve will accept: non-empty, control-free, within the bound.

    Control characters are refused rather than stripped, matching `trace.rs::string_field`. A
    producer that emitted one would have the record refused, so it is cheaper to notice here.
    """
    if not isinstance(value, str) or not value:
        return False
    if len(value.encode("utf-8")) > limit:
        return False
    return all(ord(character) >= 0x20 for character in value)


def encode(obj):
    """One artifact line, minus the newline.

    ASCII-only output, so the byte length of the line is its character length and a path with
    non-ASCII characters cannot make a legal record cross `MAX_RECORD_BYTES` unnoticed.
    """
    return json.dumps(obj, separators=(",", ":"), ensure_ascii=True)


def located_record(test_id, caller_file, caller_line, callee_file, callee_line, count, worker):
    """One observed call: two frames, a test, and how many times it was seen.

    `count` is evidence of frequency, never of importance — nothing in Nerve ranks by it.
    """
    return {
        "test_id": test_id,
        "caller_file": caller_file,
        "caller_line": caller_line,
        "callee_file": callee_file,
        "callee_line": callee_line,
        "count": count,
        "worker": worker,
        "async_context": None,
        "resolution": "located",
        "unsupported_form": None,
    }


def unsupported_record(test_id, form, callee_file, callee_line, count, worker):
    """A frame the tracer saw and could not model, named by a closed-vocabulary limitation.

    `resolution` is `unresolved` and `unsupported_form` is set, which is what makes this a
    *limitation* in `nerve trace import` rather than a *refusal*: the producer is admitting a gap,
    not handing Nerve something malformed. An unresolved record with no `unsupported_form` would be
    counted as `producer-unresolved-frame`, a refusal, so this producer never emits one.
    """
    return {
        "test_id": test_id,
        "caller_file": None,
        "caller_line": None,
        "callee_file": callee_file,
        "callee_line": callee_line,
        "count": count,
        "worker": worker,
        "async_context": None,
        "resolution": "unresolved",
        "unsupported_form": form,
    }


def build_header(
    repository_root_name,
    run_id,
    runtime,
    runtime_version,
    platform_name,
    started_at,
    producer_limitations,
    git_commit=None,
):
    """The provisional header: everything known at session start, and `partial` for what is not.

    `content_merkle` is always `None`, and that is a decision rather than an omission. The merkle is
    Nerve's own digest of the tree it indexed; a producer that computed one would be reimplementing
    an internal of the product it is deliberately not part of, and would have to be updated in
    lockstep with it. `git_commit` is enough to bind: `TraceBinding::decide` reports `bound` when any
    declared state field agrees, `stale` on a disagreement and `unverified` when neither side has a
    value — so a run traced in a git repository Nerve indexed at the same commit binds, and one
    traced outside git is honestly `unverified` rather than falsely fresh.
    """
    return {
        "format": FORMAT,
        "format_version": FORMAT_VERSION,
        "producer": PRODUCER,
        "producer_version": VERSION,
        "repository_root_name": repository_root_name,
        "git_commit": git_commit,
        "content_merkle": None,
        "run_id": run_id,
        "test_framework": TEST_FRAMEWORK,
        "runtime": runtime,
        "runtime_version": runtime_version,
        "platform": platform_name,
        "started_at": started_at,
        "completed_at": None,
        "completion_state": "partial",
        "partial_reason": PROVISIONAL_REASON,
        "source_map_state": "none",
        "producer_limitations": sorted(producer_limitations),
    }


class ArtifactWriter:
    """Streams one `nerve-trace/v1` artifact, header first and flushed as it goes.

    Not reusable and not reopenable: one writer is one artifact, which is what lets the header's
    reserved span be computed once and trusted for the life of the file.
    """

    def __init__(self, path, header):
        self.path = path
        self.records_written = 0
        self.records_dropped_too_large = 0
        self.records_dropped_over_ceiling = 0
        self._closed = False
        self._header = dict(header)
        directory = os.path.dirname(os.path.abspath(path))
        if directory:
            os.makedirs(directory, exist_ok=True)
        # Line-buffered text, LF endings on every platform: the reader tolerates CRLF but an
        # artifact that spells its newlines differently per platform would make the end-to-end
        # fixture diff platform-dependent for no gain.
        self._file = open(path, "w", encoding="utf-8", newline="\n")
        line = encode(self._header)
        self._width = len(line) + HEADER_SLACK
        self._bytes = self._width + 1
        self._file.write(self._pad(line))
        self._file.write("\n")
        self._file.flush()

    def _pad(self, line):
        """`line` padded with trailing spaces to the reserved span.

        Whitespace *after* the closing brace rather than inside the object: `serde_json::from_str`
        skips it, and nothing in the reader's `json_depth` byte scan or its key walk can see it.
        """
        return line + " " * (self._width - len(line))

    @property
    def capped(self):
        """Whether any record was dropped because a reader bound was about to be crossed."""
        return self.records_dropped_over_ceiling > 0

    def write(self, records):
        """Append records, flushing so that a killed run leaves everything already written."""
        if self._closed:
            raise ValueError("the artifact is closed")
        wrote = False
        for record in records:
            line = encode(record)
            if len(line) > MAX_RECORD_BYTES:
                # Unreachable for a well-formed record — ten fields each bounded by
                # MAX_STRING_BYTES cannot reach 8 KiB — and counted rather than asserted so that a
                # future field cannot turn a bound into a crash in a user's test run.
                self.records_dropped_too_large += 1
                continue
            if self.records_written >= MAX_RECORDS:
                self.records_dropped_over_ceiling += 1
                continue
            if self._bytes + len(line) + 1 > MAX_ARTIFACT_BYTES - ARTIFACT_BYTE_MARGIN:
                self.records_dropped_over_ceiling += 1
                continue
            self._file.write(line)
            self._file.write("\n")
            self._bytes += len(line) + 1
            self.records_written += 1
            wrote = True
        if wrote:
            self._file.flush()

    def close(self, completion_state, completed_at, partial_reason):
        """Rewrite the header with the run's real outcome, then close.

        Idempotent, because `pytest_sessionfinish` and an exception path may both reach it.
        """
        if self._closed:
            return
        # Validated before anything is marked closed, so a refused outcome leaves a writer that can
        # still be closed properly rather than a file handle nothing will ever release.
        if completion_state not in COMPLETION_STATES:
            raise ValueError("completion_state is outside the contract's vocabulary")
        if completion_state == "complete" and not completed_at:
            # The one contradiction the contract names outright: a producer that claims completion
            # must say when. Refusing here keeps the artifact readable instead of shipping a header
            # `read_header` will reject as `header-invalid`.
            raise ValueError("a complete run must state completed_at")
        self._closed = True
        final = dict(self._header)
        final["completed_at"] = completed_at
        final["completion_state"] = completion_state
        final["partial_reason"] = partial_reason
        line = encode(final)
        if len(line) > self._width:
            # Only `partial_reason` is producer-chosen prose, so it is the only field that can
            # overflow the reserved span, and shortening it is preferable to a corrupted header.
            final["partial_reason"] = None
            line = encode(final)
        if len(line) > self._width:
            raise ValueError("the final header does not fit its reserved span")
        self._file.seek(0)
        self._file.write(self._pad(line))
        self._file.flush()
        self._file.close()
