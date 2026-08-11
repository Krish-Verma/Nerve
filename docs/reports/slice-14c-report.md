# Slice 14c — memory on HTTP and MCP, read-only, and T13

**Objective.** Surface 14b's memory read model on the two agent-facing surfaces without giving
either of them a way to write, and write down the boundary's limit rather than implying a control.

**Starting HEAD:** `e82915b` · **Ending HEAD:** `6a56ee7`

---

## What shipped

| | |
|---|---|
| **HTTP** | `GET /api/memory` (filters `scope`, `status`, `subject`, `q`, `limit`, `offset`) and `GET /api/memory/record?memory_id=…`. `ROUTES` 21 → 23 |
| **MCP** | `nerve_memory`. `TOOL_NAMES` 7 → 8 |
| **Threat model** | **T13**, with its residual stated as unmitigable |
| **Acceptance** | §4g, four checks. 83 → **87** |

Three `result_kind` values, not two: `no_memory_recorded` (nothing was ever written) and
`no_memory_matches` (filters accepted, nothing matched) are **different absences**, and collapsing
them would make "there are no notes" and "your filter excluded them" the same sentence.

An unknown `scope` or `status` is refused over HTTP **with the admitted set named**, never answered
with `[]` — and a derived view asked for as a status is refused *and named as one*
(`named_a_derived_view: true`).

---

## T13, and why it is a boundary rather than a check

Row 14's brief requires that agents may not confirm their own proposals. Nerve has no accounts, no
network and no identity provider, so at a local shell `nerve memory confirm <id>` run by a human and
run by an agent holding that shell are **byte-indistinguishable**: same executable, same uid, same
argv.

Nerve makes no attempt to separate them — no tty check, no parent-process inspection, no environment
sniffing — because each of those is a guess, and a guess presented as a control is worse than a
stated limit. That is ADR-0008's reasoning about `nerve affected`, applied in a new place.

So the control is a **surface boundary**, and **absence** is what makes it real: a gate can be
mis-configured; a code path that does not exist cannot be.

| control | test |
|---|---|
| no lifecycle writer anywhere in `crates/nerve-server/src/` | `layering.rs::no_memory_lifecycle_write_is_reachable_from_the_mcp_surface` |
| MCP connection `query_only`, session byte-identical | `mcp.rs::a_memory_session_leaves_the_database_byte_identical` |
| every write verb on a memory route → 405, database byte-identical | `memory.rs::no_memory_route_is_reachable_by_a_write_verb_and_the_database_never_changes` |

**The scan covers the whole crate, not only `src/mcp/`** — a writer added to `src/api/memory.rs` and
called from a tool would satisfy an MCP-only scan. And it enforces **eight** writers, not the seven
the brief listed: `insert_memory_citation` is the eighth, found by the implementer.

**The residual, unmitigable and written down:** a human who hands an agent their shell has removed
the boundary. Nerve cannot detect it, cannot report it afterwards, and does not claim to.
`author_label` is a label nothing verified; `status: active` means only that `nerve memory confirm`
ran on this machine. Nerve reports which *surface* wrote a record, never *who*.

---

## The MCP admission rule, applied in both directions

`nerve_memory` earns a tool of its own because:

1. **The output is not evidence.** Every other tool returns machine-derived evidence with a source
   type, directness and extractor id. §2 keeps a memory record out of `assertion_state` so a human
   sentence never becomes truth by arriving in the same table — folding it into
   `nerve_investigate`'s evidence packet undoes that decision one layer up.
2. **The input cannot be a selector.** The case 14a exists for is a note whose subject entity was
   pruned — which has **no live entity** for a selector-keyed tool to reach.

Applied honestly the other way: naming one record and filtering a list return the **same shape**, so
**no `question` enum was added** — it would advertise a mode switch that switches nothing, which is
exactly what 12c-iii-b refused for the seven history questions.

---

## Three plan items the repository refuted

1. **`GET /api/memory/<memory_id>` is not implementable here.** The router matches an exact table
   and `tests/api.rs` compares that table against the dispatch **in both directions by scraping
   string-literal arms**. A path parameter cannot be a table entry, so it would need a prefix arm
   the parity scrape cannot see — an unadvertised route, the exact hole that test exists to close. A
   `memory_id` is also caller-supplied text that may contain `/`. Shipped as
   `/api/memory/record?memory_id=`, consistent with `/api/history/commit?commit=`.
2. **Seven lifecycle writers is eight** (above).
3. **Acceptance §4e hardcoded `len(names) == 7`** for `tools/list`, which this slice's eighth tool
   broke. Bumped, with a comment that the number is `mcp::TOOL_NAMES.len()` — which a shell script
   cannot read.

---

## Verification

```
cargo fmt --all -- --check                            → clean
cargo clippy --workspace --all-targets -- -D warnings → 0 warnings
cargo test --workspace --no-fail-fast                 → 1758 passed, 0 failed, 2 ignored (60 targets)
scripts/final_acceptance.sh                           → 87 passed, 0 failed, 0 skipped
Cargo.lock                                            → 106 packages, unchanged
```

Tests 1730 → **1758**. Acceptance 83 → **87**.

**T7 anti-vacuity:** six hostile fields a human types freely (content, author label, claim key, event
note, invalidation reason, subject path). **33 labelled occurrences, 0 outside**, with the floor
verified by temporarily raising it to 100 000 and reading the failure, and each field re-checked
individually so one loud field cannot carry the total.

**Cross-surface agreement** asserted twice: a test scrapes the CLI's own JSON-emitting functions for
their key sets (≥5-key anti-vacuity floor each) and asserts set equality against the served record;
and acceptance §4g compares `nerve memory list --json` against `GET /api/memory` **field for field**
keyed by `memory_id`, with two different real statuses in the fixture so it cannot pass on nulls.

### The mutation probe, and the first attempt being wrong

Appending `nerve_store::confirm_memory` to the **end** of `mcp/memory.rs` left the scan green. That
was **not** a vacuous guard but a misplaced probe: `product_code()` truncates each file at its first
`#[cfg(test)]`, so the scan deliberately covers product code only, and the appended line landed in
the excluded region. Re-run with the reference inserted **before** that marker, it fails by name:

```
nerve-server/mcp/memory.rs reaches "confirm_memory"; the MCP surface must not be able to
write a memory record, and the boundary is that the path is absent rather than gated
```

Recorded because a probe that passes for the wrong reason is how a never-firing test gets trusted —
which this project has already been caught by once, in Slice 7b.

---

## Carried defects

- `crates/nerve-cli/src/main.rs:340` still describes `nerve mcp` as *"One tool,
  `nerve_investigate`"* — stale since 8b-ii, now eight tools. Pre-existing, out of scope here.
- `nerve affected` and `nerve trace-tests` are still held refused **only** by the acceptance script
  (found in 14b-ii). Unchanged by this slice.

**Next:** Slice 14d — the functional reference UI, which closes row 14.
