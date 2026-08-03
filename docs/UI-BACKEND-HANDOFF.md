# Nerve — UI ↔ Backend Handoff

**Purpose.** From 2026-08-02 the visual interface is owned by the user and worked on separately.
Backend work continues. This file is the accumulating record of **every** frontend integration
requirement introduced after that freeze: new endpoints, new fields, changed fields, removed
fields, and the states a view must be able to render.

**What the backend has been doing to `apps/nerve-web/` in the meantime:** nothing discretionary.
The only edits permitted are minimal compatibility corrections needed to keep the repository
building and the embedded bundle honest — a type mirror, a field binding. No layout, no
typography, no new views, no visual polish. Each such edit is listed below with its reason so it
can be reviewed or reverted.

**Reading order for a UI session:** the newest entry is at the bottom. Anything above it that is
marked ✅ is already wired in the shipped bundle.

---

## Standing contract notes

These apply to every endpoint and are not repeated per entry.

| | |
|---|---|
| **Base** | `http://127.0.0.1:<port>` — loopback only, never a remote host |
| **Auth** | Every `/api/*` request needs the session token, as header `X-Nerve-Token: <token>` or query `?token=<token>`. The token is minted per `nerve serve` run and printed in its announcement (`--json` gives `{address, token}`). Static assets are exempt; `Host` and `Origin` are still enforced on them |
| **Errors** | `401` no token · `403` wrong token, bad `Origin`, bad `Host`, path outside root · `405` any method but `GET` · `404` unknown route. Bodies are JSON |
| **Method** | `GET` only. The API is read-only and proven so — the database is byte-identical before and after a UI session |
| **Absence vs zero** | Nerve distinguishes "no evidence" from "measured zero" throughout. A view must never render an absent measurement as `0`. Where an endpoint can return `null` for a total, that `null` means *unanswerable*, and the UI must say so in words |
| **Freshness** | Freshness is computed at query time by re-hashing the file, never stored. Where a response carries freshness, it is arithmetic the UI can show: recorded hash vs on-disk hash, plus a verdict |

---

## Entry 1 — `symbols_total` on `/api/overview`

**Slice 7a-iii** · corrective · backend commit: see `docs/reports/slice-07a-iii-report.md`
**Status: ✅ already wired by the backend** (one-line binding; see "Edits made" below).

### Why this exists

The navigation rail printed `entities_total` under the label **"Symbols"**. `entities_total`
counts every entity kind — repository, directory, file, module, document, section, unresolved
reference, and since Slice 6a `coverage_run` as well. On the coverage fixture the rail read **18**
while the Coverage view on the same screen correctly read **8 symbols in scope**. Two numbers for
one word, on one screen, and the larger one was wrong.

The defect is pre-existing from Slice 4b. It was made visible by the Slice 7a-ii Coverage view.

### The contract

```
GET /api/overview
```

Unchanged except for **one added field**. Nothing was renamed, changed or removed, so this is a
backward-compatible addition.

| field | type | meaning |
|---|---|---|
| `symbols_total` | `number` | **New.** Entities that are symbols: `function`, `method`, `class`, `interface`. Derived from `EntityKind::is_symbol()` in the Rust vocabulary |
| `entities_total` | `number` | Unchanged. Every entity kind, symbols included. Always `>= symbols_total` |

`symbols_total` is also now present, with identical semantics and the same value for the same
database, on:

- `nerve status --json` → `symbols_total`
- `nerve index --json` → `symbols_total`
- both commands' human-readable output, as a `symbols` line beneath the `entities` breakdown

A test asserts the CLI and the API report the same `symbols_total` for the same database.

### Example

`fixtures/ts-coverage`, after `nerve index` and `nerve coverage`:

```json
{
  "entities_total": 18,
  "symbols_total": 8,
  "entities_by_kind": {
    "class": 1, "coverage_run": 1, "directory": 1, "document": 1,
    "file": 3, "function": 3, "interface": 1, "method": 3,
    "module": 2, "section": 1
  }
}
```

`8 = 3 function + 3 method + 1 class + 1 interface`. The other 10 are not symbols.

### Display language

- Print `symbols_total` — never `entities_total` — beside the word **Symbols**.
- `entities_total` is correct beside the word **entities**. `Overview.tsx` already does this
  (`label="entities" … note="named things"`) and was deliberately **left untouched**.
- Do not label `entities_total` "objects", "nodes" or "items" and treat it as a symbol count in
  prose. It includes the repository itself and every directory.

### States

| state | response | render |
|---|---|---|
| never indexed | `symbols_total: 0`, `schema_version: null` | the existing empty state; do not say "0 symbols" as if measured |
| indexed, no symbols | `symbols_total: 0`, `entities_total > 0` | a real measured zero — a docs-only repository is the normal case |
| normal | `0 < symbols_total < entities_total` | the count |
| `symbols_total === entities_total` | — | **cannot occur** on any real repository; every repository has at least a `repository` entity. If you see it, the backend has regressed |

No pagination, no limits, no truncation — it is a scalar.

### Edits the backend made to `apps/nerve-web/`

Two files, four lines, no visual change:

1. `src/api/types.ts` — added `symbols_total: number` to the `Overview` interface, plus a one-line
   doc on it and on `entities_total` saying which is which. *Reason: the TS mirror must carry the
   field or the build fails.*
2. `src/App.tsx:144` — `count(data.entities_total)` → `count(data.symbols_total)`. *Reason: this
   one expression is the defect; leaving it would mean shipping a backend fix that changes
   nothing a user sees.*

`src/views/Overview.tsx:165` was **not** touched. Adding a symbols figure to the Overview view is
a product decision and is yours.

### Still outstanding for the UI (not backend work)

- **Narrow-viewport QA at 380px remains unverified.** Carried over from Slice 7a-ii: the browser
  window ignored programmatic resize during that session. The responsive CSS rules were confirmed
  present in the media-query block, but no screenshot at 380px exists. This is unrelated to
  `symbols_total` and is listed here only so it is not lost.

---

## Entry 2 — `/api/impact`, a new endpoint

**Slice 7b** · new capability · report: `docs/reports/slice-07b-report.md`
**Status: ⬜ no UI exists.** This is a new endpoint with no view. Building one is yours.

### What it answers

*"If I change this symbol, what else might break?"* — a reverse dependency closure. Everything
that reaches the subject through `CALLS`, `REFERENCES`, `EXTENDS` or `IMPLEMENTS`, transitively,
with the evidence for the edge that reached it.

### The contract

```
GET /api/impact?subject=<selector>
```

| parameter | type | default | notes |
|---|---|---|---|
| `subject` | selector string | **required** | entity id, `rel/path.ts`, `rel/path.ts#Name`, or a unique name. Missing → `400` |
| `max_depth` | int ≥ 1 | `6` | **clamped** to 32, and the applied value is echoed back in `max_depth` |
| `limit` | int ≥ 1 | `50` | **clamped** to 500, echoed back in `limit`. Caps rows only — every tally stays exact |
| `relation` | repeatable | the four above | from the closed vocabulary. Unknown → `400 unknown_relation` with the allowed list |

**Empty `relation` means the four defaults, NOT every relation.** This is the opposite of
`/api/path`, where empty means all. Following `CONTAINS` would answer that every symbol impacts
the repository.

### Response

| field | type | meaning |
|---|---|---|
| `subject` | entity | the symbol asked about. Never appears in `results` |
| `relations` | string[] | the relations actually walked |
| `max_depth`, `limit` | number | as applied after clamping |
| `totals.entities` | number | size of the **whole** closure, not the page |
| `totals.by_depth` | `{depth, entities}[] ` | **an array, not an object** — JSON object keys are strings and `"10"` would sort before `"2"` |
| `totals.by_relation`, `totals.by_kind` | `Record<string, number>` | exact tallies |
| `totals.stale` | number | reached through evidence that no longer matches its file |
| `unresolved` | object | **see below — this is the important one** |
| `count` / `results_total` / `truncated` | number / number / bool | page size vs closure size, and whether the cap cut |
| `files_probed` | number | files re-hashed to compute freshness |
| `results[]` | row[] | `entity`, `depth` (≥1), `relation`, `direction`, `reached_entity_id`, `assertion_id`, `status`, `strongest_source_type`, `observation_count`, `is_unresolved`, `file_path`, `start_line`, `evidence_freshness` |

Rows are sorted nearest-first, then by kind, then qualified name. Deterministic.

### `unresolved` — do not render this as a footnote

```jsonc
"unresolved": { "sites": 4, "assertions": 4, "targets": 4, "by_category": { "value": 4 } }
```

**It is always a present object, never `null`, never omitted, including when every count is zero.**

Nerve has no type inference, so a method call on a typed receiver (`shape.area()`) is recorded as
unresolved rather than guessed at. Slice 2a measured 38.1% of call sites on the resolution corpus
as unresolved. So a UI that shows *"3 things depend on this"* without the account beside it is
telling the user it is safe to change something, on evidence that does not support the claim.

- `sites` counts **observations** — individual reference sites. This is the number to show.
- `assertions` and `targets` are the same fact at coarser grain; `sites ≥ assertions` always.
- `by_category` splits by `UnresolvedCategory` — a broken Markdown link (`module`) and an
  unresolvable method call (`value`) are not the same warning.
- Scope is **repository-wide, restricted to the relations walked** — a hidden edge could attach
  anywhere, and narrowing it without name matching is not possible.

**Required display language.** When `sites > 0`, something equivalent to: *"N reference sites in
this repository resolved to nothing. Any of them could reach this symbol and this answer cannot
rule them out."* When `sites == 0`, say so explicitly — *"every reference site Nerve indexed under
these relations resolved, so no failed resolution is hiding a dependency from this answer"* — do
**not** hide the section. The zero case is the one where a silent omission most invites the wrong
conclusion.

**Do not** present unresolved sites as a list of suspect callers, and do not match their names
against the subject's. That is identity by coincidence; Nerve does not do it and the API gives you
no data to do it with.

### States

| state | response | render |
|---|---|---|
| nothing depends on it | `totals.entities: 0`, `results: []` | "Nothing in the index depends on this through …" — **plus the unresolved account**, which is exactly when it matters |
| truncated | `truncated: true`, `results_total > count` | show the page, say how many were cut, note the tallies are exact |
| stale evidence | row `evidence_freshness` ≠ `"fresh"`, `totals.stale > 0` | mark per row; the edge was recorded against a file that has since changed |
| unresolved edge | row `is_unresolved: true` | the edge itself points at something Nerve could not name |
| ambiguous subject | `409 ambiguous_selector` with `detail.candidates[]` | let the user pick. Nerve refuses to guess |
| unknown subject | `404 selector_not_found` with `detail.suggestions[]` | offer the suggestions |

### Example

`fixtures/ts-basic`, `subject=add`, `limit=1` — abbreviated:

```jsonc
{
  "subject": { "entity_id": "fn_5fcd…", "kind": "function", "name": "add",
               "file_path": "src/math.ts", "start_line": 3 },
  "relations": ["CALLS", "REFERENCES", "EXTENDS", "IMPLEMENTS"],
  "max_depth": 6, "limit": 1,
  "totals": { "entities": 3, "by_depth": [{ "depth": 1, "entities": 3 }],
              "by_relation": { "CALLS": 3 }, "by_kind": { "function": 3 }, "stale": 0 },
  "unresolved": { "sites": 4, "assertions": 4, "targets": 4, "by_category": { "value": 4 } },
  "count": 1, "results_total": 3, "truncated": true, "files_probed": 3,
  "results": [{ "entity": { "kind": "function", "name": "describe",
                            "file_path": "src/alias.ts", "start_line": 7 },
                "depth": 1, "relation": "CALLS", "direction": "outgoing",
                "status": "SUPPORTED", "strongest_source_type": "AST_RESOLVED",
                "observation_count": 1, "is_unresolved": false,
                "evidence_freshness": "fresh" }]
}
```

Three dependants, four unresolved sites. The caveat is larger than the answer, and that is the
honest shape of this repository — not a defect in the response.

### One thing to avoid calling it

This is **not** "affected tests". `nerve affected` is refused, not deferred: LCOV carries no
per-test attribution (ADR-0008 §A.2). If a test file appears in an impact set it is there because
code depends on code. Do not label the view or any part of it as test impact.

### Frontend edits the backend made

**None.** No file under `apps/nerve-web/` was touched by Slice 7b. `ROUTES` grew from 10 to 11
entries in the Rust router; no TypeScript mirror asserts that list, so nothing broke.

---

## Entry 3 — selectors now name documents and files, and refuse differently

**Slice 8b-i** · behaviour change on **every** endpoint that takes a selector ·
report: `docs/reports/slice-08b-i-report.md`
**Status: ⬜ no UI change made.** Nothing under `apps/nerve-web/` was touched. Two of the changes
below are backward-compatible additions; **one changes a status code you may already handle.**

### What changed

A repository-relative path now names whatever is actually at it. Before this slice
`docs/architecture.md` resolved to **nothing** — the path stage asked only about modules — and at
`src/app.ts` the `File` entity was silently passed over in favour of the `Module` with no
indication a choice had been made. On the documentation fixture that was 26 % of entities
unreachable by their most natural identifier.

### Selector grammar — the whole of it

```
selector  := [ qualifier ":" ] body
body      := <entity_id> | <rel_path> | <rel_path> "#" <qualified_name> | <name>
```

Qualifiers are generated from the entity-kind vocabulary, so the list below **is** the list of
kinds, plus two aliases. If a kind is ever added, its qualifier exists the same day.

| qualifier | selects |
|---|---|
| `repository:` `directory:` `file:` `module:` `function:` `method:` `class:` `interface:` `document:` `section:` `unresolved:` `coverage_run:` | that kind only |
| `symbol:` | alias — `function`, `method`, `class`, `interface` |
| `adr:` | alias — a `document` whose `meta.adr` is true, matched on its ADR id (`adr:ADR-0001`) |

A colon only introduces a qualifier when it comes before the first `/` and the first `#`, so a path
containing a colon (`docs/a:b.md`) is still a path. **`#` remains the symbol separator** — there is
no `::` form.

### One path, two readings — `alternatives`

`src/app.ts` holds both a `Module` and a `File`; `docs/architecture.md` holds both a `Document` and
a `File`. The rule: **content wins, container is reported.**

Every endpoint that resolves a selector now carries a `selectors` object:

```jsonc
"selectors": {
  "subject": {                     // keyed by the query parameter name
    "matched_by": "path",          // entity_id | path | path_qualified | name
    "alternatives": [ { "kind": "file", "name": "app.ts", "entity_id": "file_…", … } ]
  }
}
```

`alternatives` is **`[]` for the overwhelming majority of selectors** and only ever non-empty when
a path had a second reading. When it is non-empty the UI should say which entity was chosen and
offer the other — the passed-over entity is always addressable as `file:<path>` or
`directory:<path>`.

This is an **addition**; every existing field is unchanged.

> **Shape note.** `nerve <cmd> --json` emits `selectors` as an *array* of
> `{role, selector, matched_by, alternatives}`. The HTTP API emits an *object keyed by query
> parameter name*. Same information, two shapes; each is uniform within its own surface. If you
> ever diff CLI and API output, expect this.

### Error states — one is new, one is a changed status

| condition | status | code | note |
|---|---|---|---|
| resolves | `200` | — | |
| several candidates | `409` | `ambiguous_selector` | unchanged, `detail.candidates[]` |
| nothing matches | `404` | `selector_not_found` | unchanged, `detail.suggestions[]` |
| **malformed selector** | **`400`** | **`invalid_selector`** | **new.** Unknown qualifier (`banana:foo`), empty qualifier, empty body. A malformed request, *not* a miss — do not render it as "not found" |
| **path escapes the root** | **`400`** | **`refused_selector`** | **changed.** Previously this returned `404 selector_not_found` |

**`refused_selector` is the one to check your existing handling against.** `../../etc/passwd`,
`/etc/passwd` and `..\..\x` used to come back as an ordinary miss, which asserted a check the
backend had never run. They are now an explicit refusal. If a view maps every non-200 selector
outcome onto "not found", it will now mislabel a refusal.

Suggested wording — *"That selector is refused: a path outside the repository root, or one
containing `..`, is never resolved."* Do not offer search suggestions for it; there is nothing to
suggest, and offering alternatives implies Nerve went looking.

### Suggestions are now typeable

The `detail.suggestions[]` list attached to a `404` previously rendered strings like
`docs/architecture.md.architecture` — which resolve to nothing. Every suggestion now carries a
`qualified_name` that **can be typed back as a selector**. A UI may safely make them clickable.

### States for a selector input

| state | render |
|---|---|
| resolved, `alternatives: []` | the entity |
| resolved, `alternatives` non-empty | the entity, plus *"also at this path: file `app.ts`"* with the `file:` selector |
| `409` | the candidate list; Nerve refuses to choose |
| `404` | the suggestions, as clickable selectors |
| `400 invalid_selector` | "not a selector" — a typo in the qualifier, not a missing entity |
| `400 refused_selector` | a refusal, with no suggestions |

### Known gap

`./docs/architecture.md` — with a leading `./` — returns `404`. It is correctly **not** refused,
but the `./` is not normalised away either. A path pasted from shell tab-completion will miss.
Deferred deliberately; normalisation is its own design question.

### Frontend edits the backend made

**None.** No file under `apps/nerve-web/` was touched.

---

<!--
  Append new entries below this line, newest last. Each entry should carry:
  endpoint · method · authentication · query parameters · request schema · response schema ·
  empty state · unsupported state · stale state · error states · pagination · limits ·
  example response · user-facing interpretation.
-->
