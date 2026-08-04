# Slice 12b — Git history ingestion

**Objective.** Answer questions about what this repository *was*, from the object store Slice 12a can
already read, without ever describing an absent history as an empty one.

**Plan:** `docs/plans/slice-12b-historical-model.md`
**Commits:** `24da6ef` fixtures · `4b0d926` storage and schema v6 · `848af72` the ingestion engine ·
this commit, the CLI surface
**Schema:** v5 → **v6**, additive: four new tables, no change to any existing table

---

## 1. User value

`nerve history sync` reads the repository's own commits and records what each one changed.
`nerve history log` lists them. `nerve history file <path>` says which commits touched a path,
including a path that no longer exists.

The value is not the listing — `git log` exists. It is that every answer says **what it could not
see, and why**. A shallow clone, a bounded read and a corrupt object are three different reasons
history stops, and none of them is "the project started here."

## 2. Scope, and what was deliberately left out

Delivered: the write path, the availability model, exact-content rename hypotheses, and three CLI
commands with text and `--json`.

Deferred to **12c**, and named in the plan so row 12 is not declared finished: similarity-based
rename hypotheses, first/last-observed query surfaces, change frequency, labelled co-change,
state-to-state diff, the HTTP API, MCP tools, the UI view, and glosses for the six new vocabularies.

**Refused in row 12, with the cost stated rather than half-shipped:**

- **Symbol-level history.** It needs every historical symbol to become an identity, and Nerve's
  entity table is defined as describing the *current* repository. A historical symbol would move
  `entities_total`, become searchable through `entity_fts`'s `AFTER INSERT` trigger, and resolve as a
  selector through `path_role()`. Three measured invariants with committed tests, one of which
  (Slice 7a-iii) was an entire corrective slice.
- **Historical impact.** A reverse closure over the *current* graph labelled historical would
  attribute today's edges to yesterday's commit. Refused in the manner of `nerve affected`.
- **"When was this symbol first observed."** `git_change` is keyed on `path`, and
  `Function | Method | Class | Interface` have `PathRole::None`, so the query would answer "when did
  the file containing it appear" while wearing the words of a different question.

## 3. Starting state

`2d68d58`, working tree clean, 1321 tests passing. Verified by the orchestrator before any work: fmt
clean, clippy 0 warnings, 1321 passed / 0 failed / 2 ignored, release build, 115 Python tracer tests,
`final_acceptance.sh` 35/35.

## 4. The storage strategy, measured

The roadmap row required this measured rather than assumed. `scripts/measure_history_storage.sh`,
counting repository shape only and reading no file contents:

| Repository | Commits | Files at HEAD | Snapshot path-rows | Delta change-rows | Ratio |
|---|---|---|---|---|---|
| Nerve at `2d68d58` | 85 | 420 | 22,958 | 762 | **30.1×** |
| A local 1,214-commit repository | 1,214 | 865 | 682,940 | 3,858 | **177×** |

**The ratio grows with history depth** — snapshot cost is `O(commits × tree_size)`, delta cost is
`O(total churn)` — so a single repository would have understated it. 682,940 rows exceeds the entire
current graph of anything Nerve has indexed. Per-commit snapshots, selected-snapshots-plus-deltas
and on-demand reconstruction are all rejected on this evidence: the delta table answers every
accepted question directly, so a reconstruction path would be cost with no query behind it.

Sensitivity: `git diff-tree` without `-m` emits nothing for a merge, which matches the chosen design
(§6.2 enumerates nothing for merges). Nerve has 0 merges so its figure is unaffected; the other
repository has 55 in 1,214, worth about 5% — 177× becomes roughly 169×.

Measured on the finished implementation, Nerve's own 95 commits: 1020 changes, **898 of 1446 subtrees
skipped** by the equal-oid shortcut, cold ingest 257–314 ms, **warm re-sync 14 ms**, database
+618 KB.

## 5. Architecture decisions

**Commits are not entities; changes are not assertions.** This contradicts the roadmap row's phrase
"commit entities", deliberately. Nerve has ruled on the entity half twice and stated the rule both
times: `CoverageRun` *is* an entity because it had to be an **endpoint** of `COVERS`; Slice 11a's
`TraceRun` is *not*, because it is **provenance**. A commit is provenance for a change fact whose
subject is a *path*, and a historical path is not a current entity.

The change fact rests on a separate argument: the evidence model's fields — source type, directness,
extractor version, match quality, query-time freshness — all exist to qualify a *derived* claim. A
tree diff is a primary-source fact read out of an immutable object. Routing it through
Assertion/Observation/AssertionState costs three rows per fact and a freshness computation whose
answer can never change, to express certainty about something already certain. Nerve already keeps
primary facts in plain tables: `repository_state`, `extractor_run`, `module_facts`.

The consequence is the strongest evidence for the design: **no entity kind, relation, source type or
directness is added**, so `entity_fts`, `symbols_total`, `entities_total`, selector resolution and
`ui_vocabulary.rs` are untouched, and zero frontend lines changed.

**Five reasons a commit has no visible parent, kept apart.** `root` · `shallow_boundary` ·
`parents_available` · `parents_missing` · `parents_unverifiable`. The fifth exists because adversarial
review found the four-way vocabulary was not derivable from what 12a exposes: `read_pointer_file`
returns `None` for absent, over-bound and unreadable alike, so an unreadable `.git/shallow` reads as
*not shallow*, and every boundary would then have been called a fault — the inverse of the error the
section warns about.

**A sixth reason history stops, which the brief's list does not name:** `commit_budget`. The history
is present on disk and Nerve declined to read all of it. That is Nerve's own doing and must never be
drawn as a property of the repository.

**Merges enumerate nothing.** A change is only defined against one parent. Diffing against parent 0
double-counts every change the branch already recorded, corrupting 12c's frequency answer before it
is written; diffing against every parent produces conflicting kinds for one path that the primary key
cannot hold. The walk still records the merge, and `merge_not_enumerated` says why it has no rows.

**Author identity is off by default.** Not one accepted question asks *who*. The columns exist and
`--with-identity` implements them, so enabling it later needs no migration.

## 6. The invariant this slice exists to protect

> A shallow boundary means "history before this point is unavailable to this repository", not "the
> project's history definitively begins here."

A root commit is diffed against the empty tree, so every file in it is `added`. Correct for a root.
Doing the same to a boundary reports **every file in the boundary tree as newly added at the
boundary** — the claim, stated as data rather than as prose.

`ParentCompleteness::may_claim_history_begins_here()` is true for `root` and nothing else, is
asserted exhaustively so a sixth value cannot default, and is the **only** copy of the rule: the CLI
calls it rather than re-deriving from the column, and the JSON exports it as a field so a consumer
never re-derives it either.

## 7. Files changed

| File | Change |
|---|---|
| `crates/nerve-core/src/vocab.rs`, `lib.rs` | six closed vocabularies, five exhaustiveness tests |
| `crates/nerve-store/src/schema.rs` | `SCHEMA_V6_DESCRIPTION`, `V6` DDL, `MIGRATIONS` 5→6, `SCHEMA_VERSION` 6 |
| `crates/nerve-store/src/history.rs` | NEW — 4 writes, 7 reads, 5 row structs, `HistoryTotals` |
| `crates/nerve-index/src/history.rs` | NEW — the walk, tree diff, availability, renames, bounds |
| `crates/nerve-index/src/gitinfo.rs` | `commondir` resolution (pre-existing defect) |
| `crates/nerve-index/src/discover.rs` | `safe_tree_name`, `TreeNameRefusal` |
| `crates/nerve-index/src/error.rs` | the `gitobj::Error` bridge 12a deferred |
| `crates/nerve-cli/src/main.rs` | `history sync` / `log` / `file`, text and `--json` |
| `scripts/make_history_fixtures.sh`, `fixtures/history-*` | 7 fixtures, 160 files, 105 KB |
| `scripts/measure_history_storage.sh` | the §4 measurement |
| `fixtures/ts-basic/golden.json` | `schema_version` 5 → 6, the only changed line |

## 8. Two pre-existing defects fixed

**`gitinfo::head_commit` did not follow `commondir`.** Measured on a real `git worktree add`: the
worktree's private HEAD names `refs/heads/feat`, which exists in neither the worktree's `refs/` nor
its `packed-refs`. It returned `None` — and its only production caller, `pipeline.rs:649`, feeds
`repository_state.git_commit`, so **indexing a linked worktree recorded no commit for the state.**
This is the failure ROADMAP row 12a says `commondir` was added to prevent, reproduced one layer up in
the ref reader rather than the object reader.

**`discover::canonical_child` cannot guard a historical path.** It ends in `std::fs::canonicalize`,
so it requires the path to exist; `coverage_ingest.rs:88-90` already documents the property. Routed
through it, every `deleted` change would have been refused and rename hypotheses would have been
*structurally always empty*, **while each refusal was counted as a path-safety success** — a green
suite reporting an attack surface that was never reached. `safe_tree_name` is the filesystem-free
replacement; the work is small because `parse_tree` already refuses a per-entry name that is empty,
contains `/`, or is `.` or `..`.

## 9. Corrections to this slice's own plan

Fourteen in total across four reviews. The plan's §12 records the first ten; these are the four the
implementation found afterwards.

1. **Two columns are not immutable.** `parent_completeness` and `changes_enumerated` record what
   *this repository could see*, and `git fetch --unshallow` changes them. Since `insert_commit` is
   `INSERT OR IGNORE`, a former boundary would keep `shallow_boundary` with zero change rows forever.
   The repair rule is provable rather than heuristic: **a commit classified by what was *missing*
   must be re-examined; one classified by what was *present* need not be.**
2. **§8.5's "the walk stops at the first recorded commit" made that repair unobservable** — a
   repaired commit is reached *through* recorded commits. Probed at zero re-records. The walk covers
   the whole reachable graph and skips only the tree diff.
3. **A commit and its changes must be one transaction.** A crash between them leaves a commit
   claiming `enumerated` with no rows — the exact ambiguity the column removes — and the next sync
   *skips it*, because `insert_commit` now returns `false`.
4. **Criterion 5a was unsatisfiable.** It required per-form refusal counts from `history-hostile`,
   but that fixture puts its four hostile names in the same tree as the subtree called `..`, and
   `parse_tree` refuses a whole tree rather than yielding a partial prefix — so 12a refuses the tree
   before 12b sees a name. **Taken literally, the criterion would have been satisfied by a counter
   that could never leave zero.** That is the 11a-i shape, caught by the implementer. Per-form counts
   now come from tree objects built byte-by-byte in the test.

Also: `contains() == true` does not mean a parent is *readable*; a `contains` refusal is weaker than
an absence and routes to `parents_unverifiable`; the golden dump carries `schema_version`, which
adversarial review missed by one step; and `tests/gitobj.rs:635` hard-pinned `SCHEMA_VERSION == 5`.

## 10. Anti-vacuity

Fifth vacuity trap on this project, and **the first caught by an author rather than a reviewer**: the
storage layer's first ordering test passed with the `commit_oid` tiebreak deleted, because
`idx_git_commit_time` lets SQLite satisfy `ORDER BY committer_time DESC` by scanning the index
backwards and returning ties in reverse rowid order, which the fixture's insertion order happened to
match. Rewritten with an order that is neither ascending nor descending.

The shallow test carries its own clause — *"the boundary tree is empty, so this test could pass
vacuously"* — and cross-checks the reader against Git's own count of the boundary tree's paths. The
CLI wording test asserts the boundary oid **appears** before asserting the forbidden phrases do not,
so a command that printed nothing cannot pass.

A subtle one worth recording: **`INSERT OR IGNORE` raises on a foreign-key violation but suppresses a
duplicate primary key** (verified directly against SQLite). So the foreign-key test would have passed
either way, and `a_repeated_change_row_is_an_error_rather_than_a_silent_drop` is the test that
actually catches a regression to Slice 3b's silent-data-destruction shape.

## 11. Mutation probes

Fifteen. Nine by the storage and ingestion implementers, six by the orchestrator. Each was shown to
fail the intended test for the intended reason, then reverted with the file confirmed byte-identical
by sha256.

The load-bearing one, run by the orchestrator: reclassifying the shallow boundary as ordinary **and**
mapping an unreadable parent to the empty tree fails with

```
the boundary was diffed against the empty tree: 2 of the 2 paths in the boundary tree were
reported as newly added, which states 'the project's history begins here' as data
```

**Two mutations are required, not one** — reclassifying alone leaves the test green, because that
fixture's parent object is genuinely absent. That is itself a finding, and both preconditions are now
asserted.

At the surface: replacing the boundary's wording with a root's fails the CLI test with
`claimed "the project's history begins here" about a shallow checkout`.

## 12. Security, privacy and clean-room

`crates/nerve-cli/tests/no_subprocess.rs` and `no_network.rs` are **byte-identical** to their
pre-session sha256 hashes. No subprocess, no network, no `git` binary in product code. `.git` object
data was already untrusted input from 12a; 12b adds the first **free-form repository prose** Nerve
stores — a commit summary — bounded at 512 bytes, first line only, lossy UTF-8, never interpreted,
and a T9 extension records it.

Author identity is off by default, so no third-party personal data enters the index unless asked for.
Fixtures carry a fixed synthetic identity; a byte scan of all 160 files plus 117 inflated loose
objects found no developer identity, no absolute path and no temporary directory name.

**No new dependency.** `Cargo.lock` unchanged at 106 packages. Nothing competitor-derived: the reader
is 12a's independent implementation.

## 13. Known limitations

- Truncation of a commit summary is flagged **per repository, not per commit** — v6 has no per-commit
  column, so a 512-byte summary cannot be told from a cut one. A per-commit flag needs v7.
- Changes introduced *only* by conflict resolution inside a merge are invisible to `git_change`.
- `git_rename_hypothesis` has no index, and `idx_git_change_blob` covers `blob_oid` but not
  `prev_blob_oid`, so 12c's cross-commit rename query would scan. A v7 one-liner if measurement
  warrants it; no speculative index was added.
- The repair SQL lives in `nerve-index` rather than `nerve_store::history`, because the store crate
  was off-limits to the ingestion agent. A future tidy should move it; the doc comment says so.
- Rename hypotheses carry `evidence` and `ambiguity` as named columns rather than an evidence
  profile. This is the one place the design is weaker than the evidence model, and the mechanical
  reason it cannot use it is that an observation needs an assertion, which needs two entity endpoints.
- `history-hostile`'s four hostile path names are unreachable through `parse_tree`; a future fixture
  revision could split them across trees.

## 14. Deviations from process

**The CLI implementation agent was killed mid-slice by an org monthly spend limit** — the third such
kill on this project, after Slice 7b's and 9b's. Its surviving work was 1029 lines in `main.rs`,
which compiled and smoke-tested correctly; it was inspected, kept, and finished by the orchestrator,
which wrote the eleven CLI tests, the handoff entry and this report directly. That is the fallback
`docs/CONTINUATION.md` already records, and the mutation probes carry more weight than usual because
the two-party check was unavailable for this part.

## 15. Next slice

**12c** — the derived historical questions and every remaining surface: first/last observed for
path-bearing kinds only, similarity renames as a *second* evidence value never blended with the
first, change frequency, labelled co-change, state-to-state diff, the HTTP API, MCP tools, the UI
view, and glosses for all six new vocabularies. `ui_vocabulary.rs` will not catch a missing gloss
until the TypeScript mirrors them, so adding the mirror is what turns the test back on.

`scripts/final_acceptance.sh` also needs its `history` "NOT BUILT" block turned into a real check —
it already prints `PASS … update this script` now that the command exists.
