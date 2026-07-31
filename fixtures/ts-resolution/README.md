# `ts-resolution` — the precision corpus

This tree exists to be **measured**, not to be realistic. Every file is hand-authored so that
`expected.json` can be a complete, reviewable statement of what Nerve is allowed to claim about
it. It is read by `crates/nerve-index/tests/precision.rs`, which indexes a copy of this tree and
enforces five gates.

## Ground truth

`expected.json` has four parts.

| Key | Meaning | Gate |
|---|---|---|
| `resolved` | Every `AST_RESOLVED` `CALLS` / `REFERENCES` / `EXTENDS` / `IMPLEMENTS` edge the corpus must contain — **and the only ones it may contain** | false negatives = 0 **and** false positives = 0 |
| `unresolved` | Every edge to an `Unresolved` entity the corpus must contain, with the reason `ts-js-reference` recorded — and the only ones it may contain | every entry present, nothing undeclared |
| `forbidden` | Edges that must not exist **in any form**, resolved or unresolved. Each one is the plausible wrong answer a name-matching implementation would give | none present |
| `unmodelled_call_sites` | Call sites and heritage clauses whose form Nerve does not model, counted rather than guessed | exact match |

Both `resolved` and `unresolved` are **exhaustive**, in both directions. That is deliberate: a
precision number computed against a partial ground truth measures nothing. It also means adding
a rule to the extractor requires declaring the edges it produces here, which is the point.

## Selector syntax

```
<rel_path>#<qualified_name>     a symbol: qualified_name is scope_path.name,
                                or just name at module top level
<rel_path>                      that file's Module entity
```

Examples: `src/math.ts#add`, `src/shapes.ts#Rectangle.area`,
`src/shadow.ts#sameNameDifferentScopes.helper`, `src/app.ts`.

The harness fails loudly if a selector is ambiguous or matches nothing. It never skips.

## What each file is for

| File | Role |
|---|---|
| `src/math.ts` | same-module call; the named and default exports everything else pulls on |
| `src/shapes.ts` | `implements`, `interface extends`, `class extends`; both `this.m()` failure modes |
| `src/generic.ts` | `extends Base<T>` — generic arguments stripped to the head identifier |
| `src/barrel.ts`, `src/index.ts` | a two-level barrel chain, named and star re-exports |
| `src/app.ts` | every positive import form; a typed-parameter receiver that must stay unresolved |
| `src/legacy.cjs` | `const m = require('./x')` then `m.foo()` |
| `src/shadow.ts` | shadowing by parameter, by `const`, and by a sibling scope |
| `src/withdefault.ts`, `src/starexport.ts`, `src/stardefault.ts` | `export *` does not re-export `default` |
| `src/unmodelled.ts` | `a[b]()`, `f()()`, an IIFE, `extends mixin(B)` |
| `src/globals.ts` | `console.log()` — a host global |
| `src/missing.ts` | a call through an import that resolves to nothing |
| `src/heritage.ts` | `extends` and `implements` across a module boundary |

## Changing this corpus

Adding a file or an edge means updating `expected.json` in the same commit. **Do not weaken or
delete a negative fixture to make the gate pass.** If a rule cannot reach zero false positives
without doing so, the rule is wrong: retract it and report it
(`docs/plans/slice-02-static-resolution.md`, Stop conditions).
