"""The pytest integration: the only thing here that knows what a test is.

# Why a plugin at all

`docs/plans/slice-11b-python-tracer.md` §2. A tracing mechanism reports *code objects*; it has no idea
which test is running, because that is a fact only the test framework holds. The `nerve-trace/v1`
contract requires `test_id` on every record and `nerve trace import` keys its evidence on it, so
per-test attribution needs the framework's cooperation — which is exactly why the header carries
`test_framework` instead of pretending the tracer is framework-agnostic.

Nothing else in this package imports this module, and this module imports nothing from pytest. The
hooks are found by name and the plugin object is registered with the plugin manager, so
`nerve_trace` stays importable — and unit-testable — on a machine with no pytest installed, which is
the machine this was written on.

# Inert without the flag

    pytest -p nerve_trace.pytest_plugin --nerve-trace=trace/run.jsonl

Absent `--nerve-trace`, `pytest_configure` registers nothing and no hook of this module's ever runs.
**A plugin that traced by merely being installed would be a plugin that changed behaviour without
being asked**, and this one costs real overhead.

# Bracketing on `logstart`/`logfinish`

The plan names `pytest_runtest_protocol`; `pytest_runtest_logstart` and `pytest_runtest_logfinish`
bracket the same protocol — setup, call and teardown — and hand over the node id directly, without
needing `pytest.hookimpl(hookwrapper=True)` and therefore without importing pytest. Fixture code
consequently belongs to the test it set up, which is the attribution a reader would expect.

Collection is deliberately *not* traced: the backend is enabled per test, so a module body executing
at import time produces no events at all rather than events with no test to attribute them to. That is
a real limitation of the design and is stated here rather than discovered later.

# Failure is inert, never fatal

Every failure path leaves the user's test run alone: a root with no usable name, an unwritable
artifact path, another tool already holding the tracing hook. The worst outcome is an artifact that
says `partial` and explains itself, or no artifact and a message on the terminal. A tracer that broke
a test suite to report on it would be worse than no tracer.
"""

from __future__ import annotations

import datetime
import os
import platform
import sys
import threading
import uuid

from .backend import select_backend
from .frames import DECLARED_LIMITATIONS, EdgeTable, FrameTracker
from .paths import PathScope
from .record import ArtifactWriter, build_header, is_clean_string
from .repository import git_commit, root_name

#: pytest's exit statuses that mean the suite did not reach its end: `INTERRUPTED` and
#: `INTERNAL_ERROR`. A run that merely had failing tests reached the end of the suite and is
#: `complete`; conflating the two would label every red build's evidence partial.
_UNFINISHED_EXIT_STATUSES = (2, 3)


def pytest_addoption(parser):
    group = parser.getgroup("nerve-trace", "nerve-trace-python (test-observed call tracing)")
    group.addoption(
        "--nerve-trace",
        action="store",
        dest="nerve_trace_path",
        default=None,
        metavar="PATH",
        help="Write a nerve-trace/v1 artifact to PATH. Without this, the plugin does nothing.",
    )
    group.addoption(
        "--nerve-trace-root",
        action="store",
        dest="nerve_trace_root",
        default=None,
        metavar="PATH",
        help="Repository root to relativise frames against. Defaults to pytest's rootdir.",
    )
    group.addoption(
        "--nerve-trace-backend",
        action="store",
        dest="nerve_trace_backend",
        default="auto",
        choices=("auto", "monitoring", "settrace"),
        help="Tracing mechanism. `auto` picks sys.monitoring when the interpreter has it.",
    )


def pytest_configure(config):
    path = config.getoption("nerve_trace_path", default=None)
    if not path:
        return
    requested_root = config.getoption("nerve_trace_root", default=None)
    root = requested_root or _rootdir(config)
    name = root_name(root)
    if name is None:
        _warn("--nerve-trace: the repository root has no usable name; not tracing")
        return
    try:
        plugin = _NerveTracePlugin(
            path=path,
            root=os.path.realpath(root),
            root_name_value=name,
            preference=config.getoption("nerve_trace_backend", default="auto"),
        )
    except OSError as error:
        _warn(f"--nerve-trace: cannot write {path}: {error}; not tracing")
        return
    config.pluginmanager.register(plugin, "nerve-trace-runner")


def _rootdir(config):
    rootpath = getattr(config, "rootpath", None)
    if rootpath is not None:
        return str(rootpath)
    return str(config.rootdir)


def _warn(message):
    sys.stderr.write(f"{message}\n")


def _now():
    """RFC3339, second resolution, UTC.

    `datetime.timezone.utc` rather than `datetime.UTC`, which is 3.11 and later; this package must
    import on 3.9.
    """
    moment = datetime.datetime.now(datetime.timezone.utc)
    return moment.strftime("%Y-%m-%dT%H:%M:%SZ")


def _worker():
    """`pid:<pid>/thread:<name>`, matching the shape the contract's example uses."""
    name = threading.current_thread().name
    candidate = f"pid:{os.getpid()}/thread:{name}"
    return candidate if is_clean_string(candidate) else f"pid:{os.getpid()}"


class _NerveTracePlugin:
    """Owns the artifact, the tracker and the backend for one pytest session."""

    def __init__(self, path, root, root_name_value, preference):
        self.path = path
        self.root = root
        self.scope = PathScope(root)
        self.table = EdgeTable(_worker())
        self.tracker = FrameTracker(self.scope, self.table, threading.get_ident())
        self.backend = select_backend(self.tracker, preference)
        self.started_at = _now()
        limitations = list(DECLARED_LIMITATIONS)
        self.refusal = self.backend.attach()
        if self.refusal is not None:
            limitations.append(self.refusal)
            _warn(
                "--nerve-trace: another tool holds the interpreter's tracing hook; "
                "recording the run as partial and tracing nothing"
            )
        header = build_header(
            repository_root_name=root_name_value,
            run_id=f"nerve-trace-python-{uuid.uuid4().hex}",
            runtime=platform.python_implementation().lower(),
            runtime_version=platform.python_version(),
            platform_name=f"{sys.platform}-{platform.machine()}",
            started_at=self.started_at,
            producer_limitations=limitations,
            git_commit=git_commit(root),
        )
        try:
            self.writer = ArtifactWriter(path, header)
        except OSError:
            # The mechanism was acquired a moment ago and there is now nothing to write it to.
            # Releasing it here is what keeps a failed start from leaving the interpreter's tracing
            # hook held by a plugin that will never be registered.
            self.backend.detach()
            raise

    # -- per-test bracketing ---------------------------------------------------------------------

    def pytest_runtest_logstart(self, nodeid, location):
        self.tracker.begin_test(nodeid)
        self.backend.enable()

    def pytest_runtest_logfinish(self, nodeid, location):
        self.backend.disable()
        self.writer.write(self.tracker.end_test())

    # -- session ---------------------------------------------------------------------------------

    def pytest_sessionfinish(self, session, exitstatus):
        self.backend.detach()
        # Anything the tracker still holds belongs to no bracketed test — a generator finalised after
        # its test ended, say. It is written rather than dropped: the records carry their own
        # `test_id`, so nothing is being guessed at.
        self.writer.write(self.table.drain())
        state, reason = self._outcome(exitstatus)
        completed_at = _now()
        self.writer.close(state, completed_at, reason)

    def _outcome(self, exitstatus):
        if self.refusal is not None:
            return "partial", "another tool held the interpreter's tracing hook"
        if self.writer.capped:
            return "partial", "the producer stopped at the reader's artifact ceiling"
        try:
            status = int(exitstatus)
        except (TypeError, ValueError):
            status = 0
        if status in _UNFINISHED_EXIT_STATUSES:
            return "partial", "the run did not reach the end of the suite"
        return "complete", None

    def pytest_terminal_summary(self, terminalreporter, exitstatus, config):
        counts = self.tracker.counts
        lines = [
            f"nerve-trace: {self.writer.records_written} records via {self.backend.name}"
            f" -> {self.path}",
            f"nerve-trace: {self.scope.outside_root} files outside the repository root were dropped",
        ]
        interesting = {name: value for name, value in counts.items() if value}
        if interesting:
            detail = ", ".join(f"{name}={value}" for name, value in sorted(interesting.items()))
            lines.append(f"nerve-trace: frames not recorded: {detail}")
        if self.writer.capped:
            lines.append("nerve-trace: the reader's artifact ceiling was reached; run is partial")
        for line in lines:
            terminalreporter.write_line(line)
