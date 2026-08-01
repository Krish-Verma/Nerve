# `ts-incremental` — invalidation, deletion and identity fixture

Purpose-built for Slice 3. Nothing here exercises *what* Nerve extracts — `ts-basic` and
`ts-resolution` own that — only *when* it re-extracts, what it removes, and what it refuses to
claim about identity.

## The barrel chain

```
app.ts  ──imports──▶  barrel.ts  ──export *──▶  impl.ts
                          │
                          └────export { … } from──▶  assist.ts
```

`app.ts` never names `impl.ts`. Editing `impl.ts` nevertheless changes what `app.ts` resolves,
because `exports.rs` follows the re-export closure. This is the case that makes "re-extract the
changed file and its direct importers" insufficient: the invalidation set must be the
*reverse-reachable* set over `IMPORTS`, and `export ... from` is on that edge.

`island.ts` imports nothing and is imported by nothing. It is the control: editing `impl.ts`
must **not** re-extract it. If it ever does, invalidation has become "re-index everything"
wearing a different name.

## The identity cases

`movable.ts` declares two functions with distinctive bodies. Tests move it, and the mover must
propose an `IdentityLink` carrying the evidence — matching `(kind, name, scope_path, body
digest)` across a removed and an added path.

`impostor.ts` is the **negative fixture**. It declares a function with the *same name* as one in
`movable.ts` and a completely different body. Deleting `movable.ts` while `impostor.ts` appears
must propose **nothing**: a name match is not evidence of identity, and ADR-0002 is explicit
that identity is never established by fuzzy name matching alone. It is committed here rather
than constructed in a test so that weakening the rule requires deleting a fixture, which is
visible in review.

`impostor.ts` is deliberately *not* part of the base tree; tests add it. The base tree is what
`nerve index` sees first.
