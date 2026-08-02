# Slice 7b — `nerve impact`, and the caveat that is larger than the answer

2026-08-02. Plan: `docs/plans/slice-07b-impact.md`. Follows Slice 7a-iii (`5618642`).

---

## Objective

`nerve impact <selector>` — given a symbol, report what depends on it, transitively, with the
evidence for each edge and an honest account of what the answer cannot see.

## User value

*"If I change this, what else breaks?"* is the question asked before every non-trivial edit, and
it is currently answered with grep. Grep over-reports (comments, strings, unrelated same-named
symbols) and under-reports (re-exports, aliased imports) simultaneously, and cannot tell you which
of its hits are real.

## Scope and non-goals

As `docs/plans/slice-07b-impact.md`. Non-goals honoured: no name-coincident "suspect callers", no
`nerve affected`, no reverse `COVERS`, no type inference, no `apps/nerve-web/` change, no schema
change.

## Architecture decisions

**The unresolved account is a field, not a footnote.** Slice 2a measured 38.1% of call sites on
the resolution corpus as honestly `Unresolved` — any method call on a typed receiver is
unresolvable without type inference, which Nerve does not have. A report saying *"3 entities
depend on this"* is read as *"safe to change"*. If a third of the repository's reference sites
resolved to nothing, that reading is unsupported and the command has talked someone into a
breaking change, which is worse than not answering.

`UnresolvedAccount` is therefore a plain field, never an `Option`, rendered and serialized on
every answer **including when every count is zero**. Zero is a measurement worth stating: it means
nothing is hidden from this particular answer by a failed resolution. The precedent is Slice 7a's
`CoverageEvidence::Absent` — one silence made into a value; this is a different silence given the
same treatment.

**The denominator is observations, scoped repository-wide, restricted to the relations walked, and
split by category.** An observation *is* a site — one `parse()` in one place. Two calls to the same
unresolved name from one function collapse into one assertion but remain two observations, and
"38.1% of call sites" is a per-site figure; counting assertions would under-report exactly where a
file leans hardest on one unresolvable name. Repository-wide because a hidden edge can attach
anywhere, and narrowing that set without name matching or type inference is not possible.
Relation-restricted because counting `SUPERSEDES` markers as potential hidden callers, when the
walk never follows `SUPERSEDES`, would be an exact number answering a different question. Split by
`UnresolvedCategory` because a broken Markdown link and an unresolvable method call are both
"resolved to nothing" and are not the same warning.

**Four relations in the default, and four deliberate exclusions.** `CALLS`, `REFERENCES`,
`EXTENDS`, `IMPLEMENTS` — exactly the four Slice 2a resolves and measures.

- `CONTAINS` and `DEFINES` would walk from a function to its module, file, directory and the
  repository. Every symbol would impact everything: true, useless, and it buries the real answer.
- `IMPORTS` is the right closure for incremental invalidation, where a false positive costs a
  reparse. It is wrong for a person: if A imports B and I change a function in B that A never
  calls, A is not affected, and reporting it trains the reader to ignore the output.
- `COVERS` would report a `CoverageRun` — neither a symbol nor code — as something that depends on
  your function. It is a freshness consequence of a change, not a dependency.

**An empty relation list means the default set, not every relation** — the opposite of what empty
means to `PathQuery`. A silent fallback to "all" would follow `CONTAINS`. There is a test for
exactly this.

**No second graph walker.** The traversal reuses `graph::adjacency_sql(query, reverse)` and the
existing `idx_assertion_target(target_entity_id, relation)` index. Unlike `find_paths`, which
enumerates alternative routes and must keep paths simple, impact keeps a **global** visited set
seeded with the subject: a cycle terminates, every entity appears exactly once at its shortest
depth, and a cycle back onto the subject cannot report the subject as its own dependant.

**Bounds.** The closure is expanded in full within `max_depth` *before* `limit` is applied, so
every tally describes the whole answer and not the page, and `truncated` says when the cap cut.

## Files changed

| file | why |
|---|---|
| `crates/nerve-store/src/impact.rs` | **new** — the closure, the account, the vocabulary decisions |
| `crates/nerve-store/tests/impact.rs` | **new** — 18 integration tests |
| `crates/nerve-store/src/graph.rs` | reverse-adjacency internals made reusable; no behaviour change |
| `crates/nerve-store/src/lib.rs` | re-exports |
| `crates/nerve-cli/src/main.rs` | `nerve impact` — human output, JSON, bounds, exit codes |
| `crates/nerve-cli/tests/cli.rs` | 5 tests incl. CLI↔API agreement |
| `crates/nerve-server/src/shapes.rs` | `impact_row` / `impact_report` |
| `crates/nerve-server/src/api.rs` | the `/api/impact` handler and its two ceilings |
| `crates/nerve-server/src/router.rs` | route wired; `ROUTES` 10 → 11 |
| `crates/nerve-server/tests/api.rs` | 7 tests |

## Schema changes / migrations

**None.** A query over existing tables, served by an index that already existed.

## Tests

**771 passed / 0 failed / 2 ignored**, up from 735. +36: 18 store integration, 7 store unit,
7 API, 5 CLI (incl. parity, `#[cfg(unix)]`).

Coverage includes multi-hop closure, a cycle, a re-export chain, the depth bound, truncation with
exact totals, determinism, the subject excluded from its own set, the default relation set,
an empty relation list not opening the walk, the account when non-zero, **the account when zero**,
an unresolved-heavy repository reporting a small answer beside a large caveat, an unresolved
subject, argument bounds, and an ambiguous selector refused with candidates.

## Mutation probes

**Probe 1 — zero the unresolved account** (`account.sites += 0`). **4 tests failed** across three
layers — store, CLI, API — with no compile errors:

```
the_unresolved_account_counts_sites_and_splits_by_category
an_unresolved_heavy_repository_reports_a_small_answer_beside_a_large_caveat
impact_reports_the_closure_and_always_states_what_it_cannot_see        (CLI)
the_impact_answer_always_carries_an_unresolved_account                 (API)
```

**Probe 2 — admit `CONTAINS` to the default relation set.** **10 tests failed.** But the API test
`the_default_impact_walk_does_not_climb_into_containment` did **not**, which was worth
understanding rather than accepting: reverse `CONTAINS` from a *function* subject matches nothing,
because a function is `DEFINES`'d by its module, not `CONTAINS`'d. Re-run with `DEFINES` admitted
instead, that test fails as intended. The test is sound; `CONTAINS` was simply not the relation
that could leak from this subject. Recorded because a probe that "passes" for the wrong reason is
how a test gets trusted that never fired.

Both reverted; `DEFAULT_RELATIONS` is back to 4 members and `account.sites += sites` restored.
Full gate re-run green afterwards.

## Verification

Run by the orchestrator.

```
cargo fmt --all -- --check                             → clean, exit 0
cargo clippy --workspace --all-targets -- -D warnings  → 0 warnings, exit 0
cargo test --workspace                                 → 771 passed, 0 failed, 2 ignored, exit 0
cargo build --release                                  → Finished, exit 0
```

**CLI smoke test**, release binary, `fixtures/ts-basic`, subject `add`:

```
  entities       3 depend on this, transitively
  by depth       1 3
  by relation    CALLS 3
  by kind        function 3
  stale          0 …

1     CALLS       function   describe    src/alias.ts:7    AST_RESOLVED   fresh
1     CALLS       function   legacyAdd   src/legacy.cjs:5  AST_RESOLVED   fresh
1     CALLS       function   subtract    src/math.ts:10    AST_RESOLVED   fresh

  unresolved     4 reference site(s) in this repository resolved to nothing
                 over CALLS, REFERENCES, EXTENDS, IMPLEMENTS
                 4 assertion(s), 4 distinct target(s) · value 4
  Any of them could reach this symbol, and this answer cannot rule them out.
```

Exit 0. **Three dependants beside four unresolved sites** — the caveat is larger than the answer,
which is the honest shape of this repository rather than a defect in the command.

## Security review

No new surface beyond one read-only authenticated route. The relation list interpolated into SQL
is generated from the closed `Relation` vocabulary through the same helper the traversal uses; no
caller text reaches a statement — selectors and ids are bound parameters. Depth and limit are
clamped at the surface (32 / 500) so a response stays bounded regardless of repository size.
Freshness re-reads files through `RepositoryProber`, which enforces the Slice 1 path rules on
every path the database supplies. `read_category` parses an extractor-written `meta` blob and
buckets an unparseable or unknown value rather than panicking — that blob is a file on disk Nerve
does not own.

## Privacy review

No network, no telemetry, no subprocess. Nothing leaves the machine.

## Clean-room review

No competitor source consulted. Independent implementation.

## Dependency review

**None added.** `Cargo.lock` and `third_party/LICENSES.md` untouched.

## Deviations

**One, forced and material.** The implementation subagent was terminated mid-slice by an
org-level monthly spend limit, having completed the store, the CLI and the response shapes but not
the API handler, the route, or any CLI/API test. `CLAUDE.md` §4 requires implementation be
delegated; delegation was unavailable. Rather than lose verified work, the orchestrator inspected
the partial output (it compiled, and 24 store tests passed), then wrote the remaining API handler,
the route, and all 12 CLI and API tests directly. Recorded here because it departs from the
project's working rule, and because the orchestrator reviewing its own implementation is a weaker
check than the usual two-party one — the mutation probes were relied on more heavily as a result.

## Known limitations

- **The closure is only as complete as resolution is.** 38.1% of call sites on the resolution
  corpus are unresolved. This is stated in every answer rather than hidden, but it is a real limit
  on the question's usefulness, not merely a disclosure.
- **`assertion_state` and the representative observation are read per reached entity** — N+1
  queries over a depth-bounded closure. Acceptable at the current bound; it would want batching
  before the limit ceiling rises.
- **The unresolved account is repository-wide**, so two impact queries in the same repository
  report the same caveat. That is honest but not sharp; sharpening it needs type inference.
- **No `nerve impact` view exists.** Contract recorded in `docs/UI-BACKEND-HANDOFF.md` Entry 2.

## UI backend handoff changes

`docs/UI-BACKEND-HANDOFF.md` **Entry 2** — `/api/impact`: parameters and their clamping, full
response schema, the `by_depth`-as-array rationale, all six states, a real abbreviated example, the
required display language for `unresolved` in both the non-zero and zero cases, and the
instruction not to label any of it "affected tests". No frontend file was touched by this slice.

## Commit

*feat: Slice 7b — nerve impact, and the caveat that is larger than the answer*

Hash recorded in `docs/CONTINUATION.md` by the follow-up docs commit.

## Next slice

**7c — `nerve check` (CI exit codes) + `nerve doctor` (diagnostics).**
