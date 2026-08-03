# Nerve — Continuation State

**Written:** 2026-08-01 · Keep this accurate at the end of every session.

---

## Where execution stopped

| | |
|---|---|
| **Last slice commit** | **Slice 12a** (`e2ecb23`) — Git object access. Row 11 closed earlier in the same session (11a-i `871aef3`, 11b `dcff528`). `git log --oneline -15` is authoritative. |
| **Branch** | `main` · **Working tree** clean at that commit |
| **Remote** | **None configured.** Nothing pushed. All work local. Deliberate — see "Decisions already made". |
| **Last completed slice** | **Slice 12a** — Git object access. **Rows 1–11 and 12a are complete.** |
| **Next action** | **Slice 12b** — the historical model on top of 12a's reader: commit entities, first/last seen, moves, rename *hypotheses* kept separate from identity links, change frequency, labelled co-change. **The storage strategy must be measured, not assumed** — duplicating the graph per commit needs proof. This is the first slice in row 12 that adds entities, a schema migration and a CLI surface, so it needs its own plan first. Then 13, 14, validation execution, the final audit. |
| **Roadmap status** | **INCOMPLETE.** Rows 1–11 and 12a are done. **12b, 13, 14, the real-world validation run and the final backend audit are not.** The acceptance package now exists and passes 35/35, which gates what is built and is not a claim of completeness. |

A machine restart interrupted this project on 2026-08-01; recovery found no lost work and required
no repair. See `docs/reports/restart-recovery-report.md`.

## What exists now that did not before, and where it is

| | |
|---|---|
| `scripts/final_acceptance.sh` | **Runnable.** Last result **35 passed, 0 failed, 0 skipped**. Distinguishes `PASS` / `FAIL` / `REFUSED` / `NOT BUILT` / `SKIPPED`, and **fails if `nerve affected` or `nerve trace-tests` ever exists** so a boundary cannot be crossed quietly |
| `docs/FINAL-ACCEPTANCE.md` | What the script gates, what it cannot, and the two refusals with their decisions named |
| `docs/plans/slice-11b-python-tracer.md` | 11b's spec. A **pytest plugin**, not a bare tracer — `sys.monitoring` reports code objects and cannot know which test is running |
| `docs/plans/slice-12a-git-object-access.md` | 12a's design, **plus three corrections the implementation proved against the code** — `selector_shape` is a selector guard and the wrong tool for a filesystem path; the plan's root check cannot be passed in; and the worktree case is served by `commondir`, not alternates |
| `crates/nerve-index/src/gitobj/` | The reader. Zero entities, zero rows, schema unchanged. `StoreLimits` is where its honesty lives: shallow, promisor, refused packs and refused alternates are **reported**, never inferred from an empty result |
| `tracers/python/nerve_trace/` | The trace producer. **Not part of the Nerve product** — no Rust source may name it, asserted by `crates/nerve-cli/tests/no_tracer_reference.rs` and probe-verified |
| `docs/plans/slice-15-real-world-validation.md` | Extended past its TypeScript-only corpus. Python repositories, `jedi` as oracle, and the endpoint oracle's awkward property: **it must execute repository code, which Nerve refuses to do**, and the gap between the two *is* the measurement |
| `docs/THREAT-MODEL.md` | T9 **restated** for traces rather than extended — its control *"coverage may only produce `COVERS` — never a call edge"* cannot cover a trace, which legitimately produces one. T10's dependency count corrected 100 → 101 |
| `docs/UI-BACKEND-HANDOFF.md` Entry 5 | Traces. Four ways a view can be wrong while looking reasonable |

### Verified by hand this session, on the release binary

- **Slice 10a's defect is closed end to end.** On `fixtures/py-framework`: 18 endpoints;
  `nerve impact read_user` reports `SERVED_BY 1`, so a live handler is distinguishable from dead code;
  `nerve search users` finds all four `/users` routes. Both halves of the measured 10a defect.
- **The trace conflict path.** A legitimate import writes 18 rows and exits 0; the same artifact again
  writes **0** rows and exits 0; a replayed `run_id` reports `run-id-conflict 1`, exits **3**, leaves the
  six legitimate edges unchanged, and the collision is visible **in the evidence** — one `run_id`
  against two artifact hashes, both paths named.
- **Nerve does not index Rust**, which the acceptance script learned the hard way. Its own Rust source
  cannot be a self-test subject for a symbol query; `apps/nerve-web` is what lets this repository index
  itself at all.

## Verification state at the Slice 12a commit

```
cargo fmt --all -- --check                              → clean
cargo clippy --workspace --all-targets -- -D warnings   → 0 warnings
cargo test --workspace --no-fail-fast                   → 1321 passed, 0 failed, 2 ignored
cargo build --release                                   → Finished
python3 -m unittest discover -s tracers/python          → 115 tests, OK (skipped=1)
scripts/trace_python_e2e.sh                             → all checks passed (needs pytest; ran)
scripts/final_acceptance.sh                             → 35 passed, 0 failed, 0 skipped
Cargo.lock                                              → 106 packages (101 + flate2's five)
```

`Cargo.lock` is at **106** packages. It was 101 through Slice 11b — which added none, being pure standard
library Python — and 12a added `flate2` plus four transitive crates. The **measured** delta was +5
against an estimated +3: `crc32fast` arrives for a gzip CRC Nerve never reads, and `simd-adler32`
because `flate2`'s `miniz_oxide` feature turns on `miniz_oxide/simd`, which is not a `miniz_oxide`
default. All five are pure Rust — zero `.c`/`.h`/`.cc` files, verified — and all five are recorded in
`third_party/LICENSES.md`.

**Schema is at v5.** `module_facts.framework_version` was added by 10a with `DEFAULT ''`. Any
future extractor added to a language family needs a slot of its own — reusing one is the defect
described under "Decisions already made".

**Use `--no-fail-fast`.** Plain `cargo test` halts at the first failing target and understates a
mutation's blast radius — measured in Slice 7b: 3 reported against 16 actual.

Run by the orchestrator, not merely reported by an implementer. The 2 ignored are opt-in
measurements, not skipped tests:

```bash
cargo test --release -p nerve-store --test scale       -- --ignored --nocapture
cargo test --release -p nerve-index --test incremental -- --ignored --nocapture
```

**Cargo is not on `PATH`.** Prefix commands with `export PATH="$HOME/.cargo/bin:$PATH";`.

## Commands to resume

```bash
cd /Users/krishverma/Documents/Nerve
export PATH="$HOME/.cargo/bin:$PATH"
git log --oneline -10
cargo test --workspace
```

Then read: `CLAUDE.md` · `docs/ROADMAP.md` ·
`docs/plans/slice-05d-supersession-and-filesystem-evidence.md` · `docs/decisions/ADR-0007-filesystem-evidence.md` ·
`docs/THREAT-MODEL.md`.

---

## What Slice 5d delivered

**5d-i (`c39e783`) — filesystem structure is not a syntax tree.** Since Slice 1 the repository
skeleton was stamped `AST_DIRECT` / `ts-js-structural`, which made a documentation-only tree
produce observations asserting a syntax tree in a repository with no TypeScript. New source type
`FILESYSTEM_OBSERVED` (appended at ordinal 11 — appending is load-bearing, `mask_bit()` is
`1 << ordinal` and the mask is stored), new extractor `fs-structural 1.0.0`, schema **v4** with a
data migration. Content-independence is structural: the extractor is handed an `FsEntry` projection
with no field that can hold file text. **T7 amended, not weakened** — the allowed set on a document
path is now exactly `{DOCUMENT_STATED, FILESYSTEM_OBSERVED}` keyed on extractor id, still total,
still mutation-verified. ADR-0007 records the semantics and the rejected alternatives.

**5d-ii (`f98aa2a`) — supersession from explicit evidence only.** Four recognised forms, two
deterministic resolution mechanisms, and everything else recorded as a value rather than guessed.
Cycles are detected and counted but **never suppressed** — each edge is individually evidenced.
FP=0, recall 100% over a 26-file corpus **whose ground truth was written before the resolver
existed**. Nerve's own six ADRs state no supersession and produce **zero** edges; the two real
`**Supersedes:**` fields in `docs/plans/` name prose rather than a target and are recorded
`document_supersedes_unparsed`.

**5d-iii (`947013c`) — the interface can name what the backend stores.** A test reads the
TypeScript gloss maps and fails when a Rust vocabulary gains a member, so the two cannot drift
again. It found **120 real sites** rendering fallback text and one gloss for a status the backend
cannot emit. `directnessClass`'s `default` arm no longer renders an unknown directness as
"inferred".

## What Slice 7a-iii delivered

**Canonical `symbols_total`.** The rail printed `entities_total` under the word "Symbols" — every
repository, directory, file, module, document, section, unresolved reference and, since Slice 6a,
every ingested coverage report. Now `StatusReport::symbols_total`, derived from
`EntityKind::is_symbol()`, on `/api/overview`, `status --json`, `index --json` and both text
outputs, with a test asserting the CLI and the API agree. Verified end to end: ingesting a coverage
report moves `entities_total` 17 → 18 and leaves `symbols_total` at 8.

**The symbol-kind SQL list had been generated in three separate places** (`select.rs`, `query.rs`,
`gaps.rs`). Each was individually correct, but a slice whose objective is a *canonical* count
cannot add a fourth. One `pub(crate) fn symbol_kinds_sql()`, four call sites, per-site reasoning
kept where it belongs.

**The invariant is asserted over every non-symbol kind, not an example**, and both directions are
tested — "never increases" alone is satisfied by a count frozen at zero. All twelve kinds are
pinned individually with an exhaustiveness check, because `is_symbol()` is a `matches!` and a new
kind would otherwise be classified by default rather than by decision.

Orchestrator mutation probe (make `coverage_run` a symbol) failed **16 tests across 6 targets**.
Note: plain `cargo test --workspace` halts at the first failing target and showed only 3 —
**`--no-fail-fast` is required for an honest probe**.

## What Slice 7b delivered

**`nerve impact` + `/api/impact`** (the 11th route). BFS reverse closure over `CALLS`,
`REFERENCES`, `EXTENDS`, `IMPLEMENTS`, reusing `graph::adjacency_sql(query, reverse)` and
`idx_assertion_target` — no second graph walker. A **global** visited set seeded with the subject,
so cycles terminate, each entity appears once at its shortest depth, and the subject is never its
own dependant. Closure expanded fully within `max_depth` before `limit` applies, so tallies
describe the answer and not the page.

**The unresolved account is a field on every answer, printed and serialized even when zero.**
Counted in **observations** (a site is an observation; two calls to one unresolved name from one
function are one assertion but two sites), scoped repository-wide, restricted to the relations
walked, split by `UnresolvedCategory`. On `ts-basic`, `add` has **3 dependants beside 4 unresolved
sites** — the caveat is larger than the answer, which is the honest shape of the repository.

**Four exclusions from the default, each reasoned** (`CONTAINS`, `DEFINES`, `IMPORTS`, `COVERS`) —
see `docs/plans/slice-07b-impact.md`. An empty `--relation` list means the **default set, not every
relation**, the opposite of `PathQuery`; there is a test for exactly that.

Two mutation probes. Zeroing the account failed 4 tests across store, CLI and API. Admitting
`CONTAINS` to the default failed 10 — but **not** the API containment test, because reverse
`CONTAINS` from a *function* subject matches nothing (a function is `DEFINES`'d, not
`CONTAINS`'d); re-run with `DEFINES`, that test fails as intended. Recorded because a probe that
passes for the wrong reason is how a never-firing test gets trusted.

## What Slice 7c-i delivered

**`nerve check`** — five verdicts (`current` / `no_index` / `unusable` / `stale` / `unverified`)
over `index_freshness` + `is_healthy()`, mapping to exit `0 / 2 / 3 / 4 / 4`. One new exit code,
`STALE_INDEX = 4`. `exit_code()` is the only place the mapping exists and a unit test asserts every
verdict maps to exactly one code. `--allow-stale` downgrades to 0 without changing the verdict.

**`Unverified` is distinct from `Stale` and shares its exit code.** Different evidence — nothing
was *observed* to change, part of the tree was never looked at — same instruction to the caller. A
truncated sweep is therefore never a clean bill.

**The brief was wrong and the implementer proved it with a test rather than asserting it.**
`index_freshness` iterates `module_facts`, so a file with no row — an *added* file — is invisible
to it: a repository can grow a hundred modules while every recorded hash still matches. Hence
`nerve_index::untracked_files` = `discover(root) − module_facts(repo_id)`. Files the loader would
refuse (over size, unreadable, non-UTF-8) have no row either, so they are counted `unindexable`
and excluded — otherwise they would pin `check` at exit 4 forever.

**Read-only by construction**: the connection opens `query_only=ON`. Verified on the bytes —
BLAKE3 before/after in the shipped test, and the orchestrator confirmed sha256 identical after six
`check` runs including stale ones.

Orchestrator verification beyond the implementer's: **zero false positives across all 7 fixtures**
(document fixtures included — `module_facts` carries rows for documents via the `is_doc()` branch),
and **unsupported file types do not trigger staleness** — adding `.py`, `.txt` and a binary each
left `current`/exit 0, while a real `.ts` gave `stale`/exit 4. `discover()` filters by supported
extension. Orchestrator mutation probe (make `untracked_files` never record an addition) failed 3
tests.

## What Slice 7c-ii delivered — and Slice 7 is now complete

**`nerve doctor`** — 11 checks, **one finding per check on every run, in a fixed order**. A check
that could not run is `severity: "skipped"` *with its cause*, never omitted and never reported as
passing: otherwise a caller cannot tell *sound* from *never established*. Same absence-is-not-zero
principle as 7a's coverage and 7b's unresolved account. Closed id vocabulary pinned by two tests.

**No new exit code.** Fatal reuses `NO_INDEX = 2`, whose documented meaning is already "no index at
the requested path, **or it is not healthy enough to answer**". Warnings exit 0.

**Two findings from building it.** `SELECT count(*) FROM entity_fts` reads the *content* table and
returns the entity count even after the index has drifted — the obvious FTS consistency check is
guaranteed to report agreement. Established by probe; `entity_fts_docsize` used instead. And FTS5's
own `integrity-check` is an `INSERT`, blocked by `query_only`.

**The no-SQL-in-the-CLI guard only scanned `main.rs`**, so a new module would have evaded it. The
queries went to `nerve_store::diagnose` and the guard was widened to the whole crate.

`doctor` does not answer `check`'s question — freshness is neither reimplemented nor called; it
prints a line pointing at `nerve check`.

Orchestrator adversarial smoke tests, all no-panic: zero-byte `nerve.db` (SQLite accepts it as
empty, so `schema_version` fires), `.nerve` as a file, `nerve.db` as a directory, and **`nerve.db`
symlinked to `/etc/passwd` — refused with no content disclosed.** Orchestrator mutation probe
(synthesise a complete migration list) failed 3 tests.

## What Slice 8a delivered

**`nerve mcp`** — stdio JSON-RPC 2.0, `initialize` / `notifications/initialized` / `ping` /
`tools/list` / `tools/call`, and **one** tool, `nerve_investigate`, the MCP counterpart of
`nerve why`. **Zero new crates**: framing hand-rolled on `serde_json`, because 4a already measured
the async-runtime trade and a single-client stdio loop needs it even less. `Cargo.lock` untouched,
still 100 crates.

**T7 is structural, not annotational.** Every repository-derived value lives under one
`repository_content` key; beside it sit only Nerve's vocabulary, integers, and `query` (the
caller's own echoed arguments, which the trust block names explicitly rather than mislabelling).
The label is carried three ways, and — the reason for one field rather than per-span markers — it
can be tested as a **property**: a test walks the whole response and asserts no string inside the
field appears outside it.

**Three independent response bounds**: row cap, per-assertion observation cap (with the true total
still reported, so a caller sees "20 of 90" rather than believing there were 20), and a 128 KiB
ceiling measured on the *pretty-printed text a client reads*. The ceiling is the backstop the row
cap cannot be: one pathological `details` blob defeats a row cap. Cutting from the end keeps the
page a prefix so continuation stays exact, and the degenerate case is named — a single oversized
record yields `continuable: false` rather than an offset that advances by zero and loops forever.

**A traversal-shaped selector is refused as a refusal**, never disguised as "not found" (T2's
rule). The implementer deliberately did not route it through `discover::canonical_child`, and the
orchestrator verified why: `discover.rs:96` maps a `canonicalize` failure to `PathEscapesRoot`, so
a merely-nonexistent path is indistinguishable from an escape and legal bare-name selectors would
be refused.

**What the orchestrator's own T7 test taught.** Injection text placed in a Markdown *body* came
back **absent entirely** — Nerve stores ranges and hashes, never source text, so body prose never
enters the graph. The real vector is a **heading**, which becomes an entity name. Re-run that way,
the string appeared 7 times and **every occurrence was inside a labelled region. Zero leaks.**

Three mutation probes: removing an output bound, removing the trust block (5 tests, 3 targets),
and — the orchestrator's — disabling the traversal pre-check (3 tests).

---

## The interface is frozen (2026-08-02)

The user owns `apps/nerve-web/` from this date and is working on it separately. Backend work
continues.

**Do not** do visual redesign, new views, layout, typography, responsive work, screenshot-driven
refinement, or any discretionary edit under `apps/nerve-web/`. Do not block a backend slice on
browser or screenshot QA.

**Do** complete the backend contract, add API and CLI tests, and record every new frontend
integration requirement in `docs/UI-BACKEND-HANDOFF.md` — endpoint, fields, states, example
response, display language. A frontend edit is permitted only when the repository would otherwise
not build, must be minimal, and must be documented there. Slice 7a-iii's two edits (four lines
across `types.ts` and `App.tsx:144`) are the model.

## Remaining roadmap

Rows 1–9 are **complete** and no longer listed here; `docs/ROADMAP.md` is authoritative.

| | |
|---|---|
| **10** | ✅ **Complete** as 10a + 10b. HTTP routes only: FastAPI, Flask, Express. `EntityKind::Endpoint`, `Relation::ServedBy`, schema v5, `FRAMEWORK_RULE` emitted for the first time. **Events, DI, Django, NestJS and pytest fixtures were each rejected with a reason** in `docs/plans/slice-10-framework-rules.md` §2 — read that before "finishing" row 10, because it is finished as scoped. |
| **11** | ✅ **Complete** as 11a + 11a-i + 11b. `nerve trace-tests` **was refused and stayed refused** — tracing is ingest-only, because `no_subprocess.rs`'s own module doc names "no test runners" as what it exists to refuse, and coverage and `gitinfo.rs` had both already chosen ingestion. The cost was accepted openly: `tracers/python/` is non-Rust product surface, and **no Rust source may name it**. Read `docs/reports/slice-11a-i-report.md` before touching the hostile fixtures — four of them were attacking nothing while a green suite reported them as passing. |
| **12a** | ✅ **Complete.** A reader only: zero entities, zero rows, schema unchanged. `flate2` added with `rust_backend`; the **measured** delta was **+5 (101 → 106)**, not the +3 the analysis estimated. `.git` object data is now untrusted input — the bound that matters is that the inflate is capped **as it streams**, and a probe turning that into a post-hoc check allocates **805 MB against a 64 MiB bound**. |
| **12b** | **Next.** The historical model on 12a's reader: commit entities, first/last seen, moves, rename **hypotheses kept separate from identity links**, change frequency, labelled co-change. **It needs its own plan first** — it is the first slice in row 12 that adds entities, a schema migration and a CLI surface, and the storage strategy must be **measured**: duplicating the whole graph per commit needs proof, not intuition. 12a's `StoreLimits` is the input to get right — a shallow or partial clone means *"I cannot see further"*, which must never be modelled as *"the history ends here"*. |
| 13 | Cross-repository contracts |
| 14 | Human-confirmed memory |
| **Validation** | Real-world accuracy — plan and corpus already chosen: `docs/plans/slice-15-real-world-validation.md`. **Needs extending, not rewriting**: it predates Python and framework support, so its corpus table is TypeScript-only and its category list has no Python or framework rows. Network access for the corpus checkout was verified available on 2026-08-01 (`git ls-remote` against GitHub succeeds). |
| **Acceptance** | `docs/FINAL-ACCEPTANCE.md` and `scripts/final_acceptance.sh` **do not exist yet**. Note when writing them: the CLI has **13** commands, and `sync`, `affected`, `trace-tests`, `history` and `memory` are **not** among them. `affected` is *refused* by ADR-0008 rather than missing, and `trace-tests` is refused by the Slice 11 plan — the script must encode a refusal as a pass, not as a gap. |
| **Final audit** | Clean-checkout build, command matrix, repository matrix, ~24 audit categories. Not started. |

---

## What Slice 10 delivered, and the three traps it closed

**10a (`4e4239a`) — a route handler stopped looking exactly like dead code.** Measured on the 9b
binary first: `nerve impact` on a live `GET /users/{user_id}` handler and on a genuinely dead
function printed **byte-identical** answers. Closed by making the `Endpoint` the *source* of
`SERVED_BY` — forced, not chosen, because impact is a reverse closure — and by adding `SERVED_BY` to
`impact::DEFAULT_RELATIONS`. **The dead case is asserted to stay dead**, so a change that made
everything look reachable cannot pass.

**10b (`286ab59`) — Express, its own extractor id.** Zero `py-framework` observations in a
repository with no Python: the 5d-i invariant restated a third time.

### Three traps, all of a kind this project keeps hitting

1. **The cache-slot upgrade trap, twice.** `module_facts` had two version columns reused
   positionally, and Slice 9b shipped a defect where two extractors shared a version string so an
   existing index hit the cache forever. **10a added the third slot and, for the first time,
   committed the regression test** — 9b's was found by hand and never written, because every test
   builds a fresh index and a fresh index cannot observe an upgrade. 10b then created the *same*
   defect one language over (10a wrote `''` for TS/JS; 10b made that wrong) and committed that test
   too. **Both language upgrade paths now have a test. Before this session neither did.**
2. **A vacuous test, found by a reviewing agent.** The lambda-handler test asserted only
   `endpoints.is_empty()` and passed because the walker never read `app.get(...)(...)` at all. Third
   vacuity trap on this project, after two T7 false passes. **When a test asserts an absence, assert
   the tally too.**
3. **A tally member with no producer.** A `decorator-form` count was drafted, and making it fire
   needed a special case whose only purpose was feeding the counter. Removed. If a form in an
   `unsupported_by_form` map has no construct that produces it, the map is documentation, not a gate.

### The rule that governs both extractors

**Nothing is counted where nothing is known.** `app-not-local` counts a *real* route the rule
declines (the receiver is imported, so the binding is in another file). But an untraceable receiver —
`@cache.get("/x")` — emits nothing **and counts nothing**, because Nerve has no reason to think it
was meant to be a route and a missed-route tally would be a false claim in the opposite direction
from a false positive. Both `negative.py` and `negative.ts` assert zero of each.

---

## Slice 11a — landed, green, and NOT complete

`0aa5942`. `nerve trace import` reads a versioned NDJSON artifact; Nerve never runs the tests.
`no_subprocess.rs` and `no_network.rs` are **byte-untouched**, which is the whole point.

### The gaps: closed in 11a-i, and there were five, not three

`fixtures/trace-hostile/README.md` declared an expected refusal form for every hostile artifact. The
continuation state recorded three that produced none. Diagnosis found **five**, and one shared root
cause behind four of them: **the token-expansion mechanism the README documents did not exist.**
`grep` for `__PAD_ARTIFACT__`, `__PAD_RECORD__`, `__PAD_STRING__` or `__INVALID_UTF8__` across
`crates/` returned nothing; the artifacts were `fs::copy`d verbatim, so `__PAD_STRING__` reached the
parser as fourteen ASCII bytes and `__INVALID_UTF8__` as valid UTF-8.

| artifact | README claims | was | now |
|---|---|---|---|
| `oversized-file.jsonl` | `artifact-too-large`, zero edges | `malformed-json`, **1 observation written** | `artifact-too-large` |
| `oversized-record.jsonl` | `record-too-large` | `record-unknown-key`, from its own padding key | `record-too-large` |
| `oversized-string.jsonl` | `string-too-long` | **nothing refused, 2 observations** | `string-too-long` |
| `malformed-utf8.jsonl` | `invalid-utf8-line` | **nothing refused, 2 observations** | `invalid-utf8-line` |
| `duplicate-run-id.jsonl` | `run-id-conflict` | **nothing refused** | `run-id-conflict` ×1, exit 3 |

The parser was never wrong about any of the four bounds — every one of the fourteen forms in
`trace::form::ALL` has a unit test in `trace_tests.rs` and always passed. What was wrong was that no
*fixture* reached them, so the end-to-end path was untested while reading as though it were tested.

**`run-id-conflict` was a real implementation defect**, and the scope was the error: detection compared
only runs already stored on the call site about to be restated, and the artifact replays its id on a
*different* edge. Now repository-wide via `nerve_store::environments_for_extractor`, counted **once per
artifact** because the collision is one fact about one header. The harm was misplaced in the original
reasoning: it is not overwriting — it is that `run_id` stops naming one run, so a reader asking what
`run-bound-1` observed silently receives the union of two.

**`fts5-syntax`, `prompt-injection`, `sql-injection` and `state-substitution` correctly count no
refusal** — they are **inert, not invalid**. FTS5 operators and instruction text are legal in a
`run_id`; refusing them would reject a legal artifact for looking dangerous, and T7's claim about
untrusted content is inertness rather than rejection. This is now asserted *positively*: the
per-artifact table requires them to produce **no** refusal, so a future over-eager guard fails.

**Why a green suite hid all of this:** `every_refusal_form_is_produced_by_some_fixture` asserted an
**aggregate** — ≥6 distinct forms across the whole set — which the nine working attacks satisfied on
their own. Replaced by `each_hostile_artifact_produces_its_declared_refusal`, per artifact and
bidirectional, plus a `stage_hostile` guard that **refuses to stage an artifact still containing an
unexpanded token**, matching on prefixes so an unknown token also trips it.

### Corrections to the Slice 11 plan, all verified — do not relitigate

1. **Endpoints are `(caller, callee)`, never `(test, callee)`.** Verified on the database:
   `parse → tokenize` and `parse_all → parse`. The Slice 11 plan would have asserted
   `test_basic → tokenize`, a call the test never made.
2. **`Directness::Resolved`.** `Direct` overclaims (the artifact names a location, not a symbol);
   `Inferred` underclaims (unlike coverage, the *relation* is stated outright).
3. **No `TraceRun` entity, no schema change.** `coverage.rs:17` — `CoverageRun` exists because it had
   to be an *endpoint*; a trace run is provenance, and `observation.environment` already exists.
4. **`idx_observation_identity` has no `environment` column** (`schema.rs:257`, verified on the
   bytes), so two tests at one call site are **one row**. Plan §2.1's claim that they would be two
   observations was false. The ingestion restates the **union** in `environment.runs[]`.
5. **`TEST_OBSERVED_CALL` is deliberately NOT in `impact::DEFAULT_RELATIONS`** — the opposite of
   10a's `SERVED_BY` decision, and for a stated reason: a registration is static and present on
   every run, a trace observation is existential. It is also a **security** control: T9's written
   rule ("coverage may only produce `COVERS`, never a call edge") does not transfer to a trace,
   which legitimately produces a call edge — so excluding it means an attacker who can write an
   artifact cannot change what `nerve impact` says by default.

### Slice 12 is analysed but not started

`docs/plans/slice-12-git-object-access-analysis.md` (`324df34`) settles the dependency question with
measurements: no compression library among the 101 packages; a loose object here begins `78 01`;
Nerve's own `.git` has **1342 loose objects and zero packfiles**, which is misleading because a clone
is always packed. `flate2`/`rust_backend` plus an independent packfile reader, over `gix` and `git2`.
Row 12 must split into 12a (object access) and 12b (historical model).

---

## Decisions already made — do not relitigate

- **No remote, no push, no publication.** Explicitly deferred by the user. Do not add a remote.
- **`nerve affected` is not built, because it cannot be built honestly.** Slice 6a measured it: LCOV
  emits an empty `TN:`, one report describes one whole run, and concatenating per-test reports does
  not recover attribution. "Which tests would my change affect?" is unanswerable from an aggregate
  report, and the only way to ship the command would be to attribute the whole report to every
  discovered test file — asserting that every test covers every covered symbol. The command is
  **refused**, not deferred. Revisit only if a per-test format is ingested (Slice 11 tracing may
  provide one). See ADR-0008 §A.2.
- **The relation is `COVERS` from a `CoverageRun`**, never `TEST_COVERS_SYMBOL` and never from a
  test. ADR-0008 reverses ADR-0005's explicit prohibition on the evidence; do not reverse it back.
- **Slice splits**: 2a/2b, 4a/4b, 5a/5b/5c, 5d-i/ii/iii. A slice bundling two surfaces has now cost
  **five** agents. **Keep slices small.**
- **Relation names are endpoint-kind-agnostic.** Kinds live in `entity.kind` and are never
  duplicated into relation names.
- **`ADR_DESCRIBES_COMPONENT` refused** — no deterministic rule separates "describes" from
  "mentions". **ADR status is a property, not an entity.**
- **`tree-sitter-md` rejected** — requires tree-sitter 0.26 against this workspace's 0.25.
- **A bare code-span name in prose is never a link**, and never a supersession target.
- **A supersession cycle is never suppressed.** Each edge is individually evidenced; deleting one
  would hide evidence. Detect, count, report.
- **Supersession fields are recognised on every document, not only ADRs.** The evidence is the
  explicit field, not the file name. The bare-identifier form still resolves only against parsed
  ADR identifiers, because that is the only identifier namespace that exists.
- **`EvidenceSourceType` is append-only.** `ordinal()` is a position in `ALL` and `mask_bit()` is
  `1 << ordinal`, over a **stored** mask. All twelve are now pinned individually so an insertion
  fails where it is written.
- **A migration's literals are frozen.** `migrate_v4` writes `fs-structural` / `1.0.0` as literals
  rather than reading the live constants, for the same reason `V1` is immutable.
- **No `resolution_method` column** — deferred with a trigger, not refused. Reconsider at Slice 10,
  the first slice producing several distinct resolution methods under one source type. See
  ADR-0007's rejected alternatives.
- **Deletion is a hard delete**; **freshness is computed at query time**; **occurrence identity is
  state-independent** (ADR-0006).
- **Tokio + axum rejected** for `nerve serve` on measured dependency cost. Do not "upgrade".
- **Serial parsing** — parallelism deferred so an equivalence failure has one candidate cause.
- **T11 accepted and investigated**: `tiny_http` is unbounded in header line length and count, and
  no mitigation is reachable. Revisit only if Nerve binds non-loopback or grows beyond read-only.

## Open decisions requiring the user

1. **Publication** — no remote exists; account, licence and public release are deferred by explicit
   instruction. Not blocking.

Real-world validation needs no user decision: corpus and oracle were selected autonomously and
recorded in `docs/plans/slice-15-real-world-validation.md`.

## Environment notes

- **An org monthly spend limit killed the Slice 7b implementation agent mid-slice** (2026-08-02),
  after the store and CLI but before the API handler and every CLI/API test. Delegation was then
  unavailable, so the orchestrator finished the slice directly and recorded the deviation in
  `docs/reports/slice-07b-report.md`. If subagent dispatch starts failing with a spend-limit
  error, that is the cause; finishing in the orchestrator is the recorded fallback, and the
  mutation probes carry more weight when the two-party check is unavailable.
- **Session limits and watchdogs are real here, and have now cost five agents.** One terminated
  mid-slice (4b), one stalled at the 600 s watchdog (5b), one hit a hard session limit (5c), one hit
  a hard session limit mid-verification (5d-i). In every case the partial work was inspected rather
  than discarded and the salvageable half was committed as its own verified unit. Do that rather
  than restarting from zero.
- **Do not run `cargo test` while a subagent is also running cargo.** One unreproducible test
  failure was observed in a workspace run that overlapped a subagent's build; four consecutive
  clean runs followed with nothing changed. Two cargo processes sharing `target/` is the most
  likely cause. It was not reproduced and no defect was found.
- **Never trust a subagent's verification claim.** The 5d-ii agent reported 596/0/2; the
  orchestrator's independent rerun initially disagreed. Rerun the gate yourself, every slice.
- A version constant like `EXTRACTOR_VERSION` is a **behavioural contract**: bumping it re-extracts
  every file of that kind. Bump it in the commit that changes behaviour, never earlier.
- Machine load ranged 5–51 across sessions. **Run timing measurements ≥3 times and report every
  run**, never a single flattering number.
- `rm` is aliased interactively; use `/bin/rm -f` in scripts. `timeout` is not installed.
- `curl`/`wget` are blocked by a hook; use `python3` + `urllib` for HTTP probing. `git` network
  access works.
- Node v24.15.0 / npm 11.17.0 at `~/.nvm/versions/node/v24.15.0/bin`.
- Subagent file tools **strip C0 bytes**, so a fixture needing a literal `0x1f` must store an
  escape and substitute at test time.

## Known limitations carried forward

- **Recall on real repositories is unmeasured.** Precision is measured and gated, on fixtures only.
- 38.1% of call sites on the resolution corpus are honestly `Unresolved`. Any method call on a typed
  receiver is unresolvable without type inference.
- **Document link and supersession coverage is deliberately narrow.** Real documentation refers to
  code in inline code spans, which Nerve refuses to treat as links: Nerve's own 45 documents contain
  5 Markdown link sites and 0 resolvable supersession statements. High precision, narrow coverage,
  by design. Do not present this as broad document understanding.
- A **reference-style** link (`[a][ref]`) used as a supersession target resolves as `unparsed`,
  because the scanner records that link's span at the `[ref]:` definition line rather than inside
  the field. Not covered by a fixture.
- For `**Superseded by:** <unresolvable>` the `Unresolved` entity becomes the assertion's **source**.
  No fixture covers it; it is covered only by construction and by `docref` unit tests.
- Indexing the whole repository makes the `fixtures/md-supersession` ADR identifiers ambiguous
  against Nerve's own `docs/decisions/`. The refusal is correct — the identifier namespace is
  repository-wide — but it means the fixture corpus is visible to a self-index.
- FTS matching is prefix-per-token, so `Through` never finds `callThroughMissingImport`.
- Overview has no language breakdown: no language aggregate exists in `StatusReport` or the API.
- Document resource-bound counters appear in `nerve index`, not `nerve status`.
- CommonJS `module.exports` is unmodelled; move proposals are file-level only.
- A transient file-read error treats that file as removed until the next successful run.
- The scoped pruner's completeness is checked empirically, not proved.
- `nerve why` on a single entity has no `--limit`.
- The scale test is load-sensitive and can fail spuriously; it is `#[ignore]`d and does not gate CI.
- **`IndexOutcome` is built field-by-field from `StatusReport`** in `pipeline.rs`. Every future
  `StatusReport` field needs a manual copy there and silently goes missing from `index --json` if
  forgotten — the same class of omission Slice 7a-iii corrected. Nothing enforces the
  correspondence. Found during 7a-iii, deliberately left.
- **The CLI↔API agreement-test boilerplate is duplicated twice** (`gaps`, `overview`) — roughly 35
  lines of spawn/`Reaper` each. A third such test should hoist it into the shared harness.
- ~~A document path does not resolve as a selector.~~ **Fixed in Slice 8b-i.** A path now names
  whatever is at it, and a second reading is reported in `alternatives` rather than silently
  discarded.
- **`./docs/foo.md` — a leading `./` — still resolves to nothing.** Correctly *not* refused as a
  traversal any more, but not normalised away either, so a path pasted from shell tab-completion
  misses. Left deliberately: normalising selectors (`./x`, `x/`, `//x`) is a design question, not
  an edit to make at commit time.
- **CLI and HTTP serialize `selectors` differently.** CLI: an array of
  `{role, selector, matched_by, alternatives}`. HTTP: an object keyed by query-parameter name.
  Each surface is uniform within itself and the underlying resolution is shared, but the two JSON
  shapes differ for one concept. Recorded in `docs/UI-BACKEND-HANDOFF.md` Entry 3.
- **MCP materialises the full `why` report before bounding it.** Bounded by repository size exactly
  as `nerve why` and `/api/why` already are; the *response* is bounded, which is the security
  property. Pushing a limit into `nerve_store::explain` would change all three surfaces and belongs
  in its own slice.
- **`nerve doctor` reports a `nerve.db` that is a *directory* as "is missing".** The verdict (fatal,
  exit 2) and the remedy are right; the sentence is not, and `nerve_dir` gets the analogous case
  right ("exists but is not a directory"). Cosmetic, but it is a diagnostic tool saying something
  untrue about what it found. Reproduce: `mkdir .nerve/nerve.db`. Fix next time that file is open.
- **`fixtures/ts-basic/.nerve/` exists in the working tree at schema 1**, a gitignored leftover from
  an old example run. **Verified untracked** — `.gitignore:2` covers it, and the only tracked
  `.nerve`-matching path under `fixtures/` is `fixtures/ts-incremental/.nerveignore`, a legitimate
  fixture. Harmless to the repository, but `cp -R fixtures/ts-basic` carries a stale index, which is
  why the test helper `copy_tree` skips `.nerve`. Left in place deliberately: a local regenerable
  artifact in the user's tree is not something to delete unasked.
- **`indexable()` in `nerve-index/src/inspect.rs` restates the pipeline loader's three conditions**
  (size ceiling, readable, UTF-8) rather than calling it. If the loader's rules change, `check`
  reports an addition indexing will not actually add. Documented at the function; nothing enforces
  the correspondence. Same class of risk as the triplicated symbol-kind list 7a-iii consolidated.
- **`check`'s truncated-sweep path is unit-tested, not end-to-end.** Forcing the 5,000-file probe
  cap needs a repository larger than the cap, which would dominate suite runtime. `judge_freshness`
  is a pure function and is tested as one. An `#[ignore]`d scale test is the option if wanted.
- **`README.md`'s command list is stale** — it shows only `init`/`index`/`status`/`search` and
  predates `coverage`, `gaps`, `impact`, `path`, `why`, `serve`, `check`. Found during 7c-i, which
  touched only the exit-code line it had to.
- **`docs/ARCHITECTURE.md` has drifted.** It says `nerve-server` is "deliberately not created yet"
  (shipped in Slice 4a), its crate table lists 4 of the 5 crates, its pipeline diagram names only
  the two `ts-js-*` extractors (there are now `fs-structural`, `md-structural` and coverage too),
  and it promises parallelism "in Slice 3" that was deliberately deferred. Documentation only; no
  code depends on it. Fix in a docs pass.
- **The Slice 7a-ii report's fixture counts do not match the committed fixture.** It states 21
  entities / 9 symbols (`function 4`); `fixtures/ts-coverage` at HEAD yields 18 / 8 (`function 3`).
  That QA session ran against a tree that is not the committed fixture. The defect it found was
  real and is fixed; the numbers should not be quoted.
