# Slice 2b — Graph query surface · completion report

**Date:** 2026-07-31 · **Status:** Complete · **Plan:** `docs/plans/slice-02b-query-surface.md`

---

## Summary

The graph Slice 2a produces is now inspectable. `nerve path` finds bounded-depth connections
between two entities; `nerve why` shows every observation behind a relationship — source type,
directness, extractor id + version, `file:line` — and **re-hashes the file on disk** to report
whether that evidence is still fresh.

`nerve why` is the command that expresses the product thesis: it is the difference between a
claim and an inspectable, falsifiable claim.

## Files changed

**New**

| Path | Purpose |
|---|---|
| `crates/nerve-store/src/select.rs` | Selector resolution — 4 stages, ambiguity is a refusal |
| `crates/nerve-store/src/graph.rs` | `find_paths` (bounded simple-path BFS) and `explain` (evidence assembly) |
| `crates/nerve-store/src/freshness.rs` | `FileProbe` / `FileProber` / `Freshness` / per-path cache |
| `crates/nerve-index/src/probe.rs` | `RepositoryProber` — the query-time file read under Slice 1 path rules |
| `crates/nerve-store/tests/graph.rs` | 20 traversal / evidence / selector tests |

**Modified** — `nerve-cli/src/main.rs` (`path`, `why`, renderers), `nerve-cli/tests/cli.rs` (+11),
`nerve-index/tests/safety.rs` (+8 query-time path-safety tests), `nerve-store/src/lib.rs`,
`nerve-index/src/lib.rs`, `nerve-store/tests/scale.rs` (path-walk measurement, additive),
`docs/ARCHITECTURE.md`.

**Untouched, as required** — `schema.rs`, `derive.rs`, `write.rs`, `bind.rs`, `refs.rs`,
`exports.rs`, `extract.rs`, `pipeline.rs`, `resolve.rs`, **all fixtures**, `Cargo.toml`,
`Cargo.lock`, `third_party/`. Confirmed by `git diff --stat`: empty for each.

## Architecture decisions

1. **Freshness is injected, not performed by `nerve-store`.** `nerve-store` cannot call
   `nerve-index` — the dependency runs the other way — and the path-safety choke point is
   `nerve-index::discover::canonical_child`. So `explain` takes a `&dyn FileProber`, and
   `nerve-index::RepositoryProber` implements it by reusing the existing helpers verbatim.
   Evidence assembly stays in the store; path safety stays with the crate that owns the root.
2. **Freshness has five values, not three.** `fresh` / `stale` / `file-missing` plus `refused`
   (failed the safety check) and `unreadable` (oversize or not a regular file). Reporting a
   refused symlink as `file-missing` would be a lie about which check fired.
3. **The prober refuses symlinks outright**, not only escaping ones — discovery never indexes a
   symlink, so a path that is one now was swapped after indexing.
4. **A search budget with an honest `truncated` flag.** Simple-path enumeration is exponential;
   `MAX_EXPANSIONS = 100_000` bounds it. "No path within depth N" and "I gave up" render
   differently and are separate fields in `--json`.
5. **Relation filters are inlined from `Relation::as_str()` literals**, mirroring `derive.rs`.
   No user text reaches SQL; selector values are bound parameters.

## Verification — run by the orchestrator

```
cargo fmt --all -- --check                              → clean
cargo clippy --workspace --all-targets -- -D warnings   → 0 warnings
cargo test --workspace                                  → 261 passed, 0 failed, 1 ignored
cargo build --release                                   → Finished
```

Test totals: cli 26 · nerve-cli unit 3 · no_network 3 · nerve-core 20 · nerve-index lib 113 ·
index/graph 18 · precision 5 · safety 26 · nerve-store lib 18 · store/graph 20 · schema 9 ·
scale 0 (**1 ignored** — the opt-in scale test).

### Security — independently constructed attacks, not just the shipped tests

`nerve why` is the first command that opens a repository file at query time, using a path taken
from the database. The orchestrator built the attacks by hand:

| Attack | Result |
|---|---|
| Replace an indexed source file with a **symlink to a secret file outside the root**, then run `nerve why` | `freshness refused`. Grep of the `--json` output for the secret's contents: **0 matches** |
| Replace the whole `src/` directory with a **symlink to a directory outside the root** | `freshness refused` |
| `nerve why "x'; DROP TABLE entity;--"` and `"' OR 1=1--"` | exit 2 (not found); `entity` table intact at 85 rows |
| Hash the database before and after running `why` and `path` | **byte-identical — queries are read-only** |

Baseline confirmed first: the same file reported `freshness fresh` before the swap, so the
`refused` result is the guard firing, not an unrelated failure.

### Freshness, verified end-to-end

Before editing, all five `CALLS` observations on `add` report `fresh`. After appending one line
to `src/math.ts`, **only the `src/math.ts:4` observation flips to `stale`**; the observations in
`app.ts` and `legacy.cjs` stay `fresh`. Per-observation freshness works, and the file hash is
computed once per distinct path.

### Traversal latency at depth 4 — measured, with a caveat

ADR-0001 budget: p95 < 200 ms at depth 4 on the 200 k-entity / 2 M-assertion synthetic graph.

| run | machine load | CTE p95 (Slice 1) | `nerve path` p95 (2b) | FTS p95 |
|---|---|---|---|---|
| 1 | 41.1 | 71.20 ms | 83.45 ms | 0.61 ms |
| 2 | 36.3 | 38.84 ms | 120.46 ms | 0.98 ms |
| 3 | 30.4 | 68.83 ms | 104.53 ms | 0.76 ms |

**All three runs pass.** Under contention observed latency ≥ true latency, so the minimum is the
tightest valid upper bound: `nerve path` depth-4 p95 **≤ 83.45 ms** against a 200 ms budget.

**Honest caveat.** The measurement was taken on a machine under sustained load average 26–51
(external processes, not Nerve). An earlier run on this machine **failed**, reporting a CTE p95
of 1004 ms. That measurement is attributable to contention rather than to Slice 2b, on direct
evidence: the failing figure is the **Slice 1 recursive-CTE** measurement, whose code Slice 2b
does not touch (the `scale.rs` diff is purely additive), and which Slice 1 recorded at 23.05 ms.
Unchanged code cannot regress 43×. The subagent reported 19–22 ms from a quieter window; **that
figure is not repeated here as verified**, because the orchestrator could not reproduce quiet
conditions. The numbers above are the conservative ones actually observed.

**Recorded limitation:** the scale test is load-sensitive and can fail spuriously on a busy
machine. It is `#[ignore]`d and does not gate CI, but its threshold assertion should be
interpreted alongside machine state. Worth making it report load or take a best-of-N in a later
slice.

## Manual CLI verification

All six required scenarios were run by the orchestrator on a copy of `fixtures/ts-resolution`
(committed fixture untouched):

- **Multi-hop path** — `viaBarrel → add → normalize`, 2 hops, each edge showing `AST_RESOLVED`
  and its `file:line`. exit 0
- **No path** — `No path found within depth 6 (forward, 1 partial path(s) explored).` exit **0**
  (absence is not an error)
- **Ambiguous selector** — `"area" matches 3 entities; nothing was chosen`, listing
  `Bubble.area`, `Circle.area`, `Rectangle.area` with their ids. exit **10**. Nothing guessed.
- **Missing selector** — `"normalise" matches no indexed entity` with `did you mean: normalize`.
  exit **2**
- **`why` on a resolved CALLS edge** — full evidence packet with extractor `ts-js-reference
  1.0.0`, `AST_RESOLVED / RESOLVED`, `file:line`, details, freshness
- **`why` after mutation** — correct per-file `stale` / `fresh` split

## Clean-room, dependencies, schema

No new dependencies — `Cargo.toml`, `Cargo.lock`, `third_party/LICENSES.md` byte-identical.
Zero networking crates; `no_network` green. No competitor reference anywhere in the new code.
**No schema change** — schema stays v1, no migration; this slice is read-only over existing
tables.

## Known limitations

- **Path enumeration is unindexed BFS over `assertion`.** Sound at depth 4 on 2 M edges; depth 6
  on a dense graph will hit the budget and report `truncated: true` rather than a wrong answer.
  A materialized adjacency layer remains its own slice.
- **Freshness is whole-file**, because `observation.content_hash` is the file hash. Any edit to a
  file marks every observation in it stale, even ones far from the change. Correct but coarse.
- `--limit` counts distinct node-and-edge sequences, so a pair connected by two relations
  consumes two slots.
- `why` on a single entity has no `--limit`; a large module reports every edge.
- Suggestion fallback is prefix-based, so a typo in the first three characters yields nothing.
- The "no traversal logic in the CLI" test is a source-text check — it catches regression by
  inspection, not by the type system.
- The scale test's load sensitivity, above.

## Result

All eight acceptance criteria met. Nothing in scope was dropped; no test was weakened.

**Next slice:** 3 — incremental indexing, which turns the `stale` reading `why` now produces
into an actionable refresh.
