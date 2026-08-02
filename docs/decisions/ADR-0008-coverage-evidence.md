# ADR-0008 — Coverage evidence: the run is the endpoint, and the relation is `COVERS`

**Status:** Accepted · **Date:** 2026-08-01 · **Slice:** 6a
**Amends:** [ADR-0005](./ADR-0005-coverage-is-not-calls.md) (relation name only; its argument stands)

---

## Context

Slice 6 ingests test-coverage evidence. The roadmap, `CLAUDE.md` §3, the master brief and ADR-0005
all name the relation `TEST_COVERS_SYMBOL` — an edge from **a test** to a symbol.

`docs/plans/slice-06-test-evidence.md` §2.1 refused to accept that name on trust and made it the
slice's gating question: *does the format actually carry per-test attribution?* The plan is explicit
that the design must follow the finding rather than the wording, because attributing an aggregate
report to each discovered test file would assert that every test covers every covered symbol — the
fuzzy inference this project refuses everywhere else.

## The finding — LCOV is aggregate

Measured by the orchestrator before any implementation, on Node v24.15.0, using the runtime's
**built-in** coverage and LCOV reporter so that no dependency and no network was involved. Two
source files, two test files, each test exercising exactly one source file. Recorded in full at
`docs/plans/slice-06-test-evidence.md` §A.1.

**Probe 1 — one run over both test files.** The report opens with a bare `TN:`. LCOV's *test name*
field is **empty**. There is exactly one record set per source file for the whole run, and every
`DA:` line carries a hit count with no test dimension. Two tests went in; one undifferentiated
report came out.

**Probe 2 — one run over one test file.** The source file the other test exercises is **absent**
from the report entirely. The only attribution that exists is *per run*; obtaining per-test
attribution would require N separate runs of N tests.

**Probe 3 — the concatenation workaround.** Concatenating the two single-test reports yields two
records whose `TN:` fields are **both blank**. Even the workaround fails, because the reporter never
populates the field. A consumer reading that file cannot tell which test produced which record.

**Conclusion: there is no per-test attribution to read.**

## Decision

### 1. The source endpoint is a `CoverageRun`, not a `Test`

`EntityKind::CoverageRun`, canonical name `coverage_run`, prefix `cov`:

> One coverage report, identified by its repository-relative path and its content hash.

A coverage run is a thing that genuinely exists, with a real occurrence at a real path. A test as
the endpoint of a coverage edge does not.

This is **structural, not a convention**. It is impossible to state "test X covers symbol Y" in
Nerve's graph, because no such endpoint exists to state it with. Product language says *"the test
suite covers this symbol"*, never *"this test covers it"*.

**Affected-test analysis is therefore unsupported for aggregate reports.** The exact input that
would support it, named so that the limit is a fact rather than a shrug: one coverage report per
test, produced by N separate runs, with each test's identity carried **outside** the report — the
format cannot carry it.

### 2. The relation is `COVERS`

`Relation::Covers`, rendering as `COVERS`, from a `CoverageRun` to a symbol.

`TEST_COVERS_SYMBOL` is rejected, for two reasons, the second of which is load-bearing.

1. **It breaks a recorded convention.** Nerve's relation vocabulary is endpoint-kind-agnostic:
   `CONTAINS`, `DEFINES`, `IMPORTS`, `EXPORTS`, `CALLS`, `REFERENCES`, `EXTENDS`, `IMPLEMENTS`,
   `SUPERSEDES`. Kinds live in `entity.kind` and are never duplicated into a relation name — a
   decision already made in Slice 5a, where the briefed `DOCUMENT_CONTAINS_SECTION` was rejected for
   exactly this reason and shipped as `CONTAINS`. `TEST_COVERS_SYMBOL` names *both* endpoints.

2. **It asserts an endpoint the evidence cannot support.** After the finding above, the source is a
   coverage run. A relation called `TEST_COVERS_…` would state, in the vocabulary itself, a per-test
   attribution the format does not carry. That is the same defect class Slice 5d-i corrected when
   filesystem containment was labelled `AST_DIRECT`: a name claiming evidence that does not exist.
   Shipping it would be a regression against the reason this project exists.

`COVERS`, from a `CoverageRun` to a symbol, states exactly what is known and nothing more.

**This reverses an explicit prohibition, and says so rather than quietly overriding it.** ADR-0005's
Enforcement section read: *"The relation name in the schema is `TEST_COVERS_SYMBOL`, not `COVERS` and
not `CALLS`."* `COVERS` was named and forbidden, not merely unconsidered.

That prohibition is reversed on evidence that did not exist when it was written. ADR-0005 was
accepted on 2026-07-31, before any coverage format had been measured; it bundled two things into one
sentence — *don't call it `CALLS`* and *do call it `TEST_COVERS_SYMBOL`* — and the measurement
falsifies only the second. `COVERS` was rejected then for being vaguer than `TEST_COVERS_SYMBOL`;
after the finding, `TEST_COVERS_SYMBOL` is not more precise, it is **precisely wrong**, naming a test
endpoint that cannot exist. A name that is vague about a kind is a style question. A name that
asserts an attribution the evidence cannot carry is the defect this project exists to prevent.

The half of that sentence that carries the ADR's actual argument — *not `CALLS`* — is untouched, and
is now enforced by a test rather than by a spelling.

**The invariant `CLAUDE.md` §3 protects is untouched, and is enforced harder than the name ever
enforced it.** Its requirement is that coverage must never be relabelled as a call relationship —
that is ADR-0005, and ADR-0005 stands in full. Slice 6b adds a test asserting that the coverage
extractor produces **zero** call-shaped relations, checked exhaustively rather than by inspection.
A name is a reminder; a test is a control.

### 3. Directness is `INFERRED`

A coverage report states that **line `n` of a file executed**. Concluding that a **symbol** is
covered requires mapping that line onto a symbol's extent — a rule concluding something the artifact
did not say, which is ADR-0003's definition of `INFERRED`.

Recording it as `DIRECT` would repeat exactly the defect Slice 5d-i corrected: an evidence label
that is knowably stronger than the evidence behind it. The lossiness is real and is not smoothed
over — a covered line inside a symbol proves the symbol was **entered**, not that it ran to
completion, which is why `docs/plans/slice-06-test-evidence.md` §2.3 makes `partial` a recorded
value rather than something rounded to covered or uncovered.

### 4. The source type already exists; no new one is added

`EvidenceSourceType::TestCoverage` (`TEST_COVERAGE`) has been declared at ordinal 5 since Slice 1
and emitted by nothing. Slice 6 is what finally emits it. ADR-0005 §2 keeps it permanently distinct
from `TEST_CALL_TRACE` and `RUNTIME_CALL_TRACE`, and that separation is unchanged.

The new extractor is `coverage 1.0.0`, declaring `[TEST_COVERAGE]` and nothing else.

### 5. No schema migration is required

`entity.kind` and `assertion.relation` are `TEXT NOT NULL` columns with no `CHECK` constraint and no
integer encoding anywhere (`crates/nerve-store/src/schema.rs`). Nothing on disk encodes a position
in `EntityKind::ALL` or `Relation::ALL`, so adding a member changes no stored byte and invalidates
no existing database.

This is **not** the situation `EvidenceSourceType` is in, and the difference is worth stating so
that a future reader does not generalise the wrong way: `assertion_state.source_type_mask` is a
**stored** integer whose bit layout is `1 << ordinal` over `EvidenceSourceType::ALL`, which is why
that vocabulary is append-only and why every one of its ordinals is pinned by a test. `EntityKind`
and `Relation` carry no such obligation to the database.

They do carry one to the **interface**: `apps/nerve-web/src/api/types.ts` mirrors both arrays in
order, and `crates/nerve-server/tests/ui_vocabulary.rs` asserts the two match exactly. Both new
members are therefore appended rather than inserted, and both are glossed in the same commit.

## What the parser does and does not do (Slice 6a)

`crates/nerve-index/src/coverage.rs` is a **pure** LCOV reader: `&[u8]` in, a `CoverageReport` out.
It opens no file, spawns nothing and reaches no socket — the signature is the proof, as `FsEntry` is
the proof for `fs-structural` (ADR-0007 §2).

- It yields, per source file, the `SF:` value **exactly as written** and the set of `(line, hits)`
  pairs. Paths are not normalised, resolved or canonicalised here — not even to strip a control
  byte — for the same reason `crate::markdown` carries control bytes through: the path guard is
  where a hostile path is refused, and a refusal it never sees is a refusal nobody reports. Slice 6b
  routes every path through `discover::canonical_child`.
- `DA:<n>,0` is **preserved**. A zero hit count says the line was instrumented and not executed,
  which is the raw material of the gap question; discarding it would leave "not covered" and "never
  instrumented" indistinguishable.
- Branch data (`BRDA:`/`BRF:`/`BRH:`) and function data (`FN:`/`FNDA:`/`FNF:`/`FNH:`) are recognised
  and shape-checked, and **not modelled**. Branch coverage answers "did both arms run", a different
  question from "was this symbol entered".
- Summary totals (`LH:`, `LF:`, `FNF:`, `FNH:`, `BRF:`, `BRH:`) are not used to cross-check the
  `DA:` lines. A disagreement between a summary and the data it summarises has no correct resolution
  at parse time, and picking one would mean believing a number over the evidence, or the reverse,
  with no grounds for either.
- A non-empty `TN:` is **counted** (`test-name-present`). This ADR rests on the finding that the
  field is emitted empty; the counter means a producer that populates it contradicts the finding
  with a number rather than leaving it to be defended from memory.
- Duplicate records for one path are **merged by maximum, never by sum**. With `TN:` empty there is
  no way to tell two runs from one run recorded twice, and the maximum is the weakest claim
  consistent with both readings: it never invents an execution no record stated.
- A record with no closing `end_of_record` is **dropped and counted**. `end_of_record` is the
  format's own statement that a record is complete; without it a truncated `DA:` list is
  indistinguishable from a finished one, and a truncated list reads as "those lines never executed".

Three resource bounds, each refusing and counting rather than truncating: `MAX_REPORT_BYTES`
(32 MiB, also the parser's memory ceiling), `MAX_RECORDS` (100,000 — one per source file, bounding
the graph one attacker-controlled file can create), and `MAX_LINES_PER_RECORD` (1,000,000, derived
from `DEFAULT_MAX_FILE_BYTES` of 2 MiB at two bytes minimum per line).

## Alternatives considered

**Keep `TEST_COVERS_SYMBOL` and attribute the aggregate report to every discovered test file.**
Rejected. It would assert that every test covers every covered symbol — a claim that is false for
almost every pair, expensive to compute, and most convincing exactly where it is most wrong. It is
the same failure mode as establishing identity by name similarity, which ADR-0002 forbids.

**Keep `TEST_COVERS_SYMBOL` as a name while pointing it at a `CoverageRun`.** Rejected. A vocabulary
member that says `TEST_` while its source endpoint is a run is a false label, and a false label is
worse than no label because the product's whole claim is that its labels can be trusted without
re-deriving them.

**Run each test separately to synthesise per-test attribution.** Rejected for this slice, and
recorded rather than dismissed. It would require executing tests, which the plan's §2.5 puts outside
Slice 6 entirely: Nerve reads a report someone else produced, spawns no subprocess, and leaves test
execution to Slice 11 as a separately invoked, user-authorised workflow. N runs of N tests is also
quadratic in wall-clock on any real suite.

**Emit a `NOT_COVERED` edge for uncovered symbols.** Rejected. Absence is the answer to the gap
question, and a negative claim does not belong in a positive-evidence store. The uncovered *lines*
are still recorded on the observation; it is the *edge* that would be an invention.

## Consequences

- `EntityKind::ALL` has 12 members and `Relation::ALL` has 10. Anything matching exhaustively over
  either is forced by the compiler to consider the new variant, which is the intended behaviour.
- `crates/nerve-server/tests/ui_vocabulary.rs` — the Slice 5d-iii test that fails when a Rust
  vocabulary gains a member the interface cannot name — fires on both additions, and both glosses
  ship in the same commit. That is the test working.
- `CLAUDE.md` §3, `ADR-0005` §1 and §"Enforcement", `docs/ROADMAP.md` row 6 and
  `docs/THREAT-MODEL.md` T9 are corrected in the same commit, so no contradictory spelling is left
  silently authoritative. Historical planning documents
  (`docs/plans/2026-07-31-claude-architecture-plan.md`,
  `docs/plans/nerve-master-build-plan.md`) are left as written: they are a record of what was
  planned, and this ADR is the record of what was found instead.
- "Which tests would my change affect?" is answered *"not from an LCOV report, and here is what it
  would take"* rather than answered badly.
