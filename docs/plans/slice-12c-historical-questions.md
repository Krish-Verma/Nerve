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

### 1.2 Execution order changed after 12c-i: iii and iv run before ii

**Decided 2026-08-05, after 12c-i-b landed.** The numbering above is kept — renumbering committed
work would break every reference to it — but the **execution order is iii → iv → ii**, and the reason
is evidence rather than preference.

1. **The brief requires the surfaces and merely permits the feature.** *"Do not stop before all
   accepted human-facing features are operable in the current reference UI or explicitly documented as
   intentionally non-UI operations."* Everything 12c-i built is currently reachable only from a
   terminal: the measured UI matrix (`docs/plans/ui-parity-matrix.md`) records **zero** history UI and
   **zero** history endpoints. Similarity renames are this plan's own choice; the surfaces are the
   instruction.
2. **12c-ii's output would have nowhere to go.** It adds a second `RenameEvidence` value. With no
   `/api/history*` and no UI, shipping it means adding evidence that no surface can display — and then
   iii and iv would carry *both* the backlog and the new value. Running iii and iv first means the
   second value arrives into surfaces that already render the first.
3. **The duplication clock is running.** 12c-i-b was forbidden from editing `nerve-core`, so prose for
   `FirstObservedKind`, `HistoryFreshness` and `EarlierHistoryUnavailable` sits **inside the CLI
   binary** (`main.rs:2520-2529` records it). Every slice that adds a surface before that hoist is a
   slice that copies it. §9.2 exists because this exact thing already happened once with four
   functions; letting it happen twice in the same row would be choosing the defect knowingly. The
   hoist therefore moves into **12c-iii-a**, ahead of any new surface.
4. **12c-ii is the only part of row 12 that is a heuristic**, and it is the part most likely to need
   its own corrective pass. Exact-content renames — the honest, evidence-backed form — already ship.
   Deferring the heuristic behind the surfaces means the surfaces are complete either way, and a
   precision gate that fails does not block the row's usability.

**12c-iii is split**, on the recorded rule that a slice bundling two surfaces has cost this project
five agents: **12c-iii-a** is the three hoists plus the HTTP endpoints, **12c-iii-b** is MCP.

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

> **Corrected 2026-08-05, before 12c-ii was implemented. The paragraph above is false, and one
> column proves it.**
>
> `git_rename_hypothesis` carries `blob_oid TEXT NOT NULL` (`schema.rs:376`) — **one** blob column,
> because an exact-content hypothesis is *defined* by both paths naming the same blob oid. A
> similar-content hypothesis is defined by the opposite: the from-blob and the to-blob are
> **different objects**. There is no value that column can honestly take for a similarity row.
> Writing the to-blob there makes one column mean two things depending on a sibling column;
> writing a sentinel makes it mean nothing. Either is the failure `CLAUDE.md` §3 names —
> a measurement hidden inside a label.
>
> The claim was also **too narrow about what similarity evidence is**. `evidence` and `ambiguity`
> can record *that* a hypothesis is similarity-based and *how ambiguous* the pairing is. They cannot
> record the matching method, its version, the measured value, the threshold it was compared
> against, or whether the candidate set was complete. A hypothesis a reader cannot reproduce is not
> inspectable evidence, and this project's whole claim is inspectable evidence.
>
> The correction is **schema v7** (§6.1). It is additive in effect and lossless: every v6 row
> survives with `from_blob_oid = to_blob_oid = blob_oid`, which is exactly what an exact-content row
> already meant. §6 is rewritten below to specify it.
>
> 12b's own plan anticipated the second half of this: *"A per-commit flag needs v7; 12c decides
> whether the surface needs it."* (`slice-12b-historical-model.md:835`, on summary truncation).
> 12c-ii is that decision, and the answer is yes — §6.7.

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
| `CreatedInVisibleHistory` | earliest change is `Added`, **nothing is hidden above this path**, and **exactly one addition is recorded** — see §4.2.1 | **yes** |
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

### 4.2.1 The creation rule, corrected twice during implementation

This section first said the licence was `may_claim_history_begins_here()` on the earliest change's
commit — that is, a parentless commit. Two corrections, both found by running the code:

**First: it rests on no date, and that is why the parentless requirement looked right.** `first` is the
earliest change by `committer_time`, which a rebase or a fabricated clock reorders freely. So
*"the earliest dated change is an addition"* does **not** establish that it is the topologically first
change, and a rule resting on it would over-claim. Requiring a parentless commit sidesteps dates
entirely — nothing precedes a root.

But it also made the value **unreachable for every file outside a root commit** — on a complete clone
of this repository, 6 files of roughly 420 — while the response simultaneously reported
`earlier_history_unavailable: None` and `earlier_changes_may_exist: false`. A result kind meaning
*"history above may be hidden"* beside two fields stating nothing is hidden is not caution; it is a
third statement contradicting both, and every surface would have rendered it.

The rule that keeps the date-proofing without the incoherence is three facts:

1. the earliest recorded change is an `Added` — so the path was absent from a parent tree Nerve read;
2. **nothing is hidden above this path** (`earlier_history_unavailable` is `None`);
3. **exactly one addition is recorded** for the path.

(3) is what replaces the parentless requirement as the clock-independent part: a path created, deleted
and re-created records **two** additions, so one addition in a history where nothing is hidden means one
creation, whatever the timestamps say. A path with two additions is `EarliestVisibleChange` even though
nothing is hidden — the refusal there is about *ordering*, not availability, and that distinction is
asserted.

**Second: the two questions are at different scopes, and demanding they agree denies a provable
creation.** `earlier_history_unavailable` is about **this path**; `earlier_changes_may_exist` is about
**the repository's ingest**. A shallow clone can contain a genuine root — one branch fetched whole,
another truncated — so a path created at that root has nothing hidden above it while the repository
still reports that earlier commits may exist. A first draft of the fix short-circuited on
`ingest.shallow` for every path and broke exactly that case, caught by an existing control assertion.

So: `ParentCompleteness::Root` returns "nothing hidden" immediately; `ParentsAvailable` falls through to
the repository-wide checks, because a visible parent settles the immediate question and no more. The
narrow equivalence — that the path-level reason and the repository-level boolean agree **when the anchor
has available parents** — is asserted over every `WalkTermination`, and the independence is asserted for
the root anchor.

**One residual, carried as data.** A merge enumerates no changes (12b §6.2), so a path created inside one
merge and deleted inside another has both events unrecorded, and a later addition can look like a first
one. The response carries the repository's merge count, so a consumer can see whether the possibility
exists at all rather than reading it in prose.

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

The rest of §6 was written on 2026-08-05, before implementation, because the three constraints
above are a policy and not a specification: they do not say what is stored, what the method is, what
the numbers mean, or what makes the result reproducible.

### 6.1 Schema v7 — what the evidence has to survive as

Storage first, matcher second. A matcher whose output cannot be recorded honestly is not worth
writing.

**`git_rename_hypothesis` is rebuilt.** `blob_oid TEXT NOT NULL` is replaced by `from_blob_oid` and
`to_blob_oid`, because a similarity pair has two. The rebuild is SQLite's documented
recreate-and-copy, inside the migration's transaction, and it is lossless: every v6 row copies with
`from_blob_oid = to_blob_oid = blob_oid`.

```sql
CREATE TABLE git_rename_hypothesis (
    repo_id           TEXT    NOT NULL REFERENCES repository(repo_id),
    commit_oid        TEXT    NOT NULL,
    from_path         TEXT    NOT NULL,
    to_path           TEXT    NOT NULL,
    evidence          TEXT    NOT NULL,   -- closed vocabulary: RenameEvidence
    from_blob_oid     TEXT    NOT NULL,
    to_blob_oid       TEXT    NOT NULL,
    matcher_id        TEXT    NOT NULL,   -- which method produced this row
    matcher_version   TEXT    NOT NULL,
    match_numerator   INTEGER,            -- NULL iff evidence = exact_content
    match_denominator INTEGER,            -- NULL iff evidence = exact_content
    ambiguity         TEXT    NOT NULL,   -- closed vocabulary: RenameAmbiguity
    PRIMARY KEY (repo_id, commit_oid, from_path, to_path),
    FOREIGN KEY (repo_id, commit_oid) REFERENCES git_commit(repo_id, commit_oid),
    CHECK (...)
);
```

**The measurement is two integers, not a float.** `match_numerator / match_denominator` is an exact
rational. A float would reintroduce the thing §3 of `CLAUDE.md` forbids by the back door: it is
comparable against anything, it rounds, and it does not say what was counted. Two integers say
*"1,320 of 1,500 lines"*, which is a measurement a reader can check by hand.

**The `CHECK` constraint is where "never blended" stops being a convention.** Written out:

```sql
CHECK (
    (evidence = 'exact_content'
        AND from_blob_oid = to_blob_oid
        AND match_numerator IS NULL AND match_denominator IS NULL)
 OR (evidence = 'similar_content'
        AND from_blob_oid <> to_blob_oid
        AND match_numerator IS NOT NULL AND match_denominator IS NOT NULL
        AND match_denominator > 0
        AND match_numerator >= 0
        AND match_numerator <= match_denominator)
)
```

An exact-content row *cannot* carry a measurement and a similar-content row *cannot* omit one. A
future writer that tried to give an exact match a score, or to record a similarity hypothesis
without saying what was measured, gets a constraint violation rather than a code review. The
primary key needs no widening: for one `(commit, from_path, to_path)` the two blob oids are fixed,
so a pair is exact or similar and never both.

**Candidate-set completeness is per commit, not per row, and it needs its own table.** The decisive
case is the one the policy above already requires: when the candidate set exceeds a bound, the
commit records **no** similarity hypothesis. A per-row flag cannot state that, because there is no
row to carry it. An absence would once again have to be interpreted, which is the failure 12b's
`changes_enumerated` exists to prevent.

```sql
CREATE TABLE git_rename_analysis (
    repo_id               TEXT    NOT NULL REFERENCES repository(repo_id),
    commit_oid            TEXT    NOT NULL,
    matcher_id            TEXT    NOT NULL,
    matcher_version       TEXT    NOT NULL,
    threshold_numerator   INTEGER NOT NULL,
    threshold_denominator INTEGER NOT NULL,
    deletions_considered  INTEGER NOT NULL,
    additions_considered  INTEGER NOT NULL,
    pairs_considered      INTEGER NOT NULL,
    pairs_measured        INTEGER NOT NULL,
    completeness          TEXT    NOT NULL,   -- closed vocabulary: RenameAnalysisCompleteness
    unmeasured            TEXT    NOT NULL,   -- JSON object, reason -> count
    PRIMARY KEY (repo_id, commit_oid, matcher_id),
    FOREIGN KEY (repo_id, commit_oid) REFERENCES git_commit(repo_id, commit_oid),
    CHECK (threshold_denominator > 0 AND pairs_measured <= pairs_considered)
);
```

`matcher_id` is in the primary key so a second matcher can analyse the same commit later without a
migration, and it is on the hypothesis row so a row names its own producer rather than being
attributed by a join a caller might forget.

**Exact-content renames get no analysis row, and that is a claim rather than an omission.** The
exact matcher reads no blob content: the oids are already in the tree diff. It is therefore complete
exactly when the diff was enumerated, which `git_commit.changes_enumerated` already records. Giving
it an analysis row with a meaningless threshold would be inventing a measurement to fill a column.
This is the same separation §6's first constraint demands, made structural.

### 6.2 The method

`matcher_id = "nerve-line-multiset"`, `matcher_version = "1"`. Named in full so that changing it is
a version bump rather than a silent redefinition.

Given two blobs:

1. Inflate each through the 12a `ObjectStore`, refusing above the matcher's own byte bound (§6.4),
   which is far tighter than 12a's 64 MiB.
2. Refuse a blob containing a `NUL` byte. Binary content has no lines and a ratio over it means
   nothing.
3. Split on `b'\n'`. A single trailing empty segment is dropped, so `"a\nb\n"` and `"a\nb"` are the
   same two lines. No trimming, no case folding, no normalisation — the bytes are compared as Git
   stored them.
4. Build a multiset of lines per blob (`BTreeMap<&[u8], usize>`). Lines are compared by bytes, not
   by hash, so there is no collision question to reason about.
5. `numerator = Σ over distinct lines of min(count_from, count_to)` — the multiset intersection.
   `denominator = max(lines_from, lines_to)`.
6. Admit when `numerator × threshold_denominator >= threshold_numerator × denominator`. Integer
   arithmetic throughout; no float is computed anywhere on this path.

**Two properties of the method, stated because a reader will otherwise assume the opposite.** It
cannot see line *order*, so a file whose lines were reordered measures 1/1; that is a real property
of a multiset and it is documented rather than patched. And it cannot tell *shared content* from
*shared boilerplate* — a licence header is lines like any other. §6.5 is how that is handled, and
the answer is the threshold, not a heuristic.

### 6.3 Which pairs are even considered

A candidate is a path **deleted** in a commit paired with a path **added** in the same commit — the
same candidate shape 12b's exact matcher uses. Two consequences fall out for free:

- **A copy is never called a move.** A copy leaves the source in the tree, so the source is not a
  deletion, so the pair is never a candidate. The requirement that a move be evidenced by a
  deletion *and* an addition is satisfied structurally, not by a check that could be removed.
- **Similarity never re-derives an exact match.** A pair whose blobs are equal is skipped: it is the
  exact matcher's row, and emitting it twice under two evidence values would be the blend §6's first
  constraint forbids.

### 6.4 Bounds

Every one of these is a named constant with a test that exercises it. A bound that cannot be
exercised cannot be tested — the correction 12c-i-b already had to make for `WalkBudgetExhausted`.

| Bound | What it protects |
|---|---|
| `MAX_SIMILARITY_BLOB_BYTES` | bytes inflated for one blob, beneath 12a's own object bound |
| `MAX_SIMILARITY_LINES` | lines held in memory for one blob |
| `MAX_SIMILARITY_DELETIONS` | deletions in one commit considered |
| `MAX_SIMILARITY_ADDITIONS` | additions in one commit considered |
| `MAX_SIMILARITY_PAIRS` | `deletions × additions`, the quadratic term |
| `MAX_SIMILARITY_ROWS_PER_COMMIT` | output rows one commit may record |
| `MIN_SIMILARITY_LINES` | floor beneath which a ratio is not a measurement |

Delta reconstruction beneath the matcher is bounded by 12a's `MAX_OBJECT_BYTES`, its delta-depth
limit and its declared-size checks; the matcher adds its own tighter ceiling on top rather than
trusting the one below.

**When a bound refuses**, `completeness` records which, **no similarity row is written for that
commit**, and the exact-content rows are untouched. `RenameAnalysisCompleteness`:

| value | meaning |
|---|---|
| `complete` | every candidate pair was measured |
| `partial` | some pairs could not be measured; the rows present are **not** the full set |
| `refused_bound` | the candidate set exceeded a bound; **no** similarity row for this commit |
| `not_attempted` | the diff was not enumerated, so there is no candidate set |

`partial` is not a violation of "no partial set presented as exhaustive" — the prohibition is on
presenting it as exhaustive, and this names it. `unmeasured` carries the reasons as a JSON
`reason -> count` object over a closed vocabulary: `blob-absent`, `blob-unreadable`,
`blob-too-large`, `blob-binary`, `blob-too-small`.

### 6.5 Precision, and the threshold is an output of measurement

Ground truth is written **before** the matcher, as a committed fixture inventory, and every pair is
labelled rename or not-rename by construction rather than by running Nerve. The cases:

true rename with modification · delete and unrelated add · two similar boilerplate files ·
empty files · licence/header-heavy files · one-to-many · many-to-one · generated files · binary
files · files over the bounds · same content with lines reordered · minor edits · major edits ·
copy rather than move.

**The gate is false positives = 0, and the threshold is chosen to meet it.** The boilerplate and
licence-header cases are the ones that decide the number: a pair that is mostly a shared header
measures high, and the only honest lever is to require more. Recall is whatever that costs and is
**reported, not optimised** — a major-edit rename falling below the threshold is a false negative
and is published as one. Buying recall by lowering the threshold until a boilerplate pair passes
would be trading the gate for the number, which is the trade this project does not make.

Exact-content and similar-content precision are two tables. They are never summed and never
averaged.

### 6.6 What every surface must show

A similarity hypothesis rendered without its method is a percentage from nowhere. CLI, JSON, HTTP,
MCP and the UI each carry: that it is a **hypothesis, not a confirmed rename**; the evidence kind;
`matcher_id`; `matcher_version`; the measurement as **numerator of denominator**; the threshold;
the ambiguity; the candidate-set completeness; both paths; both blob oids; the commit; and the
limitations. Paths, summaries and evidence text remain untrusted repository content and stay inside
the MCP `repository_content` envelope.

### 6.7 Per-record summary truncation

`git_commit.summary` is bounded at `MAX_SUMMARY_BYTES = 512` and truncation is counted **per
repository** — `git_history_ingest.refusals["history-summary-truncated"]` and the ingest outcome's
`summaries_truncated`. From a single summary a reader cannot tell a short one from a cut one, which
is `slice-12b-historical-model.md:835`'s recorded limitation and its explicit hand-off to v7.

v7 adds `git_commit.summary_truncation`, and it is a **three-value closed vocabulary rather than a
boolean**, because a boolean would have to lie about the past:

| value | meaning |
|---|---|
| `complete` | the summary is the whole first line |
| `truncated` | the first line was longer than the bound and was cut |
| `unknown` | written before Nerve recorded this, and it cannot be recovered |

A v6 row cannot be backfilled. Length is not the answer: a summary of exactly 512 bytes is
**not** truncated, so `length(summary) = 512 ⟹ truncated` would manufacture false positives on
precisely the boundary case §6.8 tests. `DEFAULT 'unknown'` is the honest migration, and `unknown`
is reachable and tested through the v6→v7 upgrade path.

`insert_commit` is `INSERT OR IGNORE`, so a commit already recorded keeps its stored value — the
same documented limitation `history.rs:341-346` already states for `parent_completeness` and
`changes_enumerated`. It is stable in practice because the bound is a compile-time constant.

**No surface renders a summary without its flag.**

### 6.8 Tests that must exist

Truncation: below the limit · exactly at the limit · one byte above · multibyte UTF-8 cut at a
character boundary · invalid UTF-8 (lossy conversion happens *before* the bound, so a replacement
character cannot push a summary past it) · CLI · JSON · HTTP · MCP trust envelope · UI.

Migration: clean database at v7 · v6 upgrade preserving every rename row and every commit · a
representative older version upgraded end to end · rollback leaving the database at its prior
version on failure · re-migration is a no-op · existing API and UI queries still answer.

Mutation probes, each applied, each required to fail a **named** test for the **intended** reason,
each reverted and the file confirmed byte-identical: promote a similarity hypothesis to a confirmed
rename · remove `matcher_id` · remove `matcher_version` · remove the measurement · ignore the
threshold · report an incomplete candidate set as complete · blend exact and similarity evidence ·
admit a known false-positive boilerplate pair · remove a candidate bound · omit per-summary
truncation · render a truncated summary as complete · leak evidence text outside the MCP envelope.

### 6.9 Schema numbering after this slice

12c-ii takes **v7**. The Row 13 and Row 14 plans were written assuming v7 and v8 were theirs; both
are unimplemented, so both are corrected to **v8** and **v9** in their own files rather than here.
No applied migration is edited.

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
