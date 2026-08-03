"""The frame stack and the edge accumulator — the load-bearing half of the tracer.

`FrameTracker` is driven directly here, with synthetic frame identifiers, because the property that
matters is a rule about a stack rather than a behaviour of an interpreter: driving it directly is the
only way to reach the cases a real run reaches rarely and a released tracer must still get right.
`test_backends.py` then runs both real mechanisms over real code to show the wiring agrees.
"""

from __future__ import annotations

import os
import tempfile
import unittest

from nerve_trace.frames import DECLARED_LIMITATIONS, EdgeTable, FrameTracker
from nerve_trace.paths import PathScope

#: `trace.rs::limitation::ALL`, quoted. The vocabulary is closed there: a value outside it is counted
#: as `limitation-unknown` and dropped, so a producer inventing one would have its admission of a gap
#: reported as Nerve rejecting its output.
READER_LIMITATIONS = (
    "async-continuations",
    "threads",
    "multiprocessing-children",
    "native-frames",
    "dynamic-imports",
    "generated-code",
    "framework-wrappers",
    "sampling-gap",
    "crashed-test",
    "interrupted-run",
    "parallel-tests-shared-process",
    "profiler-contention",
)

MAIN = 1
OTHER = 2


class TrackerFixture(unittest.TestCase):
    def setUp(self):
        self._temporary = tempfile.TemporaryDirectory()
        self.root = os.path.realpath(self._temporary.name)
        self.scope = PathScope(self.root)
        self.table = EdgeTable("pid:1/thread:main")
        self.tracker = FrameTracker(self.scope, self.table, MAIN)
        self.tracker.begin_test("tests/test_parse.py::test_basic")

    def tearDown(self):
        self._temporary.cleanup()

    def inside(self, name):
        return os.path.join(self.root, name)

    def enter(self, frame_id, callee, callee_line, caller_id=None, caller=None, caller_line=None):
        self.tracker.enter(
            MAIN,
            frame_id,
            self.inside(callee),
            callee_line,
            caller_id,
            None if caller is None else self.inside(caller),
            caller_line,
        )

    def edges(self):
        """Every located edge as `(caller_file, caller_line, callee_file, callee_line, count)`."""
        out = []
        for record in self.table.drain():
            if record["resolution"] != "located":
                continue
            out.append(
                (
                    record["caller_file"],
                    record["caller_line"],
                    record["callee_file"],
                    record["callee_line"],
                    record["count"],
                )
            )
        return out


class AttributionTests(TrackerFixture):
    def test_a_nested_call_is_attributed_to_its_real_caller_and_never_to_the_test(self):
        """`test_basic -> parse -> tokenize` records `parse -> tokenize`, and nothing else.

        The consumer-side twin of
        `crates/nerve-index/tests/trace.rs::a_nested_call_is_attributed_to_its_real_caller_and_never_to_the_test`.
        Slice 11a's §2.1 correction exists because an earlier plan made the test the source of every
        edge; this is the producer refusing to undo it.
        """
        # pytest's own frame called the test function: the caller is out of the repository, so the
        # test's own frame acquires no edge and simply joins the stack.
        self.tracker.enter(
            MAIN, 10, self.inside("tests/test_parse.py"), 6, 9, "/elsewhere/pytest/python.py", 200
        )
        self.enter(11, "src/parse.py", 8, 10, "tests/test_parse.py", 8)
        self.enter(12, "src/lex.py", 4, 11, "src/parse.py", 10)

        edges = self.edges()
        self.assertIn(("src/parse.py", 10, "src/lex.py", 4, 1), edges)
        self.assertIn(("tests/test_parse.py", 8, "src/parse.py", 8, 1), edges)
        callers_of_lex = [edge[0] for edge in edges if edge[2] == "src/lex.py"]
        self.assertEqual(callers_of_lex, ["src/parse.py"])
        self.assertNotIn("tests/test_parse.py", callers_of_lex)

    def test_the_test_id_is_the_only_place_the_test_appears_for_a_deep_edge(self):
        self.tracker.enter(
            MAIN, 10, self.inside("tests/test_parse.py"), 6, 9, "/elsewhere/pytest.py", 1
        )
        self.enter(11, "src/parse.py", 8, 10, "tests/test_parse.py", 8)
        self.enter(12, "src/lex.py", 4, 11, "src/parse.py", 10)
        deep = [r for r in self.table.drain() if r["callee_file"] == "src/lex.py"]
        self.assertEqual(len(deep), 1)
        self.assertEqual(deep[0]["test_id"], "tests/test_parse.py::test_basic")
        self.assertEqual(deep[0]["caller_file"], "src/parse.py")

    def test_an_edge_is_counted_not_listed(self):
        """A loop calling one function a million times is one record with a count."""
        self.enter(1, "src/parse.py", 8, None, None, None)
        for frame_id in range(100, 1100):
            self.enter(frame_id, "src/lex.py", 4, 1, "src/parse.py", 10)
            self.tracker.exit(MAIN, frame_id)
        located = [r for r in self.table.drain() if r["resolution"] == "located"]
        self.assertEqual(len(located), 1)
        self.assertEqual(located[0]["count"], 1000)

    def test_two_call_sites_in_one_caller_are_two_edges(self):
        self.enter(1, "src/parse.py", 8, None, None, None)
        self.enter(2, "src/lex.py", 4, 1, "src/parse.py", 10)
        self.tracker.exit(MAIN, 2)
        self.enter(3, "src/lex.py", 9, 1, "src/parse.py", 11)
        self.tracker.exit(MAIN, 3)
        self.assertEqual(len(self.edges()), 2)


class DropTests(TrackerFixture):
    def test_the_first_frame_of_a_thread_is_dropped_and_counted(self):
        """No Python caller at all: reported as `native-frames`, never attributed to the stack."""
        self.enter(1, "src/parse.py", 8, None, None, None)
        self.assertEqual(self.tracker.counts["caller_not_observed"], 1)
        records = self.table.drain()
        self.assertEqual(len(records), 1)
        self.assertEqual(records[0]["unsupported_form"], "native-frames")
        self.assertEqual(records[0]["resolution"], "unresolved")
        self.assertIsNone(records[0]["caller_file"])

    def test_a_caller_the_tracer_never_saw_is_dropped_rather_than_taken_from_the_top(self):
        """The whole point. Frame 1 is on the stack; the callee's real caller is frame 99."""
        self.enter(1, "src/parse.py", 8, None, None, None)
        self.enter(2, "src/lex.py", 4, 99, "src/other.py", 3)
        located = [r for r in self.table.drain() if r["resolution"] == "located"]
        self.assertEqual(located, [])
        self.assertEqual(self.tracker.counts["caller_not_observed"], 2)

    def test_a_frame_on_another_thread_is_dropped_and_counted_as_threads(self):
        self.tracker.enter(
            OTHER, 5, self.inside("src/lex.py"), 4, 6, self.inside("src/parse.py"), 10
        )
        self.assertEqual(self.tracker.counts["other_thread"], 1)
        records = self.table.drain()
        self.assertEqual(len(records), 1)
        self.assertEqual(records[0]["unsupported_form"], "threads")
        self.assertEqual(self.tracker.depth(), 0, "another thread must not touch this stack")

    def test_a_callee_outside_the_root_is_dropped_silently(self):
        self.enter(1, "src/parse.py", 8, None, None, None)
        self.tracker.enter(
            MAIN, 2, "/elsewhere/site-packages/json/encoder.py", 20, 1, self.inside("src/parse.py"), 10
        )
        self.assertEqual(self.tracker.counts["callee_outside_root"], 1)
        located = [r for r in self.table.drain() if r["resolution"] == "located"]
        self.assertEqual(located, [])

    def test_a_caller_outside_the_root_is_dropped_silently(self):
        self.tracker.enter(
            MAIN, 1, self.inside("src/parse.py"), 8, 0, "/elsewhere/pytest/python.py", 100
        )
        self.assertEqual(self.tracker.counts["caller_outside_root"], 1)
        self.assertEqual(self.table.drain(), [])

    def test_an_out_of_root_drop_is_not_reported_as_a_limitation(self):
        """The standard library not being in the repository is not a gap in the trace."""
        self.enter(1, "src/parse.py", 8, None, None, None)
        self.table.drain()
        self.tracker.enter(MAIN, 2, "/elsewhere/json/encoder.py", 20, 1, self.inside("src/parse.py"), 10)
        self.assertEqual(self.table.drain(), [])

    def test_events_outside_any_test_are_dropped(self):
        self.tracker.end_test()
        self.enter(1, "src/parse.py", 8, None, None, None)
        self.assertEqual(self.tracker.counts["no_current_test"], 1)
        self.assertEqual(self.table.drain(), [])

    def test_a_test_id_the_reader_would_refuse_leaves_no_current_test(self):
        for bad in ("", "a" * 513, "has\nnewline"):
            tracker = FrameTracker(self.scope, EdgeTable(None), MAIN)
            tracker.begin_test(bad)
            self.assertIsNone(tracker.current_test)
            self.assertEqual(tracker.counts["test_id_refused"], 1)

    def test_a_line_number_below_one_is_dropped(self):
        self.enter(1, "src/parse.py", 8, None, None, None)
        self.enter(2, "src/lex.py", 0, 1, "src/parse.py", 10)
        self.assertEqual(self.tracker.counts["bad_line_number"], 1)
        self.assertEqual([r for r in self.table.drain() if r["resolution"] == "located"], [])


class StackTests(TrackerFixture):
    def test_the_stack_mirrors_the_interpreter_even_for_frames_that_record_nothing(self):
        self.tracker.enter(MAIN, 1, "/elsewhere/a.py", 1, None, None, None)
        self.assertEqual(self.tracker.depth(), 1)
        self.enter(2, "src/parse.py", 8, 1, "src/parse.py", 1)
        self.assertEqual(self.tracker.depth(), 2)

    def test_an_exit_pops(self):
        self.enter(1, "src/parse.py", 8, None, None, None)
        self.enter(2, "src/lex.py", 4, 1, "src/parse.py", 10)
        self.tracker.exit(MAIN, 2)
        self.assertEqual(self.tracker.depth(), 1)

    def test_a_missed_exit_resynchronises_rather_than_leaving_a_lie(self):
        self.enter(1, "src/parse.py", 8, None, None, None)
        self.enter(2, "src/lex.py", 4, 1, "src/parse.py", 10)
        self.enter(3, "src/lex.py", 9, 2, "src/lex.py", 5)
        self.tracker.exit(MAIN, 1)
        self.assertEqual(self.tracker.depth(), 0)
        self.assertEqual(self.tracker.counts["stack_resynchronised"], 1)

    def test_an_exit_for_a_frame_never_entered_is_counted(self):
        self.tracker.exit(MAIN, 77)
        self.assertEqual(self.tracker.counts["unbalanced_exit"], 1)

    def test_beginning_a_test_clears_the_previous_tests_stack(self):
        self.enter(1, "src/parse.py", 8, None, None, None)
        self.tracker.begin_test("tests/test_parse.py::test_method")
        self.assertEqual(self.tracker.depth(), 0)

    def test_ending_a_test_drains_only_that_test(self):
        self.enter(1, "src/parse.py", 8, None, None, None)
        self.enter(2, "src/lex.py", 4, 1, "src/parse.py", 10)
        self.table.observe("tests/test_parse.py::test_other", "src/a.py", 1, "src/b.py", 2)
        drained = self.tracker.end_test()
        self.assertTrue(all(r["test_id"] == "tests/test_parse.py::test_basic" for r in drained))
        remaining = self.table.drain()
        self.assertEqual([r["test_id"] for r in remaining], ["tests/test_parse.py::test_other"])


class VocabularyTests(unittest.TestCase):
    def test_every_declared_limitation_is_a_member_of_the_readers_closed_vocabulary(self):
        for form in DECLARED_LIMITATIONS:
            self.assertIn(form, READER_LIMITATIONS)

    def test_the_declaration_omits_the_forms_this_producer_does_not_have(self):
        """Declaring a limitation this tracer does not have would overstate its blindness."""
        for absent in ("dynamic-imports", "sampling-gap", "profiler-contention"):
            self.assertNotIn(absent, DECLARED_LIMITATIONS)


class DrainOrderTests(unittest.TestCase):
    def test_records_come_out_in_a_stable_order(self):
        """Two runs that saw the same calls must produce byte-identical lines, or the end-to-end
        fixture diff in `scripts/trace_python_e2e.sh` would fail on dictionary ordering."""
        first = EdgeTable(None)
        second = EdgeTable(None)
        calls = [
            ("t", "b.py", 2, "c.py", 3),
            ("t", "a.py", 1, "z.py", 9),
            ("t", "a.py", 1, "b.py", 2),
        ]
        for call in calls:
            first.observe(*call)
        for call in reversed(calls):
            second.observe(*call)
        first.observe_unsupported("t", "threads", None, None)
        second.observe_unsupported("t", "threads", None, None)
        self.assertEqual(first.drain(), second.drain())

    def test_a_null_location_sorts_without_comparing_across_types(self):
        table = EdgeTable(None)
        table.observe_unsupported("t", "threads", None, None)
        table.observe_unsupported("t", "native-frames", "a.py", 4)
        forms = [record["unsupported_form"] for record in table.drain()]
        self.assertEqual(sorted(forms), forms)


if __name__ == "__main__":
    unittest.main()
