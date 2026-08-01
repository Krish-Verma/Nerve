# Nerve — Security and Privacy

Binding from the first commit.

## Absolute rules

| Rule | Status |
|---|---|
| No telemetry, no analytics | Absent from the codebase — not opt-out |
| **No outbound network activity** | Enforced by test: no HTTP/RPC client, TLS, async runtime, telemetry, analytics, update-checker or crash-reporter crate is reachable from the workspace, including dev- and build-dependencies |
| No external LLM calls from product code | Absent |
| No repository code execution during indexing | We parse bytes; we never run them |
| **No subprocess spawned by product code** | Enforced by test (`no_subprocess.rs`): a source scan for process-creation APIs, a dependency scan for process-runner crates, and an end-to-end test that indexes a repository whose `package.json` scripts and module top-level would write a marker file if anything ever ran them |
| No package-script execution | Never invoked; covered by the end-to-end test above |
| No `eval` of source files | Never |
| Local server binds `127.0.0.1` only | Slice 4a — token-gated, origin/host-validated, read-only |

### Precisely what "offline-first" claims

Since Slice 4a, Nerve **does** contain a network stack: `tiny_http`, an **inbound** HTTP listener
bound to loopback. Claiming "no networking crates" would therefore be false. The accurate and
enforced claims are:

- Nerve runs a **local inbound** HTTP server, on `127.0.0.1` only, read-only, token-gated.
- Nerve has **no outbound network client**. It listens; it never dials out.
- No telemetry, no analytics, no update checks, no crash reporting.
- Source code is never uploaded anywhere.
- After dependencies are installed, no internet connection is required for any operation.
- Fetching crates at build time, and acquiring a validation corpus during development, are
  **development-time** activities and are not part of the shipped product's runtime behaviour.

## Repository content is untrusted input

Source files, file names, and (later) documents are attacker-controlled data. They are parsed,
never executed; escaped before rendering; and never interpreted as instructions.

## Path safety

- Every path is canonicalized and asserted to be inside the repository root before any read.
- Symlinks are not followed out of the repository root.
- Path traversal via crafted file names (`../`, absolute paths, NUL bytes) is rejected.
- Covered by `path-safety` tests from Slice 1.

### Query-time file reads (from Slice 2b)

`nerve why` computes freshness by re-hashing the file an observation points at. That path comes
out of the database, which is a file on disk and therefore **not a trusted channel**. Every
discovery-time rule is re-applied at query time through the same `canonical_child` choke point,
never a second implementation:

- the path must be relative, with only ordinary components (no `..`, no NUL, not absolute)
- it must not be a symlink — discovery never indexes one, so a path that is one now was swapped
  after indexing
- it must canonicalize to a location inside the repository root, which is what catches a
  symlinked *parent* directory
- it must not match the secret deny-list, and must not exceed the file-size ceiling

Anything refused is **reported as refused** rather than as missing, so the output never disguises
which check fired. Verified by constructing both escape vectors and confirming no content leaks.

All query commands are read-only; the database is byte-identical before and after.

## Exclusions

Respected by default:

- `.gitignore` (full semantics, including nested and negated patterns)
- `.nerveignore` (same syntax, Nerve-specific)
- Built-in secret deny-list, applied even if the file is not git-ignored:
  `.env`, `.env.*`, `*.pem`, `*.key`, `*.p12`, `*.pfx`, `*.keystore`, `*.jks`,
  `id_rsa*`, `id_ed25519*`, `.npmrc`, `.netrc`, `.pgpass`, `credentials`, `secrets.*`

The deny-list is user-extensible via `.nerve/config.toml`. It is applied **before** file
contents are read, so denied files are never loaded into memory.

## Data at rest

- The index lives in `.nerve/` inside the repository and is git-ignored by default
  (`.nerve/.gitignore` contains `*`).
- `nerve.db` is created with mode `0600` on Unix.
- Source text is **not** copied into the database. Only source ranges and content hashes are
  stored; current source is read from disk when evidence is presented. This limits the blast
  radius if the index is shared or leaked, and keeps "verbatim source" true by construction.

## Deferred, with a gate

A written threat model is required **before**:

- Document ingestion (Slice 5) — prompt-injection surface via indexed prose.
- MCP server (Slice 8) — untrusted client surface.
- Any future networked or team-hosted mode.

## Reporting

Security issues should be reported privately to the maintainers before public disclosure.
