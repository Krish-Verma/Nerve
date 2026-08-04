# Nerve

A local, offline code evidence graph.

Nerve indexes a repository into a graph where **every claim carries inspectable evidence**:
what was observed, how it was obtained, which extractor produced it, where in the source it
was seen, and which repository state it was seen in. There is no scalar `confidence` field,
because a number like `0.94` on an individual relationship is not falsifiable.

Slice 1 supports TypeScript and JavaScript.

## Offline-first

The core product works with **no** cloud account, API key, external model, telemetry,
analytics, source upload, or network connection.

- No network calls on **any** path — indexing, ingestion or query. `nerve serve` binds
  `127.0.0.1` only, validates `Host` and `Origin`, and requires a session token.
- No telemetry, no analytics — absent from the codebase, not opt-out.
- No repository code is executed during indexing. Nerve parses bytes; it never runs them, and
  it never invokes `git` or package scripts as subprocesses. It does not run your tests either:
  `nerve trace import` reads an artifact your tracer produced.
- No source text is copied into the database. Only source ranges and content hashes are
  stored, so the index leaks far less than the code it describes. Entity *names* are stored, and
  so a heading or an identifier does reach output — which is why repository-derived text is
  confined to a labelled region in the MCP response and escaped in the UI.

A test (`crates/nerve-cli/tests/no_network.rs`) fails the build if any networking crate
becomes reachable in the dependency tree.

## Build

Rust 1.97.1 (pinned in `rust-toolchain.toml`). SQLite is compiled from source and statically
linked, so there is nothing to install.

```bash
cargo build --release
```

The binary is `target/release/nerve`.

## Run

```bash
# build the index
nerve init [path]            # create .nerve/{config.toml,nerve.db,cache/,logs/}
nerve index [path]           # parse and persist the evidence graph

# ingest evidence another tool produced — Nerve runs nothing itself
nerve coverage <report>       # one LCOV report
nerve trace import <artifact> # one nerve-trace/v1 test-call-trace artifact

# ask
nerve search <query>         # FTS5 over entity names and scope paths
nerve why <from> [<to>]      # the evidence behind a relationship
nerve path <from> <to>       # how two entities are connected
nerve impact <symbol>        # what depends on this, and what the answer cannot see
nerve gaps                   # symbols no test is known to touch

# judge
nerve status [path]          # counts, freshness, schema version
nerve check                  # is this index trustworthy right now — for CI
nerve doctor                 # diagnose the install, the database, the configuration

# serve
nerve serve                  # loopback, read-only HTTP API + the explorer UI
nerve mcp                    # Model Context Protocol over stdin/stdout, for an agent
```

Global flags: `--json`, `--quiet`, `--no-color`.

Two commands are **deliberately absent**, and their absence is a decision rather than a gap.
`nerve affected` ("which tests would my change affect?") is refused because LCOV is an aggregate
report with no per-test attribution, so the only way to ship it would be to assert that every test
covers every covered symbol — see `docs/decisions/ADR-0008-coverage-evidence.md` §A.2.
`nerve trace-tests` is refused because running your suite would need an exception to
`crates/nerve-cli/tests/no_subprocess.rs`; you run the tracer, Nerve reads the artifact.

Exit codes: `0` success · `2` no or unhealthy index · `3` partial index (some files were
skipped) · `4` index sound but stale — `nerve check` only · `10` usage error · `70` internal
error.

Every command accepts `--json`, which prints a single JSON object with stable keys.

```bash
nerve init  fixtures/ts-basic
nerve index fixtures/ts-basic
nerve status --path fixtures/ts-basic --json
nerve search area --path fixtures/ts-basic --kind method
```

## What Nerve extracts

**Languages:** TypeScript and JavaScript (`.ts`, `.tsx`, `.js`, `.jsx`, `.mjs`, `.cjs`), Python,
and Markdown including ADRs. **Nerve does not index Rust** — its own source is not a subject for a
symbol query.

**Entities:** `Repository`, `Directory`, `File`, `Module`, `Function`, `Method`, `Class`,
`Interface`, `Document`, `Section`, `CoverageRun`, `Endpoint`, `Unresolved`.

**Relations:** `CONTAINS`, `DEFINES`, `IMPORTS`, `EXPORTS`, `CALLS`, `REFERENCES`, `EXTENDS`,
`IMPLEMENTS`, `SUPERSEDES`, `COVERS`, `SERVED_BY`, `TEST_OBSERVED_CALL`.

**Evidence source types**, which are never presented as equally certain: `AST_DIRECT`,
`AST_RESOLVED`, `AST_HEURISTIC`, `TYPE_RESOLVED`, `FRAMEWORK_RULE`, `TEST_COVERAGE`,
`TEST_CALL_TRACE`, `RUNTIME_CALL_TRACE`, `DOCUMENT_STATED`, `HUMAN_CONFIRMED`, `LLM_DERIVED`,
`FILESYSTEM_OBSERVED`.

**Import resolution** covers relative specifiers, resolved against files that are actually indexed.
Bare package specifiers, unresolvable relatives and everything else become `Unresolved` entities
with real `IMPORTS` assertions, so they are countable and queryable rather than silently dropped.

**What it declines to know is on the record.** 38.1% of call sites on the resolution corpus are
honestly `Unresolved`, because any method call on a typed receiver is unresolvable without type
inference. Python's `self.method()` is unresolved on purpose: knowing `self` is an `Engine` does not
say what `self.start()` calls, since a subclass instance is what is usually passed. Coverage is
`COVERS` from a **coverage run**, never `TEST_COVERS_SYMBOL` from a test, because LCOV carries no
per-test attribution. Precision is measured per language and per extractor and never summed into
one number — and it is measured **on fixtures**; recall on real repositories is not yet measured.

## Where the index lives

`.nerve/` inside the repository, git-ignored by default (`.nerve/.gitignore` contains `*`).
`nerve.db` is created mode `0600` on Unix.

Excluded from indexing by default: `.gitignore`, `.ignore` and `.nerveignore` rules;
`node_modules/`, `.git/` and `.nerve/`; and a built-in secret deny-list (`.env`, `.env.*`,
`*.pem`, `*.key`, `*.p12`, `*.pfx`, `*.keystore`, `*.jks`, `id_rsa*`, `id_ed25519*`,
`.npmrc`, `.netrc`, `.pgpass`, `credentials`, `secrets.*`) applied **before** any file is
read. The deny-list is extensible via `.nerve/config.toml`.

## Layout

| Crate | Responsibility |
|---|---|
| `nerve-core` | Identity, vocabularies, errors, canonical graph dump |
| `nerve-store` | SQLite schema, migrations, all SQL, FTS5, `rebuild_assertion_state`, selector resolution, graph traversal, evidence assembly |
| `nerve-index` | Discovery, ignore rules, tree-sitter parsing, extraction, pipeline, coverage and trace ingestion, Git object reading |
| `nerve-server` | Loopback HTTP API, token/`Host`/`Origin` validation, the embedded explorer UI, and MCP over stdio |
| `nerve-cli` | The `nerve` binary — argument parsing, rendering, exit codes |

`apps/nerve-web/` is the React explorer, compiled into the binary with `include_bytes!`, so
`nerve serve` needs no Node, no build step and no network. Its runtime dependencies are `react` and
`react-dom` only, lint-enforced.

**Surfaces contain no business logic.** The CLI, the HTTP API and the MCP tools call the same
application functions, and a test in each surface crate greps its own source for SQL and traversal
to keep it that way.

Design decisions live in `docs/decisions/`. The load-bearing ones are ADR-0001 (SQLite),
ADR-0002 (identity), ADR-0003 (the evidence model), ADR-0006 (state-independent occurrences),
ADR-0007 (filesystem evidence) and ADR-0008 (coverage evidence, which reverses ADR-0005 on the
evidence). `docs/ROADMAP.md` is authoritative about what is built.

## Tests

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cargo test --workspace -- --ignored scale --nocapture   # scale/latency harness
```

The golden graph test compares the `fixtures/ts-basic` graph against
`fixtures/ts-basic/golden.json` byte for byte. Regenerate deliberately, and review the diff:

```bash
NERVE_UPDATE_GOLDEN=1 cargo test -p nerve-index golden
```

## Independence

Nerve is an independent, clean-room implementation. It does not depend on, embed, fork,
vendor, read the format of, or reproduce the schema or algorithms of any other code-knowledge
-graph product. Every third-party dependency is a foundational library and is recorded with
its license in `third_party/LICENSES.md`. See `docs/CLEANROOM.md`.

## License

Dual-licensed under MIT or Apache-2.0, at your option.
