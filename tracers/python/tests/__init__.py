"""Tests for `nerve_trace`, written against the standard library's `unittest` and nothing else.

    python3 -m unittest discover -s tracers/python -v

**Not pytest.** The tracer's own tests must run on a machine with no pytest installed — which is the
machine this was developed on — and a suite that needed the framework it integrates with could not
tell an integration failure from a missing dependency. The pytest half is exercised by
`scripts/trace_python_e2e.sh`, which builds a throwaway venv and is deliberately not part of any
`cargo test` or `unittest` run.
"""
