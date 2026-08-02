# Slice 7b — `nerve impact`

**Status:** planned 2026-08-02, ahead of implementation.
**Roadmap row:** 7b — "reverse dependency closure with evidence".

---

## Objective

`nerve impact <selector>` — given a symbol, report what depends on it, transitively, with the
evidence for each dependency and an honest account of what the answer cannot see.

## User value

The question is *"if I change this, what else might break?"* It is the question a person asks
before every non-trivial edit, and the one they currently answer with grep. Grep over-reports
(comments, strings, unrelated same-named symbols) and under-reports (re-exports, aliased imports,
inherited methods) at the same time, and tells you nothing about which of its hits are real.

## Non-negotiable: the answer must state what it cannot see

This is the slice's central design problem and the reason it needs care.

Slice 2a measured **38.1% of call sites on the resolution corpus as honestly `Unresolved`** — any
method call on a typed receiver is unresolvable without type inference, and Nerve has none. A
command that prints

```
3 entities depend on parseConfig
```

will be read as *"only 3 things use this, it is safe to change"*. If a third of the repository's
call sites could not be resolved, that reading is unsupported and the command has actively misled
someone into a breaking change. That is worse than not shipping the command.

So the unresolved count is **structural, not a footnote**: a field on the report, printed by the
CLI whether or not it is zero, and carried in the JSON. The precedent is Slice 7a's
`CoverageEvidence::Absent` — "no coverage ingested" is a distinct, unanswerable state with
`totals: null` rather than a list of zeroes. Same principle, applied to a different silence.

**What may be claimed, and what may not.** Nerve may say *"N call sites in this repository resolved
to nothing; any of them could reach this symbol and this answer cannot rule them out."* Nerve may
**not** match an unresolved site's name against the target's name and present the result as a
probable caller — that is identity by fuzzy name matching, which this project forbids and which
ADR-0002's tuples exist to prevent. Surfacing name-coincident unresolved sites as an explicitly
non-evidential "you may also want to look at" list is a *defensible* future feature and is a
**non-goal here** (see below).

---

## Pushback on the briefed shape

### 1. The default relation set must exclude `CONTAINS` and `DEFINES`

A reverse closure over containment from a function reaches its module, its file, its directory and
the repository itself. Every symbol would "impact" the repository. True, and useless — worse,
it buries the four edges that carry the actual answer under structural noise.

**Default:** `CALLS`, `REFERENCES`, `EXTENDS`, `IMPLEMENTS`. These are the symbol-level dependency
relations, and they are exactly the four Slice 2a resolves and measures.

### 2. The default must also exclude `IMPORTS`

Module-level `IMPORTS` closure is what *incremental invalidation* walks (`importers_of`,
`query.rs:210`), and it is correct there because re-extraction should be conservative — a false
positive costs a reparse.

It is the wrong default for a user-facing answer. If module A imports module B and I change a
function in B that A never calls, A is not affected. Reporting it as impact trains the user to
ignore the output. Different consumer, different tolerance for false positives; the same closure
is right for one and wrong for the other.

`--relation IMPORTS` remains available for anyone who wants the conservative closure, and reaching
it requires a two-stage walk (symbol → its module by reverse `DEFINES` → reverse `IMPORTS`) which
the implementation should support explicitly rather than by accident.

### 3. `COVERS` is excluded from the default, and is not impact

A reverse `COVERS` edge would report "coverage run R covered this symbol". That is a *freshness*
consequence of the change, not a dependency on the symbol. Mixing it into an impact list would
also put a `CoverageRun` — which is not a symbol and not code — into a list of things that depend
on your function. Excluded. Noted as a possible future addition under its own heading.

### 4. This is not `nerve affected`, and must never grow into it

`nerve affected` ("which tests would my change affect?") is **refused**, not deferred —
ADR-0008 §A.2, because LCOV carries no per-test attribution. `nerve impact` answers a static
dependency question over resolved AST edges. The two must not be conflated in output text, help
text, or naming. If the impact set happens to contain test files, that is because code depends on
code; it is not test attribution and must not be described as such.

### 5. Exit 0 when nothing depends on the symbol

Established by Slice 2b: absence is not an error. "Nothing resolved depends on this" is a finding,
and it is the *most* dangerous finding to report without the unresolved caveat attached.

---

## Scope

- `nerve store` — a reverse-closure query reusing `graph.rs`'s existing reverse adjacency
  (`adjacency_sql(query, reverse)`) and the `idx_assertion_target(target_entity_id, relation)`
  index. Do not write a second traversal engine.
- Bounded: `--max-depth` (follow `path`'s convention, default 6) and `--limit`, with exact totals
  reported regardless of the cap and an honest `truncated` flag — the `GapReport` pattern.
- Per result: the entity, the depth it was reached at, the relation and direction of the edge that
  reached it, and that edge's strongest evidence type plus computed freshness.
- The unresolved account, as above.
- `nerve impact` CLI with `--json`, stable exit codes, and the ambiguous-selector refusal every
  other selector command already implements (exit 10, list candidates, guess nothing).
- `/api/impact` on the existing read-only server, same application-layer call. No business logic
  in either surface (ARCHITECTURE.md invariant 3).
- Tests: fixtures with multi-hop closure, a cycle, a re-export chain, an unresolved-heavy case, an
  empty case, truncation, and a CLI↔API agreement test.

## Non-goals

- Name-coincident unresolved sites as suggested callers. Deferred deliberately; see above.
- `nerve affected`. Refused.
- Reverse `COVERS`, and coverage-staleness consequences of a change.
- Type inference to close the 38.1%. Out of reach and out of scope.
- Any `apps/nerve-web/` view for impact. The interface is frozen; the contract goes in
  `docs/UI-BACKEND-HANDOFF.md` and the user builds the view.
- Schema change. This is a query over existing tables.

## Acceptance criteria

1. Reverse closure over the four default relations, multi-hop, depth- and count-bounded, with
   exact pre-cap totals and a `truncated` flag.
2. Every result carries how it was reached and the evidence for that edge.
3. The unresolved count is present in human output and JSON **whether or not it is zero**, and a
   test asserts it is present when zero.
4. A cycle in the dependency graph terminates and each entity appears once.
5. Ambiguous selector → exit 10 with candidates; unknown selector → the existing not-found path.
6. Empty result → exit 0, with the unresolved caveat still printed.
7. CLI and API byte-agree on the same database.
8. Full gate green; a mutation probe confirms the unresolved caveat cannot be silently dropped.

## Verification

The standard gate, plus a CLI smoke test on a fixture with a known closure, plus a structural
bound check: an impact query must not scan the whole `assertion` table — it is indexed by
`idx_assertion_target` and the walk is bounded by depth.
