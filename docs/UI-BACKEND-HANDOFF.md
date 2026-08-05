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

## Entry 4 — Slice 10a: endpoints, and one changed default

`4e4239a`. Backend surface only; no frontend product work was done.

### One new entity kind

| | |
|---|---|
| `kind` | `endpoint` |
| `name` | the canonical address, e.g. `GET /users/{user_id}` — **this is what FTS5 indexes**, so `nerve search users` now returns routes |
| `scope_path` | the module that declares it |
| `is_symbol` | **false.** It does not move `symbols_total` |
| addressable by path | **no.** `api/routes.py` names the module, never the routes inside it |

`meta` carries `endpoint_kind` (closed vocabulary, one member today: `http_route`), `framework`
(`fastapi` \| `flask`), `method`, `path`, `rule_id`, and `declarations_in_module`.

**`path` is the DECLARED path, not the deployed one.** No prefix from `APIRouter(prefix=…)` or a
blueprint registration is composed in. If the interface ever shows this next to a real URL, it must
not imply they are the same string.

### One new relation

`SERVED_BY`, from the endpoint to its handler. Gloss added to `RELATION_VERB` as
`['is served by', 'serves']`.

**Recommended wording.** A registration proves a table entry, not an execution. It does **not** mean
the route is reachable in production, that middleware permits access, that dynamic configuration has
not replaced it, or that two matching path strings are one deployed endpoint. Every observation
carries a `proves` field saying so; prefer quoting it to inventing a phrase. **Never render
`SERVED_BY` as a call** — the same rule ADR-0005 sets for `COVERS`.

### The one changed field

`relations_effective` on `/api/impact`, `nerve impact --json`, and the `nerve_impact` MCP tool now
contains **five** relations by default where it contained four:

```json
["CALLS", "REFERENCES", "EXTENDS", "IMPLEMENTS", "SERVED_BY"]
```

Any interface that hard-codes four, or renders a fixed-width legend, needs the fifth. This is the
change that closes the measured defect: without it a live route handler and dead code produce
identical impact answers.

### Empty and ambiguous states

| state | meaning |
|---|---|
| no `endpoint` entities at all | either the repository declares no routes, or its framework has no rule yet. **These are not distinguishable from the API today** — a known gap, and the interface should not say "no routes" |
| `declarations_in_module > 1` | the same method and path declared more than once in one module. **Both edges are kept**; Nerve does not choose. Render as ambiguity, not as a duplicate to hide |
| handler is a `method` | the target is a `Method`, not a `Function`. Both are valid |

### Not yet available

TypeScript/JavaScript routes (Express) are **Slice 10b**. Until then a TS/JS repository yields zero
endpoints, and that absence means "no rule yet", not "no routes".

### Frontend edits the backend made

**10 added lines**, all of them vocabulary entries that `crates/nerve-server/tests/ui_vocabulary.rs`
requires in order to pass:

- `apps/nerve-web/src/api/types.ts` — `'endpoint'` in `ENTITY_KINDS`, `'SERVED_BY'` in `RELATIONS`
- `apps/nerve-web/src/vocab.ts` — `endpoint` gloss in `KIND_GLOSS`
- `apps/nerve-web/src/format.ts` — `SERVED_BY` verb pair in `RELATION_VERB`

No styling, no layout, no navigation, no views. There is **no Endpoints view** — building one is the
user's call.

---

## Entry 5 — Slice 11a: test-observed calls, and three values where you expect two

**Slices 11a, 11a-i** · new relation, new evidence source type, no new endpoint ·
**no UI exists for any of it**

### What landed

`nerve trace import` reads a `nerve-trace/v1` artifact that a user's own tracer produced and asserts
`TEST_OBSERVED_CALL` between the two frames of each observed call. **Nerve runs no tests** — the
producer (`tracers/python/`) runs in the user's process, by the user's explicit invocation.

**There is no new HTTP endpoint.** Import is a write path and is CLI-only. Trace observations appear
through the endpoints that already exist — `/api/why`, `/api/entity`, `/api/neighbourhood` — because
they are observations like any other and those endpoints are generic over the evidence model.

### The four things a view must get right

**1. `TEST_OBSERVED_CALL` is deliberately *not* in the default impact closure.**

`/api/impact` returns a closure over `CONTAINS`, `IMPORTS`, `CALLS`, `REFERENCES` and `SERVED_BY`. It
does **not** include `TEST_OBSERVED_CALL`, and that is a decision rather than an omission — see
`docs/plans/slice-11a-trace-ingestion.md` §8. A trace says *one run took this edge*; a blast radius
built on it would grow and shrink with which tests happened to run. There is also a security reading in
`docs/THREAT-MODEL.md` T9: an artifact is untrusted input, and an edge it could inject into the default
closure would change what Nerve tells a user to review before a change.

So: if a view offers a relation filter, `TEST_OBSERVED_CALL` must be **opt-in and labelled**, and an
impact view must not imply trace edges were considered. Contrast `SERVED_BY`, which *is* in the default
set — Entry 4 explains why the two decisions differ.

**2. The evidence names a set of runs, not a run.**

`observation.environment` holds `runs[]`, an array, because `idx_observation_identity` has no column
that could hold a second row per test: two tests reaching one callee from one line are **one
observation** naming both. A view rendering "which test observed this" must render a **list**.

The derived scalars on that object — `completion_state`, `repository_binding` — are the **weakest**
value across contributing runs, not the first one's. A site observed by one complete run and one
interrupted run reads as `partial`. Do not recompute them from `runs[0]`.

**3. `repository_binding` has three values, and `unverified` is not `stale`.**

| value | means | do not render as |
|---|---|---|
| `bound` | the artifact names this exact tree, verified | — |
| `stale` | the artifact names a **different** tree | anything reassuring |
| `unverified` | the artifact named **no** tree, so nothing was checked | `stale`, or a failure |

`unverified` is the absence of a check, not a failed check. This is the same three-valued shape as
`CoverageEvidence::Absent` and `gaps totals: null` in the standing contract note above: **absence of
verification is not verification of absence.** A two-state badge here would be a lie in one direction
or the other.

An attacker cannot upgrade a binding by asserting a state: a plausible 40-hex commit and 64-hex merkle
for a tree that is not this one yields `stale`, never `bound`.

**4. `count` is not a frequency.**

A record's `count` is how many times **one run** took that edge. It is not a claim about runs in
general, and two runs' counts are recorded per run rather than summed into a total. Never label it
"called N times" without naming the run.

### The sentence the CLI prints, and why a view needs its own version

```
A trace is existential evidence: it says this run took these edges, not
that every run does, and absence of an edge is absence of observation.
```

**Absence of a `TEST_OBSERVED_CALL` edge means nothing was observed, not that no call exists.** An
untraced repository has zero trace edges and a fully-traced one has some; a view that draws "no
observed calls" the way it draws "no callers" would turn missing instrumentation into an apparent fact
about the code. This is `gaps`' `null`-versus-`0` problem in a new place.

### Frontend edits the backend made

**4 added lines**, all vocabulary entries `crates/nerve-server/tests/ui_vocabulary.rs` requires:

- `apps/nerve-web/src/api/types.ts` — `'TEST_OBSERVED_CALL'` in `RELATIONS`
- `apps/nerve-web/src/format.ts` — its verb pair, `['was observed calling', 'was observed called by']`

The passive, hedged verb is deliberate and matches `SERVED_BY`'s reasoning in Entry 4: `calls` would
assert a static fact from a single observation. `TEST_CALL_TRACE` already had a gloss in
`SOURCE_TYPES` — *"A call observed during a test run, through instrumentation"* — so nothing was added
for it.

No styling, no layout, no navigation, no views. **There is no trace view, no run picker and no test-to-
symbol view** — building them is the user's call.

---

## Entry 6 — Slice 12b: history, and the difference between "nothing" and "nothing visible"

Three new CLI commands and a new schema version. **No new HTTP endpoint, no new MCP tool and no new
view** — 12c adds those. This entry exists so that when they arrive, the display language is already
decided, because every mistake available in this data is a wording mistake.

### What landed

Schema **v6**: four new tables — `git_commit`, `git_change`, `git_rename_hypothesis`,
`git_history_ingest`. **No new entity kind and no new relation.** A commit is not an entity and a
change is not an assertion; the reasoning is in `docs/plans/slice-12b-historical-model.md` §3. That
means `EntityKind`, `Relation` and `EvidenceSourceType` are unchanged, so nothing in
`apps/nerve-web/src/api/types.ts` or `vocab.ts` needed an edit and **zero frontend lines changed in
this slice.**

Six new closed vocabularies in `nerve-core`, none of them mirrored into the UI yet:

| vocabulary | values |
|---|---|
| `ChangeKind` | `added` `modified` `deleted` `mode_changed` |
| `ParentCompleteness` | `root` `shallow_boundary` `parents_available` `parents_missing` `parents_unverifiable` |
| `ChangesEnumerated` | `enumerated` `merge_not_enumerated` `parent_unavailable` `refused` |
| `WalkTermination` | `exhausted` `commit_budget` `shallow_boundary` `missing_object` `refused` |
| `RenameEvidence` | `exact_content` (12c adds `similar_content`) |
| `RenameAmbiguity` | `unique` `many_from` `many_to` `many_both` |

**12c must add a gloss for every member of all six**, and `ui_vocabulary.rs` will *not* catch the
omission the way it caught Slice 5d-iii's, because nothing in the TypeScript references these names
yet. Adding the mirror is what turns the test back on.

### The commands

```
nerve history sync [--max-commits N] [--with-identity] [PATH]
nerve history log  [--limit N] [--offset N] [--path P]
nerve history file <PATH_AS_A_TREE_RECORDED_IT> [--limit N] [--path P]
```

`sync` is the only one that opens `.git`; `log` and `file` read the database with
`PRAGMA query_only=ON` and are asserted to leave it byte-identical. `sync` exits **3**
(`PARTIAL_INDEX`) when anything was refused and **0** otherwise, so a caller can tell a complete read
from a partial one; **10** for `--max-commits` above the clamp of 5000, which is refused rather than
silently honoured.

`history file` takes a path **as a tree recorded it**, matched literally against stored bytes. It is
not a selector and is not resolved on disk, because a historical path may not exist any more —
`discover::canonical_child` canonicalizes and so cannot validate one. The JSON says so in
`path_is_as_recorded_in_a_tree`.

### Six things a view must get right

**1. `answerable: false` is not zero commits.** `history log` on a repository that has never been
synced returns `answerable: false`, and that is a different answer from a sync that ran and found
nothing. The CLI spells it out — *"This is not 'this repository has no commits'… listing zero commits
would report the second as the first"* — and a view must too. This is `gaps`' `totals: null` problem
for the third time on this project.

**2. Never say history begins anywhere unless the backend says it may.** Every commit row carries
`may_claim_history_begins_here`, computed by
`ParentCompleteness::may_claim_history_begins_here()`, which is true for `root` and nothing else. **Do
not re-derive it from `parent_completeness` in TypeScript.** That would be a second copy of the one
rule this slice exists to protect, free to drift from the first. Approved wording for a boundary:
*"earliest commit visible in this checkout; history before this point is unavailable to this
repository."* Forbidden: *"the project's history begins here"*, *"first commit"*, *"beginning of
history"*.

**3. Zero changes has four different meanings, and `changes_enumerated` is which.** `enumerated` — the
diff ran and nothing changed. `merge_not_enumerated` — a merge has several parents and a change is
only defined against one, so none were enumerated. `parent_unavailable` — the parent tree could not be
read. `refused` — a bound or a malformed tree stopped it. A view rendering "0 changes" without the
reason commits the defect the column exists to prevent, and the two most common cases mean opposite
things: one is *"nothing happened"* and the other is *"we cannot see what happened."*

**4. Three reasons history stops, and only one is the repository's fault.** `walk_terminated_by` is
`shallow_boundary` (declared, expected), `missing_object` (a fault), or **`commit_budget` — Nerve
stopped, and the repository has more history than this ingest read.** The third is Nerve's own doing
and must never be drawn as a property of the repository. `earlier_changes_may_exist` on `log` and
`file` is the single boolean a view should key its caveat on.

**5. A rename is a hypothesis and `ambiguity` is not decoration.** `unique` `many_from` `many_to`
`many_both`. When one deleted blob matches several added paths, **every pairing is recorded and none is
promoted** — identical content is common (an empty file, a copied licence header, a barrel re-export).
There is no score, no threshold and no tie-break, and a view must not invent one or render a
`many_to` pairing the way it renders a `unique` one.

**6. Refusals are per-ingest; the per-commit record is not.** `refused` and `refused_total` describe
*the ingest that produced them*, so a re-sync with no tree work correctly reports zero. The durable
record is `changes_enumerated == 'refused'` on the commit. A view showing only the ingest tally would
let a clean re-run erase the evidence of a hostile tree.

### And one thing the data is not

**A commit summary is the first free-form repository *prose* Nerve stores.** Until now the only
repository-derived strings in the graph were *names* — identifiers, headings, paths — which is why
Slice 8a found the real T7 vector was a Markdown heading and not body text. A summary is body text: it
is attacker-influencable in any repository that takes contributions, it is bounded at 512 bytes, first
line only, lossy UTF-8, and **it is data on every surface**. Render it as text, never as HTML.
`fixtures/history-hostile` carries `<script>alert(1)</script>` and an instruction-shaped summary so
the escaping has something real to catch.

Truncation is flagged **per repository, not per commit** — v6 has no per-commit column — so from
`git_commit.summary` alone a 512-byte summary cannot be told from a cut one. The flag is
`summaries_truncated` on the ingest. Say "one or more summaries were cut", not "this one was".

### Frontend edits the backend made

**None.** No entity kind, no relation and no evidence source type was added, so nothing the interface
mirrors changed. The six vocabularies above are 12c's work.

---

## Entry 7 — Slice 12c: the history endpoints, and the one sentence a view may not write

Seven read-only routes, `ROUTES` 11 → 18. Every one is `GET`, token-gated, `Host`- and
`Origin`-checked by the same `guard.check` the other eleven use — which runs **before** dispatch, so
there is no per-route opt-in to forget.

| route | parameters | answers |
|---|---|---|
| `/api/history` | — | is history ingested, is it shallow, where is the boundary, is it fresh |
| `/api/history/commits` | `limit`, `offset` | the commit log, newest committer time first |
| `/api/history/commit` | `commit` | what one commit changed |
| `/api/history/path` | `path`, `limit` | one path's history **plus the first/last-observed block** |
| `/api/history/diff` | `from`, `to`, bounds | what lies between two recorded states |
| `/api/history/frequency` | `limit` | which paths changed most often in visible history |
| `/api/history/cochange` | `path`, `limit` | which paths changed in the same commits |

### The availability block is on every answer, and it is not decoration

Every response carries the same block, assembled in one place in the server:

```
repository_id · current_repository_state{state_id, git_commit} · requested_subject
history_ingested · shallow · shallow_boundary · promisor
walk_terminated_by + walk_terminated_note · commits_recorded · commit_budget
refusals · refusals_total · reader_version · totals
result_kind · freshness + freshness_note · ingest_head_oid
truncation · continuation · limitations{earlier_changes_may_exist, …}
```

**Render it.** A history answer without it is a set of dates with no statement of what they are dates
*of*, which is the whole failure this row exists to prevent.

### The one sentence a view may not write

**Only `may_claim_created: true` licenses the word "created".** The flag is on the response, it comes
from `FirstObservedKind::may_claim_created()` in `nerve-core`, and it is the only copy of that
permission in the product. Beside it is `may_claim_created_note`, the sentence the backend wrote for
this case. Render the note; do not compose your own from the `kind`.

`kind` has **six** values and only one of them is a creation:

| `kind` | render as | never |
|---|---|---|
| `created_in_visible_history` | "created at this change" | — |
| `earliest_visible_change` | "the earliest change Nerve can see" | "first ever", "created" |
| `present_before_visible_history` | "in the tree now, untouched in visible history" | "no history" |
| `absent_from_visible_history` | "not in the tree, and no recorded commit touched it" | "deleted" |
| `current_tree_unknown` | "no index, so the current tree could not be consulted" | "does not exist" |
| `no_history_ingested` | "history has never been read here" | "no history" |

The last three are the shallow-clone reality: `present_before_visible_history` is the **common** case
there, because every unchanged file has zero change rows. Rendering it as an empty result says the
file has no history, which is false.

`earlier_history_unavailable` names which of five reasons puts history above the earliest change out
of reach — `shallow_boundary`, `parents_missing`, `parents_unverifiable`, `commit_budget`,
`walk_refused` — and each carries its own note. **The last two are Nerve's own doing**, never a
property of the repository, and the notes say so. `null` means nothing is hidden above **this path**.

Note the scope trap: `earlier_history_unavailable` is about the **path**;
`limitations.earlier_changes_may_exist` is about the **repository's ingest**. They legitimately
disagree — a shallow clone can hold a genuine root commit, so a path created at that root has nothing
hidden above it while the repository still reports that earlier commits may exist. Do not "fix" that
by showing only one.

### `history/diff`: `null` is not `[]`

Five outcomes. Four are refusals — `state_not_recorded`, `not_an_ancestor`, `ancestry_incomplete`,
`walk_budget_exhausted` — and **every diff-shaped key is present on every outcome, `null` on the
four**. So `commits: null` means no range was computed and `commits: []` means one was and holds
nothing. A view that treats absent-or-empty alike turns a refusal into "nothing changed", which is
the failure the shape exists to make impossible.

A merge enumerates **no** changes by 12b's decision, so a merge-heavy range with few changes is
expected. `merges_in_range` and per-commit `changes_enumerated` are what stop that reading as "little
changed".

### `history/cochange`: an observation, and the word is load-bearing

The response carries `disclaimer`, which is `nerve_store::COCHANGE_IS_NOT_A_DEPENDENCY` verbatim.
**Print it.** Two files changing together is equally consistent with coupling, a formatting sweep, a
version bump, and one commit that did two unrelated things. The count is a raw shared-commit count and
never a normalised affinity, because a normalised number invites exactly the comparison the label
forbids. Do not label it `related`, `coupled`, `depends` or `affinity` — a server test fails on those
words, and the UI should hold the same line.

### A symbol selector is refused, and nothing is guessed

`path` names a **path as a tree recorded it** — matched literally, never resolved against the working
tree, because a historical path routinely does not exist on disk. A symbol-shaped selector
(`src/app.ts#parse`, `function:parse`) is refused with `reason: "symbol_selector_refused"`,
`path_guessed: false`, `nothing_was_looked_up: true`.

**Do not offer the containing file as a fallback.** `git_change` is path-keyed, so a symbol's dates
would be *its file's* dates — a different claim wearing the same words. Surface the refusal.

One HTTP detail the UI must get right: a raw `#` in a query string is a fragment and is **dropped by
the browser before the request is sent**, so `?path=README.md#parse` arrives as `path=README.md` and
is answered for the file. **Encode it as `%23`** when a user types a symbol selector, or the refusal
never fires and the answer looks right while describing something else.

### Untrusted text

`summary`, `path`, `from_path`, `to_path` and every refusal key are repository-derived. Render as
text, never as HTML, never as a field name. `fixtures/history-hostile` carries
`<script>alert(1)</script>` and an instruction-shaped summary; a server test asserts no raw `<`
reaches the wire and that a summary never becomes a key.

### Frontend edits the backend made

**None.** This entry is the contract; the views are 12c-iv. Eight vocabularies still have no gloss —
the six from 12b plus `FirstObservedKind` and `HistoryFreshness` — and
`crates/nerve-server/tests/ui_vocabulary.rs` does **not** yet know they exist, so it cannot fail for
them. See `docs/plans/ui-parity-matrix.md` §3: the glosses and the guard extension belong in one
commit, or the next vocabulary is unguarded again.
