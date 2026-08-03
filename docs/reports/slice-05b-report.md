# Slice 5b — the Markdown link scanner · completion report

**Date:** 2026-08-01 · **Status:** Complete · **Plan:** `docs/plans/slice-05-document-evidence.md`

---

## Summary

The scanner now records **link destinations**, exactly as written, with the syntax form that
carried them and a span pointing at the link rather than the line. It does not normalize, resolve,
stat, open or fetch anything — resolution is the next slice. Bare inline-code identifiers are
**counted, never emitted**: `` `parseConfig` `` in prose is not evidence that the document means
the symbol `parseConfig`.

`UnresolvedCategory::DocumentLink` and `DocumentSupersedes` are declared and emitted by nothing,
the same way `Relation::Supersedes` was declared in 5a.

## A decomposition error, and how it was handled

Slice 5b as originally dispatched bundled the scanner, a resolver, a precision harness, an
invalidation extension and an equivalence extension. **The implementation agent stalled at the
600 s watchdog** — the second time this project has hit that failure on an oversized slice, and
`docs/CONTINUATION.md` had explicitly warned about it. **That is an orchestrator error, recorded
here rather than attributed to the agent.**

The partial work was inspected rather than discarded: it built, clippy was clean (so nothing was
dead code), the full suite was green at 506, and `scan_line_links` was already wired into `scan`.
It was a coherent, self-contained unit one step short of tested. The agent was resumed with a
deliberately narrowed instruction — *test what you built, then stop* — which completed. The
resolver moves to its own slice.

## The tests found six real defects — this was not a formality

The first test run was **57 passed / 6 failed**, and every failure was the scanner:

| # | Defect | Fix |
|---|---|---|
| 1 | `[t](<./a file.md>)` produced nothing — `angle_destination` refused whitespace | CommonMark treats a *bracketed destination* (may contain spaces — the only way to write a path with one) differently from an *autolink* (may not). One function was doing both jobs; split with an `allow_space` flag |
| 2 | **`</div>` was recorded as a link to `/div`** | A leading `/` was accepted as "root-relative". `</div>` is the closing half of every HTML tag pair and is indistinguishable from `</src/a.ts>` without knowing whether `div` names a directory. Angle brackets now accept only a scheme URI or an explicit `./`/`../`. **Ambiguity is a refusal** |
| 3 | **`<script>alert(1)</script>` produced a link** | Same root cause, same fix. This shape is committed in `fixtures/md-docs/docs/hostile.md`, so it is now pinned by a test |
| 4 | `[![img](./i.png)](./a.md)` silently dropped the inner destination | Link text is not descended into — recursing into attacker-controlled nesting makes ten thousand nested brackets a stack overflow. Now **counted** under `link-in-link-text`, on the same principle as `heading-in-block-quote`: report the number rather than let the reader assume there was nothing there |
| 5–6 | Two further consequences of (1) | — |

Defects 2 and 3 are the ones that matter: an HTML fragment in a hostile document was being turned
into a link destination. Without tests, they would have reached the resolver.

## Verification — run by the orchestrator

```
cargo fmt --all -- --check                              → clean
cargo clippy --workspace --all-targets -- -D warnings   → 0 warnings
cargo test --workspace                                  → 541 passed, 0 failed, 2 ignored
```

506 → 541 (+35). No test deleted, weakened or `#[ignore]`d. Two files changed; the only six deleted
lines are the `UnresolvedCategory::ALL` widening and a helper refactor.

### Adversarial probe — run by the orchestrator

Fenced-code suppression is the single most important negative rule: a link in a code fence is an
example, not a claim. `scan_line_links` was injected into the in-fence branch of `scan`:

```
a_link_inside_a_fenced_code_block_produces_nothing ......... FAILED
code_span_mentions_follow_the_same_block_rules_as_links .... FAILED
```

Reverted, verified **byte-identical by sha256**, suite re-run green at 222 lib tests.

## Handoff notes the implementer raised, and the decisions they force

These are real and belong to the resolution slice:

1. **A control byte in a destination is carried through, not truncated.** Deliberate: the path
   guard is where a hostile path must be refused, and a refusal the guard never sees is one nobody
   reports. **The resolver must route every destination through `canonical_child`** — otherwise
   this becomes a hole rather than a handoff. Given Slice 5a's finding that a `0x1f` in a path can
   forge identity, this is load-bearing.
2. **Percent-encoding is not decoded**, so `./my%20file.md` reads as broken. Decoding it would let
   `%1f` smuggle a separator past the scanner into the guard. Needs an explicit decision.
3. **Inline code spans are matched within a line only** — a span opened on one line and closed on
   the next will not hide a link on the first. Bounded, but real.
4. **`[id]: dest` is single-line only**; CommonMark allows the destination on the next line.
5. **`is_bare_identifier` is narrow on purpose** — `a.b` and `x/y` are not counted. The count is
   informational, so under-counting is the safe direction, but "code mentions" really means "bare
   identifiers".

## Not implemented, by design

Resolution, `Section REFERENCES`, `Document SUPERSEDES Document`, unresolved entities, the
precision harness and invalidation. All of that is Slice 5c.
