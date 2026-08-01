# Slice 5 — Markdown and ADR evidence

**Date:** 2026-07-31 · **Status:** Approved, split into 5a and 5b
**Gate:** `docs/THREAT-MODEL.md` **T7** (prompt injection through documents)

---

## 1. Objective and user value

A repository's prose is evidence about itself, and today Nerve cannot see any of it. A README, an
ADR and an architecture note say things about the code; some of what they say is *checkable*, and
almost all of it goes stale silently. Slice 5 ingests that prose into the same evidence model as
source, under one rule: **a document is a witness, never an authority.**

The user value is concrete:

- "Which documents talk about this file?" — answered with exact citations.
- "Which ADR governs this decision, and has it been superseded?" — answered from explicit links.
- "Which document links are now broken?" — answered because unresolved is a value, not an omission.

## 2. Disagreements and pushback

### 2.1 The proposed relation names do not fit the accepted schema — rejected

The brief proposes `DOCUMENT_CONTAINS_SECTION`, `SECTION_REFERENCES_FILE`,
`SECTION_REFERENCES_SYMBOL`, `ADR_DESCRIBES_COMPONENT`, `ADR_SUPERSEDES_ADR`,
`ADR_CONTRADICTS_CURRENT_SOURCE` — and in the same breath says "use only relationship names that
fit the accepted schema." Those two instructions conflict, and the schema wins.

Nerve's relation vocabulary is **endpoint-kind-agnostic by construction**
(`crates/nerve-core/src/vocab.rs:110`). We already have `Directory CONTAINS File`,
`File CONTAINS Module` and `Module DEFINES Function` — the endpoint kinds are carried by
`entity.kind`, never duplicated into the relation name. Encoding kinds into relation names would:

- explode the vocabulary combinatorially (`MODULE_CONTAINS_FUNCTION`, `CLASS_DEFINES_METHOD`, …),
- duplicate information the schema already stores, creating a second thing that can disagree,
- and break every existing relation filter — `nerve path --relation`, `nerve why`, the API's
  `relation` parameter and the UI's relation chips all take values from `Relation::ALL`.

**Decision:** reuse `CONTAINS` for document→section and section→section, reuse `REFERENCES` for
section→code, and add exactly **one** new relation, `SUPERSEDES`, which has no existing analogue.

### 2.2 `ADR_DESCRIBES_COMPONENT` — refused

There is no deterministic rule that separates "describes" from "mentions" in Markdown. A section
containing a link to `pipeline.rs` may be describing it, criticising it, or citing it as a
counter-example. Emitting `DESCRIBES` would be a semantic claim dressed as a structural one —
precisely the fuzzy-naming failure the brief forbids two paragraphs later. `REFERENCES` states
exactly what we can prove: this section points at that entity.

### 2.3 `ADR_CONTRADICTS_CURRENT_SOURCE` — mostly refused, one deterministic part kept

Doc-versus-source contradiction requires extracting a *claim* from prose ("`foo` calls `bar`"),
which is not deterministically possible without an LLM. The brief forbids requiring an LLM. So the
general form is refused.

Two deterministic contradiction signals **are** kept, because they need no semantics:

1. A document link whose destination does not resolve to anything indexed — a broken reference,
   recorded as `Unresolved` with a reason, surfaced in gaps. (5b)
2. An ADR that is the target of a `SUPERSEDES` edge while its own status still reads `Accepted` —
   the document contradicts itself, checkable by string comparison. (5b)

Also note: assertion-level disagreement already has a home. `AssertionStatus::Contradicted` is
derived when observations disagree (`nerve-store/src/derive.rs`). A new relation for contradiction
would put the same concept in two places.

### 2.4 "ADR status" is not an entity — it is a section

The brief lists "ADR status" as an entity to create. An entity in this model is a thing with
**occurrences** — physical appearances at byte spans. A status *value* ("Accepted") is a property,
not a thing; `entity(kind=adr_status, name="Accepted")` would be a value masquerading as a node and
would merge every Accepted ADR in the repository onto one entity.

**Decision:** status is recorded in the document entity's `meta` as a closed-vocabulary value, and
the exact `file:line` it was read from is recorded in the observation's `details`, so the citation
requirement is met without inventing a node. Where an ADR uses a `## Status` section, that section
exists as a `Section` entity anyway and carries the span.

### 2.5 Markdown parser: no new dependency — measured

`tree-sitter-md 0.5.3` was the obvious candidate and was checked before rejecting it. Its manifest
requires **`tree-sitter = "0.26"`**; this workspace is on `0.25` (`Cargo.toml`), with
`tree-sitter-typescript 0.23` and `tree-sitter-javascript 0.25` already pinned against it. Adopting
it means either

- bumping the whole workspace to tree-sitter 0.26, which re-parses every TS/JS fixture through a
  new runtime — a large regression risk on frozen, precision-gated extraction code, incurred *for a
  documentation slice*; or
- carrying two major versions of tree-sitter and two copies of its C runtime, which is exactly the
  dependency-cost argument that rejected Tokio+axum in Slice 4a.

A third option, `tree-sitter-md-025`, is an unofficial third-party republish — a supply-chain
surface we will not take on for convenience.

**Decision:** a hand-written, deliberately restricted Markdown **block scanner** in `nerve-index`.
This is not "reimplementing CommonMark". We do not want CommonMark fidelity — we want a conservative
structural subset with everything else reported as unsupported. The subset (ATX and setext headings,
fenced and indented code regions, inline code spans, front-matter delimiters, link destinations) is
a scanner, not a parser, it is exhaustively unit-testable, it adds zero dependencies, and it cannot
execute anything. Constructs outside the subset are **counted and reported**, never guessed at.

### 2.6 Slice 5 is too large for one unit — split

`docs/CONTINUATION.md` records that a slice bundling two surfaces stalled an agent at the 600 s
watchdog and that another was terminated mid-slice, and that the same work split in two succeeded.
Slice 5 as briefed contains an ingestion path, a resolver with a precision gate, an invalidation
extension and a UI surface. It is split on the same seam as 2a/2b and 4a/4b:

| | Scope |
|---|---|
| **5a** | Discovery of Markdown · `Document` / `Section` entities · `CONTAINS` structure · ADR recognition and status · resource bounds · **T7 controls** · incremental + full/incremental equivalence for documents |
| **5b** | Link resolution: `Section REFERENCES` file/symbol, `Document SUPERSEDES Document` · unresolved reasons · **measured precision gate** · invalidation across document→code edges · CLI and UI surfacing |

5a is independently valuable (structure, ADR index, citations) and independently verifiable.

## 3. Design

### 3.1 Entity kinds

| Kind | Prefix | Identity tuple |
|---|---|---|
| `Document` | `doc` | `("document", project_id, rel_path)` — mirrors `Module`, 1:1 with a file |
| `Section` | `sect` | `("section", project_id, rel_path, heading_path, sibling_ordinal)` |

`heading_path` is the `>`-joined chain of ancestor heading texts. **Heading text is
attacker-controlled**, so before it enters an identity tuple every byte below `0x20` is stripped —
including `0x1f`, the canonical tuple separator. Without that, a heading containing a literal unit
separator could forge a different tuple and collide two sections deliberately. A test must assert
the forgery attempt fails, mirroring `unresolved_modules_and_values_with_the_same_name_are_distinct`.

`sibling_ordinal` disambiguates two sections with identical heading text under the same parent, and
is scoped to the parent so inserting a section elsewhere in the document does not churn ids.

### 3.2 Relation

One addition: `Relation::Supersedes` → `"SUPERSEDES"`. Declared in 5a, **emitted in 5b**, following
the precedent set for `CALLS`/`REFERENCES` in Slice 1.

### 3.3 Extractor

`md-structural` version `1.0.0`, declaring **`[DOCUMENT_STATED]` and nothing else**.

Everything derived from a document carries `DOCUMENT_STATED` — including `File CONTAINS Document`,
which is arguably a filesystem fact. That is deliberate. It makes the T7 separation a **total
function of the source file** rather than a per-claim judgement, which yields a single invariant
that can be checked exhaustively:

> No observation whose `file_path` is a document has any `evidence_source_type` other than
> `DOCUMENT_STATED`.

`directness` is `Direct` for structure the file literally contains.

### 3.4 Emissions in 5a

- `File CONTAINS Document`
- `Document CONTAINS Section` (top-level headings)
- `Section CONTAINS Section` (nesting by heading level)
- Document `meta`: `{"adr": bool, "adr_id": …, "status": …, "unsupported": {…}}`, canonical JSON.

No link, no reference, no cross-file edge in 5a.

### 3.5 ADR recognition — deterministic, closed

A document is an ADR when **either**:

- its file name matches `ADR-<digits>` case-insensitively (Nerve's own `ADR-0006-…md`), **or**
- it sits directly in a directory named `decisions`, `adr` or `adrs` (case-insensitive).

Status is read from the first match of either form, both of which occur in this repository:

- a line `**Status:** <word>` in the document's header block (before the first `##`), or
- the first non-empty line of a `## Status` section.

The status vocabulary is **closed**: `Proposed`, `Accepted`, `Rejected`, `Deprecated`, `Superseded`.
An unrecognised value is recorded as `unparsed` with the raw text preserved in `details` — never
coerced, never guessed.

### 3.6 Resource bounds (T7)

Every bound refuses and **counts**; nothing is silently truncated.

| Bound | Value | Rationale |
|---|---|---|
| Document bytes | existing `index.max_file_bytes` (2 MiB) | reuse, do not invent a second limit |
| Headings per document | 10 000 | a 2 MiB file of `#` lines is ~1 M headings |
| Heading depth | 6 | CommonMark's own limit; `#######` is paragraph text, so nesting is bounded by construction and recursion is impossible |
| Front-matter lines | 1 000 | |

### 3.7 Incremental behaviour

A document has no imports, so in 5a a changed document invalidates **only itself**. The full-vs-
incremental **byte-identical equivalence property** from Slice 3 must hold over an edit sequence
that includes documents — added, modified, removed, moved, and a heading renamed. Extending the
invalidation closure across document→code edges belongs to 5b, where those edges first exist.

## 4. Acceptance criteria — 5a

1. `.md` and `.markdown` files are discovered, respecting `.gitignore`, `.nerveignore`, the secret
   deny-list, symlink refusal and the size ceiling. Existing TS/JS discovery counts unchanged.
2. `Document` and `Section` entities with exact spans and content hashes; `nerve search` finds them.
3. Section nesting matches heading levels, including setext headings and a document whose first
   heading is not level 1.
4. ADR recognition and status on Nerve's own `docs/decisions/`, verified against the real files.
5. **T7:** an exhaustive test that no document-sourced observation carries a non-`DOCUMENT_STATED`
   source type; a hostile-document fixture containing script tags, event handlers, inline HTML,
   prompt-injection text, traversal-shaped link text, control characters and a unit separator in a
   heading — all stored as inert data, none executed, none able to forge an identity.
6. Every resource bound fires on a fixture and is reported in `nerve status`-visible counters.
7. Full-vs-incremental equivalence holds over a seeded edit sequence including documents.
8. Malformed Markdown never panics: unterminated fences, unterminated front matter, mixed line
   endings, a lone `#`, 10 000 nested-looking lines, invalid UTF-8 handled at the read boundary.
9. **No new dependency.** `third_party/LICENSES.md` unchanged in count.
10. Full gate green: fmt, clippy `-D warnings`, `cargo test --workspace`, release build.

## 5. Non-goals

No LLM. No summarisation. No fuzzy name matching. No link resolution (5b). No document rendering.
No CommonMark conformance. No schema migration unless the entity kinds force one — they should not,
since `entity.kind` carries no `CHECK` constraint (`nerve-store/src/schema.rs:46`).
</content>
</invoke>
