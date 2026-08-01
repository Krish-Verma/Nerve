# Slice 5c — document↔code link resolution · completion report

**Date:** 2026-08-01 · **Status:** Complete · **Plan:** `docs/plans/slice-05-document-evidence.md`

---

## Summary

A Markdown link that names a repository path now becomes an edge. `md-structural 1.1.0` emits
`Section REFERENCES <File>`, and — when the destination carries a `#L<n>` anchor — additionally
`Section REFERENCES <symbol>`, resolved to the innermost symbol span covering that line. Anything
that fails to resolve becomes an `Unresolved` entity carrying a closed reason, which is how Nerve
reports a broken documentation link.

Everything still carries `DOCUMENT_STATED`; only `directness` moves to `RESOLVED`. Verified across
the whole real repository: `md-structural` emits exactly `DOCUMENT_STATED/DIRECT` and
`DOCUMENT_STATED/RESOLVED`, and **zero** non-`DOCUMENT_STATED` observations exist on any `.md` path.

## The result that matters more than the precision number

**Nerve's own 45 documents contain 5 Markdown link sites in total.** Across the entire repository:

```
document_link_resolved_file          2
document_link_target_not_indexed     2
document_link_external               1
```

Precision on the fixture corpus is 100%, but the construct that precision is measured on is **rare
in real prose**. Documentation overwhelmingly refers to code the way this very sentence does — with
an inline code span like `crates/nerve-index/src/pipeline.rs` — and Slice 5c deliberately refuses to
treat a code span as a link, because a document mentioning `parseConfig` is not evidence that it
means the symbol `parseConfig`.

So the honest statement of this slice's capability is: **high precision, narrow coverage, by
design.** That trade is the correct one — a fuzzy rule would have produced hundreds of edges here
and no way to tell which were real — but it must not be reported as "documents are now linked to
code". They are linked where the author wrote an actual link, which is not often.

This is a product finding, not a bug, and it belongs in the real-world validation phase's
document-link category as an expected low-recall result.

## Emission rules as implemented

| Outcome | Emitted |
|---|---|
| destination names an indexed file | `REFERENCES File`, `DOCUMENT_STATED` / **`RESOLVED`** |
| `#L<n>` resolves via `innermost_covering` | *additionally* `REFERENCES <symbol>`, with the **target file's** `content_hash` in `details.target_content_hash` |
| `#L<n>` covered by no symbol, or past EOF | file edge stands, **plus** `Unresolved` — `document_anchor_no_symbol` |
| path climbs above the root, or the guard refuses | `Unresolved` — `document_link_refused` |
| repository-shaped path naming nothing indexed | `Unresolved` — `document_link_target_not_indexed` |

Source is the innermost `Section` whose span covers the link; a link before the first heading
belongs to the `Document`.

**Produce nothing at all:** external destinations of any scheme (counted, never fetched, never
entity-ised — nothing failed, so there is nothing unresolved to record); bare `#fragment`; bare
code-span mentions; anything inside a fence or code span; percent-encoded paths; link *text*
(only the destination is ever read). `Document SUPERSEDES Document` is deferred to Slice 5d, and
the emitted count of `SUPERSEDES` is asserted to be 0.

## Precision — **fixture-only**, `fixtures/md-links`, 17 sites

```
sites scanned          17
  resolved to a file   12      resolved to a symbol 3
  external 1 · fragment only 1 · refused 1 · not indexed 2 · anchor-no-symbol 2
code-span mentions     8  (counted, never emitted)
edges: 11 resolved, 5 unresolved (from 20 observations)
TP 11 · FP 0 · FN 0
precision 100.0% · recall 100.0% · unresolved 31.2% · unsupported 40.0%
```

**Seventeen sites is a small corpus.** The number is a gate against regression, not a claim about
real-world accuracy. All eight required negatives are present: a link in a fenced code block, a link
in an inline code span, a bare code-span name, a non-indexed file type, a traversal destination, an
external URL, an anchor past end-of-file, and a percent-encoded path. Nothing was loosened to reach
100%.

## Verification — run by the orchestrator

```
cargo fmt --all -- --check                              → clean
cargo clippy --workspace --all-targets -- -D warnings   → 0 warnings
cargo test --workspace                                  → 564 passed, 0 failed, 2 ignored
cargo build --release                                   → Finished
```

553 → 564 (+11). `no_network.rs` and `no_subprocess.rs` pass untouched.

### Adversarial probe — run by the orchestrator

`normalize`'s traversal refusal was mutated to *clamp* at the root instead (`segments.pop()?` →
`segments.pop()`), which is a real security regression: `../../../etc/passwd` would become a
resolvable `etc/passwd`.

```
docref::tests::climbing_above_the_root_is_refused_rather_than_clamped ... FAILED
  left: Some("etc/passwd")
```

Reverted and verified **byte-identical by sha256**.

Note what this probe also showed: the *integration* fixtures would not have caught it, because a
clamped path is still unindexed and still lands as unresolved — only the reason changes. The unit
test is what holds this line.

### Real-repository check

45 documents, 10 ADRs, 489 sections. `ADR-0002` → `ADR-0006` resolves (2 edges). The two hostile
fixture links land as `Unresolved`. **0** entities named `http…` or `javascript:…` — external
destinations are counted and never entity-ised. **0** non-`DOCUMENT_STATED` observations on `.md`.

## One existing test changed meaning — reviewed, and it is stricter

`documents.rs::a_hostile_document_is_stored_as_inert_data_and_forges_no_identity` asserted *"a
document may only CONTAIN"* — a Slice-5a property this slice deliberately ends. The replacement is
strictly stronger: relations out of a document or section are **exactly** `{CONTAINS, REFERENCES}`;
hostile.md's references are **exactly** the two named unresolved entities; no entity is named
`javascript:…`; the external destination is counted. The old form was one scalar count; the new form
pins identities. The agent flagged the change rather than making it quietly, and **the hostile
payloads in the fixture were not touched** — only the explanatory prose beneath them.

## Invalidation

A document enters `resolution_changed` when a cached destination resolves differently against the
whole path set (target added, removed or moved) via the pure `docref::resolve_path`, or when a
destination carrying a line anchor points into a file whose content hash changed. An unanchored link
deliberately does not depend on target bytes.

Because `#L<n>` needs symbol *extents*, which exist only for a file the run parsed, a re-extracted
document forces its code anchor targets into the extraction set for one round. **This costs extra
re-extraction**: editing a document with an anchor re-parses the target file even if that file did
not change. The alternative — caching spans in `CachedSymbol` — changes the cache payload for every
code file in the repository. The cheaper-to-review option was taken and is stated in the code.

The equivalence sequence gained five scripted steps — link added, target edited, target deleted,
target restored, target moved — each checked byte-identical incremental-versus-from-scratch, with
the moved case additionally asserted to leave a broken-link entity behind.

## Known limitations

- **Coverage on real prose is small** — see above. This is the headline limitation.
- Inline code spans are matched within a line only; a span opened on one line and closed on the next
  will not hide a link on the first.
- `[id]: dest` is single-line only; CommonMark allows the destination on the next line.
- Percent-encoded paths read as broken, deliberately — decoding would let `%1f` smuggle a tuple
  separator past the scanner into the path guard.
- Heading anchors (`#some-heading`) are not modelled; they are counted as fragment-only.
- `document_link_refused` does not fire on Nerve's own repository, because the traversal-shaped
  fixture link does not actually climb above the root from where it sits. It fires on the fixture
  corpus.
</content>
