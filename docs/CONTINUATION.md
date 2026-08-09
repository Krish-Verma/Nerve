# Nerve — Continuation State

**Written:** 2026-08-01 · Keep this accurate at the end of every session.

---

## Where execution stopped

| | |
|---|---|
| **Last slice commit** | **Slice 12c-ii complete — ROW 12 IS CLOSED.** Four commits: plan correction `2517736`, storage `d20abfd`, matcher `aedf952`, surfaces `8f91cf7`, plus the report and narrow-width fix. Report: `docs/reports/slice-12c-ii-report.md`. `git log --oneline -20` is authoritative. |
| **Branch** | `main` · **Working tree** clean |
| **Remote** | **None configured.** Nothing pushed. All work local. Deliberate — see "Decisions already made". |
| **Last completed slice** | **Slice 12c-ii** — similarity rename hypotheses. **Rows 1–12 are complete.** |
| **Verification at that commit** | **1532** Rust tests (0 failed, 2 ignored) · frontend **34** · acceptance **50/50** · tracers 115 OK · `Cargo.lock` 106 · `SCHEMA_VERSION` **7**. |
| **Narrow-width QA is CLOSED (2026-08-08)** | The gap open since row 7a-ii is closed, and the obvious approach is a **trap**: `--window-size=380` writes a 380px PNG while `window.innerWidth` reports **500**, because Chrome clamps a headless window to a platform minimum — the page lays out at 500 and the screenshot is cropped, the *same* failure as the extension's `resize_window`. Use `scripts/viewport_qa.mjs` (CDP `Emulation.setDeviceMetricsOverride` over Node's built-in `WebSocket`; no dependency, no network). It **asserts** `innerWidth` rather than assuming it, and reports console messages, exceptions, 4xx responses and `scrollWidth > clientWidth` overflow. Usage: start Chrome with `--headless=new --remote-debugging-port=9222 --user-data-dir=<tmp>`, then `node scripts/viewport_qa.mjs <url> <width> [png] [label]`; `QA_HEIGHT` overrides the 900px default. **It found a real defect on first use** (`.chip` nowrap vs a 49-character sentence), so it is not decorative. |
| **Still open for the UI parity phase** | Systematic **keyboard navigation** was not tested, and the **corrupt-history UI** (deliberately damaged object store) was not exercised — server-side behaviour is covered by tests, the UI rendering of it is not. |
| **SCHEMA IS NOW v7 (2026-08-05)** | `SCHEMA_VERSION = 7`. `git_rename_hypothesis` was **rebuilt**: `blob_oid` → `from_blob_oid` + `to_blob_oid`, plus `matcher_id`, `matcher_version`, `match_numerator`, `match_denominator`, and a `CHECK` making exact/similar blending a constraint violation. New table `git_rename_analysis` (per commit, per matcher) carries the threshold, candidate counts, `completeness` and `unmeasured` reasons. New column `git_commit.summary_truncation` (`complete`/`truncated`/`unknown`, defaulting to `unknown` because a v6 row cannot be backfilled). `V1`–`V6` are byte-identical, verified by extracting each raw string and comparing. **Rows 13 and 14 are therefore v8 and v9**, already renumbered in their own plans. |
| **Next action (2026-08-08, current)** | **Row 13a — the local registry and schema v8.** **Read `docs/plans/slice-13-cross-repository-contracts.md`'s "Corrected 2026-08-08" blocks first; they overrule the surrounding text.** The correction that matters: **C2 cannot be a local assertion.** `assertion.target_entity_id` is `NOT NULL REFERENCES entity(entity_id)` (`schema.rs:97`, immutable since v1) with `foreign_keys=ON` (`db.rs:37`), and C2's target is a file in repository B with no row in A's `entity` table — so an ordinary local `REFERENCES` assertion **cannot be inserted**, and the only ways to force it (a proxy entity, or dropping the FK) are refused. Every cross-repository link lives in `contract_link`. The new `EvidenceSourceType` is **withdrawn** with it, because `observation.assertion_id` is `NOT NULL` so no observation could carry it; C2 uses a contract-specific closed vocabulary in `contract_link.resolution_method` instead. Also corrected: `contract_link` now carries **target snapshots** and a `withdrawn_at`/`status` lifecycle (without them a renamed target makes `contract_deleted`, `target_changed` and `contract_file_missing` indistinguishable), expected vs observed contract version, the freshness set is **twelve not thirteen** with `generated_client_stale` removed as unreachable (§2.1 of the same plan refuses the evidence it rests on), and `nerve repo remove` **tombstones** rather than deletes so `registry_entry_removed` stays reportable. Row 14's plan was corrected the same day (`d9120c8`) — most importantly `memory.subject_entity_id` must **not** be an FK into `entity`, because `prune_orphans` deletes entity rows routinely (`prune.rs:376`, `:440`) and the FK would either block re-indexing or cascade-destroy a human's note. |
| **Superseded next action (2026-08-05)** | **Slice 12c-ii Pass B then Pass C.** Pass B: the `nerve-line-multiset` v1 matcher in `crates/nerve-index/src/similarity.rs`, the `fixtures/history-similar` corpus with a **hand-written** `ground_truth.json`, and the precision measurement at threshold **7/8** with a **false-positives = 0** gate. Pass C: the surfaces — CLI, JSON, HTTP, MCP, UI — plus `summary_truncation` on every surface (§6.7's *"no surface renders a summary without its flag"* is **currently unmet**), the acceptance-script rows, and the mutation probes in plan §6.8. **Pass C must move `RENAME_ANALYSIS_COMPLETENESS`, `SUMMARY_TRUNCATION` and `SIMILARITY_UNMEASURED` from `DECLARED_NOT_RENDERED` to `GLOSS_TABLES`** in `crates/nerve-server/tests/ui_vocabulary.rs` — a test enforces the move rather than trusting memory. |
| **Superseded next action (2026-08-05, after QA)** | **Slice 12c-ii — similarity renames**, which closes row 12: a **second** `RenameEvidence` value with its own precision table, never blended with `exact_content` and never a score (plan §6). Then rows 13 and 14, whose plans are committed (`307b029`, `eb68830`). **Browser QA of the history views is DONE for desktop** (`docs/plans/ui-parity-matrix.md` §4b) — the wording invariant was confirmed live, zero console messages, refresh preserves deep links. **Narrow width remains unverified** and it is *not* a history-specific gap: `resize_window` succeeds while the viewport stays at 1456px, which is the same limitation row 7a-ii recorded in 2026-08-02. Cover the history views and the 7a-ii views together when a working resize or a fixed-viewport headless browser is available. |
| **Superseded (2026-08-05, final)** | **Two things, in this order. (1) Browser QA of the history views — the one gap in 12c-iv.** A session limit killed that agent before it, and the slice was committed (`397ea3e`) with the gap recorded rather than a claim invented. Start `nerve serve` on `fixtures/history-shallow`, open the UI at ~1600px and ~380px, and check: normal / shallow / error / empty states, deep links, refresh, no horizontal overflow, no console errors, no CSP violations. `CLAUDE.md` §11 forbids claiming visual verification that did not happen, and row 7a-ii records a screenshot-QA failure that came from assuming. **(2) Slice 12c-ii — similarity renames**, which closes row 12: a **second** `RenameEvidence` value with its own precision table, never blended with `exact_content` and never a score (plan §6). Then rows 13 and 14, whose plans are committed (`307b029`, `eb68830`) and settle several questions — read them before planning. |
| **Superseded next action (2026-08-05, later)** | **Slice 12c-iv — the reference UI view and the eight glosses.** 12c-i, 12c-iii-a and 12c-iii-b are complete: the history questions are answerable from the **CLI, HTTP and MCP**. The UI is the last surface, and it is the last place `shallow_boundary` could still be re-worded. Read `docs/plans/ui-parity-matrix.md` first — §3 lists the **eight** unmirrored vocabularies and §4a records that the frontend builds offline (`node_modules` present, `npm run check` green, Node at `~/.nvm/versions/node/v24.15.0/bin`). **The glosses and the `ui_vocabulary.rs` guard extension must land in one commit**, or the next vocabulary is unguarded again — 5d-iii is the precedent, and it found 120 sites rendering fallback text. `docs/UI-BACKEND-HANDOFF.md` **Entry 7** is the contract: seven routes, the availability block, the six-value `kind` table with what each must never be rendered as, `null` is not `[]` on a diff, and the `%23` requirement. Then **12c-ii** (similarity renames) closes row 12. |
| **Superseded next action (2026-08-05)** | **Slice 12c-ii — similarity rename hypotheses.** 12c-i-a and **12c-i-b are complete**; the whole derived-history CLI exists. 12c-ii adds a **second** `RenameEvidence` value with its own precision table, never blended with `exact_content` and never a score (plan §6). Then 12c-iii (API + MCP) and 12c-iv (UI + **eight** glosses — see `docs/plans/ui-parity-matrix.md` §3). **Fold two hoists into 12c-iii before three more surfaces copy the prose**: `FirstObservedKind` and `HistoryFreshness` have no `note()`, and neither does `nerve_store::EarlierHistoryUnavailable`, so 12c-i-b had to write their prose in the CLI — recorded at `main.rs:2520-2529`. |
| **Superseded next action (2026-08-04, later session)** | **Slice 12c-i-b — the CLI family.** 12c now has a committed plan (`352c824`, corrected `2b558af` and again by the implementation, see the 12c section below) that splits it into **four** sub-slices, and **12c-i-a is done**. Next is 12c-i-b: extend `nerve history file` with the first/last block, add `history diff` / `frequency` / `cochange` / `availability`, and make probes 11 and 12 testable (a symbol selector refused *as a refusal*; `canonical_child` never on a historical path). Then 12c-ii similarity renames, 12c-iii API + MCP, 12c-iv UI + six glosses. Rows 13 and 14 have committed plans too (`307b029`, `eb68830`) — read them before planning, they settle several questions. |
| **Superseded next action, kept for the record** | **Slice 12c** — the derived historical questions and every remaining surface. Needs its own plan. Scope is already fixed by 12b's plan §1.1: first/last observed **for path-bearing kinds only** (`File`/`Directory`/`Module`/`Document` — a symbol has `PathRole::None` and `git_change` is path-keyed, so the symbol form is refused), similarity renames as a **second** `RenameEvidence` value never blended with `exact_content`, change frequency, labelled co-change, state-to-state diff, `/api/history*`, MCP tools, the UI view, and **glosses for all six new vocabularies**. Two things to do first, both cheap: extend `scripts/final_acceptance.sh` (its `history` block now prints `PASS … update this script`), and note that `crates/nerve-server/tests/layering.rs` now scans `src/` dynamically so a new `mcp/history.rs` cannot evade the no-SQL, no-graph-walker, loopback-only and no-CORS scans. Then 13, 14, the UI completion pass, real-world validation, acceptance expansion, the clean-checkout audit. |
| **Roadmap status** | **INCOMPLETE.** **Rows 1–12 are complete.** **Rows 13 and 14, the functional UI parity pass, the real-world validation run, the acceptance expansion and the final clean-checkout audit are not started** — though the Row 13 and Row 14 *plans* were both corrected on 2026-08-08 before implementation (`2b91a25`, `d9120c8`) and their correction blocks overrule the surrounding text. The acceptance script gates what is built and is **not** a claim of completeness. |
| **A committed plan was refuted, 2026-08-05** | The 12c plan claimed at §3 that similarity renames needed **no schema change**, citing the `similar_content` comment already on `git_rename_hypothesis.evidence`. That is false by inspection: the column beside it, `blob_oid TEXT NOT NULL`, is **one** blob column, and it exists precisely because an exact-content hypothesis means both paths name the *same* blob. A similarity pair names two different objects, so no value that column can take is honest — and `evidence` + `ambiguity` cannot record the matcher, its version, the measurement, the threshold or candidate-set completeness. §6 was rewritten from a policy into a specification and committed as `2517736` **before** any code was written. Do not relitigate; the reasoning is in the commit message and in plan §3's quoted correction block. |

A machine restart interrupted this project on 2026-08-01; recovery found no lost work and required
no repair. See `docs/reports/restart-recovery-report.md`.

## Open defects found 2026-08-05/08 — recorded, deliberately not fixed in 12c-ii

1. **`scripts/make_history_fixtures.sh` is no longer the source of truth for
   `fixtures/history-hostile/README.md`.** The committed file carries a hand-written
   *"Corrected 2026-08-03"* section about `canonical_child` vs `safe_tree_name` (landed in
   `848af72`); the generator's heredoc does not contain it. **Re-running the generator silently
   reverts a documented correction**, which refutes the script's own byte-identical determinism
   claim for that file. Verified by the orchestrator. Fix: fold the correction into the heredoc.
2. **Seven fixture READMEs say "alongside its six siblings"** while eight fixtures now exist.
   `readme_head` would rewrite seven committed files, so 12c-ii added `readme_head_similar` and left
   the drift. Docs-pass item.
3. **12b write-path defect, still open**: `delete_commits_with_unavailable_parents` deletes
   unconditionally each sync assuming the re-walk re-reaches those commits, which fails once HEAD
   moves. The asymmetry is the defect — a commit with *available* parents that becomes unreachable
   stays recorded.
4. **`MAX_SIMILARITY_LINES` has no `SimilarityUnmeasured` value.** Recording it as `blob-too-large`
   would attach a reason whose stored text is false for it, so the line cap refuses the **commit**
   (`RefusedBound`) instead. A sixth vocabulary value would be more precise.
5. **Similarity blob size is measured after inflation**, because `ObjectStore::read` has no size
   preview. Bounded by 12a's 64 MiB first, then by the matcher's 1 MiB — not as tight as plan §6.4's
   wording implies.
6. Coverage stated honestly: `blob-unreadable` is untested (needs a delta with a missing base as a
   rename candidate); `blob-absent` is unit-tested against an empty store rather than a fixture;
   `RenameAmbiguity::ManyBoth` has no fixture case.

## Environment facts worth not rediscovering

- **`cargo` is not on `PATH`** in this shell, interactive or not. Prefix everything with
  `export PATH="$HOME/.cargo/bin:$PATH"`. Node/npm *are* on `PATH` via nvm.
- **A cold `cargo test --workspace` takes ~10 minutes**, and it is macOS **linking ~48 test
  binaries**, not test runtime — the slowest suite is `gitobj_bomb` at ~15s. This exceeds the
  10-minute foreground command cap, so run the full gate backgrounded or it will look like a hang.
- **Narrow-width browser QA is solved, and the obvious approach is a trap.** Chrome clamps a
  headless window to a platform minimum (~500 CSS px), so `--window-size=380,700` renders the page
  at **500px** and merely crops the screenshot to 380 — `window.innerWidth` reports 500. That is the
  same failure as the extension's `resize_window` that row 7a-ii recorded, with a new tool. The
  working mechanism is CDP `Emulation.setDeviceMetricsOverride` with `mobile: false`, driven over
  Node 22+'s **built-in** `WebSocket` against `--remote-debugging-port`: no new dependency, no
  network, no install. It reports `innerWidth: 380, viewportHonoured: true` and *asserts* that
  rather than assuming it, and it also collects console messages, exceptions and
  `scrollWidth > clientWidth` overflow. Promote the driver to `scripts/` when the UI parity pass
  uses it.
- **Delegation hit a session limit** mid-12c-ii Pass B, after the agent had delivered its report.
  Recorded as a process deviation; the orchestrator verified and continued directly. Delegation
  recovered afterwards.

## Slice 12c-i-a — the derived queries, the wording hoist, and a defect in the plan itself

**Delegation is available again.** A cheap read-only dispatch succeeded on 2026-08-04, and a full
implementation subagent completed 12c-i-a. The three spend-limit kills recorded under "Environment
notes" were **not** in force. It cost **109 minutes** for one sub-slice, which is the number to plan
around: at that rate the remaining rows are many sessions of work, not one.

**What landed:** `FirstObservedKind` (six values) and `HistoryFreshness` (four) in `nerve-core`; the
four wording functions hoisted out of the CLI binary as inherent methods, `earlier_changes_may_exist`
hoisted to `nerve-store` beside `IngestRow`; five derived queries (`first_last_observed`, `state_diff`,
`change_frequency`, `cochange`, `history_freshness`); and `crates/nerve-cli/tests/history_wording.rs`,
a byte-level recursive source scan proving the notes exist in exactly one crate — with sentinels
**generated from the vocabulary** rather than retyped, and a planted file containing invalid UTF-8 and a
NUL to prove the scan cannot be blinded the way `grep` was in `82a6ff3`.

**Schema unchanged at v6.** No table, column, index or migration. `Cargo.lock` unchanged at 106.

### The defect the orchestrator found, and Git settled it

The plan's rule for "may say created" was `Added` **and** `ParentCompleteness::may_claim_history_begins_here()`,
which is true only for a root commit. So a file added in commit #5 of a **complete** clone returned
`EarliestVisibleChange` — a kind meaning *history above may be hidden* — while the two fields beside it
reported `earlier_history_unavailable: None` and `earlier_changes_may_exist: false`. Three statements,
two of them contradicting the third, and every surface would have rendered the kind.

`fixtures/history-basic/inventory.json` carries **Git's own answers**: seven paths, each with exactly
**one** addition, `src/app/extra.rs` added at `522afd34` whose parent `68cde314` is present, not shallow.
Nerve was disagreeing with `git` about a fact `git` is authoritative on — and per the validation plan's
new §3.4 that has no "both reasonable" reading, unlike a `tsc` disagreement.

**Three corrections, each preserving something the implementer was right about:**

1. **`additions_recorded == 1` replaces the parentless requirement.** The implementer's reason for
   conservatism was real and nearly lost: `first` is ordered by `committer_time`, which a rebase or a
   fabricated clock reorders freely, so *"the earliest dated change is an addition"* does not establish
   it is the topologically first. Exactly one recorded addition does, **without consulting a clock** —
   a path created, deleted and re-created records two. A path with two additions is
   `EarliestVisibleChange` **although nothing is hidden**: the refusal there is about *ordering*, not
   availability, and that is asserted.
2. **`ParentCompleteness::Root` returns "nothing hidden" immediately.** The orchestrator's own first fix
   short-circuited on `ingest.shallow` for every path, and **an existing control assertion caught it**:
   a shallow clone can contain a genuine root, one branch fetched whole and another truncated. That
   exposed a scope error in the orchestrator's own "coherence invariant" — `earlier_history_unavailable`
   is **path-level**, `earlier_changes_may_exist` is **repository-level**, and they may only be required
   to agree when the anchor has *available parents*. The narrow equivalence is asserted over every
   `WalkTermination`; the independence is asserted for the root anchor.
3. **`EarlierHistoryUnavailable::WalkRefused` added (five values, was four).** `WalkTermination::Refused`
   mapped to *no reason* beside a `true` boolean — two derivations of one question disagreeing, which is
   the duplication this slice exists to remove, occurring inside the slice.

**One residual, carried as data not prose:** a merge enumerates no changes (12b §6.2), so a path created
inside one merge and deleted inside another has both events unrecorded and a later addition can look
like a first one. The response carries `merges_in_repository` so a consumer can see whether the
possibility exists at all. Zero removes it entirely.

### Four findings the implementer proved against the plan — do not relitigate

1. **§4.1's four reasons do not always apply.** A path added in a non-root commit of an exhausted,
   non-shallow walk has no reason among them. `Option<EarlierHistoryUnavailable>`, with `None` meaning
   *nothing is hidden* rather than *no reason could be found* — which is only coherent because of
   correction 1 above.
2. **`state_diff` needed a fourth refusal, `WalkBudgetExhausted`.** A budget-stopped walk has not
   established that `from` is not an ancestor, so `not_an_ancestor` there would state an unmeasured
   property of the repository. It also covers the subtler case: a truncated ancestors-of-`from` prune
   set would silently **widen** the range, which is a wrong answer rather than a short one.
3. **The plan's claim that the ordering tiebreak is load-bearing is FALSE on this schema, and it was
   measured.** `EXPLAIN QUERY PLAN` shows `idx_git_change_path` with **no** temp b-tree for `GROUP BY`,
   so groups already arrive in `path` order; deleting `path ASC` left **every** ordering assertion
   passing. Both clauses are kept (a query plan is not part of the contract) and the guard is a
   **source-level** test, labelled as one, because a behavioural test for this cannot exist while the
   index does.
4. **Acceptance criterion 3 was unsatisfiable**, the same shape as 12b's criterion 5a.
   `fixtures/history-shallow/inventory.json` records `boundary_tree_path_counts = 2` and its single
   visible commit `68f9ab1f` **modifies both**, so no path in that fixture has zero change rows and it
   cannot produce `PresentBeforeVisibleHistory`. Produced from `history-basic` under a commit budget
   plus a materialised working tree and a real index run instead.

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

## Verification state at the Slice 12c-i-a completion commit

Run by the orchestrator, not quoted from the implementer — and the implementer's own figure of 1428 was
**independently reproduced** before the orchestrator's corrections were applied, which is the first time
on this project a subagent's count has matched on the first rerun.

```
cargo fmt --all -- --check                              → clean
cargo clippy --workspace --all-targets -- -D warnings   → 0 warnings
cargo test --workspace --no-fail-fast                   → 1429 passed, 0 failed, 2 ignored (47 targets)
Cargo.lock                                              → 106 packages, unchanged
SCHEMA_VERSION                                          → 6, schema.rs byte-unchanged
no_network.rs / no_subprocess.rs / no_tracer_reference.rs → byte-unchanged
```

Test-count history: 1402 (12b) → 1428 (12c-i-a implementer) → **1429** (orchestrator corrections).

**Three orchestrator mutation probes on the corrections, each failing a named test for the intended
reason** — the file was saved with `cp` and restored, and the diffstat confirmed byte-identical after:

| probe | fails |
|---|---|
| `ParentCompleteness::Root` falls through instead of returning `None` | `a_path_level_reason_and_a_repository_level_boolean_agree_only_where_they_should` (`left: Some(CommitBudget)`, `right: None`) **and** `an_addition_at_a_shallow_boundary_is_not_a_creation` (`left: EarliestVisibleChange`, `right: CreatedInVisibleHistory`) |
| creation no longer requires `additions_recorded == 1` | `every_first_observed_kind_is_produced_by_real_rows` (`left: CreatedInVisibleHistory`, `right: EarliestVisibleChange`) |
| `WalkTermination::Refused` names no reason | `a_path_level_reason_and_a_repository_level_boolean_agree_only_where_they_should`, message *"Refused: below a visible parent, a named reason and the boolean must agree"* |

`scripts/trace_python_e2e.sh` **was not run and cannot be**, verified rather than assumed this session:
there is no venv anywhere in the repository and the system `python3` has no `pytest`, so the script's own
`pip install` step needs the network. Last green at `2d68d58`. Do not claim it passes.

## Verification state at the Slice 12b completion commit

Run by the orchestrator, not quoted from an implementer.

```
cargo fmt --all -- --check                              → clean
cargo clippy --workspace --all-targets -- -D warnings   → 0 warnings
cargo test --workspace --no-fail-fast                   → 1402 passed, 0 failed, 2 ignored
cargo build --release                                   → Finished
python3 -m unittest discover -s tracers/python          → 115 tests, OK (skipped=1)
scripts/final_acceptance.sh                             → 43 passed, 0 failed, 0 skipped
Cargo.lock                                              → 106 packages, unchanged by row 12b
```

`scripts/trace_python_e2e.sh` was **not** re-run this session — it needs pytest in a venv, which needs
network. Last green at `2d68d58`, and row 12b touched nothing it exercises.

The acceptance script is at **43** checks. It moved 35 → 36 on its own when `nerve history` appeared,
because its "unbuilt" loop awards a `PASS` for a command's mere existence — a pass that checked
nothing. That row is now eight real checks (`2dc3a7d`), including the product assertion that a shallow
boundary is never described as the start of history, verified on the shipped binary and probed by
setting the forbidden pattern to a phrase the output does contain. Its stale "the frontend is frozen"
line is corrected too.

**`nerve memory` is still in that same loop**, so row 14 will award itself the same empty pass. Replace
it with real checks when it lands, rather than after.

**Schema is at v6.** Four new tables — `git_commit`, `git_change`, `git_rename_hypothesis`,
`git_history_ingest` — and no change to any existing table, so `entity_fts`, `symbols_total`,
`entities_total`, selector resolution and `ui_vocabulary.rs` are untouched. Six new closed
vocabularies in `nerve-core`; **none of them is mirrored into the UI yet**, which is 12c's job.

Test-count history: 1321 (12a) → 1349 (12b storage) → 1391 (12b ingestion).

Still to re-run at the end of row 12b, because they were last run at `2d68d58`:

```
python3 -m unittest discover -s tracers/python          → was 115 tests, OK
scripts/trace_python_e2e.sh                             → was all checks passed (needs pytest)
scripts/final_acceptance.sh                             → was 35 passed; will change once
                                                          `nerve history` exists
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

## The interface freeze is lifted for *function* and kept for *visuals* (2026-08-03)

**This supersedes the 2026-08-02 freeze below it.** The user's instruction of 2026-08-03 requires
the existing frontend to become a **functionally complete reference UI** for the finished product,
so that the whole product can be tested before the frontend is redesigned. Direct user instruction
outranks a repository decision, and the earlier entry is kept only so the change is visible rather
than silent.

**Do** implement every finalized human-facing capability in `apps/nerve-web/`: routes or panels,
working API integration, stable types, and every required state — loading, empty, error, partial,
stale, unsupported, ambiguous, truncated. Perform real browser QA on new or materially changed
surfaces. Keep `docs/UI-BACKEND-HANDOFF.md` current for contracts, and maintain
`docs/UI-FEATURE-SPEC.md` as the authoritative handoff for the later redesign.

**Still do not** do visual redesign, rebranding, typography or layout work, or speculative polish.
Use the existing information architecture and design vocabulary. A backend feature is not complete
until the UI provides a usable way to inspect or operate it — and an operation that is
intentionally CLI-only for security reasons must **show its imported results, explain the
boundary, and print the exact command**, never present a disabled button as though implementation
were pending.

Known gaps against that bar, verified 2026-08-03 and not yet fixed:

- ~~`views/Path.tsx` is dead code.~~ **This was wrong, and the way it was wrong matters more than
  the claim.** `views/Graph.tsx:31` imports `PathFinder` from `./Path`, and `/api/path` is reachable
  at `#/entity/<id>/graph?to=<selector>`. The finding came from `grep -rn PathFinder`, which printed
  nothing — because **`Graph.tsx` contained a literal NUL byte**, so `grep` classified it as binary
  and suppressed every match. `file(1)` called it "data". Fixed: the NUL separators in `Graph.tsx`
  and `search.ts` are now `\u0000` escapes, and
  `ui_vocabulary.rs::no_interface_source_file_contains_a_raw_control_byte` fails if any interface
  source regains a raw control byte. **Never audit this repository with plain `grep` alone** — use
  `grep -a`, or a byte-level scan.
  The real `path` defect is narrower: a path is a question about *two* entities and can only be
  asked from inside one of them, so it has no top-level route and is undiscoverable.
- `/api/impact` (Slice 7b, "the 11th route") has **no UI at all** — the string `impact` appears
  nowhere under `apps/nerve-web/src/`, not even as a TypeScript type.
- **Nothing renders `selectors` / `alternatives`.** The API sends it on every selector answer and
  four TypeScript mirrors omit the field, so "resolved by this rule, and here is what it passed
  over" never reaches a human. That is Slice 8b-i's whole point, unsurfaced.
- `/api/partial-parses` — surfacing unverified.

### The superseded entry, kept for the record (2026-08-02)

The user owned `apps/nerve-web/` from that date and was working on it separately; backend work
continued, and discretionary frontend edits were forbidden. Slice 7a-iii's two four-line edits
were the model for the only permitted kind. That constraint held for rows 8b through 12a and
explains why those slices recorded UI requirements in `docs/UI-BACKEND-HANDOFF.md` instead of
implementing them.

## Remaining roadmap

Rows 1–9 are **complete** and no longer listed here; `docs/ROADMAP.md` is authoritative.

| | |
|---|---|
| **10** | ✅ **Complete** as 10a + 10b. HTTP routes only: FastAPI, Flask, Express. `EntityKind::Endpoint`, `Relation::ServedBy`, schema v5, `FRAMEWORK_RULE` emitted for the first time. **Events, DI, Django, NestJS and pytest fixtures were each rejected with a reason** in `docs/plans/slice-10-framework-rules.md` §2 — read that before "finishing" row 10, because it is finished as scoped. |
| **11** | ✅ **Complete** as 11a + 11a-i + 11b. `nerve trace-tests` **was refused and stayed refused** — tracing is ingest-only, because `no_subprocess.rs`'s own module doc names "no test runners" as what it exists to refuse, and coverage and `gitinfo.rs` had both already chosen ingestion. The cost was accepted openly: `tracers/python/` is non-Rust product surface, and **no Rust source may name it**. Read `docs/reports/slice-11a-i-report.md` before touching the hostile fixtures — four of them were attacking nothing while a green suite reported them as passing. |
| **12a** | ✅ **Complete.** A reader only: zero entities, zero rows, schema unchanged. `flate2` added with `rust_backend`; the **measured** delta was **+5 (101 → 106)**, not the +3 the analysis estimated. `.git` object data is now untrusted input — the bound that matters is that the inflate is capped **as it streams**, and a probe turning that into a post-hoc check allocates **805 MB against a 64 MiB bound**. |
| **12b** | **Next — plan accepted and committed** (`3d7ffe9`), `docs/plans/slice-12b-historical-model.md`. Ingestion, storage, availability, `nerve history sync`/`log`/`file`, schema **v6**. Settled and **not to be reopened**: storage is delta, measured at 30.1× and 177× row amplification against per-commit snapshots; **commits are not entities and changes are not assertions** (a commit is provenance for a change fact whose subject is a path, and a historical path is not a current entity — the CoverageRun/TraceRun rule); symbol-level history and historical impact are **refused in row 12** with their costs stated. The plan carries ten verified refutations from adversarial review in its §12, including two pre-existing defects it must fix (`canonical_child` cannot guard a path that no longer exists; `gitinfo::head_commit` does not follow `commondir`). |
| **12c** | Derived historical questions (first/last observed **for path-bearing kinds only**, similarity renames as a second evidence value, change frequency, labelled co-change, state-to-state diff) plus API, MCP and the UI view. Must add UI glosses for the six vocabularies 12b introduces. |
| 13 | Cross-repository contracts |
| 14 | Human-confirmed memory |
| **UI completion** | The frontend freeze is **lifted for function** — see the section above. Three verified pre-existing gaps: `Path.tsx` is dead code, `/api/impact` has no UI, `/api/partial-parses` unverified. |
| **Validation** | Real-world accuracy — plan and corpus already chosen: `docs/plans/slice-15-real-world-validation.md`. ~~**Needs extending**: it predates Python and framework support, so its corpus table is TypeScript-only and its category list has no Python or framework rows.~~ **That was stale and is corrected (`08b6d9b`).** The plan already had a "Python, added after Slices 9a, 9b and 10a landed" section, a Python oracle §3.1, a framework oracle §3.2, trace evidence §3.3, and Python + Framework category lists in §4 with their own FP/FN lines. The **real** gap was rows 12–14, now filled: history/rename/shallow categories, cross-repository contract categories, a **new §3.4 Git-object oracle**, and the recorded decision that row 14 gets **no precision number** because there is no ground truth for whether a human's note is correct. §3.4 matters beyond its section: `git` is not a second implementation to adjudicate against like `tsc` — it reads the same immutable objects, so a history disagreement is a Nerve defect with no "both reasonable" case. That principle already caught a real defect in 12c-i-a. Network verified available again **2026-08-04** (`git ls-remote` against GitHub succeeds). |
| **Acceptance** | `docs/FINAL-ACCEPTANCE.md` and `scripts/final_acceptance.sh` **exist and pass 35/35** — this row said they did not, and was stale. The CLI has **14** commands (this row said 13, and so did `FINAL-ACCEPTANCE.md:59`; the script itself was right): `init index coverage trace status check doctor search gaps impact path serve mcp why`. `sync`, `affected`, `trace-tests`, `history` and `memory` are not among them — `affected` is *refused* by ADR-0008 and `trace-tests` by the Slice 11 plan, and the script encodes a refusal as a **pass**, not a gap. `history` arrives with Slice 12b and `memory` with row 14, at which point the script's "must not exist" checks for those two become "must exist". **The 35 checks gate what is built and must grow with rows 12b–14 and the UI pass.** |
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

- **Delegation was available on 2026-08-04 (later session) and is the recorded state to assume next.**
  A cheap read-only dispatch succeeded, then a full implementation subagent completed 12c-i-a. **It took
  109 minutes for one sub-slice**, which is the planning number that matters: the remaining rows (12c-i-b
  through 12c-iv, 13a–13d, 14, the UI pass, validation, acceptance expansion, the clean-checkout audit)
  are **many sessions**, not one. Test delegation with one cheap dispatch each session anyway — the
  spend limit reset silently and could return the same way.
- **Two background jobs were killed mid-run on 2026-08-04** by an infrastructure event, not a spend
  limit. Nothing was lost: `fmt`/`clippy` results and a partial test count survived in the scratchpad
  and the working tree was untouched. The lesson that worked is to **commit each plan as its own
  commit** rather than batching — six commits were already durable when the kill happened.
- **Probing is much cheaper per target than per workspace.** A `cargo test --workspace` probe cycle
  exceeds a two-minute foreground limit and had to be backgrounded; `cargo test -p nerve-store --test
  history` is seconds once warm. Probe with the narrowest target that can fail, and save/restore the
  file with `cp` rather than `git checkout` — a checkout would discard the whole uncommitted slice.
- **An org monthly spend limit killed the Slice 12b CLI agent mid-slice** (2026-08-04) — the **third**
  such kill on this project, after Slice 7b and Slice 9b. Its 1029 surviving lines in
  `crates/nerve-cli/src/main.rs` compiled and smoke-tested correctly, so they were inspected, kept,
  and finished by the orchestrator, which wrote the eleven CLI tests, the handoff entry and the slice
  report directly. **That is the recorded fallback and it worked**, but it means delegation should be
  assumed unavailable until the limit resets: a fresh session should test it with one cheap dispatch
  before planning around subagents. The mutation probes carry more weight than usual for that part of
  the slice, because the two-party check was unavailable.
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

## Found 2026-08-03 while preparing Slice 12b — open unless marked fixed

- **FIXED (`ee7f124`)** — `crates/nerve-server/tests/layering.rs` scanned a hand-written
  `include_str!` array of **16** files against a `src/` holding **17**. Four invariants read from
  it (no SQL, no second graph walker, loopback-only bind, no CORS header), so a new module was one
  commit from being exempt from all four — and `token.rs`, the omitted file, had never been scanned
  by any of them. Now a recursive `read_dir` with an anti-vacuity floor, mirroring the fix Slice
  7c-ii already made in `crates/nerve-cli/tests/cli.rs`. Probe: a new `src/mcp/probe_history.rs`
  containing `SELECT` fails the test by name.
- **OPEN, fixed by 12b** — `gitinfo::head_commit` does not follow `commondir`. Measured on a real
  `git worktree add`: it returns `None`, so **indexing a linked worktree records no
  `repository_state.git_commit`**. Pre-existing; `pipeline.rs:649` is the only caller.
- **OPEN, guarded by 12b** — `discover::canonical_child` ends in `std::fs::canonicalize` and so
  requires the path to *exist*. It cannot validate a historical or otherwise absent path. Not a
  defect in the function; a trap for any slice that reaches for it. Rows 13 and 14 can make the
  same mistake.
- **OPEN, found by the 12c-i-b implementer, reported rather than fixed** —
  `delete_commits_with_unavailable_parents` (`crates/nerve-index/src/history.rs:490`) deletes **every**
  commit whose `parent_completeness` is not `root`/`parents_available` at the start of *each* sync, on
  the stated assumption that the new walk re-reaches it (`history.rs:335-338`). **If HEAD has moved, it
  does not** — the walk starts from the current tips, so a commit that is no longer reachable is
  deleted and never re-recorded. Verified by reading, not only reported: the delete is unconditional
  and the re-walk is reachability-bounded.
  **The defect is the asymmetry, not the deletion.** A commit with *available* parents that becomes
  unreachable stays recorded; one with *unavailable* parents in the same position is dropped. Two
  commits in the identical situation, two different outcomes, decided by a field that describes their
  parents rather than their reachability.
  The repair itself is right and must be kept — `git fetch --unshallow` can turn "unavailable" into
  "available", and 12b's plan §8.5 records that stale availability is the one thing this surface must
  not keep. What is wrong is repairing by *delete-then-rewalk* when the rewalk's reach can shrink.
  Out of scope for 12c-i-b, which is a read surface. **Fix in a corrective slice**, and the test must
  move HEAD backward between two syncs — no shipped fixture reaches it in one sync, because
  `history-missing` records two commits along one chain.
- **OPEN** — four slice reports do not exist: `slice-10b-report.md`, `slice-11a-report.md`,
  `slice-11b-report.md`, `slice-12a-report.md`. CLAUDE.md §7 requires one per slice. The ROADMAP
  rows for those slices are unusually detailed and carry the substance. Address in the
  documentation audit; do not fabricate detail the roadmap and plans do not already record.

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
- ~~**`docs/ARCHITECTURE.md` has drifted.**~~ **Fixed 2026-08-03.** Two of the four items in this
  entry were already stale when written: the crate table lists all five crates and the document
  already said `nerve-server` shipped in Slice 4a. The two real ones are fixed — the pipeline
  diagram now names all eight extractors plus the two ingestion commands, and the parallelism
  promise is corrected to record that it was **deferred**, not delivered.
  A worse error this entry did not mention was also fixed: the Repository State section claimed
  "all observations are scoped to a state", which **contradicts ADR-0006**. Schema v3 dropped
  `state_id` from `occurrence`, `observation` and `assertion_state`; state lives only on
  `repository_state` and `extractor_run`, and freshness is computed at query time. A document
  that contradicts an accepted ADR is worth more attention than one with a stale crate count.
- **The Slice 7a-ii report's fixture counts do not match the committed fixture.** It states 21
  entities / 9 symbols (`function 4`); `fixtures/ts-coverage` at HEAD yields 18 / 8 (`function 3`).
  That QA session ran against a tree that is not the committed fixture. The defect it found was
  real and is fixed; the numbers should not be quoted.
