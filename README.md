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

- No network calls during `init`, `index`, `status` or `search`.
- No telemetry, no analytics — absent from the codebase, not opt-out.
- No repository code is executed during indexing. Nerve parses bytes; it never runs them, and
  it never invokes `git` or package scripts as subprocesses.
- No source text is copied into the database. Only source ranges and content hashes are
  stored, so the index leaks far less than the code it describes.

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
nerve init [path]        # create .nerve/{config.toml,nerve.db,cache/,logs/}
nerve index [path]       # parse and persist the evidence graph
nerve status             # counts, freshness, schema version
nerve search <query>     # FTS5 symbol search
```

Global flags: `--json`, `--quiet`, `--no-color`.

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

## What Slice 1 extracts

**Entities:** `Repository`, `Directory`, `File`, `Module`, `Function`, `Method`, `Class`,
`Interface`, `Unresolved`.

**Relations:** `CONTAINS`, `DEFINES`, `IMPORTS`, `EXPORTS`. `CALLS`, `REFERENCES`, `EXTENDS`
and `IMPLEMENTS` exist in the vocabulary and are deliberately emitted by nothing until
Slice 2 — an empty edge is more honest than a guessed one.

**Import resolution** covers relative specifiers only (`./`, `../`), resolved against files
that are actually indexed. Bare package specifiers, unresolvable relatives, and everything
else become `Unresolved` entities with real `IMPORTS` assertions, so they are countable and
queryable rather than silently dropped.

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
| `nerve-store` | SQLite schema, migrations, all SQL, FTS5, `rebuild_assertion_state` |
| `nerve-index` | Discovery, ignore rules, tree-sitter parsing, extraction, pipeline |
| `nerve-cli` | The `nerve` binary |

Design decisions live in `docs/decisions/`. The load-bearing ones for this slice are
ADR-0001 (SQLite), ADR-0002 (identity) and ADR-0003 (the evidence model).

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
