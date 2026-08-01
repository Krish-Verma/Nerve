# `md-docs` — the document-ingestion fixture

Read by `crates/nerve-index/tests/documents.rs`. It holds both code and documents on purpose:
the THREAT-MODEL.md **T7** invariant is a statement about *which side of the split* an
observation falls on, and a tree with documents only could not distinguish "the rule holds" from
"there was nothing to get wrong".

| Path | Role |
|---|---|
| `src/app.ts`, `src/util.ts` | the code side of the T7 invariant; also proves TS/JS extraction is unaffected by documents in the tree |
| `README.md` | the supported subset: front matter, ATX, setext, fenced and indented code, an inline code span, and two sibling sections with identical text |
| `docs/architecture.md` | a document whose first heading is level 3, and a later heading shallower than it |
| `docs/decisions/ADR-0001-header-status.md` | ADR by file name; status on a `**Status:**` header line, the form this repository uses |
| `docs/decisions/ADR-0002-status-section.md` | ADR by file name; status as the first line of a `## Status` section |
| `docs/decisions/ADR-0003-unparsed-status.md` | a status outside the closed vocabulary — recorded `unparsed`, never coerced |
| `docs/decisions/plain-note.md` | ADR by *directory*, with no id and no status; absent must not read as unparsed |
| `docs/hostile.md` | the negative fixture: scripts, event handlers, inline HTML, a `javascript:` link, traversal-shaped link text, prompt-injection prose, and both identity-forgery separators |

Link *resolution* is measured on its own corpus, `fixtures/md-links`. What `docs/hostile.md`
proves here is narrower and complementary: a hostile destination reaches the graph as an inert
`Unresolved` entity with a reason, and a traversal-shaped **link text** is never read as one.

## Changing this fixture

Adding a document means updating the expectations in `tests/documents.rs` in the same commit.
**Do not weaken or delete `docs/hostile.md` to make a test pass.** Every construct in it is a
recorded attack; if a control cannot hold against one of them, the control is wrong.

`docs/hostile.md` writes control bytes as the escapes `\x1f`, `\x01` and `\x0b`, which the test
harness substitutes for the real bytes before indexing. A raw C0 byte does not survive an
editor or a diff viewer, and the test asserts the substitution happened rather than trusting it.
