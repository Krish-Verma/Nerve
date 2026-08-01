# Slice 5d — supersession, filesystem evidence, and the UI vocabulary

**Date:** 2026-08-01 · **Status:** Approved by the orchestrator, split into 5d-i / 5d-ii / 5d-iii
**Gates:** `docs/THREAT-MODEL.md` **T7** (amended here, see §4) · migration tests · precision gate

---

## 1. Objective and user value

Three things are unfinished after 5c, and they are unfinished in three different layers.

1. **Nerve mislabels filesystem facts as syntax-tree facts.** Indexing a documentation-only tree
   produces observations that say `AST_DIRECT`, attributed to `ts-js-structural`, in a repository
   containing no TypeScript. The user value of fixing it is the whole product thesis: if Nerve will
   say "a syntax tree contains this" when no syntax tree exists, its evidence labels are decoration.
2. **"Which ADR governs this, and has it been superseded?" is still unanswerable**, although the
   relation `SUPERSEDES` has been declared since 5a and the plan promised it in 5b.
3. **The interface cannot name what the backend now stores.** `kindGloss` falls back to "This build
   has no description for that entity kind" for `document` and `section` — the two kinds Slice 5a
   added.

## 2. Disagreements and pushback

### 2.1 Slice 5d as briefed is three surfaces — split, on recorded evidence

The brief asks for supersession, an evidence-vocabulary correction with a migration, and a UI
catch-up in one slice. `docs/CONTINUATION.md` records that **three implementation agents have now
been lost to exactly this shape**: one terminated mid-slice (4b), one stalled at the 600 s watchdog
(5b), one hit a hard session limit (5c). It also records the remedy that worked twice: the same work
split in two succeeded. That file states the rule as binding — "**Keep slices small.**"

Splitting is therefore not a scope reduction; it is the decomposition the evidence requires:

| | Scope | Layer |
|---|---|---|
| **5d-i** | `FILESYSTEM_OBSERVED` + `fs-structural`, ADR-0007, schema **v4** migration of stored rows | vocabulary + storage |
| **5d-ii** | `Document SUPERSEDES Document` — parsing, resolution, chains, cycles, fixtures, precision | extraction |
| **5d-iii** | UI vocabulary catch-up, asset re-embed, screenshot review | frontend |

Each is independently valuable and independently verifiable. All three land before Slice 6.

### 2.2 A new orthogonal `resolution_method` column — refused for this slice

The brief proposes promoting `resolution_method` to a first-class field beside `source_type` and
`directness`. It is refused **now** and reconsidered when a second consumer exists.

- The information already exists and is already cited: `docref.rs:86-88` records
  `document_link_resolved_file` / `document_link_resolved_symbol` in the observation's `details`,
  which `nerve why` prints verbatim.
- Adding a column means a migration touching **every** observation row for a field that, today,
  exactly one extractor would populate non-trivially. ADR-0003 forbids `confidence: float` because
  it invents structure that carries no evidence; a column that is `NULL` for 99% of rows is the
  same mistake in the other direction.
- The genuine defect — an evidence label that is factually false — is fixed by a **source type**,
  which is the field whose documented meaning is "how this evidence was obtained".

Recorded so it is not relitigated as an omission: **this is a deferral with a trigger**, not a
rejection. The trigger is Slice 10 (framework rules), the first slice that will produce several
distinct resolution methods under one source type.

### 2.3 `AST_DIRECT` is wrong for more than directories

The reported defect is directory containment. Inspecting the emission site shows the same wrongness
covers a wider set, and fixing only what was reported would leave the vocabulary incoherent:

`crates/nerve-index/src/pipeline.rs:1216-1307` builds, under `ts-js-structural` / `AST_DIRECT`:

- the `Repository` entity and its occurrence,
- every `Directory` entity and its occurrence,
- `Repository CONTAINS Directory`, `Directory CONTAINS Directory`,
- `Directory CONTAINS File` and `Repository CONTAINS File`,
- the `File` entity and its occurrence.

**Not one of these requires opening a file.** They are all derivable from a directory walk. They all
become `fs-structural` / `FILESYSTEM_OBSERVED`.

What stays `AST_DIRECT`, because a parse genuinely produced it: `File DEFINES Module`,
`Module DEFINES <symbol>`, and every symbol entity, occurrence and edge.

### 2.4 The vocabulary addition is append-only, and that is load-bearing

`EvidenceSourceType::ordinal()` is the index into `ALL`, and `mask_bit()` is `1 << ordinal`.
`observation.source_type_mask` is a **stored** `INTEGER` (`nerve-store/src/schema.rs:116`).
Appending `FilesystemObserved` at index 11 leaves every existing ordinal and every stored mask bit
correct. Inserting it anywhere else silently reinterprets every stored mask in every existing
database. **Append only. A test must pin the ordinals.**

There are no `CHECK` constraints on the vocabulary columns, so the addition itself forces no
migration. The **data** does — see §3.3.

## 3. Design — 5d-i

### 3.1 The new source type

```rust
/// The filesystem contains this. Derived from a directory walk, never from file content.
FilesystemObserved,          // ordinal 11, mask bit 1 << 11
```

Canonical database name `FILESYSTEM_OBSERVED`. `directness` stays `DIRECT`: the filesystem literally
states it, no resolution step occurred.

### 3.2 The new extractor

`fs-structural` version `1.0.0`, declaring `[FILESYSTEM_OBSERVED]` and nothing else.

Its defining property, which is what makes §4 sound: **`fs-structural` never reads file bytes.** It
consumes only the discovery walk's metadata — relative path, size, kind. A test asserts the
property by construction, not by inspection.

This is a fourth `extractor_run` per index. It is deliberately trivial: retraction per extractor
(`DELETE FROM observation WHERE extractor_id = 'fs-structural'`) is what makes the contribution
revocable, which is the same argument that gave `md-structural` its own run.

### 3.3 Migration — schema v4

**No DDL is required, and this is worth stating precisely because it is easy to over-engineer.**
`observation.evidence_source_type` and `observation.directness` are `TEXT` with **no `CHECK`
constraint and no lookup table** (`nerve-store/src/schema.rs:95-96`); the vocabulary is enforced in
Rust. The only integer encoding, `assertion_state.source_type_mask`, lives in a **derived** table
whose `ordinal`/`mask`/`name` SQL `CASE` expressions are **generated from `ALL` at runtime**
(`nerve-store/src/derive.rs:59-97`). So masks regenerate themselves; they are never patched.

What genuinely needs migrating is the **data**: rows already on disk that say `AST_DIRECT` /
`ts-js-structural` for filesystem structure. A re-index cannot be assumed, and worse, it would not
be sufficient — `structure_graph` re-derives directory containment every run, but repository→file
and directory→file rows are re-derived only for **changed** files, so an unchanged file keeps a
wrong row indefinitely. v4 is therefore a `Step::Rust` migration that:

1. Rewrites the qualifying `observation` rows' `evidence_source_type`, `extractor_id` and
   `extractor_version`.
2. Re-runs the **existing whole-table `assertion_state` derivation**, which recomputes
   `strongest_source_type` and `source_type_mask` from the corrected observations. Reuse it as the
   oracle rather than writing a second derivation — the Slice 3b precedent.

"Is filesystem structure" is decided **without guessing**: the migration selects on the assertion's
relation and its source endpoint kind — `CONTAINS` where the source entity kind is `repository` or
`directory` — which is exactly the set §2.3 enumerates and is a closed query over stored columns.

Migration tests required: v1→v4, v2→v4, v3→v4, and an end-to-end check against a **real v3 database
produced by the current binary** — the pattern Slice 3b established after the v2 data-destruction
bug, which only that end-to-end form would have caught. `nerve-store/tests/schema.rs:186` pins
`SCHEMA_VERSION == 3` and must move to 4.

### 3.4 Observation identity

Whether `extractor_id` participates in `observation_id` decides whether the migration produces
duplicate rows or updates in place. The implementer must check `ids::observation_id` first and
report the answer. If the id includes the extractor, the migration must delete-and-reinsert with
recomputed ids, and a test must assert no orphaned duplicate survives.

## 4. T7 is amended, and stated more precisely

Slice 5a's invariant is:

> No observation whose `file_path` is a document has any `evidence_source_type` other than
> `DOCUMENT_STATED`.

Today `Directory CONTAINS File` for a `.md` file is emitted by `md-structural` as `DOCUMENT_STATED`
precisely so that this stays a total function. After 5d-i it is emitted by `fs-structural`, because
"`docs/` contains `ROADMAP.md`" is a filesystem fact whoever asks, and having two extractors answer
that question differently by file extension is the incoherence this slice exists to remove.

**Amended T7:**

> Every observation whose `file_path` is a document carries `DOCUMENT_STATED`, except observations
> from `fs-structural`, which carries `FILESYSTEM_OBSERVED`. The allowed set is exactly those two,
> and it is checked exhaustively.

This does not weaken the control, and the reason is structural rather than a promise:

- T7 defends against **content** written by an attacker inside a document. `fs-structural` cannot
  carry document content anywhere, because it never reads any. That is testable and must be tested.
- The one attacker-controlled input it does touch — the path — already passes the Slice 5a
  `canonical_child` choke point that refuses the whole C0 range, and is the same exposure `.ts`
  files have had since Slice 1.
- The invariant stays **total**: a two-value allowlist keyed on extractor id, still exhaustively
  queryable, still mutation-verifiable.

The existing mutation probe must be re-pointed: declaring `AST_DIRECT` on a document path must still
fail the test and name every offender.

## 5. Design — 5d-ii: supersession from explicit evidence only

### 5.1 Recognised evidence, and nothing else

Mirroring `status_from_header_line` (`docs.rs:286`), which already parses Nerve's own
`**Status:** Accepted · **Date:** …` header form:

| Form | Meaning |
|---|---|
| `**Supersedes:** <target>` in the header block | this ADR supersedes `<target>` |
| `**Superseded by:** <target>` in the header block | `<target>` supersedes this ADR |
| First non-empty line of a `## Supersedes` section | as above |
| First non-empty line of a `## Superseded by` section | as above |

`Superseded by` normalises to the same edge with endpoints swapped. The relation is stored one way
only — `A SUPERSEDES B` means A replaces B — so a reverse lookup is a query, not a second edge.

**Nothing else is evidence.** Not similarity of subject, not adjacency of ADR numbers, not a
`Superseded` status with no target, not prose containing the word.

### 5.2 Target resolution — deterministic, three outcomes

A `<target>` is resolved by exactly two mechanisms, tried in order:

1. **A Markdown link.** Resolved through the existing 5c `docref` resolver — same path
   normalisation, same repository-root containment, same refusals. No new resolution code.
2. **A bare ADR identifier** matching `ADR-\d+`, resolved against the `adr_id` already parsed for
   every indexed ADR (`docs.rs:258`).

Outcomes, all recorded, none silent:

| Outcome | Result |
|---|---|
| Exactly one indexed ADR matches | `SUPERSEDES` edge, `DOCUMENT_STATED` / `RESOLVED` |
| No match | `Unresolved`, `document_supersedes_target_not_indexed` |
| More than one match | `Unresolved`, `document_supersedes_target_ambiguous` — **never guessed** |
| Target is the document itself | `Unresolved`, `document_supersedes_self` |
| Field present but empty or unparseable | `Unresolved`, `document_supersedes_unparsed` |
| Target resolves outside the repository root | reuse 5c's `document_link_refused` |

`UnresolvedCategory::DocumentSupersedes` already exists (`vocab.rs:211`); the reasons above are
values in the `details`, matching how 5c recorded `document_link_target_not_indexed`.

### 5.3 Chains and cycles

A chain is a path over `SUPERSEDES` and needs no new storage — `nerve path --relation SUPERSEDES`
already walks it.

A cycle **is not suppressed.** Each edge is individually backed by an explicit statement in a real
file; deleting one would hide evidence, and Nerve's rule is that unresolved and contradictory are
values rather than omissions. A cycle is instead **detected and reported** as a counter, and the
per-document contradiction promised in the Slice 5 plan §2.3 lands with it:

> An ADR that is the target of a `SUPERSEDES` edge while its own status still reads `Accepted`
> contradicts itself. Checkable by string comparison, no semantics required.

Both surface as counters now and in `nerve check` in Slice 7.

### 5.4 Fixtures and measurement

A new `fixtures/md-supersession/` corpus with ground truth in `expected.json`, mirroring
`fixtures/md-links/`. It must contain positive, negative, ambiguous, malformed, cyclic, self-,
missing-target and stale-target cases — the list in §5.2 plus a two-step chain and a three-ADR
cycle.

**Precision is measured and gated on the fixture corpus, and the result is a regression gate, not an
accuracy claim** — the language Slice 5c established and which must not drift.

Expected real-world result, stated in advance so it cannot be spun afterwards: **Nerve's own six
ADRs contain no supersession metadata at all.** The real-repository run should therefore produce
**zero** `SUPERSEDES` edges. That is a true negative and the honest outcome to report.

## 6. Design — 5d-iii: the UI vocabulary

Glosses for what the backend now stores and the interface cannot name: entity kinds `document` and
`section`; relation `SUPERSEDES`; source type `FILESYSTEM_OBSERVED`; every unresolved reason added
by 5c and 5d-ii; the ADR status vocabulary including `unparsed`.

Two rules, both from the brief and both binding:

- **Human-readable labels in the interface, raw enum names never shown as prose.**
- **The JSON API keeps the structured values unchanged.** The gloss is a presentation layer; an API
  consumer must still receive `FILESYSTEM_OBSERVED`. A test asserts the API response is unchanged.

Requires an `apps/nerve-web` rebuild and asset re-embed, and screenshot review at 380px and 1600px
as Slice 4b established.

## 7. Non-goals

No LLM. No fuzzy target matching. No inferring supersession from status alone. No new dependency.
No `resolution_method` column (§2.2). No change to `Relation::ALL` — `SUPERSEDES` is already there.
No re-ranking of evidence: ADR-0003's rule that ordering is a query-time policy stands, and
`FILESYSTEM_OBSERVED`'s position in `ALL` is structural, not a truth ranking.

## 8. Acceptance criteria

**5d-i**

1. `FILESYSTEM_OBSERVED` appended at ordinal 11; a test pins every ordinal and mask bit.
2. `fs-structural 1.0.0` emits all of §2.3; `ts-js-structural` emits none of it.
3. A test proves `fs-structural` never reads file content.
4. Indexing a documentation-only tree produces **zero** observations mentioning `ts-js-structural`.
5. Schema v4 with v1→v4, v2→v4, v3→v4 migrations and a real-v3-database end-to-end test.
6. Amended T7 exhaustive and mutation-verified.
7. Full-vs-incremental equivalence still byte-identical; golden dump updated deliberately.
8. Full gate green.

**5d-ii**

9. Every row of §5.2 covered by a fixture and asserted.
10. Chain, cycle and self-supersession fixtures; cycles detected, counted, and **not** suppressed.
11. Status-contradiction check implemented and counted.
12. Fixture precision measured and gated; real-repository run reported honestly (expected: zero).
13. Full gate green.

**5d-iii**

14. No `kindGloss` fallback for any kind, relation, source type, directness or unresolved reason
    the backend can emit — asserted by a test driven from the Rust vocabularies, so the two cannot
    drift.
15. API JSON unchanged, asserted.
16. Screenshots at 380px and 1600px reviewed; 0 CSP violations.
17. Full gate green, assets re-embedded, release build.
