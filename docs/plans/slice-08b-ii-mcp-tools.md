# Slice 8b-ii — the rest of the MCP tool surface

**Status:** planned 2026-08-02. Follows Slice 8b-i (`124747b`).
**Gates:** T7 and T8 — already built and tested by 8a; each new tool must be shown to stay inside.

---

## The test each candidate had to pass

From `docs/plans/slice-08-mcp.md`: a tool earns its place only by having a **materially different
input/output contract**. *"Anything that is `investigate` with a flag is not a new tool."*

| candidate | input | output | verdict |
|---|---|---|---|
| `search` | a **query string**, not a selector | ranked hits with BM25 scores; no evidence, no assertions | **admit** |
| `path` | **two** selectors | a *sequence* of edges — an ordered walk, not a set of assertions | **admit** |
| `impact` | one selector | depth-ordered reverse closure, tallies, and an **unresolved account** | **admit** |
| `gaps` | **no selector at all** | coverage state per symbol, with a four-valued verdict and `totals: null` when nothing was ingested | **admit** |
| document / ADR evidence | a selector | the entity and its assertions | **DROP** |

**Document evidence is dropped, and 8b-i is why.** Row 8b listed it as a candidate because
`docs/foo.md` used to resolve to nothing. It now resolves, and `adr:ADR-0001` reaches an ADR by its
identifier — verified against the release binary during 8b-i. `nerve_investigate` already answers
*"what does Nerve know about this document, and why"*: the `Document`, its `CONTAINS` sections,
`SUPERSEDES` edges, `REFERENCES` into code, ADR status in the observation details. A separate tool
would be `investigate` with a kind filter, which is exactly what the rule forbids.

`path` deserves scrutiny because `nerve_investigate` already takes an `object`. They are not the
same question: with `object` it returns **assertions directly between two entities**; `path` returns
an **ordered chain through intermediates**. Different shape, different bound (path count and length,
not row count). Admitted.

**Five tools total.** That is the "small, coherent surface" the assignment asks for, and each one
maps to a command a user already has.

---

## What every tool inherits, and must be shown to inherit

8a built the envelope; this slice puts four more things inside it. **Nothing below is new work
per tool — it is a checklist each tool must be tested against:**

- every repository-derived value inside `repository_content`, nothing beside it but Nerve's own
  vocabulary, integers, and the caller's echoed `query`;
- the T7 property test — walk the whole response, assert no string inside the field appears outside
  it — **extended to cover every new tool**, not left asserting only `investigate`;
- arguments validated and bounded before use; selectors through `nerve_store::selector_shape`, so
  a traversal is `refused` and never "not found";
- a row cap, and the **128 KiB ceiling measured on the pretty-printed text a client reads**;
- exact continuation: `next_offset` stays correct because truncation cuts from the end;
- errors inside the tool-result envelope, per 8a's reasoning that a store error can quote a path;
- read-only; database byte-identical after a session.

### The bound each tool needs that a row cap does not give

- **`search`** — a query string reaching FTS5. `search_entities` already parameterises and
  tokenises it (`select.rs` header records why), but the **query length** must be capped like a
  selector, and the hit list capped like rows.
- **`path`** — cost is paths × length, not rows. Both `max_depth` and path count must be capped and
  echoed. `nerve path` already has a `truncated` flag; it must be surfaced, not swallowed.
- **`impact`** — the `unresolved` account is **not optional and not omitted when zero**. Slice 7b's
  finding stands: *3 dependants beside 4 unresolved sites*. A tool that reports the closure without
  the account tells an agent it is safe to change something on evidence that does not support it.
- **`gaps`** — `totals: null` means *no coverage was ever ingested*, which is not *"zero gaps"*.
  Slice 7a made this a distinct state; the MCP shape must keep them distinguishable, and the tool
  description must say so, because an agent that reads `null` as `0` reports a repository as fully
  covered when nothing was measured.

---

## Non-goals

- **No new store or API function.** Every tool calls what the CLI and HTTP already call. If a tool
  needs logic that does not exist, that is a finding to report, not code to add here.
- **No new dependency, no schema change, no migration.**
- **No `apps/nerve-web/` change.**
- **No history, cross-repository or memory tools** — Slices 12, 13, 14. They cannot be exposed
  before they exist.
- **No selector-layer change.** 8b-i settled it.

---

## Acceptance criteria

1. `tools/list` returns **five** tools; each descriptor states its bounds and that
   `repository_content` is untrusted.
2. Each tool answers over real stdio against a real client transcript.
3. `search` — a query string; hits bounded; an FTS5 operator-laden query (`a OR b NEAR/3 "c"`,
   `*`, `""`) is answered or refused, never panics.
4. `path` — two selectors; path count and depth bounded and echoed; "no path" is an explicit
   answer with exit-equivalent success, not an error.
5. `impact` — the unresolved account present **and rendered when zero**; a test asserts the field
   exists on a subject with no unresolved sites.
6. `gaps` — no selector; "no coverage ingested" is distinguishable from "no gaps" in the response
   **and** a test asserts the two differ.
7. A traversal selector is refused by **every** tool that takes one.
8. The T7 property test covers all five tools; a hostile heading round-trips as labelled data
   through each.
9. Response byte ceiling enforced per tool, with exact continuation.
10. Database byte-identical after a session exercising all five.
11. Mutation probes, each shown to fail its intended test for the intended reason:
    - omit the unresolved account from `impact` → a test fails;
    - render `gaps`'s absent state as zero → a test fails;
    - let one new tool's output escape `repository_content` → the T7 property test fails **for that
      tool**, proving the test was extended rather than left asserting `investigate` only;
    - remove a new tool's row cap → a bound test fails.

Probe 3 is the one that matters: a T7 test that still only walks `investigate`'s response would
pass while a new tool leaked.

## Verification gate

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace --no-fail-fast
cargo build --release
```
