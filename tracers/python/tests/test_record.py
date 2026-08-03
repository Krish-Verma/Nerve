"""The artifact writer, measured against the contract `crates/nerve-index/src/trace.rs` enforces.

The key lists are restated here rather than imported from the module under test. That is the point:
if `record.py` and the Rust reader ever disagree, importing the producer's own opinion of the contract
would agree with itself and prove nothing.
"""

from __future__ import annotations

import json
import os
import tempfile
import unittest

from nerve_trace.record import (
    HEADER_KEYS,
    MAX_RECORD_BYTES,
    MAX_STRING_BYTES,
    RECORD_KEYS,
    ArtifactWriter,
    build_header,
    is_clean_string,
    located_record,
    unsupported_record,
)

#: `trace.rs::HEADER_KEYS`, quoted. An unknown key here refuses the artifact **whole**.
READER_HEADER_KEYS = (
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

#: `trace.rs::RECORD_KEYS`, quoted. An unknown key here is counted as `record-unknown-key`.
READER_RECORD_KEYS = (
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


def a_header():
    return build_header(
        repository_root_name="repo",
        run_id="run-1",
        runtime="cpython",
        runtime_version="3.12.4",
        platform_name="darwin-arm64",
        started_at="2026-08-03T10:00:00Z",
        producer_limitations=["threads", "native-frames"],
        git_commit="0" * 40,
    )


class ContractTests(unittest.TestCase):
    def test_the_header_carries_exactly_the_keys_the_reader_knows(self):
        self.assertEqual(tuple(HEADER_KEYS), READER_HEADER_KEYS)
        self.assertEqual(tuple(a_header().keys()), READER_HEADER_KEYS)

    def test_a_record_carries_exactly_the_keys_the_reader_knows(self):
        self.assertEqual(tuple(RECORD_KEYS), READER_RECORD_KEYS)
        located = located_record("t", "a.py", 1, "b.py", 2, 3, "pid:1/thread:main")
        self.assertEqual(tuple(located.keys()), READER_RECORD_KEYS)
        gap = unsupported_record("t", "threads", None, None, 1, None)
        self.assertEqual(tuple(gap.keys()), READER_RECORD_KEYS)

    def test_no_record_key_could_hold_a_value(self):
        """The contract has nowhere to put an argument or a return value, and neither does this."""
        for key in READER_RECORD_KEYS:
            for forbidden in ("arg", "return", "local", "value", "repr", "source"):
                self.assertNotIn(forbidden, key)

    def test_the_content_merkle_is_null_and_the_commit_carries_the_binding(self):
        header = a_header()
        self.assertIsNone(header["content_merkle"])
        self.assertEqual(header["git_commit"], "0" * 40)

    def test_a_located_record_never_declares_a_limitation(self):
        located = located_record("t", "a.py", 1, "b.py", 2, 1, None)
        self.assertEqual(located["resolution"], "located")
        self.assertIsNone(located["unsupported_form"])

    def test_an_unresolved_record_always_declares_one(self):
        """An unresolved record with no form is counted as a refusal, not a limitation."""
        gap = unsupported_record("t", "native-frames", "a.py", 4, 2, None)
        self.assertEqual(gap["resolution"], "unresolved")
        self.assertEqual(gap["unsupported_form"], "native-frames")
        self.assertIsNone(gap["caller_file"])

    def test_the_provisional_header_says_partial_and_explains_itself(self):
        header = a_header()
        self.assertEqual(header["completion_state"], "partial")
        self.assertIsNone(header["completed_at"])
        self.assertTrue(header["partial_reason"])

    def test_producer_limitations_are_sorted(self):
        self.assertEqual(a_header()["producer_limitations"], ["native-frames", "threads"])


class CleanStringTests(unittest.TestCase):
    def test_the_readers_rules_are_applied_here(self):
        self.assertTrue(is_clean_string("tests/test_x.py::test_y"))
        self.assertFalse(is_clean_string(""))
        self.assertFalse(is_clean_string(None))
        self.assertFalse(is_clean_string("a\tb"))
        self.assertFalse(is_clean_string("a" * (MAX_STRING_BYTES + 1)))
        self.assertTrue(is_clean_string("a" * MAX_STRING_BYTES))

    def test_the_bound_is_measured_in_bytes_not_characters(self):
        # The reader measures `text.len()`, which is UTF-8 bytes.
        self.assertFalse(is_clean_string("é" * ((MAX_STRING_BYTES // 2) + 1)))


class WriterFixture(unittest.TestCase):
    def setUp(self):
        self._temporary = tempfile.TemporaryDirectory()
        self.path = os.path.join(self._temporary.name, "trace", "run.jsonl")

    def tearDown(self):
        self._temporary.cleanup()

    def open_writer(self, header=None):
        """An `ArtifactWriter` that is closed when the test ends, whatever the test did to it.

        `close` is idempotent, so a test that closes it deliberately is unaffected; the cleanup exists
        so an assertion failure cannot leave a file handle open and turn one failure into a warning
        storm.
        """
        writer = ArtifactWriter(self.path, header if header is not None else a_header())
        self.addCleanup(writer.close, "partial", None, "closed by the test harness")
        return writer

    def read(self):
        with open(self.path, "r", encoding="utf-8") as handle:
            return handle.read()

    def lines(self):
        return [line for line in self.read().split("\n") if line.strip()]


class WriterTests(WriterFixture):
    def test_the_header_is_on_disk_and_parseable_before_any_record_is_written(self):
        writer = self.open_writer()
        header = json.loads(self.lines()[0])
        self.assertEqual(header["format"], "nerve-trace")
        self.assertEqual(header["format_version"], 1)
        writer.close("complete", "2026-08-03T10:00:04Z", None)

    def test_the_directory_is_created(self):
        self.open_writer().close("complete", "2026-08-03T10:00:04Z", None)
        self.assertTrue(os.path.isfile(self.path))

    def test_the_header_line_is_padded_and_json_tolerates_the_padding(self):
        writer = self.open_writer()
        first = self.read().split("\n")[0]
        self.assertTrue(first.endswith(" "), "the header must reserve room for its rewrite")
        self.assertEqual(json.loads(first)["completion_state"], "partial")
        writer.close("complete", "2026-08-03T10:00:04Z", None)

    def test_close_rewrites_the_header_in_place_without_moving_any_record(self):
        writer = self.open_writer()
        writer.write([located_record("t", "a.py", 1, "b.py", 2, 1, None)])
        before = self.lines()[1]
        writer.close("complete", "2026-08-03T10:00:04Z", None)
        after = self.lines()
        self.assertEqual(after[1], before)
        header = json.loads(after[0])
        self.assertEqual(header["completion_state"], "complete")
        self.assertEqual(header["completed_at"], "2026-08-03T10:00:04Z")
        self.assertIsNone(header["partial_reason"])

    def test_a_complete_run_must_say_when(self):
        """The one contradiction the contract names: refused here rather than shipped."""
        writer = self.open_writer()
        with self.assertRaises(ValueError):
            writer.close("complete", None, None)

    def test_a_state_outside_the_vocabulary_is_refused(self):
        writer = self.open_writer()
        with self.assertRaises(ValueError):
            # `interrupted` is not one of the three; an interrupted run is `partial` with a reason.
            writer.close("interrupted", "2026-08-03T10:00:04Z", "stopped")

    def test_closing_twice_is_a_no_op(self):
        writer = self.open_writer()
        writer.close("partial", None, "stopped")
        writer.close("partial", None, "stopped")

    def test_an_overlong_partial_reason_is_shortened_rather_than_corrupting_the_header(self):
        writer = self.open_writer()
        writer.write([located_record("t", "a.py", 1, "b.py", 2, 1, None)])
        record_line = self.lines()[1]
        writer.close("partial", None, "x" * 4000)
        self.assertEqual(self.lines()[1], record_line)
        self.assertIsNone(json.loads(self.lines()[0])["partial_reason"])

    def test_every_record_is_flushed_as_it_is_written(self):
        writer = self.open_writer()
        writer.write([located_record("t", "a.py", 1, "b.py", 2, 1, None)])
        self.assertEqual(len(self.lines()), 2, "an unflushed record is a record a kill would lose")
        writer.close("complete", "2026-08-03T10:00:04Z", None)

    def test_a_record_over_the_readers_line_bound_is_dropped_and_counted(self):
        writer = self.open_writer()
        writer.write([located_record("t" * 9000, "a.py", 1, "b.py", 2, 1, None)])
        self.assertEqual(writer.records_written, 0)
        self.assertEqual(writer.records_dropped_too_large, 1)
        writer.close("complete", "2026-08-03T10:00:04Z", None)

    def test_a_well_formed_record_is_far_below_the_line_bound(self):
        record = located_record(
            "t" * MAX_STRING_BYTES,
            "a" * MAX_STRING_BYTES,
            999999,
            "b" * MAX_STRING_BYTES,
            999999,
            999999,
            "w" * MAX_STRING_BYTES,
        )
        self.assertLess(len(json.dumps(record)), MAX_RECORD_BYTES)

    def test_the_artifact_is_ascii_so_a_line_length_is_a_byte_length(self):
        writer = self.open_writer()
        writer.write([located_record("t", "café.py", 1, "b.py", 2, 1, None)])
        writer.close("complete", "2026-08-03T10:00:04Z", None)
        with open(self.path, "rb") as handle:
            self.assertTrue(all(byte < 0x80 for byte in handle.read()))

    def test_writing_after_close_is_refused(self):
        writer = self.open_writer()
        writer.close("partial", None, "stopped")
        with self.assertRaises(ValueError):
            writer.write([located_record("t", "a.py", 1, "b.py", 2, 1, None)])


class TruncationTests(WriterFixture):
    def test_a_truncated_artifact_keeps_a_readable_header_and_every_whole_record(self):
        """A killed run. NDJSON's whole reason for being: the break costs one line, not the file.

        The header still says `partial`, which is the truth about a run that was killed, because the
        rewrite that would have claimed completion never happened.
        """
        writer = self.open_writer()
        writer.write(
            [
                located_record("t", "a.py", 1, "b.py", 2, 1, None),
                located_record("t", "a.py", 3, "c.py", 4, 2, None),
            ]
        )
        whole = self.read()
        # Cut the file in the middle of the final record, as a signal would.
        cut = len(whole) - 20
        with open(self.path, "w", encoding="utf-8", newline="\n") as handle:
            handle.write(whole[:cut])

        raw = self.read().split("\n")
        header = json.loads(raw[0])
        self.assertEqual(header["completion_state"], "partial")
        self.assertIsNone(header["completed_at"])
        self.assertEqual(json.loads(raw[1])["callee_file"], "b.py")
        with self.assertRaises(ValueError):
            json.loads(raw[2])

    def test_a_header_written_and_nothing_else_is_still_a_readable_artifact(self):
        self.open_writer()
        header = json.loads(self.read().split("\n")[0])
        self.assertEqual(header["completion_state"], "partial")


if __name__ == "__main__":
    unittest.main()
