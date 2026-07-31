# ADR-0005 — Coverage is not a call graph

**Status:** Accepted · **Date:** 2026-07-31 · **Applies from:** Slice 6

## Context

An earlier Nerve planning document (`docs/plans/2026-07-31-claude-architecture-plan.md`)
proposed that ingesting per-test coverage would "discover the dynamic-dispatch edges the
static resolver missed", and made that the centrepiece of the first validation experiment.

**That claim is wrong and is withdrawn.**

## The error

Line and branch coverage produce a *set of executed regions per test*. From

> symbols `A` and `B` were both executed during test `T`

it does **not** follow that

> `A` calls `B`

`A` and `B` may be siblings invoked independently by the test, or connected through
intermediate frames that coverage never records. Coverage records *that* code ran, never
*who invoked it*. Inferring call edges from co-execution is the same class of error as
inferring a relationship from name similarity — it is merely more expensive to compute and
more convincing when wrong.

## Decision

1. Coverage supports exactly one relation:

   ```
   Test T  TEST_COVERS_SYMBOL  Symbol S     evidence_source_type = TEST_COVERAGE
   ```

2. The following evidence source types are kept **permanently distinct** and must never be
   collapsed, aliased, or presented interchangeably in the schema, CLI, API, or UI:

   ```
   TEST_COVERAGE        symbol executed during a test
   TEST_CALL_TRACE      call observed during a test, via instrumentation
   RUNTIME_CALL_TRACE   call observed at runtime
   AST_DIRECT           call present in the syntax tree
   TYPE_RESOLVED        call resolved by a type checker
   FRAMEWORK_RULE       call inferred by a deterministic framework rule
   ```

3. Observed call relationships require explicit instrumentation and land in a **separate
   slice** (Roadmap #11), not in the coverage slice (#6).

## Instrumentation options for observed calls, with their evidence properties

| Mechanism | Property | Implication |
|---|---|---|
| V8 `--cpu-prof` / Inspector `Profiler` domain | **Sampled** — fast frames are missed | Must carry a sampling rate; may never be presented as a complete call graph; absence is not evidence |
| V8 `Debugger` instrumentation breakpoints | Deterministic but severe slowdown | Unusable on real suites |
| Source transform wrapping function entries | Deterministic, but alters timing and semantics | Changes the artifact under test; opt-in only |
| Python `sys.setprofile` | Deterministic call/return events, moderate overhead | Best available of these |

Whichever is chosen, sampling rate and mechanism are stored on the observation, and
`RUNTIME`/`TEST` call traces are never treated as exhaustive.

## Consequence for the validation experiment

The affected-test experiment remains worth running, but its honest framing is:

> Does coverage-derived test↔symbol evidence select affected tests better than the
> "run tests whose files changed" heuristic?

It is **not**:

> Does coverage discover call edges the static graph missed?

The second question requires call tracing and belongs to Roadmap slice 11.

## Enforcement

The relation name in the schema is `TEST_COVERS_SYMBOL`, not `COVERS` and not `CALLS`.
Any pull request that maps coverage data onto a call relation must be rejected.
