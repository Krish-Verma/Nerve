# Slice 12b — the historical model

**Status:** storage layer implemented and verified; ingestion and CLI outstanding
**Depends on:** Slice 12a (`e2ecb23`) — the Git object reader
**Schema:** v5 → **v6** (additive; four new tables, no change to any existing table)
**Roadmap row:** 12b

---

## 1. Objective

Answer questions about what this repository *was*, from the object store 12a can already read,
without ever describing an absent history as an empty one.

12a is a reader and nothing more: zero entities, zero rows, `SCHEMA_VERSION` unchanged.
12b is the first slice in row 12 that persists anything.

### 1.1 Non-goals, deferred to 12c

Named here so "row 12" is not declared finished at 12b:

- Rename hypotheses from **content similarity** (12b ships exact-content only — §7).
- `first_observed` / `last_observed` **derived query surfaces** with boundary qualification.
- Change frequency, labelled co-change, diff between two arbitrary states.
- Historical **symbol** changes. 12b is file-granular. See §2.3.
- HTTP API, MCP tools, and the reference-UI view.

12b ships the write path, the availability model, and two read commands that make it
independently testable. 12c ships the derived questions and every remaining surface.
This is the same seam as 5a/5b/5c, 6a/6b and 11a/11b, and it is chosen for the recorded
reason: a slice bundling ingestion and a surface has cost this project five agents.

---

## 2. The questions, chosen before the storage

Per the brief's requirement that storage not be designed before the queries. Each row states
whether 12b's tables can answer it, and where the answer is assembled.

| Question | 12b | Where |
|---|---|---|
| Which commits are reachable, and in what order? | ✅ | `git_commit` |
| What changed in this commit? | ✅ **for a non-merge** | `git_change`, qualified by `changes_enumerated` — a merge has zero rows by decision (§6.2), and the column is what stops that reading as "nothing changed" |
| Which commits touched this path? | ✅ | `git_change` by `path` |
| Was this file added, deleted, or modified, and when? | ✅ | `git_change.change_kind` |
| Did this path's exact content appear at another path? | ✅ | §7, exact blob identity |
| Which history is unavailable, and *why*? | ✅ | `git_history_ingest` + §5 |
| Is this repository shallow, and where is the boundary? | ✅ | `git_history_ingest` |
| When was this **path** first / last observed? | 12c | derived over `git_change` |
| When was this **symbol** first / last observed? | **refused in row 12** | §2.5 — `git_change` is path-keyed and a symbol carries no path |
| Which files changed frequently? | 12c | aggregate over `git_change` |
| Which files changed together? | 12c | self-join over `git_change` |
| What changed between two repository states? | 12c | range over `git_commit` |
| Was this **symbol** added, removed, moved, renamed? | **refused in row 12** | §2.3 |
| Which relationships appeared or disappeared? | **refused in row 12** | §2.3 |
| What was the historical *impact* of a change? | **refused** | §2.4 |

### 2.2 Why the 12c rows are not 12b rows

Every 12c row is an aggregate or a range query over the same two tables. None of them needs a
schema change. Splitting them out costs nothing structurally and keeps 12b's diff reviewable.

### 2.3 Symbol-level history is refused in row 12, with the cost stated

Answering "was this symbol added or removed" requires parsing historical blobs into symbols.
The cost is not the parse count — it is bounded by churn, which is measurable (§4) — it is that
**every historical symbol becomes an identity**, and Nerve's entity table is defined as
describing the *current* repository:

- `EntityKind::is_symbol()` feeds `symbols_total`, which Slice **7a-iii was an entire corrective
  slice about**. A historical symbol counted there makes the rail lie.
- `entity_fts` is populated by an `AFTER INSERT` trigger on `entity`
  (`schema.rs:172`). A historical symbol is therefore searchable, and `nerve search` would
  return symbols deleted years ago with nothing in the row to say so.
- `EntityKind::path_role()` (Slice 8b-i) makes a path name whatever is at it. A historical path
  resolves as a selector to something that no longer exists.

Each of those is a measured invariant with committed tests. Symbol-level history is a product
capability that needs its own decision about a second identity namespace, and inventing one
inside 12b would put it behind three invariants it would quietly break. **Recorded as a
limitation, not shipped as a half-measure.**

### 2.5 "First observed" is a claim about a path, not about a symbol

`git_change` is keyed on `path`. `EntityKind::path_role()` (`vocab.rs:153-161`) gives
`Function | Method | Class | Interface` the value `PathRole::None` — a symbol entity carries no
path at all, only `occurrence.file_path` (`schema.rs:71`).

So for a symbol, a query over `git_change` answers **"when did the file containing it first
appear"**, which is a different claim wearing the same words. A function added to a five-year-old
file would report the file's age as its own.

12c's first/last-observed surface is therefore restricted to the path-bearing kinds —
`File`, `Directory`, `Module`, `Document` (`vocab.rs:151-152`) — and the symbol form is refused
for the same reason as §2.3. This distinction was found by adversarial review of this plan, which
had the row unqualified.

### 2.4 Historical impact is refused outright

`nerve impact` is a reverse closure over the *current* graph (Slice 7b). "The historical impact
of a change" requires the historical *graph*, which requires §2.3. There is no cheap honest
version: computing it over the current graph and labelling it historical would attribute
today's edges to yesterday's commit. Refused with the reason, in the manner of `nerve affected`.

---

## 3. What 12b does **not** put in the evidence model, and why

**Decision: commits are not entities, changes are not assertions.** History lives in four
dedicated tables. This contradicts the roadmap row's phrase "commit entities" and the
contradiction is deliberate.

### 3.1 Why the commit is not an entity

Nerve has ruled on *this half* of the question twice, and stated the rule both times:

- **`CoverageRun` *is* an entity** — `coverage.rs:19-21`, because it had to be an **endpoint** of
  `COVERS`: "It is impossible to state 'test X covers symbol Y' because no such endpoint exists
  to state it with."
- **`TraceRun` is *not* an entity** — Slice 11a decision 3, because a trace run is
  **provenance**, and `observation.environment` already existed to carry it. No schema change, no
  migration.

A commit is provenance for a change fact. The subject of "`src/lib.ts` was modified" is a
*path*, and a path in history is not a current entity (§2.3). So the commit has nothing to be an
endpoint *of*, and it follows `TraceRun`.

**This argument covers the commit only.** Adversarial review of an earlier draft correctly
objected that it was also being used to justify keeping the *change fact* out of the evidence
model, which it cannot: `docs/plans/slice-11a-trace-ingestion.md:39-45` demoted the trace *run*
to provenance while keeping the *fact* inside `assertion` + `observation`. 11a is precedent for
demoting a run, not for excluding a class of fact. The change fact rests on §3.2 and §2.3
instead, and those two stand on their own.

### 3.2 The evidence model exists to express doubt; a tree diff has none

Assertion / Observation / AssertionState carry evidence source type, directness, extractor id and
version, match quality, and a **query-time recomputed freshness**. Every one of those fields
exists to qualify a *derived* claim.

A tree diff is a primary-source fact read directly out of an immutable object. Routing it
through the evidence model costs three rows per fact plus a freshness computation whose answer
can never change, in order to express certainty about something already certain. Nerve already
keeps primary facts in plain tables: `repository_state`, `extractor_run`, `module_facts`.

This is not collapsing the evidence model (CLAUDE.md §3). No existing concept is merged. The
model is declined for facts it was not built for.

### 3.3 The one genuinely uncertain fact, and why it still cannot go in the model

A **rename hypothesis** *is* uncertain, and it is the one place where ADR-0003's "every conclusion
is backed by inspectable evidence" bites this design. ADR-0003 even has the field for it:
`observation.match_quality`, documented as "only for extractors that perform matching, `NULL`
otherwise". A rename hypothesis is matching.

It still cannot be an observation, and the reason is mechanical rather than philosophical: **an
observation requires an `assertion_id`** (`schema.rs`, `observation.assertion_id NOT NULL
REFERENCES assertion(assertion_id)`), and an assertion requires `source_entity_id` and
`target_entity_id`, both `REFERENCES entity(entity_id)`. A rename relates two *paths*, and the
`from_path` of a rename is by definition no longer in the tree. There is no entity to point at.
Manufacturing one is §2.3.

So the honest statement is: **nothing in 12b goes into the evidence model.** The uncertainty is
carried by named columns instead — `evidence` and `ambiguity` in §7 — which is weaker than an
evidence profile and is recorded as such in §11 rather than dressed up. An earlier draft of this
plan claimed the hypothesis went into the model and then put it in a plain table; that
inconsistency was found by adversarial review.

### 3.4 The additive consequence, which is the strongest evidence for the design

Because no entity kind, relation, source type or directness is added:

- `EntityKind::ALL`, `Relation::ALL`, `EvidenceSourceType::ALL` unchanged → `Relation` and
  `EntityKind` are **mirrored by index** in `apps/nerve-web/src/api/types.ts` and asserted by
  `crates/nerve-server/tests/ui_vocabulary.rs`. That test needs no edit.
- `assertion_state`, `entity_fts`, `symbols_total`, `entities_total`, selector resolution and
  `nerve search` are untouched.
- `nerve_store::canonical_dump` builds **explicit per-table queries** and does not enumerate
  `sqlite_master` (verified in `dump.rs:24`), so the four new tables are invisible to the dump
  comparison.

Two qualifications, both found by adversarial review, both of which narrow this claim:

**`documents.rs:331` does need a v6 rewind clause after all.** The dump half of the argument is
sound but the conclusion did not follow. `a_real_v3_database_migrates_to_exactly_what_the_current_build_produces`
rewinds a physically-current database with `DELETE FROM schema_version WHERE version >= 4` and
then calls `migrate()`, which replays every step above the recorded version — so `Step::Sql(V6)`
would run `CREATE TABLE git_commit` against a table that already exists and panic on the
`.unwrap()`. `IF NOT EXISTS` is not the escape: that test's own comment states the project's
position, that "a migration that tolerated re-application would hide a real double-apply. So the
downgrade has to be a real one." The rewind therefore gains `DROP TABLE` for all four new tables,
exactly as it already drops `module_facts.framework_version` for v5. Criterion 4 is corrected
accordingly.

**A new path guard is added, though none is modified.** §8.4 originally routed historical paths
through `discover::canonical_child`, which is unusable here (§8.4). 12b adds a filesystem-free
syntactic check beside it. That is an addition, not a change to an existing invariant — but the
claim below is "no existing invariant is *modified*", not "no code is added".

No entity kind, relation, source type or directness is added, and no existing table, index or
guard changes behaviour. That is the smallest footprint this slice can have.

### 3.5 Rejected alternatives

| Alternative | Rejected because |
|---|---|
| `EntityKind::Commit` + `Relation::ChangedIn` | §3.1 and §3.4. Also forces a `DEFAULT_RELATIONS` decision (it must be excluded, like `COVERS` and `TEST_OBSERVED_CALL`) and a `path_role`/`is_symbol` classification for a kind that is not code. |
| Historical paths as `File` entities | Moves `entities_total` (`query.rs:183`), makes deleted paths searchable through the `entity_fts` `AFTER INSERT` trigger (`schema.rs:172`), and makes a historical path resolve as a selector (`PathRole::Container`, `vocab.rs:152`). **Not** `symbols_total` — `is_symbol()` is `Function\|Method\|Class\|Interface` (`vocab.rs:122-127`) and `File` is not among them. An earlier draft claimed otherwise and adversarial review caught it; the three real consequences are enough. |
| `EntityKind::HistoricalPath` | A path that exists both now and historically then has two identities, which is the identity-collision class Slice 2a already corrected once. |
| Full graph snapshot per commit | §4 — measured 30.1× and 177× row amplification. |

---

## 4. Storage strategy — measured, not assumed

The brief and the roadmap row both require this to be measured. Measured on two real
repositories, counting only repository *shape* (no file contents read):

| Repository | Commits | Files at HEAD | Snapshot path-rows | Delta change-rows | Ratio |
|---|---|---|---|---|---|
| Nerve, at `2d68d58` | 85 | 420 | **22,958** | **762** | **30.1×** |
| Repository B (local, private, unnamed by policy) | 1,214 | 865 | **682,940** | **3,858** | **177×** |

Method: for the snapshot column, `sum over commits of |tree|`; for the delta column,
`sum over commits of |changed paths|`. Reproduced by `scripts/measure_history_storage.sh`
(added by this slice) so the numbers are re-derivable rather than quoted.

**The Nerve row names its commit because the subject moves.** Re-running the script one commit
later gives 86 / 420 / 23,378 / 763 / 30.6× — the ratio *rises* as history lengthens against a
stable tree, which is the trend the design rests on, but it means a bare figure with no commit
beside it is not reproducible. Repository B is pinned only by its commit count, since naming it
would put a private path in a committed document (§6.3 of the brief).

**The ratio grows with history depth**, because snapshot cost is `O(commits × tree_size)` while
delta cost is `O(total churn)`. 30.1× at 85 commits, 177× at 1,214. Extrapolating a
10,000-commit repository at Repository B's tree size gives roughly 5.6 M snapshot rows against
roughly 32 k delta rows.

682,940 rows would exceed the entire current graph of any repository Nerve has indexed. The
delta design is chosen on this evidence. Per-commit snapshots, selected snapshots plus deltas,
and on-demand reconstruction are all rejected: the delta table answers every §2 question
directly, so a reconstruction path would be cost with no query behind it.

**What the measurement does and does not say about merges.** `git diff-tree` without `-m` emits
**zero** lines for a merge commit — verified on a constructed merge. That happens to match §6.2's
decision exactly, so the script measures the design as specified rather than by accident of a
flag, and the comment in the script says so. But it means the delta column omits merges, so the
figures need a sensitivity check rather than a footnote: Nerve has **0** merge commits, so 30.1× is
unaffected; Repository B has **55** merges in 1,214 commits, so enumerating them at its mean of
3.2 changes would add roughly 176 rows, moving 177× to about **169×**. The conclusion does not
depend on the difference.

Measurements still owed by the implementation, and gated in §9: database growth on a real
ingest, ingest wall time, incremental re-ingest time, and query latency for the two read
commands.

---

## 5. History availability — the invariant this slice is really about

> A shallow boundary means "history before this point is unavailable to this repository", not
> "the project's history definitively begins here."

12a already reports the inputs: `StoreLimits::shallow: Option<Vec<Oid>>` (`None` = not shallow)
and `StoreLimits::promisor: bool`.

### 5.1 Five reasons a commit has no visible parent, kept distinct

`git_commit.parent_completeness`, a closed vocabulary:

| Value | Meaning | May a consumer say "history begins here"? |
|---|---|---|
| `root` | No parents in the commit object, **and** not a shallow boundary. | **Yes.** This is the beginning. |
| `shallow_boundary` | Listed in `.git/shallow`. The commit object may name parents; they are absent by declaration. | **No.** "Earliest commit visible in this checkout." |
| `parents_available` | Has parents, all present in the object store. | n/a |
| `parents_missing` | Has parents, at least one absent, and the shallow declaration was **read cleanly** and does not list this commit. Promisor, or corrupt. | **No.** Unexpected, distinct from shallow. |
| `parents_unverifiable` | Has parents, at least one absent, and Nerve **could not establish** whether that absence was declared. | **No**, and it may not be called corrupt either. |

`shallow_boundary` and `parents_missing` both mean "cannot see further" and are kept apart
because one is declared and expected while the other is a fault. Collapsing them would report a
corrupt repository as a shallow one.

**Why the fifth value exists.** Adversarial review of an earlier draft found the four-way
vocabulary was not always derivable from what 12a exposes, and that the undecidable case would
have been mislabelled `parents_missing` — the inverse of the error this section warns about,
reporting a *shallow* repository as *corrupt*. Three paths in `store.rs` produce it:

- `read_shallow` is `read_pointer_file(&common_dir.join("shallow"))?` (`store.rs:472`), and
  `read_pointer_file` returns `None` for absent, **over `MAX_POINTER_FILE_BYTES`**, or unreadable
  alike (`store.rs:367-375`). `StoreLimits::shallow == None` is defined as *not shallow*, so an
  oversized or unreadable `.git/shallow` reads as a complete repository.
- An unparseable line is counted `SHALLOW_ENTRY_UNPARSED` and **dropped from the boundary vector**
  while `Some(..)` is still returned (`store.rs:487-489`) — pinned by the committed test
  `a_malformed_shallow_line_is_counted_and_the_rest_is_kept` (`store.rs:1005`). A boundary oid can
  therefore be missing from a boundary list that looks complete.
- Past `MAX_SHALLOW_ENTRIES` the remainder is counted and dropped (`store.rs:481-483`).

**The rule:** `parents_missing` may be asserted only when no shallow-related refusal form was
counted for this repository — `SHALLOW_ENTRY_UNPARSED`, `SHALLOW_ENTRIES_EXCEEDED` — **and** the
`shallow` pointer file was not silently unreadable. Since the third case is indistinguishable from
absence inside 12a, 12b re-stats `<common_dir>/shallow` itself: a file that exists while
`StoreLimits::shallow` is `None` is exactly the undecidable case, and it yields
`parents_unverifiable`. That is a three-line check in 12b rather than a change to 12a's reader,
and it is the only place 12b looks at a `.git` path 12a already looked at.

### 5.2 The third reason, which the brief's list does not name

A commit can also have no visible parent because **Nerve stopped walking**.
`git_history_ingest.walk_terminated_by`:

`exhausted` · `commit_budget` · `shallow_boundary` · `missing_object` · `refused`

This is a distinct concept from history *availability*: the history is present on disk and Nerve
declined to read all of it. `first_observed` in 12c must be qualified by
`commit_budget` as well as by shallow state, or a bounded ingest silently becomes a claim about
the project's origin. Recorded here because it is the one boundary reason that is Nerve's own
doing.

### 5.3 The concrete trap, and the test that catches it

A root commit is diffed against the **empty tree**, so every file in it is `added`. That is
correct for a root.

Doing the same to a `shallow_boundary` commit reports **every file in the boundary tree as
newly added at the boundary** — which is precisely "the project's history begins here", stated
as data rather than as prose. It is the single most likely way this slice ships the error it
exists to avoid.

Therefore: a `shallow_boundary` commit gets `changes_enumerated = 'parent_unavailable'` and
**zero** rows in `git_change`. Zero rows because the parent tree is unreadable, not because
nothing changed — which is why `changes_enumerated` is a stored column rather than an absence
to be inferred.

**Gate (§9.7):** a test over a real shallow fixture asserts zero `added` rows at the boundary,
and the mutation probe that diffs the boundary against the empty tree must fail it with the
count of paths it wrongly reported.

---

## 6. Schema v6

Additive. Four new tables, no `ALTER` of any existing table, no data migration.
`Step::Sql(V6)`, appended to `MIGRATIONS` with the array length `5` → `6`, `SCHEMA_VERSION`
5 → 6, and `crates/nerve-store/tests/schema.rs:344` updated.

```sql
CREATE TABLE git_commit (
    repo_id              TEXT    NOT NULL REFERENCES repository(repo_id),
    commit_oid           TEXT    NOT NULL,   -- 40 lowercase hex
    tree_oid             TEXT    NOT NULL,
    parent_oids          TEXT    NOT NULL,   -- JSON array, listed order; [] for a root commit
    parent_completeness  TEXT    NOT NULL,   -- §5.1 closed vocabulary
    changes_enumerated   TEXT    NOT NULL,   -- §6.1 closed vocabulary
    author_time          INTEGER NOT NULL,   -- epoch seconds, signed, as the object records it
    author_tz            TEXT    NOT NULL,
    committer_time       INTEGER NOT NULL,
    committer_tz         TEXT    NOT NULL,
    author_ident         TEXT,               -- NULL unless --with-identity; §8.3
    committer_ident      TEXT,               -- NULL unless --with-identity; §8.3
    summary              TEXT    NOT NULL,   -- first message line, bounded, lossy UTF-8
    is_merge             INTEGER NOT NULL,
    PRIMARY KEY (repo_id, commit_oid)
);

CREATE INDEX idx_git_commit_time ON git_commit(repo_id, committer_time);

CREATE TABLE git_change (
    repo_id        TEXT    NOT NULL REFERENCES repository(repo_id),
    commit_oid     TEXT    NOT NULL,
    path           TEXT    NOT NULL,   -- as recorded in the tree, refused by canonical_child if hostile
    change_kind    TEXT    NOT NULL,   -- added | modified | deleted | mode_changed
    blob_oid       TEXT,               -- NULL iff deleted
    prev_blob_oid  TEXT,               -- NULL iff added
    mode           INTEGER,
    prev_mode      INTEGER,
    PRIMARY KEY (repo_id, commit_oid, path),
    FOREIGN KEY (repo_id, commit_oid) REFERENCES git_commit(repo_id, commit_oid)
);

CREATE INDEX idx_git_change_path ON git_change(repo_id, path);
CREATE INDEX idx_git_change_blob ON git_change(repo_id, blob_oid);

CREATE TABLE git_history_ingest (
    repo_id             TEXT    PRIMARY KEY REFERENCES repository(repo_id),
    head_oid            TEXT,               -- NULL on an unborn branch
    walked_from         TEXT    NOT NULL,   -- JSON array of tip oids
    commits_recorded    INTEGER NOT NULL,
    commit_budget       INTEGER NOT NULL,
    walk_terminated_by  TEXT    NOT NULL,   -- §5.2 closed vocabulary
    shallow             INTEGER NOT NULL,
    shallow_boundary    TEXT    NOT NULL,   -- JSON array of boundary oids, [] when not shallow
    promisor            INTEGER NOT NULL,
    refusals            TEXT    NOT NULL,   -- JSON object, form -> count, from StoreCounters
    reader_version      TEXT    NOT NULL,
    ingested_at         TEXT    NOT NULL
);
```

### 6.1 `changes_enumerated`

`enumerated` · `merge_not_enumerated` (§6.2) · `parent_unavailable` (§5.3) · `refused`
(a bound in §8.2 was hit; the count is in `git_history_ingest.refusals`)

A commit with zero `git_change` rows is never ambiguous: this column says which of the four it
is.

### 6.2 Merge commits are recorded but their changes are not enumerated

The walk records every reachable commit, merges included. Change enumeration is only defined
against a *single* parent, and a merge has several. Three options were considered:

- **Diff against parent 0** — this is `git log --first-parent`'s rule. Rejected for 12b because
  it double-counts: a file modified on a branch appears once in the branch commit and again in
  the merge, which corrupts the 12c change-frequency answer before it is written.
- **Diff against every parent** — costs `parents × tree` and produces a path with several
  conflicting change kinds for one commit, which the primary key cannot hold and a consumer
  cannot read.
- **Enumerate nothing, and say so** — chosen. Each non-merge commit contributes its changes
  exactly once, side branches stay visible because the walk still records them, and the merge
  carries `merge_not_enumerated` rather than looking like an empty commit.

The cost is stated: a change introduced *only* by conflict resolution inside a merge is
invisible to `git_change`. Declared as a limitation, counted, and revisited in 12c if the
measured frequency answer needs it.

### 6.3 Migration discipline

Per the brief and `schema.rs:4-6`: append only, never edit an applied migration.
Tests required in §9.4 — clean create, v5 → v6, v1 → v6, v2 → v6, v3 → v6, re-migration is a
no-op, interrupted migration leaves no partial state (each `MIGRATIONS` step already commits in
its own transaction — the test proves it for v6), malformed historical rows, foreign-key
integrity, and every existing CLI/API/MCP query still answering after the upgrade.

**No destructive migration.** v6 adds tables and touches no user row, so the brief's
"destructive migration against actual user data" approval gate is not reached.

`nerve history sync` must call `nerve_store::migrate` before any write, joining the four
existing writers (`init.rs:154`, `pipeline.rs:667`, `coverage_ingest.rs:283`,
`trace_ingest.rs:512`). The reason is the Slice 3b data-destruction bug documented at
`pipeline.rs:654-666`: a writer that does not migrate deletes rows and then silently fails to
re-insert them.

---

## 7. Historical identity — one dimension in 12b, kept as a hypothesis

### 7.1 Exact content identity only

Git stores no renames; `git diff` *detects* them. 12b ships the one signal that costs nothing
and claims the least:

A commit in which path `A` is `deleted` and path `B` is `added` with **the same `blob_oid`** is
recorded as a rename hypothesis with evidence `exact_content`. The oids are already in hand from
the tree diff, so there is no similarity computation and no threshold.

Stored as its own kind of row rather than a score:

```sql
CREATE TABLE git_rename_hypothesis (
    repo_id       TEXT NOT NULL REFERENCES repository(repo_id),
    commit_oid    TEXT NOT NULL,
    from_path     TEXT NOT NULL,
    to_path       TEXT NOT NULL,
    evidence      TEXT NOT NULL,   -- exact_content (12b); similar_content added in 12c
    blob_oid      TEXT NOT NULL,
    ambiguity     TEXT NOT NULL,   -- unique | many_from | many_to | many_both
    PRIMARY KEY (repo_id, commit_oid, from_path, to_path)
);
```

`ambiguity` is the point. Two files with identical content — an empty file, a copied licence
header, a re-exported barrel — split and merge constantly. When one deleted blob matches
several added paths, every pairing is recorded with `many_to` and **none is promoted**.
Ambiguous identity stays ambiguous. There is no threshold, no tie-break, and no single
"confidence" number: `evidence` and `ambiguity` are separate columns because they are separate
facts.

Content similarity is 12c's dimension, and it will be a *second* `evidence` value beside this
one, never a blend with it.

### 7.2 Why this is not `identity_link`

`identity_link` (Slice 1, populated in Slice 3) proposes that two **indexed entities** are the
same thing across indexing runs, keyed
`UNIQUE (repo_id, left_entity_id, right_entity_id, link_kind)`.

A git rename relates two **paths**, at least one of which usually is not a current entity at
all. Writing paths into columns named `left_entity_id` / `right_entity_id` would make those
columns hold two different kinds of identifier, and `link_kind` two different kinds of claim.
The table's `left`/`right` columns carry no foreign key, so the database would permit it — which
makes this a decision to state rather than one the schema will catch.

A path rename and an indexing-run move are evidence about possibly the same event. 12c may
report them side by side. Neither is derived from the other.

---

## 8. Ingestion

### 8.1 Walk

`nerve history sync [--max-commits N] [--with-identity] [path]`

1. `gitinfo::git_dir(root)` → `gitobj::ObjectStore::open(git_dir)`. This is the pairing 12a's
   doc comment already specifies, and it is what makes a linked worktree work: `open` follows
   `commondir`, without which a worktree reads as a repository with no history.
2. Tips: `gitinfo::head_commit(root)`. Detached HEAD yields the hex directly; an unborn branch
   yields `None`, recorded as `head_oid = NULL`, `commits_recorded = 0`,
   `walk_terminated_by = 'exhausted'`. **Ref enumeration is not in scope** — `gitinfo` exposes
   HEAD only, and adding branch/tag enumeration is a bounded reader change that belongs with the
   surface that needs it (12c). `walked_from` is a JSON array so that change needs no migration.

   **`gitinfo::head_commit` must first be fixed for linked worktrees, and this is a pre-existing
   defect rather than new work.** 12a made `ObjectStore::open` follow `commondir` precisely because
   a linked worktree has no `objects/` of its own. `gitinfo.rs:58-79` never learned the same
   lesson: it reads `<git_dir>/HEAD`, then `<git_dir>/<ref_name>`, then
   `<git_dir>/packed-refs`, with no `commondir` handling anywhere in the file.

   Measured on a real `git worktree add`: the worktree's private `HEAD` is
   `ref: refs/heads/feat`; `<wt_git_dir>/refs/heads/feat` **does not exist**;
   `<wt_git_dir>/packed-refs` **does not exist**; the ref lives in the common dir. So
   `head_commit` returns `None`, and 12b would record `head_oid = NULL`,
   `commits_recorded = 0` — *"this worktree has no history"*, which is the exact failure
   ROADMAP row 12a says `commondir` was added to prevent, reproduced one layer up.

   The blast radius is wider than 12b: `pipeline.rs:649` is `head_commit`'s only production
   caller, and it feeds `repository_state.git_commit`. **Indexing a linked worktree today
   therefore records no commit for the state**, silently. That is a pre-existing defect, it gets a
   regression test of its own, and the `history-worktree` fixture must assert
   `commits_recorded > 0` rather than merely not crashing — otherwise the fixture passes while
   producing nothing, which is the 11a-i failure shape.
3. Breadth-first over `parent_oids`, visited set keyed on oid, so a diamond is walked once.
4. Terminate on: budget reached (`commit_budget`), no unvisited parents (`exhausted`), a parent
   that is a declared shallow boundary (`shallow_boundary`), a parent absent from the store
   (`missing_object`), or a refusal (`refused`).
5. `ObjectStore::read` is three-valued and stays that way: `Ok(None)` — absent, recorded as
   `parents_missing` on the child; `Err` — refused and counted, never mistaken for absent.

### 8.2 Bounds

New constants, each with a test that the bound is reachable and enforced:

| Constant | Value | Refusal form |
|---|---|---|
| `MAX_HISTORY_COMMITS` | `5_000` | `history-commit-budget` |
| `MAX_TREE_ENTRIES` | `100_000` | `history-tree-too-large` |
| `MAX_CHANGES_PER_COMMIT` | `10_000` | `history-changes-too-many` |
| `MAX_SUMMARY_BYTES` | `512` | truncated, flagged, never silently cut |

`--max-commits` is clamped to `MAX_HISTORY_COMMITS`; a larger request is refused with the
clamp stated, not silently honoured. Every 12a bound (`MAX_OBJECT_BYTES`, `MAX_DELTA_DEPTH`,
`MAX_COMMIT_PARENTS`, …) continues to apply beneath these.

Tree diff is recursive over subtrees, and **subtrees with equal oids are skipped entirely** —
that is the property that makes the delta cost of §4 achievable rather than aspirational, since
an unchanged directory costs one oid comparison regardless of its size.

### 8.3 Author identity is off by default

`author_ident` / `committer_ident` are `NULL` unless `--with-identity` is passed.

Not one accepted question in §2 asks *who*. Storing contributor names and email addresses that
no query reads would put third-party personal data in the index for no product value, and
`.nerve/nerve.db` is `0600` local storage rather than a reason to collect it. Times and
timezones are always stored, because "when" is asked repeatedly.

"Who last touched this" is a real question and is **deferred, not refused** — the columns exist
and the flag implements them, so enabling it later needs no migration.

### 8.4 What ingestion must not do

No subprocess, no `git` binary, no hooks, no repository code, no network. `no_subprocess.rs` and
`no_network.rs` stay **byte-untouched**; the fixture generator is a shell script that Rust source
may not name, exactly as `tests/gitobj.rs::no_rust_source_references_the_fixture_script` already
enforces for 12a.

Tree entry names arrive as `Vec<u8>` and are **not** trusted.

**`discover::canonical_child` cannot be the guard here, and an earlier draft of this plan was
wrong to name it.** It ends in
`std::fs::canonicalize(&joined).map_err(|_| IndexError::PathEscapesRoot(..))?`
(`discover.rs:96-97`), so **it requires the path to exist on disk**.
`coverage_ingest.rs:88-90` already documents the consequence verbatim: its `PATH_REFUSED` covers
"a path that does not exist, because the guard canonicalizes and a path that cannot be
canonicalized cannot be proven to be inside the root."

Routed through it, 12b would have:

1. refused **every `deleted` change**, since a deleted path is by definition not on disk;
2. made `git_rename_hypothesis` **structurally always empty**, because §7 needs the deleted
   `from_path` — so acceptance criterion 10 could never have passed, and would have looked like a
   repository with no renames rather than a broken guard;
3. rewritten any surviving path to its symlink target, contradicting `git_change.path`'s own
   stated meaning, "as recorded in the tree";
4. counted every one of those as a **path-safety refusal**, hiding the defect behind a counter
   that reads as a security control working.

That is the 11a-i shape exactly: a green suite reporting an attack surface that was never
reached. It was found by adversarial review of this plan rather than by a test, which is the
cheaper place to find it.

`nerve_store::selector_shape` is not the alternative either: it is purely syntactic, but
`select.rs:385-407` splits a `qualifier:body`, which is precisely the footgun ROADMAP row 12a
recorded when it rejected the same function for a filesystem path.

**So 12b adds a filesystem-free syntactic choke point**, `discover::safe_tree_name`, and the work
it has to do is smaller than it looks because `gitobj::parse_tree` already refuses a per-entry
name that is empty, contains `/`, or is `.` or `..` (`tree.rs`). A path assembled from validated
entry names cannot contain a traversal segment. What remains for the new guard, per entry:

- the C0 range, `0x00`–`0x1f` — the identity-forgery class Slice 5a closed at
  `canonical_child`, which this path no longer passes through, so it must be closed again here;
- UTF-8 validity, decided rather than assumed, since `TreeEntry::name` is `Vec<u8>`;
- a backslash, which Slice 8b-i found was **not** refused where `../` was;
- a leading `/` on the assembled path;
- a recursion-depth bound, so a pathological tree cannot exhaust the stack.

Every refusal is counted by form, never dropped silently, and the `history-hostile` fixture must
produce a **nonzero** count for each form it claims to exercise.

Gitlinks (submodule commits, mode `0o160000`) are recorded as a change to the gitlink path and
**never followed** — a submodule is another repository, which is row 13's subject.

### 8.5 Incremental re-ingest

`git_commit` is keyed `(repo_id, commit_oid)` and a commit object is immutable, so re-ingest is an
`INSERT OR IGNORE` over commits already present. New commits cost their own changes and nothing more.

**Correction: the walk does not stop at the first recorded commit, and this sentence originally said
it did.** That contradicted §8.5.1 and criterion 11: a repaired commit is reached *through* recorded
commits, so a walk that stopped at the first one would never revisit it, and the repair would be
unobservable. Probed by the implementation — zero re-records. The walk covers the whole reachable
graph and skips only the **tree diff** for a commit already recorded, which is the cost that
matters: measured warm re-sync on Nerve's own 95 commits is 14 ms against a 257–314 ms cold ingest.

#### 8.5.1 Two columns are *not* immutable, and the ingester must repair them

Found by the storage implementation, verified, and accepted against this plan's own earlier
wording. "A commit is immutable" is true of `commit_oid`, `tree_oid`, `parent_oids`, the times and
`summary`. It is **false** of `parent_completeness` and `changes_enumerated`: those record what
*this repository could see at read time*, and a `git fetch --unshallow` — or a fetch that fills a
promisor hole — changes them.

Because `insert_commit` ignores the second insert, an unshallowed repository would otherwise keep a
former boundary commit at `shallow_boundary` / `parent_unavailable` with **zero change rows
forever**. That is availability data which is now false, at exactly the boundary §5.3 calls the most
likely way this slice ships the error it exists to avoid.

**The rule, and it is provable rather than heuristic:** a commit classified by what was *missing*
must be re-examined; a commit classified by what was *present* need not be. So on every sync, before
walking, the ingester deletes every commit whose `parent_completeness` is neither `root` nor
`parents_available` — together with its `git_change` and `git_rename_hypothesis` rows, in foreign-key
order — and lets the walk re-record them. `root` and `parents_available` are conclusions from
presence and cannot be improved by fetching; the other three are conclusions from absence and can.

This needs one store function, `delete_commits_with_unavailable_parents`, and it is cheap: on a
complete repository the set is empty, so an ordinary re-sync pays one indexed count.

#### 8.5.2 A commit and its changes are one transaction

Also found by the storage implementation. Nothing in the store layer can enforce it, and the
consequence of getting it wrong is precisely the ambiguity `changes_enumerated` exists to remove: a
crash between `insert_commit` and `insert_changes` leaves a commit claiming `enumerated` with no
change rows, and the next sync **skips it**, because `insert_commit` now returns `false`. The
ambiguity would then be permanent and indistinguishable from a legitimately empty commit.

The ingester wraps each commit's rows in one transaction. Tested by making `insert_changes` fail
mid-commit and asserting the commit row is absent afterwards, so a later sync re-records it.

**Rewritten history** (rebase, amend, force-update) leaves commits recorded that HEAD no longer
reaches. They are not deleted — they were really read, and deleting them would lose the record
of an ingest that happened. `git_history_ingest.head_oid` moves, so a commit unreachable from
the current head is reportable as such. 12c surfaces it; 12b records enough to.

### 8.6 The error bridge

12a deliberately left `gitobj::Error` unbridged, with the comment that 12b adds it "when it has
a caller for it". 12b is that caller: a new `IndexError` variant wrapping `gitobj::Error`,
carrying `Error::form()` so a refusal keeps its closed-vocabulary tag through to the CLI.

---

## 8.7 Security — the commit summary is a new kind of stored string

T9 already covers `.git` object data as untrusted input; 12a extended it there. 12b adds
something T9 does not yet describe, and it is worth naming precisely because it is easy to walk
past.

**Storing a commit summary makes Nerve store free-form repository prose for the first time.**

Slice 8a established the opposite property by accident and then relied on it: injection text
placed in a Markdown *body* came back **absent entirely**, because "Nerve stores ranges and
hashes, never source text". The only repository-derived strings in the graph today are *names* —
identifiers, headings, paths — which is why 8a found that the real T7 vector was a Markdown
**heading**, not body prose.

A commit summary is body prose. It is attacker-influencable in any repository that accepts
contributions, it is unstructured, and `nerve history log` is useless without it.

The decision is to **store it and confine it**, not to drop it:

- Bounded at `MAX_SUMMARY_BYTES` (512), truncation flagged rather than silent.
- First line only — a whole commit message is an unbounded prose channel with no query behind it.
- Lossy UTF-8 conversion, because `Commit::message` is `Vec<u8>` and may not be valid UTF-8.
- Never interpreted. It is data on every surface.
- **T7 applies to it in full** when history reaches MCP in 12c: a summary must live inside the
  `repository_content` region, and 8a's property test — which walks the whole response and
  asserts no string inside that field appears outside it — must cover it. 12b's read commands
  are CLI-only, so 12b's obligation is to bound and escape; 12c's is to confine.
- The UI must render it as text, never as HTML. `history-hostile` (§10) carries a summary with
  `<script>` and an instruction-shaped sentence so the escaping has something to catch.
- If `--with-identity` is passed, author names and emails are untrusted strings on the same
  terms.

`docs/THREAT-MODEL.md` T9 gains a paragraph saying this. It is an extension of T9 rather than a
new threat, in the manner of Slice 11a's restatement for traces — but it is **not** the same
control as 12a's, because 12a read object bytes and stored none of them.

---

## 9. Acceptance criteria

A criterion is met only when a named test asserts it and a mutation probe has been shown to
fail that test for the intended reason.

1. **Questions defined before storage** — §2, and the 12c/refused rows are explicit.
2. **Storage measured** — §4, two repositories, script committed, plus measured database growth
   and ingest time on a real repository reported in the slice report.
3. **Model documented** — §3 and §5, including the pushback against "commit entities" with its
   evidence.
4. **Migration** — clean create; v5→v6; v1→v6; v2→v6; v3→v6; re-migration a no-op; a step that
   fails leaves no partial state; malformed historical rows refused; foreign keys hold; every
   existing CLI, API and MCP query answers after the upgrade. **`documents.rs:331`'s rewind gains
   `DROP TABLE` for all four new tables** — it re-migrates a physically-current database and
   `Step::Sql(V6)` would otherwise replay `CREATE TABLE` against an existing one. The test's
   existing assertion that the downgrade actually changed the dump is what keeps that honest.
5. **Ingestion bounded and safe** — every §8.2 bound reachable and enforced by a test; no
   subprocess; `no_subprocess.rs` and `no_network.rs` byte-identical.
5a. **The new path guard is real** (§8.4) — `discover::safe_tree_name` refuses C0 bytes, invalid
   UTF-8, a backslash, a leading `/`, and over-deep recursion, each with a **nonzero** counted
   refusal. And the guard must not refuse a legitimate *deleted* path: a test asserts `git_change`
   holds `deleted` rows and `git_rename_hypothesis` is **non-empty**, because both were structurally
   impossible under the guard this plan first named.

   **Amended: the per-form counts cannot come from `history-hostile`, and the implementation was
   right to say so.** That fixture puts `../escape.txt`, `back\slash.txt`, `ctl\x01name.txt` and
   `nl\nname.txt` in the *same tree* as the subtree named `..`, and `gitobj::parse_tree` refuses a
   whole tree on any malformed entry (`gitobj/tree.rs:124-133`) rather than yielding a partial
   prefix. So the **12a format reader** refuses that tree before 12b sees any of its names. The
   fixture's README also declared it carries no invalid UTF-8, and a leading `/` cannot survive
   `parse_tree` at all.

   The per-form counts therefore come from **tree objects built byte-by-byte in the test**, one
   hostile name per tree with no sibling to be refused alongside it — the only construction in which
   a per-form count means what it says. `history-hostile`'s job is narrower and still real: the
   malformed trees are **refused and counted**, never dropped silently, and the ingest continues so
   the repository's other commits still land. Had this criterion been taken literally it would have
   been satisfied by a count that could never rise above zero, which is the 11a-i shape exactly —
   caught here by the implementation rather than by a later corrective slice.
5b. **`gitinfo::head_commit` follows `commondir`** (§8.1 step 2) — the `history-worktree` fixture
   asserts `commits_recorded > 0`, and a separate regression test asserts that indexing a linked
   worktree records a non-NULL `repository_state.git_commit`, which is a pre-existing defect this
   slice fixes.
6. **Shallow modelled honestly** — a real shallow fixture; the boundary commit is
   `shallow_boundary`, never `root`; `changes_enumerated = 'parent_unavailable'`; **zero**
   `added` rows at the boundary.
7. **The §5.3 mutation probe** — diffing a shallow boundary against the empty tree fails a named
   test, and the failure message states how many paths were wrongly reported as added.
8. **`commit_budget` distinct from shallow** — a bounded ingest of a complete repository reports
   `walk_terminated_by = 'commit_budget'` with `shallow = 0`, and the two are not conflated.
9. **`parents_missing` distinct from `shallow_boundary`** — a fixture with a genuinely absent
   parent object is not reported as shallow.
9a. **`parents_unverifiable` distinct from both** (§5.1) — with `.git/shallow` present but
   unreadable by 12a's bound, an absent parent is `parents_unverifiable`, **not**
   `parents_missing`. Tested in both directions, because "never says shallow" is satisfied by a
   value frozen at `parents_missing` — the same trap 7a-iii recorded for a count frozen at zero.
10. **Rename hypotheses stay hypotheses** — an ambiguous exact-content match records every
    pairing with `many_to`, promotes none, and produces no score.
11. **Merge handling** — a merge commit is recorded, carries `merge_not_enumerated`, and has
    zero `git_change` rows; a test distinguishes that from an empty commit.
12. **Anti-vacuity, per the brief's six requirements** — for every fixture: the construct is
    generated (asserted against the fixture inventory, as 12a does with `inventory.json`), the
    ingester consumed it (a nonzero count somewhere), the expected record or refusal count is
    nonzero where required, a controlled mutation fails the test, **no aggregate threshold
    stands in for a per-case assertion** (the 11a-i trap), and placeholder values satisfy their
    own field contracts (the 11b `__GIT_COMMIT__` lesson — a 40-hex field gets 40 hex).
13. **Read surface** — `nerve history log` and `nerve history file <path>`, text and `--json`,
    with empty, shallow, budget-bounded, unavailable and refused states all represented; CLI and
    JSON asserted to agree.
13a. **The summary is confined** (§8.7) — a hostile commit summary is stored bounded, escaped on
    output, never interpreted; truncation at `MAX_SUMMARY_BYTES` is flagged, not silent.
14. **Full gate** — `cargo fmt`, `cargo clippy -D warnings`, `cargo test --workspace
    --no-fail-fast`, `cargo build --release`, the Python tracer suite,
    `scripts/trace_python_e2e.sh`, and `scripts/final_acceptance.sh` extended with history
    checks.
15. **Dependency review** — no new crate is expected. `Cargo.lock` diffed and the count stated
    as measured, not estimated.
16. Committed, reported, `docs/CONTINUATION.md` updated, tree clean.

---

## 10. Fixtures

Generated by `scripts/make_history_fixtures.sh`, following
`scripts/make_gitobj_fixtures.sh`'s determinism rules verbatim: `GIT_CONFIG_GLOBAL=/dev/null`,
`GIT_CONFIG_SYSTEM=/dev/null`, fixed synthetic `GIT_AUTHOR_*` / `GIT_COMMITTER_*`, fixed dates,
`TZ=UTC`. **No developer identity in a committed fixture.**

| Fixture | Exercises |
|---|---|
| `history-basic` | linear history, add / modify / delete, root commit vs empty tree |
| `history-shallow` | `.git/shallow`, boundary commit, §5.3 |
| `history-rename` | exact-content rename, and an ambiguous many-to case |
| `history-merge` | a merge commit, and a genuinely empty commit beside it |
| `history-worktree` | a linked worktree, `commondir` |
| `history-missing` | a parent object removed, `parents_missing` distinct from shallow |
| `history-hostile` | tree entry names with traversal and C0 bytes, refused and counted |

An `inventory.json` written by Git itself, as 12a does, so the expected values come from Git
rather than from Nerve's own reader.

---

## 11. Risks

| Risk | Mitigation |
|---|---|
| The shallow boundary is diffed against the empty tree | §5.3, criterion 7. The headline gate. |
| A bounded ingest reads as a complete history | §5.2 `commit_budget`, criterion 8. |
| Change frequency double-counts through merges | §6.2, decided before 12c needs it. |
| Ingest time dominates on a large repository | `MAX_HISTORY_COMMITS`, equal-subtree skipping (§8.2), measured in criterion 2. |
| A rename hypothesis is read as a fact | `ambiguity` column, no score, criterion 10. |
| `ObjectStore` is not `Send`/`Sync` (`RefCell`) | History is persisted at ingest and queried from SQLite only. No `.git` access on any query path, so the server's worker pool never touches the store. |
| Third-party PII in the index | §8.3, off by default. |
| A truncated commit summary is flagged **per repository, not per commit** | Schema v6 has no per-commit truncation column, so from `git_commit.summary` alone a 512-byte summary is indistinguishable from a cut one. The flag lives in `git_history_ingest.refusals` as `history-summary-truncated` and in the ingest outcome. A per-commit flag needs v7; 12c decides whether the surface needs it. Found by the implementation. |
| A rename hypothesis carries no evidence *profile*, only two named columns | Stated in §3.3 rather than hidden. This is the one place the design is weaker than the evidence model would be, and the mechanical reason it cannot use it is that an observation needs an assertion, which needs two entity endpoints. |

---

## 12. What adversarial review changed

This plan was reviewed against the code before implementation, with the reviewer instructed to
refute rather than confirm. Ten findings were returned; every one was independently verified by the
orchestrator before being accepted, and the four that required running code are recorded with what
was measured.

| # | Finding | Verified how | Change |
|---|---|---|---|
| R1 | `documents.rs:331` **will** break — `migrate()` replays `Step::Sql(V6)` against existing tables | Read `schema.rs:466-478` and `documents.rs:364-381`; the test's own comment forbids `IF NOT EXISTS` | §3.4, criterion 4: the rewind gains `DROP TABLE` ×4 |
| R2 | `canonical_child` requires the path to **exist**, so every deleted path is refused and §7 is structurally empty | Read `discover.rs:96-97`; `coverage_ingest.rs:88-90` documents the property already | §8.4 rewritten; new `discover::safe_tree_name`; criterion 5a |
| R3 | `gitinfo::head_commit` has no `commondir`, so a linked worktree reads as having no history | **Measured** on a real `git worktree add`: private HEAD names `refs/heads/feat`, which exists in neither `<wt>/refs/` nor `<wt>/packed-refs` | §8.1 step 2; criterion 5b; a pre-existing `repository_state.git_commit` defect named |
| R4 | `shallow_boundary` and `parents_missing` are **indistinguishable** in three paths | Read `store.rs:367-375, 472, 481-489`; committed test `store.rs:1005` pins the drop-and-continue behaviour | Fifth value `parents_unverifiable`; criterion 9a |
| R5 | §3.3 claimed the hypothesis went into the evidence model, then put it in a plain table | Internal contradiction; `observation.assertion_id` → `assertion.*_entity_id` chain read | §3.3 rewritten with the mechanical reason, and the weakness admitted in §11 |
| R6 | 11a demoted the *run* to provenance but kept the *fact* in the model, so it is not precedent for excluding a fact class | Read `docs/plans/slice-11a-trace-ingestion.md:39-45` | §3.1 narrowed to the commit; the fact rests on §3.2 + §2.3 |
| R7 | Historical `File` entities do **not** break `symbols_total` | Read `vocab.rs:122-127` — `File` is not in `is_symbol()` | §3.5 row 2 corrected to `entities_total`, FTS, selector resolution |
| R8 | "First observed" over a path-keyed table is not a claim about a **symbol** | Read `vocab.rs:153-161`, `schema.rs:71` | New §2.5; the §2 row split into path and symbol, the latter refused |
| R9 | §2's "what changed in this commit ✅" was unqualified against §6.2 | Internal | §2 row qualified by `changes_enumerated` |
| R10 | The script's merge comment was wrong about `diff-tree` | **Measured**: `diff-tree` without `-m` emits 0 lines for a merge; Nerve has 0 merges, Repository B has 55 | Script comment corrected; §4 gains a sensitivity check (177× → ~169×) |

The reviewer also confirmed, having tried to break them: the §4 measurement reproduces byte-exactly;
`canonical_dump` does not enumerate `sqlite_master`; `ui_vocabulary.rs` needs no edit; the
table-list tests tolerate new tables; §2.3's three invariants are each real with committed tests;
`identity_link`'s endpoint columns carry no foreign key; **no §2 question refutes the
dedicated-table design**; and nothing in the plan contradicts `docs/CONTINUATION.md`'s
"do not relitigate" list. The roadmap override is justified in substance — it was §3's *argument*
that needed repair, not its conclusion.
