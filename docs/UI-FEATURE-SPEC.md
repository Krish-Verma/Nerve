# Nerve — UI Feature Specification

**Purpose.** This is the complete human-facing surface of Nerve, written so that the visual
interface can be rebuilt from scratch without reading any Rust. Every capability the product has
today is here, with the exact route, the exact endpoint, the exact response field names, the exact
bounds, and the wording the evidence supports.

**Its relationship to `docs/UI-BACKEND-HANDOFF.md`.** The two are companions and neither replaces
the other.

| | |
|---|---|
| `UI-BACKEND-HANDOFF.md` | The **change log**. Appended per slice, newest last. It records what a slice added, renamed or removed, and why. Read it to find out what moved since the last UI session |
| `UI-FEATURE-SPEC.md` (this file) | The **surface**. A standing description of everything that exists right now, in one schema per feature. Read it to build a screen |

Where the two disagree, this file was written from the code and cites `file:line`; the handoff was
written at the time of the change. Check the citation.

**Scope of this revision.** Written for the capabilities shipping today — `docs/ROADMAP.md` rows 1
through 12a. Rows **12b** (Git history ingestion), **12c** (historical questions), **13**
(cross-repository contracts) and **14** (human-confirmed memory) will append their features to
§6 and their vocabularies to §4. The per-feature schema in §2 is fixed so that appending is
mechanical.

**Accuracy rule applied throughout.** No field name in this document was invented. Anything that
could not be established from the code is marked `UNVERIFIED —` with what could not be
established.

---

## 1. Contents

- §2 — the per-feature schema, and how to read it
- §3 — the feature matrix: every capability × CLI, HTTP, MCP, current UI
- §4 — the closed vocabularies a UI must be able to render
- §5 — authentication and the security envelope
- §6 — the features
- §7 — the honesty rules that cut across every feature
- §8 — every bound in one table
- §9 — what is deliberately not in the UI, and why
- §10 — append point for rows 12b, 12c, 13, 14

---

## 2. The per-feature schema

Every feature in §6 gives exactly these fields, in this order, as a two-column table. The shape is
fixed; a new feature copies the table and fills it in.

| field | means |
|---|---|
| **user goal** | the question a person came to the screen with, in their words |
| **route or current surface** | the real fragment string in the shipped bundle, or `not in the UI` |
| **backend service** | the `nerve-store` / `nerve-index` function actually called |
| **HTTP endpoint** | exact path and query parameters, or `none` |
| **MCP equivalent** | exact tool name, or `none`, with the reason |
| **CLI equivalent** | exact command, or `none` |
| **required inputs** | what a caller must supply |
| **response model** | field names and types, transcribed from `shapes.rs` |
| **main actions** | what a user does on the screen |
| **loading state** | — |
| **empty state** | — |
| **error state** | — |
| **partial state** | — |
| **stale state** | — |
| **unsupported state** | — |
| **ambiguity state** | — |
| **pagination or bounds** | the actual caps and continuation fields |
| **evidence fields** | which of: entity identity · kind · repository state · source location · evidence type · directness · resolution method · extractor id · extractor version · freshness · parse health · ambiguity · unresolved state · partial state · coverage run |
| **recommended user wording** | honest phrasing — see §7 |
| **security constraints** | — |
| **screens or components** | what a redesign will need to build |

**On the state rows.** A state that genuinely cannot occur says `cannot occur — <reason>`. It never
says `n/a`. This is not pedantry: Slice 5d-iii found the interface carrying a gloss for an
assertion status the backend cannot produce, and this document found two more of the same class
(§7.8). A state invented in a spec becomes a state built into a screen and never exercised.

---

## 3. Feature matrix

`●` shipped · `○` absent · `◐` partly

| capability | CLI | HTTP | MCP | current UI |
|---|:--:|:--:|:--:|:--:|
| Index overview / counts / freshness verdict | ● `nerve status` | ● `/api/overview` | ○ | ● `#/overview` |
| Search by name | ● `nerve search` | ● `/api/search` | ● `nerve_search` | ● `#/search`, plus the bar omnibox |
| One entity: identity, occurrences, relation counts | ○ | ● `/api/entity` | ○ | ● `#/entity/<id>/relations` |
| Evidence for a relationship | ● `nerve why` | ● `/api/why` | ● `nerve_investigate` | ● `#/entity/<id>/evidence` |
| Bounded neighbourhood graph | ○ | ● `/api/neighbourhood` | ○ | ● `#/entity/<id>/graph` |
| Path between two entities | ● `nerve path` | ● `/api/path` | ● `nerve_path` | ● `#/entity/<id>/graph?to=<selector>` |
| Source snippet + hash comparison | ○ | ● `/api/source` | ○ | ● `#/entity/<id>/source`, and inside the evidence tab |
| Unresolved references | ○ | ● `/api/unresolved` | ○ | ● `#/unresolved` |
| Partial parses / parse health | ◐ counts only, in `nerve index` output | ● `/api/partial-parses` | ○ | ● `#/unresolved`, second panel |
| Coverage gaps | ● `nerve gaps` | ● `/api/gaps` | ● `nerve_gaps` | ● `#/coverage` |
| **Impact — reverse dependency closure** | ● `nerve impact` | ● `/api/impact` | ● `nerve_impact` | **○ no UI at all** |
| Selector resolution + `alternatives` | ● every selector command | ● every selector endpoint | ● every selector tool | **◐ resolves, but `alternatives` is unrenderable — see §7.7** |
| Initialize an index | ● `nerve init` | ○ write path | ○ write path | ○ |
| Build / update the index | ● `nerve index` | ○ write path | ○ write path | ○ |
| Ingest a coverage report | ● `nerve coverage` | ○ write path | ○ write path | ○ |
| Import a test-call trace | ● `nerve trace import` | ○ write path | ○ write path | ○ |
| Trust verdict for CI | ● `nerve check` | ○ | ○ | ○ |
| Installation diagnosis | ● `nerve doctor` | ○ | ○ | ○ |
| Serve the local interface | ● `nerve serve` | — | ○ | — |
| MCP session | ● `nerve mcp` | ○ | — | ○ |
| Which tests exercise a symbol | **refused** | **refused** | **refused** | **refused** — §9 |
| Run the repository's test suite | **refused** | **refused** | **refused** | **refused** — §9 |

### The gaps this matrix actually found

**1. `/api/impact` has no interface of any kind.** No view, no route, and no TypeScript type: the
string `impact` does not appear anywhere under `apps/nerve-web/src/`. `api/types.ts` has no
`ImpactReport`, `ImpactRow`, `ImpactTotals` or `UnresolvedAccount` interface. This is the largest
single gap in the matrix, and it is the endpoint carrying the project's most load-bearing honesty
requirement (§7.2). Building it is entirely new work, TypeScript mirrors included.

**2. `views/Path.tsx` *is* reachable — the assumption that it is dead code is wrong.**
`apps/nerve-web/src/views/Graph.tsx:31` imports `PathFinder` from `./Path`, and `Graph.tsx:71–75`
renders it whenever the route carries a `to` option. So `/api/path` has a reachable UI at
`#/entity/<id>/graph?to=<selector>`, entered by the "path to something" segmented control
(`Graph.tsx:60–68`).

It is, however, **undiscoverable**: there is no rail entry, no top-level route, and the only way in
is a control on the *graph tab of an entity you already found*. A path is a question about two
things and the UI only lets you ask it from inside one of them. Treat it as a real feature with a
navigation defect, not as dead code.

> A note on how this was established, because it is a trap. `grep` classifies
> `apps/nerve-web/src/views/Graph.tsx` as a **binary file** (it contains characters outside the
> shell's byte locale), so a plain `grep -rn PathFinder .` prints nothing and the file looks
> unreferenced. `grep -a` is required. Ten of the interface's source files are affected. Any audit
> of this codebase that uses `grep` without `-a` will produce false negatives.

**3. Nothing in the UI reads `selectors` / `alternatives`.** The API attaches a `selectors` object
to every answer that resolved a selector (`api.rs:668–685`), but `api/types.ts` omits the field
from `EntityDetail`, `Neighbourhood`, `PathReport` and `WhyReport`. "This path also names a `file`
— open that instead" cannot be rendered today. See §7.7.

**4. The MCP surface answers 5 of the 11 HTTP questions.** No tool covers `overview`, `entity`,
`neighbourhood`, `source`, `unresolved` or `partial-parses`. The stated rule is that a tool must
have a materially different input/output contract (`mcp.rs:79–85`), so this is a decision rather
than an omission — but a UI author comparing surfaces should not expect parity.

**5. The CLI has no `entity`, `neighbourhood`, `source`, `unresolved` or `partial-parses`
command.** Parse health reaches a terminal only as summary counts inside `nerve index` output
(`main.rs:730–736`).

**6. The CLI has 14 top-level commands, not 13.** `crates/nerve-cli/src/main.rs:48–309` declares
`init` `index` `coverage` `trace` `status` `check` `doctor` `search` `gaps` `impact` `path` `serve`
`mcp` `why`, and `scripts/final_acceptance.sh:108` iterates the same fourteen names. `trace` is a
subcommand group with one member, `nerve trace import` (`main.rs:317–333`). Two documents say
thirteen — `docs/FINAL-ACCEPTANCE.md:59` and `docs/CONTINUATION.md:319` — and both are off by one.
Recorded here rather than corrected, because those files are outside this document's scope.

---

## 4. Closed vocabularies a UI must render

Every vocabulary below is closed in Rust: values are parsed, never invented, and an unknown string
is an error rather than a tolerated tag (`crates/nerve-core/src/vocab.rs:1–4`).

**Line citations into `vocab.rs` are to declarations, which are appended to and never inserted
into** (`vocab.rs:60–63` states this as a contract with the interface). Every enum below sits above
the point new slices append at, so these citations are stable; the file's *test module* is not, and
is referenced by test name instead.

**A test enforces the glosses.** `crates/nerve-server/tests/ui_vocabulary.rs` reads the shipped
TypeScript *as text*, extracts the keys of each gloss table, and fails if any Rust vocabulary
member has no entry. A redesign that renames a gloss table, or drops a member, breaks
`cargo test --workspace`. It also asserts that no gloss is empty and none is the fallback sentence
(`ui_vocabulary.rs:516–549`).

| vocabulary | members | defined in | interface table the test requires |
|---|:--:|---|---|
| `EntityKind` | 13 | `vocab.rs:64–80` | `KIND_GLOSS` in `vocab.ts:91`, **and** `ENTITY_KINDS` in `api/types.ts:347` mirrored **in order** |
| `Relation` | 12 | `vocab.rs:337–353` | `RELATION_VERB` in `format.ts:102` (both directions), **and** `RELATIONS` in `api/types.ts:364` mirrored **in order** |
| `EvidenceSourceType` | 12 | `vocab.rs:490–508` | `SOURCE_TYPES` in `format.ts:51` |
| `Directness` | 3 | `vocab.rs:572–576` | `DIRECTNESS` in `format.ts:71` **and** `directnessClass` in `format.ts:90` |
| `AssertionStatus` | 5 | `vocab.rs:622–628` | `STATUS_GLOSS` in `format.ts:135` — but see §7.8, only two are producible |
| `Freshness` | 5 | `freshness.rs:53–59` | `FRESHNESS` in `format.ts:13` |
| `SymbolCoverage` | 4 | `gaps.rs:121–126` | `COVERAGE_STATE` in `vocab.ts:124` |
| `CoverageEvidence` | 2 | `gaps.rs:77` | rendered inline in `Coverage.tsx`; not a test-checked table |
| `UnresolvedCategory` | 4 | `vocab.rs:419–424` | `UNRESOLVED_CATEGORY` in `vocab.ts:145` |
| `EndpointKind` | 1 | `vocab.rs:190` | no gloss table — carried inside `meta.endpoint_kind` |
| `PathRole` | 3 | `vocab.rs:222–230` | internal to selector resolution; never serialized |
| unresolved-reason codes | `UnresolvedReason::ALL` + `docref::reason::ALL` | `nerve-index` | `UNRESOLVED_REASON` in `vocab.ts:12` |
| unmodelled call forms | `UNMODELLED_FORMS` | `nerve-index/src/refs.rs` | `UNMODELLED_FORM` in `vocab.ts:72` |
| ADR statuses | `AdrStatus::ALL` + `STATUS_UNPARSED` | `nerve-index/src/docs.rs` | `ADR_STATUS` in `vocab.ts:163` |
| selector qualifiers | 13 kinds + 2 aliases | `select.rs:286–292` | not glossed; offered as typeable text |
| `SelectorKind` (`matched_by`) | 4 | `select.rs` | not glossed |
| `InvalidSelector` | 3 | `select.rs:296–314` | not glossed |
| `SelectorRefusal` | 1 | `select.rs:320–341` | carries its own `statement()` |
| `nerve check` verdicts | 5 | `main.rs:1244–1252` | CLI only |
| `nerve doctor` severities | 4 | `doctor.rs:58–65` | CLI only |

### Member lists, verbatim

**`EntityKind`** (`vocab.rs:83–99`) — `repository` `directory` `file` `module` `function` `method`
`class` `interface` `document` `section` `unresolved` `coverage_run` `endpoint`.
Exactly four are symbols: `function` `method` `class` `interface` (`vocab.rs:122–127`).
Order is a contract with the interface (`vocab.rs:60–63`) and appended to, never inserted into.

**`Relation`** (`vocab.rs:364–379`) — `CONTAINS` `DEFINES` `IMPORTS` `EXPORTS` `CALLS`
`REFERENCES` `EXTENDS` `IMPLEMENTS` `SUPERSEDES` `COVERS` `SERVED_BY` `TEST_OBSERVED_CALL`.

**`EvidenceSourceType`** (`vocab.rs:511–526`) — `AST_DIRECT` `AST_RESOLVED` `AST_HEURISTIC`
`TYPE_RESOLVED` `FRAMEWORK_RULE` `TEST_COVERAGE` `TEST_CALL_TRACE` `RUNTIME_CALL_TRACE`
`DOCUMENT_STATED` `HUMAN_CONFIRMED` `LLM_DERIVED` `FILESYSTEM_OBSERVED`.
**Declaration order is not a truth ranking** (`vocab.rs:456–459`); it is the structural ordering
behind `strongest_source_type` and the stored `source_type_mask` bit layout. Do not draw it as a
scale.

**`Directness`** (`vocab.rs:579–585`) — `DIRECT` `RESOLVED` `INFERRED`.
`directnessClass` must have **no `default:` arm** — a test asserts it (`ui_vocabulary.rs:346–357`).
An unrecognised directness must render as visibly unknown; reusing a real member's visual class
asserts something about the evidence that was never observed.

**`Freshness`** (`freshness.rs:62–70`) — `fresh` `stale` `file-missing` `refused` `unreadable`.
Note the hyphen in `file-missing`.

**`SymbolCoverage`** (`gaps.rs:129–136`) — `covered` `partial` `uncovered` `unmeasured`.
**`CoverageEvidence`** (`gaps.rs:80–85`) — `absent` `present`.

**`UnresolvedCategory`** (`vocab.rs:427–434`) — `module` `value` `document_link`
`document_supersedes`. A fifth bucket, `uncategorised`, appears in `impact.unresolved.by_category`
for a stored category this build cannot parse (`impact.rs:267`). It is not a vocabulary member and
must be rendered as "Nerve could not classify this site", never as a category.

**`AssertionStatus`** (`vocab.rs:631–639`) — `SUPPORTED` `CONTRADICTED` `STALE` `UNRESOLVED`
`DELETED`. All five must be glossed; only two can occur. §7.8.

---

## 5. Authentication and the security envelope

### The session token

| | |
|---|---|
| What it is | 256 bits from the operating system's CSPRNG, held as 64 lowercase hex characters (`token.rs:25`, `token.rs:46–53`) |
| Where it comes from | minted once per `nerve serve` run. **Never written to disk.** It dies with the process |
| How the page learns it | `nerve serve` prints `http://127.0.0.1:<port>/?token=<hex>` (`lib.rs:152–159`). The page reads it from `location.search` and holds it in memory for the tab (`api/client.ts:44–49`) |
| How the page sends it | header `X-Nerve-Token` (`token.rs:28`, `api/client.ts:14`). The query parameter `token` is also accepted (`token.rs:36`) |
| Comparison | constant-time, length checked first and non-secretly (`token.rs:72–83`) |
| Never persisted | the interface deliberately does **not** copy it into `localStorage` or `sessionStorage`, because both are restored across sessions and `serve` promises the token is never written to disk (`api/client.ts:1–9`) |
| `Debug` is redacted | `SessionToken` prints as `SessionToken(redacted)` (`token.rs:87–91`) |

**Bookmarking the page cannot work.** The token is different every run. `App.tsx:40–65` renders a
dedicated `Gate` screen for this — not an error state, an explanation. A redesign must keep an
equivalent: this is the single most common way a user reaches the page with no credential.

### `Host` and `Origin`

Three checks, applied in this order, all required, none sufficient alone
(`guard.rs:107–130`):

1. **`Host` must equal the bound loopback address**, as `127.0.0.1:<port>`
   (`guard.rs:88–96`). This is the DNS-rebinding defence: an attacker who owns `evil.test` can
   point it at `127.0.0.1`, and the connection genuinely arrives on loopback — but the browser
   sends *their* name in `Host`. `localhost` is **not** accepted, because it is a name and names
   are resolved by a resolver Nerve does not control.
2. **`Origin`, if present, must be exactly `http://127.0.0.1:<port>`.** Browsers omit it on
   same-origin `GET` and send it cross-origin, so "absent or exactly ours" is the precise rule.
   `null` — what a sandboxed iframe or a `file://` page sends — is refused. There is **no CORS
   response header anywhere in the crate**; a test asserts it (`respond.rs:177–181`).
3. **The token must be present and correct.** This is the control that survives a non-browser
   client, where checks 1 and 2 are worthless.

| condition | status | code |
|---|:--:|---|
| bad or absent `Host` | `403` | `host_not_allowed` |
| bad `Origin` | `403` | `origin_not_allowed` |
| no token | `401` | `token_required` |
| wrong token | `403` | `token_invalid` |

(`guard.rs:42–57`. The interface treats `token_required` and `token_invalid` as the auth pair —
`api/client.ts:31–33`.)

### Why static assets are exempt from the token but not from Host/Origin

A browser cannot attach a header to a subresource request. The document is opened at `/?token=…`,
but the `<script>` and `<link>` it names are fetched with no token and no way to supply one — so
requiring the token on the embedded assets makes the interface unloadable. The exemption is
narrow and its narrowness is the point (`router.rs:118–143`):

- `Host` and `Origin` are **still enforced**. `Guard::check` applies them *before* the token, so a
  `MissingToken` or `BadToken` verdict is itself proof that both already passed. The
  DNS-rebinding defence and the cross-origin refusal are untouched.
- Only a path that resolves in the **fixed asset table** is served — `index.html`,
  `assets/nerve.js`, `assets/nerve.css`, `assets/favicon.svg` (`assets.rs:45–66`). Anything else
  still gets the guard's refusal, so an unauthorised caller still cannot learn which `/api/*`
  routes exist.
- What is served is **build-constant**: identical bytes in every copy of the binary, containing no
  repository content, no index content and no session state. A caller who can reach them already
  has the executable they came out of.
- Every `/api/*` route remains gated on all three checks.

### The CSP, and the other response headers

Sent on **every** response, including every refusal (`respond.rs:44–51`):

```
Content-Security-Policy: default-src 'none'; script-src 'self'; style-src 'self';
                         img-src 'self' data:; font-src 'self'; connect-src 'self';
                         base-uri 'none'; form-action 'none'; frame-ancestors 'none';
                         object-src 'none'
X-Content-Type-Options: nosniff
X-Frame-Options: DENY
Referrer-Policy: no-referrer
Cache-Control: no-store
Cross-Origin-Resource-Policy: same-origin
```

What this forbids a redesign from doing:

- **No inline `<script>` body and no inline `<style>` block.** `script-src 'self'` with no
  `unsafe-inline`. Two independent checks enforce it: `embed.mjs` refuses to copy in an HTML file
  that breaks it, and `assets.rs:113–150` asserts it again from Rust against the bytes that
  shipped. No `style=` attribute in the served HTML either, and no `onload` / `onerror` / `onclick`
  attribute.
- **No `style=` attributes in the *document*** — note that inline `style={{…}}` props inside the
  compiled React bundle are fine, and the current views use them freely; the prohibition is on the
  served `index.html`.
- **No remote anything.** No CDN font, no remote image, no external stylesheet. `img-src` permits
  `data:` URIs only. A test asserts the document contains no `http://` and no `https://`.
- **No framing**, no form submission (`form-action 'none'`, which is what stops a submitted form
  carrying the session token anywhere), no `<base>`.
- `Cache-Control: no-store` matters more than it looks: a cached response would outlive the token
  that authorised it.

### The rest of the envelope

| control | where |
|---|---|
| `GET` only. Any other method is `405 method_not_allowed`, decided **before** routing | `router.rs:69–75` |
| A request body is refused with `413 body_not_accepted` rather than streamed and discarded | `router.rs:76–82` |
| Request target bounded at 8 KiB → `414 target_too_long`; at most 32 query parameters → `400 too_many_parameters`; strict percent-decoding, NUL refused → `400 malformed_target` | `request.rs:13–16`, `router.rs:84–107` |
| Guard runs **before** routing, so an unauthorised caller cannot learn which routes exist, and before any database work, so it cannot make the server do work | `router.rs:11–13` |
| The database is opened `PRAGMA query_only`, so read-onlyness is a property SQLite enforces rather than one the crate promises | `lib.rs:261–265` |
| Every repository string leaves as a JSON string value with `<` `>` `&` U+2028 U+2029 escaped to `\uXXXX`. The served bytes contain no `<` at all | `respond.rs:58–72` |
| Binds `Ipv4Addr::LOCALHOST` constructed directly — there is no code path in the crate that can produce another bind address | `lib.rs:190–192` |
| Unknown route → `404 no_such_route`, whose `detail` carries `{path, routes}` — the full route list | `router.rs:165–170` |

### The error envelope

Success (`router.rs:181–195`): the answer's fields are **merged into** the envelope, not nested.
`{"ok": true, …the answer…}`. A non-object answer is carried under `data`.

Failure (`respond.rs:116–130`):

```json
{ "ok": false, "status": 404,
  "error": { "code": "selector_not_found", "message": "…", "detail": { … } } }
```

`detail` is frequently the most useful thing on the screen — an ambiguous selector arrives with
every candidate attached. `ui/parts.tsx:219–321` renders it as somewhere to go next; a redesign
must keep that. Discarding `detail` and printing "404" throws away the answer.

---

## 6. The features

### 6.1 Index overview — is this graph still true?

| | |
|---|---|
| **user goal** | "Does this index still describe my repository, and how big is it?" |
| **route or current surface** | `#/overview` (`routing.ts:62–63`), plus the always-visible rail counts in `App.tsx:132–180` |
| **backend service** | `nerve_store::status` (`query.rs:132`) + `nerve_index::index_freshness` (`inspect.rs:106`) + `nerve_store::database_bytes` |
| **HTTP endpoint** | `GET /api/overview` — **no parameters** |
| **MCP equivalent** | `none`. Repository identity is carried on every tool answer instead, as `repository_content.repository` (`mcp.rs:244–255`): `repo_id`, `project_id`, `root_path`, `state_id`, `git_commit`, `schema_version`, `supported_schema_version`. There is no counts-or-freshness tool |
| **CLI equivalent** | `nerve status [PATH]` / `nerve status --path PATH`, `--json` |
| **required inputs** | none |
| **response model** | `schema_version: number\|null` · `supported_schema_version: number` · `healthy: bool` · `project_id: string\|null` · `root_path: string\|null` · `state_id: string\|null` · `git_commit: string\|null` · `database_bytes: number\|null` · `entities_total: number` · `symbols_total: number` · `entities_by_kind: Record<string,number>` · `assertions_total: number` · `assertions_by_relation: Record<string,number>` · `occurrences_total: number` · `observations_total: number` · `assertion_states_total: number` · `unresolved_entities: number` · `unresolved_assertions: number` · `last_run: RunSummary\|null` · `runs: RunSummary[]` · `freshness: FreshnessReport\|null` (`api.rs:135–157`).<br>`RunSummary` = `run_id: number` · `state_id` · `extractor_id` · `extractor_version` · `started_at` · `finished_at: string\|null` · `files_processed` · `files_failed` · `status` (`api.rs:160–172`).<br>`FreshnessReport` = `files_total` · `files_probed` · `fresh` · `stale` · `missing` · `refused` · `unreadable` · `truncated: bool` (`api.rs:121–130`, `inspect.rs:82–99`) |
| **main actions** | read the verdict; navigate into Symbols / Unresolved / Coverage |
| **loading state** | `Loading label="Measuring the index"` (`Overview.tsx:103`). Real work: the server re-hashes up to 5 000 files on this request |
| **empty state** | `entities_total === 0` → "This index is empty" (`Overview.tsx:111–131`). A database exists with nothing in it. Offer `nerve index`. Distinct from *no database*, which fails at open with `ServerError::NotIndexed` before the server binds (`lib.rs:200–202`) |
| **error state** | `Failure` with retry (`Overview.tsx:104`). `TransportError` means the process is gone — the token died with it |
| **partial state** | `freshness.truncated === true`: the sweep stopped at `FRESHNESS_PROBE_CAP = 5 000` files (`api.rs:46`). Rendered as `partial sweep · N of M files` (`Overview.tsx:148–153`). **A truncated sweep must never be reported as clean** |
| **stale state** | `freshness.stale + freshness.missing > 0`. The rail says `N files have drifted` (`App.tsx:172`); the verdict sentence names the count (`Overview.tsx:49–52`) |
| **unsupported state** | `schema_version !== supported_schema_version` → `healthy` is false. `nerve doctor` diagnoses which direction. The overview shows an `unhealthy` chip and both numbers (`Overview.tsx:204–215`) |
| **ambiguity state** | cannot occur — the endpoint takes no selector |
| **pagination or bounds** | freshness sweep capped at 5 000 files, reported via `files_probed` vs `files_total` and `truncated`. Every other field is a scalar or a complete tally. No paging |
| **evidence fields** | repository state (`state_id`, `git_commit`, `project_id`) · freshness · extractor id · extractor version · kind (via `entities_by_kind`) |
| **recommended user wording** | Print `symbols_total` — never `entities_total` — beside the word **Symbols**. `entities_total` counts the repository entity itself, every directory, every coverage run and every unresolved reference. `freshness.refused` and `freshness.unreadable` are **not** drift: they are files whose currency could not be established, and the honest sentence is "could not be checked", never "unchanged" (`Overview.tsx:41–48`). Say that freshness was "measured on this request, by re-hashing the files" — it is not a stored flag |
| **security constraints** | `root_path` is an absolute filesystem path from the machine's own disk. It is already the user's own path, but do not put it anywhere copyable-by-accident in a shared context |
| **screens or components** | verdict banner · stacked freshness gauge · figure grid · two horizontal-bar tallies · index-state definition list · extractor-run list |

---

### 6.2 Search — find a thing by name

| | |
|---|---|
| **user goal** | "I half-remember what this is called." |
| **route or current surface** | `#/search?q=<text>&kind=<kind>` (`routing.ts:64–71`). Also the always-present bar omnibox, focused with `/` from anywhere (`Omnibox.tsx:49–56`) |
| **backend service** | `nerve_store::search_entities` (`query.rs:706`), FTS5 over `entity_fts` |
| **HTTP endpoint** | `GET /api/search?q=<text>&kind=<kind>&limit=<n>` |
| **MCP equivalent** | `nerve_search`. Arguments `query` (required), `kind`, `limit` (`mcp/search.rs:42`) |
| **CLI equivalent** | `nerve search <QUERY> [--kind K] [--limit N] [--path P]` |
| **required inputs** | `q`. Absent → `400 bad_request` "q is required" (`api.rs:178–180`) |
| **response model** | `query: string` · `kind: string\|null` · `limit: number` · `count: number` · `results: SearchHit[]`.<br>`SearchHit` = `entity_id` · `kind` · `name` · `scope_path` · `language: string\|null` · `file_path: string\|null` · `start_line: number\|null` · `end_line: number\|null` · `score: number` (`shapes.rs:49–61`) |
| **main actions** | type; filter by kind; arrow-key through hits; Enter opens an entity |
| **loading state** | `Loading label="Searching"` (`Search.tsx:129`). Input is debounced 160 ms in the view, 140 ms in the omnibox (`Search.tsx:31`, `Omnibox.tsx:36`) |
| **empty state** | Two different empties, and the difference matters. **No query yet** → "Type to search", stating the matching rule (`Search.tsx:118–127`). **Query, no hits** → "Nothing starts with that", plus alternative queries built from the user's own string by `search.ts`'s `widenings` (`Search.tsx:185–219`). Every suggestion is a visible query, never a hidden rewrite. A query with no alphanumeric token returns zero rows rather than an error (`query.rs:713–715`) |
| **error state** | `Failure` with retry. `400 unknown_kind` carries `detail.allowed` — the full kind vocabulary — which `parts.tsx:303–312` renders as chips |
| **partial state** | `count === limit`: the store stopped at the cap. The view says `first N matches` rather than `N matches` (`Search.tsx:138–140`); the omnibox says `first 8` (`Omnibox.tsx:162`). **There is no total**, so "N matches" on a full page would be a completeness claim the query cannot make. The MCP tool names this `limit_reached` and deliberately does **not** emit `truncated` (`mcp/search.rs:196–221`) |
| **stale state** | cannot occur on this response — a search hit carries no `content_hash` and no freshness. A hit is a lexical match, not evidence, so there is nothing for freshness to be about. The index as a whole may be stale; that is §6.1's job |
| **unsupported state** | cannot occur — `kind` is validated against the closed vocabulary before the query runs, and any other malformed input is a `400` |
| **ambiguity state** | cannot occur — search *returns* many results by design; ambiguity is a selector concept |
| **pagination or bounds** | `limit` default 20, clamped to `MAX_SEARCH_LIMIT = 200` (`api.rs:28`). **No offset.** MCP: default 20, max 100, `next_offset` always `null`, `continuable` always `false` (`mcp/search.rs:36–39`, `217–219`). UI asks for 60 in the view (`Search.tsx:20`) and 8 in the omnibox (`Omnibox.tsx:21`). Non-numeric `limit` is an error, not a silent default (`request.rs:83–93`) |
| **evidence fields** | entity identity · kind · source location. **No evidence type, no freshness, no directness** — deliberately |
| **recommended user wording** | State the matching rule where the user can see it: **each term is matched as a prefix, and every term must match**. Searching `area` finds `Circle.area`; searching `rea` finds nothing (`Search.tsx:121`). FTS5 operators (`OR`, `NEAR`, `*`, quotes) are **inert** — tokenised like any other text, not interpreted (`mcp/search.rs:45–50`). `score` is BM25 rank: **lower is a better lexical match**, and it is a text-similarity number, not evidence and not a confidence (`mcp/search.rs:53–58`). Never draw it as a bar or a percentage. On no hits, say the absence is in *Nerve's index* — the index may be stale, or the name may live in a file Nerve does not parse — never "no such symbol exists" |
| **security constraints** | the query never reaches FTS5 verbatim: `nerve-store` splits it into alphanumeric tokens and quotes each as a phrase, so operator characters are inert rather than an injection surface. Every hit field is repository text and must be rendered as a child, never as markup |
| **screens or components** | debounced text field · kind filter chip row · keyboard-navigable hit list · combobox popover with `role="listbox"` · empty state with generated widenings |

---

### 6.3 Entity detail — what is this, and what is it connected to?

| | |
|---|---|
| **user goal** | "What is this thing, where does it live, and what touches it?" |
| **route or current surface** | `#/entity/<id>/relations` — the default tab (`routing.ts:44`). Four tabs: `relations` `evidence` `graph` `source` (`routing.ts:12–14`) |
| **backend service** | `nerve_store::occurrences_of` (`query.rs:297`) + `nerve_store::entity_relation_counts` (`query.rs:337`) + `nerve_store::neighbourhood` at depth 1 over `DEFINES`/`CONTAINS` (`api.rs:226–234`) |
| **HTTP endpoint** | `GET /api/entity?selector=<selector>` |
| **MCP equivalent** | `none`. `nerve_investigate` answers the adjacent question — the assertions around a subject with full evidence — but returns no `occurrences` and no `relation_counts`. A UI cannot get occurrence spans over MCP |
| **CLI equivalent** | `none` |
| **required inputs** | `selector`. Absent → `400 bad_request` |
| **response model** | `entity: Entity` · `occurrence_count: number` · `occurrences: Occurrence[]` · `relation_counts: { outgoing: Record<string,number>, incoming: Record<string,number> }` · `defining_edges: Neighbourhood` · `selectors: { selector: { matched_by, alternatives } }` (`api.rs:236–247`).<br>`Entity` = `entity_id` · `kind` · `name` · `scope_path` · `qualified_name` · `language: string\|null` · `file_path: string\|null` · `start_line: number\|null` · `end_line: number\|null` (`shapes.rs:19–31`).<br>`Occurrence` = `occurrence_id` · `file_path` · `start_byte` · `end_byte` · `start_line` · `start_col` · `end_line` · `end_col` · `content_hash` (`shapes.rs:34–46`) |
| **main actions** | switch tab; open a related entity; jump to the evidence for one specific assertion |
| **loading state** | `Loading label="Reading this entity"` (`Entity.tsx:46`); the relations panel then loads its own one-hop neighbourhood, `"Reading one hop out"` (`Entity.tsx:148`) |
| **empty state** | every relation count zero → "Nothing connects to this", and it names the second possible cause: the file may have been only partly parsed, so it points at Unresolved (`Entity.tsx:137–146`) |
| **error state** | `Failure` with retry. The four selector refusals of §6.12 all surface here |
| **partial state** | `defining_edges.truncated` / the one-hop `hood.truncated` → "N more entities are connected to this one and were left out to keep the query bounded" (`Entity.tsx:229–236`). **`occurrence_count > occurrences.length` cannot occur**: `occurrences_of` has no `LIMIT` and returns every row (`query.rs:297–323`), so the two are always equal. `Source.tsx:37–44` renders a "N further occurrences were not returned" panel that the backend cannot trigger — a dead state of the §7.8 class, and it should be deleted rather than redesigned |
| **stale state** | not on this response directly. Each `Occurrence.content_hash` is the hash the file had when the occurrence was recorded; the Source tab compares it against `/api/source`'s live `content_hash` (`Source.tsx:80–91`) |
| **unsupported state** | an `unresolved` entity has no declaration, so the Source tab has nothing to show. The current UI says so explicitly rather than showing an empty pane (`Source.tsx:21–30`) |
| **ambiguity state** | `409 ambiguous_selector` with `detail.candidates[]` — see §6.12 |
| **pagination or bounds** | `defining_edges` uses `max_depth: 1` and `max_nodes: MAX_NEIGHBOURHOOD_NODES = 500` (`api.rs:226–232`). The relations panel asks for `depth=1&max_nodes=200` (`Entity.tsx:128`). `relation_counts` are exact and unbounded. `occurrences` is unbounded |
| **evidence fields** | entity identity · kind · source location · repository state (via occurrence `content_hash`) · ambiguity · unresolved state (`is_unresolved` on each edge) |
| **recommended user wording** | Group the one-hop edges **by relation and by direction**, and head each group with a verb from `RELATION_VERB` — "calls" and "is called by" are different questions and a `source / relation / target` table asks neither (`Entity.tsx:154–192`). `relation_counts` are exact; the *listed* other ends come from a bounded query, so the two numbers can legitimately differ and the difference must be explained, not hidden |
| **security constraints** | every string on this response is repository text. `qualified_name` is the most prominent string on the screen and is rendered verbatim |
| **screens or components** | entity header with kind chip, language chip and location · tab bar with counts · relation groups with verb headings · "Defined by" structural panel |

---

### 6.4 Evidence — why does Nerve believe this?

| | |
|---|---|
| **user goal** | "Something says A calls B. Why should I believe it, and is it still true?" |
| **route or current surface** | `#/entity/<id>/evidence`, optionally `?object=<id>&relation=<R>` to narrow to one relationship (`Evidence.tsx:46`, linked from `Entity.tsx:201–207`) |
| **backend service** | `nerve_store::explain` (`graph.rs`), which re-reads the repository through the `RepositoryProber` to compute freshness (`api.rs:335–346`) |
| **HTTP endpoint** | `GET /api/why?subject=<selector>&object=<selector>&direction=<both\|outgoing\|incoming>&relation=<R,R,…>` |
| **MCP equivalent** | `nerve_investigate`. Arguments `selector` (required), `object`, `direction`, `relations`, `limit`, `offset` (`mcp/investigate.rs:49–56`) |
| **CLI equivalent** | `nerve why <FROM> [TO] [--incoming\|--outgoing] [--relation R]… [--path P]`, `--json` |
| **required inputs** | `subject` |
| **response model** | `subject: Entity` · `object: Entity\|null` · `files_probed: number` · `count: number` · `assertions: Assertion[]` · `direction: string` · `relations: string[]` · `selectors: {…}` (`shapes.rs:170–178`, `api.rs:348–355`).<br>`Assertion` = `assertion_id` · `relation` · `direction` (`outgoing`\|`incoming`, relative to the subject) · `source: Entity` · `target: Entity` · `status` · `is_unresolved: bool` · `observation_count: number` · `strongest_source_type` · `observations: Observation[]` (`shapes.rs:154–167`).<br>`Observation` = `observation_id: number` · `evidence_source_type` · `directness` · `extractor_id` · `extractor_version` · `match_quality: string\|null` · `state_id` · `file_path: string\|null` · `start_line: number\|null` · `end_line: number\|null` · `content_hash: string\|null` · `environment: string\|null` · `details: Json` · `created_at` · `freshness` (`shapes.rs:133–151`) |
| **main actions** | filter by direction and relation; expand one observation to see how it was read; compare the two hashes; read the marked source lines |
| **loading state** | `Loading label="Gathering the evidence"` (`Evidence.tsx:52`). Then, per expanded observation, a second `/api/source` read: "re-reading `<path>`…" (`Evidence.tsx:412`) |
| **empty state** | `count === 0` → "No assertion involves this entity … There is nothing to explain" (`Evidence.tsx:65–74`). MCP names this `evidence.state: "absent"` with a statement rather than an empty list (`mcp/investigate.rs:357–361`). **Also a filter-empty**: relations exist but none matches the chosen direction+relation, which says so with the true total (`Evidence.tsx:142–149`) — never conflate the two |
| **error state** | `Failure` with retry. `400 unknown_direction` carries `detail.allowed = ["both","outgoing","incoming"]` (`api.rs:322–328`) |
| **partial state** | HTTP: **none** — `/api/why` returns every matching assertion and every observation, unbounded. MCP bounds it three ways and says so: `assertion_limit_applied` / `assertions_total` / `assertions_returned` / `truncated` / `next_offset` / `continuable`, plus `observation_limit_per_assertion = 20` with `observations_truncated` per assertion and `observation_count` left as the true total, plus `byte_limited` for the 128 KiB ceiling (`mcp/investigate.rs:314–345`). A UI paging this itself must therefore invent its own paging; there is no offset on the HTTP endpoint |
| **stale state** | per observation, `freshness` ∈ `fresh` `stale` `file-missing` `refused` `unreadable`. The current UI shows it as the arithmetic it is — recorded hash beside on-disk hash beside a verdict sentence (`Evidence.tsx:386–454`). §7.3 |
| **unsupported state** | `match_quality: null` means this kind of evidence is not matched by name, so there is no match quality to record — not "unknown" (`Evidence.tsx:313–317`). `file_path: null` means the observation records no file, so freshness cannot be measured at all (`Evidence.tsx:396–403`) |
| **ambiguity state** | `409 ambiguous_selector` on either selector. §6.12 |
| **pagination or bounds** | HTTP: unbounded. MCP: assertions default 20 / max 100, `offset` max 100 000, 20 observations per assertion, 128 KiB answer ceiling (`mcp/investigate.rs:37–46`). `next_offset` is `null` when a page came back empty because one record exceeded the byte ceiling — advancing by zero would ask the same question forever (`mcp/investigate.rs:322–329`) |
| **evidence fields** | **all of them except coverage run and parse health**: entity identity · kind · repository state (`state_id`) · source location · evidence type · directness · resolution method (`match_quality`) · extractor id · extractor version · freshness · ambiguity · unresolved state. Coverage evidence *does* appear here when a `COVERS` assertion is in scope, carried as an ordinary observation |
| **recommended user wording** | Write the assertion as a **sentence** — "Circle defines Circle.area" — because reading it aloud is how a reader notices it is wrong (`Evidence.tsx:11–14`). Show freshness as the comparison, not a badge: a badge asks for trust, two hashes and a verdict give the reader the check. Name the extractor **and its version**: a different version is a different witness. `strongest_source_type` is the *structurally most direct* source type, **not** the truest — ADR-0003 is explicit that ranking is a query-time policy. Never total the observation counts into a score |
| **security constraints** | `details` is an extractor-written JSON blob living in a file on disk Nerve does not own exclusively. A value that does not parse is surfaced as the text it is, never dropped or guessed (`shapes.rs:297–299`). Render it as labelled facts when shallow, formatted text when not — never as markup |
| **screens or components** | claim card with a sentence header · direction segmented control · relation chip filter · observation "spine" with per-directness fill · expandable fact grid · two-hash check block with verdict · code snippet with the observed range marked |

---

### 6.5 Neighbourhood — draw what is around this

| | |
|---|---|
| **user goal** | "Show me the shape of what is near this." |
| **route or current surface** | `#/entity/<id>/graph?depth=&max_nodes=&direction=&resolved_only=&relation=` (`Graph.tsx:80–107`) |
| **backend service** | `nerve_store::neighbourhood` (`graph.rs`) |
| **HTTP endpoint** | `GET /api/neighbourhood?selector=<s>&depth=<1-4>&max_nodes=<n>&direction=<any\|forward>&relation=<R,R>&resolved_only=<1\|true>` |
| **MCP equivalent** | `none`. No tool draws a graph; a picture is not a useful shape for an agent, and `nerve_path` covers "how are these two related" |
| **CLI equivalent** | `none` |
| **required inputs** | `selector` |
| **response model** | `focus: Entity` · `max_depth: number` · `max_nodes: number` · `truncated: bool` · `omitted_nodes: number` · `frontier_nodes: number` · `node_count: number` · `edge_count: number` · `nodes: {depth: number, entity: Entity}[]` · `edges: NeighbourEdge[]` · `direction: string` · `relations: string[]` · `resolved_only: bool` · `selectors: {…}` (`shapes.rs:114–130`, `api.rs:268–272`).<br>`NeighbourEdge` = `assertion_id` · `relation` · `source_entity_id` · `target_entity_id` · `is_unresolved: bool` · `status` · `strongest_source_type` · `observation_count` · `file_path: string\|null` · `start_line: number\|null` (`shapes.rs:98–111`) |
| **main actions** | change hops (1–4) and node budget (25/60/150/500); toggle "follow direction" and "hide unresolved"; toggle relations; click a node or an edge to read it; re-centre on a node |
| **loading state** | `Loading label="Expanding the neighbourhood"` (`Graph.tsx:195`) |
| **empty state** | `node_count <= 1` → "Nothing within reach", and it names the fix: widen the relations, or turn off the direction and unresolved filters (`Graph.tsx:200–204`). The focus itself always counts as one node, so `node_count === 1` is the isolated case |
| **error state** | `Failure` inside the panel body (`Graph.tsx:197–199`) |
| **partial state** | **the important one.** `truncated: true` means the **node budget** refused at least one neighbour, and `omitted_nodes` is how many distinct entities were refused a slot (`graph.rs:539–563`). The current drawing puts a dashed arc outside the outer ring labelled `N more not drawn` (`Graph.tsx:288–305`) and the footer reads `N of M entities` (`Graph.tsx:380–383`). **`truncated` is deliberately not set merely because the depth bound was reached** — a caller who asked for depth 1 and got all of depth 1 got a complete answer. What lies past the boundary is `frontier_nodes`: nodes admitted at the depth bound whose own neighbours were never looked at. That is an invitation to expand, not a warning that something was dropped |
| **stale state** | cannot occur on this response — a neighbourhood edge carries no `content_hash` and no `freshness`. It carries `file_path` and `start_line` of a representative observation, and the evidence tab is where currency is established. Do not draw a freshness badge here |
| **unsupported state** | `depth > 4` is clamped to `MAX_NEIGHBOURHOOD_DEPTH = 4`, "beyond this a picture stops being a picture" (`api.rs:30`). The applied value comes back in `max_depth`, so the control must reflect the response, not the request |
| **ambiguity state** | `409 ambiguous_selector`. §6.12 |
| **pagination or bounds** | `depth` default 1, max 4. `max_nodes` default 60, max `MAX_NEIGHBOURHOOD_NODES = 500` (`api.rs:30–32`). No offset — a graph is not paged. Both applied values are echoed |
| **evidence fields** | entity identity · kind · source location · evidence type (`strongest_source_type`) · unresolved state (`is_unresolved`) · ambiguity |
| **recommended user wording** | **This is never "the repository".** It is a bounded expansion around one focused entity, and the bound is part of the picture rather than a footnote (`Graph.tsx:1–13`). Distance from the centre is **depth and nothing else** — there is no force simulation, the layout is pure, and the same neighbourhood always draws the same way, which is what makes a screenshot of it mean something. Say how many were not drawn, in the drawing. A graph that quietly drops nodes is worse than no graph, because it looks like an answer |
| **security constraints** | node labels are repository text, set as SVG `<text>` children |
| **screens or components** | concentric-ring SVG with per-depth radius · dashed omitted arc with count · edge styling for `is_unresolved` · selection dimming (selection plus one hop lit, everything else dimmed) · hops and budget segmented controls · relation toggle row · selection detail panel |

---

### 6.6 Path — how are these two connected?

| | |
|---|---|
| **user goal** | "Is there a route from here to there, and what does it go through?" |
| **route or current surface** | `#/entity/<id>/graph?to=<selector>&max_depth=<n>` — the "path to something" mode of the graph tab (`Graph.tsx:41`, `60–68`, `71–72`; view in `Path.tsx:23`). **Reachable but undiscoverable** — §3 gap 2 |
| **backend service** | `nerve_store::find_paths` (`graph.rs`) |
| **HTTP endpoint** | `GET /api/path?from=<s>&to=<s>&max_depth=<1-32>&limit=<1-25>&direction=<any\|forward>&relation=<R,R>&resolved_only=<1\|true>` |
| **MCP equivalent** | `nerve_path`. Arguments `from`, `to` (both required), `max_depth`, `limit`, `direction`, `relations`, `resolved_only` (`mcp/path.rs:43–51`) |
| **CLI equivalent** | `nerve path <FROM> <TO> [--max-depth N] [--limit N] [--relation R]… [--direction forward\|any] [--resolved-only] [--path P]`, `--json` |
| **required inputs** | `from` and `to` |
| **response model** | `from: Entity` · `to: Entity` · `max_depth: number` · `truncated: bool` · `expansions: number` · `count: number` · `paths: FoundPath[]` · `limit` · `direction` · `relations` · `resolved_only` · `selectors: {…}` (`shapes.rs:81–95`, `api.rs:300–305`).<br>`FoundPath` = `length: number` · `traverses_unresolved: bool` · `hops: PathHop[]`.<br>`PathHop` = `relation` · `assertion_id` · `from: Entity` · `to: Entity` · `traversed_backwards: bool` · `is_unresolved: bool` · `status` · `strongest_source_type` · `observation_count` · `file_path: string\|null` · `start_line: number\|null` (`shapes.rs:64–78`) |
| **main actions** | name the other end (with search-backed suggestions); change the hop limit (3/6/10/16); open the evidence for one hop |
| **loading state** | `Loading label="Walking the graph"` (`Path.tsx:129`) |
| **empty state** | Two, and the difference is the point. **No target named yet** → "Name the other end" (`Path.tsx:121–127`). **`count === 0`** → "No route found", worded as a statement about *this search*: "a longer route may exist, and a route that depends on a reference Nerve could not resolve will never be found at all" (`Path.tsx:140–168`), with a link to Unresolved. MCP splits the two absences into two different statements depending on `search_truncated` (`mcp/path.rs:303–335`) |
| **error state** | `Failure` with retry. The UI treats *any* error as "the selector needs picking" and shows search-backed candidates until the server accepts it (`Path.tsx:63`, `96–118`) |
| **partial state** | `truncated: true` means the **search budget** stopped the walk before it was exhaustive — `MAX_EXPANSIONS = 100 000` partial paths, `MAX_FRONTIER = 200 000` (`graph.rs:27–30`). `expansions` is how many were actually expanded. `count < limit` with `truncated` false means the search ran out of graph, which is a much stronger statement |
| **stale state** | cannot occur on this response — a hop carries no `content_hash` and no freshness, only the location of a representative observation. Currency is established per hop by following through to the evidence tab |
| **unsupported state** | `max_depth` clamped to `MAX_PATH_DEPTH = 32`, `limit` to `MAX_PATH_LIMIT = 25` (`api.rs:34–36`). Applied values are echoed |
| **ambiguity state** | `409` on either end, with `detail.parameter` naming which one (`api.rs:612–622`). §6.12 |
| **pagination or bounds** | `max_depth` default 6 / max 32; `limit` default 3 / max 25. MCP: same ceilings, `next_offset` always `null`, `continuable` always `false` (`mcp/path.rs:290–292`). No offset on any surface |
| **evidence fields** | entity identity · kind · source location · evidence type · unresolved state · ambiguity |
| **recommended user wording** | **`traversed_backwards` is the field a redesign must not drop.** "A is reachable from B" is a far weaker statement than "A calls B", and a reader who cannot tell them apart will over-read the picture (`Path.tsx:1–11`). Render a backwards hop with the *incoming* verb and a distinct arrow, and label the whole route "not all one way" (`Path.tsx:210–217`). **"No path" from a bounded search means "none within this many hops", never "none exists"** — and if `truncated` is also true it means less than that: the search gave up and established nothing at all. Say which of the two happened. `traverses_unresolved` marks a chain resting on an edge whose target resolved to nothing |
| **security constraints** | an empty `relation` list on this endpoint means **every relation, including `CONTAINS` and `DEFINES`** — the opposite of what empty means to `/api/impact`. A structural route through the repository entity is real and usually useless; a UI offering a relation filter should say what "all" includes |
| **screens or components** | target field with search-backed picker · hop-limit segmented control · one card per route · ordered chain with per-hop verb, arrow direction, observation count and "why?" link · warning chips for unresolved and backwards traversal |

---

### 6.7 Source — show me the actual bytes

| | |
|---|---|
| **user goal** | "Show me the code this claim was read from, and tell me whether it is still there." |
| **route or current surface** | `#/entity/<id>/source` (`Source.tsx:20`), and inside every expanded observation on the evidence tab (`Evidence.tsx:394`) |
| **backend service** | `nerve_store::path_is_indexed` (`query.rs:490`) then `RepositoryProber::read_snippet` (`nerve-index/src/probe.rs`) |
| **HTTP endpoint** | `GET /api/source?path=<rel_path>&start_line=<n>&end_line=<n>` |
| **MCP equivalent** | `none`, and the absence is a security posture rather than a gap: the tool surface returns evidence about code, and an agent that can read arbitrary indexed file content through the same channel that carries untrusted repository text is a larger blast radius than the surface needs. **UNVERIFIED —** no document found stating this refusal explicitly; the omission is real but I could not locate a written decision for it |
| **CLI equivalent** | `none` |
| **required inputs** | `path`. Absent → `400 bad_request` "path is required" |
| **response model** | `path: string` · `start_line: number` · `end_line: number` · `total_lines: number` · `truncated: bool` · `content_hash: string` · `max_lines: number` · `max_bytes: number` · `text: string` (`api.rs:403–413`) |
| **main actions** | read; compare the hash against the recorded one |
| **loading state** | `Loading label="Reading the file"` (`Source.tsx:70`) |
| **empty state** | `occurrence_count === 0` on the entity, handled before the endpoint is called: an unresolved reference is named by what was *written*, not by where anything is declared, so there is no declaration to show (`Source.tsx:21–30`) |
| **error state** | **four distinct refusals, and each must render differently** — a refusal is never disguised as a miss (`api.rs:414–433`): `403 not_indexed` (the path is not in the index; **the filesystem was never touched**) · `403 path_refused` (the repository path guard refused it; nothing was read) · `404 file_missing` (the indexed file no longer exists) · `409 file_unreadable` (it exists but is not UTF-8 text within the size ceiling) |
| **partial state** | `truncated: true` — the requested range exceeded `max_lines` (2 000) or `max_bytes` (256 KiB) and was cut. Rendered as a `snippet truncated` chip (`Source.tsx:93`). `max_lines` and `max_bytes` come back on every response so the UI never hard-codes them (`api.rs:410–411`) |
| **stale state** | `content_hash` is the hash of the file **right now**. Compare it against the `Occurrence.content_hash` or `Observation.content_hash` you already hold: identical → "the file still hashes to what was indexed"; different → "the file has changed since it was indexed, so these line numbers may no longer point at the same code" (`Source.tsx:80–91`) |
| **unsupported state** | a binary or non-UTF-8 file is `409 file_unreadable`, not an empty snippet |
| **ambiguity state** | cannot occur — the input is a literal repository-relative path, not a selector. There is no resolution stage to be ambiguous |
| **pagination or bounds** | `start_line` default 1; `end_line` default `start_line + 39`, i.e. a 40-line window (`api.rs:378–383`). Clamped by the prober, not the handler, because the ceiling belongs with the code that owns the repository root: `MAX_SNIPPET_LINES = 2 000`, `MAX_SNIPPET_BYTES = 256 KiB` (`probe.rs:68`, `probe.rs:71`) |
| **evidence fields** | source location · repository state (`content_hash`) · freshness (derived by comparison, here) · parse health (indirectly: a file listed in `/api/partial-parses` should be read with suspicion) |
| **recommended user wording** | Say the comparison out loud: two hashes and a verdict sentence, not a badge. Mark the observed line range inside the snippet so the reader can see which lines the claim rests on (`Evidence.tsx:457–473`). When the read is refused, name **which check fired** — "not indexed", "refused by the path guard", "the file is gone", "could not be read as text" are four different facts |
| **security constraints** | **Two independent gates, in this order, neither sufficient alone and neither skipped** (`api.rs:361–373`). (1) The path must already appear in the index; a client-supplied path Nerve never indexed is refused *before the filesystem is touched at all*. (2) The read goes through `nerve-index`'s prober, which resolves the path through the same `canonical_child` choke point discovery uses — that is what catches a path that *was* indexed and has since been replaced by a symlink, a deny-listed name, or a `..` component a corrupted database could contain. `text` is raw repository content and must be rendered as a text child, never as markup |
| **screens or components** | per-occurrence panel · hash-comparison chip · numbered code block with a marked range · truncation chip |

---

### 6.8 Unresolved references — what Nerve could not work out

| | |
|---|---|
| **user goal** | "What did the indexer fail to connect, and why?" |
| **route or current surface** | `#/unresolved`, first panel (`Unresolved.tsx:42`) |
| **backend service** | `nerve_store::unresolved_entities` (`query.rs:383`) + `nerve_store::status` for the two totals |
| **HTTP endpoint** | `GET /api/unresolved?limit=<n>&offset=<n>` |
| **MCP equivalent** | `none` as a browsable list. The unresolved *account* reaches MCP through `nerve_impact`'s `evidence.unresolved` (§6.11), which is counts rather than rows — deliberately: the account exists to qualify an answer, and a per-row list invites name matching, which the project forbids |
| **CLI equivalent** | `none` |
| **required inputs** | none |
| **response model** | `limit: number` · `offset: number` · `unresolved_entities_total: number` · `unresolved_assertions_total: number` · `count: number` · `results: UnresolvedRow[]`.<br>`UnresolvedRow` = `entity_id` · `name` · `scope_path` · `meta: Json` · `referencing_assertions: number` (`api.rs:450–463`). Rows are ordered most-referenced first, then by scope and name (`query.rs:392–393`) |
| **main actions** | page; filter by reason; open one to see every place that refers to it |
| **loading state** | `Loading label="Reading what could not be resolved"` (`Unresolved.tsx:64`) |
| **empty state** | `unresolved_entities_total === 0` → "Everything resolved", **and it says this is suspicious**: "On a repository of any size that is unusual — check that indexing covered what you expected" (`Unresolved.tsx:110–119`). Do not congratulate the user here |
| **error state** | `Failure` with retry (`Unresolved.tsx:66`) |
| **partial state** | `offset + count < unresolved_entities_total` — a real page of a known total, which is why this is the one endpoint with genuine pagination |
| **stale state** | cannot occur on this response — an unresolved entity carries no `content_hash`. What went wrong is a fact about the source text at index time, and the reason code is not something freshness applies to |
| **unsupported state** | `meta.reason` may be a **code** (glossed by `UNRESOLVED_REASON`) **or already prose**. `vocab.ts:61–65` returns any value containing a space as it stands, because the indexer records some reasons as sentences and glossing those with "this build has no description" would look like a defect while saying nothing |
| **ambiguity state** | cannot occur — no selector |
| **pagination or bounds** | `limit` default 100, max `MAX_UNRESOLVED_LIMIT = 500` (`api.rs:38`). `offset` default 0, no ceiling (`api.rs:443–445`). UI pages 100 at a time (`Unresolved.tsx:28`) |
| **evidence fields** | entity identity · unresolved state · resolution method (the reason code) · source location (`scope_path`, or `meta.importer` where present) |
| **recommended user wording** | **Do not style this as a warning.** Most of these are statements about the *language* — a method call on a value whose type is not written down cannot be resolved by reading syntax, full stop — and dressing them in alarm colours trains the reader to ignore the ones that matter (`Unresolved.tsx:8–11`). Say explicitly that these are **Nerve's** gaps in knowledge, not the repository's gaps in testing, and point at Coverage for the other question — the two screens were one screen called "Gaps" until Slice 7a-ii, and a reader could ask one question and be answered the other. A reference can genuinely name **nothing** (an ADR that writes `**Supersedes:**` and then nothing): render "nothing was named", never a blank row (`Unresolved.tsx:180–190`) |
| **security constraints** | `meta` is an arbitrary JSON blob; read fields defensively and drop what fails the shape test rather than rendering a hole (`parts.tsx:197–211`) |
| **screens or components** | reason filter chip row with per-reason counts · row list with name-leading layout (a reason is a clause, not a one-word kind, and will not fit a kind-sized column) · previous/next pager with `N–M of T` |

---

### 6.9 Partial parses — which files were only half understood

| | |
|---|---|
| **user goal** | "Which files produced syntax errors, so I know what to read with suspicion?" |
| **route or current surface** | `#/unresolved`, second panel (`Unresolved.tsx:80`, `231`) |
| **backend service** | `nerve_index::partial_parses` (`inspect.rs:52`) |
| **HTTP endpoint** | `GET /api/partial-parses` — **no parameters** |
| **MCP equivalent** | `none` |
| **CLI equivalent** | `none` as a listing. `nerve index` prints the aggregate counts and the per-form breakdown (`main.rs:730–736`), and `--json` carries `unmodelled_call_sites`, `unmodelled_by_form`, `dynamic_imports_without_specifier` and `files_with_syntax_errors` (`main.rs:780–786`) |
| **required inputs** | none |
| **response model** | `count: number` · `results: PartialParseRow[]`.<br>`PartialParseRow` = `rel_path` · `language` · `content_hash` · `dynamic_imports_without_specifier: number` · `unmodelled_call_sites: number` · `unmodelled_by_form: Record<string,number>` (`api.rs:555–565`, `inspect.rs:37–50`) |
| **main actions** | read; understand which forms were skipped |
| **loading state** | the current UI renders **nothing** while this panel loads (`Unresolved.tsx:77`), because the primary panel above it already occupies the screen. A redesign may show a skeleton; it must not show an empty state during load |
| **empty state** | `count === 0` → "Every file parsed cleanly … nothing on this index was extracted from a partly understood file" (`Unresolved.tsx:232–241`) |
| **error state** | `Failure` with retry, rendered in place (`Unresolved.tsx:78`) |
| **partial state** | this endpoint **is** the partial state of everything else. Its own response is unbounded and complete. A module whose cached payload this build cannot read is skipped rather than guessed at (`inspect.rs:54–55`), so a row absent here is not proof a file parsed cleanly |
| **stale state** | `content_hash` is the hash the file had **when it was extracted**. The endpoint does no re-hashing, so a row may describe a file that has since been fixed. **The current UI prints the hash without comparing it** (`Unresolved.tsx:298`) — a redesign could compare it against `/api/source` but must not imply the comparison happened if it did not |
| **unsupported state** | `count: 0` is also what a **never-indexed** repository returns: the handler short-circuits when `repo_id` is absent (`api.rs:551–553`). "No syntax errors" and "no index" are indistinguishable on this response alone; read `/api/overview`'s `entities_total` to tell them apart |
| **ambiguity state** | cannot occur — no selector |
| **pagination or bounds** | none. Unbounded and complete |
| **evidence fields** | parse health · source location (`rel_path`) · repository state (`content_hash`) · unresolved state (the form breakdown overlaps with unresolved reasons) |
| **recommended user wording** | "The parser recovers and carries on, so these files **did** contribute to the graph — but read anything extracted from them with suspicion, because whole regions may have been skipped" (`Unresolved.tsx:252–256`). `dynamic_imports_without_specifier` is not a defect: an `import()` whose path is computed at runtime is not a fact about the source and cannot be followed by reading it |
| **screens or components** | per-file group with language · severity chips · per-form fact grid with glosses from `UNMODELLED_FORM` · content hash |

---

### 6.10 Coverage gaps — which symbols is no test known to touch?

**This feature carries the project's central honesty requirement. Read §7.1 before designing it.**

| | |
|---|---|
| **user goal** | "Which of my symbols does no test touch?" |
| **route or current surface** | `#/coverage` (`Coverage.tsx:76`) |
| **backend service** | `nerve_store::gaps` (`gaps.rs:415`), which re-reads the repository through the prober to compute freshness |
| **HTTP endpoint** | `GET /api/gaps?under=<path>&kind=<symbol_kind>&include_partial=<1\|true>&limit=<n>` |
| **MCP equivalent** | `nerve_gaps`. Arguments `under`, `kind`, `include_partial`, `limit` — **all optional**, because it is a question about the repository rather than about an entity (`mcp/gaps.rs:44`, `131`) |
| **CLI equivalent** | `nerve gaps [PATH] [--under P] [--kind K] [--include-partial] [--limit N]`, `--json`. Exits **0 whatever it finds**, including the unanswerable case: reporting a fact is not a failure (`main.rs:1776–1783`) |
| **required inputs** | none |
| **response model** | `coverage: "absent"\|"present"` · `answerable: bool` · `runs: CoverageRunRef[]` · `symbols_in_scope: number` · **`totals: GapTotals\|null`** · `limit: number` · `count: number` · `results_total: number` · `truncated: bool` · `files_probed: number` · `results: GapRow[]` · `under: string\|null` · `kind: string\|null` · `include_partial: bool` (`shapes.rs:208–231`, `api.rs:508–512`).<br>`GapTotals` = `covered` · `partial` · `uncovered` · `unmeasured` · `gaps` (= `uncovered + unmeasured`; `partial` is **never** counted here) · `stale` · `measured_files` · `stale_files` (`shapes.rs:214–223`, `gaps.rs:192–214`).<br>`GapRow` = `entity: Entity` · `state: "covered"\|"partial"\|"uncovered"\|"unmeasured"` · `coverage_freshness: string\|null` · `covered_lines: number\|null` · `instrumented_lines: number\|null` · `covered_by: string[]` (`shapes.rs:192–201`).<br>`CoverageRunRef` = `entity_id` · `report_path: string\|null` · `report_content_hash: string\|null` · `freshness: string\|null` · `source_files_in_report: number\|null` (`shapes.rs:181–189`) |
| **main actions** | toggle "include partial"; filter by state; open a symbol's evidence |
| **loading state** | `Loading label="Reading what coverage says"` (`Coverage.tsx:98`) |
| **empty state** | **Two, and they are opposite findings.** (1) `coverage === "absent"` / `answerable === false` / `totals === null` / `results: []` — *unanswerable*. The `Unanswerable` screen renders **neither a list nor a tally, not even zeroes**, says "Nerve has not been told what your tests cover", and gives the one command that changes it (`Coverage.tsx:124–164`). (2) `answerable === true` and `totals.gaps === 0` — *measured, and nothing is uncovered*: "Nothing in scope is a gap", with the caveat that it is a statement about what those reports measured, not about every test you have (`Coverage.tsx:264–286`). **These two must never render the same way.** |
| **error state** | `Failure` with retry. `400 unknown_kind` carries `detail.allowed` restricted to the four **symbol** kinds (`api.rs:481–492`) — a coverage gap is a property of a symbol |
| **partial state** | Two unrelated meanings of the word, and both are in play. (1) `truncated: true` / `count < results_total`: the row cap cut rows; **the tallies are exact regardless**, computed before the cap (`gaps.rs:563–566`), and the UI says so (`Coverage.tsx:357–366`). (2) `state === "partial"`: some instrumented lines inside the symbol ran and some did not. **`partial` is never a gap and never rounded to either neighbour** — a covered line proves the symbol was *entered*, not that it ran through (`gaps.rs:105–107`). `include_partial` decides only whether partial rows are *listed*; they are tallied either way |
| **stale state** | `coverage_freshness` per row and `freshness` per run: coverage evidence cites the **covered file's** content hash at ingestion, and `gaps.rs` re-hashes at query time. `totals.stale` counts symbols whose answer rests on coverage that no longer matches the file it measured, over `totals.stale_files` of `totals.measured_files`. Where several observations cite different hashes for one file, **the worst answer wins** — "some of this is stale" is not a fresh answer (`gaps.rs:441–449`). This applies to `uncovered` rows exactly as to `covered` ones: "the run measured this file and this symbol never ran" is only as current as the file it was measured against |
| **unsupported state** | `coverage_freshness: null` on an `unmeasured` row — there is no evidence to be fresh or stale about (`gaps.rs:225–227`), rendered as "no evidence to date" (`Coverage.tsx:388–394`). `run.freshness: null` when the run has no path *or* no recorded hash — "freshness unmeasurable" (`Coverage.tsx:447–453`). `source_files_in_report: null` when the report did not record it. `covered_lines`/`instrumented_lines` `null` when the report recorded no line counts for the symbol |
| **ambiguity state** | cannot occur — `under` is a path prefix compared on a `/` boundary using `substr`, never a `LIKE` pattern and never a selector, so there is no resolution stage to be ambiguous (`gaps.rs:378–391`) |
| **pagination or bounds** | HTTP `limit` default 100, max `MAX_GAPS_LIMIT = 500` (`api.rs:40`). CLI default 50. MCP default 20, max 100, `next_offset` always `null`, `continuable` always `false` (`mcp/gaps.rs:38–41`, `288–290`). UI asks for 200 (`Coverage.tsx:39`). **No offset on any surface** — narrow with `under` or `kind` instead |
| **evidence fields** | entity identity · kind · source location · coverage run · freshness · partial state · repository state (`report_content_hash`) |
| **recommended user wording** | The four states have four different sentences and they are already written, in `vocab.ts:124–132` — quote them rather than inventing. `uncovered` — "a coverage run measured the file this symbol is in, and no line inside the symbol ran. **The absence is a measurement.**" `unmeasured` — "no coverage evidence names the file this symbol is in. **The absence is silence rather than a measurement**: the file may be excluded from instrumentation, or never loaded by the suite at all." A coverage report says a line executed. **It never says which test executed it, so nothing here is a call** (`Coverage.tsx:91–94`). Never label anything on this screen "tested by". The known limit of the `uncovered`/`unmeasured` distinction is stated rather than glossed over: a file that appears in the report with *every* line dead is indistinguishable from a file the report never mentioned, and lands in `unmeasured` — the weaker of the two claims, which is the right direction to be wrong in (`gaps.rs:32–39`) |
| **security constraints** | `under` is refused if it is traversal-shaped, on the MCP surface before the index is touched at all (`mcp/gaps.rs:188–194`) |
| **screens or components** | unanswerable screen (a first-class screen, not an empty state) · gap/no-gap banner · five-figure grid with `uncovered` and `unmeasured` **separate** · "two ways of not being covered" definition panel · coverage-run provenance panel with per-run freshness · state filter chips · row list with state chip, freshness chip and line counts · truncation panel |

---

### 6.11 Impact — what depends on this?

**No UI exists. This section is the whole specification for building one.**

| | |
|---|---|
| **user goal** | "If I change this, what else might break?" |
| **route or current surface** | **not in the UI.** No view, no route, no TypeScript type |
| **backend service** | `nerve_store::impact` (`impact.rs:368`), which re-reads the repository through the prober |
| **HTTP endpoint** | `GET /api/impact?subject=<selector>&max_depth=<1-32>&limit=<1-500>&relation=<R,R>` |
| **MCP equivalent** | `nerve_impact`. Arguments `selector` (required), `max_depth`, `limit`, `relations` (`mcp/impact.rs:51`) |
| **CLI equivalent** | `nerve impact <SELECTOR> [--max-depth N] [--relation R]… [--limit N] [--path P]`, `--json`. Exits **0** whatever it finds |
| **required inputs** | `subject` |
| **response model** | `subject: Entity` · `relations: string[]` (the set actually walked) · `max_depth: number` · `limit: number` · `totals: {…}` · **`unresolved: {…}`** · `count: number` · `results_total: number` · `truncated: bool` · `files_probed: number` · `results: ImpactRow[]` · `selectors: {…}` (`shapes.rs:259–291`).<br>`totals` = `entities: number` · **`by_depth: {depth, entities}[]` — an array, not an object**, because JSON object keys are strings and `"10"` sorts before `"2"` (`shapes.rs:271–274`) · `by_relation: Record<string,number>` · `by_kind: Record<string,number>` · `stale: number`.<br>`unresolved` = `sites: number` · `assertions: number` · `targets: number` · `by_category: Record<string,number>` (`shapes.rs:279–284`).<br>`ImpactRow` = `entity: Entity` · `depth: number` (never 0) · `relation` · `direction` · `reached_entity_id` · `assertion_id` · `status` · `strongest_source_type` · `observation_count` · `is_unresolved: bool` · `file_path: string\|null` · `start_line: number\|null` · `evidence_freshness: string\|null` (`shapes.rs:234–250`) |
| **main actions** | choose a subject; change depth; add or remove relations; read the unresolved account; open a dependant's evidence |
| **loading state** | the closure is computed in full within the depth bound and every reached file is re-hashed, so this is the most expensive read on the surface. `files_probed` reports the cost after the fact |
| **empty state** | `totals.entities === 0` / `results: []` → "Nothing in the index depends on this through `<relations>` within depth `<n>`" — **and the unresolved account beside it, which is exactly when it matters most** (`mcp/impact.rs:287–291`) |
| **error state** | `400 unknown_relation` carries `detail.allowed` — the full relation vocabulary (`api.rs:712–720`). Plus the four selector refusals of §6.12 |
| **partial state** | `truncated: true` / `count < results_total`: the row cap cut rows. **Every tally stays exact** — `totals.entities` is the size of the whole closure, not of the page (`impact.rs:103–110`) |
| **stale state** | `evidence_freshness` per row; `totals.stale` counts entities reached through evidence that no longer matches its file. `null` on a row means the edge's representative observation records no file to check |
| **unsupported state** | `max_depth` clamped to `MAX_IMPACT_DEPTH = 32`, `limit` to `MAX_IMPACT_LIMIT = 500` (`api.rs:42–44`). `by_category` may contain the bucket `uncategorised` for a stored category this build cannot parse (`impact.rs:267`) — render it as "Nerve could not classify this site", never as a category name |
| **ambiguity state** | `409 ambiguous_selector` with `detail.candidates[]`. §6.12 |
| **pagination or bounds** | `max_depth` default 6 / max 32; HTTP `limit` default 50 / max 500; CLI default 50; **MCP default 20 / max 100** (`mcp/impact.rs:45–48`). No offset anywhere. Both applied values echoed |
| **evidence fields** | entity identity · kind · source location · evidence type · freshness · unresolved state · ambiguity · repository state |
| **recommended user wording** | **The relation set is not what a reader will assume.** An empty `relation` list means the five defaults — `CALLS` `REFERENCES` `EXTENDS` `IMPLEMENTS` `SERVED_BY` (`impact.rs:131–139`) — **not** every relation. This is the opposite of `/api/path`. `CONTAINS` and `DEFINES` would answer that every symbol impacts the repository; `IMPORTS` would report a module that never calls the changed function; `COVERS` would put a coverage run in a list of things that depend on your function; `TEST_OBSERVED_CALL` is excluded because a trace is existential and a blast radius built on it would grow and shrink with which tests happened to run. Echo the walked set on screen. **`SERVED_BY` in the answer is a declaration, never a proof of reachability** — see §7.6. **And the unresolved account must be on every answer.** See §7.2, which is the single most important paragraph in this document |
| **security constraints** | do **not** present unresolved sites as a list of suspect callers, and do not match their names against the subject's. That is identity by coincidence; Nerve does not do it and the API deliberately gives you no data to do it with (`impact.rs:33–36`) |
| **screens or components** | subject picker (reuse the path finder's search-backed selector input) · relation toggle row **with the effective set shown** · depth control · **unresolved account panel, always rendered, never collapsed by default** · `by_depth` bar chart (array order, not key order) · `by_relation` and `by_kind` tallies · dependant row list with depth, reaching relation, freshness and an evidence link · truncation panel |

---

### 6.12 Selector resolution — naming a thing

A cross-cutting feature. Every endpoint that takes a `selector`, `subject`, `object`, `from` or `to`
resolves it through one function and refuses in one of four ways.

| | |
|---|---|
| **user goal** | "Let me type what I mean and get the thing I meant, or be told why not." |
| **route or current surface** | the path finder's `to` field is the only free-text selector input in the UI (`Path.tsx:70–80`). Everywhere else the UI passes an `entity_id` it already holds |
| **backend service** | `nerve_store::resolve_selector`, with the pre-decision in `nerve_store::selector_shape` (`select.rs:365`) |
| **HTTP endpoint** | not an endpoint — a parameter on `/api/entity`, `/api/neighbourhood`, `/api/path`, `/api/why`, `/api/impact` |
| **MCP equivalent** | the `selector` / `object` / `from` / `to` / `under` arguments of all five tools |
| **CLI equivalent** | the positional arguments of `why`, `path`, `impact`, and `gaps --under` |
| **required inputs** | one selector string |
| **response model** | on success, every answer carries `selectors`. **HTTP shape** (`api.rs:668–685`): an object keyed by query-parameter name — `{"subject": {"matched_by": "path", "alternatives": [Entity, …]}}`. `matched_by` ∈ `entity_id` `path` `path_qualified` `name`. **CLI `--json` shape** (`main.rs:2040–2055`): an **array** of `{role, selector, matched_by, alternatives}`. Same information, two shapes. **MCP shape**: the HTTP object, nested inside `repository_content.selectors` (`mcp/investigate.rs:383–391`) |
| **main actions** | type a selector; pick from candidates; accept or override an alternative reading |
| **loading state** | resolution is part of the answer's own request; there is no separate spinner |
| **empty state** | `404 selector_not_found`, whose `detail` carries `qualifier`, `excluded[]` (what the qualifier ruled out — so the refusal can say "no *module* there, there is a *document*") and `suggestions[]`. Every suggestion carries a `qualified_name` that **can be typed back as a selector**, so a UI may safely make them clickable (`api.rs:623–638`) |
| **error state** | **`400 invalid_selector`** — an unknown or empty qualifier, or an empty body. `detail.reason` ∈ `unknown_qualifier` `empty_qualifier` `empty_body`; `detail.accepted_qualifiers` is the full list. A malformed *request*, not a search that came back empty (`api.rs:639–649`) |
| **partial state** | `alternatives` non-empty: the path had **two readings** and a stated rule chose one. `src/app.ts` holds both a `Module` and a `File`; `docs/architecture.md` both a `Document` and a `File`. **Content wins, container is reported.** Every entity in `alternatives` is addressable as `<kind>:<path>` (`select.rs:209–216`). `alternatives` is `[]` for the overwhelming majority of selectors. **The UI cannot render this today** — §7.7 |
| **stale state** | cannot occur — resolution reads the index and touches no file, so there is nothing to be stale against |
| **unsupported state** | **`400 refused_selector`** — a path outside the repository root, or one containing `..`. `detail.reason` is `path_refused`. Nothing was looked up: this is a refusal, not an absence (`api.rs:650–659`, `select.rs:320–341`). On MCP it is refused as a `-32602` **before the index is queried at all** (`tool.rs:439–457`), along with control characters in the argument. **Known gap:** `./docs/architecture.md` — with a leading `./` — is correctly *not* refused but is not normalised either, so it returns `404`. A path pasted from shell tab-completion will miss |
| **ambiguity state** | **`409 ambiguous_selector`** with `detail.parameter`, `detail.selector`, `detail.matched_by` and `detail.candidates[]` — every candidate, in a stable order. **Nothing is chosen on the caller's behalf.** Silently picking one is the failure mode that makes a tool untrustworthy in exactly the situation where the caller most needs it to be right (`api.rs:580–584`) |
| **pagination or bounds** | selector length capped at `MAX_SELECTOR_BYTES = 2 KiB` on MCP (`tool.rs:64`). MCP caps candidate, suggestion and excluded lists at `MAX_CANDIDATES = 25` each and reports the true totals as `candidates_total`, `suggestions_total`, `excluded_total` (`tool.rs:170–206`) |
| **evidence fields** | entity identity · kind · ambiguity · resolution method (`matched_by`) |
| **recommended user wording** | The grammar, in full: `selector := [ qualifier ":" ] body`; `body := <entity_id> \| <rel_path> \| <rel_path> "#" <qualified_name> \| <name>`. A colon introduces a qualifier **only** when it precedes the first `/` and the first `#`, so `docs/a:b.md` is still a path (`select.rs:379–398`). `#` is the symbol separator; there is no `::` form. Qualifiers are **generated from the entity-kind vocabulary** (`select.rs:286–292`), so all 13 kinds work, plus two aliases: `symbol:` (the four symbol kinds) and `adr:` (a `document` whose `meta.adr` is true, matched on its ADR id). For a refusal, say *"That selector is refused: a path outside the repository root, or one containing `..`, is never resolved."* — and offer **no** search suggestions for it, because offering alternatives implies Nerve went looking |
| **security constraints** | **A view that maps every non-200 selector outcome onto "not found" will mislabel a refusal.** Four outcomes, four different fixes. The refusal is syntactic and never a statement about the filesystem: nothing is resolved, no filesystem call is made, and the authoritative path check remains `nerve-index`'s |
| **screens or components** | selector input with search-backed suggestions · candidate picker for `409` · clickable suggestion list for `404` · a distinct refusal presentation for `400 refused_selector` that offers nothing · an "also at this path" affordance for `alternatives` |

---

### 6.13 Initialize an index

| | |
|---|---|
| **user goal** | "Set Nerve up in this repository." |
| **route or current surface** | not in the UI — a write path, and the server has none |
| **backend service** | `nerve_index::init` |
| **HTTP endpoint** | `none`. The API is `GET`-only and proven so: the database is byte-identical before and after a UI session |
| **MCP equivalent** | `none`. The MCP connection is opened `query_only` (`mcp.rs:50–52`) |
| **CLI equivalent** | `nerve init [PATH]`, `--json`. Idempotent |
| **required inputs** | a directory |
| **response model** | `command` · `ok` · `exit_code` · `root` · `nerve_dir` · `database_path` · `project_id` · `schema_version` · `created: bool` (`main.rs:596–606`) |
| **main actions** | run it once |
| **loading state** | cannot occur in a UI — no UI |
| **empty state** | `created: false` means "already initialized", which is a success, not a no-op to hide |
| **error state** | exit `10` for a bad path, `2` for a config problem (`main.rs:560–576`) |
| **partial state** | cannot occur — init either writes the schema or fails |
| **stale state** | cannot occur — nothing has been extracted yet |
| **unsupported state** | cannot occur — a schema written by a newer Nerve is `doctor`'s subject, not `init`'s |
| **ambiguity state** | cannot occur — no selector |
| **pagination or bounds** | none |
| **evidence fields** | repository state (`project_id`, `schema_version`) |
| **recommended user wording** | this is the command a UI's "no index" empty state should name |
| **security constraints** | writes only inside `<root>/.nerve/` |
| **screens or components** | none. A UI may quote the command, as `App.tsx:58–61` and `Coverage.tsx:151` already do |

---

### 6.14 Build or update the index

| | |
|---|---|
| **user goal** | "Read my repository and build the graph." |
| **route or current surface** | not in the UI — a write path |
| **backend service** | `nerve_index::index_repository_with` |
| **HTTP endpoint** | `none` |
| **MCP equivalent** | `none` |
| **CLI equivalent** | `nerve index [PATH] [--full]`, `--json`. Incremental by default: only files that changed, and the files that import them transitively, are re-extracted |
| **required inputs** | a directory containing an index |
| **response model** | `state_id` · `git_commit` · `status` · `files_processed` · `files_failed` · `files_with_syntax_errors` · `skipped_unsupported` · `skipped_symlinks` · `denied_secrets` · `dynamic_imports_without_specifier` · `unmodelled_call_sites` · `unmodelled_by_form` · `documents_processed` · `adr_documents` · `document_sections` · `unsupported_markdown` · `unsupported_markdown_by_form` · `supersession_edges` · `supersession_cycles` · `supersession_cycle_documents` · `supersession_contradictions` · `entities_total` · `symbols_total` · `entities_by_kind` · `assertions_total` · `assertions_by_relation` · `observations_total` · `unresolved_entities` · `unresolved_assertions` · `duration_ms` · `incremental: {…}` (`main.rs:770–806`). `incremental` = `full` · `files_unchanged` · `files_modified` · `files_added` · `files_removed` · `files_resolution_changed` · `files_seeded` · `files_re_extracted` · `files_skipped_unchanged` · `files_changed` · `amplification` · `removed_paths` · `observations_removed` · `occurrences_removed` · `assertions_removed` · `entities_removed` · `assertions_derived` · `rows_written` · `identity_links_proposed` · `identity_links_recorded` (`main.rs:614–636`) |
| **main actions** | run it after editing |
| **loading state** | cannot occur in a UI |
| **empty state** | a repository with no supported files yields `entities_total` of 1 (the repository entity) upwards |
| **error state** | exit `2` if there is no index; `10` for a bad path |
| **partial state** | `status: "partial"` → **exit 3**. Some files could not be read or parsed. The human output names the count and the reason class (`main.rs:763–768`) |
| **stale state** | cannot occur — indexing is what removes staleness. Note that **editing a covered or traced file does not delete its coverage or trace evidence; it makes it *stale***, which `nerve why` reports, and which is strictly more informative than silence (`main.rs:74–76`) |
| **unsupported state** | `skipped_unsupported` counts files in languages with no extractor; `denied_secrets` counts files the deny-list refused. Both are reported whether or not anyone asked — a bound that refused something silently would be indistinguishable from a file that contained nothing |
| **ambiguity state** | cannot occur — no selector |
| **pagination or bounds** | none in the output. `amplification` is `null` when nothing changed |
| **evidence fields** | repository state · parse health · unresolved state · extractor id/version (via the run it writes) |
| **recommended user wording** | deletion is destructive, so it is reported whether or not anyone asked (`main.rs:691`). A supersession cycle is **reported, never suppressed**: each edge is individually evidenced by an explicit statement in a real file, and deleting one to make the graph acyclic would hide evidence |
| **security constraints** | writes only inside `.nerve/`. Stores no source text — only ranges and content hashes (`main.rs:26–28`) |
| **screens or components** | none |

---

### 6.15 Ingest a coverage report

| | |
|---|---|
| **user goal** | "Tell Nerve what my tests covered." |
| **route or current surface** | not in the UI — a write path. `#/coverage`'s unanswerable screen quotes the command (`Coverage.tsx:151`) |
| **backend service** | `nerve_index::ingest_coverage` |
| **HTTP endpoint** | `none` |
| **MCP equivalent** | `none` |
| **CLI equivalent** | `nerve coverage <REPORT> [PATH]`, `--json` |
| **required inputs** | a path to an LCOV report **inside the repository** |
| **response model** | `report_path` · `report_content_hash` · `coverage_run_entity_id` · `state_id` · `status` · `files_in_report` · `files_ingested` · `files_refused` · `symbols_covered` · `symbols_fully_covered` · `symbols_partially_covered` · `covered_lines` · `uncovered_lines` · `refused: Record<string,number>` · `refused_total` · `rows_written` · `observations_removed` · `occurrences_removed` · `assertions_removed` · `entities_removed` · `duration_ms` · **`per_test_attribution: false`** (`main.rs:890–917`) |
| **main actions** | run it after a test run |
| **loading state** | cannot occur in a UI |
| **empty state** | a report naming no indexed file ingests nothing and every record lands in `refused` |
| **error state** | a report path outside the root is exit `10` — a wrong argument, not an internal failure (`main.rs:567–573`) |
| **partial state** | `status: "partial"` → exit `3`: a path outside the repository, a file Nerve never indexed, a file whose bytes have moved, or anything the parser declined. **Lines that map to no symbol are *not* a partial ingestion** — that is the documented lossiness of mapping lines onto symbols, reported as a number, and treating it as a failure would make every real repository exit 3 forever (`main.rs:813–820`) |
| **stale state** | ingesting against a file whose bytes have moved since indexing is a refusal, counted in `refused` |
| **unsupported state** | LCOV only. `report_content_hash: null` means the report could not be read |
| **ambiguity state** | cannot occur — no selector |
| **pagination or bounds** | none |
| **evidence fields** | coverage run · repository state · source location · partial state |
| **recommended user wording** | the command prints it and a UI should repeat it: "Coverage is not a call graph. The source of every edge is the coverage run, **not a test**: LCOV carries no per-test attribution, so *which tests would my change affect?* is not answerable from this report" (`main.rs:882–888`). It is a separate command rather than a flag on `nerve index` on purpose: were it a flag, the ordinary post-edit `nerve index` — run without it, as it always is — would silently destroy every coverage edge in the repository |
| **security constraints** | reads **only** the report named. Nerve runs no tests, spawns no process, and looks for no report of its own |
| **screens or components** | none |

---

### 6.16 Import a test-call trace

| | |
|---|---|
| **user goal** | "Tell Nerve what my tests actually called." |
| **route or current surface** | not in the UI — a write path, and **there is no trace view, no run picker and no test-to-symbol view** |
| **backend service** | `nerve_index::ingest_trace` |
| **HTTP endpoint** | `none`. Trace observations reach the read surfaces through `/api/why`, `/api/entity` and `/api/neighbourhood`, because they are observations like any other |
| **MCP equivalent** | `none` for import. Trace observations appear through `nerve_investigate` |
| **CLI equivalent** | `nerve trace import <ARTIFACT> [PATH]`, `--json` |
| **required inputs** | a `nerve-trace/v1` artifact inside the repository, produced by the user's own tracer (`tracers/python/`) |
| **response model** | `artifact_path` · `artifact_content_hash` · `state_id` · `run_id` · **`repository_binding: "bound"\|"stale"\|"unverified"\|null`** · `completion_state` · `partial_reason` · `declared_limitations: string[]` · `status` · `records_in_artifact` · `records_accepted` · `records_unsupported` · `edges_observed` · `observations_written` · `observations_merged` · `refused` · `refused_total` · `limitations` · `limitations_total` · `rows_written` · `duration_ms` · **`runs_tests: false`** (`main.rs:1016–1043`) |
| **main actions** | run it after running your suite under the tracer |
| **loading state** | cannot occur in a UI |
| **empty state** | an artifact with no usable header yields `run_id: null` |
| **error state** | exit `10` for a path outside the root |
| **partial state** | exit `3` when anything was refused **or** when the traced run itself did not finish. The second half is the point: a script that only checked for refusals would treat an interrupted suite's trace as a complete one (`main.rs:924–929`) |
| **stale state** | `repository_binding: "stale"` — the artifact names a **different** tree |
| **unsupported state** | **`repository_binding: "unverified"` is not `stale`.** The artifact named **no** tree, so nothing was checked. Absence of verification is not verification of absence. A two-state badge here would be a lie in one direction or the other |
| **ambiguity state** | cannot occur — no selector |
| **pagination or bounds** | none |
| **evidence fields** | evidence type (`TEST_CALL_TRACE`) · extractor id/version · source location · repository state · partial state |
| **recommended user wording** | the command prints the sentence a view needs its own version of: *"A trace is existential evidence: it says this run took these edges, not that every run does, and absence of an edge is absence of observation."* Also: the endpoints are the two frames of each call — **never the test**, which is recorded on the evidence instead. A record's `count` is how many times **one run** took that edge; never label it "called N times" without naming the run. `observation.environment` holds **`runs[]`, an array** — two tests reaching one callee from one line are *one* observation naming both — and the derived scalars on it are the **weakest** value across contributing runs, not the first one's |
| **security constraints** | an artifact is untrusted input. `TEST_OBSERVED_CALL` is deliberately **not** in `/api/impact`'s default closure, partly for that reason (THREAT-MODEL T9): an edge injectable into the default closure would change what Nerve tells a user to review before a change. If a view offers a relation filter, `TEST_OBSERVED_CALL` must be **opt-in and labelled** |
| **screens or components** | none today. When built: a three-state binding badge, a run list (not a run), and an existential-evidence caveat that is not optional |

---

### 6.17 Trust verdict for CI

| | |
|---|---|
| **user goal** | "Can a script trust this index right now?" |
| **route or current surface** | not in the UI |
| **backend service** | `nerve_store::status` + `nerve_index::index_freshness` + `nerve_index::untracked_files`. **No new analysis** — `check` adds the judgement over facts the other commands already produce |
| **HTTP endpoint** | `none` |
| **MCP equivalent** | `none` |
| **CLI equivalent** | `nerve check [PATH] [--allow-stale]`, `--json` |
| **required inputs** | a directory |
| **response model** | `verdict: "current"\|"no_index"\|"unusable"\|"stale"\|"unverified"` · `reason` · `allow_stale: bool` · `downgraded: bool` · `root` · `database_path` · `schema_version` · `supported_schema_version` · `runs_running` · `freshness: {files_total, files_probed, fresh, stale, missing, refused, unreadable, truncated}\|null` · `added: number\|null` · `added_paths: string[]` · `unindexable: number\|null` (`main.rs:1499–1529`) |
| **main actions** | branch on the exit code |
| **loading state** | cannot occur in a UI |
| **empty state** | `verdict: "no_index"` → exit `2` |
| **error state** | exit `10` for bad arguments |
| **partial state** | `verdict: "unusable"` → exit `3`: the schema is behind, or a run never finished |
| **stale state** | `verdict: "stale"` → exit `4`. `--allow-stale` downgrades the **exit code** to 0 and sets `downgraded: true`; the verdict, the reason and every freshness count are unchanged (`main.rs:1503–1506`) |
| **unsupported state** | **`verdict: "unverified"`** — the sweep could not establish whether the index is current, because the 5 000-file cap bit. Same exit code as `stale`, different evidence, and deliberately a separate verdict: "I could not check" is not a clean bill of health (`main.rs:1234–1239`) |
| **ambiguity state** | cannot occur — no selector |
| **pagination or bounds** | `CHECK_PROBE_CAP = 5 000` files; the human output names at most `CHECK_ADDED_SHOWN = 10` added paths then gives a count (`main.rs:1218–1221`) |
| **evidence fields** | repository state · freshness · parse health (indirectly, via `runs_running`) |
| **recommended user wording** | `status` **reports**; `check` **judges**, and its judgement is an exit code another program branches on — the output is secondary. It never repairs, re-indexes or migrates: a command that silently fixed the thing it was asked to judge could not be trusted to judge it, so its connection is opened `query_only`. It applies **no policy**: there is no `--max-unresolved` and no `--max-gaps`, because `nerve gaps` and `nerve impact` already emit JSON a CI script can threshold for itself (`main.rs:108–137`) |
| **security constraints** | read-only by construction |
| **screens or components** | none |

---

### 6.18 Diagnose the installation

| | |
|---|---|
| **user goal** | "My tooling is misbehaving. What is wrong?" |
| **route or current surface** | not in the UI |
| **backend service** | `crates/nerve-cli/src/doctor.rs` — many checks over the installation, the database and the config |
| **HTTP endpoint** | `none` |
| **MCP equivalent** | `none` |
| **CLI equivalent** | `nerve doctor [PATH]`, `--json` |
| **required inputs** | a directory |
| **response model** | `root` · `nerve_dir` · `database_path` · `counts: {checks, ok, warning, fatal, skipped}` · `findings: {id, group, severity, checked, found, remedy}[]` (`doctor.rs:801–828`). `severity` ∈ `ok` `skipped` `warning` `fatal` (`doctor.rs:58–65`); `group` ∈ `installation` `database` `configuration` `index` (`doctor.rs:93–100`) |
| **main actions** | read the findings and the remedies |
| **loading state** | cannot occur in a UI |
| **empty state** | cannot occur — the check list is fixed, so there is always at least one finding |
| **error state** | exit `2` only if something **fatal** was found. A warning is not a failure |
| **partial state** | `severity: "skipped"` — a check that could not run, which is reported rather than counted as passing |
| **stale state** | cannot occur — `doctor` deliberately does **not** judge index freshness. That is `check`'s question (`main.rs:150–151`) |
| **unsupported state** | it runs on a **broken installation on purpose**: no database, a corrupt database, a schema written by a newer Nerve and an unparseable config are its subject matter, not reasons to bail out |
| **ambiguity state** | cannot occur — no selector |
| **pagination or bounds** | none |
| **evidence fields** | repository state · parse health (via `fts_consistency`, `migration_history`) |
| **recommended user wording** | each finding says **what was checked, what was found, how bad it is, and what to do about it** — four fields, and a UI reproducing this must keep all four. It diagnoses and never repairs; there is no `--fix`. It makes no network call of any kind |
| **security constraints** | read-only |
| **screens or components** | none |

---

### 6.19 Serve the interface

| | |
|---|---|
| **user goal** | "Let me look at this in a browser." |
| **route or current surface** | the whole UI. `GET /` and `/index.html` serve the embedded document |
| **backend service** | `nerve_server::serve` (`lib.rs:193`) |
| **HTTP endpoint** | the server itself. 11 `/api/*` routes plus 4 embedded assets |
| **MCP equivalent** | `none` — `nerve mcp` is the sibling surface, §6.20 |
| **CLI equivalent** | `nerve serve [PATH] [--port N] [--workers N]`, `--json` |
| **required inputs** | a directory that already contains an index |
| **response model** | `--json` emits `address` · `port` · `base_url` · `url` · `token` · `token_header` · `workers` · **`routes`** — the full route list (`main.rs:2819–2831`). Note: `UI-BACKEND-HANDOFF.md`'s standing-contract note describes this as `{address, token}`; it is eight fields |
| **main actions** | open the printed URL; Ctrl-C to stop |
| **loading state** | `serve` returns as soon as the socket is bound and the workers are running, so the URL can be printed before anything is served |
| **empty state** | an unindexed directory is refused **before anything binds** — `ServerError::NotIndexed`, exit `2` (`lib.rs:200–202`) |
| **error state** | exit `10` for a missing root, `2` for no index, `70` for a bind failure |
| **partial state** | cannot occur — the server is either bound or not |
| **stale state** | cannot occur for the server itself; index staleness is reported per answer |
| **unsupported state** | on a non-Unix platform the token cannot be minted (`token.rs:102–108`) and `serve` fails rather than falling back to a weaker source |
| **ambiguity state** | cannot occur |
| **pagination or bounds** | `workers` default 4, clamped to `MAX_WORKERS = 16` (`lib.rs:74`, `lib.rs:77`, `lib.rs:224`). One SQLite connection per worker |
| **evidence fields** | repository state (the identity block every answer carries) |
| **recommended user wording** | the announcement already says it: the token is required on every request, is not written to disk, dies with the process, and requests from another origin or naming another host are refused. **The API is read-only** (`main.rs:2812–2815`) |
| **security constraints** | all of §5 |
| **screens or components** | the shell: a bar that is always the way in (search), a rail that is always the way around, and one scrolling region for whatever is being read. The rail carries the counts that change how everything else should be read — how much is unresolved, whether the index is still current — because those are the facts a reader needs *before* they trust a screen, not after (`App.tsx:1–12`) |

---

### 6.20 MCP session

| | |
|---|---|
| **user goal** | "Let an agent ask Nerve about this repository." |
| **route or current surface** | not a UI. Line-delimited JSON-RPC 2.0 on stdin and stdout |
| **backend service** | the same `api` functions the HTTP surface calls, so the two cannot drift into different answers (`mcp.rs:1–7`) |
| **HTTP endpoint** | `none` — **stdio only: no socket, no port, no outbound client** |
| **MCP equivalent** | itself. Five tools: `nerve_investigate` `nerve_search` `nerve_path` `nerve_impact` `nerve_gaps` (`mcp.rs:86–92`) |
| **CLI equivalent** | `nerve mcp [PATH]` |
| **required inputs** | a directory that already contains an index |
| **response model** | every tool result has the same envelope: `tool` · `trust` · `query` · `bounds` · `evidence` · **`repository_content`** (`tool.rs:86–95`). `trust` = `repository_content_is_untrusted: true` · `untrusted_field` · `echoed_arguments_field: "query"` · `statement` · `document_derived_evidence` (`tool.rs:103–111`). The MCP tool result itself carries **two text blocks on purpose** — the first is the untrusted-content statement, the second is the answer — plus `structuredContent` and `isError` (`mcp.rs:608–617`) |
| **main actions** | `initialize`, `tools/list`, `tools/call`, `ping`, `notifications/initialized` (`mcp.rs:130–136`) |
| **loading state** | not applicable — request/response |
| **empty state** | every tool has an explicit `evidence.state: "absent"` with a statement rather than an empty list |
| **error state** | a `400` from the application layer becomes JSON-RPC `-32602`, because that is what a client fixes by sending different arguments. Everything else — an ambiguous selector's candidates, a suggestion set, an internal failure — becomes a **tool result with `isError: true`**, because it carries repository text and therefore needs the envelope, and because the candidate list is something the agent should read and act on (`tool.rs:162–206`) |
| **partial state** | `bounds.byte_limited` — the 128 KiB ceiling cut rows. It is applied **last**, because it is the only bound that depends on the size of what the earlier bounds selected, and it cuts from the end so the answer stays a **prefix** of the page and every reported count stays true of the page actually returned (`tool.rs:113–136`) |
| **stale state** | per-observation `freshness`, as on HTTP |
| **unsupported state** | an unknown protocol revision is answered with `2025-06-18` rather than failing the handshake (`mcp.rs:110–113`, `532–541`). An unknown method is `-32601` with the supported list. An **unknown argument is refused, not ignored** (`tool.rs:226–239`) — ignoring it would let a caller believe a filter was applied that never was |
| **ambiguity state** | `isError: true` with `repository_content.detail.candidates`, capped at 25 with `candidates_total` giving the truth |
| **pagination or bounds** | one input line capped at `MAX_REQUEST_BYTES = 256 KiB`, and an oversized line is **discarded as it arrives** rather than buffered and then rejected (`mcp.rs:121`, `368–419`). A client-supplied string echoed into an error is capped at `MAX_ECHO_CHARS = 128` (`mcp.rs:127`). **Three independent response bounds per tool**: the tool's own row cap, the per-record sub-cap where one exists (20 observations per assertion), and `MAX_ANSWER_BYTES = 128 KiB` measured on the **pretty-printed text a client actually reads** (`tool.rs:61`). Only `nerve_investigate` has an offset; the other four carry `next_offset: null` and `continuable: false` with a statement saying why (`tool.rs:146–150`) |
| **evidence fields** | all of them, per tool |
| **recommended user wording** | not a UI surface, but the wording is the model for one. Every string that came out of the repository is returned inside **one field**, `repository_content` — the label is **structural**, not annotational: an agent does not have to notice a flag on a span, it has to notice which subtree it is reading (`tool.rs:29–34`). Nothing in any tool description, the server instructions, or any response tells the consuming model to trust repository text; three tests assert it |
| **security constraints** | THREAT-MODEL T7 (prompt injection) and T8 (malicious tool arguments). The client is an adversary. No argument reaches SQL as text. The connection is `query_only`. A response is serialized **in full** before a single byte is written, so a failure cannot leave a half-written line on stdout. **Nothing else may write to stdout for the lifetime of the session** — not a banner, not a `--json` summary, not a progress line |
| **screens or components** | none |

---

## 7. The honesty rules

These cut across every feature. They are the product, not a style guide.

### 7.1 Absence of evidence is not negative evidence

`nerve gaps` has **four coverage states across two levels**, and collapsing any pair of them
produces a confident falsehood.

**Level one — is the question answerable at all?** (`gaps.rs:12–17`)

| response | means | must render as |
|---|---|---|
| `coverage: "absent"`, `answerable: false`, `totals: null`, `results: []` | no `CoverageRun` exists in this repository. The gap question is **unanswerable** | "Nerve has not been told what your tests cover", with no list and **no tally, not even zeroes** |
| `coverage: "present"`, `answerable: true`, `totals: {…}` | at least one report was ingested; gaps are measurable against it | the measurement |

`totals` is `None` rather than a row of zeroes because **a number that could not be computed must
not be printed as `0`** — `0` is a measurement and this is not one. The backend enforces the
distinction structurally: when coverage is absent it returns early with `totals: None` and an empty
row list rather than listing every symbol (`gaps.rs:457–472`).

**Level two — what does the evidence say about one symbol?** (`gaps.rs:99–147`)

| state | means | is a gap? |
|---|---|:--:|
| `covered` | every instrumented line inside the symbol executed | no |
| `partial` | at least one instrumented line did not execute. **Never rounded to either neighbour** — a covered line proves the symbol was *entered*, not that it ran through | no |
| `uncovered` | a coverage run named this symbol's file, and no line inside the symbol executed. **Absence here is a measurement** | yes |
| `unmeasured` | no coverage evidence names this symbol's file at all. **Absence here is silence** — the file may be excluded from instrumentation, may not be loaded by the suite, may not even be reachable | yes |

`totals.gaps === uncovered + unmeasured`. `partial` is never counted there.

**The three sentences that must never be confused:**

- `totals: null` — *"nothing was measured, so nothing can be reported as covered or uncovered."*
- `totals.gaps === 0` — *"coverage was ingested and every symbol in scope is covered or partially covered. This is a measurement, not an absence of one."*
- `state: "unmeasured"` — *"a measurement exists for this repository, and it does not reach this file."*

The same shape recurs elsewhere and takes the same treatment every time:

- **Absence of a `TEST_OBSERVED_CALL` edge** means nothing was observed, not that no call exists.
  An untraced repository has zero trace edges and a fully-traced one has some; a view that draws
  "no observed calls" the way it draws "no callers" turns missing instrumentation into an apparent
  fact about the code.
- **`repository_binding: "unverified"`** is the absence of a check, not a failed check
  (§6.16).
- **`freshness: "refused"` / `"unreadable"`** mean currency could not be established, not that the
  file is unchanged (§6.1).
- **`nerve check`'s `unverified` verdict** — "I could not check" is not a clean bill of health
  (§6.17).

### 7.2 `nerve impact` reports an unresolved account on every answer, and the UI must not hide it

`unresolved` is **a field on the report, never an `Option`**, always serialized, including when
every one of its counts is zero (`impact.rs:24–28`, `shapes.rs:252–258`).

Why: Slice 2a measured **38.1% of call sites on the resolution corpus as honestly unresolved**. Any
method call on a typed receiver — `shape.area()` — is unresolvable without type inference, and
Nerve has none. So a report reading

```
3 entities depend on parseConfig
```

will be read as *"only three things use this, it is safe to change"*, and on a repository where a
third of the reference sites resolved to nothing that reading is unsupported. The command would
then have talked someone into a breaking change, which is worse than not answering.

On `fixtures/ts-basic` with `subject=add`, the honest shape is **3 dependants beside 4 unresolved
sites**. The caveat is larger than the answer. That is not a defect in the response.

| field | is | show |
|---|---|---|
| `sites` | **observations** — individual reference sites. One `parse()` in one place is one site | **this is the number to show** |
| `assertions` | the same fact at coarser grain. `sites >= assertions` always | secondary |
| `targets` | distinct `Unresolved` entities those assertions name | secondary |
| `by_category` | split by `UnresolvedCategory`, so a broken Markdown link (`document_link`) is not read as a lost call (`value`) | show the split |

**Scope**: the whole repository, restricted to the relations walked. Repository-wide because a
hidden edge can attach anywhere; relation-restricted because counting `SUPERSEDES` markers as
potential hidden callers, when the walk never follows `SUPERSEDES`, would be an exact number
answering a different question.

**Required wording when `sites > 0`:** *"N reference sites in this repository resolved to nothing.
Any of them could reach this symbol and this answer cannot rule them out."*

**Required wording when `sites == 0`:** say so explicitly, and do **not** hide the section. The
CLI's phrasing is the one to copy, and note what it does *not* say (`main.rs:2533–2541`):

> No reference site under those relations failed to resolve. That is a count of failed
> resolutions, not of coverage: a construct Nerve does not model — or a language it does not yet
> resolve — contributes no site to count.

It deliberately does **not** say "every reference site resolved". In a repository that indexed no
reference site at all, that older wording was vacuously true while reading as a coverage claim.

**What may not be done:** matching an unresolved site's name against the subject's and presenting
the result as a probable caller. That is identity by fuzzy name matching, which this project
forbids and which ADR-0002's tuples exist to prevent. Nothing in `nerve-store` compares a name to
a name, and the API gives a UI no data to do it with.

### 7.3 Freshness is arithmetic, shown as arithmetic

`observation.content_hash` records what the file said when the observation was made.
Whether that is still true is **derived, never stored**: the file is re-hashed on disk and the two
hashes are compared (`freshness.rs:1–10`).

So show the comparison, not a verdict alone: **recorded hash · on-disk hash · verdict sentence**
(`Evidence.tsx:15–18`, `386–454`). A badge asks for trust; two hashes and a verdict give the
reader the check itself. This is a strictly stronger claim than a stored flag, because it catches a
change that was made and never indexed — and it is invisible unless the two strings are put side by
side.

Each distinct path is probed once per query however many observations quote it, and `files_probed`
on the response is the real cost (`freshness.rs:79–83`).

Where several observations cite different hashes for one file, **the worst answer wins**
(`gaps.rs:441–449`).

### 7.4 There is no confidence score, anywhere, and the UI must never invent one

The evidence model deliberately has no `confidence: float`. It has a **structured evidence
profile** (ADR-0003):

- `evidence_source_type` — how the evidence was obtained
- `directness` — `DIRECT` / `RESOLVED` / `INFERRED`
- `extractor_id` + `extractor_version` — **a different version is a different witness**
- `match_quality` — how the name was matched, `null` when this kind of evidence is not matched by
  name at all
- `state_id` — the merkle of the whole indexed tree when it was recorded
- `file_path` + `start_line` + `end_line` + `content_hash` — the exact bytes read
- `freshness` — computed now

Three specific prohibitions:

1. **Do not derive a score from `observation_count`.** Ten observations of a weak source type are
   not stronger than one `AST_DIRECT`.
2. **Do not draw `EvidenceSourceType` as a scale.** Declaration order is the structural ordering
   behind `strongest_source_type` and the stored bit mask; ADR-0003 is explicit that truth ranking
   is supplied by an evidence policy at query time (`vocab.rs:456–459`).
3. **Do not render `score` from `/api/search` as a strength.** It is BM25 lexical rank, lower is a
   better *text* match, and it says nothing about the code.

### 7.5 Truncation is always disclosed

Every bound in the product reports itself. §8 has the full table. The three shapes:

- **Row caps with an exact total** — `count` / `results_total` / `truncated`, on `/api/gaps` and
  `/api/impact`. The tallies are computed before the cap and stay exact whatever it cuts, and the
  UI must say both things.
- **Row caps with no total** — `/api/search`. There is no total because the query is the filter and
  the store stops at `limit`, so the honest signal is `count === limit` → "first N matches". The
  MCP tool names this `limit_reached` and deliberately omits `truncated`, because
  `truncated: false` would be a claim the caller has seen everything (`mcp/search.rs:196–201`).
- **Budget exhaustion** — `/api/path`'s `truncated` (the search gave up: `MAX_EXPANSIONS`,
  `MAX_FRONTIER`) and `/api/neighbourhood`'s `truncated` + `omitted_nodes` (the node budget refused
  neighbours). The neighbourhood also carries `frontier_nodes`: nodes at the depth boundary whose
  own neighbours were never examined. **`truncated` and `frontier_nodes` are different facts** —
  one is "something was dropped", the other is "there is more out there if you ask".

**The MCP surface has three independent bounds per answer, including a byte ceiling.** A tool's own
row cap; a per-record sub-cap where one exists (20 observations per assertion, with
`observation_count` left as the true total and `observations_truncated` set per assertion); and
`MAX_ANSWER_BYTES = 128 KiB` measured on the pretty-printed text. The byte ceiling is applied last
and cuts from the end, so the answer is a prefix and `next_offset` stays correct. A single record
larger than the whole ceiling ends with an **empty** row list, `byte_limited: true`, and
`next_offset: null` — because advancing by zero would ask the same question forever
(`tool.rs:113–136`, `mcp/investigate.rs:322–329`).

**The neighbourhood graph draws a bounded subset and says how many were not drawn.** The current
implementation puts the count in the drawing, as a dashed arc outside the outer ring labelled
`N more not drawn` (`Graph.tsx:288–305`). Keep that, or do something equally hard to miss. A graph
that quietly drops nodes is worse than no graph, because it looks like an answer.

### 7.6 Three relations that are never calls

| relation | what it proves | what it does not |
|---|---|---|
| `COVERS` | a coverage run executed at least one line inside a symbol | who invoked whom. Two symbols executing during one run says nothing about that. It comes **from a `CoverageRun`, never from a test** — LCOV carries no per-test attribution (ADR-0005, ADR-0008) |
| `SERVED_BY` | a framework registration exists, from the endpoint to its handler | that the route is reachable in production, that middleware permits access, that dynamic configuration has not replaced it, that a decorator-generated wrapper preserved the handler's identity, or that two matching path strings are one deployed endpoint (`vocab.rs:296–302`) |
| `TEST_OBSERVED_CALL` | one run, in one environment, took this edge | that every run does. **Existential, never universal**, and absence of the edge is absence of observation (`vocab.rs:314–317`) |

The verb pairs in `format.ts` are deliberately passive or hedged for exactly this reason —
`SERVED_BY` is `['is served by', 'serves']` and `TEST_OBSERVED_CALL` is
`['was observed calling', 'was observed called by']`. Every active alternative
(`DISPATCHES_TO`, `INVOKES`, `ROUTES_TO`, `TRACED_CALL`, `TEST_CALLS`) asserts something the
evidence does not carry, and two tests in `vocab.rs` assert that none of them parses —
`the_framework_vocabulary_states_only_what_a_registration_carries` and
`the_trace_vocabulary_states_only_what_one_execution_carries`. (Named rather than cited by line:
`vocab.rs` is appended to per slice, so its test module moves.)

Also: an `endpoint`'s `meta.path` is **the declared path, not the deployed one**. No prefix from
`APIRouter(prefix=…)` or a blueprint registration is composed in (`vocab.rs:179–185`). If a screen
ever shows it next to a real URL, it must not imply they are the same string.

### 7.7 `alternatives` is unrenderable today, and that is a real gap

`api.rs:668–685` attaches `selectors` to every answer that resolved a selector, and every one of
those answers therefore carries `alternatives`. But `apps/nerve-web/src/api/types.ts` declares no
`selectors` field on `EntityDetail` (`types.ts:176`), `Neighbourhood` (`types.ts:123`),
`PathReport` (`types.ts:160`) or `WhyReport` (`types.ts:221`), and no view reads it.

Consequence: when a user opens `src/app.ts` and Nerve resolves it to the **`Module`** while a
**`File`** also exists at that path, the interface shows the module and says nothing. The rule is
*content wins, container is reported* — and the reporting half is currently missing.

A redesign should add the field to the four mirrors and render, when `alternatives` is non-empty:
*"also at this path: file `app.ts`"*, with the passed-over entity's `file:<path>` or
`directory:<path>` selector as the link. `alternatives` is `[]` for the overwhelming majority of
selectors, so this is a rare affordance that must nonetheless exist.

### 7.8 States the backend cannot produce — do not build screens for them

Slice 5d-iii found the interface carrying a gloss for a status the backend could not emit. Two more
of that class are live today, plus one that must be glossed but must not be designed around.

**1. `occurrence_count > occurrences.length` cannot happen.**
`nerve_store::occurrences_of` has no `LIMIT` (`query.rs:297–323`), so `/api/entity` returns every
occurrence and the two numbers are always equal. `Source.tsx:37–44` renders a panel reading
*"N further occurrences were not returned"* which can never appear. Delete it rather than carry it
forward.

**2. `assertion_state.status` can only be `SUPPORTED` or `UNRESOLVED`.**
The derivation is a single SQL `CASE`: `UNRESOLVED` when the assertion's target is an `unresolved`
entity, `SUPPORTED` otherwise (`derive.rs:147`). `CONTRADICTED` requires multiple extractors
disagreeing, `STALE` requires per-state comparison the derivation no longer keeps, and `DELETED`
requires explicit retraction — none of which any writer produces (`derive.rs:211–213`).

The interface must still **gloss** all five, because `ui_vocabulary.rs:360–371` requires an entry
for every `AssertionStatus::ALL` member and the test reads the shipped TypeScript as text. But it
must not build a state, a colour, a filter or a count around the three unreachable ones. The
pinned-vocabulary test on the documentation fixture observes exactly one:
`assertion_status=SUPPORTED` (`ui_vocabulary.rs:663–679`).

Note also that the current UI colours anything that is not `SUPPORTED` with the `stale` tone
(`Evidence.tsx:212`, `Graph.tsx:475`). In practice that means `UNRESOLVED` renders in the same hue
as stale evidence, which are different facts. A separate chip does say "one end unresolved", so the
information is present — but a redesign should give `UNRESOLVED` its own tone rather than borrowing
staleness's.

**3. `endpoint` entities: "none" and "no rule yet" are indistinguishable.**
A repository with no `endpoint` entities either declares no routes, or is written in a language
whose framework rule does not exist yet (TypeScript/JavaScript routes are unimplemented). The API
cannot tell them apart today. **A UI must not say "no routes".**

### 7.9 Two direction vocabularies, and one default that differs by surface

A UI author building a shared direction control will get this wrong. There are **two** vocabularies:

| endpoint | values | default | source |
|---|---|---|---|
| `/api/neighbourhood`, `/api/path` | `any` · `forward` | **`any`** | `api.rs:687–699` |
| `/api/why` | `both` · `outgoing` · `incoming` | `both` | `api.rs:318–330` |
| `nerve path --direction` | `forward` · `any` | **`forward`** | `main.rs:249` |
| `nerve_path` (MCP) | `any` · `forward` | **`any`** | `mcp/path.rs:191` |

The `/api/why` triple is about *which side of the subject an assertion sits on*; the `any`/`forward`
pair is about *whether the walk respects the recorded edge direction*. They are genuinely different
questions and should not share a control.

**`nerve path` defaults to `forward` while `/api/path` and `nerve_path` default to `any`.** The same
question asked three ways gives three different answers on the same repository unless the caller
sets the flag. Flagged here rather than papered over.

### 7.10 The relation filter is comma-separated, not repeatable

Over HTTP, `relation` is read as a **single comma-separated value**: `target.list("relation")`
splits on `,` (`request.rs:116–122`). And `Target::parse` stores parameters in a `BTreeMap`, so a
**repeated** `relation=` parameter silently keeps only the last (`request.rs:34–35`, `67`).

`?relation=CALLS&relation=DEFINES` therefore filters on `DEFINES` alone, with no error.

`UI-BACKEND-HANDOFF.md` Entry 2 describes `relation` as "repeatable"; over HTTP it is not, and the
failure is silent. The interface's own client is correct — `query()` joins an array with commas
(`api/client.ts:64–76`) — and the CLI's `--relation` genuinely is repeatable
(`main.rs:224–225`). A redesign must use the comma form.

---

## 8. Every bound, in one table

| bound | value | where | reported back as |
|---|--:|---|---|
| search `limit` (HTTP) | default 20, max **200** | `api.rs:28` | `limit`, `count` |
| search `limit` (MCP) | default 20, max **100** | `mcp/search.rs:36–39` | `hit_limit_applied`, `hit_limit_max`, `limit_reached` |
| MCP search query length | **512 B** | `tool.rs:71` | `query_byte_limit` |
| neighbourhood `depth` | default 1, max **4** | `api.rs:30` | `max_depth` |
| neighbourhood `max_nodes` | default 60, max **500** | `api.rs:32` | `max_nodes`, `truncated`, `omitted_nodes`, `frontier_nodes` |
| path `max_depth` | default 6, max **32** | `api.rs:34` | `max_depth` |
| path `limit` | default 3, max **25** | `api.rs:36` | `limit`, `count` |
| path search budget | **100 000** expansions, **200 000** frontier | `graph.rs:27–30` | `truncated`, `expansions` |
| unresolved `limit` | default 100, max **500** | `api.rs:38` | `limit`, `offset`, `count`, `unresolved_entities_total` |
| gaps `limit` (HTTP) | default 100, max **500** | `api.rs:40` | `limit`, `count`, `results_total`, `truncated` |
| gaps `limit` (CLI) | default **50** | `main.rs:200` | same |
| gaps `limit` (MCP) | default **20**, max **100** | `mcp/gaps.rs:38–41` | `row_limit_applied`, `row_limit_max`, `rows_total` |
| impact `max_depth` | default 6, max **32** | `api.rs:42` | `max_depth` |
| impact `limit` (HTTP, CLI) | default 50, max **500** | `api.rs:44`, `main.rs:227` | `limit`, `count`, `results_total`, `truncated` |
| impact `limit` (MCP) | default **20**, max **100** | `mcp/impact.rs:45–48` | `row_limit_applied`, `row_limit_max` |
| `why` assertions (HTTP) | **unbounded** | — | `count` |
| `why` assertions (MCP) | default 20, max **100**; `offset` max **100 000** | `mcp/investigate.rs:37–43` | `assertion_limit_applied`, `assertions_total`, `next_offset`, `continuable` |
| observations per assertion (MCP) | **20** | `mcp/investigate.rs:46` | `observation_limit_per_assertion`, `observations_truncated`, and `observation_count` stays the true total |
| MCP answer size | **128 KiB**, on the pretty-printed text | `tool.rs:61` | `answer_byte_limit`, `byte_limited` |
| MCP request line | **256 KiB**, discarded as it arrives | `mcp.rs:121` | error `data.max_request_bytes` |
| MCP echoed string | **128 chars** | `mcp.rs:127` | truncated with `…` |
| MCP selector length | **2 KiB** | `tool.rs:64` | `max_bytes` in the refusal |
| MCP relation filters | **32** | `tool.rs:77` | `maximum` in the refusal |
| MCP candidates / suggestions / excluded | **25** each | `tool.rs:74` | `candidates_total`, `suggestions_total`, `excluded_total`, `candidate_limit` |
| source snippet | **2 000 lines**, **256 KiB** | `probe.rs:68`, `probe.rs:71` | `max_lines`, `max_bytes`, `truncated` |
| source default window | 40 lines | `api.rs:382` | `start_line`, `end_line` |
| freshness sweep (`/api/overview`) | **5 000 files** | `api.rs:46` | `files_probed`, `files_total`, `truncated` |
| freshness sweep (`nerve check`) | **5 000 files** | `main.rs:1218` | same, plus verdict `unverified` |
| request target | **8 KiB** | `request.rs:13` | `414 target_too_long` |
| query parameters | **32** | `request.rs:16` | `400 too_many_parameters` |
| server workers | default 4, max **16** | `lib.rs:74`, `lib.rs:77` | `workers` |

**The only continuation offset in the product is `/api/unresolved`'s `offset`, plus
`nerve_investigate`'s.** Everything else says so explicitly rather than handing back a
`next_offset` no query honours: *"narrow the query, or raise `limit` up to the stated maximum, to
see more"* (`tool.rs:146–150`).

---

## 9. Deliberately not in the UI, and why

### Intentionally absent for a security or evidence reason — do not build these

| capability | why it is refused, not missing |
|---|---|
| **`nerve affected`** — "which tests would my change affect?" | **Refused, not deferred.** LCOV carries no per-test attribution: its `TN:` field is empty and one report describes one whole run, so the only endpoint the evidence supports is the run that produced the report (ADR-0008 §A.2). If a test file appears in an impact set it is there because **code depends on code**. Do not label an impact view, or any part of one, as test impact. `scripts/final_acceptance.sh:122` **fails the build if this command ever exists** |
| **`nerve trace-tests`** — run the repository's test suite under a tracer | **Refused, not deferred.** `crates/nerve-cli/tests/no_subprocess.rs` forbids process creation in product code, and running a test suite would need an exception to it (THREAT-MODEL T1). The user runs their suite under the tracer, in their own environment, with their own secrets; Nerve reads the artifact afterwards and spawns nothing. `scripts/final_acceptance.sh:124` fails the build if it ever exists |
| **Any write path over HTTP or MCP** | The API is `GET`-only, decided before routing (`router.rs:69–75`), and both surfaces open the database `PRAGMA query_only` (`lib.rs:261–265`). `init`, `index`, `coverage` and `trace import` are CLI-only because a read-only surface that could be tricked into a state change by a forged form submission is not read-only. There is no `POST` handler to harden |
| **A "re-index" button** | Same reason. A UI may **name** the command (`App.tsx:58–61`, `Coverage.tsx:151`) and must not run it |
| **`/api/source` over MCP** | Not exposed. See §6.7 — the omission is real; **UNVERIFIED —** I could not find a written decision recording it, so treat the reason as inferred |
| **Any network call from the product path** | Offline-first is non-negotiable. There is a test asserting the indexing path performs no network I/O, and no async runtime is in the tree |
| **Any external model** | There is no language model anywhere in Nerve's own path, which is what makes "repository text cannot alter Nerve's behaviour" a theorem rather than a policy |
| **A `confidence` number** | §7.4 |
| **Name-matching an unresolved site against a subject** | §7.2. The API deliberately gives a UI no data to do it with |
| **`TEST_OBSERVED_CALL` in a default relation set** | §6.16. Opt-in and labelled, or not offered |

### Not built yet — these are gaps, and naming them as gaps is the point

| capability | status |
|---|---|
| **An impact view** | `/api/impact` has shipped since Slice 7b and has no UI, no route and no TypeScript type. §6.11 is its specification |
| **A discoverable path entry point** | The path finder exists but is reachable only from an entity's graph tab. A path is a question about two things and can currently only be asked from inside one of them. §3 gap 2 |
| **Rendering `alternatives`** | The API sends it on every selector answer; four TypeScript mirrors omit the field. §7.7 |
| **An endpoints view** | `endpoint` entities and `SERVED_BY` shipped in Slice 10a. There is no view, and building one is a product decision. Note the "none vs no rule yet" ambiguity in §7.8 |
| **A trace view, a run picker, a test-to-symbol view** | `TEST_OBSERVED_CALL` shipped in Slice 11a. None of the three exists |
| **A partial-parse freshness comparison** | The row carries the hash it was extracted with; nothing compares it against disk. §6.9 |
| **A symbols figure on the Overview screen** | `symbols_total` reaches the rail (`App.tsx:144`) but `Overview.tsx:165` still shows only `entities`. Adding it is a product decision that was deliberately left alone |
| **Narrow-viewport QA at 380 px** | Carried over unverified from Slice 7a-ii. The responsive rules are present in the media-query block; no screenshot at 380 px exists |
| **Git history, cross-repository contracts, human-confirmed memory** | `docs/ROADMAP.md` rows 12b, 12c, 13, 14. `nerve history` and `nerve memory` do not exist, and the acceptance script records them as **unbuilt**, distinct from the two refusals above (`scripts/final_acceptance.sh:128–131`) |

---

## 10. Append point

<!--
  Rows 12b, 12c, 13 and 14 append below.

  Each new feature copies the §2 table verbatim and fills in every row. A state that cannot occur
  says `cannot occur — <reason>`; it never says `n/a`.

  A new slice also updates, in the same commit:
    · §3   the feature matrix — one row, marked honestly for all four surfaces
    · §4   any new closed vocabulary, with the interface gloss table `ui_vocabulary.rs` will require
    · §7   if it introduces a new absence-versus-zero distinction, a new refusal, or a new bound
             whose disclosure is load-bearing
    · §8   every new cap, with its constant's `file:line` and the field that reports it back
    · §9   if it closes a listed gap, or opens a new one
-->

### Reserved — Slice 12b · Git history ingestion

Schema v6, commit + change + availability tables, exact-content rename hypotheses,
`nerve history sync` / `log` / `file`. Plan accepted: `docs/plans/slice-12b-historical-model.md`.
Expect at least one new absence-versus-zero distinction — the plan already records that
**historical impact is refused outright, in the manner of `nerve affected`**
(`docs/ROADMAP.md:289`), and a refusal belongs in §9 rather than in a "coming soon" list.

**In progress as this document was written.** `crates/nerve-core/src/vocab.rs` is being appended to
with **six** new closed vocabularies, every one of which a UI will have to render and gloss, and
every one of which is an epistemic qualifier rather than a label:

| vocabulary | members | why a UI cannot ignore it |
|---|--:|---|
| `ChangeKind` | 4 | `added` `modified` `deleted` `mode_changed`. **There is deliberately no `renamed` member** — Git records no rename, a rename is *detected*, and a `renamed` change kind would state as fact the one thing about history that is a guess |
| `ParentCompleteness` | 5 | whether the parent side of a diff was fully available |
| `ChangesEnumerated` | 4 | whether a commit's change list is complete |
| `WalkTermination` | 5 | why a history walk stopped — the `truncated`-versus-exhausted distinction of §7.5, in a new place |
| `RenameEvidence` | 1 | `exact_content` only. A rename is a **hypothesis with evidence**, not a fact |
| `RenameAmbiguity` | 4 | how a rename hypothesis fails to be decidable |

When 12b lands, each needs a row in §4, a gloss table `ui_vocabulary.rs` will check, and — for
`ParentCompleteness`, `ChangesEnumerated` and `WalkTermination` — an entry in §7.1, because all
three are "the answer may be incomplete and here is why" values of exactly the kind this project
refuses to render as silence.

### Reserved — Slice 12c · Historical questions

First/last seen with boundary qualification, similarity rename hypotheses, change frequency,
labelled co-change, state-to-state diff, plus API + MCP + UI. "Boundary qualification" is an
absence-versus-zero problem by another name: *"first seen in the oldest commit Nerve has"* is not
*"first seen"*, and the two must not render the same way.

### Reserved — Slice 13 · Cross-repository contracts

### Reserved — Slice 14 · Human-confirmed memory

Note in advance: `HUMAN_CONFIRMED` already exists in `EvidenceSourceType` with a gloss
(`format.ts:61`), and `nerve memory` is currently recorded as **unbuilt** rather than refused.
