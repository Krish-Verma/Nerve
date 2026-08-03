"""The tracer's own frame stack, and the accumulator that counts edges instead of listing them.

# A nested call belongs to its real caller

`docs/plans/slice-11a-trace-ingestion.md` §2.1 is the correction this module exists to preserve. For
the stack `test_basic -> parse -> tokenize` the artifact must carry `parse -> tokenize`, and
`test_basic` must appear **only** in `test_id`. Making the test the source of every edge would either
assert a call the test never made or throw away everything below depth 1;
`crates/nerve-index/tests/trace.rs::a_nested_call_is_attributed_to_its_real_caller_and_never_to_the_test`
is the consumer-side half of the same invariant.

Neither tracing mechanism hands over a caller. `sys.monitoring`'s `PY_START` says *this code began*,
not *from where*; `sys.settrace`'s `'call'` hands over a frame and leaves the rest to the caller. So
the tracer keeps its own stack, and the rule is deliberately narrow:

> The caller is the frame the interpreter itself names as the caller — `frame.f_back` — **and only**
> when that frame is the one on top of the tracer's stack. Otherwise the callee is dropped.

The second clause is the whole point. Taking the top of the stack as the caller without checking is
the bug: after any missed event the top of the stack is *some* frame, and the edge would be recorded
against it with no sign that anything was wrong. Comparing against `f_back` means a divergence
between the interpreter's stack and the tracer's produces a **drop and a count**, never a confident
lie.

## Which frames are dropped, and how each is reported

| case | why the caller is unavailable | reported as |
|---|---|---|
| the first Python frame of a thread | `f_back` is `None` | `native-frames` record |
| a frame on a thread the tracer is not installed on | no stack for it, and no certainty about which test it belongs to | `threads` record |
| a caller frame the tracer never saw start | tracing began mid-stack, or an unobserved frame stands between | `native-frames` record |
| either side outside the repository root | Nerve has no row for the file | counted, not recorded — see `paths` |
| no test executing | nothing to attribute it to | counted, not recorded |

A drop that is *reported* becomes a record with `resolution: "unresolved"` and an
`unsupported_form` from `trace.rs::limitation::ALL`, which `nerve trace import` counts as a
limitation rather than a refusal. A drop that is merely *counted* appears in pytest's terminal
summary. The split is the one the reader's two counter vocabularies already make: a limitation is
the producer admitting a gap, and the standard library not being part of the repository is not a gap.

## The residual case this tracer cannot see, stated rather than hidden

A native frame *between* two Python frames — `sorted(key=f)`, a C-implemented decorator — is
invisible to both mechanisms: `f_back` skips it and names the Python frame below, which is on the
stack, so the edge is recorded as if the outer Python frame had called `f` directly. Detecting it
would require monitoring every `CALL` including calls to C functions, whose cost is the reason
`sys.monitoring`'s per-event opt-in was chosen in the first place. The header therefore declares
`native-frames` for every run: the artifact says, up front, that native frames were not observable.

# Edges are counted, not listed

The accumulator is keyed on `(test_id, caller_file, caller_line, callee_file, callee_line)` with a
`count`. A loop calling one function a million times is **one** record with `count: 1000000`, which
is what keeps an artifact bounded, and `count` is evidence of frequency and never of importance.
Records are drained per test, so memory is bounded by one test's distinct call sites and a killed run
loses only the test in progress.
"""

from __future__ import annotations

from .record import is_clean_string, located_record, unsupported_record

#: Members of `trace.rs::limitation::ALL` this producer emits on a record. The vocabulary is closed
#: there: a value outside it is counted as `limitation-unknown` and dropped, so these are quoted
#: exactly and `tests/test_frames.py` asserts they are a subset of the reader's list.
NATIVE_FRAMES = "native-frames"
THREADS = "threads"
ASYNC_CONTINUATIONS = "async-continuations"
MULTIPROCESSING_CHILDREN = "multiprocessing-children"
GENERATED_CODE = "generated-code"
PROFILER_CONTENTION = "profiler-contention"
INTERRUPTED_RUN = "interrupted-run"

#: What the header declares for every run of this producer, whatever it happens to observe.
#:
#: A header limitation says *this run could not see this class of thing at all*, which is a statement
#: about the mechanism rather than about what turned up. `native-frames` because a C frame has no
#: source location and an interposed one is invisible; `threads` because only the thread pytest calls
#: the tracer on is traced; `async-continuations` because a coroutine resumed by an event loop has no
#: recoverable logical caller; `multiprocessing-children` because a child process has its own
#: interpreter and no tracer; `generated-code` because `exec` and `eval` frames have no file in the
#: repository to name.
#:
#: Deliberately absent: `dynamic-imports` (a dynamically imported module's body is an ordinary Python
#: frame this tracer sees), `sampling-gap` (nothing here samples) and `profiler-contention` (added
#: only when installation is actually refused).
DECLARED_LIMITATIONS = (
    NATIVE_FRAMES,
    THREADS,
    ASYNC_CONTINUATIONS,
    MULTIPROCESSING_CHILDREN,
    GENERATED_CODE,
)


class EdgeTable:
    """Counts observed edges and admitted gaps, and drains them per test as records."""

    def __init__(self, worker=None):
        self.worker = worker if is_clean_string(worker) else None
        self._edges = {}
        self._unsupported = {}

    def observe(self, test_id, caller_file, caller_line, callee_file, callee_line):
        """Record one occurrence of a located edge."""
        key = (test_id, caller_file, caller_line, callee_file, callee_line)
        self._edges[key] = self._edges.get(key, 0) + 1

    def observe_unsupported(self, test_id, form, callee_file, callee_line):
        """Record one occurrence of a frame that could not be modelled, by limitation form."""
        key = (test_id, form, callee_file, callee_line)
        self._unsupported[key] = self._unsupported.get(key, 0) + 1

    def pending(self):
        """How many records a drain would produce right now."""
        return len(self._edges) + len(self._unsupported)

    def drain(self, test_id=None):
        """Remove and return records, for one test or for every test.

        Sorted by key, so two runs that observed the same calls produce byte-identical record lines
        and `scripts/trace_python_e2e.sh` can diff an artifact against a committed fixture.
        """
        records = []
        located = [key for key in self._edges if test_id is None or key[0] == test_id]
        for key in sorted(located, key=_sort_key):
            records.append(
                located_record(
                    key[0], key[1], key[2], key[3], key[4], self._edges.pop(key), self.worker
                )
            )
        gaps = [key for key in self._unsupported if test_id is None or key[0] == test_id]
        for key in sorted(gaps, key=_sort_key):
            records.append(
                unsupported_record(
                    key[0], key[1], key[2], key[3], self._unsupported.pop(key), self.worker
                )
            )
        return records


def _sort_key(key):
    """Order a mixed-nullability key deterministically.

    `None` sorts before any string or number, which `<` refuses to do across types, so each element
    becomes `(is_not_none, value)` with a type-stable placeholder.
    """
    ordered = []
    for element in key:
        if element is None:
            ordered.append((0, "", 0))
        elif isinstance(element, str):
            ordered.append((1, element, 0))
        else:
            ordered.append((2, "", element))
    return tuple(ordered)


class FrameTracker:
    """One thread's frame stack, and the attribution rules applied on the way in.

    One stack, not a map of them, and the reason is the `threads` rule rather than an omission:
    frames on any thread other than the one the tracer was installed on are turned away at the door,
    so no second stack could ever acquire an entry. `sys.settrace` cannot see other threads at all
    and `sys.monitoring` sees all of them, so turning them away is also what makes the two backends
    agree — which is what lets one test suite hold both to the same expectations.
    """

    def __init__(self, scope, table, thread_id):
        self.scope = scope
        self.table = table
        self.thread_id = thread_id
        self.current_test = None
        self._stack = []
        self.counts = {
            "callee_outside_root": 0,
            "caller_outside_root": 0,
            "caller_not_observed": 0,
            "other_thread": 0,
            "no_current_test": 0,
            "unbalanced_exit": 0,
            "stack_resynchronised": 0,
            "test_id_refused": 0,
            "bad_line_number": 0,
        }

    def _count(self, name):
        self.counts[name] += 1

    def begin_test(self, test_id):
        """Bracket a test. Every edge recorded until `end_test` is attributed to `test_id`.

        A node id Nerve would refuse — empty, over `MAX_STRING_BYTES`, or carrying a control
        character — leaves no current test rather than being truncated: a shortened node id is a
        different test's name, and naming the wrong test is worse than naming none.
        """
        if not is_clean_string(test_id):
            self.current_test = None
            self._count("test_id_refused")
        else:
            self.current_test = test_id
        # pytest's own frames come and go between tests and are none of the tracer's business; a
        # frame left on the stack by the previous test would otherwise be offered as a caller.
        self._stack = []

    def end_test(self):
        """Close the current test and return its records."""
        test_id = self.current_test
        self.current_test = None
        self._stack = []
        if test_id is None:
            return []
        return self.table.drain(test_id)

    def enter(
        self,
        thread_id,
        frame_id,
        callee_file,
        callee_line,
        caller_frame_id,
        caller_file,
        caller_line,
    ):
        """A Python frame began, or a generator frame resumed.

        `caller_frame_id` and `caller_file` describe `frame.f_back` — the interpreter's own answer to
        who the caller is — and are `None` when there is no Python caller.
        """
        if thread_id != self.thread_id:
            self._count("other_thread")
            test = self.current_test
            if test is not None:
                callee = self.scope.relative(callee_file)
                if callee is not None:
                    self.table.observe_unsupported(test, THREADS, callee, callee_line)
            return
        top = self._stack[-1] if self._stack else None
        # Pushed before any decision about whether the edge is recordable: the stack mirrors the
        # interpreter's, and a frame omitted from it because its path was uninteresting would make
        # the next frame's caller check compare against the wrong entry.
        self._stack.append(frame_id)

        test = self.current_test
        if test is None:
            self._count("no_current_test")
            return
        callee = self.scope.relative(callee_file)
        if callee is None:
            self._count("callee_outside_root")
            return
        if not _is_line(callee_line):
            self._count("bad_line_number")
            return
        if caller_frame_id is None or caller_file is None:
            self._count("caller_not_observed")
            self.table.observe_unsupported(test, NATIVE_FRAMES, callee, callee_line)
            return
        caller = self.scope.relative(caller_file)
        if caller is None:
            self._count("caller_outside_root")
            return
        if not _is_line(caller_line):
            self._count("bad_line_number")
            return
        if top is None or top != caller_frame_id:
            self._count("caller_not_observed")
            self.table.observe_unsupported(test, NATIVE_FRAMES, callee, callee_line)
            return
        self.table.observe(test, caller, caller_line, callee, callee_line)

    def exit(self, thread_id, frame_id):
        """A Python frame returned, yielded, or was unwound by an exception."""
        if thread_id != self.thread_id:
            return
        stack = self._stack
        if stack and stack[-1] == frame_id:
            stack.pop()
            return
        if frame_id in stack:
            # Events were missed above this frame. Unwinding to it keeps the stack honest, so the
            # next caller check compares against a real frame; the alternative is a stack that
            # silently describes a shape the interpreter left behind long ago.
            index = len(stack) - 1 - stack[::-1].index(frame_id)
            del stack[index:]
            self._count("stack_resynchronised")
            return
        self._count("unbalanced_exit")

    def depth(self):
        """How many frames the tracer believes are executing. Test and diagnostic use only."""
        return len(self._stack)


def _is_line(value):
    return isinstance(value, int) and not isinstance(value, bool) and value >= 1
