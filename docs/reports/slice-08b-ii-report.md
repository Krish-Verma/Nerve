# Slice 8b-ii — the rest of the MCP tool surface

2026-08-02. Plan: `docs/plans/slice-08b-ii-mcp-tools.md`. Follows Slice 8b-i (`124747b`).
**Row 8 is now complete.**

---

## Objective

Four more MCP tools — `search`, `path`, `impact`, `gaps` — inside the T7/T8 envelope Slice 8a
built. Five in total.

## One candidate dropped, and 8b-i is why

Row 8b listed document/ADR evidence as a fifth candidate. It was dropped at planning time:
`docs/foo.md` used to resolve to nothing, which is what made it look like a separate capability.
8b-i fixed that, and `adr:ADR-0001` reaches an ADR by its identifier. `nerve_investigate` already
answers *"what does Nerve know about this document, and why"* — the `Document`, its `CONTAINS`
sections, `SUPERSEDES` edges, `REFERENCES` into code, ADR status in the observation details. A
document tool would be `investigate` with a kind filter, which is what the
materially-different-contract rule exists to forbid.

## Each tool's contract, and the bound a row cap does not give

| tool | why it is not `investigate` with a flag | its own bound |
|---|---|---|
| `search` | input is a **query string**, not a selector; output carries `score` and **no assertions** | cost grows with token count, not with what is named — `MAX_QUERY_BYTES = 512`, an eighth of a selector's |
| `path` | **two** selectors; output is an *ordered chain* — a test asserts `hops[i].to == hops[i+1].from`. `investigate` with `object` returns a **set of direct assertions**, not a walk | paths × length: `max_depth` **and** path count capped and echoed |
| `impact` | one selector; depth-ordered reverse closure with tallies | the **unresolved account**, present on every answer including the all-zero case |
| `gaps` | **no selector at all** | `totals: null` ≠ `gaps: 0` — three independent signals keep them apart |

## The refactor that made it safe

`crates/nerve-server/src/mcp/tool.rs` — the envelope, trust block, byte ceiling and **all**
argument validation, lifted out of `investigate.rs`. Four copies of a security property is four
places for it to rot. `envelope()` is the only way a tool builds a result, so a tool cannot ship a
payload without the label. **Every 8a test was kept verbatim and still passes.**

## Absence is not zero, twice more

- **`impact`** — the unresolved account is serialized even when every count is zero. Verified live:
  `src/math.ts#add` on `ts-coverage` returns `{sites: 0, assertions: 0, targets: 0,
  by_category: {}}` rather than omitting the field.
- **`gaps`** — verified live on two copies of the same fixture, one with coverage ingested and one
  without:

```
NO coverage:        state=coverage_absent  answerable=false  totals=null
coverage ingested:  state=gaps_present     answerable=true   totals={covered:4, gaps:2, …}
```

Three independent signals, plus prose: *"`totals` is null because no coverage report has ever been
ingested into this index. Null is not zero."* An agent that reads `null` as `0` would report an
unmeasured repository as fully covered.

`totals` sits under `evidence`, **not** under `repository_content` — correctly: counts are Nerve's
own integers, not repository text. (My first probe read the wrong path and briefly looked like a
contradiction; the implementation was right and the probe was wrong.)

## Files changed

**New** — `crates/nerve-server/src/mcp/`: `tool.rs`, `search.rs`, `path.rs`, `impact.rs`, `gaps.rs`.

**Modified** — `src/mcp.rs` (closed-table dispatch, `TOOL_NAMES`, `descriptors()`),
`src/mcp/investigate.rs` (consumes `tool::*`), `tests/layering.rs` (SOURCES 11 → 16),
`tests/mcp.rs`, `nerve-cli/tests/cli.rs`, `nerve-cli/tests/no_subprocess.rs`.

**Untouched, and verified so:** `nerve-store`, `nerve-core`, `nerve-index`, `api.rs`, `Cargo.lock`,
every `Cargo.toml`, `third_party/`, `apps/`, `fixtures/`. The tools are pure surface over the
application layer the CLI and HTTP already call — no new store or API function, exactly as the plan
required.

## Tests

**970 passed / 0 failed / 2 ignored**, up from 911. **+59.**

## Verification

```
cargo fmt --all -- --check                             → clean, exit 0
cargo clippy --workspace --all-targets -- -D warnings  → 0 warnings, exit 0
cargo test --workspace --no-fail-fast                  → 970 passed, 0 failed, 2 ignored
cargo build --release                                  → Finished, exit 0
```

**100 crates**, unchanged.

### Orchestrator adversarial session, release binary, real stdio

11 messages including an SQL-injection string, FTS5 operators, traversal at three arguments, a
negative limit, an unknown tool, an undeclared argument and malformed JSON:

```
exit=0 · 10 stdout lines for 11 messages (notification unanswered) · 0 bytes stderr
database SHA-256 identical before and after
tools/list → nerve_investigate, nerve_search, nerve_path, nerve_impact, nerve_gaps

'; DROP TABLE entity; --      answered, no panic          a OR b NEAR/3 "c" * ^   answered, no panic
path from ../../etc/passwd    -32602 selector is refused  gaps under ../../etc    -32602 refused
gaps under ..\..\etc          -32602 refused              gaps under /etc         -32602 refused
impact limit: -5              -32602 non-negative integer unknown tool            -32602
5000-byte query               -32602 longer than maximum  bogus argument          -32602 unknown
```

### Mutation probes — the implementer's four, and a fifth of mine

| probe | failures | note |
|---|---|---|
| omit `impact`'s unresolved account | 4 / 3 targets | incl. the explicit zero-case test |
| render `gaps`'s absent tally as zeroes | 2 | `left: {gaps:0,…} right: Null` |
| leak `search`'s top hit into `evidence` | T7 property test | **counterfactual run recorded**: under the same mutation the investigate-only spot check still *passed*, so the failure came from the extension, not from an inherited assertion |
| remove `impact`'s row cap | 2 | `left: 100000 right: 100` |
| **mine — disable the shared traversal refusal for all tools** | **7 / 3 targets** | one test *per tool*: `mcp::gaps::…`, `mcp::impact::…`, `mcp::investigate::…`, `mcp::path::…`, plus `mcp::tool::…` and the integration test `a_traversal_selector_is_refused_by_every_tool_that_takes_one` |

My probe verifies criterion 7 is tested per tool rather than once. `tool.rs` restored
byte-identical afterwards; gate re-run at 970.

### The T7 test was extended, and it cannot pass vacuously

13 cases across all five tools plus a coverage-ingested hostile repository. The helper carries
**two anti-vacuity assertions** of its own:

```
"nothing was labelled, so the check is vacuous"
"hostile content never reached the answer, so the check is vacuous"
```

That closes the exact trap that produced a false pass twice on this project — in Slice 8a
(injection in a Markdown *body*, which never enters the graph) and again during 8b-i review (a
level-2 heading, which is contained by its parent section rather than by the document).

A separate test asserts `calls.len() == mcp::TOOL_NAMES.len()`, so a sixth tool cannot be added
without a bound case.

## Findings the implementer reported rather than hid

1. **Exact continuation is not available for the four new tools.** `search_entities`,
   `find_paths`, `ImpactQuery` and `GapQuery` have **no offset** — only `unresolved_entities` does.
   Rather than invent paging on this surface that the CLI and HTTP do not have, each tool ships
   `next_offset: null`, `continuable: false` and a statement saying so, and cuts from the end so
   exact totals stay exact. `search` reports `limit_reached` rather than a `truncated` it cannot
   compute — the store stops at `limit` and returns no total, so `truncated: false` would be a
   completeness claim the query cannot support. **Criterion 9 is therefore partially met and the
   gap is named.** Real continuation is its own slice across all three surfaces.

2. **Criterion 8 cannot literally hold for `nerve_gaps`.** A hostile heading becomes a `Section`;
   a gap row is only ever a symbol. No heading can reach `nerve_gaps` by construction. The hostile
   **file name** was used instead, which does reach a gap row's `file_path`. The property holds;
   the vector differs.

3. **The T7 walker does not scan JSON object *keys*.** Pre-existing in 8a's helper. **I verified
   this empirically** rather than accepting it: across live `impact`, `search` and `gaps`
   responses the only dynamic keys are `by_kind/method` and `by_relation/CALLS` — `EntityKind` and
   `Relation`, both closed compile-time vocabularies. Nothing leaks today. A future map keyed by
   repository-derived text (`by_file: {"src/app.ts": 3}`) would leak silently.

   The implementer's reason for keeping tallies as maps rather than arrays of `{kind, count}` is
   sound and worth preserving: as array *values*, `"method"` and `"CALLS"` would sit outside
   `repository_content` while also appearing inside it, which the property test would flag. The
   test is a deliberate over-approximation, and vocabulary-as-keys is what keeps it from
   false-positiving on Nerve's own words.

## Security / privacy / clean-room / dependency review

stdio only; no socket, no port, no outbound client. Read-only, database byte-identical after a
five-tool session. The T1 no-subprocess loop now drives **all five tools** against the hostile
repository. No LLM in the product path. No telemetry. No dependency. Independent implementation.

## Known limitations

- **No continuation cursor on the four new tools** (finding 1). Totals are exact and truncation is
  explicit, so a caller knows what it is missing — it just cannot ask for the next page.
- **The T7 property test does not scan object keys** (finding 3). Not exploitable today; verified.
- **`evidence.state` values (`present`/`absent`) would collide with a repository entity literally
  named `present`.** Pre-existing 8a design; not reachable on any fixture.
- **Refusal envelopes are bounded by `MAX_CANDIDATES = 25`, not by the byte ceiling** — 8a
  behaviour, deliberately preserved.
- **A fixture sharp edge worth knowing:** a file `src/hub.ts` exporting `hub()` makes
  `src/hub.ts#hub` **ambiguous** — the module and the function share a qualified name. Product
  behaviour is correct (both candidates listed, nothing chosen); the test fixture was renamed to
  `src/core.ts` to test other things. No committed fixture was changed.

## UI backend handoff changes

**None.** MCP is an agent surface. No endpoint, no view, no TypeScript.

## Commit

`b40458d` — *feat: Slice 8b-ii — four more MCP tools, and one that stopped being needed*.

## Next slice

**9 — Python.** `tree-sitter-python 0.25.0` (MIT) matches the workspace `tree-sitter = "0.25"`.
