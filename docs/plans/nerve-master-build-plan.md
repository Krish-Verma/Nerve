# Nerve — Master Build Plan

**Date:** 2026-07-31 · **Supersedes:** `docs/plans/2026-07-31-claude-architecture-plan.md` (partially — see §2)

---

## 1. Product thesis

> Nerve builds a local, offline evidence graph of a software system in which **every conclusion
> is backed by inspectable evidence, scoped to a repository state, and able to say when the
> evidence is missing, stale, uncertain, or contradictory.**

The differentiator is not graph size, language count, or token savings. It is **epistemic
honesty made queryable**: the schema itself distinguishes "a parser saw this token", "a type
checker resolved this", "a framework rule inferred this", "a test executed this", and "a human
confirmed this" — and every query can filter on that distinction.

---

## 2. What is superseded from the prior plan

The prior architecture plan is retained for its competitive and licensing research, which
remains valid. The following recommendations in it are **explicitly withdrawn**:

| Withdrawn | Reason |
|---|---|
| "Prototype consuming CodeGraph's `.codegraph/codegraph.db` as a static-graph source" (Open Decision #7) | Violates the clean-room rule. Nerve owns its engine from the parser boundary upward. |
| "Position Nerve as complementary to CodeGraph; do not compete" | Superseded by the strategic decision that Nerve is an independent product. |
| "Consume CodeGraph as an optional static-graph adapter" anywhere in the roadmap | Withdrawn entirely. |
| TypeScript/Node as the implementation language | Superseded by Rust (ADR-0004). |
| "Missed-edge discovery via coverage" as the wedge experiment | **Technically wrong.** See §3.1 — coverage cannot establish call edges. |

Also withdrawn: the recommendation to benchmark GitNexus. Its PolyForm Noncommercial license
makes benchmarking-during-commercial-development a commercial use. Nerve will not benchmark it.

---

## 3. Disagreements and Pushback

### 3.1 I was wrong, and §9 of the build prompt is right — coverage is not a call graph

My prior plan proposed that ingesting per-test coverage would "discover the dynamic-dispatch
edges the static resolver missed." **That claim does not survive scrutiny and I withdraw it.**

Line/branch coverage produces a *set of executed regions per test*. From
"symbols A and B both executed during test T" you cannot derive "A calls B" — they may be
siblings called independently, or connected through frames you never observed. Deriving call
edges from co-execution is the same class of error as inferring a relationship because two
names look similar; it is merely more expensive to compute.

**Consequence for the roadmap.** Coverage supports exactly one honest relation:

```
Test T  TEST_COVERS_SYMBOL  Symbol S
```

Real observed call edges require actual call instrumentation, and the available mechanisms
have materially different evidence properties:

| Mechanism | Evidence property |
|---|---|
| V8 `--cpu-prof` / Inspector `Profiler` domain | **Sampled.** Yields probabilistic call trees; fast frames are missed. Must carry a sampling rate and must never be presented as complete. |
| V8 `Debugger`/instrumentation breakpoints | Deterministic but very slow; unusable on real suites. |
| Source transform wrapping function entries | Deterministic, but changes timing and semantics, and pollutes the artifact under test. |
| Python `sys.setprofile` | Deterministic call/return events, moderate overhead. Best-in-class among these. |

None of these is coverage. Therefore:

- **Slice 6 ships `TEST_COVERS_SYMBOL` only** and is labelled as such in the schema, CLI, and UI.
- `TEST_OBSERVED_CALL` becomes a separate, later slice with its own evidence source type,
  its own sampling metadata, and its own negative fixtures.
- The affected-test experiment is still worth running, but its honest framing is
  *"does coverage-derived test↔symbol evidence beat the changed-files heuristic?"* — not
  *"does it discover missing call edges?"*

**Blocks Slice 1?** No. It corrects Slice 6 and later.

### 3.2 `assertion_state` in Slice 1 carries no information — build it anyway, but prove it is derived

With one extractor and four relations, there is exactly one observation per assertion, so
`assertion_state` is a pure projection with zero information content. It would be reasonable
to defer it.

I recommend building it now regardless, for one reason: it establishes the invariant
*"no extractor writes assertion state"* while the codebase is small enough that the invariant
is free. Retrofitting that boundary later is expensive and is exactly the kind of thing that
silently rots.

**Condition:** it must be implemented as a **pure rebuild from observations**, with a test
that truncates `assertion_state`, rebuilds, and asserts byte-identical content. If it is not
provably derived, it is just a second source of truth.

**Blocks Slice 1?** No — it shapes it.

### 3.3 `CALLS` must not ship in Slice 1

The prompt permits `CALLS` "only if it can be implemented honestly without fragile
heuristics." It cannot, yet. Honest cross-file call resolution needs import resolution,
lexical scope and shadowing analysis, and method-receiver typing. Without those, a `CALLS`
edge is a name match — and shipping name-matched call edges would poison the precision claim
that is the entire product thesis, in the very first slice.

**Recommendation:** Slice 1 ships `CONTAINS`, `DEFINES`, `IMPORTS`, `EXPORTS` only.
`CALLS`/`REFERENCES`/`EXTENDS`/`IMPLEMENTS` land in Slice 2 **together with negative
fixtures and a measured precision number.**

**Blocks Slice 1?** No — it narrows it, deliberately.

### 3.4 Six crates is premature; four have real boundaries

`nerve-core` (model, ids, evidence vocabulary), `nerve-store` (SQLite, migrations, queries),
`nerve-index` (discovery, parsing, extraction), `nerve-cli` (binary). `nerve-query` and
`nerve-server` have no distinct boundary yet — query logic is three functions in `nerve-store`
today, and there is no server. Splitting now creates circular-dependency pressure and churn.

**Blocks Slice 1?** No.

### 3.5 Identity will include the file path, and that is a known defect

Without package resolution (tsconfig `paths`, workspace layouts, `node_modules` semantics),
two `function handle()` in different files are indistinguishable unless the path participates
in the identity. So Slice 1's `EntityId` includes the module-relative path — which means
**a file move changes every symbol id in that file.**

This is a real, documented limitation, not a solved problem. It is bridged in Slice 3 by
`IdentityLink` rows carrying body-hash + git-lineage evidence. ADR-0002 records the failure
modes explicitly rather than claiming identity is solved.

**Blocks Slice 1?** No.

### 3.6 Highest-risk assumptions that must be measured, not argued

1. **SQLite traversal at scale.** Recursive CTE, depth 4, ~2M assertions, p95 < 200 ms.
   Measured in Slice 1 via a reproducible synthetic scale test. If it fails, the schema is
   still fine but the traversal strategy needs a materialized adjacency layer.
2. **FTS5 availability in the bundled SQLite build.** Asserted by a runtime test, not assumed.
3. **Tree-sitter TSX/JSX grammar fidelity** on realistic files, including type-only imports,
   `export * as ns from`, decorators, and generics.
4. **Determinism.** Parallelism and hash-map iteration order are the usual sources of
   nondeterminism; a byte-identical double-index test is the only reliable guard.

### 3.7 Where I think the plan is simply sound

The eight-concept evidence model, the extractors-emit-observations-only rule, the rejection
of scalar confidence, offline-first with no telemetry, SQLite-by-default with a scale test
gating it, and stopping after each slice — these are correct and I am adopting them without
modification.

---

## 4. Stack

| Layer | Choice | License | Rationale |
|---|---|---|---|
| Engine | **Rust 1.97.1** | — | Single static binary, no user-installed runtime; native Tree-sitter; predictable performance and memory. ADR-0004. |
| Parsing | `tree-sitter` + `tree-sitter-typescript` + `tree-sitter-javascript` | MIT | Canonical implementation; TS grammar crate provides both TS and TSX. |
| Storage | `rusqlite` (`bundled`) | MIT | Bundling SQLite fixes the version and feature set across platforms — no system-SQLite variance. FTS5 verified by test. |
| CLI | `clap` (derive) | MIT/Apache-2.0 | Standard; generates help and completions. |
| Serialization | `serde`, `serde_json`, `toml` | MIT/Apache-2.0 | Stable JSON output contract. |
| Hashing | `blake3` | CC0-1.0/Apache-2.0 | Fast, stable content addressing. |
| File discovery | `ignore` | MIT/Unlicense | `.gitignore` semantics, parallel walk, no symlink following by default. |
| Errors | `thiserror`, `anyhow` | MIT/Apache-2.0 | Typed library errors, ergonomic binary errors. |
| Tests | `tempfile` | MIT/Apache-2.0 | Isolated filesystem fixtures. |

**No async runtime in Slice 1.** Indexing is CPU- and disk-bound; Tokio would add surface area
with no benefit. It will be introduced only when `nerve serve` exists and needs it.

**No networking crate anywhere in the dependency tree.** This is enforced by a test.

---

## 5. Architecture

```
                    ┌──────────────────────────────────────────┐
                    │  Surfaces (thin — no business logic)     │
                    │  nerve-cli   [Slice 4: server, UI]       │
                    │              [Slice 8: MCP]              │
                    └───────────────────┬──────────────────────┘
                                        │ application services
                    ┌───────────────────▼──────────────────────┐
                    │  nerve-index                             │
                    │  discovery → parse → extract → persist   │
                    │  emits Observations ONLY                 │
                    └───────────────────┬──────────────────────┘
                                        │
                    ┌───────────────────▼──────────────────────┐
                    │  nerve-store   SQLite + migrations       │
                    │  derives assertion_state (pure rebuild)  │
                    └───────────────────┬──────────────────────┘
                                        │
                    ┌───────────────────▼──────────────────────┐
                    │  nerve-core    ids · kinds · evidence     │
                    │  vocabulary · errors · canonical dump    │
                    └──────────────────────────────────────────┘
```

**Load-bearing invariant:** extractors produce `Observation` values. Nothing outside
`nerve-store::rebuild_assertion_state` may write `assertion_state`. A buggy extractor can add
filterable noise; it cannot corrupt the derived view.

---

## 6. Data model (Slice 1 subset)

Full schema in ADR-0003. Tables: `schema_version`, `repository`, `repository_state`,
`entity`, `occurrence`, `assertion`, `observation`, `assertion_state`, `extractor_run`,
`identity_link` (created, unused until Slice 3), `entity_fts` (FTS5).

Entity kinds: `Repository`, `Directory`, `File`, `Module`, `Function`, `Method`, `Class`,
`Interface`, `Unresolved`.

Relations: `CONTAINS`, `DEFINES`, `IMPORTS`, `EXPORTS`.

Evidence source types (vocabulary defined now, only the first used in Slice 1):
`AST_DIRECT`, `AST_RESOLVED`, `AST_HEURISTIC`, `TYPE_RESOLVED`, `FRAMEWORK_RULE`,
`TEST_COVERAGE`, `TEST_CALL_TRACE`, `RUNTIME_CALL_TRACE`, `DOCUMENT_STATED`,
`HUMAN_CONFIRMED`, `LLM_DERIVED`.

**Unresolved imports** are modelled as a real `Unresolved` entity plus an `IMPORTS` assertion,
so they are queryable rather than discarded, and `assertion_state.is_unresolved` is set.

---

## 7. Repository layout

```
nerve/
├── Cargo.toml · rust-toolchain.toml · .gitignore · CLAUDE.md · README.md
├── crates/{nerve-core,nerve-store,nerve-index,nerve-cli}/
├── fixtures/ts-basic/
├── docs/{PRODUCT,ARCHITECTURE,CLEANROOM,ROADMAP,TESTING,SECURITY}.md
├── docs/{plans,decisions}/
├── scripts/
└── third_party/LICENSES.md
```

---

## 8. CLI plan

Slice 1: `nerve init [path]`, `nerve index [path]`, `nerve status`, `nerve search <query>`.
Later: `sync`, `path`, `impact`, `affected`, `why`, `gaps`, `check`, `serve`, `mcp`, `doctor`.

Global: `--json`, `--quiet`, `-v/-vv`, `--no-color`.
Exit codes: `0` success · `2` no/unhealthy index · `3` partial index · `10` usage · `70` internal.
Empty search results are exit `0` — absence of matches is not an error.

---

## 9. Visual product roadmap

Slice 4 introduces `nerve serve` (axum, `127.0.0.1` only) plus `apps/nerve-web` (React + Vite),
built assets embedded in the binary. Architecture obligation now: keep all query logic in
`nerve-store`/application services so the UI is a client, never a second implementation.

---

## 10. Security requirements (from Slice 1)

No network. No telemetry. No repository code execution. No package scripts. No `eval`.
Canonicalize every path and assert it is inside the repository root. Do not follow symlinks
out of root. Respect `.gitignore` and `.nerveignore`. Deny-list secret files by default
(`.env*`, `*.pem`, `*.key`, `id_rsa*`, `*.p12`, `*.pfx`, `.npmrc`, `.netrc`, `*.keystore`).
Database file mode `0600` on Unix. Treat all repository content as untrusted data.

Threat model document required before document ingestion (Slice 5) and MCP (Slice 8).

---

## 11. Testing strategy

Parser unit tests · golden-graph tests · determinism (byte-identical double index) ·
idempotent re-index · unresolved-reference · migration · CLI smoke · JSON-output contract ·
path-safety · ignore-rule · FTS5 availability · scale/latency test.

Negative fixtures become mandatory from Slice 2, when the first inferred relations appear.

---

## 12. Slice sequence

1. **Indexing foundation** — init/index/status/search, schema, TS/JS entities, CONTAINS/DEFINES/IMPORTS/EXPORTS.
2. **Static relationship resolution** — CALLS/REFERENCES/EXTENDS/IMPLEMENTS, module resolution, precision fixtures, `path`, `why`.
3. **Incremental indexing** — content hashes, changed-file indexing, importer invalidation, moves/deletes, identity links.
4. **Visual explorer** — local server, overview, search, graph canvas, evidence inspector.
5. **Markdown / ADR evidence** — sections, citations, document↔code identity links.
6. **Test evidence (coverage only)** — `TEST_COVERS_SYMBOL`, freshness, affected-test experiment.
7. **CLI and query expansion** — `impact`, `gaps`, `check`, evidence packets.
8. **MCP** — one default investigation tool.
9+. Python · framework rules · **test call tracing (separate from coverage)** · git history · cross-repo contracts · human-confirmed memory.

---

## 13. Risks

| Risk | P | Impact | Mitigation |
|---|---|---|---|
| Inferred-relation false positives (from Slice 2) | High | High | Negative fixtures + measured precision, gated in CI |
| Identity churn on file moves | High | Medium | Documented in ADR-0002; bridged by IdentityLink in Slice 3 |
| Nondeterministic indexing | Medium | High | Byte-identical double-index test from Slice 1 |
| SQLite traversal at scale | Low | High | Scale test in Slice 1 gates the design |
| Scope creep back to seven layers | High | High | Slice non-goals are contractual; stop-and-report after each |
| Rust ⇄ TS boundary slows Slice 4 | Medium | Low | Embed prebuilt SPA; keep query logic server-side |

---

## 14. Open decisions

| # | Decision | Recommendation | Blocks Slice 1? |
|---|---|---|---|
| 1 | `File` and `Module` as distinct entities | Yes — keeps Python packages / re-exports honest later | No |
| 2 | Repository identity across clones | `project_id` generated at `init`, stored in `.nerve/config.toml` | No |
| 3 | Read git commit for `repository_state` | Read `.git/HEAD` + ref file directly, no subprocess; optional | No |
| 4 | Parallel file parsing in Slice 1 | Serial. Determinism first; parallelize in Slice 3 with an ordered merge | No |
| 5 | Golden-test snapshot library | None — plain canonical JSON files, zero extra dependency | No |
| 6 | Second language | Python, after the model proves language-independent | No |
