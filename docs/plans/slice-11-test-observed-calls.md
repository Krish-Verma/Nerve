# Slice 11 plan — Test-observed calls, and why Nerve must not run your tests

2026-08-03. Row 11 of `docs/ROADMAP.md`. Written while Slice 10a was in implementation.

---

## 1. The one design decision that matters

The brief proposes `nerve trace-tests ...` — a command that executes the repository's test suite
under instrumentation — and permits *"scoping any allowed subprocess use narrowly to the explicit
tracing command"*.

**I am not taking that permission, and the reason is a test in this repository that would have to be
weakened to accept it.**

`crates/nerve-cli/tests/no_subprocess.rs` forbids eight process-creation substrings anywhere under
`crates/*/src/**`. Its module documentation names this exact scenario as the thing it exists to
refuse:

> *"repository content is untrusted, and Nerve must parse it rather than run it. No package scripts,
> no build tools, no compilers, **no test runners**, no `git` binary — Git HEAD is read from
> `.git/HEAD` directly for exactly this reason. … A future extractor shelling out to `tsc` for type
> resolution would be a natural-looking change that silently breaks T1; this test is what refuses
> it."*

A `nerve trace-tests -- pytest tests/` needs `Command::new` in `crates/nerve-*/src/`. There is no
way to add it that does not amount to adding an exception to `FORBIDDEN`, and an invariant with an
exception is a convention. The test would go from *structural* to *approximately structural*, which
is the transition the file's second paragraph says it was written to prevent.

**Two independent precedents in this codebase already chose the other way, and both are shipped:**

| precedent | the tempting design | what was actually built |
|---|---|---|
| Slice 6b — coverage | run the test suite with `--coverage` | **ingest an LCOV file** the user's own run produced |
| `gitinfo.rs` — Git state | shell out to `git rev-parse HEAD` | **read `.git/HEAD` and the packed-refs files directly** |

Neither is a compromise; both are how Nerve works. Slice 11 is the same shape.

### The design

**Nerve ships a tracer. The user runs it. Nerve ingests the artifact.**

```bash
# 1. The user runs their own tests, in their own environment, under Nerve's tracer.
python -m nerve_trace --out .nerve/trace/run-1.jsonl -m pytest tests/

# 2. Nerve reads the artifact. No process is created.
nerve trace ingest .nerve/trace/run-1.jsonl
```

| | subprocess design | ingest design |
|---|---|---|
| `no_subprocess.rs` | needs an exception | **passes untouched** |
| T1 | scoped exception | **absolute, unchanged** |
| secrets in the environment | Nerve inherits the parent env and passes it to a child | **Nerve never holds them** |
| CI | Nerve must reimplement the runner invocation | **CI already runs the tests; add the hook** |
| test-runner support | Nerve must know pytest *and* vitest *and* jest | the tracer is one Python module |
| process hazards | argument injection, signal handling, zombies, output bounds, timeouts | **none exist** |
| steps for the user | one | two |

The cost is one extra step in the user's hands. What is bought is that Nerve's most important
security invariant stays a theorem rather than a policy. **`nerve trace-tests` is refused, and the
refusal is recorded**, in the manner of `nerve affected` (ADR-0008 §A.2) — a command that is refused
for a stated reason is a design position, not a gap, and the final acceptance script must encode it
as such.

### What Nerve ships, and the honest cost

`tracers/python/nerve_trace/` — a small pure-standard-library Python package using
`sys.monitoring` (3.12+) with a `sys.settrace` fallback. It is **not** a Rust crate, adds nothing to
`Cargo.lock`, and is not imported by product code. It is new non-Rust product surface, and that is a
real maintenance cost stated rather than hidden: it needs its own tests, and its output format is a
versioned contract.

**Python only in Slice 11.** A JavaScript tracer needs `--require` hooks, source-map resolution
back to TypeScript, and per-runner worker models — that is a slice of its own, and shipping half of
it would produce a `TEST_OBSERVED_CALL` relation whose coverage silently depends on language.

## 2. Vocabulary

`Relation::TestObservedCall`, appended (`ALL` 11 → 12 after Slice 10a).

`EvidenceSourceType::TestCallTrace` **already exists**, declared since Slice 1 and never emitted.
Slice 11 is its first emitter, exactly as Slice 10a is the first emitter of `FrameworkRule`. No
member is added on that axis and no `source_type_mask` ordinal moves.

`Directness::Inferred` is **wrong** here and `Direct` is right: the tracer *observed* the call. It
is the most direct evidence of a call that Nerve will ever hold — stronger than `AST_DIRECT`, which
only proves a call was written.

### The distinction the brief requires, stated as an invariant

```
STATIC_CALL          Relation::Calls  + AST_DIRECT / AST_RESOLVED   — the source says so
TEST_COVERS_SYMBOL   refused by the vocabulary (ADR-0008)           — LCOV has no per-test attribution
COVERS               Relation::Covers + TEST_COVERAGE               — a run executed lines in a symbol
TEST_OBSERVED_CALL   Relation::TestObservedCall + TEST_CALL_TRACE   — a tracer saw A call B during a test
RUNTIME_OBSERVED_CALL / FRAMEWORK_INFERRED_CALL                     — not emitted in Slice 11
```

**A call is never inferred from coverage co-occurrence.** Two symbols executing in one run says
nothing about who invoked whom — ADR-0005, restated for a new relation. The gate is a test asserting
that the coverage extractor and the trace extractor share no assertion, and that no `COVERS`
observation ever becomes a `TEST_OBSERVED_CALL`.

**A trace is not a call graph either.** A tracer proves *this* run took *this* edge. It proves
neither that the edge is always taken nor that unobserved edges do not exist. `TEST_OBSERVED_CALL`
is existential evidence; absence of it is absence of observation, not absence of a call — the
"absence is not zero" rule this project has applied to coverage, gaps, unresolved accounts, and
verdicts.

## 3. Trace identity, and the endpoint problem

The brief lists what every observation must carry: repository state, test identity, test-run
identity, extractor id and version, environment identity, language/runtime version, framework,
source-map configuration, start time, completion status, partial/failure status.

Most of that is `ExtractorRun` plus `observation.environment`, which already exist. **One item needs
a decision: `test identity`.**

`COVERS` has a `CoverageRun` endpoint precisely because LCOV carries no per-test attribution
(ADR-0008). **A tracer does not have that limitation** — it knows which test function was executing.
So Slice 11 *can* carry per-test attribution that Slice 6b could not, and refusing it would be
throwing away evidence the artifact genuinely contains.

The endpoint is therefore the **test symbol itself**: `Function TEST_OBSERVED_CALL Function`, source
= the test function (an ordinary indexed `Function` entity), target = the callee. A `TraceRun`
entity records the run, and each observation cites it in `environment`.

This is the asymmetry with `COVERS` and it must be documented, not smoothed over: **`COVERS` has a
run endpoint because LCOV cannot attribute; `TEST_OBSERVED_CALL` has a test endpoint because a
tracer can.** Same project, different evidence, different shape. If the tracer cannot identify the
executing test for some frame, the observation is **dropped and counted**, never attributed to a
guess.

## 4. Limitations, each counted rather than described

Per the brief, and following Slice 9b's gate 7 — a tally the fixture asserts, so a silently growing
set of unobservable forms fails the build:

`async` continuations · threads · `multiprocessing` children · native/C extension frames ·
dynamic imports · generated code · framework wrappers · sampling gaps · a crashed test ·
an interrupted run · parallel tests sharing a process · `sys.setprofile` contention.

A run that did not finish is `partial` and **every query that reports its evidence says so**. A
partial trace must never read as a complete one — the `Unverified`-vs-`Stale` distinction Slice 7c-i
established for `nerve check`.

## 5. T9 — untrusted trace input

`docs/THREAT-MODEL.md:238` records *"Test evidence — tracing (Slice 11) — required before Slice 11
ships"*. The artifact is untrusted input, exactly as an LCOV file is:

- Path traversal and symlink escape in a recorded file path: **refused and counted**, never mapped.
- A file changed since indexing: **refused**, not mapped through stale extents (the Slice 6b lesson).
- Every resource bound refuses whole rather than truncating.
- A trace naming an unindexed file creates **no** entity.
- **Privacy: no argument values, no return values, no locals, no source text.** File, line, symbol
  identity, timing. A trace that captured arguments would capture credentials, and there is no
  redaction scheme that is safe by construction. The tracer must be incapable of it, not configured
  against it.
- Retention: the artifact lives where the user put it; `nerve` never uploads it, and ingestion is
  explicit. Trace evidence is withdrawn by repository state like any other observation.
- Trace text reaching MCP is repository-derived and stays inside `repository_content` (T7).

## 6. Acceptance criteria

1. `Relation::TestObservedCall` appended; every exhaustiveness test states it. `TestCallTrace`
   emitted for the first time; no `EvidenceSourceType` member added.
2. `no_subprocess.rs` and `no_network.rs` pass **unmodified**. `git diff` on both is empty.
   This is the criterion the whole design exists to satisfy.
3. `nerve trace-tests` does not exist, and the refusal is documented with its reason.
4. A tracer artifact from a real `pytest` run over a fixture is ingested; `TEST_OBSERVED_CALL` edges
   appear with per-test attribution.
5. **A `COVERS` observation never becomes a `TEST_OBSERVED_CALL`**, asserted over `Relation::ALL`.
6. A partial/crashed run is labelled partial everywhere its evidence is reported.
7. Every unobservable form counted by form, and the tally asserted.
8. T9 attacked: traversal, symlink escape, unindexed file, changed file, oversized artifact,
   malformed JSON — each refused, counted, and disclosing nothing.
9. No argument or return value is capturable by the tracer, asserted by a test over its output.
10. No new Rust dependency. `Cargo.lock` unchanged.
11. Full gate: fmt, clippy `-D warnings`, `cargo test --workspace --no-fail-fast`, release build.

## 7. Non-goals

Running the user's tests. JavaScript/TypeScript tracing. Runtime (production) tracing. Sampling
configuration. Flakiness detection. Test selection or prioritisation — `nerve affected` stays
refused; per-test *call* attribution does not make LCOV attributable, and a trace-derived
affected-tests answer would be a different command with a different evidence basis.
