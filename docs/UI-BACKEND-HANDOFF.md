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

<!--
  Append new entries below this line, newest last. Each entry should carry:
  endpoint · method · authentication · query parameters · request schema · response schema ·
  empty state · unsupported state · stale state · error states · pagination · limits ·
  example response · user-facing interpretation.
-->
