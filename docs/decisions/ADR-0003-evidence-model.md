# ADR-0003 — Evidence model and schema v1

**Status:** Accepted · **Date:** 2026-07-31 · **Slice:** 1

## Context

Nerve's differentiator is that every conclusion is backed by inspectable evidence scoped to a
repository state. That is only true if the *schema* enforces it. A single edge table with a
`confidence` float cannot.

## Decision

### No scalar confidence

We do not store `confidence: f64`. A number like `0.94` on an individual relationship is not
falsifiable: there is no procedure that would tell us it should have been `0.91`.

Instead, an observation carries a **structured evidence profile**:

| Field | Meaning |
|---|---|
| `evidence_source_type` | How the evidence was obtained (closed vocabulary below) |
| `directness` | `DIRECT` (the artifact literally states it) / `RESOLVED` (derived through a resolution step) / `INFERRED` (a rule concluded it) |
| `extractor_id`, `extractor_version` | Who produced it; enables surgical invalidation |
| `match_quality` | **Only** for extractors that perform matching; `NULL` otherwise. Semantics documented per extractor |
| `state_id` | Which repository state it was observed in — freshness is derived, not stored |
| `environment` | For execution evidence: `test` / `dev` / `prod` |
| `file_path`, `start_line`, `end_line`, `content_hash` | Where to look, and what the source said at the time |
| `details` | JSON, extractor-specific, human-readable evidence steps |

"How confident are we?" is answered by the **extractor's measured precision on its fixture
corpus**, which is a property of `(extractor_id, extractor_version)` and is testable and
regressable in CI — not by a per-row guess.

### Closed vocabulary — `evidence_source_type`

```
AST_DIRECT          the syntax tree literally contains this relationship
AST_RESOLVED        resolved through import/module resolution
AST_HEURISTIC       name-based or otherwise ambiguous match
TYPE_RESOLVED       a type checker resolved it
FRAMEWORK_RULE      a deterministic framework rule inferred it
TEST_COVERAGE       a test executed this symbol   (NOT a call relationship — ADR-0005)
TEST_CALL_TRACE     a call was observed during a test, via instrumentation
RUNTIME_CALL_TRACE  a call was observed at runtime
DOCUMENT_STATED     a document asserts it
HUMAN_CONFIRMED     a human confirmed it
LLM_DERIVED         a language model suggested it
```

**These are deliberately not a single truth ranking.** Different queries need different
policies: change-impact prioritises source + execution evidence; architecture-intent
prioritises ADR and human evidence; documentation-drift *compares* document assertions against
source assertions. Ranking is therefore supplied by an **evidence policy** at query time, not
baked into the vocabulary.

### Extractors emit observations only

No extractor may write `assertion_state`. That table is rebuilt as a **pure function** of
`observation`, and there is a test that truncates it, rebuilds, and asserts identical content.
This bounds the blast radius of a buggy extractor to "adds filterable noise at a declared
source type", and makes its contribution revocable with one `DELETE ... WHERE extractor_id=?`.

Each extractor **declares** the source types it is permitted to emit; emitting outside that
declaration is a hard error, not a warning.

### Unresolved references are entities, not omissions

An import that cannot be resolved creates an `Unresolved` entity and a real `IMPORTS`
assertion pointing at it, with `assertion_state.is_unresolved = 1`. Unresolved relationships
are therefore queryable and countable rather than silently dropped.

## Schema v1

```sql
schema_version(version PK, applied_at, description)

repository(repo_id PK, project_id, root_path, created_at)

repository_state(state_id PK, repo_id, kind, git_commit NULL,
                 content_merkle, created_at)

entity(entity_id PK, repo_id, kind, name, scope_path, language, meta JSON)

occurrence(occurrence_id PK, entity_id, state_id, file_path,
           start_byte, end_byte, start_line, start_col, end_line, end_col,
           content_hash)

assertion(assertion_id PK, repo_id, source_entity_id, relation, target_entity_id)

observation(observation_id PK AUTOINCREMENT, assertion_id, extractor_run_id,
            evidence_source_type, directness, extractor_id, extractor_version,
            match_quality NULL, state_id, file_path, start_line, end_line,
            content_hash, environment NULL, details JSON, created_at)

assertion_state(assertion_id PK, state_id, status, strongest_source_type,
                source_type_mask, observation_count, is_unresolved,
                last_seen_state_id)          -- DERIVED, rebuilt from observation

extractor_run(run_id PK, repo_id, state_id, extractor_id, extractor_version,
              started_at, finished_at, files_processed, files_failed, status)

identity_link(link_id PK, repo_id, left_entity_id, right_entity_id,
              link_kind, evidence JSON, created_at)   -- created, unused until Slice 3

entity_fts  FTS5(name, scope_path, content='entity', content_rowid=...)
```

`assertion_state.status` ∈ `SUPPORTED` | `CONTRADICTED` | `STALE` | `UNRESOLVED` | `DELETED`.
In Slice 1 only `SUPPORTED` and `UNRESOLVED` occur; the others require multiple extractors or
incremental indexing and are defined now so the vocabulary does not churn later.

`source_type_mask` is a bitmask over `evidence_source_type` enabling
`WHERE source_type_mask & ?` filtering without a join — the query shape that evidence policies
need most.

## Consequences

- More tables and more joins than an edge list. Accepted: the joins are the product.
- `assertion_state` carries no information in Slice 1 (one extractor ⇒ one observation per
  assertion). Built anyway to establish the "extractors never write state" boundary while it
  is free to establish.
