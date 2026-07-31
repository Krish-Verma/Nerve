# Nerve — Project Instructions

These instructions are binding for every session. They override default behavior.
If a future session loses context, re-read this file plus `docs/ROADMAP.md` before doing anything.

---

## 1. Clean-room rule (non-negotiable)

Nerve is an **independent implementation**. See `docs/CLEANROOM.md` for the full statement.

**Never:**
- Depend on, embed, fork, vendor, or import CodeGraph, Graphify, GitNexus, or any competing
  code-knowledge-graph product.
- Read, consume, or migrate a competitor's database, API, or on-disk format.
- Copy, translate, adapt, or paraphrase competitor source code.
- Recreate a competitor schema or algorithm from memory.
- Build compatibility shims around a competitor.
- Require a user to install a competitor product.
- Open competitor source code during implementation.
- Invoke the installed `graphify` skill, or any competitor-specific skill, to guide implementation.

**Allowed:** permissively licensed foundational libraries (parsers, databases, hashers,
watchers, protocol SDKs, UI frameworks), and general software-engineering concepts
(AST parsing, symbol indexing, graph traversal, incremental invalidation, content-addressed
caching, file watching, full-text search, evidence provenance, MCP tools, static analysis).

Every third-party dependency must be recorded with its license in `third_party/LICENSES.md`.

If competitor-derived detail is ever discovered in the codebase, **stop, report it, remove it.**

---

## 2. Offline-first (non-negotiable)

The core product must work with **no** cloud account, API key, external model, telemetry,
analytics, source upload, or network connection after dependencies are installed.

- No network calls during `init`, `index`, `status`, `search`, or any query path.
- No telemetry. No analytics. Not opt-out — **absent**.
- Any local server binds `127.0.0.1` only.
- Never call an external LLM from product code.

There is a test that asserts the indexing path performs no network I/O. Keep it passing.

---

## 3. Evidence-model principles

- Separate **Entity / Occurrence / Assertion / Observation / AssertionState / IdentityLink /
  ExtractorRun / RepositoryState**. Do not collapse them into one edge table.
- Extractors emit **Observations only**. No extractor may write `assertion_state`;
  it is derived and rebuilt as a pure function of observations.
- No generic `confidence: float`. Use structured evidence profiles
  (source type, directness, extractor id+version, measured precision, match quality,
  freshness, repository state, environment). See `docs/decisions/ADR-0003-evidence-model.md`.
- **Coverage is not a call graph.** `TEST_COVERS_SYMBOL` must never be relabelled as a call
  relationship. See `docs/decisions/ADR-0005-coverage-is-not-calls.md`.
- Unresolved references are recorded explicitly, never silently discarded.
- Identity is never established by fuzzy name matching alone.

---

## 4. Working style

- **Bite-sized vertical slices.** One slice at a time.
- **Stop after each slice.** Report, then wait. Never begin the next slice automatically.
- **Push back with evidence** — repository facts, library docs, tests, measurements,
  licensing terms, observed behavior. Do not agree merely to be cooperative.
- Delegate focused implementation to a fresh subagent per slice; the orchestrator reviews
  every changed file, runs verification, and owns correctness.
- Never claim verification that was not performed. Never hide failures.

---

## 5. Verification gate (every slice)

A slice is not done until all of these have been **run and shown**:

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cargo build --release
```

Plus: manual CLI smoke test, inspection of generated output, dependency-license review,
and a clean-room check.

---

## 6. Current status

See `docs/ROADMAP.md` for the authoritative slice list and what is done.

---

## 7. Reporting requirement

End every slice with: summary, files changed, architecture decisions, exact commands run,
results (pass/fail/ignored counts, timings, DB size, graph counts), safety + clean-room checks,
git commit hash, remaining issues, and exactly one recommended next slice.
