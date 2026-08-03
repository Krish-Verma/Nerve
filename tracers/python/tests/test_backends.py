"""One suite, both mechanisms, the same expectations.

`docs/plans/slice-11b-python-tracer.md` §3: *a fallback tested only on the machine that does not need
it is a fallback nobody has run.* On 3.12 and later the `sys.settrace` backend is therefore selected
**explicitly** here, so it is exercised on the interpreter that would otherwise never reach it, and
`test_both_backends_agree_exactly` compares the two answers rather than each against a list — a shared
misreading would still be caught by the per-backend expectations, and a divergence is caught by the
comparison.

The sample tree is written to a temporary directory and imported before tracing starts, so its module
body — which executes at import time — is outside the traced window, exactly as a real repository's
module bodies are. See `pytest_plugin`'s module documentation for why that window is per test.
"""

from __future__ import annotations

import importlib.util
import os
import tempfile
import threading
import unittest

from nerve_trace.backend import has_monitoring, select_backend
from nerve_trace.frames import EdgeTable, FrameTracker
from nerve_trace.paths import PathScope
from nerve_trace.record import RECORD_KEYS

#: A tree whose line numbers are load-bearing. The table in `LAYOUT` pins them, so a stray blank line
#: fails with "the sample moved" rather than with an unexplained missing edge.
SAMPLE = '''"""A sample repository, small enough that every expected edge can be read off it."""

import os


def leaf(value):
    return value + 1


def middle(value):
    total = leaf(value)
    total += leaf(value)
    return total


def counted(times):
    total = 0
    for _ in range(times):
        total += leaf(1)
    return total


def produce(count):
    index = 0
    while index < count:
        yield index
        index += 1


def consume():
    total = 0
    for value in produce(2):
        total += value
    return total


def outside():
    return os.path.join("a", "b")


def driver():
    result = middle(1)
    result += counted(3)
    result += consume()
    outside()
    return result
'''

#: `name -> the line its `def` is on`, which is `co_firstlineno` and therefore the `callee_line` this
#: tracer records: it falls inside the symbol extent Nerve will map the record onto, and unlike the
#: frame's current line it does not move as the function runs.
LAYOUT = {
    "leaf": 6,
    "middle": 10,
    "counted": 16,
    "produce": 23,
    "consume": 30,
    "outside": 37,
    "driver": 41,
}

#: Every edge one call to `driver()` must produce, as
#: `(caller_file, caller_line, callee_file, callee_line, count)`.
#:
#: `19 -> 6` has `count: 3` because the loop calls `leaf` three times and edges are counted rather
#: than listed. `32 -> 23` has `count: 3` because a generator's frame is entered once and resumed
#: twice — `PY_START` plus two `PY_RESUME` under `sys.monitoring`, three `'call'` events under
#: `sys.settrace`, which is the measured agreement that lets one suite hold both mechanisms.
EXPECTED_EDGES = {
    ("sample.py", 42, "sample.py", 10, 1),
    ("sample.py", 11, "sample.py", 6, 1),
    ("sample.py", 12, "sample.py", 6, 1),
    ("sample.py", 43, "sample.py", 16, 1),
    ("sample.py", 19, "sample.py", 6, 3),
    ("sample.py", 44, "sample.py", 30, 1),
    ("sample.py", 32, "sample.py", 23, 3),
    ("sample.py", 45, "sample.py", 37, 1),
}


class BackendFixture(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls._temporary = tempfile.TemporaryDirectory()
        cls.root = os.path.realpath(cls._temporary.name)
        path = os.path.join(cls.root, "sample.py")
        with open(path, "w", encoding="utf-8", newline="\n") as handle:
            handle.write(SAMPLE)
        spec = importlib.util.spec_from_file_location("nerve_trace_sample", path)
        cls.sample = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(cls.sample)

    @classmethod
    def tearDownClass(cls):
        cls._temporary.cleanup()

    def trace(self, preference):
        """Run `driver()` under one backend and return `(located edges, records, tracker)`."""
        scope = PathScope(self.root)
        table = EdgeTable("pid:1/thread:main")
        tracker = FrameTracker(scope, table, threading.get_ident())
        backend = select_backend(tracker, preference)
        self.assertIsNone(backend.attach(), "nothing else should hold the hook in a unittest run")
        try:
            tracker.begin_test("tests/test_backends.py::test_driver")
            backend.enable()
            try:
                # Called from this frame, which is outside the sample's root: `driver` itself
                # therefore acquires no edge, which is the same shape pytest's own frame produces.
                # middle(1) = 4, counted(3) = 6, consume() = 1.
                self.assertEqual(self.sample.driver(), 11)
            finally:
                backend.disable()
            records = tracker.end_test()
        finally:
            backend.detach()
        edges = {
            (r["caller_file"], r["caller_line"], r["callee_file"], r["callee_line"], r["count"])
            for r in records
            if r["resolution"] == "located"
        }
        return edges, records, tracker


class LayoutTests(BackendFixture):
    def test_the_sample_is_where_the_expectations_say_it_is(self):
        for name, line in LAYOUT.items():
            self.assertEqual(
                getattr(self.sample, name).__code__.co_firstlineno,
                line,
                f"the sample moved: {name} is no longer on line {line}",
            )


class SharedExpectations:
    """The expectations both mechanisms must meet. Mixed into one test case per backend."""

    preference = None

    def test_the_edge_set_is_exactly_what_the_sample_calls(self):
        edges, _records, _tracker = self.trace(self.preference)
        self.assertEqual(edges, EXPECTED_EDGES)

    def test_a_nested_call_is_attributed_to_its_real_caller(self):
        """`driver -> middle -> leaf` records `middle -> leaf`; `driver` never calls `leaf`."""
        edges, _records, _tracker = self.trace(self.preference)
        callers_of_leaf = sorted({edge[1] for edge in edges if edge[3] == LAYOUT["leaf"]})
        self.assertEqual(callers_of_leaf, [11, 12, 19])
        self.assertNotIn(LAYOUT["driver"], [edge[1] for edge in edges if edge[3] == LAYOUT["leaf"]])

    def test_a_loop_produces_one_record_with_a_count(self):
        edges, _records, _tracker = self.trace(self.preference)
        loop = [edge for edge in edges if edge[1] == 19]
        self.assertEqual(len(loop), 1)
        self.assertEqual(loop[0][4], 3)

    def test_a_generator_is_entered_and_resumed_and_the_stack_survives_it(self):
        edges, _records, tracker = self.trace(self.preference)
        resumes = [edge for edge in edges if edge[3] == LAYOUT["produce"]]
        self.assertEqual(len(resumes), 1)
        self.assertEqual(resumes[0][4], 3)
        # The edge after the generator loop is the proof the stack was not left corrupted by it.
        self.assertIn(("sample.py", 45, "sample.py", LAYOUT["outside"], 1), edges)

    def test_the_standard_library_is_not_in_the_artifact(self):
        edges, records, tracker = self.trace(self.preference)
        for edge in edges:
            self.assertEqual(edge[0], "sample.py")
            self.assertEqual(edge[2], "sample.py")
        self.assertGreater(tracker.scope.outside_root, 0, "the run must have left the repository")
        self.assertGreater(tracker.counts["callee_outside_root"], 0)
        for record in records:
            for key in ("caller_file", "callee_file"):
                value = record[key]
                if value is None:
                    continue
                self.assertFalse(value.startswith("/"), value)
                self.assertNotIn("..", value.split("/"))

    def test_every_record_carries_exactly_the_contract_keys(self):
        _edges, records, _tracker = self.trace(self.preference)
        self.assertTrue(records)
        for record in records:
            self.assertEqual(tuple(record.keys()), tuple(RECORD_KEYS))

    def test_the_tracer_stack_is_empty_afterwards(self):
        _edges, _records, tracker = self.trace(self.preference)
        self.assertEqual(tracker.depth(), 0)

    def test_the_hook_is_released(self):
        self.trace(self.preference)
        second_scope = PathScope(self.root)
        tracker = FrameTracker(second_scope, EdgeTable(None), threading.get_ident())
        backend = select_backend(tracker, self.preference)
        try:
            self.assertIsNone(backend.attach(), "detach must leave the mechanism available")
        finally:
            backend.detach()


class SettraceBackendTests(SharedExpectations, BackendFixture):
    """The fallback, selected explicitly so it runs on 3.12+ as well as on the versions that need it."""

    preference = "settrace"

    def test_the_backend_really_is_settrace(self):
        tracker = FrameTracker(PathScope(self.root), EdgeTable(None), threading.get_ident())
        self.assertEqual(select_backend(tracker, "settrace").name, "sys.settrace")

    def test_it_refuses_to_displace_another_tracer(self):
        """Evicting `coverage.py` to produce a trace nobody asked for would be the wrong trade."""
        import sys

        def other(frame, event, _arg):
            return None

        tracker = FrameTracker(PathScope(self.root), EdgeTable(None), threading.get_ident())
        backend = select_backend(tracker, "settrace")
        sys.settrace(other)
        try:
            self.assertEqual(backend.attach(), "profiler-contention")
        finally:
            sys.settrace(None)
        self.assertIs(sys.gettrace(), None, "the refusal must not have touched the other tracer")


@unittest.skipUnless(has_monitoring(), "sys.monitoring needs CPython 3.12 or later")
class MonitoringBackendTests(SharedExpectations, BackendFixture):
    preference = "monitoring"

    def test_the_backend_really_is_monitoring(self):
        tracker = FrameTracker(PathScope(self.root), EdgeTable(None), threading.get_ident())
        self.assertEqual(select_backend(tracker, "monitoring").name, "sys.monitoring")

    def test_auto_selects_monitoring_when_the_interpreter_has_it(self):
        tracker = FrameTracker(PathScope(self.root), EdgeTable(None), threading.get_ident())
        self.assertEqual(select_backend(tracker, "auto").name, "sys.monitoring")

    def test_it_refuses_to_displace_another_tool_holding_its_id(self):
        import sys

        tracker = FrameTracker(PathScope(self.root), EdgeTable(None), threading.get_ident())
        backend = select_backend(tracker, "monitoring")
        sys.monitoring.use_tool_id(backend.TOOL_ID, "someone-else")
        try:
            self.assertEqual(backend.attach(), "profiler-contention")
        finally:
            sys.monitoring.free_tool_id(backend.TOOL_ID)

    def test_a_frame_on_another_thread_is_reported_as_the_threads_limitation(self):
        """Only `sys.monitoring` can see another thread at all, so only it can report on one."""
        scope = PathScope(self.root)
        table = EdgeTable("pid:1/thread:main")
        tracker = FrameTracker(scope, table, threading.get_ident())
        backend = select_backend(tracker, "monitoring")
        self.assertIsNone(backend.attach())
        try:
            tracker.begin_test("tests/test_backends.py::test_thread")
            backend.enable()
            try:
                worker = threading.Thread(target=self.sample.driver)
                worker.start()
                worker.join()
            finally:
                backend.disable()
            records = tracker.end_test()
        finally:
            backend.detach()
        self.assertGreater(tracker.counts["other_thread"], 0)
        forms = {record["unsupported_form"] for record in records}
        self.assertIn("threads", forms)
        located = [record for record in records if record["resolution"] == "located"]
        self.assertEqual(located, [], "another thread's frames must not become located edges")


class AgreementTests(BackendFixture):
    @unittest.skipUnless(has_monitoring(), "needs both mechanisms present to compare them")
    def test_both_backends_agree_exactly(self):
        settrace_edges, _r1, _t1 = self.trace("settrace")
        monitoring_edges, _r2, _t2 = self.trace("monitoring")
        self.assertEqual(settrace_edges, monitoring_edges)


class SelectionTests(unittest.TestCase):
    def test_an_unknown_preference_is_refused(self):
        with self.assertRaises(ValueError):
            select_backend(None, "strace")

    @unittest.skipIf(has_monitoring(), "this interpreter has sys.monitoring")
    def test_asking_for_monitoring_without_it_raises_rather_than_falling_back(self):
        with self.assertRaises(RuntimeError):
            select_backend(None, "monitoring")


if __name__ == "__main__":
    unittest.main()
