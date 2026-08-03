# Slice 10a — Framework rules, and a route handler that looked exactly like dead code

2026-08-03. Plan: `docs/plans/slice-10-framework-rules.md`. Follows Slice 9b (`6e0412e`).
**Row 10 is split; 10a is complete, 10b is not started.**

---

## Objective

Make a declared HTTP route a first-class piece of evidence: `Endpoint SERVED_BY Function|Method`,
from FastAPI and Flask decorators, with FastAPI and Flask measured separately.

## The defect, measured before anything was designed

Indexed a FastAPI + Flask + Express fixture with the **9b release binary**:

| question | 9b answer | correct? |
|---|---|---|
| `impact fastapi_app.py#read_user` — a live `GET /users/{user_id}` handler | *"Nothing in the index depends on this"* | **no** |
| `impact fastapi_app.py#not_a_route` — genuinely dead | *"Nothing in the index depends on this"* | yes |
| `search users` — three `/users` routes exist | *"No matches for \"users\""* | **no** |

**A live HTTP endpoint and dead code produced a byte-identical answer.** Both are now closed, on the
release binary:

```
subject  function   read_user                          fastapi_app.py:18
  entities       1 depend on this, transitively
  by relation    SERVED_BY 1
  by kind        endpoint 1
1     SERVED_BY   endpoint   GET /users/{user_id}   fastapi_app.py:18   FRAMEWORK_RULE  fresh
```

and the dead function still reports **0**. Both halves are asserted, because a change that made
everything look reachable would satisfy the first assertion alone.

Two further measurements corrected the plan's premises:

- **9a records a decorator's name but not its arguments** — `{"decorators":["app.get"]}`. The
  roadmap's note that 9a's metadata is this slice's input was **half true**: the method is derivable
  from the dotted name, the address is not. A framework rule needs its own AST walk.
- **An inline callback is never extracted as an entity**, so a route written that way has no target
  and must be counted rather than dropped.

## Architecture decisions

### `EntityKind::Endpoint`, not `Route` — and the source of the edge

`ALL` 12 → 13, appended. `is_symbol()` false, `path_role()` `None`. Named generally because CLI
commands, queue consumers and scheduled tasks are the same concept with a different address form,
and the discriminator is a **closed `EndpointKind` in `nerve-core`** with exactly one member
(`http_route`). A general name with an open discriminator is a free-text tag wearing a vocabulary's
clothes; a test asserts `cli_command` does **not** parse until a rule produces it.

`Relation::ServedBy`, `ALL` 10 → 11. **The endpoint is the source, and that is forced rather than
chosen**: `nerve impact X` is a reverse closure, so a handler stops looking like dead code only if
the endpoint asserts the edge.

**Passive voice, deliberately** — a deviation from every other relation, recorded rather than hidden.
`DISPATCHES_TO` and `INVOKES` assert a runtime invocation from a static registration; `ROUTES_TO` is
HTTP-only and contradicts the general kind.

### Why it is in `DEFAULT_RELATIONS`, when `COVERS` is not

The objection answered rather than dodged: an `Endpoint` is not a symbol and not code, exactly like a
`CoverageRun`. The distinction is real — **a coverage run is a report *about* the code, and changing a
symbol makes it *stale*, which is a freshness matter; an endpoint is a declaration *in* the code,
withdrawn when the file changes.** Excluding it reproduces the measured defect exactly. A test
asserts both membership decisions together, because they are the same argument in opposite
directions, plus `DEFAULT_RELATIONS.len() + 6 == Relation::ALL.len()` so a new relation cannot be
absent from both lists.

### Evidence typing reuses the existing axes

The brief offered `FRAMEWORK_DIRECT` / `FRAMEWORK_RESOLVED` / `FRAMEWORK_HEURISTIC`. Nerve already
has both axes, and a parallel one would be the duplication this project keeps correcting.
`EvidenceSourceType::FrameworkRule` + `Directness::Direct`. **This is the first emitter of
`FRAMEWORK_RULE`, declared since Slice 1 and never used.** `Direct` rather than `Inferred` because
the decorator and its function are adjacent in one tree: the rule reads a declaration, it does not
conclude one. No vocabulary member added on that axis, no `source_type_mask` ordinal moved.

Every observation carries a `proves` field stating what the registration does **not** establish, and
`nerve why` prints it — the claim a reader is most likely to over-read, stated on the evidence rather
than in documentation nobody opens.

## Schema v5, and the defect class 9b shipped

`module_facts` had exactly two version columns reused positionally per language family. **There was
no slot for a third extractor** — the precise location of the 9b upgrade bug.

```sql
ALTER TABLE module_facts ADD COLUMN framework_version TEXT NOT NULL DEFAULT '';
```

The default is the whole point: `''` equals no released version, so **every Python file in an
existing index misses and is re-extracted**, while a document expects `''` and keeps hitting, so
Markdown does not churn. Both directions are tested.

**This slice commits the regression test Slice 9b never got.** 9b shipped exactly this defect one
slot narrower, I caught it by hand last session by rewriting cache rows, and *no test was written* —
because every test in the suite builds a fresh index and a fresh index cannot observe an upgrade.
The simulation is now committed: write the row shape an older build wrote, run the current build,
assert re-extraction and that the new evidence exists.

## The rule, and why the decorator's name is not enough

`.get`, `.post` and `.route` are ordinary method names. The receiver is traced through **two ordinary
binding lookups**: the application object must be bound at module scope *in the same file* to a call
of a framework constructor, and that constructor must itself be bound by an import from the
framework's package. Aliases fall out for free.

**No prefix composition.** `APIRouter(prefix="/v1")` and `include_router` mean the deployed address
is not what the decorator says; composing it needs a fact from another file, and a confidently wrong
URL is the worst possible failure for this feature.

**The cross-module limit is stated and counted**, not silently missed: `from .main import app` then
`@app.get(...)` is a real route this rule declines, counted `app-not-local`. But where the receiver
cannot be traced at all, **nothing is emitted *and nothing is counted*** — Nerve does not know
`@cache.get("/x")` was meant to be a route, so a missed-route tally there would be a false claim in
the opposite direction from a false positive. `negative.py` contributes zero of both, asserted.

## Measured accuracy — two frameworks, two tables, never summed

`fixtures/py-framework`, 7 files, ground truth written **before** the resolver.

```
framework    TP   FP   FN
fastapi     12    0    0
flask        7    0    0

unsupported, repository-wide, not attributable to a framework: 7
  app-not-local          2
  handler-not-a-symbol   1
  methods-not-literal    1
  path-not-literal       3
```

Unsupported is deliberately **outside** the per-framework tables: an `app-not-local` construct has no
framework attached — tracing it to one is precisely the step the rule declined — so attributing it
per framework would invent that attribution.

Four false negatives are declared and **asserted absent**, so one cannot be "fixed" by a guess
without deliberately promoting it. Twelve `forbidden` addresses are checked **first**, because Slice
9a found a `forbidden` list sitting behind a set-equality assertion where it was unreachable and had
never fired.

TS/JS and Python reference tables re-run and unchanged.

## Three corrections the work forced

**1. A reviewing agent found my lambda test passed vacuously.** The walker visited only
`decorated_definition`, so `app.get("/lambda")(lambda: [])` was never read and
`handler-not-a-symbol` was never counted — where the fixture requires 1. I confirmed by probe
(`unsupported_by_form: {}`), implemented the applied-decorator form, and the test now asserts the
count and has a named-handler companion so a rule that refused *every* applied decorator could not
pass. **Third vacuity trap caught on this project.**

**2. A `decorator-form` tally member was removed.** Making it fire required a
`subscript_on_known_app` special case whose only purpose was feeding the counter, and it contradicted
the count-nothing rule. 9a already counts unsupported decorator shapes.

**3. `methods=METHODS` was written against a FastAPI object.** FastAPI has no `route` decorator, so
it is not a route at all and the extractor was right to decline. The case moved to Flask. **The
fixture lost this one and the implementation won** — the inverse of 9a, where the fixture won.

Also: my own anti-vacuity guard caught my own bad SQL. A Python symbol's `scope_path` is its
enclosing *lexical* scope, not its file, so my `negative.py` check found no symbols and correctly
refused to pass.

## Mutation probes — three, all mine

| probe | result |
|---|---|
| accept `@X.get(...)` without tracing `X` to a framework constructor | **3 targets fail**, and the `forbidden` list fires **first** with four named wrong answers: `GET /not-a-route`, `GET /also-not-a-route`, `GET /imported`, `POST /imported-router` |
| compose `APIRouter(prefix="/v1")` into the declared path | `FORBIDDEN GET /v1/items exists at fastapi_app.py:list_items` — named exactly, with the reason |
| ignore `framework_version` in the cache-hit check | the upgrade test fails: *"a pre-10a index hit the cache and skipped framework extraction — the Slice 9b defect, one slot wider. files_re_extracted was 0"* |

All reverted; both source files diffed **byte-identical**; gate re-run.

## Security

**T1 attacked directly and non-vacuously.** A hostile repository whose decorator **factory**, module
top level, `setup.py` and `conftest.py` all write markers and call `os.system` / `subprocess.run`:

```
markers created: 0
python entities indexed: 8
endpoints indexed: 2   →  GET /detonate, POST /items
```

`GET /detonate`'s route decorator sits directly above `@hostile_decorator("boom")`. **The route was
read and the factory was never called.**

`no_network.rs` and `no_subprocess.rs` pass **unmodified**. Zero `ts-js-*` observations in a Python
repository (the 5d-i invariant), verified on the real database: `fs-structural 7`, `py-framework 19`,
`py-reference 17`, `py-structural 57`.

**Privacy:** an endpoint's `name` is a route path — repository-derived text. It is subject to T7 like
any other; the MCP trust-envelope property test covers the new kind because it walks whole responses
rather than a field list.

## Files changed

**New:** `crates/nerve-index/src/pyframework.rs`, `tests/py_framework_precision.rs`,
`fixtures/py-framework/` (7 `.py` + `expected.json`), `docs/plans/slice-10-framework-rules.md`,
`docs/plans/slice-11-test-observed-calls.md`.

**Modified:** `nerve-core/{vocab,ids,lib}.rs`, `nerve-store/{schema,facts,impact}.rs`,
`nerve-index/{pipeline,facts,lib}.rs`, six test files, `fixtures/ts-basic/golden.json`
(**one line** — `schema_version: 4 → 5`, zero entity or assertion drift), `docs/ARCHITECTURE.md`.

**`apps/nerve-web/`: 10 added lines**, all vocabulary entries the 5d-iii drift test requires. No
styling, no layout, no views.

## Verification

```
cargo fmt --all -- --check                             → clean, exit 0
cargo clippy --workspace --all-targets -- -D warnings  → 0 warnings, exit 0
cargo test --workspace --no-fail-fast                  → 1094 passed, 0 failed, 2 ignored
cargo build --release                                  → Finished, exit 0
```

**1058 → 1094, +36.** `Cargo.lock` unchanged at **101** — no dependency added.

## Deviation from CLAUDE.md §4

Two implementation agents were lost to infrastructure limits — the first to a **weekly** limit (a new
kind; the third such kill on this project), the second stood itself down on detecting concurrent
writes and deleted its own three files rather than corrupt the tree. Its read-only review found the
vacuity defect above, which is the single most valuable thing any agent contributed to this slice.
**The orchestrator wrote the implementation directly.** The first agent's accepted foundation
(vocabulary, identity, schema, facts, impact) was reviewed file by file and kept.

## Known limitations

- **Cross-module application objects are not resolved** — counted `app-not-local`. The stated lower
  bound, analogous to 9a's `sys.path` refusal.
- **No prefix composition**, so a declared path is not a deployed URL. Stated on every observation.
- **Django, NestJS, pytest fixtures, queues, events, DI are not implemented.** Each rejected with a
  reason in the plan rather than deferred silently.
- **`add_url_rule` / `APIRouter.add_api_route` are not read** — imperative registration APIs, no
  fixture, no rule.
- **Express is Slice 10b**, so `framework_version` is `''` for TS/JS and no TS/JS file carries
  framework evidence yet.
- **7 fixture files.** These are fixture numbers and a regression gate, **not** real-world accuracy.

## UI backend handoff changes

`docs/UI-BACKEND-HANDOFF.md` gains an entry: one new `EntityKind` (`endpoint`), one new `Relation`
(`SERVED_BY`), and **one changed default** — `/api/impact` and `nerve impact` now report five
relations in `relations_effective` where they reported four.

## Commit

`4e4239a` — *feat: Slice 10a — a route handler stops looking exactly like dead code*.

## Next slice

**10b — `ts-js-framework`** (Express), with its own precision table, then **11**.
