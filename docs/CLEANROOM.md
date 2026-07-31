# Nerve Clean-Room Statement

**Effective date:** 2026-07-31
**Status:** Binding on all contributors and all automated agents working in this repository.

---

## 1. Declaration

Nerve is an **independent implementation**. Its engine, schema, indexing pipeline, query
system, evidence model, CLI, MCP server, and visual product are original work owned by this
project.

- No competitor code is incorporated, in whole or in part.
- No competitor product is a runtime dependency, build dependency, or optional dependency.
- No competitor database, on-disk format, or API is read, written, consumed, or migrated.
- Nerve does not require, detect, or interoperate with any competing product.

"Competitor" here includes, without limitation: CodeGraph, Graphify, GitNexus, and any other
code-knowledge-graph, code-index, or code-intelligence product.

---

## 2. Basis of the implementation

The implementation is derived exclusively from:

1. Nerve's own written requirements (`docs/PRODUCT.md`, `docs/ARCHITECTURE.md`,
   `docs/plans/nerve-master-build-plan.md`, and the decision records in `docs/decisions/`).
2. Primary documentation for foundational technologies (Tree-sitter, SQLite, Rust crates,
   the Model Context Protocol specification, OS filesystem APIs).
3. Nerve's own fixtures, tests, and measurements.

General software-engineering concepts are not proprietary and may be implemented
independently: abstract syntax tree parsing, symbol indexing, graph traversal, incremental
invalidation, content-addressed caching, file watching, full-text search, evidence
provenance, MCP tool design, local graph visualization, static analysis, test-evidence
ingestion, and runtime tracing.

---

## 3. Prior research disclosure

Competitive research was conducted **before** this clean-room decision was taken. That
research was limited to publicly published material: README files, public documentation
sites, package manifests, license files, and repository metadata retrieved from public APIs.

That research may inform **product-level** decisions only — what problems are worth solving,
what the market already serves, where the gaps are.

It must not inform implementation. Specifically:

- Competitor schemas must not be recreated from memory.
- Competitor algorithms must not be recreated from memory.
- Competitor internal identifiers, table names, column names, tool names, file layouts, or
  configuration keys must not be reproduced.
- Similarity of outcome is acceptable where it is forced by the problem or by a foundational
  library. Similarity of *internal structure* is not.

From the effective date onward, **no additional competitor source code may be inspected**
by any contributor or agent working on the implementation.

---

## 4. Prohibited tooling

Agents working on this repository must not invoke competitor-specific skills, plugins, MCP
servers, or CLIs to guide implementation — including any locally installed `graphify`,
`gitnexus`, or `codegraph` skill or binary.

Permitted tooling: generic architecture, Rust, TypeScript, frontend, design, database,
testing, security, performance, and debugging assistance.

---

## 5. Justification requirement

Every non-obvious implementation decision must be justified by **Nerve's own** requirements,
fixtures, tests, or measurements — recorded in an ADR under `docs/decisions/`.

"Because that is how it is usually done" is acceptable when supported by primary
documentation. "Because a competitor does it that way" is not acceptable and must never
appear as a rationale.

---

## 6. Dependency review

Every third-party dependency must be:

1. Recorded in `third_party/LICENSES.md` with name, version, license, and purpose.
2. Reviewed for license compatibility with a commercial product.
3. Rejected if it carries copyleft obligations incompatible with the intended distribution,
   noncommercial restrictions, or source-available restrictions.

Known-prohibited license families for Nerve: PolyForm Noncommercial, Business Source License
(during its restricted term), Commons Clause, SSPL, and any "source-available, not open
source" license. GPL/AGPL dependencies require an explicit, documented decision before use.

---

## 7. Contamination response

If any contributor or agent discovers implementation detail in Nerve that appears to be
derived from a competitor:

1. **Stop work on the affected area immediately.**
2. Report it in the slice report and open a tracking note in `docs/decisions/`.
3. Remove or independently re-derive the affected code from Nerve's requirements.
4. Record the incident, the removal, and the re-derivation in this file's log below.

---

## 8. Incident log

| Date | Area | Finding | Resolution |
|---|---|---|---|
| — | — | No incidents recorded. | — |

---

## 9. Attestation

| Slice | Date | Clean-room check performed | Result |
|---|---|---|---|
| 1 | 2026-07-31 | Dependency audit; source-tree scan for competitor names; confirmation that no competitor DB/API is read | Pass |
| 2a | 2026-07-31 | Scan of new resolver sources and fixtures for competitor names; confirmed no new dependency (`Cargo.lock` byte-identical); confirmed no network surface in `bind.rs`/`refs.rs`/`exports.rs`; no competitor skill invoked | Pass |
