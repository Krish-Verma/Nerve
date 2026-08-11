# Row 14 — human-confirmed project memory

**Status:** planned, not started
**Depends on:** row 13 (schema ordering only — v8 → **v9**)

> **Renumbered 2026-08-05.** Slice 12c-ii took v7, so Row 13 is v8 and Row 14 is **v9**. Every
> "v8" below means v9.
**Roadmap row:** 14

---

## 1. The claim this row must not make

The brief requires that *"agents may not confirm their own proposal."* Nerve is offline, has no
accounts, no network, no telemetry and no identity provider (`CLAUDE.md` §2). At a local CLI, **an
agent invoking `nerve memory confirm` is byte-indistinguishable from a human invoking it.** There is
no cryptographic, procedural or observational control available inside this product that separates
the two callers.

So the honest control is not an identity check. It is a **surface boundary**, and it is testable:

| surface | may propose | may confirm |
|---|---|---|
| CLI (`nerve memory …`) | yes | **yes** — this is the human's surface |
| MCP (the agent surface) | yes | **no — the code path is absent, not gated** |
| HTTP API | yes (read-only surface: **no**, see §5) | **no** |

> ### Corrected 2026-08-08 — "MCP may propose" would break an invariant already proved
>
> The row reading *"MCP · may propose: yes"* is wrong if "propose" means **persist** a `proposed`
> record, and the HTTP row contradicts itself in the same table. Persisting anything from MCP is a
> database write, and MCP's read-only behaviour is not an aspiration here — it is an invariant with
> a passing byte-hash test behind it (12c-iii-b proved the database byte-identical across a full MCP
> session, and a mutation probe confirmed the test catches a write). One `proposed` insert makes
> that test fail, and the honest options would be to delete the test or to weaken it.
>
> The corrected matrix — **MCP and HTTP are read-only, without exception**:
>
> | surface | read | propose *persistently* | confirm | supersede | invalidate |
> |---|---|---|---|---|---|
> | CLI | yes | yes | yes | yes | yes |
> | HTTP / UI | yes | **no** | **no** | **no** | **no** |
> | MCP | yes | **no** | **no** | **no** | **no** |
>
> An agent still has a useful path, and it needs no write: MCP may **list, search and inspect**
> memory, **report possible staleness**, and **return the exact `nerve memory …` command as text**
> for a human to run. That is a proposal in the only sense that matters — a suggestion a human
> executes — and it keeps the boundary in §1 real rather than negotiated. It is the same shape §5
> already requires of the UI: show the data, explain the boundary, print the command.

An agent reaches Nerve through MCP. `tools/call` has no confirmation tool and the confirm function is
not reachable from `crates/nerve-server/src/mcp/`, asserted by a source scan of the same shape as
`crates/nerve-server/tests/layering.rs`. That is a true statement Nerve can enforce.

**What must be written down rather than implied:** a human who hands an agent their shell has removed
the boundary, and Nerve cannot detect it. Stating that is the difference between a control and a
claim. It goes in `docs/THREAT-MODEL.md` as **T13**, and in `docs/SECURITY.md`'s limitations, in the
manner T11 records the `tiny_http` header bound as accepted-and-unmitigable.

This is the same discipline as `nerve affected`, which was **refused** because LCOV cannot support the
claim (ADR-0008 §A.2), rather than shipped with the attribution guessed.

---

## 2. Memory is evidence, and it is not an Assertion

`CLAUDE.md` §3: extractors emit **Observations only**, and `assertion_state` is *derived and rebuilt
as a pure function of observations*. A memory record does not fit that shape and must not be forced
into it:

- An `Assertion` is `(source, relation, target)`. A memory record is a *statement about* one subject,
  not a relation between two. There is no relation to name, and inventing one
  (`HUMAN_NOTED_ABOUT`) repeats the mistake `ADR_DESCRIBES_COMPONENT` was refused for.
- If memory entered `assertion_state`, it would be a human sentence inside a table defined as a pure
  function of machine observations. That is precisely "an unstructured agent-memory store that
  silently becomes truth", arrived at through the schema instead of through a feature.

**Decision: memory is its own table set with a subject reference into `entity`.** It is *offered
beside* evidence and never *mixed into* it — the same placement decision as `git_commit` in 12b and
for the same reason.

**The invariant, and it is mutation-probe-able:** `assertion`, `observation`, `occurrence` and
`assertion_state` are **byte-identical** before and after any memory operation. Asserted by hash over
those tables, not by inspection. A probe that writes a memory record into `assertion_state` must fail
a named test.

### 2.1 No confidence number

`CLAUDE.md` §3 forbids a generic `confidence: float`. A memory record carries **`status`** (§3), a
**`validity_anchor`** (the repository state it was confirmed against) and **citations**. Whether it is
still true is derived from the anchor at query time — the same query-time freshness `nerve why` has
used since Slice 2b — never stored as a score.

---

## 3. States, and the two that a first draft collapses

`proposed` · `active` · `potentially_stale` · `superseded` · `invalidated` · `conflicted`

- **`potentially_stale` is derived, never stored.** It is `active` plus "the anchor state is not the
  current state". Storing it would need a writer to keep it true, and the writer would be a query.
  This is the shape 12c-i's `history_freshness` establishes and 7c-i's `Unverified` before it.
- **`invalidated` is not `superseded`.** Superseded means *something replaced it*; invalidated means
  *it stopped being true and nothing replaced it*. Collapsing them loses the question "what did we
  once believe and no longer do, with no successor" — which is the one a returning human asks.
- **`conflicted` is derived too**: two `active` records on one subject whose content the resolver
  cannot order. Nerve **detects and reports, never resolves** — the same rule as a supersession cycle,
  which is *detected, counted and never suppressed* because each edge is individually evidenced
  (`CONTINUATION.md:499`).

> ### Corrected 2026-08-08 — six "statuses" are four plus three views, and a shared subject is not a contradiction
>
> **Stored and derived are different kinds and the plan lists them as one set.** §7.4 then requires
> "six statuses" while §3 says two of them are never stored, so the acceptance criterion contradicts
> the design it is checking. Separate them:
>
> | | values |
> |---|---|
> | **stored lifecycle** (`memory.status`) | `proposed` · `active` · `superseded` · `invalidated` |
> | **derived views** (query time, never written) | `potentially_stale` · `conflicted` · `multiple_active` |
>
> `multiple_active` is new and it is the point of the next correction.
>
> **Two notes about the same file are not a contradiction.** As drafted, `conflicted` fires on two
> `active` records sharing a subject "whose content the resolver cannot order" — and since the
> content is free prose, the resolver can *never* order it, so every second note on a subject becomes
> a conflict. That is a false claim manufactured by a rule, and it is the same error
> `ADR_DESCRIBES_COMPONENT` was refused for: asserting a semantic relationship that the evidence —
> here, two English sentences — cannot support.
>
> Corrected: a memory record may carry an **optional `claim_key`**, a short caller-supplied label
> naming *what question this record answers* ("owner", "deprecation-status", "retry-policy"). Only
> records agreeing on **repository + subject + scope + `claim_key`** are evaluated as competing
> structured claims and can be reported `conflicted`. Records without a claim key are
> **`multiple_active`** — several notes on one subject, which is normal and is reported as what it
> is. An explicit human-created conflict link is also admissible, because a human asserting the
> contradiction is evidence a resolver reading prose is not.

---

## 4. History is append-only

Three tables, schema **v8**:

```
memory(memory_id, repo_id, subject_entity_id, subject_kind, scope, content,
       author_label, created_at, anchor_state_id, status, supersedes, superseded_by,
       invalidated_at, invalidation_reason)
memory_citation(memory_id, cited_entity_id | cited_path, cited_span, cited_at_state)
memory_event(event_id, memory_id, at, operation, from_status, to_status, note)
```

`memory_event` is **append-only and never deleted**, including on invalidation. `nerve memory
supersede` writes an event and flips a status; it does not rewrite content and it does not delete a
row. Deletion is refused as an operation: **there is no `nerve memory delete`**, because the brief
requires history preserved and a delete verb is how it stops being.

`author_label` is a **local label, not an identity**. It records what the caller said it was, and the
schema comment says so, because a field named `author` in a product with no accounts invites being
read as authentication. Untrusted string, T7 terms.

> ### Corrected 2026-08-08 — `subject_entity_id` as written destroys memory on re-index
>
> **This is the row's most serious defect and it is mechanical.** `memory.subject_entity_id`
> referencing `entity(entity_id)` cannot survive ordinary operation, because entity rows are
> **routinely deleted**: `prune_orphans` issues `DELETE FROM entity WHERE …` (`prune.rs:376`, and
> again scoped at `:440`), and `deleting_a_file_removes_its_entities_assertions_and_observations`
> (`incremental.rs:290`) pins that as required behaviour. With `foreign_keys=ON` (`db.rs:37`) a
> memory row pointing at a pruned entity leaves exactly two outcomes:
>
> - **the delete is refused** — a human note about a file now blocks re-indexing that file, so
>   writing memory would break indexing; or
> - **the delete cascades** — the human's note is silently destroyed by a routine re-index, which is
>   the one thing a memory feature must never do.
>
> Neither is acceptable, so **memory does not hold a foreign key into `entity`.** It stores a
> *snapshot* and resolves the live subject at query time — the same move §3 already makes for
> staleness, and the same move Row 13 was corrected to make for its targets:
>
> ```
> memory(
>   memory_id, repo_id,
>   -- the subject, as it was when the human wrote it. No FK: entities are pruned.
>   subject_entity_id_snapshot, subject_kind_snapshot, subject_name_snapshot,
>   subject_path_snapshot, subject_selector_snapshot, anchor_state_id,
>   scope, claim_key,                       -- see §3 as corrected
>   content, author_label, created_at,
>   status,                                 -- STORED lifecycle only, four values
>   supersedes_memory_id,                   -- one direction; the inverse is derived
>   invalidated_at, invalidation_reason
> )
> ```
>
> Subject resolution is a **query-time verdict**, reported and never guessed:
> `resolved` · `resolved_through_identity_link` · `missing` · `ambiguous` ·
> `repository_state_unavailable`. The identity-link path matters: Nerve already records renames as
> `IdentityLink`, so a note about a moved file can often still be attached honestly — but only when
> a link says so, never by name similarity (`CLAUDE.md` §3).
>
> **Citations need the same treatment.** `memory_citation(cited_entity_id | cited_path, …)` has the
> identical problem and takes the identical fix: durable snapshots plus a resolution verdict.
>
> ### Supersession is stored in one direction
>
> The sketch stores **both** `supersedes` and `superseded_by`. Two independently writable directions
> of one fact can disagree, and nothing in the schema would notice — the same "two writable copies of
> one fact" Row 13 §4.1 rejected for cross-repository links. Store `supersedes_memory_id`; derive the
> inverse. The status change and the `memory_event` append are **one transaction**, or a crash
> between them leaves a superseded record with no record of being superseded.

---

## 5. Surfaces, and the one that stays read-only

`nerve serve` is a **read-only** HTTP API and has been since Slice 4a: one `PRAGMA query_only`
connection per worker, `POST` → 405, read-only proven by sha256 before and after
(`ROADMAP.md:218-223`). Row 14 does **not** change that.

So: the HTTP API **reads** memory and cannot write it. `GET /api/memory` and
`GET /api/memory/<id>`; no write route, no `POST`. The reference UI therefore *displays* memory,
proposals, conflicts and audit history, and for confirmation it does what the lifted interface freeze
already requires of a CLI-only operation: **shows the imported result, explains the boundary, and
prints the exact command** — never a disabled button implying unfinished work
(`CONTINUATION.md:313-316`).

That is not a shortcut. Making the API writable would relax the one control that makes `query_only`
provable, for a mutation whose whole point is that it is deliberate.

---

## 6. The acceptance-script defect this row must fix first

`scripts/final_acceptance.sh` has an "unbuilt commands" loop that awards a **`PASS` for a command's
mere existence**. `nerve history` moved 35 → 36 checks by appearing, a pass that checked nothing, and
it was replaced by eight real checks in `2dc3a7d`. **`nerve memory` is still in that loop**
(`CONTINUATION.md:72`), so row 14 will award itself the same empty pass.

**Fix it in the same commit that lands the command, not after.** The replacement checks must fail if:
memory creation does not persist · a proposal can self-confirm through MCP · supersession destroys an
event row · invalidation is not reflected in a read · a read mutates the database · memory text
escapes the MCP trust envelope · the UI cannot display a created record.

---

## 6b. Export, and why import is conditional (added 2026-08-08)

Memory is the only thing in the database a human authored, so it is the only thing whose loss is not
recoverable by re-indexing. **`nerve memory export` ships: versioned, deterministic, local, offline**
— a stable schema version, canonical key order, and byte-identical output for identical input, in
the manner `fixtures/*/inventory.json` is already required to be reproducible.

**Import ships only if it can be made safe**, and safe means all of: a `--dry-run` that reports
exactly what would change and writes nothing · schema-version validation with refusal on unknown ·
stable ids so a re-import is not a duplicate · explicit duplicate handling · **refusal on repository
mismatch**, because importing another repository's memory silently re-subjects every record ·
no silent overwrite of an existing record · bounds on file size and record count · one transaction
with full rollback. If any of those cannot be met, **export ships and import is explicitly deferred**
with the reason recorded — a half-safe import is how a human's notes get overwritten by a file.

---

## 6c. Sub-slices (added 2026-08-08)

Same reason as Row 13 §3 — a slice bundling a store layer and a surface has cost this project five
agents.

| | scope | independently testable at |
|---|---|---|
| **14a** | schema v9, durable subject snapshots, the read model and query-time resolution | a memory row whose subject entity has been pruned still resolves to `missing`, not to nothing |
| **14b** | CLI lifecycle, citations, `memory_event`, export | supersede + invalidate leave every prior event readable |
| **14c** | read-only HTTP and MCP, T7 and T13 | database byte-identical across a full MCP session |
| **14d** | functional UI and the semantic acceptance checks that replace §6's existence loop | the UI displays a record, explains the boundary, prints the command |

**14b is split in two (added 2026-08-10), for the reason this table already gives.** 14b as scoped
above is a vocabulary decision, a schema migration, a store layer and a command family in one slice,
which is the bundling that cost this project five agents.

| | scope | independently testable at |
|---|---|---|
| **14b-i** | `MemoryScope` and `MemoryOperation`, schema **v10**, the transactional lifecycle writes | a lifecycle write that fails leaves neither the status change nor the event |
| **14b-ii** | the `nerve memory` command family, citations, export, and the acceptance rows that replace §6's existence loop | `nerve memory export` is byte-identical twice and contains a record just written |

---

## 6d. The `scope` domain, decided (added 2026-08-10, Slice 14b-i)

14a stored `scope` as an opaque non-empty string and recorded that **14b decides the values**
(§14a correction 1). This settles it, and it also settles the vocabulary 14a left open for
`memory_event.operation` (§14a correction 5).

### Why `scope` must be closed, and why the argument is not the one used for `status`

`status` was closed in SQL because a *derived* view could be *stored*. `scope` has no such
confusion, so that argument does not transfer and is not reused. The argument that does apply is
narrower and it is about **what scope is load-bearing for**:

```
multiple_active   groups by (repo, subject, scope)
conflicted        groups by (repo, subject, scope, claim_key)
```

Scope is in both grouping keys, so **a typo in a scope silently suppresses a conflict report.** Two
records that genuinely answer the same named claim about the same subject — one written
`operations`, one written `opertions` — land in different groups and are reported as unrelated
notes. That is a **false negative on the one contradiction claim this feature makes**, produced by a
spelling mistake, and no test can catch it because both spellings are legal.

The second failure is the one this project already has a name for. `nerve memory list --scope
opertions` against a free-form column returns **zero records**, which reads as *there are no notes*
rather than *there is no such scope* — `absence is not zero`, the rule 7c-ii's `doctor` and 7b's
unresolved account both exist to enforce.

A closed domain turns both into a refusal at the point of entry, which is where a refusal is useful.

### The values, and the axis they are on

`scope` must not duplicate the two fields beside it, and a first draft of this section did:

- it is **not** the kind of thing the subject is — `subject_kind_snapshot` holds that, and 14a's
  test values `"file"` and `"repository"` are exactly that redundancy. They are placeholders, which
  is what the plan says they are, and they are **not** the vocabulary;
- it is **not** the question the record answers — `claim_key` holds that (`owner`,
  `deprecation-status`, `retry-policy` are the plan's own examples).

The axis left, and the one that makes the grouping do work, is **which facet of the subject the
claim is about**. The worked case is the plan's own `owner`:

> Subject `src/payments/charge.ts`, `claim_key = "owner"`, two active records: *"the payments team
> owns this code"* and *"platform owns its deployment"*. Same subject, same named claim, **not a
> contradiction.** Without a facet axis Nerve reports one, which is `ADR_DESCRIBES_COMPONENT`'s
> refusal in a third place.

Four values. Each is admitted only because a real note **in this repository** already needs it —
the Slice 10 rule that *a vocabulary member with no producer is documentation, not a gate*:

| value | what the claim is about | a real producer, from this repository |
|---|---|---|
| `implementation` | what the code does and why it is written this way | *"the 7/8 similarity threshold is an output of measurement, not a preference — `c5` measures exactly 4/5"* |
| `interface` | what it promises to whoever calls or consumes it | *"`nerve affected` is refused, not unimplemented — ADR-0008 §A.2"* |
| `operations` | how it behaves when it runs: configuration, deployment, environment | *"`cargo` is not on `PATH` in this shell, interactive or not"* |
| `process` | how people work on it: ownership, review, convention | *"brief every implementation agent on why it must not commit"* |

`ownership` is deliberately **not** a value: *owner* is a `claim_key` in the plan's own text, and
promoting it to a scope would put one concept on two axes.

The residual is stated rather than hidden: **a retry policy is `implementation` when it is coded and
`operations` when it is configured**, and a human can reasonably file it either way. A misfiled
record is not lost — it is found by subject and by search — it only fails to compete with a record
filed elsewhere. That is the same trade `claim_key` already makes, and it is strictly better than
the free-form column, where *every* record is misfiled with respect to every typo.

### `memory_event.operation` is closed in the same migration

14a left it an open string because *"14b's commands own the verbs"*. 14b owns them now, and the
invariant that closes it is 14d's: **every event must be renderable**, and `ui_vocabulary.rs` can
only guard a vocabulary it knows exists — 12c-iv found eight unmirrored vocabularies where the
guard *could not fail* for them. Five values, one per mutating operation, and no more:

```
proposed · confirmed · superseded · invalidated · cited
```

`cited` is the one that is not a status transition. Its event carries `from_status = to_status`,
which [`MemoryEventRow`] already documents as the shape of *an event that changed no status*.

### Schema v10, and why it is not an edit to v9

`V9` shipped in `2190643` and `MIGRATIONS`' doc comment states that **editing an existing entry is
prohibited**; V1–V9 byte-identical is an asserted invariant with a passing test. So the two `CHECK`s
arrive as **v10**, by SQLite's table-rebuild procedure, and the rebuild is cheap **now** and never
cheaper: the feature has no command yet, so outside this repository's own tests no `memory` row
exists anywhere.

Unknown-value behaviour, in three places rather than one:

1. **`MemoryScope::from_str` / `MemoryOperation::from_str`** refuse with `NerveError::unknown`, the
   existing mechanism every closed vocabulary in `nerve-core` already uses.
2. **The `CHECK`** refuses a raw-SQL writer, which is the reason it is in SQL at all.
3. **The v10 migration refuses to run** against a database holding a `scope` outside the domain,
   naming the offending values, rather than dropping the rows or rewriting them to a default. A
   human's note is the one thing in this database re-indexing cannot rebuild (§6b), so a migration
   that silently edits one is refused on the same ground as a delete verb.

## 6e. The export format, decided (added 2026-08-11, Slice 14b-ii)

§6b requires export to be *"versioned, deterministic, local, offline"* and §7.4e requires the same
database to export **byte-identically twice**. Those two sentences together decide more of the
format than they look like they do.

**One JSON document, not JSONL.** §6b offered canonical JSONL "or another explicitly versioned
format", and a single document wins here for a reason specific to this schema: a memory record is
not self-contained. It owns citations and an event history, and JSONL would either split one record
across several lines — losing the containment the reader needs — or nest them anyway, at which
point the line-orientation buys nothing. `serde_json`'s default map is a `BTreeMap`, so keys
serialise in sorted order without a sorting pass; determinism is a property of the library here
rather than of a convention someone has to remember.

```
format · format_version · schema_version · repo_id · record_count · records[]
records[] : the stored columns, plus citations[] and events[]
ordering  : records by memory_id · citations by citation_id · events by event_id
```

**Three deliberate omissions, each of which is a test rather than a note.**

1. **No `exported_at`.** This is the one that matters, and it is forced: an export timestamp makes
   two exports of an unchanged database differ, so §7.4e's byte-identical requirement and an
   "exported at" field cannot both be satisfied. The brief's own *"exported-at metadata if useful"*
   is therefore **declined**, and determinism is what it is declined in favour of. A human who
   wants to know when they took a backup has the file's own mtime.
2. **No derived state.** `potentially_stale`, `conflicted`, `multiple_active`, the subject
   resolution verdict and `current_state_id` are computed at read time and must not appear.
   Exporting one would write a query's answer into a file that outlives the query — which is the
   *stored versus derived* split this row spent §3 establishing, undone at the last surface.
3. **No absolute path.** Repository-relative only, asserted by searching the output for the
   repository root string rather than by inspection.

**Import stays out of 14b-ii.** §6c scopes this sub-slice to "the command family, citations, export
and the acceptance rows", and §6b makes import conditional on a safety contract with thirteen parts
and eleven tests. A half-safe importer is how a human's notes get overwritten by a file, so it is
deferred **explicitly**: export is the supported backup mechanism, and import is its own slice if
and when the full contract is met.

---

## 7. Acceptance criteria

1. `assertion` / `observation` / `occurrence` / `assertion_state` byte-identical across every memory
   operation, asserted by hash. A probe writing memory into `assertion_state` fails a named test.
2. No confirm path reachable from `crates/nerve-server/src/mcp/`, asserted by a source scan with an
   anti-vacuity floor. A probe adding one fails by name.
3. T13 written, with the shared-shell limitation stated rather than implied.
4. **Four stored statuses and three derived views** (§3 as corrected); `potentially_stale`,
   `conflicted` and `multiple_active` derived at query time, never stored — a probe storing any of
   them fails by name.
4b. **A memory record survives the deletion of its subject entity.** Asserted directly: write a
   record about a file, delete the file, re-index so `prune_orphans` runs, and the record is still
   readable with subject resolution `missing`. A cascade or a blocked prune both fail this.
4c. **Two notes on one subject are reported `multiple_active`, not `conflicted`.** `conflicted`
   requires a shared `claim_key` or an explicit human conflict link — asserted by a negative test,
   because this is a claim the product would otherwise manufacture.
4d. **Supersession has one writable direction**; the inverse is derived, and a source scan proves no
   second column stores it. Status change and event append are one transaction, asserted by an
   interrupted-write test leaving neither applied.
4e. **Export is deterministic**: the same database exports byte-identically twice, and the output
   carries its schema version. If import ships, `--dry-run` writes nothing (hash-asserted) and a
   repository-mismatch import is refused.
5. `invalidated` and `superseded` distinguishable in every surface's output.
6. `memory_event` append-only: after supersede and invalidate, every prior event still readable.
   No delete verb exists — ~~asserted by the CLI-surface test that already fails if a forbidden
   command appears, which is how `nerve affected` and `nerve trace-tests` are held refused.~~

   > **Corrected 2026-08-11, during 14b-ii — that test does not exist.** The criterion named a
   > mechanism to reuse, and building on it found there is nothing to reuse: `affected` appears in
   > **no** test under `crates/nerve-cli/tests/` at all, and `trace-tests` only in doc comments and
   > in `scripts/final_acceptance.sh`. So the only thing holding those two commands refused is the
   > acceptance script's `refused` helper — **a script a developer may not run, not a gate**. The
   > same false claim was in `CONTINUATION.md` and is corrected there too.
   >
   > 14b-ii therefore built both halves for its own verb rather than inheriting the assumption:
   > `there_is_no_delete_verb` (a real `cargo test` assertion that rejects `delete`, `remove`,
   > `purge` and `forget`, checks the help listing, and confirms the record survives) **and** a
   > `refused_verb` row in the acceptance script, proved non-vacuous by pointing it at `memory
   > show`, which makes it FAIL.
   >
   > **Carried forward as a defect worth its own corrective slice:** `nerve affected` and
   > `nerve trace-tests` — two of this product's most load-bearing refusals — are still held only
   > by the acceptance script. They deserve the test this criterion assumed they already had.
7. HTTP surface read-only: `POST /api/memory` → 405, database hash unchanged.
8. Memory text confined to `repository_content` in MCP by the existing walk-the-whole-response
   property test.
9. The §6 acceptance rows replaced with behaviour in the landing commit.
10. Full gate.

---

## 8. Refutations of this plan's own first draft

1. **"Agents cannot self-confirm" was drafted as an identity check.** Nerve has no identity. §1 is the
   correction: the control is surface separation, it is testable, and its limit is written down.
2. **Memory was drafted as an `Assertion` with a `HUMAN_CONFIRMED` source type**, which is tidy and
   wrong: an assertion is a relation between two entities, `assertion_state` is defined as a pure
   function of observations, and a human sentence in that table is the silent-truth failure arrived at
   through the schema. §2.
3. **`potentially_stale` was drafted as a stored status.** Keeping it true needs a writer, and the
   writer is a query. Derived. §3.
4. **A writable `POST /api/memory` was drafted** so the UI could confirm. It relaxes the `query_only`
   guarantee Slice 4a proves on the bytes, for the one operation whose value is that it is
   deliberate. §5.
5. **`nerve memory delete` was drafted** beside invalidate. A delete verb is how "history preserved"
   stops being true. Refused, not deferred. §4.

### Corrections from implementing 14a (2026-08-10)

The orchestrator overruled one implementation decision, and the repository settled it:

**§7.4's "a probe storing any of them fails by name" needs a `CHECK`, not a Rust vocabulary.** 14a
first left `memory.status` unconstrained in SQL, on the reasonable ground that this schema closes
vocabularies in `nerve-core` and stores the text — which `V4`'s doc comment does state. But **v7
already contradicts that as a blanket rule**: `git_rename_hypothesis` enumerates `'exact_content'`
and `'similar_content'` in a `CHECK`, precisely because an invariant depended on the column's
*domain* rather than on its spelling. The same holds here: with the vocabulary closed only in Rust,
raw SQL can store `potentially_stale`, which then fails on the next read through
`MemoryStatus::FromStr` — loudly, but **after the row is on disk**, which is a repair job rather than
a refusal. `V9` therefore enumerates the four stored statuses. The cost is stated: a fifth *stored*
status needs a table rebuild, and that is the intended price, because a fifth stored status changes
what a memory record can be and should not be reachable by adding a Rust variant and finding the
database already accepted it.

Eight things the plan left under-specified, found by implementing it and recorded rather than
improvised:

1. **`scope` is never defined**, though the corrected §3 conflict key depends on it. Stored as an
   opaque non-empty string the store never interprets; **14b decides the values.**
2. **`multiple_active`'s grouping is unspecified.** Grouped by `(repository, subject, scope)` — the
   conflict key minus `claim_key` — so the two views are decided over nested groups.
3. **§4's citation sketch asks for a resolution verdict**; 14a ships the durable snapshots and
   defers the verdict, because nothing in 14a reads a citation's live target and a verdict with no
   reader is a code path no test exercises.
4. **`memory_id` is composite `(repo_id, memory_id)`**, following `repo_registry`, which buys
   composite foreign keys stating that a citation, an event or a supersession may not cross
   repositories.
5. **`memory_event.operation` has no vocabulary**; left an open string, since 14b's commands own the
   verbs.
6. **"Append-only" has no stated enforcement.** A workspace source scan was chosen over a
   `BEFORE DELETE … RAISE(ABORT)` trigger, because a trigger can be dropped by a later migration and
   would make any future whole-database purge require a v10.
7. **Subject resolution follows exactly one identity link, forwards only.** A subject moved twice
   across two indexing runs reports `missing` — a true statement about the snapshot. Bounded and
   pinned, with a control proving the second hop is reachable on its own.
8. **`scripts/final_acceptance.sh` still awards `nerve memory` an existence-only PASS** (§6's own
   defect). **14d must replace it**, and until then row 14's acceptance coverage is a claim about a
   command name.

### Refutations of the corrected draft, found 2026-08-08 before implementation

6. **`memory.subject_entity_id` was drafted as a foreign key into `entity`.** Entity rows are
   routinely deleted — `prune_orphans` (`prune.rs:376`, `:440`) and the required behaviour pinned by
   `deleting_a_file_removes_its_entities_assertions_and_observations` (`incremental.rs:290`) — so with
   `foreign_keys=ON` the delete is either refused (a human note blocks re-indexing a file) or
   cascades (a routine re-index silently destroys the note). Snapshots plus query-time resolution.
   §4.
7. **The surface table let MCP "propose".** If that means persisting a `proposed` row it is a
   database write, and MCP's byte-identical read-only test — passing today, and proved non-vacuous by
   a mutation probe in 12c-iii-b — would have to be deleted or weakened. MCP returns the command as
   text instead. §1.
8. **Six "statuses" were four stored plus two derived**, and §7.4 required all six as statuses, so
   the acceptance criterion contradicted the design. Split, and `multiple_active` added. §3.
9. **`conflicted` would have fired on every second note about a subject.** The content is free prose,
   so a resolver can never order it, so the rule manufactures a contradiction from two unrelated
   sentences — `ADR_DESCRIBES_COMPONENT`'s refusal in a new place. Gated behind an optional
   `claim_key` or an explicit human conflict link. §3.
10. **Supersession was drafted with two independently writable directions** (`supersedes` and
    `superseded_by`), which can disagree with nothing to notice — the "two writable copies of one
    fact" Row 13 §4.1 already rejected. One direction, inverse derived. §4.
11. **There was no export at all.** Memory is the only thing in the database a human authored and
    therefore the only thing re-indexing cannot rebuild. Export ships; import ships only if it can be
    made safe, and is otherwise deferred explicitly rather than half-built. §6b.
</content>
