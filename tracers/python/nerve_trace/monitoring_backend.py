"""The `sys.monitoring` backend (PEP 669, CPython 3.12+).

`sys.monitoring` is touched only inside function bodies, never at import time, so this module imports
cleanly on Python 3.9 — which matters because `nerve_trace/__init__.py` imports it unconditionally and
a package that cannot be imported on an old interpreter cannot even report that it needs a newer one.

# Five events, and why each is needed

`PY_START` and `PY_RESUME` push; `PY_RETURN`, `PY_YIELD` and `PY_UNWIND` pop. The generator half is
not optional: a generator's frame starts once and then suspends and resumes repeatedly, so a tracer
watching only `PY_START`/`PY_RETURN` would leave the generator's frame on its stack while execution
continued in the consumer — and every call the consumer then made would fail the caller check and be
dropped. Measured on CPython 3.14, `for value in gen()` over two yields produces
`START, YIELD, RESUME, YIELD, RESUME, RETURN`: three pushes and three pops, which balance.

`sys.settrace` reports the same shape as three `'call'`/`'return'` pairs, so both backends record the
same edge with the same count. That agreement is the reason one test suite can hold them to one set of
expectations.

# Where the frame comes from

`PY_START` hands over a code object and an instruction offset — no frame, and therefore no argument
values on offer. The frame is obtained with `sys._getframe(1)` **called directly in the callback
body**, which was measured to be the event's own frame for all five events; going through a helper
would add a frame and silently return the wrong one. Only `f_back`, `f_lineno` and `f_code`'s filename
and first line are read from it. Nothing else exists to read: see `tests/test_no_capture.py`.

# Re-entrancy

Measured on CPython 3.14: calling a Python function from inside a monitoring callback produces no
further events for this tool, so the callbacks may call ordinary methods without recursion. That is
the same protection `sys.settrace` gives by suspending tracing inside the trace function.

# One thread

Monitoring events arrive from every thread, including the interpreter's own `threading` bootstrap.
The tracker turns away everything but the thread pytest called it on — see `frames.FrameTracker` for
why that is a correctness decision and not a simplification.
"""

from __future__ import annotations

import sys
import threading

from .frames import PROFILER_CONTENTION


class MonitoringBackend:
    """Feeds a `FrameTracker` from `sys.monitoring`."""

    name = "sys.monitoring"

    #: `sys.monitoring.PROFILER_ID`. A trace of which code called which code is a profiler's
    #: business, and taking `DEBUGGER_ID` or `COVERAGE_ID` would contend with the tools most likely
    #: to be running alongside — a debugger the user is sitting in, or the `coverage.py` run whose
    #: output Nerve's own coverage ingestion reads.
    TOOL_ID = 2

    def __init__(self, tracker):
        self._tracker = tracker
        self._attached = False
        self._enabled = False

    def _event_mask(self):
        events = sys.monitoring.events
        return (
            events.PY_START
            | events.PY_RESUME
            | events.PY_RETURN
            | events.PY_YIELD
            | events.PY_UNWIND
        )

    def attach(self):
        monitoring = sys.monitoring
        if monitoring.get_tool(self.TOOL_ID) is not None:
            return PROFILER_CONTENTION
        try:
            monitoring.use_tool_id(self.TOOL_ID, "nerve-trace-python")
        except ValueError:
            return PROFILER_CONTENTION
        events = monitoring.events
        monitoring.register_callback(self.TOOL_ID, events.PY_START, self._on_enter)
        monitoring.register_callback(self.TOOL_ID, events.PY_RESUME, self._on_enter)
        monitoring.register_callback(self.TOOL_ID, events.PY_RETURN, self._on_exit)
        monitoring.register_callback(self.TOOL_ID, events.PY_YIELD, self._on_exit)
        monitoring.register_callback(self.TOOL_ID, events.PY_UNWIND, self._on_exit)
        self._attached = True
        return None

    def enable(self):
        if not self._attached or self._enabled:
            return
        sys.monitoring.set_events(self.TOOL_ID, self._event_mask())
        self._enabled = True

    def disable(self):
        if not self._attached or not self._enabled:
            return
        sys.monitoring.set_events(self.TOOL_ID, 0)
        self._enabled = False

    def detach(self):
        if not self._attached:
            return
        self.disable()
        monitoring = sys.monitoring
        events = monitoring.events
        for event in (
            events.PY_START,
            events.PY_RESUME,
            events.PY_RETURN,
            events.PY_YIELD,
            events.PY_UNWIND,
        ):
            monitoring.register_callback(self.TOOL_ID, event, None)
        monitoring.free_tool_id(self.TOOL_ID)
        self._attached = False

    def _on_enter(self, code, _offset):
        """`PY_START` or `PY_RESUME`: a Python frame began or a generator frame resumed."""
        frame = sys._getframe(1)
        caller = frame.f_back
        if caller is None:
            self._tracker.enter(
                threading.get_ident(),
                id(frame),
                code.co_filename,
                _first_line(code),
                None,
                None,
                None,
            )
            return None
        self._tracker.enter(
            threading.get_ident(),
            id(frame),
            code.co_filename,
            _first_line(code),
            id(caller),
            caller.f_code.co_filename,
            caller.f_lineno,
        )
        return None

    def _on_exit(self, code, _offset, _arg):
        """`PY_RETURN`, `PY_YIELD` or `PY_UNWIND`.

        `_arg` is the returned value, the yielded value or the exception. It is named `_arg` and is
        never read, which is the whole of this tracer's relationship with values.
        """
        self._tracker.exit(threading.get_ident(), id(sys._getframe(1)))
        return None


def _first_line(code):
    """The line the callee's code begins on, clamped to the 1-based line numbers Nerve accepts.

    `co_firstlineno` rather than the frame's current line: it is the `def`, which falls inside the
    symbol extent Nerve will map the record onto, and it does not move as the function executes.
    """
    first = code.co_firstlineno
    return first if isinstance(first, int) and first >= 1 else 1
