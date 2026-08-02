# Slice 6 — test evidence (coverage only)

**Date:** 2026-08-01 · **Status:** Approved by the orchestrator
**Gate:** `docs/THREAT-MODEL.md` **T9** · ADR-0005 (coverage is not a call graph)

---

## 1. Objective and user value

Two questions, and only two, are in scope:

- **"Which symbols does no test touch?"** — the gap question. It needs aggregate coverage only.
- **"Is this coverage still true of the code as it stands?"** — the freshness question. Coverage is
  the most silently-stale evidence a repository has: an `lcov.info` from three weeks ago looks
  exactly like one from this morning.

**"Which tests would my change affect?"** is *not* automatically in scope. It requires per-test
attribution, and §2.1 is the finding that decides whether that exists at all.

## 2. Disagreements and pushback

### 2.1 `TEST_COVERS_SYMBOL` presumes an attribution the common formats do not carry

The roadmap names the relation `TEST_COVERS_SYMBOL`, which is an edge from **a test** to a symbol.
The brief also says, correctly, *"Validate actual per-test coverage capabilities before claiming
support."*

That validation is the **first task of this slice and its gating question**, because the standard
JavaScript coverage artefacts are aggregate: LCOV (`lcov.info`) records, per source file, which
lines were hit by *the run*, and Istanbul/`nyc`'s `coverage-final.json` records statement, function
and branch maps for *the run*. Neither names a test. A run of 400 tests produces one report in which
every covered line looks identical regardless of which test caused it.

**The implementer must establish empirically what the tooling actually emits before writing the
emission path**, and the design must follow the finding rather than the roadmap's wording:

- **If a format carries per-test attribution**, the source endpoint is a test entity and
  `TEST_COVERS_SYMBOL` means what its name says.
- **If it does not**, Nerve must not manufacture the attribution. Attributing an aggregate report to
  each discovered test file would be exactly the fuzzy inference this project refuses everywhere
  else, and it would be worse than useless: it would assert that every test covers every covered
  symbol.

In the aggregate case the honest source endpoint is **the coverage run**, identified by the report
file and its content hash — a thing that genuinely exists, with a real occurrence at a real path —
and the product language must say "the test suite covers this symbol", never "this test covers it".
Affected-test analysis is then reported as **unsupported for aggregate reports**, with the exact
input that would support it named.

Whatever the finding, **record it in the report with the evidence**, and update the roadmap's
wording if it turns out to promise something the formats cannot deliver.

### 2.2 Coverage is not a call graph — and this is where it would be violated

ADR-0005 is binding and this is the slice that tests it. Two functions executing during one test run
is **not** evidence that either calls the other. Nothing in this slice may emit `CALLS`,
`TEST_OBSERVED_CALL` or any call-shaped relation. `TEST_CALL_TRACE` is Slice 11 and needs actual
instrumentation.

A test must assert it: after ingesting coverage, the count of call-shaped relations attributable to
the coverage extractor is **zero**, checked exhaustively rather than by inspection.

### 2.3 Line coverage maps to symbols through spans, and the mapping is lossy

Coverage is per line; Nerve's symbols have byte and line extents. The mapping reuses Slice 5c's
`innermost_covering` — the same function that maps a `#L<n>` document anchor to a symbol — so there
is one implementation of "which symbol owns this line" rather than two that can disagree.

The lossiness must be stated rather than smoothed: a line covered inside a symbol proves the symbol
was **entered**, not that it was fully executed; a symbol with some lines covered and some not is
**partial**, and partial must be a recorded value, not rounded to covered or uncovered.

### 2.4 A coverage report is attacker-controlled input — T9

It is a file in the repository. Required controls, all of which must be tested by attack rather than
assumed:

- Paths inside the report resolve through the **same path guard** as everything else. A report
  naming `../../../../etc/passwd` is refused and counted.
- A report naming a file that is not indexed is **rejected and counted**, never trusted into
  existence — it must not create entities.
- A symbol that does not exist is rejected and counted.
- Resource bounds: report size, record count, line numbers. Every bound refuses and counts.
- Malformed input never panics: truncated LCOV, unterminated records, absurd line numbers,
  duplicate records, mixed line endings, invalid UTF-8 at the read boundary.

### 2.5 Ingestion is explicit, and reads only what the user names

Coverage is **not** discovered by walking for anything that looks like a report. Nerve reads the
report the user names. Auto-discovering `coverage/lcov.info` would mean an index silently changing
meaning because a test run happened to leave a file behind.

No test is executed. No subprocess is spawned. The `no_subprocess` invariant is untouched: Nerve
**reads a report someone else produced**. Running tests is Slice 11 and is a separate, explicitly
invoked, user-authorised workflow.

## 3. Design

### 3.1 Evidence

`EvidenceSourceType::TestCoverage` already exists at ordinal 5 — declared since Slice 1, emitted by
nothing. Directness is `Inferred`: a line hit is not a statement that a symbol is covered; a
mapping step concluded it.

New extractor `coverage 1.0.0`, declaring `[TestCoverage]` and nothing else.

### 3.2 Freshness is the point, not a footnote

Every coverage observation records the **content hash of the covered file at ingestion time**. That
is already how `nerve why` computes freshness — re-hash at query time and compare — so coverage
inherits it for free and a stale report is visibly stale rather than quietly wrong.

The report's own hash and mtime are recorded too, so "this coverage came from a report that predates
the code" is answerable.

### 3.3 What is emitted

- A coverage-run or test entity per §2.1's finding, with a real occurrence at the report path.
- `TEST_COVERS_SYMBOL` to each symbol with at least one covered line.
- Per-observation `details`: covered lines, total lines in the symbol's extent, and a
  `covered`/`partial` value. Uncovered symbols get **no edge** — absence is the answer, and
  inventing a `NOT_COVERED` edge would put a negative claim in a positive-evidence store.

### 3.4 Incremental behaviour

A changed coverage report re-ingests only that report. A changed source file invalidates the
coverage edges that name it, because the line-to-symbol mapping may have moved. Full-vs-incremental
byte-identical equivalence must hold over an edit sequence including a coverage report added,
changed and removed.

## 4. Acceptance criteria

1. §2.1 answered empirically, with evidence, before the emission path is written; the design follows
   the finding and the roadmap wording is corrected if it over-promised.
2. Explicit ingestion only — no auto-discovery, no test execution, no subprocess.
3. LCOV supported; a second format supported or explicitly declared unsupported with the reason.
4. Line-to-symbol mapping reuses `innermost_covering`.
5. `partial` is a recorded value, never rounded.
6. **Zero call-shaped relations from the coverage extractor**, asserted exhaustively.
7. Every T9 control tested by attack: traversal, unindexed file, unknown symbol, every bound.
8. Malformed input never panics — the fixture list in §2.4.
9. Freshness works: a report predating the code is visibly stale.
10. Full-vs-incremental equivalence holds over a coverage edit sequence.
11. Fixture precision measured and gated; framed as a regression gate, not an accuracy claim.
12. No new dependency. Full gate green.

---

# Addendum — §2.1 answered empirically, 2026-08-01

Run by the orchestrator before any implementation, on Node v24.15.0, using the runtime's **built-in**
coverage and LCOV reporter so that no dependency and no network are involved. Two source files, two
test files, each test exercising exactly one source file.

## A.1 The evidence

**Probe 1 — one run over both test files** (`node --test --experimental-test-coverage
--test-reporter=lcov`):

```
TN:
SF:src/alpha.js
FN:1,alphaCovered
FNDA:1,alphaCovered
DA:1,1 … DA:6,0 DA:7,0
end_of_record
SF:src/beta.js
…
end_of_record
```

`TN:` — LCOV's *test name* field — is **empty**. There is exactly one record set per source file for
the whole run, and every `DA:` line carries a hit count with no test dimension. Two tests went in;
one undifferentiated report came out.

**Probe 2 — one run over `test/alpha.test.js` alone:** `src/beta.js` is **absent** from the report
entirely (`grep -c "SF:src/beta.js"` → `0`). So the only attribution that exists is *per run*, and
obtaining per-test attribution would require N separate runs of N tests.

**Probe 3 — can the merge workaround recover it?** Concatenating the two single-test reports gives:

```
TN:<empty>   SF:src/alpha.js
TN:<empty>   SF:src/beta.js
```

Both records' test-name field is blank. **Even the concatenation workaround fails**, because the
reporter never populates `TN:`. A consumer reading that file cannot tell which test produced which
record.

## A.2 The finding

**Aggregate. There is no per-test attribution to read.** §2.1's second branch applies, and the
design follows the finding:

- The source endpoint is **the coverage run**, a `CoverageRun` entity identified by the report's
  repository-relative path and content hash, with a real occurrence at that real path.
- No `Test` entity is an endpoint of a coverage edge. This is **structural, not a convention**: it is
  impossible to state "test X covers symbol Y" because no such endpoint exists to state it with.
- Product language says *"the test suite covers this symbol"*, never *"this test covers it"*.
- **Affected-test analysis is unsupported**, and the report must name the exact input that would
  support it: one coverage report per test, produced by N separate runs, with the test's identity
  carried outside the report (the format cannot carry it).

## A.3 The relation is named `COVERS`, not `TEST_COVERS_SYMBOL`

The roadmap, `CLAUDE.md` §3 and the master brief all spell it `TEST_COVERS_SYMBOL`. That spelling is
rejected, for two reasons, the second of which is the load-bearing one.

1. **It breaks a recorded convention.** Nerve's relation vocabulary is endpoint-kind-agnostic:
   `CONTAINS`, `DEFINES`, `IMPORTS`, `EXPORTS`, `CALLS`, `REFERENCES`, `EXTENDS`, `IMPLEMENTS`,
   `SUPERSEDES`. Kinds live in `entity.kind` and are never duplicated into a relation name — a
   decision already made in Slice 5a, where the briefed `DOCUMENT_CONTAINS_SECTION` was rejected for
   exactly this reason and shipped as `CONTAINS`. `TEST_COVERS_SYMBOL` names *both* endpoints.

2. **It asserts an endpoint the evidence cannot support.** After §A.2 the source is a coverage run,
   not a test. A relation called `TEST_COVERS_…` would state, in the vocabulary itself, a per-test
   attribution that the format does not carry. That is the same defect class Slice 5d-i corrected
   when filesystem containment was labelled `AST_DIRECT`: a name claiming evidence that does not
   exist. Shipping it would be a regression against the reason this project exists.

`COVERS`, from a `CoverageRun` to a symbol, states exactly what is known and nothing more.

**The invariant `CLAUDE.md` protects is untouched.** Its requirement is that coverage must never be
relabelled as a call relationship, and that is enforced harder here than the name ever did: a test
asserts the coverage extractor produces **zero** call-shaped relations, exhaustively. `CLAUDE.md`
§3, `docs/decisions/ADR-0005-coverage-is-not-calls.md` and the roadmap row are updated in the same
commit so no contradictory spelling is left silently authoritative. ADR-0008 records this.

## A.4 Two deviations from §3.4, with reasons

**Ingestion is a standalone `nerve coverage` command, not a flag on `nerve index`.** A flag would
mean that the ordinary post-edit `nerve index` — run without it, as it always is — silently
destroys every coverage edge. Making ingestion its own explicitly-invoked verb also makes "no
auto-discovery" structural rather than a promise.

**A source-file edit does not delete the coverage edges naming that file.** §3.4 proposed
invalidating them. That is superseded by §3.2, which is the better mechanism and already exists:
the observation records the covered file's content hash at ingestion, and `nerve why` re-hashes at
query time, so an edited file makes its coverage **visibly stale** — strictly more informative than
deleting it, which would destroy the evidence that coverage was ever ingested and leave silence in
its place. Edges whose symbol genuinely no longer exists are removed by the existing orphan pruner,
which needs no coverage-specific logic. Equivalence is therefore stated as: a full index followed by
ingestion and an incremental index followed by ingestion are byte-identical.

## 5. Non-goals

No test execution. No call tracing (Slice 11). No affected-test analysis unless §2.1 finds real
per-test attribution. No coverage *thresholds* or quality judgements — Nerve reports what is
covered, it does not grade it. No new schema version unless the entity kinds force one.
