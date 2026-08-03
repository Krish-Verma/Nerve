# Nerve Roadmap

Authoritative slice list. Update the status column at the end of every slice.
**Never begin a slice without explicit approval.**

| # | Slice | Status |
|---|---|---|
| 1 | Indexing foundation — `init`/`index`/`status`/`search`, SQLite evidence schema, TS/JS entities, `CONTAINS`/`DEFINES`/`IMPORTS`/`EXPORTS` | ✅ Complete (2026-07-31) — 137 tests pass, ADR-0001 gate passed |
| 2a | Static relationship resolution — `CALLS`, `REFERENCES`, `EXTENDS`, `IMPLEMENTS`, lexical binding + import resolution, negative fixtures + measured precision | ✅ Complete (2026-07-31) — 203 tests pass, FP=0 FN=0 on 24 resolved edges |
| 2b | Graph query surface — `nerve path`, `nerve why`, evidence packets | ✅ Complete (2026-07-31) — 261 tests pass, query-time path safety verified by attack |
| 3 | Incremental indexing — changed-file detection, import-closure invalidation, **deletion**, `IdentityLink`, full-vs-incremental equivalence | ✅ Complete (2026-07-31) — 295 tests, equivalence holds over 24 seeded edits; **speed target missed (24.9% vs 20%)** |
| 3b | Normalize repository state out of `occurrence`/`observation` and out of `occurrence_id` — removes the O(repository) restatement pass | ✅ Complete (2026-07-31) — 306 tests; **24.9% → 2.0%**; counted-writes gate; found and fixed a silent data-destruction bug |
| 4a | `nerve serve` — loopback HTTP, read-only JSON API, T4/T5/T6 security controls | ✅ Complete (2026-07-31) — 427 tests; token/origin/host/traversal/symlink/XSS all attack-verified |
| 4b | `apps/nerve-web` — the visual explorer SPA, asset embedding, screenshot QA | ✅ Complete (2026-07-31) — 435 tests, 31 screenshots reviewed, 0 CSP violations; **fixed a 4a bug that made the UI unloadable** |
| 5a | Markdown + ADR ingestion — `Document`/`Section` entities, `CONTAINS` structure, ADR status, T7 controls | ✅ Complete (2026-08-01) — 506 tests; T7 exhaustive and mutation-verified; **closed two identity-forgery vectors**, one of them pre-existing in every id constructor |
| 5b | Markdown **link scanning** — destinations, forms, spans; code-span mentions counted not emitted | ✅ Complete (2026-08-01) — 541 tests; the tests found 6 real scanner defects, two of them turning hostile HTML into link destinations |
| 5c | Document↔code **link resolution** — `REFERENCES` by explicit path and `#L<n>` anchor, unresolved reasons, measured precision, invalidation | ✅ Complete (2026-08-01) — 564 tests; FP=0 on fixtures, but **only 5 link sites exist across Nerve's own 45 documents** |
| 5d-i | **Corrective** filesystem evidence — `FILESYSTEM_OBSERVED` + `fs-structural`, ADR-0007, schema v4 migration, amended T7 | ✅ Complete (2026-08-01) — 577 tests; a docs-only tree now yields **0** `ts-js-structural` observations, was 4 mislabelled `AST_DIRECT` |
| 5d-ii | `Document SUPERSEDES Document` — explicit evidence only, chains, cycles, ambiguity, measured precision | ✅ Complete (2026-08-01) — 596 tests; FP=0 over a 26-file corpus whose ground truth was written before the resolver; **Nerve's own ADRs state no supersession and produce 0 edges** |
| 5d-iii | UI vocabulary catch-up — glosses driven from the Rust vocabularies, asset re-embed, screenshots | ✅ Complete (2026-08-01) — 610 tests; a new test reads the TypeScript gloss maps and fails when a Rust vocabulary gains a member, so the two cannot drift again; **found 120 real sites rendering fallback text**, and a gloss for a status the backend cannot emit |
| 6a | Coverage **vocabulary + LCOV parser** — `CoverageRun`, `COVERS`, ADR-0008 | ✅ Complete (2026-08-01) — 648 tests; the gating question answered empirically: **LCOV is aggregate**, so the endpoint is a coverage run and `TEST_COVERS_SYMBOL` is refused by the vocabulary |
| 6b | Coverage **ingestion** — `nerve coverage`, line→symbol mapping, freshness, T9 attack tests, equivalence | ✅ Complete (2026-08-02) — 692 tests; T9 verified by attack; **zero call-shaped relations asserted over `Relation::ALL`**; found that coverage surviving a re-index required splitting the pipeline's withdrawal in two, and that a file changed since indexing must be refused rather than mapped through stale extents |
| 7a | `nerve gaps` — the coverage-gap question, CLI + API | ✅ Complete (2026-08-02) — 729 tests; **"no coverage ingested" is a distinct, unanswerable state with `totals: null`**, not a list of every symbol; four states incl. `unmeasured`; CLI and API asserted byte-equal; **fixed a harness defect where a failing server test hung the whole suite instead of failing** |
| 7a-ii | **Corrective** — "Gaps" meant two things across surfaces; SPA view renamed **Unresolved**, new **Coverage** view over `/api/gaps` | ✅ Complete (2026-08-02) — 729 tests, frontend 15 tests, lint clean, assets re-embedded; **screenshot QA performed** (the earlier failure was two Chrome browsers connected at once) — absent/uncovered/unmeasured/partial/stale all verified live, 0 console messages; **narrow-viewport QA still outstanding** (window ignores resize) |
| 7a-iii | **Corrective** — the rail labels `entities_total` as "Symbols". Pre-existing from 4b, made visible by 7a-ii | ✅ Complete (2026-08-02) — 735 tests; canonical `symbols_total` from `EntityKind::is_symbol()` on `/api/overview`, `status --json`, `index --json` and both text outputs; **the symbol-kind SQL list had been generated in three separate places and is now one helper with four call sites**; the invariant is asserted over *every* non-symbol kind, not an example, and all twelve kinds are individually pinned so a new kind cannot default silently; orchestrator mutation probe (`coverage_run` made a symbol) failed **16 tests across 6 targets** |
| 7b | `nerve impact` — reverse dependency closure with evidence | ✅ Complete (2026-08-02) — 771 tests; BFS reverse closure over `CALLS`/`REFERENCES`/`EXTENDS`/`IMPLEMENTS` reusing `graph.rs` adjacency, exact tallies beneath a row cap, cycles terminate with each entity once; **the unresolved account is a field on every answer, printed and serialized even when zero** — on `ts-basic`, `add` has 3 dependants beside 4 unresolved sites; `CONTAINS`/`DEFINES`/`IMPORTS`/`COVERS` excluded from the default with a reason each; `/api/impact` is the 11th route; **the implementation agent was killed mid-slice by an org spend limit and the orchestrator finished the API surface and all 12 CLI/API tests directly** |
| 7c-i | `nerve check` — the CI verdict and its exit codes | ✅ Complete (2026-08-02) — 795 tests; five verdicts over `index_freshness` + `is_healthy()`, one new exit code `STALE_INDEX = 4`, `Unverified` distinct from `Stale` so a truncated sweep is never a clean bill; **the brief was wrong and the implementer proved it with a test** — `index_freshness` iterates `module_facts`, so an *added* file is invisible to it, and `untracked_files` was needed; read-only enforced by `query_only=ON` and verified on the bytes; orchestrator confirmed **zero false positives across all 7 fixtures** and that unsupported file types do not trigger staleness |
| 7c-ii | `nerve doctor` — diagnostics | ✅ Complete (2026-08-02) — 821 tests; 11 checks, one finding per check every run, a check that could not run is `skipped` **with its cause** rather than omitted or reported as passing; closed id vocabulary pinned by two tests; no new exit code (fatal reuses `NO_INDEX`, whose documented meaning already covers it); **`SELECT count(*) FROM entity_fts` was found by probe to read the content table and so can never detect FTS drift** — the `entity_fts_docsize` shadow table is used instead; the no-SQL-in-the-CLI guard was widened from `main.rs` to the whole crate rather than evaded; orchestrator verified no panic on a zero-byte database, a `.nerve` that is a file, a `nerve.db` that is a directory, and a `nerve.db` symlinked to `/etc/passwd` with no content disclosed |
| 8a | MCP — stdio JSON-RPC, `nerve_investigate`, **T7 + T8 gates** | ✅ Complete (2026-08-02) — 862 tests; **zero new crates** (framing hand-rolled on `serde_json`; the async-runtime trade was already measured in 4a and not reopened); every repository-derived value confined to one `repository_content` field and held there by a property test that walks the whole response; three independent response bounds incl. a byte ceiling measured on the text a client actually reads, with exact continuation and the degenerate case named; a traversal-shaped selector refused **as a refusal**, never disguised as "not found"; orchestrator injected a hostile Markdown **heading** and found all 7 occurrences inside labelled regions, 0 leaks |
| 8b-i | **Selector resolution by entity kind** — path names what is at it, qualifiers, shared traversal refusal | ✅ Complete (2026-08-02) — 911 tests; measured first: **26% of entities in `fixtures/md-docs` could not be named by their path** and the failure message suggested strings that resolve to nothing; `EntityKind::path_role()` puts the classification in the vocabulary with all twelve kinds pinned, so a new kind cannot be silently unaddressable; qualifiers are **generated** from `EntityKind::as_str()` rather than hand-listed, the drift 5d-iii and 7a-iii were corrective slices for; a path with two readings resolves by a stated rule and **reports what it passed over** rather than choosing silently, while two readings *inside* one tier stay ambiguous; the traversal refusal became one helper for all three surfaces — **CLI and HTTP previously answered "matches no indexed entity" to `../../etc/passwd`, asserting a check they never ran**; two private copies of `qualified_name` deleted; **two defects inherited from 8a found in review** — `./x` was refused as an escape, `..\..\x` was not refused at all; orchestrator ran all four mutation probes (19/21/8/5 failures) after the implementation agent was interrupted twice by infrastructure limits |
| 8b-ii | MCP — the rest of the tool surface (`search`, `path`, `impact`, `gaps`) | ⬜ Not started |
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

## Slice 3b — delivered

- **ADR-0006**: occurrence and observation identity is state-independent. An occurrence is a
  physical location fact; an observation is evidence about a file at a content hash. Neither
  depends on which run observed it. `content_hash` is the freshness anchor — already how
  `nerve why` worked, so product semantics are unchanged.
- The O(repository) state-restatement pass is **deleted**; `assertion_state` derivation and
  orphan pruning are scoped to what moved, with the whole-table versions kept as the oracle.
- **Slice 3's missed target is met: 24.9% → 2.0%** on the realistic corpus (orchestrator
  measured 2.0 / 2.0 / 2.5% at loads 5.3–9.2).
- **Counted-writes gate**: a one-file leaf edit writes the same 52 rows in a 100-file and a
  520-file repository. Deterministic, not timing-based, and cannot be gamed by a fast machine.
- Schema **v3**, with v1→v3 and v2→v3 migration tests plus an end-to-end check against a real
  v2 database produced by the Slice 3 binary.
- **Found and fixed a silent data-destruction bug**: only `nerve init` migrated, so a v3 binary
  indexing an un-migrated v2 database dropped every insert after deleting rows — 49 entities to
  33, exit code 0. Report: `docs/reports/slice-03b-report.md`.

## Slice 5 scope split (2026-07-31)

Row 5 was one row covering an ingestion path, a resolver with a precision gate, an invalidation
extension and a UI surface. It is split into **5a** (ingestion, structure, ADR status, T7) and
**5b** (link resolution, precision gate, surfacing) on the same seam as 2a/2b and 4a/4b, and for
the same recorded reason: a slice bundling two surfaces stalled an implementation agent, and the
same work split in two succeeded. Rationale, pushback and acceptance criteria:
`docs/plans/slice-05-document-evidence.md`.

Four points of pushback are recorded there and are binding on the implementation: the briefed
relation names (`DOCUMENT_CONTAINS_SECTION`, …) are rejected because Nerve's relation vocabulary is
endpoint-kind-agnostic; `ADR_DESCRIBES_COMPONENT` is refused as non-deterministic; "ADR status" is
a property, not an entity; and `tree-sitter-md` is rejected because it requires tree-sitter 0.26
against this workspace's 0.25.

## Slice 5a — delivered

- `.md`/`.markdown` discovered by the same rules as source; `md-structural 1.0.0` declaring
  `[DOCUMENT_STATED]` and nothing else; `Document` and `Section` entities with spans, content
  hashes and heading nesting; deterministic ADR recognition with a closed status vocabulary.
- **T7 satisfied by an exhaustive query, not a spot check**: no observation whose `file_path` is a
  document carries any source type but `DOCUMENT_STATED`. Mutation-verified — declaring
  `AST_DIRECT` makes the test fail and name all 54 offenders.
- **Two identity-forgery vectors closed.** The plan's `>`-joined `heading_path` was forgeable
  through printable text (found by the implementer); and `rel_path` — a field of *every* identity
  tuple — could carry a literal `0x1f`, which the orchestrator demonstrated merging two sections in
  two different files onto one entity. `canonical_child` now refuses the whole C0 range at the
  single path choke point, closing the class for every constructor, and counts the refusal.
- No dependency added. No schema migration. Report: `docs/reports/slice-05a-report.md`.

## Slice 5b — delivered, and an orchestrator decomposition error

Slice 5b was first dispatched as scanner **plus** resolver **plus** precision harness **plus**
invalidation. **The implementation agent stalled at the 600 s watchdog** — the second time this
project has hit that failure on an oversized slice, after `docs/CONTINUATION.md` had explicitly
warned about it. That is an orchestrator error and is recorded as one.

The partial work was inspected rather than discarded — it built, clippy was clean, the suite was
green, and the scanner was already wired in. The agent was resumed with a narrowed instruction
("test what you built, then stop"), which completed. Resolution became Slice 5c.

Delivered: link destinations recorded exactly as written, with syntax form and a span pointing at
the link; nothing normalized, resolved, stat-ed, opened or fetched. Bare inline-code identifiers are
counted, never emitted. Writing the tests found **six real defects**, including `</div>` and
`<script>alert(1)</script>` being recorded as link destinations — both from accepting a leading `/`
as "root-relative". Ambiguity is now a refusal. Report: `docs/reports/slice-05b-report.md`.

## Slice 5c — delivered

- `md-structural 1.1.0` emits `Section REFERENCES <File>`, and `REFERENCES <symbol>` where a
  `#L<n>` anchor resolves to the innermost covering span. Everything still carries
  `DOCUMENT_STATED`; only `directness` moves to `RESOLVED`. Verified across the whole real
  repository: **0** non-`DOCUMENT_STATED` observations on any `.md` path.
- Broken documentation links are first-class: `document_link_target_not_indexed`,
  `document_link_refused`, `document_anchor_no_symbol`. External destinations are counted, never
  fetched, and never entity-ised — **0** entities named `http…` or `javascript:…` on the real repo.
- **Fixture precision FP=0, recall 100% over 17 sites** — a regression gate, not an accuracy claim.
- **The finding that matters: Nerve's own 45 documents contain 5 Markdown link sites in total.**
  Real documentation refers to code in inline code spans, which this slice deliberately refuses to
  treat as links. High precision, narrow coverage, by design. Report:
  `docs/reports/slice-05c-report.md`.

## Slice 5d — why it exists

Indexing a documentation-only tree revealed 4 observations labelled `AST_DIRECT` in a repository
with **no TypeScript**: directory containment, attributed to `ts-js-structural`. `AST_DIRECT` means
"the syntax tree literally contains this relationship", and there is no syntax tree behind a
directory. This is the same defect class Slice 2a corrected for resolved imports. It is pre-existing
from Slice 1 and was made visible, not caused, by Slice 5a. The fix is a vocabulary addition plus a
UI gloss, so it is its own slice rather than a review amendment.

## Security gate

`docs/THREAT-MODEL.md` (2026-07-31) satisfies the `docs/SECURITY.md` requirement for a written
threat model. It specifies the blocking controls for the local HTTP surface (T4 CSRF/token +
DNS-rebinding, T5 XSS, T6 source serving), documents (T7), test evidence (T9) and MCP (T8).
It also raised one corrective item: there is no test asserting Nerve spawns no subprocess.

## Slice 4a — delivered

- New crate `nerve-server`: blocking loopback HTTP, worker pool, one `PRAGMA query_only`
  connection per worker. Nine read-only endpoints calling the same `nerve-store` functions the
  CLI uses — no business logic in the surface.
- **Tokio + axum rejected on measured evidence.** `tiny_http` costs **3** transitive crates
  against roughly 80–100 for the async stack, for a loopback-only single-user read-only server.
  93 → 100 crates total, all permissive, all recorded.
- **T4/T5/T6 implemented and attack-verified by the orchestrator**, not only by shipped tests:
  no token → 401, wrong token → 403, `Origin: evil.test` → 403, `Host: evil.test` → 403
  (DNS-rebinding), `POST` → 405, traversal and `/etc/passwd` → 403, a `.env` secret absent from
  every response, and an orchestrator-authored XSS payload escaped with **0 raw angle brackets**
  while still round-tripping losslessly.
- Read-only proven by sha256 before/after; clean SIGTERM shutdown leaving no database lock.
- One accepted risk recorded as threat-model **T11**: `tiny_http` reads header lines unbounded,
  a local availability issue with no disclosure path. Report: `docs/reports/slice-04a-report.md`.

## Slice 4b — delivered

- `apps/nerve-web`: React SPA compiled into the binary via `include_bytes!`. `nerve serve` needs
  no Node, no build step and no network. Runtime dependencies are **`react` + `react-dom` only**,
  lint-enforced; 67 build-time packages recorded as **not distributed**.
- Six views: Overview · Symbols · Entity (Relations / **Evidence** / Neighbourhood / Source) ·
  Gaps. The **evidence inspector is the centrepiece** — an assertion written as a sentence, each
  observation carrying extractor id+version and evidence type, and **freshness shown as the
  arithmetic it is**: recorded hash beside on-disk hash, with a verdict.
- The graph is always a **bounded neighbourhood**: on a 485-entity repository it draws 25 of 170
  and says `145 MORE NOT DRAWN`. Not a hairball (plan P4).
- **Fixed a Slice 4a bug that made the interface unloadable**: static assets required the session
  token, which a browser cannot attach to `<script src>`. Relaxed for the fixed asset table only;
  `Host` and `Origin` still enforced (they run *before* the token in `guard.check`), API routes
  and unknown paths still 401. Verified live.
- 31 screenshots reviewed across 4 repositories at 380px and 1600px; 0 CSP violations; database
  byte-identical before and after a UI session. Report: `docs/reports/slice-04b-report.md`.
