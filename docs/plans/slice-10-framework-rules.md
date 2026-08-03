# Slice 10 plan — Framework rules, split, and a defect measured before it was designed for

2026-08-03. Follows Slice 9b (`6e0412e`). Row 10 of `docs/ROADMAP.md`.

---

## 1. The defect, measured on the current release binary

Before designing anything, a framework fixture was indexed with the Slice 9b binary
(`api/routes.py` — FastAPI; `api/flaskapp.py` — Flask; `web/server.ts` — Express):

| question asked of Nerve today | answer | correct? |
|---|---|---|
| `nerve impact api/routes.py#read_user` — a live `GET /users/{user_id}` handler | *"Nothing in the index depends on this"* | **no** |
| `nerve impact api/routes.py#helper_never_called` — genuinely dead | *"Nothing in the index depends on this"* | yes |
| `nerve search "users"` — three routes contain `/users` | *"No matches for \"users\""* | **no** |
| `nerve impact web/server.ts#listUsers` | 1 dependant: **the module**, `REFERENCES`, line 9 | technically true, uninformative |

**A live HTTP endpoint and dead code produce a byte-identical answer.** That is the defect this
slice exists to fix, and it is the only claim in this plan that was measured rather than assumed.

Two further measurements shaped the design:

- **9a records a decorator's *name* but not its arguments.** `@app.get("/users/{user_id}")` is
  stored as `{"async":false,"decorators":["app.get"]}`. The path literal is gone. So the roadmap's
  note that *"9a's decorator metadata is the input"* is **half true**: the method is derivable from
  the dotted name, the address is not. A framework rule needs its own AST walk.
- **An inline callback has no entity to point at.** In `app.post("/users", function createUser…)`,
  `createUser` was never extracted as an entity; only the named, top-level `listUsers` was. So a
  route whose handler is written inline **cannot** be given a target, and must be counted as
  unsupported rather than silently dropped.

## 2. Scope: one framework family, three rules, two slices

§4.1 of the brief ranks candidate rules and says *"do not optimize for framework count"*. The
initial supported set is **HTTP route declaration** and nothing else:

| rule | language | form | why it ranks first |
|---|---|---|---|
| **FastAPI** | Python | `@app.get("/p")`, `@router.post("/p")` | decorator and handler are adjacent in one file; address is a string literal |
| **Flask** | Python | `@app.route("/p", methods=[…])`, `@bp.get("/p")` | same shape; `methods=` list is a literal in the common case |
| **Express** | TS/JS | `app.get("/p", handler)`, `router.post(…)` | registration is an ordinary call the reference extractor already resolves |

**Rejected for this slice, with reasons:**

- **Django URLconf** — `urlpatterns` is a list literal frequently built by loops and `include()`
  recursion across modules. Feasible, but it is a slice of its own, not an afterthought to three
  decorator rules.
- **NestJS** — requires class-decorator plus method-decorator composition *and* the DI graph to say
  anything useful. Two unsolved problems, not one.
- **Pytest fixtures** — resolution is by *parameter name* against `conftest.py` scope. That is how
  pytest itself resolves, so it is not coincidence; but it is still name matching across files, and
  it needs a conftest scope model. Deferred, not refused.
- **Generic event emitters / queues** — `emitter.on("x", h)` gives no way to know *which* emitter
  without cross-module value tracking. Would be `AST_HEURISTIC` at best.
- **React components/hooks** — explicitly outside backend value.

**The row is split**, for the reason Slice 8b was split: 10a changes two closed vocabularies *and*
the persisted schema, and writing a second language's rule against a moving vocabulary is what that
split existed to prevent.

- **10a — the endpoint model + Python.** `EntityKind::Endpoint`, `Relation::ServedBy`, schema v5,
  `py-framework` with the FastAPI and Flask rules, per-rule fixtures and measurement, query
  exposure, UI mirror compatibility fix.
- **10b — `ts-js-framework`.** The Express rule, consuming already-resolved references; its own
  fixture and its own separately reported precision table; CLI/API/MCP contract tests for the new
  kind across all three surfaces.

## 3. The model

### 3.1 Why a new entity kind is unavoidable

`assertion` is `(source_entity_id, relation, target_entity_id)` with **both endpoints `NOT NULL`
and foreign-keyed to `entity`**. There is no property-assertion form. Anything that is to be
*evidence* — carrying an `EvidenceSourceType`, a `Directness`, an extractor id and version, a
repository state, and an `assertion_state` derivation — must be a relation between two entities.
Entity `meta` (where 9a's decorators live) carries none of that and is not evidence.

So an endpoint must be an entity.

### 3.2 `EntityKind::Endpoint`, not `EntityKind::Route`

> **Endpoint** — a declared entry point at which something outside the indexed code can cause a
> symbol to run.

`ALL` goes 12 → 13 (**appended**, never inserted — `entity.kind` is `TEXT` so nothing on disk
encodes a position, but `apps/nerve-web/src/api/types.ts` mirrors the array *in order*).

- `is_symbol()` → **false.** An endpoint is not code. This keeps `symbols_total` (Slice 7a-iii)
  unchanged, and the existing test that pins *every* kind individually will force the new one to be
  classified rather than defaulted.
- `path_role()` → **`None`.** An endpoint is not addressed by a repository path. The Slice 8b-i
  exhaustiveness test will fail until it is stated.

Named `Endpoint` rather than `Route` because the *known* extension path — CLI commands
(Click/Typer), queue consumers, scheduled tasks — is the same concept with a different address
form, and three vocabulary members for one concept is the drift that Slices 5d-iii and 7a-iii were
corrective slices for. The discriminator lives in `meta.endpoint_kind`, from a closed list in
`nerve-core`, and **Slice 10 emits exactly one value: `http_route`.** A general name is not a
licence to emit general things.

- `name` — the framework-canonical address, e.g. `GET /users/{user_id}`. FTS5 tokenises this, so
  `nerve search "users"` starts matching, which is the second measured defect closed for free.
- `scope_path` — the declaring module's path, so an endpoint is contained by the file that declares
  it.
- `meta` — `{endpoint_kind, framework, method, path, rule_id}`.

### 3.3 `Relation::ServedBy`, and its direction

`ALL` goes 10 → 11, appended.

> `Endpoint SERVED_BY Function` — the source declares that the target implements it.

**Direction is forced by `nerve impact`.** Impact is a *reverse* closure: `impact X` finds `A` where
`A rel X`. To make a handler stop looking like dead code, the endpoint must be the **source**.
Semantically this is also the right way round: change the handler and the endpoint's behaviour
changes, so the endpoint depends on the handler.

**Voice.** Every existing relation is active (`CONTAINS`, `COVERS`, `SUPERSEDES`). `SERVED_BY` is
passive, and that is a deliberate deviation. The active alternatives all over-claim: `DISPATCHES_TO`
and `INVOKES` assert a runtime invocation from a static registration — precisely what §4.4 forbids
— and `ROUTES_TO` is HTTP-only, which contradicts the general kind. Clarity and honesty beat
grammatical consistency; the deviation is recorded here rather than hidden.

**`SERVED_BY` must never be relabelled as a call.** The same invariant ADR-0005 states for `COVERS`.
A registration proves a table entry, not an execution.

### 3.4 It goes in `impact::DEFAULT_RELATIONS`

`DEFAULT_RELATIONS` is currently the four Slice 2a relations, with `CONTAINS`/`DEFINES`/`IMPORTS`/
`COVERS` excluded and a reason recorded for each. `SERVED_BY` is **included**, and the anticipated
objection is the `COVERS` one — *"would put a `CoverageRun`, which is neither a symbol nor code, in
a list of things that depend on your function."* An `Endpoint` is also neither a symbol nor code.
The distinction is real:

- A `CoverageRun` is an artifact **about** the code — a report of a past execution. Changing a
  symbol does not break a report; it makes it *stale*, which is a freshness matter, and that is
  exactly what the exclusion note says.
- An `Endpoint` is a declaration **in** the code. It exists because a decorator in a source file
  declares it, it is withdrawn when that file changes, and changing its handler changes what the
  endpoint does. It genuinely depends on the symbol.

Excluding it would reproduce the measured defect exactly, which is the whole reason for the slice.

### 3.5 Evidence typing

§4.4 asks for `FRAMEWORK_DIRECT` / `FRAMEWORK_RESOLVED` / `FRAMEWORK_HEURISTIC` *"or the existing
equivalent evidence dimensions"*. Nerve already has them, and adding a parallel axis would be the
duplication this project keeps having to correct. The existing pair carries it:

| | `EvidenceSourceType` | `Directness` | when |
|---|---|---|---|
| decorator on the function it decorates | `FRAMEWORK_RULE` | `DIRECT` | FastAPI/Flask: the decorated symbol is syntactically adjacent |
| handler resolved through a reference | `FRAMEWORK_RULE` | `RESOLVED` | Express: the identifier argument was resolved by `ts-js-reference` |
| — | `FRAMEWORK_RULE` | `INFERRED` | **not emitted in Slice 10.** Reserved for a rule that concludes rather than reads |

`EvidenceSourceType::FrameworkRule` has existed since Slice 1, declared and never emitted. Slice 10
is the first emitter. No vocabulary member is added on this axis and no `source_type_mask` ordinal
moves.

### 3.6 What a route registration does **not** prove

Recorded in the module documentation and in the CLI/API wording, per §4.4. A registration does not
prove that the route is reachable in production, that middleware permits access, that dynamic
configuration has not replaced it, that a decorator-generated wrapper preserves the handler's
identity, or that two matching path strings denote one deployed endpoint. Nerve reports **that the
source declares this endpoint and names this symbol**, and nothing further.

## 4. Schema v5 — the cache slot, and the bug it prevents

`module_facts` has exactly two version columns, `structural_version` and `reference_version`, reused
positionally per language family (docs write the doc version twice; Python writes
`py-structural`/`py-reference`; TS/JS writes `ts-js-structural`/`ts-js-reference`). **There is no
slot for a third extractor.** This is the precise location of the Slice 9b upgrade bug, where two
extractors happened to share a version string and an existing index hit the cache forever.

**Schema v5: `ALTER TABLE module_facts ADD COLUMN framework_version TEXT NOT NULL DEFAULT ''`.**

The default is load-bearing. Every row written before Slice 10 gets `''`, which equals no released
version, so **every Python and TS/JS file in an existing index misses the cache and is
re-extracted** — which is the required behaviour. Markdown expects `''` (there is no framework
extractor for prose), so document rows still hit and do not churn.

Rejected alternatives: folding the version into `reference_version` as a compound string (drifts,
unparseable); bumping `reference_version` whenever a framework rule changes (couples two
independent extractors, and re-extracts references to publish a route change). Also considered and
rejected as premature: replacing both columns with one canonical `extractor_id=version;…` string.
It is more future-proof, but Slices 11–14 are run-level, repository-level, and non-extraction
work — the demand it anticipates does not exist. Smallest correct change.

**Acceptance criterion, not a nicety:** a test that writes a Slice-9b-era `module_facts` row (v4
column set, no framework version), runs the migration, re-indexes, and asserts the file was
re-extracted and framework observations exist. Slice 9b proved this class of defect is invisible to
every test that builds a fresh index — and all 1058 of them do.

## 5. Rules, precisely

### 5.1 Recognising the application object

A decorator `@X.get(...)` or a call `X.get(...)` is a route rule **only if `X` is bound, at module
scope in the same file, to a call of a known framework constructor**:

| framework | constructors |
|---|---|
| FastAPI | `FastAPI(...)`, `APIRouter(...)` |
| Flask | `Flask(...)`, `Blueprint(...)` |
| Express (10b) | `express()`, `express.Router()`, `Router()` |

and the constructor name is itself bound by an import from the framework's own package. Both hops
are ordinary binding lookups that 9b's `pybind` and Slice 2a's binding machinery already perform;
neither is name matching.

**Same-module binding is required.** `from .main import app` followed by `@app.get(...)` in another
file is **not** supported in Slice 10. This is a stated lower bound in the manner of 9a's `sys.path`
detection, it is given its own fixture, and its occurrences are **counted by form** so the limit is
measured rather than invisible. It is not a silent miss.

If `X` cannot be traced to a framework constructor, **nothing is emitted and nothing is counted as
a missed route** — Nerve does not know that `@foo.get("/x")` is a route at all, and a tally implying
otherwise would itself be a false claim.

### 5.2 Address

- **Method** — from the decorator/call name for `get|post|put|patch|delete|head|options|trace`;
  from Flask's `methods=[...]` list literal for `route`; Flask's documented default is `GET` when
  `methods` is absent, and that default is *the framework's*, not Nerve's guess.
- **Path** — the first positional argument, **only when it is a plain string literal**. Concatenation,
  f-strings, names, and computed expressions are refused.
- **No prefix composition.** `APIRouter(prefix="/v1")` and `app.include_router(...)` mean the
  deployed address is not what the decorator says. Slice 10 records the **declared** path and states
  that it is the declared path. Composing prefixes across `include_router`/`register_blueprint`
  requires cross-module value tracking, and inventing a composed address would produce a confident
  wrong URL — the worst possible failure for this feature.

### 5.3 The unsupported and ambiguous tally

Following Slice 9b's gate 7, which made the precision denominator auditable. Every form is counted,
the counts are asserted by the fixture gate, and a silently growing set fails the build:

| form | meaning |
|---|---|
| `handler-not-a-symbol` | inline function expression / lambda — no entity to target |
| `app-not-local` | application object bound in another module |
| `path-not-literal` | computed path expression |
| `methods-not-literal` | Flask `methods=` is not a list of string literals |
| `decorator-form` | decorator is not a dotted name (already counted by 9a) |
| `duplicate-address` | the same method+path declared twice — **ambiguous**, both edges kept, flagged |

### 5.4 Measurement

Per §4.5, **each rule is measured separately and never combined into one score**, and never merged
with the TS/JS or Python reference tables. Ground truth is written **before** the resolver, as in
5d-ii, 9a and 9b. Fixture categories: positive, negative (framework-shaped code that is not a
route), ambiguous, unsupported, malformed, aliasing (`from fastapi import FastAPI as F`), and
wrapper (a decorator stacked above/below the route decorator).

**Gate: false positives = 0.** False negatives are declared and pinned as must-stay-absent, so an FN
cannot be "fixed" by guessing without deliberately promoting it — the mechanism 9b introduced.

## 6. Security

`py-framework` and `ts-js-framework` read syntax trees. They import no Python module, execute no
decorator, evaluate no expression, load no framework, run no package script, spawn no process and
open no socket. **T1 is attacked directly**, as in 9a: a hostile fixture whose decorator *factory*,
module top level, and `setup.py` all attempt to write marker files and spawn processes. Zero
markers, and a non-zero entity count so the check is not vacuous.

Route path strings are repository-derived text. They become an entity `name`, which reaches MCP —
so the T7 property test must see the new kind inside `repository_content` and nowhere else. A route
path is a natural prompt-injection carrier (`@app.get("/ignore-previous-instructions")`) and gets a
hostile fixture in 10b's MCP contract tests.

## 7. Non-goals

Runtime reachability. Middleware. Deployment topology. OpenAPI generation. Prefix composition.
Django, NestJS, pytest fixtures, queues, events, DI. Any change to `apps/nerve-web` beyond the
vocabulary mirror and gloss entries that the existing `ui_vocabulary` test *requires* in order to
compile — a minimal compatibility correction under the frontend freeze, precedent 5d-iii.

## 8. Acceptance criteria (10a)

1. `EntityKind::Endpoint` and `Relation::ServedBy` appended; every exhaustiveness test — `ALL`,
   `is_symbol`, `path_role`, UI mirror — states the new members rather than defaulting them.
2. Schema v5 applies to a v4 database; **an existing index re-extracts and gains framework
   observations**, proven by a test that constructs the old row shape.
3. `py-framework 1.0.0` emits `Endpoint SERVED_BY Function|Method` with `FRAMEWORK_RULE`/`DIRECT`.
4. Zero `ts-js-*` and zero `py-structural`/`py-reference` observations carry `FRAMEWORK_RULE`.
5. FastAPI and Flask measured **separately**; FP = 0; FNs declared and pinned absent.
6. Every unsupported/ambiguous form counted, and the tally asserted.
7. Static indexing executes nothing: T1 attacked with a hostile framework fixture, non-vacuously.
8. `nerve impact` on a route handler reports the endpoint; on a dead function it still reports
   nothing. Both asserted.
9. `nerve search` finds an endpoint by a fragment of its path.
10. Incremental and full indexing agree on framework observations; editing a decorator withdraws and
    re-emits correctly; deleting the file withdraws the endpoint.
11. No new dependency. `Cargo.lock` stays at 101.
12. Full gate: fmt, clippy `-D warnings`, `cargo test --workspace --no-fail-fast`, release build.
