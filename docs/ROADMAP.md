# Nerve Roadmap

Authoritative slice list. Update the status column at the end of every slice.
**Never begin a slice without explicit approval.**

| # | Slice | Status |
|---|---|---|
| 1 | Indexing foundation — `init`/`index`/`status`/`search`, SQLite evidence schema, TS/JS entities, `CONTAINS`/`DEFINES`/`IMPORTS`/`EXPORTS` | ✅ Complete (2026-07-31) — 137 tests pass, ADR-0001 gate passed |
| 2a | Static relationship resolution — `CALLS`, `REFERENCES`, `EXTENDS`, `IMPLEMENTS`, lexical binding + import resolution, negative fixtures + measured precision | ✅ Complete (2026-07-31) — 203 tests pass, FP=0 FN=0 on 24 resolved edges |
| 2b | Graph query surface — `nerve path`, `nerve why`, evidence packets | ✅ Complete (2026-07-31) — 261 tests pass, query-time path safety verified by attack |
| 3 | Incremental indexing — changed-file detection, import-closure invalidation, **deletion**, `IdentityLink`, full-vs-incremental equivalence | ✅ Complete (2026-07-31) — 295 tests, equivalence holds over 24 seeded edits; **speed target missed (24.9% vs 20%)** |
| 3b | Normalize repository state out of `occurrence`/`observation` and out of `occurrence_id` — removes the O(repository) restatement pass | ⬜ Not started — **corrective, recommended next**; unblocks Slice 3 §5 |
| 4 | Initial visual explorer — `nerve serve`, overview, search, graph canvas, evidence inspector | ⬜ Not started |
| 5 | Markdown + ADR evidence — sections, citations, document↔code identity links | ⬜ Not started |
| 6 | Test evidence (**coverage only**) — `TEST_COVERS_SYMBOL`, freshness, affected-test experiment | ⬜ Not started |
| 7 | CLI + query expansion — `impact`, `gaps`, `check`, evidence packets | ⬜ Not started |
| 8 | MCP — one default investigation tool | ⬜ Not started |
| 9 | Python language support | ⬜ Not started |
| 10 | Framework rules (routes, events, DI) with negative fixtures | ⬜ Not started |
| 11 | **Test call tracing** — `TEST_OBSERVED_CALL`, distinct from coverage, with sampling metadata | ⬜ Not started |
| 12 | Git history / temporal layer | ⬜ Not started |
| 13 | Cross-repository contracts | ⬜ Not started |
| 14 | Human-confirmed memory | ⬜ Not started |

## Slice 1 — delivered

- 4-crate Rust workspace (`nerve-core`, `nerve-store`, `nerve-index`, `nerve-cli`).
- SQLite schema v1 with the eight-concept evidence model + FTS5 symbol search.
- Tree-sitter extraction for `.ts`, `.tsx`, `.js`, `.jsx`, `.mjs`, `.cjs`.
- Entities: Repository, Directory, File, Module, Function, Method, Class, Interface, Unresolved.
- Relations: `CONTAINS`, `DEFINES`, `IMPORTS`, `EXPORTS`. Unresolved imports retained as first-class entities.
- `assertion_state` proven to be a pure rebuild from observations.
- CLI: `init`, `index`, `status`, `search` with `--json`.
- Security: secret deny-list, `.gitignore`/`.nerveignore`, path-traversal and symlink-escape guards, `0600` DB.

## Deferred out of Slice 1 (deliberate)

`CALLS`, `REFERENCES`, `EXTENDS`, `IMPLEMENTS` — deferred to Slice 2 so they ship with negative
fixtures and a measured precision number rather than as name matches. See master plan §3.3.

## Slice 2 scope split (2026-07-31)

Roadmap row 2 was one row covering a resolver, a measurement apparatus, and two query
surfaces. It is split into **2a** (resolution + relations + measured precision) and **2b**
(`nerve path`, `nerve why`). Rationale, pushback and acceptance criteria:
`docs/plans/slice-02-static-resolution.md`.

## Slice 2a — delivered

- Second extractor `ts-js-reference 1.0.0`, declaring `[AST_DIRECT, AST_RESOLVED]`.
- Lexical binding table with `Opaque` shadowing guard; transitive re-export closure.
- `CALLS`, `REFERENCES`, `EXTENDS`, `IMPLEMENTS` — resolved edges `AST_RESOLVED`, unnameable
  targets recorded as `Unresolved` entities with a closed-vocabulary reason.
- **Measured: FP=0, FN=0 across 24 resolved edges; 38.1% of call sites honestly unresolved.**
  Gate validity confirmed by orchestrator mutation probes against the implementation.
- Corrected a Slice 1 defect: resolved `IMPORTS`/re-export `EXPORTS` were mislabelled
  `AST_DIRECT`; now `AST_RESOLVED` (`ts-js-structural` → 1.1.0).
- Corrected an identity collision: unresolved ids now carry a `module`/`value` category.
- No schema change, no new dependencies. Report: `docs/reports/slice-02a-report.md`.

## Slice 2b — delivered

- `nerve path <from> <to>` — bounded-depth simple-path walk with relation/direction filters,
  an honest `truncated` flag, and exit `0` on "no path" (absence is not an error).
- `nerve why <from> [<to>]` — the full evidence packet per assertion: every observation with
  source type, directness, extractor id + version, `file:line`, details, and **computed
  freshness** (the file is re-hashed at query time).
- Selectors refuse ambiguity: multiple matches exit `10` listing candidates; nothing is guessed.
- Query-time file reads reuse the Slice 1 path-safety choke point. Verified by constructing
  symlink-escape attacks: both file-level and parent-directory escapes are `refused`, with zero
  content leakage. Queries are provably read-only.
- Traversal p95 ≤ 83.45 ms at depth 4 on 2 M assertions, against ADR-0001's 200 ms budget
  (measured under adverse machine load; see report for the attribution of one spurious failure).
- No schema change, no new dependencies. Report: `docs/reports/slice-02b-report.md`.

## Slice 3 — delivered

- **Deletion works.** Before this slice the pipeline only `INSERT OR IGNORE`d, so a removed
  file's entities and edges survived forever and the graph was wrong after any deletion.
- Change classification + **transitive** invalidation closure over `IMPORTS` (a barrel-file edit
  reaches importers that never name the edited file).
- **Equivalence property:** an incremental re-index is byte-identical to a full index of the same
  tree, verified at every step of a seeded 24-edit sequence across six edit kinds.
- `IdentityLink` populated for moves, proposed with evidence; a coincidental name match proposes
  nothing.
- Schema **v2** (additive: `module_facts` cache), with v1→v2 migration tests.
- `nerve index --full`; removals reported loudly.
- **Speed target missed:** 24.9% of a full index on a realistic corpus vs a < 20% target.
  Amplification 1.00. Cause is the O(repository) state-restatement pass forced by `state_id`
  being inside `occurrence_id` (ADR-0002) — hence Slice 3b.
- Two plan deviations accepted after review: P4's tombstone half is superseded by the equivalence
  invariant, and `STALE` falls to the same argument. Report: `docs/reports/slice-03-report.md`.
