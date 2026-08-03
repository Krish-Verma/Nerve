# Slice 11a plan — Trace ingestion, and three corrections to the Slice 11 plan

2026-08-03. Supersedes parts of `docs/plans/slice-11-test-observed-calls.md`, which stays
authoritative on §1 (no test runner), §4 (limitations), §5 (T9) and §7 (non-goals).
Row 11 of `docs/ROADMAP.md`, split.

---

## 1. What survives from the Slice 11 plan

**§1 stands, and is reinforced.** Nerve does not run the test suite. `nerve trace-tests` is refused.
`crates/nerve-cli/tests/no_subprocess.rs:1-21` forbids process creation in `crates/*/src/**` and its
own module doc names *"no test runners"* as the thing it exists to refuse. Two shipped precedents
chose ingestion: coverage reads LCOV, `gitinfo.rs` reads `.git/HEAD`. Nothing found since changes
this.

**§4, §5, §7 stand.** Limitations counted by form; the artifact is untrusted input under T9; no
JavaScript tracer, no runtime tracing, no flakiness detection, `nerve affected` stays refused.

## 2. Three corrections, each decided by code already in the repository

### 2.1 The plan's endpoint model is wrong for any call deeper than the test body

The plan §3 says:

> "The endpoint is therefore the **test symbol itself**: `Function TEST_OBSERVED_CALL Function`,
> source = the test function … target = the callee."

**This is false for nested frames, which are most of a trace.** For a stack `test_x → parse → lex`,
the tracer observes two call events. Under the plan's model Nerve would either

- emit `test_x TEST_OBSERVED_CALL lex`, which **asserts a call the test never made**, or
- emit only depth-1 events, discarding most of the artifact.

The evidence model already has the right shape, and ADR-0003 exists to keep these separated:

| layer | what it holds |
|---|---|
| `assertion` | `(caller_symbol, TEST_OBSERVED_CALL, callee_symbol)` — the call that happened |
| `observation` | *which test, which run, which environment, which producer* observed it |

So **the assertion endpoints are the two frames of the call, and test identity lives in the
observation.** Two tests observing one edge produce two observations on one assertion, which is
exactly what `observation_count` is for.

This also answers the brief's §8 queries without a second model: *"which tests observed a call to
this symbol?"* is a query over observations of assertions targeting it.

### 2.2 No new `EntityKind`, and no schema change

The plan implies a `TraceRun` entity by analogy with `CoverageRun`. **The analogy does not hold.**
`coverage.rs:17-19` states why `CoverageRun` exists:

> "The source endpoint of a coverage edge is a `CoverageRun`, never a test. It is impossible to state
> 'test X covers symbol Y' because no such endpoint exists to state it with."

`CoverageRun` exists because it had to be an **endpoint**. Under §2.1 a trace run is *not* an
endpoint — it is provenance. Provenance already has a home: `observation.environment` and
`observation.details` are `TEXT` columns that exist today (`schema.rs:116-117`), and
`extractor_run` records one row per import.

**Consequence: Slice 11a needs no new `EntityKind`, no schema change and no migration.** The only
vocabulary addition is one `Relation`. That is a materially smaller change than the plan implied, and
smaller is the point.

One deliberate non-reuse: `extractor_run.status` stays a statement about **Nerve's own** file
processing. The *traced run's* completion state goes in `observation.environment`. Two different
partialities must not share one column, or a partial trace read whole by Nerve would report
`complete`.

### 2.3 `Directness::Direct` is wrong. It is `Resolved`

The plan §2 argues:

> "`Directness::Inferred` is **wrong** here and `Direct` is right: the tracer *observed* the call."

`coverage.rs:20-21` already settled the shape of this question:

> "Directness is `Inferred`: a line hit is not a statement that a symbol is covered. A mapping step —
> line to enclosing symbol — concludes it."

A trace artifact names **locations**, and ingestion maps a location to a symbol entity. That is the
same mapping step. `Direct` means *"the artifact literally states it"* (`vocab.rs:442`), and the
artifact does not literally state `nerve_index::pystruct::extract_module` — it states a file and a
line.

But `Inferred` is also wrong, and the difference from coverage is real and worth stating:

- **Coverage infers the relation.** A line hit does not say "covered"; a rule concludes it.
- **A trace does not infer the relation at all.** The call is stated outright. Only the *endpoints*
  need resolving.

`Directness::Resolved` — *"Derived through a resolution step"* — is exactly that, and is the value
`AST_RESOLVED` uses when import resolution names a target. **`TEST_CALL_TRACE` / `RESOLVED`.**

## 3. Why the row splits, and why hand-written artifacts come first

**11a — the artifact contract and its ingestion.** Fixtures are **hand-written artifacts.**

That is not a shortcut; it is the established method and it is strictly better here.
`fixtures/ts-coverage/README.md:7` records the same choice for the same reason:

> "`coverage/lcov.info` is written by hand rather than generated, so that every `DA:` line is a
> [deliberate case]."

A real tracer cannot emit a traversal path, a 10 MB record, a duplicate run id, malformed UTF-8 or a
prompt-injection payload. **Every T9 case in §5 of the Slice 11 plan is only reachable from a
hand-written artifact.** Generating fixtures from a tracer would make the security half of the slice
untestable.

**11b — the reference producer.** `tracers/python/nerve_trace/`, a pure-standard-library
`sys.monitoring` tracer, plus a real `pytest` run end to end, plus the criterion that no argument or
return value is *capturable*. It is new non-Rust product surface with its own tests and its own
versioned output; it is a slice, not an appendix.

Splitting is this project's habit for exactly this reason — 8b, 9 and 10 all split, and 8b split
because writing tool tests against a moving contract is a mistake. 11b's tracer must be written
against a **frozen** artifact contract, which is what 11a produces.

## 4. The artifact contract

`nerve-trace/v1`, newline-delimited JSON. One header object, then one record per line. NDJSON rather
than one JSON document because a tracer streams and may be killed mid-run — a truncated NDJSON file
still has valid records above the break, which is what makes an honest `partial` possible.

### Header (first line)

```json
{
  "format": "nerve-trace",
  "format_version": 1,
  "producer": "nerve-trace-python",
  "producer_version": "0.1.0",
  "repository_root_name": "nerve",
  "git_commit": "…40 hex…",
  "content_merkle": "…blake3…",
  "run_id": "…producer-chosen, unique per run…",
  "test_framework": "pytest",
  "runtime": "cpython",
  "runtime_version": "3.12.4",
  "platform": "darwin-arm64",
  "started_at": "…RFC3339…",
  "completed_at": "…RFC3339 or null…",
  "completion_state": "complete | partial | crashed",
  "partial_reason": "…or null…",
  "source_map_state": "none | applied | unavailable",
  "producer_limitations": ["native-frames", "threads", "async-continuations"]
}
```

`repository_root_name` + `git_commit` + `content_merkle` are the **binding triple** (§5). Both state
fields may be `null`; a `null` is honest and is handled, but it downgrades the binding and is
reported. `completed_at: null` with `completion_state: "complete"` is a **contradiction and is
rejected** — a producer that claims completion must say when.

### Record (each subsequent line)

```json
{
  "test_id": "tests/test_parse.py::test_basic",
  "caller_file": "src/parse.py", "caller_line": 12,
  "callee_file": "src/lex.py",   "callee_line": 40,
  "count": 3,
  "worker": "pid:412/thread:main",
  "async_context": null,
  "resolution": "located | unresolved",
  "unsupported_form": null
}
```

`count` is the number of times the producer observed this edge under this test. It is **evidence of
frequency, never of importance**, and nothing ranks by it.

**Fields deliberately absent, per the brief's "do not require fields the producer cannot honestly
provide" and §5 of the Slice 11 plan:** argument values, return values, locals, exception values,
source text, timings per call, and any symbol *name* for the endpoints. Names are absent on purpose —
including one would invite resolving by name, which §6 forbids. A frame is a **location**.

### Policy on unknown fields

**Unknown top-level keys in the header are rejected; unknown keys in a record are ignored and
counted.** The asymmetry is deliberate: a header key Nerve does not understand may change the meaning
of the whole file (imagine `"paths_are_absolute": true`), whereas an unknown record key is at worst
one extra datum about one edge. The count is reported, so a systematically-ignored field is visible
rather than silent.

## 5. Repository-state binding — the load-bearing invariant

A trace made for state A must never silently become evidence for state B.

| case | behaviour |
|---|---|
| `git_commit` and `content_merkle` both match the index | **bound**, full confidence |
| `content_merkle` differs, `git_commit` matches | **bound, stale** — reported, not refused |
| neither state field present | **bound, unverified** — a distinct third value, never "fresh" |
| `repository_root_name` differs | **refused.** The artifact is about another repository |
| a named source or target file is not indexed | that record is **refused and counted**; the import continues |
| a named file's content hash differs from the index | that record is **refused and counted** — the Slice 6b lesson: never map through stale extents |
| the whole artifact predates a re-index | every record refused on the hash check, and the import reports zero mapped with a reason |

`bound / stale / unverified` is a three-valued answer, not a boolean, for the same reason
`CoverageEvidence::Absent` and the `Unverified`-vs-`Stale` split in `nerve check` exist: **absence of
verification is not verification of absence.**

Path handling reuses the **shared** traversal refusal from Slice 8b-i — the one helper all surfaces
call. No second implementation. Absolute paths, `..` segments and backslash spellings are refused as
refusals, never as "not found".

## 6. Identity resolution — one resolver, no name matching

A record names a **file and a line**. Resolution is: find the indexed symbol whose occurrence extent
contains that line in that file. This is the *same* mapping `coverage_ingest.rs` performs, and it
reuses that path rather than adding a second one.

- Two symbols containing the line (a nested function) → the **innermost**, stated as the rule.
- No symbol contains the line (module top level, a comment) → **unresolved, counted by form**.
- A name is **never** used to resolve. The artifact does not even carry one.

## 7. Idempotence and replacement

| action | behaviour |
|---|---|
| import the same artifact twice | **no duplicate observations.** `idx_observation_identity` already keys on `(assertion, state, extractor_id, version, source_type, file, lines)`; the import is idempotent by construction, as a re-index is |
| two shards of one run | both import; observations accumulate on the same assertions |
| two runs of the same test | both import; `run_id` distinguishes them in `environment` |
| a corrected artifact with the same `run_id` | **imported, and the conflict is reported.** Nothing is silently overwritten. Detection is **repository-wide and counted once per artifact** — see below |
| re-index after import | trace observations are withdrawn by repository state like any other |
| a failed import | **nothing commits.** One transaction, and a test asserts the database is byte-identical after a rejected artifact |

### Where a replayed `run_id` is detected, corrected in 11a-i

11a shipped this site-scoped: `merge_runs` compared the run about to be written against the runs
already stored **on that call site**. `fixtures/trace-hostile/duplicate-run-id.jsonl` walks straight
past that by replaying the id on a *different* edge — no stored observation ever sees it, so nothing
was reported and the fixture's declared refusal never fired.

The scope was wrong because the harm was misplaced. The original argument was that a replay
overlapping no earlier site "cannot overwrite anything either", which is true and beside the point:
the damage is not overwriting, it is that `run_id` **stops naming one run**. A reader asking what
`run-bound-1` observed then receives the union of two runs, at every site either artifact touched, and
is told nothing about it.

Run identity is a property of the repository, so the check is one too:
`nerve_store::environments_for_extractor` reads every environment this extractor has written and the
import compares `(run_id, artifact_content_hash)` — same id and same bytes is a **re-import**, which
stays a silent no-op; same id and different bytes is a **conflict**. Counted **once per artifact**,
because the collision is one fact about one header, and reported even when no record survives
resolution. `merge_runs` still keeps both entries and now counts nothing.

Measured on the CLI: the replay reports `run-id-conflict 1` and exits **3**, the six legitimate edges
survive unchanged, and the collision is visible in the evidence itself — one `run_id` against two
artifact hashes, both artifact paths named.

## 8. `TEST_OBSERVED_CALL` is not in `impact::DEFAULT_RELATIONS`

Slice 10a *did* add `SERVED_BY` to the default set. This is the opposite decision, and the
distinction is principled rather than convenient:

- **A route registration is a static declaration in the source.** It is present on every run, and
  reading it twice gives the same answer.
- **A trace observation is existential.** It says one run, in one environment, took this edge. It
  says nothing about the next run.

`nerve impact`'s default answers *"what does the source say depends on this?"* Silently mixing
single-run evidence into that answer would change what the answer means, which is the failure mode
`COVERS`'s exclusion note already describes. It remains available explicitly via `--relation`, and
that is where its real value lies: Python's measured **42.3%** unresolved call rate is exactly the gap
a trace can fill, and a user asking for it should get it **because they asked**.

## 9. Acceptance criteria (11a)

1. `Relation::TestObservedCall` appended (`ALL` 11 → 12); every exhaustiveness test states it,
   including the UI mirror. `TestCallTrace` emitted for the first time; **no `EvidenceSourceType`
   member added, no ordinal moved**.
2. `TEST_CALL_TRACE` / `RESOLVED`, with §2.3's reasoning recorded at the site.
3. **No schema change, no migration, no new `EntityKind`, no new dependency.** `Cargo.lock` stays 101.
4. `no_subprocess.rs` and `no_network.rs` pass **unmodified** — `git diff` on both is empty.
5. `nerve trace-tests` does not exist; the refusal is documented.
6. Assertion endpoints are `(caller, callee)`; **a nested call is attributed to its real caller**, and
   a test asserts that the test function is *not* made the source of a depth-2 edge.
7. `bound` / `stale` / `unverified` are three distinct reported states.
8. A record naming an unindexed or changed file is **refused and counted**; the import continues.
9. An artifact from another repository is **refused whole**.
10. A rejected artifact leaves the database **byte-identical**.
11. Re-importing the same artifact adds **zero** observations.
12. **No `COVERS` observation ever becomes a `TEST_OBSERVED_CALL`**, asserted over `Relation::ALL`;
    and the trace extractor asserts no other relation, likewise over `Relation::ALL`.
13. A `partial` or `crashed` run is labelled partial everywhere its evidence is reported.
14. Every unresolved/unsupported form counted by form, and the tally asserted — and **every counted
    form has a producing case in the fixture**, the 10a lesson.
15. T9 attacked with hostile artifacts: traversal in three spellings, absolute path, oversized file,
    oversized record, oversized string, deep nesting, duplicate run id, malformed UTF-8, SQL-injection
    and FTS5 strings, prompt-injection text, cross-repository substitution, state substitution.
16. Full gate: fmt, clippy `-D warnings`, `cargo test --workspace --no-fail-fast`, release build.

## 10. Non-goals for 11a

The Python tracer (11b). JavaScript. Runtime tracing. HTTP and MCP surfacing of trace-specific
queries — `nerve why`, `nerve path` and `nerve impact --relation` already reach the new evidence
through the shared services, and a trace-specific MCP tool would be `investigate` with a filter,
which is the reason 8b-ii dropped a tool.
