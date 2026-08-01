# ADR-0007 — Filesystem structure is its own evidence source type

**Status:** Accepted · **Date:** 2026-08-01 · **Slice:** 5d-i

---

## Context

Since Slice 1, Nerve has stamped the repository skeleton — the repository entity, every directory,
every file, and the `CONTAINS` edges between them — with `EvidenceSourceType::AstDirect`, attributed
to the extractor `ts-js-structural`.

`AST_DIRECT` is defined in ADR-0003 as *"the syntax tree literally contains this relationship."*

There is no syntax tree behind a directory. The defect became visible in Slice 5a, when indexing a
documentation-only tree produced observations like:

```
AST_DIRECT DIRECT ts-js-structural docs           CONTAINS repository '.' → directory 'docs'
AST_DIRECT DIRECT ts-js-structural docs/decisions CONTAINS directory docs → directory 'decisions'
```

in a repository containing no TypeScript at all. Slice 5a made this visible; it did not cause it.

This is the second instance of the same defect class. Slice 2a already corrected it once, when
resolved imports and re-export edges were found mislabelled `AST_DIRECT` and were moved to
`AST_RESOLVED`.

The two statements are separately false, and both matter:

1. **The source type is false.** No parse produced the fact. It came from a directory walk.
2. **The extractor attribution is false.** `ts-js-structural` is the TypeScript/JavaScript
   extractor. Attributing a fact about `docs/` to it means that retracting the TypeScript
   extractor's contribution — the whole point of per-extractor `extractor_run` rows — would also
   retract the repository's directory tree.

If Nerve will assert "a syntax tree contains this" where no syntax tree exists, then its evidence
labels are decoration rather than evidence, and the product thesis fails at its root. That is why
this is a corrective slice of its own rather than a review amendment.

## Decision

### 1. A new source type

`EvidenceSourceType::FilesystemObserved`, canonical name `FILESYSTEM_OBSERVED`:

> The filesystem contains this. Derived from a directory walk, never from file content.

`directness` remains `DIRECT`: the filesystem literally states it; no resolution step occurred.

It is **appended** to `EvidenceSourceType::ALL`, at ordinal 11, mask bit 2048. This is load-bearing.
`ordinal()` is the index into `ALL` and `mask_bit()` is `1 << ordinal`, and `assertion_state`
carries a stored `source_type_mask`. Inserting the variant anywhere but the end would silently
reinterpret every mask in every existing database. A test now pins **every** variant's ordinal and
mask bit so that a future insertion fails loudly rather than corrupting data quietly.

As ADR-0003 already states, declaration order is **not** a truth ranking. Ranking is supplied by an
evidence policy at query time. `FILESYSTEM_OBSERVED` being last means nothing about its strength —
it is in fact among the most reliable evidence Nerve has, because it involves no inference at all.

### 2. A new extractor

`fs-structural` version `1.0.0`, declaring `[FILESYSTEM_OBSERVED]` and nothing else.

Its defining property is that **it never reads file bytes.** It consumes only the discovery walk's
metadata: relative path, size, kind. This is enforced structurally rather than by convention, and it
is what makes the amended T7 in §3 sound.

It owns exactly the set of facts derivable from a directory walk:

| Owned by `fs-structural` | Stays elsewhere |
|---|---|
| `Repository` entity and occurrence | `File DEFINES Module` — `ts-js-structural`, a parse produced it |
| `Directory` entities and occurrences | `Module DEFINES <symbol>` and all symbol edges — `ts-js-structural` |
| `File` entities and occurrences | `File CONTAINS Document` — `md-structural` |
| `Repository`/`Directory` `CONTAINS` `Directory` | `Document CONTAINS Section`, `Section CONTAINS Section` — `md-structural` |
| `Repository`/`Directory` `CONTAINS` `File` | `Section REFERENCES <file/symbol>` — `md-structural` |

The last row includes Markdown files. Before this ADR, `Directory CONTAINS File` was emitted by
`ts-js-structural` for a `.ts` file and by `md-structural` for a `.md` file. Two extractors
answering the same structural question differently by file extension is precisely the incoherence
this ADR removes. "`docs/` contains `ROADMAP.md`" is a filesystem fact whoever asks.

### 3. T7 is amended, and stated more precisely

Slice 5a established:

> No observation whose `file_path` is a document has any `evidence_source_type` other than
> `DOCUMENT_STATED`.

Since `Directory CONTAINS File` for a `.md` file now belongs to `fs-structural`, the invariant
becomes a two-value allowlist keyed on extractor id:

> Every observation whose `file_path` is a document carries `DOCUMENT_STATED`, except observations
> from `fs-structural`, which carry `FILESYSTEM_OBSERVED`. The allowed set is exactly those two, and
> it is checked exhaustively.

**This does not weaken the control**, and the reason is structural rather than a promise:

- T7 defends against **content** an attacker writes inside a document. `fs-structural` cannot carry
  document content anywhere, because it never reads any. That is a property of the code, and it is
  tested.
- The one attacker-controlled input it does touch is the path, which already passes the Slice 5a
  `canonical_child` choke point refusing the whole C0 range — the fix that closed the identity
  forgery where a literal `0x1f` in a path could merge two entities. This is the same exposure
  `.ts` files have carried since Slice 1, not a new one.
- The invariant remains **total and exhaustively queryable**, not a spot check, and remains
  mutation-verifiable: stamping `AST_DIRECT` on a document path must still fail the test and name
  every offender.

### 4. Existing databases are migrated, not left to re-index

Schema **v4**. No DDL is required — `observation.evidence_source_type` is `TEXT` with no `CHECK`
constraint, and the only integer encoding lives in the derived `assertion_state` table, whose SQL
`CASE` expressions are generated from `ALL` at runtime and therefore regenerate themselves.

The **data** does need correcting, and a re-index is not sufficient to do it. `structure_graph`
re-derives directory containment on every run, but `Repository`/`Directory CONTAINS File` rows are
re-derived only for **changed** files — so an untouched file would keep a wrong row indefinitely.

v4 therefore rewrites the qualifying observation rows and then re-runs the existing whole-table
`assertion_state` derivation, reusing it as the oracle rather than hand-patching masks — the pattern
Slice 3b established. Qualifying rows are selected by a closed query over stored columns
(`CONTAINS` assertions whose source entity kind is `repository` or `directory`), never by guessing
at paths.

## Alternatives considered

**Leave it, and document the label as approximate.** Rejected. An evidence label that is knowably
false is worse than no label, because the product's entire claim is that its labels can be trusted
without re-deriving them. This is also the defect class Slice 2a already corrected once; leaving the
second instance would establish that the correction was a one-off rather than a rule.

**Reuse `AST_DIRECT` and only fix the extractor attribution.** Rejected. It fixes the smaller of the
two false statements and leaves the larger one.

**Add an orthogonal `resolution_method` column instead.** Rejected for this slice, with a trigger
for reconsideration. The defect is that a field whose documented meaning is "how this evidence was
obtained" holds a wrong value; the fix belongs in that field. Adding a column would migrate every
observation row for a field exactly one extractor would populate non-trivially — and ADR-0003
rejects `confidence: float` for inventing structure that carries no evidence, which a
mostly-`NULL` column repeats in the other direction. Resolution method is already recorded and
already cited, in the observation's `details` (`document_link_resolved_file`,
`document_link_resolved_symbol`), which `nerve why` prints verbatim. Reconsider at Slice 10
(framework rules), the first slice producing several distinct resolution methods under one source
type.

**Keep `md-structural` owning `Directory CONTAINS File` for Markdown, to leave T7 untouched.**
Rejected. It preserves a simpler invariant by preserving an incoherence — the same structural
question answered by different extractors according to file extension. The amended invariant is
still total, still exhaustive, and still mutation-verified, and §3 gives a structural reason rather
than an assurance.

## Consequences

- One more `extractor_run` row per index — four rather than three. Per-extractor retraction now
  works correctly: dropping `ts-js-structural` no longer takes the directory tree with it.
- `fixtures/ts-basic/golden.json` moves deliberately, and the diff is reviewed rather than accepted
  wholesale.
- Any future extractor deriving facts from filesystem structure — a build-manifest reader, a
  workspace-layout rule — has an honest source type waiting for it instead of a reason to reach for
  `AST_DIRECT`.
- `EvidenceSourceType::ALL` has 12 members. Anything matching exhaustively over it is forced by the
  compiler to consider the new variant, which is the intended behaviour.
