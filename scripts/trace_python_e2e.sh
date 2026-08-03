#!/usr/bin/env bash
#
# The one layer of Slice 11b's verification that is not hermetic: a real pytest run, under the real
# tracer, over a real repository, diffed against a committed artifact.
#
# `docs/plans/slice-11b-python-tracer.md` §6 states the split and the honest problem with it. The
# tracer's unit tests need Python and not pytest, so `cargo test` and `python3 -m unittest` can both
# stay hermetic; but nothing hermetic can show that the pytest hooks fire in the right order on a
# tree nobody wrote the expectations for. That is this script, and it is **not run by `cargo test`**
# — it installs pytest into a throwaway virtual environment, which needs the network.
#
# Installing pytest is a *development* action, not product behaviour. `no_network.rs` scans product
# source; the offline-first invariant in `CLAUDE.md` §2 is about `init`, `index`, `status`, `search`
# and the query paths, and nothing this script touches is reachable from any of them.
#
# ## The rot this script exists to prevent
#
# A committed artifact goes stale silently when the producer changes. So the run is regenerated and
# **diffed**, and any difference fails. `--update` rewrites the committed artifact, and is the only
# way it should ever change.
#
# ## Why the diff is on a canonicalised copy
#
# A byte-exact diff is impossible and pretending otherwise would mean a fixture that fails every
# run: `run_id` is a fresh UUID, `started_at` and `completed_at` are wall-clock, `platform` and
# `runtime_version` describe the machine, and `worker` carries a pid. Those seven fields are replaced
# with placeholders. **Everything that describes the trace itself — every record, every count, every
# line number, the repository name, the limitations, the completion state — is compared verbatim**,
# which is the part a change to the tracer would move.
#
# The remaining known sensitivity is the Python version: comprehensions became inline in 3.12, so an
# older interpreter produces extra `<listcomp>` frames and legitimately different records. The
# committed artifact records which interpreter produced it and the failure message says so.
#
# Usage:
#   scripts/trace_python_e2e.sh            # regenerate and diff; non-zero on any difference
#   scripts/trace_python_e2e.sh --update   # accept the current output as the fixture

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
TRACED_FIXTURE="$REPO_ROOT/fixtures/trace-basic"
COMMITTED="$REPO_ROOT/fixtures/trace-python-e2e/produced.jsonl"
WORK="${TMPDIR:-/tmp}/nerve-trace-python-e2e"
PYTHON="${PYTHON:-python3}"

UPDATE=0
if [ "${1:-}" = "--update" ]; then
  UPDATE=1
elif [ -n "${1:-}" ]; then
  echo "usage: $(basename "$0") [--update]" >&2
  exit 64
fi

# The traced copy's directory name becomes `repository_root_name` in the header, so it is fixed
# rather than a `mktemp` suffix. Re-runnable: the previous run's directory is removed first, and the
# trap removes this one however the script ends.
rm -rf "$WORK"
mkdir -p "$WORK"
trap 'rm -rf "$WORK"' EXIT

TRACED="$WORK/trace-basic"
cp -R "$TRACED_FIXTURE" "$TRACED"
# The four hand-written artifacts are inputs to `cargo test`, not to a real run. Removing them makes
# it plain that what comes out of this script was produced rather than copied.
rm -rf "$TRACED/trace"

# Make the traced copy a git repository, **before** pytest runs.
#
# Added by the orchestrator during review. Without this the tracer has no commit to declare, so
# `content_merkle` being null (correctly — computing Nerve's merkle would mean reimplementing an
# internal of the product this package is deliberately not part of) leaves `TraceBinding::decide`
# nothing to compare and every real run reports `unverified`. That is honest but it is not the
# criterion: `docs/plans/slice-11b-python-tracer.md` §7.1 asks for `binding: bound` from a real run,
# and the implementation reported it as structurally unreachable. It is reachable — it just needs the
# traced tree to be a repository Nerve indexed at the same commit, which is three commands.
#
# `-c` flags rather than global config, and a fixed identity, so this works on a machine with no
# `user.email` set and never reads the developer's own identity into a fixture.
(
  cd "$TRACED"
  git init --quiet -b main
  git -c user.name="nerve-e2e" -c user.email="nerve-e2e@localhost" add -A
  git -c user.name="nerve-e2e" -c user.email="nerve-e2e@localhost" \
      -c commit.gpgsign=false commit --quiet -m "traced fixture"
) || { echo "FAILED: could not make the traced copy a git repository" >&2; exit 1; }

echo "==> creating a throwaway virtual environment"
"$PYTHON" -m venv "$WORK/venv"
VENV_PYTHON="$WORK/venv/bin/python"

echo "==> installing pytest (this is the step that needs the network)"
if ! "$VENV_PYTHON" -m pip install --quiet --disable-pip-version-check pytest; then
  echo >&2
  echo "FAILED: pytest could not be installed. This script needs the network for that one step." >&2
  echo "Nothing about Nerve needs it: the tracer's own tests run under 'python3 -m unittest" >&2
  echo "discover -s tracers/python' with no third-party package at all." >&2
  exit 2
fi
"$VENV_PYTHON" -m pytest --version >&2

echo "==> running pytest under the tracer"
RAW="$WORK/raw.jsonl"
(
  cd "$TRACED"
  # `python -m pytest` rather than the `pytest` entry point, so the traced repository's own root is
  # on `sys.path` and `from src.parse import ...` resolves — the same thing a user's own invocation
  # from their repository root does.
  #
  # `--nerve-trace-root` is deliberately *not* passed: the default is pytest's rootdir, and the
  # canonicaliser asserts the header's `repository_root_name`, so the default is what is under test.
  PYTHONPATH="$REPO_ROOT/tracers/python" "$VENV_PYTHON" -m pytest \
    -p nerve_trace.pytest_plugin \
    -p no:cacheprovider \
    --nerve-trace="$RAW" \
    -q
)

echo "==> canonicalising"
CANONICAL="$WORK/canonical.jsonl"
"$PYTHON" - "$RAW" "$CANONICAL" <<'CANONICALISE'
"""Replace the seven per-machine, per-run header and worker fields, and check the contract.

Standard library only, and run under the *system* interpreter rather than the venv's, because
nothing here should need pytest to be installed to read an artifact.
"""

import json
import sys

RAW, OUT = sys.argv[1], sys.argv[2]

HEADER_KEYS = (
    "format", "format_version", "producer", "producer_version", "repository_root_name",
    "git_commit", "content_merkle", "run_id", "test_framework", "runtime", "runtime_version",
    "platform", "started_at", "completed_at", "completion_state", "partial_reason",
    "source_map_state", "producer_limitations",
)
RECORD_KEYS = (
    "test_id", "caller_file", "caller_line", "callee_file", "callee_line", "count", "worker",
    "async_context", "resolution", "unsupported_form",
)
# Volatile: a fresh identifier, two wall-clock stamps, two facts about the machine.
VOLATILE = {
    "run_id": "__RUN_ID__",
    # A fresh commit every run, because a commit object carries its own timestamp. Added when the
    # script started `git init`-ing the traced copy so that the binding could be `bound`; without it
    # here the diff would fail on every run, which is the fixture-rot this script exists to prevent.
    #
    # Forty zeros rather than a `__TOKEN__`, and the difference is not cosmetic: `git_commit` is the
    # one canonicalised field the reader **validates the shape of** — `optional_hex_field` requires
    # exactly 40 lowercase hex characters — so a token here is refused as `header-invalid` and the
    # artifact is rejected whole. A placeholder must satisfy its own field's contract, or the fixture
    # stops being an artifact. Measured: `__GIT_COMMIT__` failed five of the six tests in
    # `trace_produced.rs`.
    "git_commit": "0" * 40,
    "started_at": "__TIMESTAMP__",
    "completed_at": "__TIMESTAMP__",
    "platform": "__PLATFORM__",
    "runtime_version": "__RUNTIME_VERSION__",
}

with open(RAW, "r", encoding="utf-8") as handle:
    lines = [line for line in handle.read().split("\n") if line.strip()]

if not lines:
    sys.exit("the artifact is empty; the plugin did not run")

header = json.loads(lines[0])
if tuple(header.keys()) != HEADER_KEYS:
    sys.exit(f"the header's keys are not the contract's: {sorted(header)}")
if header["format"] != "nerve-trace" or header["format_version"] != 1:
    sys.exit("the header is not a nerve-trace/v1 header")
if header["repository_root_name"] != "trace-basic":
    sys.exit(
        "the header names "
        + str(header["repository_root_name"])
        + " rather than trace-basic: pytest's rootdir was not the traced copy"
    )
if header["completion_state"] != "complete":
    sys.exit(f"the run did not complete: {header['completion_state']} {header['partial_reason']}")
if not header["completed_at"]:
    sys.exit("a complete run must state completed_at")

for key, placeholder in VOLATILE.items():
    header[key] = placeholder

records = []
for number, line in enumerate(lines[1:], start=2):
    record = json.loads(line)
    if tuple(record.keys()) != RECORD_KEYS:
        sys.exit(f"line {number}: keys are not the contract's: {sorted(record)}")
    if record["resolution"] not in ("located", "unresolved"):
        sys.exit(f"line {number}: resolution is outside the vocabulary")
    if record["resolution"] == "unresolved" and record["unsupported_form"] is None:
        sys.exit(f"line {number}: an unresolved record with no form is a refusal, not a limitation")
    for field in ("caller_file", "callee_file"):
        value = record[field]
        if value is None:
            continue
        if value.startswith("/") or ".." in value.split("/") or "\\" in value:
            sys.exit(f"line {number}: {field} is not a clean relative path: {value}")
    if record["worker"] is not None:
        record["worker"] = "pid:__PID__/thread:MainThread"
    records.append(record)

if not records:
    sys.exit("the artifact carries no records; the tracer observed nothing")

def dump(obj):
    return json.dumps(obj, separators=(",", ":"), ensure_ascii=True)

with open(OUT, "w", encoding="utf-8", newline="\n") as handle:
    handle.write(dump(header) + "\n")
    for record in records:
        handle.write(dump(record) + "\n")

located = sum(1 for record in records if record["resolution"] == "located")
print(f"    {len(records)} records ({located} located)")
CANONICALISE

if [ "$UPDATE" -eq 1 ]; then
  mkdir -p "$(dirname "$COMMITTED")"
  cp "$CANONICAL" "$COMMITTED"
  echo "==> updated $COMMITTED"
  exit 0
fi

if [ ! -f "$COMMITTED" ]; then
  echo >&2
  echo "FAILED: there is no committed artifact at" >&2
  echo "  $COMMITTED" >&2
  echo "Run this script with --update to record the current output as the fixture." >&2
  exit 1
fi

echo "==> diffing against the committed artifact"
if ! diff -u "$COMMITTED" "$CANONICAL"; then
  echo >&2
  echo "FAILED: the tracer no longer produces the committed artifact." >&2
  echo "If the change is intended, re-run with --update and review the diff in the commit." >&2
  echo "If it is not, the tracer changed behaviour. One legitimate cause is a different Python" >&2
  echo "version: comprehensions became inline in 3.12, so an older interpreter adds <listcomp>" >&2
  echo "frames. fixtures/trace-python-e2e/README.md records which interpreter produced the fixture." >&2
  exit 1
fi
echo "==> identical: the tracer still produces the committed artifact"

# ---------------------------------------------------------------------------------------------
# The RAW artifact through Nerve's real reader. Added by the orchestrator during review.
#
# Two gaps this closes, and both are the shape of the defect Slice 11a-i was opened for:
#
#   1. **Nothing parsed the raw artifact.** The diff above compares a *canonicalised copy* against
#      the fixture, and `crates/nerve-index/tests/trace_produced.rs` parses that same canonicalised
#      copy. So five field values were replaced with placeholders and then the placeholders were
#      what got tested. The canonicaliser only touches fields the parser treats as opaque bounded
#      strings, so the risk was small — but "small" is not the standard, and a real `platform` or
#      `run_id` that violated a bound would have been refused while the fixture sailed through.
#
#   2. **Plan §7 criterion 1 — `binding: bound` from a real run** — reported as unreachable. It is
#      not; see the `git init` above.
#
# This is the only place the full ingestion path runs over a genuinely produced artifact. It cannot
# live in `cargo test`, because producing the artifact needs pytest.
echo "==> importing the RAW artifact into a real index"
NERVE="$REPO_ROOT/target/release/nerve"
if [ ! -x "$NERVE" ]; then
  echo "    building the release binary first"
  (cd "$REPO_ROOT" && cargo build --release --quiet) || {
    echo "FAILED: could not build the release binary" >&2; exit 1; }
fi

cp "$RAW" "$TRACED/raw.jsonl"
(
  cd "$TRACED"
  "$NERVE" init >/dev/null
  "$NERVE" index >/dev/null
) || { echo "FAILED: could not index the traced copy" >&2; exit 1; }

IMPORT_OUT="$WORK/import.txt"
IMPORT_STATUS=0
(cd "$TRACED" && "$NERVE" trace import raw.jsonl) >"$IMPORT_OUT" 2>&1 || IMPORT_STATUS=$?
sed 's/^/    /' "$IMPORT_OUT"

FAILURES=0

# `bound` and nothing else. `unverified` would mean the git binding silently stopped working, and
# `stale` would mean the tracer and Nerve disagree about which tree this is — both are the kind of
# regression that reads as "fine" in a summary line.
if grep -qE '^[[:space:]]*binding[[:space:]]+bound$' "$IMPORT_OUT"; then
  echo "==> binding: bound — the tracer's declared commit agrees with the index (plan §7.1)"
else
  echo "FAILED: expected 'binding bound'; the import reported otherwise" >&2
  FAILURES=$((FAILURES + 1))
fi

# Zero refusals *other than* line mapping. A module-level or comprehension frame that no symbol
# contains is the ordinary lossiness of mapping a line onto a symbol, and every real repository has
# some; anything else means the producer emitted a record the reader would not take.
if grep -qE '^[[:space:]]*(path-refused|file-not-indexed|file-unreadable|file-changed-since-index|record-invalid|record-too-large|string-too-long|invalid-utf8-line|malformed-json|nesting-too-deep|header-|artifact-too-large|other-repository|run-id-conflict)' "$IMPORT_OUT"; then
  echo "FAILED: the raw artifact produced a refusal beyond ordinary line mapping" >&2
  FAILURES=$((FAILURES + 1))
else
  echo "==> no refusal beyond ordinary line mapping (plan §7.1)"
fi

# Exit 3 is Nerve's "imported, with a material refusal". A genuinely produced artifact must not
# trigger it.
if [ "$IMPORT_STATUS" -ne 0 ]; then
  echo "FAILED: 'nerve trace import' exited $IMPORT_STATUS on a genuinely produced artifact" >&2
  FAILURES=$((FAILURES + 1))
fi

if [ "$FAILURES" -eq 0 ]; then
  echo
  echo "==> all checks passed: produced by a real pytest run, and accepted by Nerve's reader"
  exit 0
fi
