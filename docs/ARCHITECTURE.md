# Nerve — Architecture

## Layering

```
Surfaces        nerve-cli   ·   nerve-server + apps/nerve-web   ·   MCP (stdio)
                                  thin adapters — no business logic
                                            │
Application     index_repository() · status() · search_entities() · resolve_selector()
                find_paths() · explain() · impact() · gaps() · diagnose()
                ingest_coverage() · index_freshness() + untracked_files()  [nerve check]
                                            │
Pipeline        nerve-index:  discover → parse → extract → persist
                              emits Observations ONLY
                                            │
Storage         nerve-store:  SQLite, migrations, queries, FTS5
                              owns rebuild_assertion_state()  (pure derivation)
                                            │
Model           nerve-core:   ids · entity kinds · relations · evidence vocabulary
                              errors · canonical graph dump
```

### Crates

| Crate | Responsibility | Depends on |
|---|---|---|
| `nerve-core` | Identity computation, entity/relation/evidence vocabularies, error types, canonical serialization for golden tests | — |
| `nerve-store` | SQLite schema and migrations, all SQL, FTS5, `rebuild_assertion_state`, selector resolution, bounded path traversal, evidence assembly | `nerve-core` |
| `nerve-index` | File discovery, ignore/deny rules, tree-sitter parsing, extraction, indexing pipeline, query-time file prober | `nerve-core`, `nerve-store` |
| `nerve-server` | Loopback HTTP API, token/origin/host validation, embedded SPA assets, MCP over stdio | `nerve-core`, `nerve-store`, `nerve-index` |
| `nerve-cli` | `clap` command surface, human and `--json` rendering, exit codes | all |

`nerve-server` was created in Slice 4a and gained the MCP surface in Slice 8a. Both of its surfaces
are **pure surface over the application layer** — invariant 3 below — which Slice 8b-ii verified by
adding four MCP tools with `nerve-store`, `nerve-core`, `nerve-index` and `api.rs` byte-untouched.

`nerve-query` was **never created, and no longer will be.** The boundary it was reserved for turned
out to belong inside `nerve-store` (`query.rs`, `graph.rs`, `impact.rs`, `gaps.rs`, `select.rs`,
`freshness.rs`, `diagnose.rs`), because every one of those needs the SQL and the schema it would
have had to import wholesale. A crate whose entire dependency is one other crate's internals is a
module. Superseding master plan §3.4.

## Invariants

1. **Extractors emit observations only.** Nothing outside `nerve-store::rebuild_assertion_state`
   writes `assertion_state`. Enforced by module visibility and by a rebuild-equivalence test.
2. **Extractors declare their permitted evidence source types.** Emitting outside the
   declaration is a hard error.
3. **Surfaces contain no business logic.** If the CLI needs a computation, it lives in the
   application layer so the server, MCP, and CI reuse it unchanged.
4. **Unresolved is a value, not an omission.** Unresolvable references become `Unresolved`
   entities with real assertions, flagged `is_unresolved`.
5. **Determinism.** Indexing the same tree twice produces a byte-identical canonical dump.
   Slice 1 parses serially for this reason; parallelism arrives in Slice 3 with an ordered merge.
6. **Source text is never stored.** Ranges and content hashes only; source is read from disk
   when evidence is presented.

## Indexing pipeline

```
discover        ignore-crate walk · .gitignore · .nerveignore · secret deny-list
                path canonicalization · symlink-escape rejection
   │
read + hash     BLAKE3 content hash per file
   │
parse           tree-sitter, grammar chosen by extension
   │
extract         entities + occurrences + assertions + observations
                ts-js-structural  → CONTAINS · DEFINES · IMPORTS · EXPORTS
                ts-js-reference   → CALLS · REFERENCES · EXTENDS · IMPLEMENTS
                                    via bind (lexical scope) + exports (re-export closure)
   │
persist         single transaction: repository_state, one extractor_run per
                extractor, entities, occurrences, assertions, observations
   │
derive          rebuild_assertion_state()  — pure function of observation
```

## Repository state

Every index run creates a `repository_state` row identified by a BLAKE3 Merkle over the sorted
`(rel_path, content_hash)` pairs of all indexed files. If `.git` is present, the resolved HEAD
commit is recorded alongside it — read directly from `.git/HEAD` and the ref file, with no
subprocess. All observations are scoped to a state, which is how freshness is derived rather
than stored.

## Extension points (defined now, populated later)

- **Language extractor**: `(extension set, grammar, extract fn, declared evidence types)`.
- **Evidence policy**: ranking function over `evidence_source_type` supplied per query, so
  change-impact, architecture-intent, and documentation-drift queries can weigh evidence
  differently without schema change.
- **Identity link producer**: proposes evidence-bearing links across renames, moves, documents,
  tests, and repositories. Never silently merges.
