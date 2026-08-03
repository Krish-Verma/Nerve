# Slice 8b-i — selector resolution by entity kind

**Status:** planned 2026-08-02. **Row 8b split into 8b-i and 8b-ii before implementation.**
Follows Slice 8a (`cbce2c0`).

---

## Why the row is split

Row 8b as written bundles a **cross-surface identity change** (which entity a string names) with
**four new MCP tools**. Those are not the same work. The identity change touches `nerve-store`,
the CLI, `/api/why`, `/api/impact` and every MCP tool at once; the tools sit on top of it. Shipping
them together means the tool tests would be written against a selector contract that is changing
underneath them.

This is the seam 4a/4b and 8a/8b already used: establish the contract, then build on it.

- **8b-i — selector resolution.** This document.
- **8b-ii — the rest of the MCP tool surface** (`search`, `path`, `impact`, `gaps`), on a selector
  layer that is already correct and tested.

The two are *not* combined, because 8b-ii's acceptance tests depend on 8b-i's contract being
settled. They *are* both inside row 8b because neither delivers the row alone.

---

## The defect, measured before the plan was written

Indexed `fixtures/md-docs` (68 entities) with the Slice 8a release binary and asked it questions.

**1. A document cannot be named by its path.**

```
$ nerve why docs/architecture.md
nerve why: "docs/architecture.md" (from) matches no indexed entity
```

`resolve_selector` stage 2 is `kind = 'module' AND scope_path = ?`. A `Document` has
`scope_path = 'docs/architecture.md'` and `kind = 'document'`, so it never matches. The document
*is* reachable — as the bare stem `architecture` — but only by knowing that Nerve stores the file
stem as the name, and only while no other entity shares that stem.

**2. A file cannot be named by its path either, and at a source path the `File` entity is silently
shadowed.**

`File` has `name = 'architecture.md'`, `scope_path = 'docs'` — so `docs/architecture.md` matches
nothing, and `architecture.md` matches the file by accident of its last segment. At `src/app.ts`
both a `File` and a `Module` exist; stage 2 asks only about modules, so the module answers and the
file is unreachable by path. Nothing tells the caller a choice was made.

In this 68-entity fixture that is **8 documents + 10 files = 18 entities, 26%**, whose most natural
identifier either fails or silently resolves elsewhere.

**3. The failure message suggests strings that are not selectors.**

```
  did you mean:
    document   docs/architecture.md.architecture  docs/architecture.md:1
    file       docs.architecture.md               docs/architecture.md:1
```

Both were typed back verbatim; both return "matches no indexed entity". The cause is
`crates/nerve-cli/src/main.rs:1913`, which folds `scope_path` into `scope.name` for **every** kind.
`EntityRef::qualified_name()` guards that fold on `is_symbol()`; the CLI has a private copy that
does not.

This is the **third** instance of this project's recurring defect class — the triplicated
symbol-kind SQL (7a-iii) and the drifting UI gloss maps (5d-iii) were the first two. The fix is the
same one that worked twice: one implementation, no second copy.

**4. Traversal refusal exists on one surface only.**

`crates/nerve-server/src/mcp/investigate.rs:426` refuses a traversal-shaped selector as
`path_refused`, because 8a held that a refusal must not be disguised as an absence (T2's rule).
The CLI and `/api/why` have no such check: `nerve why ../../etc/passwd` reports "matches no indexed
entity", which asserts something Nerve never checked. Same product, same question, two answers.

---

## Pushback on the assignment's proposed design

**`symbol:src/foo.ts::parseConfig` is rejected. The separator stays `#`.**

`<rel_path>#<qualified_name>` is the committed, tested, documented form. It is used by `nerve why`,
`nerve path`, `nerve impact`, `/api/*`, `nerve_investigate`, and `crates/nerve-index/tests/coverage.rs:660`.
Introducing `::` for the same job means two separators for one concept, a migration for every
example and test, and no capability that `#` does not already have. Divergence needs a benefit;
this one has none.

**A hand-written list of five prefixes is rejected. Qualifiers derive from `EntityKind`.**

The assignment lists `file:`, `document:`, `module:`, `symbol:`, `adr:`. Writing those five by hand
creates a second vocabulary that will drift from `EntityKind::ALL` the moment a kind is added —
which is exactly the failure 5d-iii and 7a-iii were corrective slices for. Every `EntityKind` gets
a qualifier, generated from `as_str()`, so drift is structurally impossible and a new kind is
addressable the day it exists. `symbol:` and `adr:` are then **aliases** over that vocabulary, not
members of it:

- `symbol:` — any kind where `is_symbol()` holds.
- `adr:` — a `Document` whose `meta.adr` is true; the body is matched against `meta.adr_id`.
  (`json_extract` verified working on the shipped SQLite build.)

**`path_refused` does not belong in `nerve-store`.** The store has no filesystem and cannot refuse
a path on filesystem grounds; to it a traversal selector is a string that matches nothing. The
check is syntactic and belongs where 8a put it — but as **one shared helper all three surfaces
call**, not a private copy in the MCP module.

---

## The design

### Grammar

```
selector  := [ qualifier ":" ] body
qualifier := <entity-kind>      generated from EntityKind::as_str()
           | "symbol"           alias: any kind where is_symbol()
           | "adr"              alias: Document with meta.adr = true, body matched on meta.adr_id
body      := <entity_id>
           | <rel_path>
           | <rel_path> "#" <qualified_name>
           | <name> | <qualified_name>
```

A qualifier constrains **which kinds a stage may return**. It does not change the stages.

### Stages, first that matches wins

1. `entity_id` — exact.
2. **path** — every entity *at* that exact path. **This is the change.** Today it asks only about
   modules. It must cover `Module`, `Document`, `File` and `Directory`.
3. `path#qualified` — unchanged.
4. bare name — unchanged.

### Stage 2 resolves a path in two tiers

At one path there may be a content entity and a container entity. `src/app.ts` has a `Module` and a
`File`; `docs/architecture.md` has a `Document` and a `File`.

- **Tier 1, content:** `Module`, `Document`.
- **Tier 2, container:** `File`, `Directory`.

Exactly one tier-1 match → **resolved to it**, with every tier-2 match returned as an
**alternative**. Tier 1 empty, exactly one tier-2 match → resolved to that. More than one match
within the deciding tier → **ambiguous**, nothing chosen.

**This is a rule, not a guess, and the distinction is load-bearing.** The module header forbids
choosing between candidates the tool cannot tell apart — two functions named `parse` are
indistinguishable to Nerve, so it must refuse. A `File` and the `Document` inside it are
distinguishable by a fixed, total, stated rule; the answer reports that the rule fired
(`matched_by = "path"`), lists what it passed over (`alternatives`), and the passed-over entity is
directly addressable as `file:<path>`. Silence would be the defect. A 100 %-firing ambiguity would
make path selectors useless, which is a different way of being wrong.

The implementer must **verify rather than assume** that `Module` and `Document` cannot both exist
at one path. If they can, that case is ambiguous.

### `Selection` gains what the caller needs to act

`Resolved` carries `alternatives: Vec<EntityRef>` — empty for every selector that has no second
reading, so existing answers are unchanged in shape but not in truth.

Two outcomes that are currently indistinguishable from "not found" become their own values:

- **invalid selector** — a qualifier that is not a kind or alias, an empty body, an empty
  qualifier. `banana:foo` is not a miss; it is a malformed request.
- **refused** — traversal-shaped. Produced by the shared helper, not by the store.

`NotFound` records the qualifier that was applied, so the message can say *"no document at
`src/app.ts` — there is a module"* rather than a bare miss.

Adding variants to a public enum is deliberate: every match site is a compile error until it is
updated, which is how the CLI, API and MCP are proven to have been brought along.

---

## Non-goals

- **No new MCP tools.** 8b-ii.
- **No schema change, no migration.** Every field this needs is already stored.
- **No exit-code change.** `NotFound` currently exits `NO_INDEX` (2), which is a stretch — the
  index exists and is healthy, the string just missed. It is pre-existing, tested and part of a
  contract CI scripts may depend on; changing it is a corrective slice with its own justification,
  not a side effect of this one. **Recorded, not fixed.**
- **No `apps/nerve-web/` change** unless the repository will not build without it.

---

## Acceptance criteria

1. `docs/architecture.md` resolves to the `Document`, with the `File` as an alternative.
2. `file:docs/architecture.md` resolves to the `File`.
3. `src/app.ts` resolves to the `Module`, with the `File` as an alternative — and the answer says
   so, rather than being silently one of two readings.
4. `module:docs/architecture.md` does not resolve, and says a document is there.
5. `banana:foo` is an **invalid selector**, distinct from not-found.
6. `adr:ADR-0001` resolves to that ADR document.
7. `symbol:parse` constrains to symbol kinds; a same-named module or document is not a candidate.
8. Ambiguity still refuses: two symbols named `describe` remain ambiguous, nothing chosen.
9. A traversal selector is **refused, not missed**, on **all three** surfaces — CLI, HTTP, MCP —
   through one shared helper. Absolute paths, `..`, dot segments, repeated separators, and
   backslash forms each covered.
10. Unicode paths, bare names and legal paths containing `..` inside a segment (`a..b.ts`) are
    **not** refused.
11. The CLI's private `qualified_name` copy is gone; every suggestion printed can be typed back and
    resolves.
12. Every 8a invariant holds unchanged: T7 confinement, boundedness, `initialize`-first, read-only,
    database byte-identical, no new dependency.
13. Mutation probes, each shown to fail the intended test for the intended reason:
    - make stage 2 module-only again → the document tests fail;
    - drop the two-tier rule so a path is always ambiguous → the resolution tests fail;
    - let the shared traversal helper return "not found" → refusal tests fail on all three surfaces;
    - accept an unknown qualifier as a bare name → the invalid-selector test fails.

## Verification gate

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace --no-fail-fast
cargo build --release
```

`--no-fail-fast` is mandatory: plain `cargo test` halts at the first failing target and understates
a mutation's blast radius (measured in Slice 7b — 3 reported against 16 actual).
