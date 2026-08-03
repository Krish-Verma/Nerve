# Slice 11b — the reference Python tracer

2026-08-03, after 11a and 11a-i. 11a built the *consumer*: `nerve trace import` reads a
`nerve-trace/v1` artifact and Nerve runs nothing. 11b builds the *producer* — the thing that actually
watches a test run — and it runs in the **user's** process, never in Nerve's.

That sentence is the whole architecture of this slice, so it is stated before anything else.

---

## 1. What this is, and what it is not

`tracers/python/nerve_trace/` is a Python package that lives in this repository and is **not part of
the Nerve product**. It is not a crate, it is not linked, it is not invoked by any Nerve binary, and
`nerve` cannot start it. A user adds it to their own pytest invocation, their tests run, and an
artifact appears on disk. Then, separately and later, they may point `nerve trace import` at it.

The boundary is not decoration. `crates/nerve-cli/tests/no_subprocess.rs` scans `crates/*/src/**` and
its module documentation names *"no test runners"* as what it exists to refuse. A Python package
inside Nerve's repository is exactly the thing that could quietly erode that, so:

- **No Rust code may reference `tracers/`.** Asserted by test — a scan of `crates/*/src/**` for the
  string `nerve_trace`, `tracers/` and `pytest`. If Nerve's product code ever learns the tracer's
  name, it is one step from learning how to launch it.
- **`nerve trace-tests` still does not exist**, and this slice does not create it.
- The package ships **no dependencies**: standard library only. Nothing to record in
  `third_party/LICENSES.md`, and nothing for a user to install beyond the package itself.

### Non-goals, explicitly

No JavaScript tracer. No sampling. No flakiness detection. No coverage. No `nerve affected` — still
refused by ADR-0008. No runtime (non-test) tracing, though `RUNTIME_CALL_TRACE` exists in the
vocabulary for a later slice. No unittest/nose/tox integration: pytest only, and the artifact's
`test_framework` field is what makes a second framework additive rather than a rewrite.

## 2. Why a pytest plugin, and not a bare tracer

The obvious design is a `sys.monitoring` callback and nothing else. It cannot work, and the reason is
worth recording because it explains a field in the 11a contract.

`sys.monitoring` reports *code objects*. It has no idea which test is running — that is a fact only the
test framework knows. The `nerve-trace/v1` contract requires `test_id` on every record, and 11a's
`environment.runs[].tests` is keyed on it. So per-test attribution requires the framework's
cooperation, which is precisely why the header carries `test_framework` rather than pretending the
tracer is framework-agnostic.

Therefore: a pytest plugin that owns the run, using `pytest_runtest_protocol` to bracket each test
(setting the current `test_id`) and `pytest_sessionstart` / `pytest_sessionfinish` to open and close
the artifact. The monitoring backend is a separate module that knows nothing about pytest, so a second
framework later adds a plugin rather than a tracer.

Invocation is the user's:

```
pytest -p nerve_trace.pytest_plugin --nerve-trace=trace/run.jsonl
```

Absent the flag, the plugin is inert. **A plugin that traces by merely being installed would be a
plugin that changed behaviour without being asked**, and this one costs real overhead.

## 3. Two backends, and the version boundary is measured not guessed

| Python | backend | why |
|---|---|---|
| ≥ 3.12 | `sys.monitoring` | the supported mechanism, with per-event opt-in and far lower overhead |
| < 3.12 | `sys.settrace` | `sys.monitoring` does not exist; `settrace` does, back to antiquity |

`sys.monitoring` was added in 3.12 (PEP 669). The selection is `hasattr(sys, "monitoring")`, not a
version tuple: a feature check cannot be wrong about a build that has the attribute and reports an
unexpected version.

**Both backends are held to one interface** and one test suite runs against both, because a fallback
tested only on the machine that does not need it is a fallback nobody has run. On 3.12+ the settrace
backend is exercised by selecting it explicitly.

`settrace` and a profiler cannot coexist with another tracer. If `sys.gettrace()` already returns
something, the plugin **refuses to install and says so** rather than displacing it — displacing
`coverage.py` silently, in a repository that also uses Nerve's coverage ingestion, would break the
user's coverage run to produce a trace they did not prioritise. The refusal is reported as a
`producer_limitations` entry so the artifact is honest about being absent rather than merely empty.

## 4. No argument or return value is *capturable*

Not "is not captured" — **cannot be**. The distinction is the point, because "we don't record it" is a
promise and "there is no code path that could" is a property.

Three independent layers, because any one of them alone is a promise:

1. **The contract has nowhere to put one.** `nerve-trace/v1`'s record keys are fixed and none of them
   holds a value; `crates/nerve-index/src/trace.rs`'s `RECORD_KEYS` is a closed list. An artifact
   carrying `{"args": …}` would have that key counted as `record-unknown-key` and dropped. So even a
   malicious producer cannot get a value *into* Nerve.
2. **The tracer never reads one.** With `sys.monitoring`, `PY_START` hands over a code object and an
   offset — arguments are not on offer. With `sys.settrace` the callback *does* receive a frame, whose
   `f_locals` holds every argument, and the `'return'` event receives the return value as `arg`. So
   the settrace backend is where this could go wrong, and it is where the test points.
3. **A source scan asserts the absence, in both Python and Rust.** `f_locals`, `f_globals`,
   `f_builtins`, `co_varnames`, `co_consts`, `getargvalues`, `getvalue`, `pickle`, `repr(`, `format(`
   must not appear in `tracers/python/nerve_trace/**`. The `'return'` handler must take its value
   parameter as `_arg` and never name it otherwise. This is the same technique as
   `the_new_trace_modules_create_no_process_and_open_no_socket`, and it works for the same reason: it
   fails on the *addition of the capability*, not on a behaviour someone remembered to test.

What the tracer records per frame is exactly: `co_filename`, a line number, and — for the callee only
— nothing else. No function name, because the *name* is Nerve's job to resolve from the line, and a
name the tracer supplied would be a second opinion competing with the index (the 5c mapping must not
grow a rival). No `self`, no class, no module globals, no `__doc__`.

**Paths are relativised in the tracer and never absolutised.** An artifact naming
`/Users/someone/project/src/x.py` would leak a filesystem layout to whoever reads the artifact, and
11a refuses absolute paths anyway (`path-refused`). A frame outside the repository root is dropped and
counted, not emitted with `..`.

## 5. Attribution: the 11a §2.1 correction, enforced at the producer

11a's load-bearing correction was that a nested call belongs to its **real caller**, never to the test.
The producer must not undo it. For the stack `test_basic → parse → tokenize`, the tracer emits
`parse → tokenize`, and `test_basic` appears only in `test_id`.

This means the tracer maintains its own frame stack: `sys.monitoring`'s `PY_START` says *this code
began*, not *from where*. The stack is the tracer's, one per thread, and:

- a callee whose caller frame is not on the stack (the first frame of a thread, a native caller, a
  generator resumed from elsewhere) is **dropped and counted**, never attributed to whatever is on top;
- `threads`, `native-frames` and `async-continuations` are already `producer_limitations` members in
  11a's vocabulary, and this is what fills them in;
- edges are **counted, not listed**: the record's `count` field is how the artifact stays bounded when
  a loop calls one function a million times, and `MAX_RECORDS` bounds the rest.

## 6. Verification, and the honest problem with it

`pytest` is **not installed** on the development machine, and `cargo test --workspace` must pass on a
machine that has neither pytest nor Python. Three layers, so that neither hermeticity nor realism is
sacrificed to the other:

| layer | runner | hermetic? | what it gates |
|---|---|---|---|
| tracer unit tests | `python3 -m unittest` (stdlib) | yes — needs Python, not pytest | both backends, the frame stack, path relativisation, the record writer, truncation |
| non-capturability scan | both a Python test and a Rust test | **yes** — the Rust one needs nothing | that no capture code exists, from either side of the boundary |
| pytest end to end | `scripts/trace_python_e2e.sh` | **no** — needs pytest in a venv | that a real pytest run over a real repository produces an artifact `nerve trace import` accepts |

The end-to-end run's **output artifact is committed as a fixture**, and a Rust integration test imports
that fixture and asserts the edge set. That makes the ingestion of a genuinely-produced artifact
hermetic on every `cargo test`, which is the property that matters.

**The obvious rot: a committed artifact goes stale silently when the tracer changes.** So
`scripts/trace_python_e2e.sh` regenerates and **diffs against the committed fixture**, failing on any
difference. It is not run by `cargo test` — it cannot be — so the report for this slice must state
whether it was run, and no acceptance document may describe it as automated.

Installing pytest into a throwaway venv requires network. That is a **development** action, not
product behaviour: `no_network.rs` scans product source, the offline-first invariant in `CLAUDE.md` §2
is about `init`/`index`/`status`/`search`/query paths, and nothing this slice adds is reachable from
any of them. Recorded here rather than left implicit.

## 7. Acceptance criteria

1. `pytest -p nerve_trace.pytest_plugin --nerve-trace=…` over a real fixture repository produces a
   `nerve-trace/v1` artifact that `nerve trace import` accepts with `binding: bound` and zero
   refusals other than the ordinary line-mapping ones.
2. The edge set from the real run contains a **nested** call attributed to its real caller, and does
   not contain that call attributed to the test.
3. Both backends pass one suite. The settrace backend is exercised on 3.12+ by explicit selection.
4. The non-capturability scan passes and fails when a capture is introduced (mutation probe).
5. No Rust source names the tracer, pytest, or `tracers/` (test).
6. `nerve trace-tests` does not exist (already asserted by the CLI command-surface test).
7. `Cargo.lock` is unchanged. No Python dependency is added.
8. The full gate passes, and `no_subprocess.rs` / `no_network.rs` are byte-untouched.
