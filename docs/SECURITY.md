# Nerve — Security and Privacy

Binding from the first commit.

## Absolute rules

| Rule | Status |
|---|---|
| No telemetry, no analytics | Absent from the codebase — not opt-out |
| No network access during indexing or query | Enforced by test: no networking crate in the dependency tree |
| No external LLM calls from product code | Absent |
| No repository code execution during indexing | We parse bytes; we never run them |
| No package-script execution | Never invoked |
| No `eval` of source files | Never |
| Local server binds `127.0.0.1` only | From Slice 4 |

## Repository content is untrusted input

Source files, file names, and (later) documents are attacker-controlled data. They are parsed,
never executed; escaped before rendering; and never interpreted as instructions.

## Path safety

- Every path is canonicalized and asserted to be inside the repository root before any read.
- Symlinks are not followed out of the repository root.
- Path traversal via crafted file names (`../`, absolute paths, NUL bytes) is rejected.
- Covered by `path-safety` tests from Slice 1.

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
