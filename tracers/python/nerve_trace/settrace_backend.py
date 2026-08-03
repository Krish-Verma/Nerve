"""The `sys.settrace` backend, for interpreters with no `sys.monitoring`.

`sys.settrace` is where the non-capturability property could go wrong, and it is therefore where the
test points. The callback is *handed a frame*, whose locals hold every argument, and the `'return'`
event is handed the returned value. Neither is read: the value parameter is named `_arg` in both trace
functions and is never used, and `tests/test_no_capture.py` scans this package for `f_locals`,
`co_varnames` and their relatives so that the *capability* cannot be added without failing a test.
`docs/plans/slice-11b-python-tracer.md` §4 is the argument for why an absence asserted this way is a
property rather than a promise.

# `'call'` and `'return'` are enough

Measured on CPython 3.14: a generator resumption fires `'call'` and a yield fires `'return'`, so the
same three pushes and three pops the `sys.monitoring` backend derives from
`PY_START`/`PY_RESUME`/`PY_YIELD`/`PY_RETURN` fall out of two events here. An exception propagating out
of a frame fires `'exception'` and then `'return'`, so the pop happens there too and no `PY_UNWIND`
equivalent is needed.

`f_trace_lines = False` is set on every frame, which suppresses the per-line events that would
otherwise dominate the cost. There is no equivalent switch for `'exception'`, so the local trace
function ignores that event explicitly.

# One thread, for free

`sys.settrace` installs on the calling thread only, and `threading.settrace` is deliberately not
used: the tracker records edges for one thread anyway, so registering for others would spend the
overhead to produce events it will turn away.
"""

from __future__ import annotations

import sys
import threading

from .frames import PROFILER_CONTENTION


class SettraceBackend:
    """Feeds a `FrameTracker` from `sys.settrace`."""

    name = "sys.settrace"

    def __init__(self, tracker):
        self._tracker = tracker
        self._attached = False
        self._enabled = False

    def attach(self):
        if sys.gettrace() is not None:
            # Something already holds the hook — a debugger, `coverage.py`, another profiler.
            # Displacing it would break a run the user prioritised to produce one they did not ask
            # for, so the artifact says `profiler-contention` and stays empty rather than silent.
            return PROFILER_CONTENTION
        self._attached = True
        return None

    def enable(self):
        if not self._attached or self._enabled:
            return
        sys.settrace(self._global_trace)
        self._enabled = True

    def disable(self):
        if not self._attached or not self._enabled:
            return
        sys.settrace(None)
        self._enabled = False

    def detach(self):
        self.disable()
        self._attached = False

    def _global_trace(self, frame, event, _arg):
        """A frame began, or a generator frame resumed.

        `_arg` is `None` for `'call'`; it is named and ignored here for the same reason it is in
        `_local_trace`, so that neither trace function has a parameter anyone could start reading.
        """
        if event != "call":
            return None
        frame.f_trace_lines = False
        code = frame.f_code
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
        else:
            self._tracker.enter(
                threading.get_ident(),
                id(frame),
                code.co_filename,
                _first_line(code),
                id(caller),
                caller.f_code.co_filename,
                caller.f_lineno,
            )
        return self._local_trace

    def _local_trace(self, frame, event, _arg):
        """`'return'` pops the frame; `'exception'` and anything else are ignored.

        `_arg` is the returned value on a `'return'` event and the exception triple on an
        `'exception'` event. It is never read.
        """
        if event == "return":
            self._tracker.exit(threading.get_ident(), id(frame))
        return self._local_trace


def _first_line(code):
    """The line the callee's code begins on, clamped to the 1-based line numbers Nerve accepts."""
    first = code.co_firstlineno
    return first if isinstance(first, int) and first >= 1 else 1
