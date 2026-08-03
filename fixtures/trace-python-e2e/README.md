# trace-python-e2e fixture

One artifact, and it is the only one in `fixtures/` that was **not** written by hand.

`produced.jsonl` is the canonicalised output of a genuine `pytest` run over `fixtures/trace-basic`
under `tracers/python/nerve_trace`. `scripts/trace_python_e2e.sh` regenerates it and **diffs**, so it
cannot go stale silently; `--update` is the only way it should ever change.

## Why it exists next to four hand-written artifacts

`fixtures/trace-basic/README.md` records why *those* are written by hand, and the reasoning holds: a
real tracer cannot emit a traversal path, malformed UTF-8 or a prompt-injection payload, so generating
them would make the security half of Slice 11a untestable.

The converse is what this fixture is for. A hand-written artifact cannot demonstrate that a producer
exists, that it agrees with the reader, or that it gets the attribution right on a tree nobody wrote
the expectations for. `crates/nerve-index/tests/trace_produced.rs` parses this file on every
`cargo test` and asserts **zero refusals**, plus the load-bearing property — that
`test_basic → parse → tokenize` is recorded as `src/parse.py → src/lex.py` and that no test function
is ever the caller of anything in `src/lex.py`.

The two fixtures agreeing is itself the interesting result: every call site in `produced.jsonl` that
`bound.jsonl` also describes carries the same lines and the same counts, and a human wrote one of them.

## What is canonicalised, and what is compared verbatim

Five header fields and one record field are volatile and are replaced with placeholders:

| field | placeholder | why |
|---|---|---|
| `run_id` | `__RUN_ID__` | a fresh UUID per run, by contract |
| `started_at`, `completed_at` | `__TIMESTAMP__` | wall clock |
| `platform` | `__PLATFORM__` | describes the machine |
| `runtime_version` | `__RUNTIME_VERSION__` | describes the interpreter |
| `worker` | `pid:__PID__/thread:MainThread` | carries a process id |

**Everything else is compared byte for byte**: every record, every count, every line number, the
repository name, the declared limitations, and the completion state. Those are what a change to the
tracer would move.

## `content_merkle` is null; `git_commit` is a sentinel

`content_merkle` is null in every artifact this producer writes. The merkle is Nerve's own digest of
the tree it indexed, and a producer that computed one would be reimplementing an internal of the
product it is deliberately not part of.

`git_commit` is **forty zeros**, and that is a canonicalisation rather than a fact. The script makes
the traced copy a real git repository before pytest runs, so a genuine run declares a genuine commit —
but a commit object carries its own timestamp, so the id is fresh every run and the diff would fail
every time. It is therefore replaced.

**Forty zeros rather than a `__TOKEN__`, and the difference is not cosmetic.** `git_commit` is the one
canonicalised field whose *shape* the reader validates: `optional_hex_field` requires exactly 40
lowercase hex characters, so a token is refused as `header-invalid` and the artifact is rejected whole.
Measured while this fixture was being built: a `__GIT_COMMIT__` token failed **five of the six tests**
in `crates/nerve-index/tests/trace_produced.rs`. A placeholder must satisfy its own field's contract,
or the fixture stops being an artifact and becomes a file that merely looks like one.

So importing *this fixture* would report **`stale`** — it names a tree that is not yours, which is the
correct answer for a canonicalised artifact.

The binding is therefore measured where a real value exists: `scripts/trace_python_e2e.sh` imports the
**raw** artifact into a real index and asserts **`binding: bound`** with zero refusals. That is plan
§7.1, and it was reported as structurally unreachable before the script learned to `git init` the
traced copy. All three binding values are additionally measured against hand-written headers in
`crates/nerve-index/tests/trace.rs`.

## The one known sensitivity

Comprehensions became inline in CPython 3.12, so an older interpreter produces additional
`<listcomp>` frames and legitimately different records. This artifact was produced by
**CPython 3.14.6 on darwin-arm64 with pytest 9.1.1**. A diff failure on a different interpreter is a
real difference, not a bug, and the script's failure message says so.
