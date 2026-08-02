# Slice 7a-iii — the rail counted everything and called it symbols

**Corrective.** 2026-08-02. Follows Slice 7a-ii (`da4ae72`, QA docs `a1ddbdf`).

---

## Objective

Introduce a canonical `symbols_total`, derived from `EntityKind::is_symbol()`, and bind the
navigation rail's "Symbols" badge to it instead of to `entities_total`.

## User value

The interface stated a falsehood about the repository. The rail printed `entities_total` — every
repository, directory, file, module, document, section, unresolved reference, and since Slice 6a
every ingested coverage report — under the label **"Symbols"**. On `fixtures/ts-coverage` the rail
read **18** while the Coverage view, on the same screen, correctly read **8 symbols in scope**.

A tool whose product is epistemic cannot show a user two different numbers for one word and be
trusted about anything harder. The defect is pre-existing from Slice 4b; Slice 7a-ii's Coverage
view made it visible by putting the correct number next to the wrong one.

## Scope

- A canonical `symbols_total` in `nerve-store`, derived from the vocabulary.
- The field on all three JSON surfaces and both human outputs.
- The one-expression rail binding, and the TypeScript mirror it needs.
- Consolidation of the symbol-kind SQL list, which had been triplicated.
- Tests that make the defect unrepresentable.

## Non-goals

- No change to what `EntityKind::is_symbol()` returns. It was read, not redefined.
- No change to `Overview.tsx:165`, which renders `entities_total` beside the word *entities* and
  is therefore already correct. Adding a symbols figure to that view is a product decision and
  belongs to the user.
- No visual, layout, typography or component work of any kind — the interface is frozen and owned
  by the user from 2026-08-02.
- No schema migration. `symbols_total` is a derived count over the existing `entity` table.

## Architecture decisions

**The count is derived, not stored.** A stored count is a second source of truth that can drift
from the rows it summarises. `symbols_total` is `count(*) … WHERE kind IN (…)` at query time, with
the kind list generated from `EntityKind::ALL` filtered by `is_symbol()`. It cannot disagree with
the vocabulary because it is the vocabulary's own answer.

**One helper, not a fourth copy.** The slice's stated purpose was a *canonical* symbol count, and
the codebase already generated the same quoted symbol-kind SQL list in **three** independent
places — `select.rs:34`, `query.rs:421`, `gaps.rs:379`. Each was individually correct and each was
generated from the vocabulary, so nothing was semantically wrong; but adding a fourth would have
contradicted the word *canonical* in the slice's own objective. The three are now one
`pub(crate) fn symbol_kinds_sql()` with four call sites.

The consolidation was deliberately *not* allowed to eat the reasoning. Each site had a comment
explaining why it filters to symbols — a `Module`'s `scope_path` is a file path so folding it into
a dotted name would produce a name that exists nowhere in the source; a coverage edge to a
`Module` would say "the test suite covers this file", a different and weaker claim. Those stayed
at their sites. Only the shared property — built from a closed compile-time vocabulary, never from
caller text, therefore not an injection site — moved onto the helper.

**`IndexOutcome` too.** It is built field-by-field from a `StatusReport`, so the field was one
line. Leaving it out would have meant `nerve index --json` and `nerve status --json` disagreeing
about what the same database contains.

## Files changed

| file | why |
|---|---|
| `crates/nerve-store/src/select.rs` | `symbol_kinds_sql()` → `pub(crate)`, made the single source; existing drift test strengthened |
| `crates/nerve-store/src/query.rs` | `StatusReport::symbols_total` + population; second copy of the list deleted |
| `crates/nerve-store/src/gaps.rs` | third copy of the list deleted |
| `crates/nerve-store/tests/status.rs` | **new** — the invariant and its three companions |
| `crates/nerve-core/src/vocab.rs` | **tests only** (+52, −0) — all twelve kinds pinned |
| `crates/nerve-server/src/api.rs` | `symbols_total` on `/api/overview` |
| `crates/nerve-server/tests/api.rs` | strict inequality + cross-check against `entities_by_kind` |
| `crates/nerve-index/src/pipeline.rs` | `IndexOutcome::symbols_total` |
| `crates/nerve-cli/src/main.rs` | `symbols_total` in both JSON outputs and both text outputs |
| `crates/nerve-cli/tests/cli.rs` | `require_keys` for both commands; new CLI↔API agreement test |
| `apps/nerve-web/src/api/types.ts` | the TS mirror — 3 lines, no visual change |
| `apps/nerve-web/src/App.tsx` | **one expression** — `entities_total` → `symbols_total` |
| `crates/nerve-server/assets/assets/nerve.js` | rebuilt bundle, re-embedded |

## Schema changes / migrations

**None.** Derived count over an existing table.

## Tests

Six added; **735 passed / 0 failed / 2 ignored**, up from 729.

The load-bearing one is `a_non_symbol_entity_never_increases_symbols_total`, and it is written
over **every** non-symbol kind in the vocabulary rather than over a chosen example: insert one,
assert `symbols_total` is unchanged *and* `entities_total` rose by one. Both halves matter —
without the `entities_total` assertion the test would pass against a query returning a constant,
including the constant `0`.

`a_symbol_entity_increases_both_counts_by_one` closes exactly that hole from the other side: a
`symbols_total` frozen at zero satisfies "never increases" and is a different lie told in the same
place.

`every_entity_kind_is_classified_as_a_symbol_or_not` pins all twelve kinds individually and then
asserts the pinned list is exhaustive over `EntityKind::ALL`. `is_symbol()` is a `matches!` over
four variants, so a kind added to the vocabulary is silently classified *not a symbol* — a default,
not a decision. That default now decides what the interface prints beside the word *symbols*, so a
new kind must fail here until someone states which it is.

`the_cli_and_the_api_report_the_same_symbol_count` runs on the **covered** fixture, so a
`coverage_run` — the kind that made the defect visible — is in the table when the two surfaces are
compared.

## Mutation probes

**Two, by different parties, attacking different layers.**

The implementer's probe replaced the population with `SELECT count(*) FROM entity` — the exact
original bug. Five tests failed across three crates.

The orchestrator's probe was deliberately different: add `EntityKind::CoverageRun` to
`is_symbol()`, attacking the vocabulary rather than the query. Under `--no-fail-fast`, **16 tests
failed across 6 targets** — 719 passed, 16 failed. Failures were verified to be intended, not
incidental:

```
vocab::tests::every_entity_kind_is_classified_as_a_symbol_or_not
  assertion `left == right` failed: coverage_run is classified against this list
  left: true   right: false

a_non_symbol_entity_never_increases_symbols_total   left: 7   right: 8
only_symbols_are_ever_reported                       left: 6   right: 5
```

All seven `^error` lines in the log were cargo's "test failed" harness lines; **no compile
errors**. Eleven of the sixteen were pre-existing coverage and gaps tests correctly defending
ADR-0005/ADR-0008 — evidence the older invariants are load-bearing too, not just the new ones.

Reverted. `git diff crates/nerve-core/src/vocab.rs` then showed **+52, −0** — purely additive test
code, `is_symbol()` byte-identical to its original text — and the full gate re-ran green.

Note for future probes: plain `cargo test --workspace` halts at the first failing target and
reported only 3 failures. `--no-fail-fast` was required to see all 16. A probe that stops at the
first target understates its own blast radius.

## Verification

Run by the orchestrator, not merely reported by the implementer.

```
cargo fmt --all -- --check                             → clean, exit 0
cargo clippy --workspace --all-targets -- -D warnings  → 0 warnings, exit 0
cargo test --workspace                                 → 735 passed, 0 failed, 2 ignored, exit 0
cargo build --release                                  → Finished, exit 0
```

Frontend: `npm run check` → typecheck clean, lint 24 files clean, 15 pass / 0 fail.
`npm run build` re-embedded the bundle; `crates/nerve-server/assets/assets/nerve.js` contains
`symbols_total`, so the binary is not serving a stale asset.

**Observed on real fixtures:**

| fixture | `entities_total` | `symbols_total` |
|---|---|---|
| `ts-coverage`, indexed | 17 | **8** (3 function, 3 method, 1 class, 1 interface) |
| `ts-coverage`, after `nerve coverage` | **18** | **8** |
| `ts-basic` | 48 | **24** |

The middle row is the slice in one line. Ingesting a coverage report adds a `coverage_run` entity,
so `entities_total` moves 17 → 18 while `symbols_total` stays at 8. Under the old binding the rail
would have reported one more *symbol* purely because somebody ingested a coverage report, having
indexed no new code at all.

Orchestrator smoke test with the release binary on a scratch copy of the fixture: CLI text,
`status --json` and `index --json` all report the same pair, and `nerve gaps` reports
`symbols_in_scope: 8`. The rail and the Coverage view now agree, which is the whole point.

**One discrepancy, recorded rather than smoothed over.** The Slice 7a-ii QA report
(`docs/reports/slice-07a-ii-report.md:81-85`) states the rail read **21** against **9** real
symbols, itemised `function 4 · method 3 · interface 1 · class 1`. The fixture at this commit
yields 18 against 8, itemised `function 3 · method 3 · interface 1 · class 1` — one fewer function
and three fewer entities. The QA session therefore ran against a tree that is not
`fixtures/ts-coverage` as committed. This does not affect the defect or the fix, both of which are
verified above against the actual fixture, but the 7a-ii numbers should not be quoted as if they
describe the committed fixture. The roadmap row for 7a-iii has had the specific counts removed for
that reason.

## Security review

No new surface. `symbols_total` is a read-only scalar on an already-authenticated endpoint. The
kind list interpolated into the `IN (…)` clause is generated from a closed compile-time vocabulary
and can never contain caller text — the same property the three pre-existing call sites had, now
stated once and tested once. No new SQL takes a caller-supplied string.

## Privacy review

No new data leaves the machine. No network code, no telemetry, no subprocess. The added field is a
count of rows already counted by `entities_by_kind`.

## Clean-room review

No competitor source consulted. Nothing derived from CodeGraph, Graphify or GitNexus.

## Dependency review

**None added.** `Cargo.lock` and `third_party/LICENSES.md` untouched.

## Deviations

One, accepted. Item (f) of the brief asked to *extend* an existing CLI↔API agreement test; none
existed for `overview`, so a new one was written following the Slice 7a `gaps` pattern. It
duplicates roughly 35 lines of spawn/`Reaper` boilerplate, which was left in place because the
brief forbade unnamed refactors. The duplication is real and is recorded below.

## Known limitations / carried forward

- **`IndexOutcome` is built field-by-field from `StatusReport`.** Every future `StatusReport`
  field needs a manual copy in `pipeline.rs` and silently goes missing from `index --json` if
  forgotten — precisely the class of omission this slice corrected. Nothing enforces the
  correspondence. Found by the implementer, deliberately not fixed here.
- **The CLI↔API test boilerplate is duplicated twice** (`gaps`, `overview`). A third such test
  should hoist it.
- **Narrow-viewport QA at 380px is still outstanding**, carried from Slice 7a-ii. Unrelated to this
  slice; it is UI work and now belongs to the user. Recorded in `docs/UI-BACKEND-HANDOFF.md`.

## UI backend handoff changes

`docs/UI-BACKEND-HANDOFF.md` **created** by this slice, as the standing record of post-freeze
frontend integration requirements. Entry 1 documents `symbols_total`: the endpoint, the field, the
example response, the display language ("print `symbols_total` beside the word Symbols; never
`entities_total`"), the four states, and the two four-line frontend edits the backend made and why.

## Commit

*fix: Slice 7a-iii — the rail counted everything and called it symbols*

The hash is recorded in `docs/CONTINUATION.md` by the follow-up docs commit. A commit cannot
contain its own hash, and writing a guessed one here would be a fabrication — the same correction
Slice 7a-ii already had to make (`a58a9a9`).

## Next slice

**7b — `nerve impact`**: reverse dependency closure with evidence and an honest truncation flag.
