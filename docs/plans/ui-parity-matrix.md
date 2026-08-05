# Reference UI — the feature parity matrix

**Status:** measured 2026-08-05, before any UI work
**Purpose:** the brief's §9.1 requires a complete feature matrix in which every accepted human-facing
capability is either **available in the UI** or **explicitly documented as intentionally CLI-only for a
security reason**. This is that matrix, and every "no" below was verified against the source rather
than assumed.

The user will redesign the frontend later. This document is about **function**, not visuals. The
interface freeze is lifted for function and kept for visuals (`docs/CONTINUATION.md`, 2026-08-03).

---

## 1. How each row was established

- **Route/panel presence:** `apps/nerve-web/src/routing.ts:24-29` declares the whole `Route` union —
  `overview · search · entity · unresolved · coverage` — and `App.tsx:183-193` maps it to components.
  There are **five** routes. `EntityTab` (`routing.ts:12-14`) adds four tabs inside one of them:
  `relations · evidence · graph · source`.
- **Backend surface:** `crates/nerve-server/src/router.rs:150-172` declares **11** routes, and
  `ROUTES` at `:198-210` is the same list as a constant.
- **Vocabulary mirroring:** `crates/nerve-server/tests/ui_vocabulary.rs` is the drift guard. It
  currently pins **seven** vocabularies: `EntityKind`, `Relation`, `EvidenceSourceType`, `Directness`,
  `AssertionStatus`, `UnresolvedCategory`, `AdrStatus`.
- **A grep over `apps/nerve-web/src/` is not sufficient on its own.** `82a6ff3` records that a literal
  NUL byte in `Graph.tsx` made `grep` treat it as binary and silently suppress every match, which
  produced a false "dead code" finding. Use `grep -a` or a byte-level read. The rows below were
  checked that way.

---

## 2. The matrix

| # | capability | backend | in UI? | evidence |
|---|---|---|---|---|
| 1 | Overview | `/api/overview` | ✅ | `views/Overview.tsx`, route `#/overview` |
| 2 | Index status | `/api/overview` | ✅ | same view |
| 3 | Search | `/api/search` | ✅ | `views/Search.tsx`, route `#/search` |
| 4 | Entity inspection | `/api/entity` | ✅ | `views/Entity.tsx`, route `#/entity/<id>/<tab>` |
| 5 | Source inspection | `/api/source` | ✅ | `views/Source.tsx`, entity tab |
| 6 | Neighbourhood graph | `/api/neighbourhood` | ✅ | `views/Graph.tsx`, entity tab |
| 7 | Why / evidence | `/api/why` | ✅ | `views/Evidence.tsx`, entity tab |
| 8 | Unresolved | `/api/unresolved` | ✅ | `views/Unresolved.tsx`, route `#/unresolved` |
| 9 | Coverage | `/api/gaps` | ✅ | `views/Coverage.tsx`, route `#/coverage` |
| 10 | Path between two entities | `/api/path` | ⚠️ **reachable but undiscoverable** | `PathFinder` is imported by `Graph.tsx:31` and reachable at `#/entity/<id>/graph?to=<selector>`. A path is a question about *two* entities and can only be asked from inside one of them, so it has **no top-level route**. Not dead code — the earlier "dead code" finding was the NUL-byte artefact. |
| 11 | **Impact** | `/api/impact` | ❌ **none** | The string `impact` appears nowhere under `apps/nerve-web/src/`, not even as a type. Slice 7b's whole surface. |
| 12 | **Selectors / alternatives** | every selector answer | ❌ **none** | The API sends it on every selector answer and four TypeScript mirrors omit the field, so *"resolved by this rule, and here is what it passed over"* never reaches a human. Slice 8b-i's whole point. |
| 13 | **Partial parses** | `/api/partial-parses` | ❓ **unverified** | Route exists; UI surfacing never confirmed. |
| 14 | Documents / ADRs | via `/api/entity` | ⚠️ partial | Document and Section entities are reachable as entities; there is no document-specific view. `AdrStatus` **is** glossed and drift-guarded. |
| 15 | Framework endpoints | via entity + `SERVED_BY` | ⚠️ partial | `endpoint` is glossed (`vocab.ts:106`) and `SERVED_BY` is in the relation table, so an endpoint renders as an entity — but there is no endpoint list or route view. |
| 16 | Coverage runs | via entity | ⚠️ partial | `coverage_run` is glossed (`vocab.ts:104`). |
| 17 | Test-observed calls | relation vocabulary | ⚠️ partial | Renders as a relation; no trace-specific surface. |
| 18 | Trace runs | provenance only | ⚠️ by design | A trace run is provenance, not an entity (Slice 11a). It appears inside evidence, which is correct. |
| 19 | **History — availability / shallow state** | *(12c-iii)* | ❌ **none** | **Zero history UI exists.** Verified by byte-level grep: the only matches for `history` under `apps/nerve-web/src/` are `window.history.replaceState` (`routing.ts:105`), a comment about shell history (`client.ts:8`), and a comment saying the server has no history (`routing.ts:4`). The one `shallow` match (`format.ts:221`) is *"flatten a shallow object"*. |
| 20 | **History — changes in a commit** | *(12c-iii)* | ❌ none | as above |
| 21 | **History — path history** | *(12c-iii)* | ❌ none | as above |
| 22 | **History — rename hypotheses + evidence** | *(12c-iii)* | ❌ none | as above |
| 23 | **History — change frequency** | *(12c-iii)* | ❌ none | as above |
| 24 | **History — co-change** | *(12c-iii)* | ❌ none | as above, **and** it must carry the store's disclaimer verbatim when it lands |
| 25 | **Cross-repository registry** | *(row 13)* | ❌ not built | backend does not exist yet |
| 26 | **Contract links** | *(row 13)* | ❌ not built | backend does not exist yet |
| 27 | **Memory** | *(row 14)* | ❌ not built | backend does not exist yet. Per the row-14 plan §5 the **API stays read-only**, so the UI displays memory and prints the exact command for confirmation |

### 2.1 Intentionally CLI-only, with the security reason

Per the brief's §9.1, a capability may be absent from the UI **only** with a documented security reason.
Exactly one class qualifies, and it is not a UI decision — it follows from Slice 4a:

| operation | why CLI-only |
|---|---|
| `nerve index`, `nerve coverage`, `nerve trace import`, `nerve history sync`, and (row 14) `nerve memory` confirmation | **`nerve serve` is read-only and proven so on the bytes** — one `PRAGMA query_only` connection per worker, `POST` → 405, sha256 identical before and after (`ROADMAP.md:218-223`). Every one of these mutates the index. Making the API writable would relax the single control that makes read-only *provable*, for operations whose value is that they are deliberate. |

**The UI's obligation for these is not "hide the button".** `CONTINUATION.md:313-316` sets the bar: show
the imported results, explain the boundary, and **print the exact command** — never a disabled button
implying implementation is pending.

---

## 3. The vocabulary gap, measured

`crates/nerve-server/tests/ui_vocabulary.rs` pins seven vocabularies. **Eight are unmirrored**, and the
guard cannot fail for them because it does not know they exist:

| vocabulary | added by | glossed? |
|---|---|---|
| `ChangeKind` | 12b | ❌ |
| `ParentCompleteness` | 12b | ❌ |
| `ChangesEnumerated` | 12b | ❌ |
| `WalkTermination` | 12b | ❌ |
| `RenameEvidence` | 12b | ❌ |
| `RenameAmbiguity` | 12b | ❌ |
| `FirstObservedKind` | 12c-i-a | ❌ |
| `HistoryFreshness` | 12c-i-a | ❌ |

**This is why 12c-iv is a real sub-slice and not a formality.** 5d-iii was an entire corrective slice
for exactly this drift and it found **120 real sites rendering fallback text**. The guard must be
extended *with* the glosses, in the same commit — otherwise the next vocabulary is unguarded again.

Note that the four `note()` methods hoisted into `nerve-core` in 12c-i-a are **prose**, not glosses. The
UI must render the note the backend sends rather than growing a ninth copy;
`crates/nerve-cli/tests/history_wording.rs` already scans `nerve-server/src` for exactly that and will
need `apps/nerve-web/src` added when the UI lands.

---

## 4. Required states (brief §9.2)

The canonical backend states, and whether the UI has a rendering path today:

`loading` ✅ · `empty` ✅ · `error` ✅ · `refused` ⚠️ · `ambiguous` ⚠️ · `unresolved` ✅ ·
`unsupported` ⚠️ · `partial` ⚠️ · `stale` ✅ (freshness is shown as arithmetic — recorded hash beside
on-disk hash) · `shallow` ❌ · `unavailable` ❌ · `truncated` ⚠️ (the graph does it: *"145 MORE NOT
DRAWN"*) · `conflicted` ❌ (row 14) · `superseded` ⚠️ (document supersession exists) · `invalidated` ❌
(row 14)

`shallow` and `unavailable` are the two the history work introduces, and they are the two whose wording
is load-bearing: **"earliest visible" must never render as "first ever"**, which is the invariant 12b
exists to protect and `scripts/final_acceptance.sh` already greps for on the CLI.

---

## 5. Order of work

1. **12c-iv** — history views, eight glosses, guard extended, `history_wording.rs` scan widened to
   `apps/nerve-web/src`.
2. **Impact view** (row 11 above) — a backend surface with no UI at all, and the cheapest real gap to
   close.
3. **Selectors / alternatives** (row 12) — four TypeScript mirrors gain the field.
4. **Partial parses** (row 13) — verify, then surface or record why not.
5. Row 13 and row 14 views, when those backends exist.

Rows 25–27 cannot be started before their backends. Rows 11, 12 and 13 can be done at any time and do
not depend on the history work.
