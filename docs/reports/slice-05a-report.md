# Slice 5a — Markdown and ADR ingestion · completion report

**Date:** 2026-08-01 · **Status:** Complete · **Plan:** `docs/plans/slice-05-document-evidence.md`
**Gate:** THREAT-MODEL **T7** — satisfied

---

## Summary

Nerve reads prose. `.md` and `.markdown` files are discovered by exactly the same rules as source
and scanned by a new extractor, `md-structural 1.0.0`, into `Document` and `Section` entities with
exact spans, content hashes and heading nesting. ADRs are recognised deterministically and their
status is parsed from a closed vocabulary.

The rule the slice is built around: **everything derived from a document carries
`DOCUMENT_STATED`**, including the structural `File CONTAINS Document` edge. That makes the T7
separation a total function of the source file rather than a per-claim judgement, which is what
makes it checkable exhaustively rather than by spot check.

**No dependency was added.** `Cargo.toml`, `Cargo.lock` and `third_party/LICENSES.md` are byte-
identical.

## Two identity-forgery defects — one found by the implementer, one by the orchestrator

Heading text and file names are both attacker-controlled, and both feed canonical identity tuples
(ADR-0002), whose injectivity depends entirely on **no field containing the `0x1f` separator**.

### 1. The plan's own `heading_path` was forgeable — caught by the implementer

The plan specified `section_id(project, rel_path, heading_path, ordinal)` with `heading_path` as a
`>`-joined chain, and defended it by stripping control characters. That defence is insufficient,
because `>` is ordinary printable heading text:

```
# A>B          # A
## C           ## B
               ### C
```

Both `C` sections have chain `A>B>C` and sibling ordinal 0. The fix spreads each heading segment
into its **own tuple field**, so the unit separator does the separating and the encoding is
injective in the number of segments as well as their contents. `strip_control` is applied inside
the constructor, so no caller can forget it.

**This was a defect in my plan, not in the implementation.**

### 2. The same attack through the *path* field — found by orchestrator probe, missed by both

Sanitising heading text does not close the class, because `rel_path` is also a tuple field and a
file name may legally contain `0x1f` on Unix. I constructed it and it worked:

| File | Contents | Tuple |
|---|---|---|
| `docs/a.md` | `# Parent.md` / `## Child` | `(section, pid, "docs/a.md", "Parent.md", "Child", 0)` |
| `docs/a.md<0x1f>Parent.md` | `# Child` | `(section, pid, "docs/a.md<0x1f>Parent.md", "Child", 0)` |

Those encode to **identical bytes**. Before the fix, indexing that tree produced:

```
sections: [('sect_773d336d…', 'Child', 'docs/a.md')]        ← one entity
multi-file occurrences: [('sect_773d336d…', 2, 'docs/a.md,docs/a.md\x1fParent.md')]
```

One section entity with occurrences in two different files — two distinct things merged, silently,
at attacker choice.

**Fix:** `canonical_child` now refuses the entire C0 range rather than only NUL, at the single path
choke point. That closes the class for **every** identity constructor at once — sections, symbols,
modules, files — instead of leaving each to defend itself. Refusals are counted in
`DiscoveryReport::refused_paths` rather than silently dropped, matching the project's standing rule
that a refusal is reported as a refusal. A new `IndexError::ControlCharacterInPath` keeps that
distinct from `PathEscapesRoot`, because it is a different finding: the path did not escape
anything, it attacked identity.

After the fix, the same tree yields two distinct sections and refuses the hostile file.

## Files

**New** — `crates/nerve-index/src/markdown.rs` (the scanner, 27 unit tests),
`crates/nerve-index/src/docs.rs` (`md-structural`, ADR recognition, 20 unit tests),
`crates/nerve-index/tests/documents.rs` (14 integration tests), `fixtures/md-docs/**`.

**Changed** — `nerve-core`: `vocab.rs` (`Document`, `Section`, `Supersedes`), `ids.rs`
(`document_id`, `section_id`, `strip_control`). `nerve-index`: `lang.rs` (`FileKind`),
`discover.rs`, `error.rs`, `facts.rs`, `incremental.rs`, `pipeline.rs`, `lib.rs`. `nerve-cli`:
`main.rs`. Tests updated for real behaviour change: `cli.rs`, `graph.rs`, `incremental.rs`,
`safety.rs`.

**Not changed** — no schema migration (`entity.kind` carries no `CHECK`), no dependency, no
`guard.rs`/`token.rs`/`respond.rs`, no `apps/nerve-web`.

## The scanner, and what it refuses

Supported: ATX headings (`#`–`######`, closing-hash run), setext headings, fenced code (``` and
`~~~`, including unterminated), indented code, inline code spans, YAML front matter at byte 0.
Text inside a fence or a code span is not structure.

Refused and **counted** under a closed tag set: `atx-over-six-hashes`,
`setext-multiline-paragraph`, `unterminated-fence`, `unterminated-front-matter`,
`front-matter-lines-exceeded`, `headings-exceeded`, `heading-in-block-quote`,
`heading-in-list-item`, `html-block`. Bounds: 10 000 headings, 6 levels (CommonMark's own, so
recursion is bounded by construction), 1 000 front-matter lines, existing 2 MiB file ceiling.

## Verification — run by the orchestrator, not accepted from the report

```
cargo fmt --all -- --check                              → clean
cargo clippy --workspace --all-targets -- -D warnings   → 0 warnings
cargo test --workspace                                  → 506 passed, 0 failed, 2 ignored
cargo build --release                                   → Finished
```

435 → 506 (+71). Nothing weakened, skipped or ignored; the 2 ignored are the pre-existing opt-in
measurement harnesses. The baseline was re-confirmed at 435 in a clean worktree at HEAD.

`golden_graph_matches_the_committed_dump` still passes unchanged — the TS/JS graph is byte-identical,
so document support did not perturb code extraction.

### Adversarial probes — run by the orchestrator

| Mutation | Result |
|---|---|
| `md-structural` declares and emits `AST_DIRECT` instead of `DOCUMENT_STATED` | **T7 test failed**, naming 54 offenders with file paths |
| A modified document classified as unchanged | **`incremental_and_full_agree_under_a_seeded_edit_sequence` failed** |
| Control-character path refusal removed (the pre-fix state) | **Collision reproduced** — one entity, two files |

Both code mutations were reverted and the files verified **byte-identical by sha256** against
pre-mutation copies, with the suites re-run green.

### Smoke test — Nerve's own documentation

```
documents      30 scanned, 6 ADRs, 391 sections
md-unsupported 4 constructs refused  (heading-in-block-quote 2, html-block 2)
T7: observations on .md with a non-DOCUMENT_STATED source type → 0
by source type: AST_DIRECT 4, DOCUMENT_STATED 451
```

ADR statuses read from the real files: `ADR-0001` `Accepted`, **`ADR-0002` `unparsed`**, `0003`–
`0006` `Accepted`. `ADR-0002` genuinely reads `**Status:** Accepted, with documented known
defects`, which is not `Accepted`. Truncating it to the first word would delete a qualification its
author wrote deliberately, so it is recorded `unparsed` with the raw text preserved. The test pins
that outcome rather than softening the parser. **This is correct behaviour, not a gap.**

## A pre-existing defect this slice made visible — corrective slice raised

Those 4 `AST_DIRECT` observations above are in a repository containing **no TypeScript at all**.
They are directory containment — `Repository CONTAINS docs`, `docs CONTAINS decisions` — attributed
to `ts-js-structural` and labelled `AST_DIRECT`, whose definition is *"the syntax tree literally
contains this relationship."* There is no syntax tree. This is the same defect class Slice 2a
already corrected once, when resolved imports were found mislabelled `AST_DIRECT`.

It is **pre-existing** — Slice 1 emitted it — and 5a neither caused it nor worsened it. It only made
it visible, because a documentation-only repository now indexes. Raised as **Slice 5c**, not folded
into this review, because the honest fix is a vocabulary addition plus a UI gloss, which is its own
unit of work.

## Known limitations

- **The UI has no gloss for `document` or `section`.** `kindGloss` falls back to "This build has no
  description for that entity kind", which is honest but not finished. Deliberately deferred to 5b,
  the document UI slice, rather than paying an asset rebuild in a non-UI slice.
- Resource-bound counters appear in `nerve index` (text and `--json`) but not `nerve status`, which
  reads only the graph tables; storing them would require a schema change.
- No document→code link exists yet. That is 5b, by design.
- A raw `0x1f` cannot be committed through the implementer's file tools, so `fixtures/md-docs/docs/hostile.md`
  stores control bytes as escapes and the harness substitutes real bytes before indexing. A separate
  test asserts the substitution happened, so the forgery test cannot pass vacuously.
</content>
