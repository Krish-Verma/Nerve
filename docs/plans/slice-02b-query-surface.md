# Slice 2b — Graph query surface (`nerve path`, `nerve why`)

**Date:** 2026-07-31 · **Status:** Planned · **Depends on:** Slice 2a

---

## Objective

Make the graph Slice 2a produces *inspectable*: find how two entities are connected, and show
the evidence behind any single relationship — including whether that evidence is still fresh.

## User value

2a can assert `Widget CALLS area`. Without 2b a user cannot ask *why Nerve believes that*, or
*how* two parts of a system connect. Evidence that cannot be inspected is indistinguishable
from a guess, which is the entire thing Nerve claims not to be.

---

## Disagreements and Pushback

### Q1 — `nerve why` is the differentiating command, not `nerve path`

Path-finding over a code graph is table stakes. The command that expresses Nerve's actual
thesis is `why`: source type, directness, extractor id **and version**, exact `file:line`, and
the content hash recorded at observation time. If implementation time is short, `why` ships
complete and `path` ships bounded — not the reverse.

### Q2 — Freshness must be computed, not stored

`observation.content_hash` records what the file said when observed. `nerve why` should re-hash
the file on disk and report `fresh` / `stale` / `file-missing` per observation. This is the
single highest-value line in the output and it costs one BLAKE3 per distinct file.

This makes `why` the first command that reads repository files at query time. It must reuse the
Slice 1 path-safety guarantees (canonicalize, assert inside root, do not follow symlinks out) —
it must not open a path taken from the database without re-validating it.

### Q3 — Ambiguous selectors must fail loudly

`nerve path add area` where two entities are named `add` must **not** pick one. It lists the
candidates and exits `10`. Silently choosing is the failure mode that makes a tool untrustworthy
in exactly the situation where the user most needs it to be right.

### Q4 — Unresolved edges must be visible in paths, not filtered out by default

A path that traverses an `Unresolved` node is a real finding ("the connection leaves what Nerve
can see here"). Default: include them, mark them. `--resolved-only` opts out. Hiding them by
default would make the graph look more complete than it is.

---

## Scope

### `nerve path <from> <to>`

- Bidirectional or forward BFS over `assertion`, bounded depth (default 6, `--max-depth`).
- `--relation R` (repeatable) filters edge types; default is all.
- `--limit K` distinct paths (default 3).
- `--direction forward|any` (default `forward`; `any` treats edges as undirected).
- `--resolved-only` excludes edges whose `assertion_state.is_unresolved = 1`.
- Output: each hop as `entity → [RELATION] → entity` with `file:line` and the edge's
  `strongest_source_type`. `--json` gives nodes, edges and per-edge evidence summary.
- Exit `0` with an explicit "no path found within depth N" message when there is none — absence
  of a path is not an error.

### `nerve why <from> [<to>]`

- With both: every assertion between the two entities, in either direction.
- With one: every assertion where that entity is source or target (`--incoming` / `--outgoing`
  to restrict, `--relation R` to filter).
- Per assertion: relation, direction, derived `status`, `is_unresolved`, `observation_count`,
  `strongest_source_type`, and then **every observation** — source type, directness,
  extractor id + version, `file:line`, environment, `match_quality` when present, `details`,
  and computed freshness (Q2).
- `--json` output is a stable contract.

### Selectors (shared)

Resolution order, first match wins:
1. exact `entity_id`
2. `<rel_path>` → that file's `Module` entity
3. `<rel_path>#<qualified_name>`
4. bare `<qualified_name>` or `<name>` → unique match across the repository

Zero matches → exit `2` with the nearest `nerve search` suggestions.
More than one match → exit `10`, list candidates with ids (Q3).

### Where the code lives

Query logic goes in `nerve-store` (or a new `query::graph` module) so the Slice 4 server,
Slice 7 CI, and Slice 8 MCP reuse it unchanged. ARCHITECTURE.md invariant 3: the CLI renders
and maps exit codes, and contains no traversal logic.

---

## Non-goals

No `impact`, `gaps`, `check` (Slice 7). No ranking or evidence *policy* engine — `why` reports
the profile, it does not score it. No graph mutation. No schema change. No caching layer.

---

## Acceptance criteria

1. Full verification gate passes.
2. `nerve path` finds a known multi-hop path in `fixtures/ts-resolution` and reports the exact
   hop count; a pinned no-path case exits `0` with the explicit message.
3. `nerve why` on a resolved 2a edge prints every observation with extractor id + version and
   `file:line`; a golden-style test pins the `--json` shape.
4. Freshness: a test mutates a fixture file after indexing and asserts the affected observations
   report `stale` while others report `fresh`.
5. Ambiguous selector exits `10` and lists candidates; missing selector exits `2`.
6. Path traversal and symlink-escape guards apply to query-time file reads (Q2), with tests.
7. Traversal latency measured on the Slice 1 synthetic scale graph and recorded.
8. No business logic in `nerve-cli` — traversal and evidence assembly live in `nerve-store`.

## Stop conditions

- If bounded-depth traversal on the scale fixture exceeds ADR-0001's p95 budget, stop and record
  the measurement; a materialized adjacency layer becomes a slice of its own rather than an
  unmeasured optimization bolted on here.
