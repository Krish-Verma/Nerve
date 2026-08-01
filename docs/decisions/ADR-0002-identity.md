# ADR-0002 — Entity and occurrence identity

**Status:** Accepted, with documented known defects · **Date:** 2026-07-31 · **Slice:** 1
**Amended by:** [ADR-0006](ADR-0006-state-independent-occurrences.md) (Slice 3b) — §2
`OccurrenceId` no longer digests the repository state. Everything else here stands.

## Context

The build prompt requires that identity not be "file path plus symbol name", that logical
entity identity be separated from physical occurrence identity, and that we **not overclaim
that symbol identity is solved**.

## Decision

Three distinct identifiers.

### 1. `EntityId` — logical identity

`<kind-prefix>_<blake3(canonical-tuple) truncated to 32 hex chars>`

| Kind | Canonical tuple |
|---|---|
| Repository | `("repository", project_id)` |
| Directory | `("directory", project_id, rel_path)` |
| File | `("file", project_id, rel_path)` |
| Module | `("module", project_id, rel_path)` |
| Function / Class / Interface / Method | `("<kind>", project_id, module_rel_path, scope_path, name, disambiguator)` |
| Unresolved | `("unresolved", project_id, importer_rel_path, raw_specifier)` |

- `project_id` is generated once at `nerve init` and stored in `.nerve/config.toml`. It is
  **not** derived from the absolute path, so moving or re-cloning the repository does not
  change identity.
- `scope_path` encodes lexical nesting: `Shapes.area`, `outer/<local:inner>`,
  `<anon:arrow@3>` for the third anonymous arrow function in a module, by source order.
- `disambiguator` is a stable index among otherwise-identical siblings, assigned in source order.
- **Body content is deliberately excluded.** Editing a function must not change its identity.

### 2. `OccurrenceId` — physical identity

~~`blake3(entity_id, state_id, rel_path, start_byte, end_byte)`. One row per appearance of an
entity in a specific repository state.~~

**Amended by ADR-0006 (Slice 3b):** `blake3(entity_id, rel_path, start_byte, end_byte)`. One row
per appearance of an entity at a byte span in a file. An occurrence is a location fact and does
not depend on which run observed it; `occurrence.content_hash` records what the file said.
Including the state made every re-index rewrite every surviving row — an O(repository) write for
an O(change) edit.

### 3. `AssertionId` — claim identity

`blake3(source_entity_id, relation, target_entity_id)`. Deduplicates claims so that many
observations can support one claim.

## Known defects — explicitly not solved

1. **File moves change every symbol id in the moved file**, because `rel_path` participates in
   the tuple. Unavoidable in Slice 1: without package resolution (tsconfig `paths`, workspace
   layouts, `node_modules` semantics) two identically-named functions in different files are
   otherwise indistinguishable. **Bridged in Slice 3** by `IdentityLink` rows carrying
   body-hash similarity and git-lineage evidence — a *proposed, evidence-bearing* link, never
   a silent merge.
2. **Renames change identity.** Same bridge, same slice.
3. **Anonymous-function ids are position-sensitive.** Reordering sibling arrow functions
   changes their `scope_path`. This degrades to "new entity", never to "wrong entity", which
   is the acceptable direction to fail.
4. **Overloads** are separated only by `disambiguator` (source order) until Slice 2 adds
   signature shape. Two overloads whose order is swapped will exchange identities.
5. **Re-exports** create an `EXPORTS` assertion from the re-exporting module; the underlying
   entity keeps its defining module's identity. Barrel files therefore do not clone entities.

## Migration policy

Identity inputs are versioned by `schema_version`. Any change to a canonical tuple is a
breaking change requiring a migration that either re-indexes from scratch or writes
`IdentityLink` rows mapping old ids to new. Silent identity changes are prohibited.

## Rationale for excluding paths where possible

`project_id` rather than absolute path keeps indexes portable between clones and machines,
which offline-first and future portable-export both need.
