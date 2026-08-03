"""The interface both tracing mechanisms satisfy, and the choice between them.

`docs/plans/slice-11b-python-tracer.md` §3:

| Python | backend | why |
|---|---|---|
| >= 3.12 | `sys.monitoring` | the supported mechanism, with per-event opt-in and far lower overhead |
| < 3.12 | `sys.settrace` | `sys.monitoring` does not exist; `settrace` does, back to antiquity |

The selection is `hasattr(sys, "monitoring")`, not a version tuple. A feature check cannot be wrong
about a build that has the attribute and reports an unexpected version, and it is the same reason
`nerve check` prefers measuring to assuming.

**Both backends are held to one interface and one test suite**, because a fallback tested only on the
machine that does not need it is a fallback nobody has run. On 3.12+ the `settrace` backend is
exercised by asking for it explicitly, which is what `preference` is for.

# Four methods, not two, and the split is about cost

`attach` acquires the mechanism once per session and is where **contention** is detected;
`enable`/`disable` bracket a single test. Acquiring per test would be wasteful, and tracing the whole
session would mean tracing collection — during which no test is executing, so every event would be
dropped for having nothing to attribute it to. Import-time module bodies are therefore not traced at
all, which is a real limitation of the design and the honest reason
`fixtures/trace-basic/src/parse.py`'s module-level call does not appear in a produced artifact.

# Refusing to displace another tracer

`sys.settrace` and `sys.monitoring`'s tool ids are exclusive. If something already holds one, the
backend **refuses to install and says so** rather than displacing it: silently evicting
`coverage.py` in a repository that also feeds Nerve's coverage ingestion would break a run the user
prioritised in order to produce one they did not ask for. The refusal is returned as the
`profiler-contention` limitation so the artifact is honest about being absent rather than merely
empty.
"""

from __future__ import annotations

import sys


class Backend:
    """What a tracing mechanism must provide. Subclasses hold a `FrameTracker` and feed it events."""

    #: Human-readable mechanism name, reported in pytest's summary.
    name = "none"

    def attach(self):
        """Acquire the mechanism. Returns `None` on success, or a limitation form on refusal."""
        raise NotImplementedError

    def enable(self):
        """Begin delivering events. Called once per test."""
        raise NotImplementedError

    def disable(self):
        """Stop delivering events. Called once per test, and safe to call when not enabled."""
        raise NotImplementedError

    def detach(self):
        """Release the mechanism. Safe to call whether or not `attach` succeeded."""
        raise NotImplementedError


def has_monitoring():
    """Whether this interpreter has `sys.monitoring` (PEP 669, CPython 3.12)."""
    return hasattr(sys, "monitoring")


def select_backend(tracker, preference=None):
    """The backend for this interpreter, or the one asked for.

    `preference` is `"monitoring"`, `"settrace"`, or `None`/`"auto"` to choose by feature check.
    Asking for `monitoring` on an interpreter that has none raises, rather than silently falling
    back: a test that means to exercise one mechanism must fail if it exercised the other.
    """
    from .monitoring_backend import MonitoringBackend
    from .settrace_backend import SettraceBackend

    if preference in (None, "auto"):
        return MonitoringBackend(tracker) if has_monitoring() else SettraceBackend(tracker)
    if preference == "settrace":
        return SettraceBackend(tracker)
    if preference == "monitoring":
        if not has_monitoring():
            raise RuntimeError("this interpreter has no sys.monitoring")
        return MonitoringBackend(tracker)
    raise ValueError("backend preference must be auto, monitoring or settrace")
