# Slice 5d — supersession, filesystem evidence, and the UI vocabulary

**Date:** 2026-08-01 · **Commits:** `c39e783` (5d-i), `f98aa2a` (5d-ii)
**Plan:** `docs/plans/slice-05d-supersession-and-filesystem-evidence.md`
**ADR:** `docs/decisions/ADR-0007-filesystem-evidence.md`

---

## 0. Decomposition, and why

Slice 5d was briefed as one slice covering three layers: an evidence-vocabulary correction with a
migration, a new extraction path, and a frontend catch-up. It was split into **5d-i / 5d-ii /
5d-iii** before any code was written.

That was not a scope reduction. `docs/CONTINUATION.md` records that three implementation agents had
already been lost to exactly this shape — one terminated mid-slice, one stalled at the 600 s
watchdog, one hit a hard session limit — and that the remedy which worked twice was splitting on a
layer seam. The decomposition was correct: **5d-i's agent hit a hard session limit anyway**, at the
point it had finished implementing and was part-way through verification. Because the slice was
small, its work was complete enough to inspect, verify independently and commit, rather than being
a half-built change across three layers.

The count of agents lost to oversized work is now **five**. The rule stands.

---

## 1. Slice 5d-i — filesystem structure is not a syntax tree

### The defect

Since Slice 1, Nerve stamped the repository skeleton — the repository entity, every directory,
every file, and the `CONTAINS` edges between them — with `AST_DIRECT`, attributed to
`ts-js-structural`. ADR-0003 defines `AST_DIRECT` as *"the syntax tree literally contains this
relationship."* Indexing a documentation-only tree produced four observations claiming a syntax
tree, in a repository with no TypeScript in it.

Two separately false statements: the source type, and the extractor attribution. The second matters
more than it looks — attributing a fact about `docs/` to the TypeScript extractor means retracting
that extractor's contribution, which is the entire purpose of per-extractor `extractor_run` rows,
would also retract the repository's directory tree.

This is the **second** instance of the class. Slice 2a corrected the first, when resolved imports
were found mislabelled `AST_DIRECT`.

### What was built

- **`EvidenceSourceType::FilesystemObserved`** (`FILESYSTEM_OBSERVED`), appended at ordinal 11.
  Appending is load-bearing rather than tidy: `ordinal()` is a position in `ALL`, `mask_bit()` is
  `1 << ordinal`, and `assertion_state.source_type_mask` is a **stored** integer. Inserting the
  variant anywhere else would silently reinterpret every mask in every database on disk. All twelve
  ordinals and mask bits are now pinned one by one, so an insertion fails where it is written.
- **`fs-structural 1.0.0`**, declaring `FILESYSTEM_OBSERVED` and nothing else, owning everything a
  directory walk knows.
- **`Directory CONTAINS <file>` moved to it for Markdown too.** Before this, the identical
  filesystem fact was a syntax tree's claim for `math.ts` and a document's claim for `ROADMAP.md`.
  Two extractors answering one structural question by file extension was the incoherence; the fix
  removes it rather than preserving it.
- **Schema v4.** No DDL — the vocabulary is closed in Rust, not in SQL, and the only integer
  encoding is regenerated from `ALL` at runtime. The data needed correcting, and a re-index would
  not have sufficed: directory containment is re-derived every run, but directory→file rows are
  re-emitted only for files a run re-extracts, so an untouched file would keep a wrong row forever.

### Content-independence is structural, not a promise

`fs-structural` is handed an `FsEntry` projection — `rel_path`, `kind`, `size_bytes`,
`content_hash` — which has **no field that can hold file text**. The graph builder's signature is
the proof: it has nothing to leak even if it tried. Two tests back it: one builds the whole skeleton
from hand-written `FsEntry` values with no file on disk, and one indexes a repository whose files
carry a unique marker string and asserts the marker appears in no `fs-structural` row.

### T7 amended, not weakened

The Slice 5a invariant was a single allowed value on document paths. It becomes exactly two,
`{DOCUMENT_STATED, FILESYSTEM_OBSERVED}`, keyed on **extractor id** — so an `fs-structural` row
saying `DOCUMENT_STATED` is as much a violation as an `md-structural` row saying `AST_DIRECT`.

The widening is not a loosening, for a structural reason rather than an assurance: T7 defends
against *content* an attacker wrote inside a document, and `fs-structural` reads none. The one
attacker-influenced input it touches is the path, which already passes the Slice 5a
`canonical_child` guard that refuses the whole C0 range. The invariant stays total, exhaustively
queryable, and mutation-verified.

### Result, measured

A documentation-only tree, indexed with the release binary:

```
fs-structural | FILESYSTEM_OBSERVED | 39
md-structural | DOCUMENT_STATED     | 514
observations mentioning ts-js-structural: 0        (was 4 mislabelled AST_DIRECT)
```

The golden dump moved **46 lines**: `schema_version`, evidence labels and extractor attribution
only. Every entity count and relation count unchanged — checked by diffing everything that was
*not* an evidence label and finding nothing.

---

## 2. Slice 5d-ii — supersession from explicit evidence, and nothing else

### What counts as evidence

Four forms, and no others: `**Supersedes:**` and `**Superseded by:**` in a document's header block,
and the first non-empty line of a `## Supersedes` or `## Superseded by` section. `Superseded by`
normalises to the same edge with endpoints swapped, so the relation is stored one way only and a
reverse lookup is a query rather than a second edge.

Targets resolve two ways only: a Markdown link through the **existing** Slice 5c resolver — same
normalisation, same root containment, same refusals, no new resolution code — or a bare
`ADR-<digits>` against identifiers already parsed. Everything else is a recorded value:
`document_supersedes_self`, `..._target_not_indexed`, `..._target_ambiguous`, `..._unparsed`,
`document_link_refused`, and external URLs counted but never fetched and never entity-ised.

### Cycles are reported, never suppressed

Each edge in a cycle is individually backed by an explicit statement in a real file. Deleting one
to break the cycle would hide evidence, which is the opposite of the rule that unresolved and
contradictory are values rather than omissions. Cycles are detected, counted and reported. The
status contradiction promised in the Slice 5 plan lands with them: a document that is superseded
while still claiming `Accepted`, by string comparison, needing no semantics.

### The measurement, and what it is not

The fixture corpus and its ground truth were authored by the orchestrator **before the resolver
existed**, deliberately, so the number is not self-consistency. 26 files covering both directions of
one edge collapsing to a single assertion, a two-hop chain, a three-document cycle, a non-ADR
document, and the negatives that matter most — prose containing the word, the field's text inside a
code span, and the field inside a fenced code block.

```
supersession fields 16 · resolved 10 · external 1 · unresolved 5
edges 9 resolved (from 15 observations) · TP 9 · FP 0 · FN 0
precision 100.0% · recall 100.0% · unresolved rate 35.7%
cycles 1 over 3 documents · contradictions 5
```

**This is a regression gate, not an accuracy claim.** It is one hand-built corpus. It says nothing
about repositories nobody wrote for Nerve.

### The honest real-world result

Predicted in the plan before implementation, and confirmed: **Nerve's own six ADRs state no
supersession metadata and produce zero edges.**

Two things the plan did not predict, both reported rather than smoothed over. `docs/plans/` contains
two real `**Supersedes:**` header fields; neither names a resolvable target (one is a code span plus
prose, the other reads `ROADMAP row 2 (scope split)`), so both are recorded
`document_supersedes_unparsed` and neither became an edge. Nothing was loosened to make them
resolve.

### One deviation — the specification was corrected, not the corpus

The fixture spec's summary table implied a single status contradiction. The rule as written in the
plan — any document that is a `SUPERSEDES` target while still reading `Accepted` — produces **five**
on that corpus, because the three cycle documents and the chain's middle node are all `Accepted`
targets. The implementer applied the rule as specified, reported the discrepancy, and declared all
five in the ground truth **rather than editing the corpus to match the code**. That is the correct
direction, and it is recorded here because the opposite would have been invisible.

---

## 3. Verification

Every command below was run by the **orchestrator**, not merely reported by an implementer.

| Command | 5d-i (`c39e783`) | 5d-ii (`f98aa2a`) |
|---|---|---|
| `cargo fmt --all -- --check` | clean | clean |
| `cargo clippy --workspace --all-targets -- -D warnings` | 0 warnings | 0 warnings |
| `cargo test --workspace` | **577** passed, 0 failed, 2 ignored | **596** passed, 0 failed, 2 ignored |
| `cargo build --release` | Finished | Finished |

Baseline at `f344310` was 564 passed.

### A flake, reported rather than buried

The first independent run of the 5d-ii gate returned **486 passed, 1 failed**. Four consecutive
runs afterwards, with nothing changed, returned 596/0/2. The failing run overlapped a subagent's
concurrent `cargo` invocation sharing the same `target/` directory, which is the most likely cause;
it was **not reproduced and no defect was found**. It is recorded in `docs/CONTINUATION.md` as an
environment note with the mitigation — do not run the gate while a subagent is running cargo — and
it is the reason the orchestrator now reruns every gate rather than accepting an implementer's
tally. The 5d-ii agent had reported 596/0/2; the first independent rerun disagreed.

### Adversarial verification

- **T7 mutation probe ships as a test**: stamping `AST_DIRECT` on a document path fails the
  invariant and names every offender.
- **Ordinal pinning**: all twelve `EvidenceSourceType` ordinals and mask bits asserted individually,
  so inserting a variant mid-array fails at the point it is written rather than corrupting stored
  masks silently.
- **Content-independence**: proved by construction and by a marker-string test.
- **Ground truth authored before the implementation**, so the precision number is not circular.
- **Full-vs-incremental byte-identical equivalence** still holds, now over a scripted supersession
  edit sequence: field added → target deleted → restored → made ambiguous by a second document
  carrying the identifier → unambiguous again, with the citing document's bytes unchanged
  throughout. The test also asserts `files_resolution_changed >= 1` at the two steps that depend
  only on the new invalidation rule, so the mechanism is proved rather than incidental.

### Safety and clean-room

No dependency added — `Cargo.toml`, `Cargo.lock` and `third_party/LICENSES.md` untouched across both
commits. No network client. No subprocess. No competitor source consulted, referenced or vendored;
the supersession design derives from this repository's own ADR header format and from CommonMark
structure. Nothing under `.nerve/` was committed.

---

## 4. Known limitations introduced or confirmed

- A **reference-style** link (`[a][ref]`) used as a supersession target resolves as `unparsed`,
  because the scanner records that link's span at the `[ref]:` definition line rather than inside
  the field. Not covered by a fixture.
- For `**Superseded by:** <unresolvable>` the `Unresolved` entity becomes the assertion's **source**,
  since that is the endpoint the document named it for. No fixture covers it.
- Indexing the whole repository makes the `fixtures/md-supersession` ADR identifiers ambiguous
  against Nerve's own `docs/decisions/`. The refusal is correct — the identifier namespace is
  repository-wide — but the fixture corpus is visible to a self-index.
- **Coverage remains deliberately narrow.** Nerve's own 45 documents yield 5 Markdown link sites and
  0 resolvable supersession statements. Precision is high because the rules are strict, not because
  document understanding is broad. This must not be restated as the latter.

## 5. Status

**5d-i and 5d-ii complete and committed. 5d-iii — the UI vocabulary catch-up — is the next unit**,
and Slice 5d is not complete until it lands. Its scope, gaps and acceptance criteria are in the plan
at §6 and criteria 14–17; the substantive item beyond missing glosses is that `directnessClass`
currently has a `default` arm rendering an unknown directness as "inferred", which is a false claim
about evidence rather than a missing label.
