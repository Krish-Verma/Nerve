# Negatives

Nothing below may produce a resolved edge. Some of it produces an `Unresolved` entity with a
reason; the rest produces nothing at all, and "nothing at all" is a different claim from
"unresolved" — `expected.json` distinguishes the two.

## Not a link at all

A fenced block is not prose, so the link parser never sees what is inside it. It is not that the
parser rejects the destination; it is that the destination never reaches the parser:

```md
[fenced](../src/fenced.ts)
```

An inline code span is not prose either: `[spanned](../src/spanned.ts)`.

Both destinations are spelled uniquely and name no file. If the scanner ever saw one, it would
become an `Unresolved` entity under that exact name, which is what the harness looks for: the
requirement is that they produce **nothing**, not merely that they do not resolve.

A bare code-span name is prose *about* a symbol, not a reference *to* one: `describe`. Emitting
an edge for it would be name matching, which ADR-0002 refuses as a basis for identity.

A destination that is only a fragment names a heading, and heading anchors are not modelled:
[a heading](#negatives).

An external destination is counted and never fetched: [the spec](https://example.invalid/spec).
It is not an `Unresolved` entity, because nothing failed.

## Unresolved, with a reason

A file that exists on disk and is not indexed: [the diagram](../assets/diagram.svg). Resolution
is decided by membership of the indexed path set, not by the filesystem.

A destination that climbs above the repository root: [passwd](../../../etc/passwd). It is refused
before anything reaches the filesystem.

A percent-encoded path names nothing even though `docs/my guide.md` is indexed:
[the spaced guide](./my%20guide.md). Decoding `%20` would also decode `%1f`, and the guard is
asked about the bytes before any decode could happen.

A line anchor on a line no symbol covers: [a blank line](../src/util.ts#L4).

A line anchor past the end of the file: [past the end](../src/util.ts#L999).
