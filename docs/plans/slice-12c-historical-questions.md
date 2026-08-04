# Slice 12c — the derived historical questions, and every remaining surface

**Status:** planned
**Depends on:** Slice 12b (`ac21727`) — ingestion, storage, availability, `history sync`/`log`/`file`
**Schema:** **v6, unchanged.** Every question below is an aggregate or a range over the two tables
12b already writes. No new table, no new column, no migration. See §3.
**Roadmap row:** 12c

---

## 1. Objective

Make the history 12b ingested *answerable*, on every surface, without any surface being able to
describe an absent history as an empty one or an earliest-visible fact as an origin.

12b is the write path plus two read commands. 12c is the question layer.

### 1.1 The split, and why there is one

12c as briefed bundles a store layer, a CLI family, an HTTP surface, an MCP surface and a UI view.
This repository has a measured rule against that shape: *"a slice bundling ingestion and a surface
has cost this project five agents"* (`docs/ROADMAP.md:251`, restated `docs/CONTINUATION.md:492`),
and the seam has been taken five times already — 2a/2b, 5a/5b/5c, 6a/6b, 11a/11b, 12a/12b.

12c is therefore four sub-slices, each independently verifiable and independently committed:

| | scope | gate |
|---|---|---|
| **12c-i** | The derived queries in `nerve-store`, the wording hoist into `nerve-core`, the CLI family | store tests + CLI contract tests + mutation probes |
| **12c-ii** | Similarity rename hypotheses — a **second** `RenameEvidence` value in `nerve-index` | precision measured against the exact-content set, no blending |
| **12c-iii** | `/api/history*` and the MCP history mode | contract tests, T7 envelope property test, layering scan |
| **12c-iv** | The reference UI view, and glosses for the six 12b vocabularies | browser QA, bundle-freshness guard |

Row 12 is complete at 12c-iv, not at 12c-i.

---

## 2. The questions, and what is refused

The brief lists fourteen questions. Each is answered here with where it lands, or refused with the
reason. Two are refused, and both refusals are pre-existing accepted decisions rather than new ones.

| Question | 12c | Where |
|---|---|---|
| What changed in a commit? | ✅ shipped in 12b | `history log --commit`; 12c-iii adds API/MCP, 12c-iv the view |
| What changed between two visible states? | ✅ **12c-i** | §5 — ancestry walk, *not* a time range |
| When was a current file first observed? | ✅ **12c-i** | §4 — five result kinds, and only one of them may say "created" |
| When was a current file last changed? | ✅ **12c-i** | §4 |
| When was a historical path added / deleted / modified? | ✅ shipped in 12b | `history file` |
| Was a path moved or copied? | ✅ 12b exact-content · **12c-ii** similarity | §6 |
| What rename hypotheses exist? | ✅ shipped in 12b | `history file` |
| What evidence supports a rename hypothesis? | ✅ **12c-i** wording · 12c-ii second value | `RenameEvidence` + `RenameAmbiguity`, never a score |
| Which relationships appeared or disappeared? | ❌ **refused** | Needs the historical graph. 12b plan §2.3, roadmap position 3. Computing it over the *current* graph attributes today's edges to yesterday's commit |
| Which paths changed most frequently? | ✅ **12c-i** | §7 |
| Which paths changed together? | ✅ **12c-i** | §8 — labelled an observation, never a dependency |
| What visible history is unavailable? | ✅ **12c-i** | §9, from `git_history_ingest` |
| Complete or shallow? | ✅ **12c-i**, on **every** response | §9 |
| What historical facts are stale? | ✅ **12c-i** | §10 |

### 2.1 The second refusal, restated because the brief re-raises it

Symbol-level history stays refused for row 12 (12b plan §2.3, §2.5). `git_change` is keyed on
`path`; `EntityKind::path_role()` gives `Function | Method | Class | Interface` the value
`PathRole::None` (`vocab.rs:153-161`). A first/last query over `git_change` for a symbol answers
*"when did the file containing it first appear"* — a different claim wearing the same words.

**Consequence, and it is a hard gate:** `history` queries accept a **path**, never a symbol
selector. A symbol-shaped selector is refused **as a refusal** with its reason, in the manner
Slice 8a established for traversal — never answered with its file's dates. §4.4.

### 2.2 What the brief asks for that is already true

*"Do not claim symbol-level historical identity if 12b only stores path-level facts."* It does only
store path-level facts, and §2.1 is the enforcement rather than a promise.

---

## 3. No schema change, and why that is load-bearing

Every query in §4–§10 is a `SELECT` over `git_commit`, `git_change`, `git_rename_hypothesis` and
`git_history_ingest`. 12c-i therefore:

- adds **no** table, column, index or `SCHEMA_VERSION` bump — so no migration, and no migration test;
- adds **no** `EntityKind`, `Relation` or `EvidenceSourceType` — so `entity_fts`, `symbols_total`,
  `entities_total`, selector resolution and `ui_vocabulary.rs` are untouched, exactly as in 12b;
- adds **two** derived vocabularies (§4.2, §9.1) that exist only in `nerve-core` and in responses.

The index question was checked rather than assumed, because "no schema change" is a claim and an
index is a schema change. Both aggregates are already served by 12b's indexes, read at `schema.rs`:

- §7 groups by `path` → `idx_git_change_path ON git_change(repo_id, path)` (`schema.rs:367`).
- §8 self-joins on `commit_oid` → the primary key is `(repo_id, commit_oid, path)`
  (`schema.rs:363`), so the join uses a PK prefix.
- §5 walks `parent_oids` by `commit_oid` → PK `(repo_id, commit_oid)` (`schema.rs:350`).

**Nothing is added.** The claim holds by inspection, not by intention.

One thing 12b already anticipated: `git_rename_hypothesis.evidence` carries the comment
*"exact_content (12b); similar_content added in 12c"* (`schema.rs:376`), so 12c-ii needs no column
either — the closed vocabulary in `nerve-core` gains a value and the text column stores it.

---

## 4. First and last observed — the section the slice exists for

### 4.1 The failure this prevents

The earliest `git_change` row for a path is *not* when the path was created. It is the earliest
change Nerve can see. Reporting it as creation is the same defect 12b's central invariant exists
to prevent — "the project's history begins here", stated as data — one query layer up.

Four independent reasons the earliest visible change is not the first:

1. the repository is **shallow** and the boundary is above it;
2. a parent object is **absent** (`ParentCompleteness::ParentsMissing`);
3. a parent is **unverifiable** (`ParentsUnverifiable` — 12b's fifth reason, added because an
   unreadable `.git/shallow` reads as *not shallow*);
4. the walk stopped at **`WalkTermination::CommitBudget`** — Nerve's own boundary, never a property
   of the repository.

### 4.2 `FirstObservedKind` — a new closed vocabulary in `nerve-core`

**Six** values, and the sixth is why this section was rewritten. Exactly one may be rendered as
creation.

| value | condition | may say "created" |
|---|---|---|
| `CreatedInVisibleHistory` | earliest change is `Added`, at a commit whose `parent_completeness.may_claim_history_begins_here()` | **yes** |
| `EarliestVisibleChange` | changes exist; history above is unavailable for one of §4.1's four reasons, which is named | no |
| `PresentBeforeVisibleHistory` | zero change rows, **and** the path is an indexed entity — so it exists now and was never touched in visible history | no |
| `AbsentFromVisibleHistory` | zero change rows, the path is not an indexed entity, **and an index exists** | no |
| `CurrentTreeUnknown` | zero change rows, **and there is no index** — see below | no |
| `NoHistoryIngested` | no `git_history_ingest` row at all | no |

`PresentBeforeVisibleHistory` is the value a happy-path draft omits, and it is the *common* case on a
shallow clone: every unchanged file has zero change rows. Without it the answer is an empty result,
which reads as "this file has no history".

`CurrentTreeUnknown` comes from a property of 12b found by reading its own command documentation:
**`nerve history sync` requires only `nerve init`, not an index** — *"History resolves nothing against
the graph, so a repository that has never been indexed still has a history to read"*
(`main.rs:361-364`). Distinguishing `PresentBeforeVisibleHistory` from `AbsentFromVisibleHistory`
requires knowing the current tree, and the only thing that knows it is the entity table. With no
index, Nerve genuinely cannot tell the two apart, and collapsing them either way is a claim it has no
evidence for.

**The current tree is read from the entity table, never from the filesystem.** A `stat` under the
repository root would need its own path guard for a path that may not exist, and
`discover::canonical_child` is exactly the function that cannot do that (§4.4). The basis is reported
as a field so a caller knows which it got.

A method `FirstObservedKind::may_claim_created()` is the **only** copy of the permission, exported in
JSON exactly as `may_claim_history_begins_here` already is, so no surface re-derives it.

### 4.3 `last_observed` has its own trap, and it is the opposite one

The latest visible change *is* the latest change **iff** the ingest's `head_oid` equals the
repository's current `git_commit`. If it does not, the answer is bounded above by the ingest, not by
the repository, and it is `stale` (§10) rather than wrong. A `last_observed` that silently means
"as of whenever sync last ran" is the staleness defect the brief names.

### 4.4 A symbol selector is refused, not answered

Per §2.1. The refusal names the reason (`git_change` is path-keyed) and the path the caller probably
meant is **not** guessed. Guessing it is precisely the "different claim wearing the same words"
failure.

`discover::canonical_child` **must not** be used to validate the path — it ends in
`std::fs::canonicalize` and so requires existence, which would refuse every deleted historical path
and make deletion queries structurally empty while counting each refusal as path-safety coverage.
12b added `discover::safe_tree_name` for exactly this and it is what 12c calls. This is written down
because `docs/CONTINUATION.md:578` records rows 13 and 14 as able to make the same mistake, and 12c
is the first row after the warning.

---

## 5. State-to-state diff — ancestry, not a time range

`commit_log` orders by `committer_time`. A time range is **not** an ancestry range: a merge brings
in commits whose committer time precedes the merge, and a rebase or a fabricated date reorders them
freely. Diffing "commits between two timestamps" would silently answer a different question.

12c-i therefore walks `parent_oids` from `to` toward `from`:

- `from` and `to` must both be **recorded** commits. Either absent → `state_not_recorded`, naming
  which, **never** an empty diff.
- If the walk exhausts without reaching `from` → `not_an_ancestor`, naming it. Not an empty diff.
- If the walk hits an unavailable parent before reaching `from` → `ancestry_incomplete`, with the
  `ParentCompleteness` that stopped it.
- Merge commits inside the range contribute **zero** change rows by 12b's decision, so the diff
  carries `merges_in_range` and `changes_enumerated` per commit. A merge-heavy range reporting few
  changes is expected, and the field is what stops that reading as "little changed".
- Bounded by commits walked and by rows returned; truncation reported (§11).

---

## 6. Similarity renames (12c-ii) — a second value, never a blend

12b ships `RenameEvidence::ExactContent`: one blob oid, two paths, in one commit. 12c-ii adds a
second value for the case where content changed *and* moved.

Three constraints, and the first is the one that could go wrong quietly:

1. **No score, and no blending.** `CLAUDE.md` §3 forbids a generic `confidence: float`. A similarity
   hypothesis is admissible as *structured match quality* — the named method and its measured value
   — and inadmissible as a number that a consumer could compare against an `ExactContent`
   hypothesis. The two evidence values are therefore **never merged, ranked against each other, or
   summed**, exactly as `py-framework` and `ts-js-framework` precision tables are never summed.
2. **The method is named in the evidence, not implied.** A line-multiset similarity over the two
   blobs is a different claim from a byte-level one, and the value is meaningless without which.
3. **Bounded.** Candidate pairing is `deletions × additions` per commit, and blob inflation is the
   12a bound. A per-commit cap is required, and a commit that exceeds it records **no** similarity
   hypothesis and says so — never a partial pairing presented as the full set.

Precision is measured on its own, against fixtures with ground truth written first, and reported in
its own table. It does not join the exact-content number.

---

## 7. Change frequency

`SELECT path, count(DISTINCT commit_oid) FROM git_change GROUP BY path`, bounded, ordered
deterministically (count desc, then path asc — a count tie must not order by rowid; 12b's fifth
vacuity trap was exactly an ordering test that passed with its tiebreak deleted).

Two honesty requirements:

- The count is **changes within visible history**, not lifetime changes. A shallow clone's numbers
  are a floor. The response carries the availability block (§9) like every other.
- Merges contribute nothing (§5), so a repository with a merge-heavy workflow undercounts against
  its own log. Stated in the response, not only in documentation.

---

## 8. Co-change — an observation, and the word is load-bearing

`git_change` self-joined on `commit_oid`, counting how often two paths appear in one commit.

**This is not a dependency and the response must make that unmistakable.** Two files changing
together is consistent with coupling, with a formatting sweep, with a release-version bump, and with
one commit that did two unrelated things. Nerve has already refused a weaker version of exactly this
inference twice: `ADR_DESCRIBES_COMPONENT` was refused because no deterministic rule separates
"describes" from "mentions" (`CONTINUATION.md:495`), and identity is never established by fuzzy name
matching alone (`CLAUDE.md` §3).

Enforcement, not just wording:

- the field is named `cochange_observations`, never `related`, `coupled` or `depends`;
- no `Relation` is emitted and no assertion is written — co-change exists only in the response;
- the response carries the sentence naming what it is not, and a test asserts that sentence is
  present and that the forbidden words are absent;
- a **shared-commit count**, never a normalised affinity, because a normalised number invites the
  comparison the label forbids.

---

## 9. Availability on every response

### 9.1 The block

Every response in §4–§8 and §10 carries the same block, assembled in **one** place:

```
repository_id · current_repository_state · requested_subject
history_ingested · shallow · shallow_boundary · promisor
walk_terminated_by · commits_recorded · commit_budget
refusals · reader_version
result_kind · freshness · truncation · continuation · limitations
```

### 9.2 The wording hoist — a defect 12b left for 12c, and it is four functions, not one

This section was drafted naming `availability_note()` alone. Reading `main.rs` found **four**
wording functions and **one interpretation predicate**, all of them inside the CLI binary:

| | at | renders |
|---|---|---|
| `walk_termination_note` | `main.rs:2151` | `WalkTermination` |
| `availability_note` | `main.rs:2180` | `ParentCompleteness` |
| `enumeration_note` | `main.rs:2210` | `ChangesEnumerated` |
| `ambiguity_note` | `main.rs:2230` | `RenameAmbiguity` |
| `earlier_changes_may_exist` | `main.rs` (same block) | **not wording — a judgment.** Whether history exists above what was read |

The last one is the worse finding. The other four are prose; that one is an *interpretation*, and it
is the single question every history surface must agree on. Three more surfaces each deriving it
independently is the §6.2 failure in its most consequential form — and the four notes are how it
drifts, because a surface that re-words `ShallowBoundary` slightly is a surface that has restated
the invariant 12b exists to protect.

**12c-i moves all four notes to `nerve-core`,** as inherent methods beside the vocabularies they
render, so `ParentCompleteness::note()` sits next to `may_claim_history_begins_here()` — the rule and
its rendering in one place. `earlier_changes_may_exist` moves to **`nerve-store`** beside
`IngestRow`, because it takes one and `nerve-core` does not depend on `nerve-store`.

Then all four surfaces call one function per question, and the CLI keeps only formatting.

A source-scan guard makes the single-copy property testable rather than aspirational, in the manner
`crates/nerve-server/tests/layering.rs` already scans `src/` dynamically (`ee7f124`): no crate
outside `nerve-core` may contain the note strings as literals. A duplicated copy fails by name.

---

## 10. Staleness

`git_history_ingest.head_oid` against the current `repository_state.git_commit`:

| | verdict |
|---|---|
| equal | `current` |
| differ | `stale`, naming both oids — historical facts describe an older HEAD |
| current `git_commit` is `None` | `unverifiable` — not `current`, and this is the distinction `nerve check` already draws between `Stale` and `Unverified` (Slice 7c-i) |
| no ingest row | `no_history_ingested` |

`unverifiable` is not a cosmetic fourth value. Reporting "unknown" as "current" is how a truncated
sweep becomes a clean bill, which 7c-i is an entire slice about.

---

## 11. Bounds, exit codes, and the shapes that must not collapse

- Every query bounded by an explicit limit with a default; truncation reported as a field, never
  inferred from `len() == limit` by the caller.
- Continuation is an **offset the query honours**, or `null` **with a statement** — 8b-ii's recorded
  decision, where the implementer declined to invent paging on one surface.
- Exit codes reuse the existing table. No new code: `no_history_ingested` is not an error (absence is
  not a failure — 2b's rule for "no path"), a refused selector is the existing refusal code, and a
  shallow answer is a **success** carrying a qualification.
- Four states that must stay distinct, because collapsing any pair is the whole class of defect this
  row guards: **no history ingested** ≠ **history ingested, path unknown** ≠ **path known, zero
  changes in visible history** ≠ **path known, changes exist, truncated away**.

---

## 12. Anti-vacuity and mutation probes

Per the brief §6.7, every feature is asserted at the fixture, at the store, at the service, in CLI
JSON, and (12c-iii/iv) at API, MCP and UI. Aggregate counts alone are never the assertion — this
project has caught **five** vacuity traps (8b's two T7 false passes, 10a's lambda test, 11a-i's four
inert artifacts, 12b's ordering tiebreak) and four of the five passed a green suite.

Probes for 12c-i, each of which must fail a **named** test for the **intended** reason:

| # | mutation | must fail with |
|---|---|---|
| 1 | `FirstObservedKind::may_claim_created()` returns `true` for `EarliestVisibleChange` | the shallow-fixture first-observed test, naming the boundary |
| 2 | `PresentBeforeVisibleHistory` collapsed into `NotInVisibleHistory` | a current-tree file on the shallow fixture reported as unknown |
| 3 | `CommitBudget` termination omitted from §4.1's reasons | a budget-bounded ingest claiming creation |
| 4 | state diff switched from ancestry to a `committer_time` range | the merge fixture, by count |
| 5 | `not_an_ancestor` returned as an empty diff | the unrelated-commits assertion |
| 6 | co-change field renamed `related_paths` | the forbidden-word test |
| 7 | a result bound removed | the bound test, on row count |
| 8 | truncation flag hardcoded `false` | the truncation test |
| 9 | `availability_note` copied back into the CLI | the single-copy source scan, by crate and line |
| 10 | staleness `unverifiable` mapped to `current` | the no-git-commit fixture |
| 11 | a symbol selector answered with its file's dates | the §2.1 refusal test |
| 12 | path validated through `canonical_child` | every deleted-path test, and the count of refusals |

Probe 12 is the one recorded twice already (12b plan §12, `CONTINUATION.md:578`) and it is included
because a passing suite is not evidence it was avoided.

---

## 13. Files expected to change in 12c-i

| file | change |
|---|---|
| `crates/nerve-core/src/vocab.rs` | `FirstObservedKind`, `HistoryFreshness`; `ParentCompleteness::availability_note()` |
| `crates/nerve-store/src/history.rs` | `first_last_observed`, `state_diff`, `change_frequency`, `cochange`, `history_freshness` |
| `crates/nerve-cli/src/main.rs` | `history file` extended; `history diff` / `frequency` / `cochange` / `availability` added |
| `crates/nerve-store/tests/history.rs` | store-level assertions and bounds |
| `crates/nerve-cli/tests/cli.rs` | CLI contract, JSON shape, exit codes, refusals |
| `crates/nerve-core/tests/` (or `vocab.rs` unit tests) | vocabulary exhaustiveness, `may_claim_created` pinned per value |
| `fixtures/history-*` | whatever the assertions need that is not already there |
| `docs/` | `UI-BACKEND-HANDOFF.md`, `TESTING.md`, `ROADMAP.md`, `CONTINUATION.md` |

**Must not change:** `schema.rs` (§3), `no_subprocess.rs`, `no_network.rs`, `entity`/`occurrence`/
`observation`/`assertion*` tables, `symbols_total`, `Cargo.lock`.

---

## 14. Acceptance criteria for 12c-i

1. Six `FirstObservedKind` values, each produced by a fixture, each pinned individually, with an
   exhaustiveness check so a seventh cannot be classified by a `_` arm.
2. `may_claim_created()` true for exactly one value, asserted over all six.
3. On the shallow fixture: a file present in the tree with zero change rows reports
   `PresentBeforeVisibleHistory`, and **no** output describes it as created or as having no history.
   With the index removed, the same path reports `CurrentTreeUnknown` rather than either neighbour.
4. State diff refuses `state_not_recorded` and `not_an_ancestor` distinctly, neither as an empty diff.
5. Change frequency and co-change bounded, deterministically ordered with an explicit tiebreak, and
   truncation reported.
6. Co-change carries its disclaimer; the forbidden words are absent; no `Relation` row is written —
   asserted by a count over `assertion`, not by inspection.
7. Staleness distinguishes four verdicts, `unverifiable` among them.
8. `availability_note` exists in exactly one crate, enforced by a source scan with an anti-vacuity
   floor.
9. A symbol selector is refused with its reason; `canonical_child` is not on the path.
10. `SCHEMA_VERSION` is 6 and `schema.rs` is byte-unchanged.
11. All twelve probes in §12 fail a named test for the intended reason, and each is recorded with the
    failure text.
12. Full gate: `fmt`, `clippy -D warnings`, `cargo test --workspace --no-fail-fast`,
    `cargo build --release`, Python tracer suite, `scripts/final_acceptance.sh`.

---

## 15. Refutations of this plan's own first draft

Recorded so they are not re-litigated, in the manner of 12b's §12.

1. **A `nerve history path` command was drafted beside `history file`.** Both take a path and both
   answer about that path. Two commands for one subject is the redundancy the brief's §6.3 warns
   against, so `history file` is *extended* instead.
2. **The state diff was drafted as a `committer_time` range** because `commit_log` already orders
   that way and the query is trivial. §5 is the correction: it answers a different question, and the
   failure is silent.
3. **Co-change was drafted with a normalised affinity** (shared commits ÷ total commits) because it
   is more comparable across paths. Comparability is the problem — it invites exactly the dependency
   reading §8 forbids. A raw shared-commit count is kept.
4. **First-observed was drafted with four result kinds, then five, and is six.**
   `PresentBeforeVisibleHistory` was missing from the four — the *common* case on a shallow clone,
   where the vocabulary would have reported every unchanged file as having no history. `CurrentTreeUnknown`
   was missing from the five, found by reading `main.rs:361-364`: history syncs without an index, so
   with no entity table Nerve cannot tell "exists now, never changed" from "does not exist now", and
   both of the neighbouring values would have been a claim without evidence.
5. **`last_observed` was drafted as symmetric with `first_observed`.** It is not: its trap is
   staleness against current HEAD rather than unavailability below the boundary. §4.3.
6. **Similarity renames were drafted into 12c-i.** They need a bound, a named method, a precision
   measurement and their own fixtures — a second concern in a slice that already has one. 12c-ii.
</content>
