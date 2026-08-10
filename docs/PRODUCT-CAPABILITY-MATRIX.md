# Nerve — product capability matrix

**Written 2026-08-08, at the close of row 12.** One row per accepted capability, mapped to the
application service that owns it and to every surface that must agree with it.

This document exists because "the UI is behind" is not a finding a reader can act on. A gap is only
useful when it names the capability, the surface, and whether the absence is a **refusal** (decided,
with a reason) or a **gap** (not built yet). Those two are never merged here.

**Read with:** `docs/UI-BACKEND-HANDOFF.md` (the per-entry contract each surface must honour),
`docs/UI-FEATURE-SPEC.md` (the redesign handoff), and `docs/plans/ui-parity-matrix.md` (the
measured per-view audit).

---

## Legend

| mark | meaning |
|---|---|
| ✅ | present and exercised by a test |
| ➖ | **intentional refusal** — decided, with a recorded reason. Not a gap. |
| ⬜ | **gap** — the capability exists elsewhere and this surface does not carry it yet |
| n/a | the surface has no sensible form of this capability |

Surfaces, in the order a fact travels: **Svc** = shared application service in `nerve-store` /
`nerve-index` · **CLI** = `nerve …` human output · **JSON** = `--json` · **HTTP** = `/api/*` ·
**MCP** = a `nerve_*` tool · **UI** = `apps/nerve-web` · **Acc** = exercised by
`scripts/final_acceptance.sh` behaviourally (not by a command-exists check).

---

## 1. Indexing and the evidence graph

| capability | Svc | CLI | JSON | HTTP | MCP | UI | Acc |
|---|---|---|---|---|---|---|---|
| Index a repository (TS/JS, Python, Markdown) | ✅ | `index` | ✅ | ➖ write | ➖ write | ➖ read-only surface | ✅ |
| Incremental re-index equals a full index | ✅ | `index` | ✅ | n/a | n/a | n/a | ✅ |
| Index counts, freshness, schema version | ✅ | `status` | ✅ | `/api/overview` | ✅ `nerve_investigate` | Overview | ✅ |
| Trust verdict with an exit code | ✅ | `check` | ✅ | ⬜ | ⬜ | ⬜ | ✅ |
| Installation / database / config diagnosis | ✅ | `doctor` | ✅ | n/a | n/a | n/a | ✅ |
| Full-text search over names and scope paths | ✅ | `search` | ✅ | `/api/search` | ✅ `nerve_search` | Search | ✅ |
| Entity detail, occurrences, neighbourhood | ✅ | n/a | ✅ | `/api/entity`, `/api/neighbourhood` | ✅ | Entity, Graph | ✅ |
| Source text of an occurrence | ✅ | n/a | n/a | `/api/source` | ➖ | Source | ✅ |
| Evidence behind a relationship | ✅ | `why` | ✅ | `/api/why` | ✅ `nerve_investigate` | Evidence | ✅ |
| Path between two entities | ✅ | `path` | ✅ | `/api/path` | ✅ `nerve_path` | Path | ✅ |
| Unresolved references, categorised | ✅ | n/a | ✅ | `/api/unresolved` | ✅ | Unresolved | ✅ |
| Partial parses | ✅ | n/a | ✅ | `/api/partial-parses` | ✅ | Unresolved | ✅ |
| Selector alternatives when a selector is ambiguous | ✅ | ✅ | ✅ | ✅ | ✅ | ⬜ **gap** | ✅ |

## 2. Coverage and test evidence

| capability | Svc | CLI | JSON | HTTP | MCP | UI | Acc |
|---|---|---|---|---|---|---|---|
| Ingest one LCOV report | ✅ | `coverage` | ✅ | ➖ write | ➖ write | ➖ | ✅ |
| Symbols no test is known to touch | ✅ | `gaps` | ✅ | `/api/gaps` | ✅ `nerve_gaps` | Coverage | ✅ |
| `COVERS` is never relabelled a call | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ |
| Per-test attribution from LCOV | ➖ | ➖ | ➖ | ➖ | ➖ | ➖ | ➖ |

**The refusal:** LCOV carries no per-test attribution, so `nerve affected` was refused rather than
shipped with the attribution guessed (ADR-0008 §A.2). `COVERS` comes from a `CoverageRun`, never
from a test (ADR-0005).

## 3. Trace / test-observed calls

| capability | Svc | CLI | JSON | HTTP | MCP | UI | Acc |
|---|---|---|---|---|---|---|---|
| Import a trace artifact your own tracer produced | ✅ | `trace` | ✅ | ➖ write | ➖ write | ➖ | ✅ |
| `TEST_OBSERVED_CALL` kept distinct from `CALLS` | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ |
| Nerve runs your tests for you | ➖ | ➖ | ➖ | ➖ | ➖ | ➖ | ➖ |
| Trace evidence surfaced as its own view | ✅ | ✅ | ✅ | ✅ | ✅ | ⬜ **gap** | ✅ |

**The refusal:** executing repository code is outside the product (`no_subprocess`). A runtime
observation is never presented as universal behaviour.

## 4. Impact

| capability | Svc | CLI | JSON | HTTP | MCP | UI | Acc |
|---|---|---|---|---|---|---|---|
| What depends on a symbol, transitively | ✅ | `impact` | ✅ | `/api/impact` | ✅ `nerve_impact` | ⬜ **gap** | ✅ |
| What the answer **cannot** see, stated | ✅ | ✅ | ✅ | ✅ | ✅ | ⬜ **gap** | ✅ |

## 5. History (row 12, complete)

| capability | Svc | CLI | JSON | HTTP | MCP | UI | Acc |
|---|---|---|---|---|---|---|---|
| Walk and record the commit graph | ✅ | `history sync` | ✅ | ➖ write | ➖ write | ➖ | ✅ |
| Recorded commits, newest first | ✅ | `history log` | ✅ | `/api/history/commits` | ✅ | History | ✅ |
| One path's commits and rename hypotheses | ✅ | `history file` | ✅ | `/api/history/path` | ✅ | HistoryPath | ✅ |
| State-to-state diff, by ancestry never by time | ✅ | `history diff` | ✅ | `/api/history/diff` | ✅ | HistoryDiff | ✅ |
| Change frequency, stated as a floor | ✅ | `history frequency` | ✅ | `/api/history/frequency` | ✅ | History | ✅ |
| Co-change, labelled **not a dependency** | ✅ | `history cochange` | ✅ | `/api/history/cochange` | ✅ | History | ✅ |
| What visible history is unavailable | ✅ | `history availability` | ✅ | `/api/history` | ✅ | History | ✅ |
| Creation claimed only when evidence licenses it | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ |
| Exact-content rename hypotheses | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ |
| **Similarity** rename hypotheses, with method, version, measurement, threshold, completeness | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ |
| Per-record summary truncation | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ |
| Symbol-level history | ➖ | ➖ | ➖ | ➖ | ➖ | ➖ | ✅ (the refusal is tested) |
| Relationship appearance / disappearance over time | ➖ | ➖ | ➖ | ➖ | ➖ | ➖ | — |

**The refusals.** `git_change` is path-keyed and a symbol has `PathRole::None`, so a symbol selector
is refused **as a refusal** and the containing path is never substituted. Relationship history would
attribute today's edges to yesterday's commit, so it needs a historical graph that does not exist.

## 6. Cross-repository contracts (row 13 — 13a–13d complete)

| capability | Svc | CLI | JSON | HTTP | MCP | UI | Acc |
|---|---|---|---|---|---|---|---|
| Registry of neighbouring repositories | ✅ | `repo add/list/remove/relocate` | ✅ | `/api/contracts/registry` | ✅ `nerve_contracts` | Contracts | ✅ |
| Registration is explicit; nothing auto-discovered | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ |
| Target opened read-only, byte-identical after reads | ✅ | ✅ | n/a | ✅ | ✅ | n/a | ✅ |
| C1 npm local/workspace dependency | ✅ | `repo scan` | ✅ | `/api/contracts` | ✅ | Contracts | ✅ |
| C3 Python path dependency | ✅ | `repo scan` | ✅ | `/api/contracts` | ✅ | Contracts | ✅ |
| C2 npm export resolution to a target **file entity** | ✅ | `repo scan` | ✅ | `/api/contracts` | ✅ | Contracts | ✅ |
| **Is this link still current?** (link freshness) | ✅ | ⬜ **gap — 13d-ii** | ⬜ | ✅ | ✅ | ✅ | ✅ |
| Unsupported form recorded with its form named | ✅ | ✅ (tally) | ✅ | ➖ names vocabulary only | ➖ | ➖ | ✅ |
| Auto-discovery of sibling checkouts | ➖ | ➖ | ➖ | ➖ | ➖ | ➖ | ➖ |
| Fetching a package or registry from the network | ➖ | ➖ | ➖ | ➖ | ➖ | ➖ | ➖ |
| A cross-repository link as a **local** assertion | ➖ | ➖ | ➖ | ➖ | ➖ | ➖ | ➖ |
| Local `path`/`impact` traversing a contract link | ➖ | ➖ | ➖ | ➖ | ➖ | ➖ | ✅ (asserted negatively) |
| `contract_version_mismatch` verdict | ➖ | ➖ | ➖ | ➖ | ➖ | ➖ | ➖ |

**Two refusals with measured reasons.** `contract_version_mismatch` needs range satisfaction
(`^1.2.0` against `1.2.3`), which is a semver resolver and therefore a new dependency; both versions
are recorded and no verdict is derived. And a cross-repository link cannot be a local assertion
because `assertion.target_entity_id` is a hard foreign key into the local `entity` table.

**The one gap is a surface disagreement**, which is what this document exists to surface: after 13d,
HTTP, MCP and the UI can answer *"is this link still current?"* and the CLI cannot — `repo scan`
reports what a scan just wrote and carries no freshness field. `nerve repo links` closes it.

**The refusal that shapes the row:** `assertion.target_entity_id` is `NOT NULL REFERENCES
entity(entity_id)` (`schema.rs:97`, immutable since v1) with `foreign_keys=ON`, and a
cross-repository target has no row in the local `entity` table. Links live in `contract_link`, and
ordinary `path`/`impact` queries must not traverse them.

## 7. Human-confirmed memory (row 14 — not started)

| capability | Svc | CLI | JSON | HTTP | MCP | UI | Acc |
|---|---|---|---|---|---|---|---|
| Propose / confirm / supersede / invalidate | ⬜ | ⬜ | ⬜ | ➖ read-only | ➖ read-only | ➖ read-only | ⬜ |
| Read memory | ⬜ | ⬜ | ⬜ | ⬜ | ⬜ | ⬜ | ⬜ |
| Deterministic local export | ⬜ | ⬜ | ⬜ | n/a | n/a | n/a | ⬜ |
| A delete verb | ➖ | ➖ | ➖ | ➖ | ➖ | ➖ | ➖ |
| Memory inside `assertion_state` | ➖ | ➖ | ➖ | ➖ | ➖ | ➖ | ➖ |

**The refusals:** a delete verb is how "history preserved" stops being true. Memory is a statement
about one subject, not a relation between two, and `assertion_state` is defined as a pure function
of machine observations.

---

## 8. Cross-cutting guarantees every surface owes

| guarantee | where it is enforced |
|---|---|
| Offline: no network on any path | `no_network.rs`, acceptance |
| No repository code executed | `no_subprocess.rs`, acceptance |
| HTTP is read-only and loopback-only | sha256 before/after, `POST` → 405, `layering.rs` |
| MCP leaves the database byte-identical | byte-hash test, proved non-vacuous by probe |
| Repository text stays inside `repository_content` | walk-the-whole-response property test |
| Bounded output with stated truncation | per-surface bounds blocks |
| One vocabulary source; no independent frontend gloss map | `ui_vocabulary.rs`, and the on-exactly-one-list test |
| A note's prose has exactly one owner | `history_wording.rs` byte-level source scan |
| No generic confidence float | ADR-0003; v7's `CHECK` makes blending a constraint violation |

---

## 9. Known gaps, collected

These are **gaps, not refusals**, and they are the functional UI parity phase's worklist:

1. **Impact** has no UI view.
2. **Selector alternatives** are not rendered when a selector is ambiguous.
3. **Trace / test-observed-call evidence** has no dedicated UI surface.
4. **`check`'s trust verdict** is not on HTTP, MCP or the UI.
5. **Keyboard navigation** — tested 2026-08-08 with `scripts/viewport_qa.mjs`'s sibling keyboard
   probe (real `Input.dispatchKeyEvent` Tab presses, so what is recorded is what a keyboard user
   gets, not a model of focus order). Mostly clean: **no positive `tabindex`**, no unnamed buttons,
   no images without `alt`, no offscreen focus stops, and a logical order — logo → search → rail →
   tabs → path field → content. **One measured defect:** the two text inputs have no focus ring.
   `:focus-visible` sets `outline: 1px solid var(--bone)` globally, but `.field > input` sets
   `outline: none` (`nerve.css:382`), leaving only `.field:focus-within`'s border-colour change —
   measured `rgb(46,40,34)` → `rgb(87,79,72)`, a **1.81:1** contrast between states against
   WCAG 2.4.11's **3:1** minimum. Fix: `.field > input:focus-visible` restoring the global outline.
6. **Corrupt-history UI** — exercised 2026-08-08 against `fixtures/history-missing` (a parent
   commit object deleted with **no** `shallow` file, so the hole cannot be reported as a declared
   truncation), served by the release binary and driven at 1600px and 380px. **Clean, and the
   invariant holds**: the ingest stops at `missing_object` — *"a fault in this repository, not a
   declared boundary"* — the affected commit renders `parents_missing` and *"earliest visible in
   this checkout"*, its enumeration reads *"the parent tree could not be read, so nothing was
   enumerated — not an empty commit"*, and the phrase *"begins here"* appears nowhere. No
   horizontal overflow, no console messages, no exceptions, no 4xx at either width. `summary
   complete` renders on every commit, so §6.7 holds on this path too. **No gap remains here.**
7. Rows 13 and 14 are unbuilt in their entirety.

Gaps 5 and 6 are the residue of row 12's browser QA; everything needed to close 5 exists now that
`scripts/viewport_qa.mjs` drives a real viewport over CDP.
