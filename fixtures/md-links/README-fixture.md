# `md-links` — the document-link precision corpus

Read by `crates/nerve-index/tests/document_links.rs`, which measures `md-structural`'s link
resolution against the hand-written ground truth in `expected.json`. The gate is **zero false
positives**: every resolved edge in the database must be declared, and every declared edge must
be present.

If a case here stops passing, tighten the rule. Do not relax `expected.json` to match the
output — the whole point of a measured corpus is that it can disagree with the implementation.

| Path | Role |
|---|---|
| `src/app.ts`, `src/util.ts` | anchor targets; `util.ts` holds a top-level function, a class, and a method inside that class, so the innermost-symbol rule has something to be wrong about |
| `README.md` | the positives: inline, root-relative, bracketed-with-a-space, and three line anchors; also the one link written before the first heading, whose source is the document |
| `docs/guide.md` | document-to-document, a reference definition, and an angle-bracket destination |
| `docs/my guide.md` | a file name with a space, so the percent-encoding negative fails against a path that really is indexed |
| `docs/negatives.md` | everything that must not resolve, in two groups: silent, and unresolved-with-a-reason |
| `assets/diagram.svg` | present on disk and not indexed, so "the guard succeeded" is visibly not the same as "it resolves" |

## The negative cases, and what each one proves

- a link inside a **fenced code block** — block structure is resolved before inline structure, so
  the link parser never sees it
- a link inside an **inline code span** — likewise
- a **bare code-span name** — `describe` in prose is not a reference to the symbol `describe`
- a **bare `#fragment`** — heading-anchor resolution is not modelled, and is counted rather than
  guessed at
- an **external URL** — counted, never fetched, and never an `Unresolved` entity, because nothing
  failed
- a **non-indexed file type** that exists on disk — membership of the indexed set decides, not
  the filesystem
- a **traversal destination** — refused before anything reaches the filesystem
- a **percent-encoded path** — not decoded, because decoding `%20` would also decode `%1f`
- an **anchor past end-of-file**, and an anchor on a line no symbol covers — the file still
  resolves; only the anchor does not

## Changing this fixture

Adding or editing a link means updating `expected.json` in the same commit. The harness fails
loudly on an undeclared edge, so a forgotten update cannot pass quietly.
