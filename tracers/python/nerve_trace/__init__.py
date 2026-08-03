"""`nerve-trace-python` — the reference producer of `nerve-trace/v1` artifacts.

This package is **not part of the Nerve product**. It is not a crate, it is not linked into any
Nerve binary, and `nerve` cannot start it. `docs/plans/slice-11b-python-tracer.md` §1 states the
boundary and `crates/nerve-cli/tests/no_tracer_reference.rs` enforces it: no Rust source under
`crates/*/src/**` may so much as name this package, because product code that knows the tracer's
name is one step from knowing how to launch it, and
`crates/nerve-cli/tests/no_subprocess.rs` exists to refuse exactly that ("no test runners").

The direction of travel is one-way. A user adds this to *their* pytest invocation:

    pytest -p nerve_trace.pytest_plugin --nerve-trace=trace/run.jsonl

their tests run in their process, an artifact appears on disk, and later — separately — they may
point `nerve trace import` at it. Nerve never runs a test suite.

# Standard library only

There are no third-party dependencies and there is nothing to record in
`third_party/LICENSES.md`. Python 3.9 can import every module here; the `sys.monitoring` backend
touches `sys.monitoring` only inside function bodies, so importing it on a runtime that has no
such attribute is harmless.

# What is recorded, and what cannot be

A record names a **file and a line** on each side of one call, and the test that was executing.
No argument value, no return value, no local, no exception value, no source text, and no symbol
*name* — resolving the name is Nerve's job (a name from the tracer would be a second opinion
competing with the index), and a trace that captured arguments would capture credentials.

That is not a promise, it is a property, asserted three ways
(`docs/plans/slice-11b-python-tracer.md` §4): the artifact contract has nowhere to put a value,
no code path here reads one, and `tests/test_no_capture.py` scans this package for the *capability*
— `f_locals`, `co_varnames`, `pickle` and their relatives must not appear at all. The scan fails on
the addition of the capability rather than on a behaviour someone remembered to test.

# Attribution

A nested call belongs to its **real caller**, never to the test. For a stack
`test_basic -> parse -> tokenize` the artifact carries `parse -> tokenize`, and `test_basic`
appears only in `test_id`. See `frames.FrameTracker` for how, and why a callee whose caller frame
the tracer never observed is dropped rather than attributed to whatever is on top of the stack.
"""

from __future__ import annotations

from .backend import Backend, select_backend
from .frames import DECLARED_LIMITATIONS, EdgeTable, FrameTracker
from .paths import PathScope
from .record import (
    FORMAT,
    FORMAT_VERSION,
    PRODUCER,
    TEST_FRAMEWORK,
    VERSION,
    ArtifactWriter,
    build_header,
)
from .repository import git_commit, root_name

#: The artifact contract's own constants live in `record`, which owns the contract; they are
#: re-exported here so that `nerve_trace.VERSION` is the one obvious place to read the producer
#: version from.
__all__ = [
    "ArtifactWriter",
    "Backend",
    "DECLARED_LIMITATIONS",
    "EdgeTable",
    "FORMAT",
    "FORMAT_VERSION",
    "FrameTracker",
    "PRODUCER",
    "PathScope",
    "TEST_FRAMEWORK",
    "VERSION",
    "build_header",
    "git_commit",
    "root_name",
    "select_backend",
]
