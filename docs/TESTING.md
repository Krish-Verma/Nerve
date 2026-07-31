# Nerve — Testing Strategy

Correctness before speed, token counts, or language coverage.

## Verification gate (every slice)

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cargo build --release
```

Plus a manual CLI smoke test, inspection of generated output, a dependency-license review, and
a clean-room check. A slice is not complete until all have been run **and the output shown**.

## Test categories

| Category | Purpose |
|---|---|
| **Parser unit tests** | One construct per test: named/nested/anonymous functions, classes, methods, interfaces, named/default/re-exports, aliased and relative imports, JSX/TSX |
| **Golden graph tests** | Fixture repo → canonical JSON, compared to a committed golden file. Catches silent extraction regressions that unit tests miss |
| **Determinism** | Index twice into separate databases; canonical dumps must be **byte-identical** |
| **Idempotent re-index** | Re-indexing an unchanged tree produces no logical graph difference and no row growth |
| **Unresolved reference** | Unresolvable imports become `Unresolved` entities with `is_unresolved` assertions — never dropped |
| **Migration** | Fresh database reaches the current `schema_version`; re-opening is a no-op; the version table is correct |
| **`assertion_state` derivation** | Truncate, rebuild, assert identical content — proves it is a pure function of observations |
| **CLI smoke** | `init` → `index` → `status` → `search` end-to-end on a temp copy of the fixture |
| **JSON output contract** | Every `--json` output parses, and required keys are present and stably named |
| **Path safety** | Traversal attempts and symlinks escaping the repository root are rejected |
| **Ignore rules** | `.gitignore`, `.nerveignore`, and the secret deny-list all exclude as specified |
| **FTS5 availability** | `CREATE VIRTUAL TABLE ... USING fts5` succeeds in the bundled build — proven, not assumed |
| **No-network** | No networking crate appears in the dependency tree |
| **Scale / latency** | Synthetic graph, bounded-depth traversal and FTS latency measured against ADR-0001's thresholds |

## Negative fixtures

**Mandatory from Slice 2**, when the first inferred relationships appear. Every extractor that
infers a relationship must ship cases where a naive heuristic *would* fire and the correct
answer is "no relationship". Precision is measured against these and gated in CI.

Slice 1 relations (`CONTAINS`, `DEFINES`, `IMPORTS`, `EXPORTS`) are read directly from the
syntax tree, so ambiguity fixtures exist to pin behaviour rather than to measure precision.

## Metrics we track

Symbol-extraction precision and recall · relationship precision and recall ·
unresolved-reference rate · determinism · indexing speed · incremental update speed ·
query latency · database size · memory usage · source-grounding quality ·
false-positive and false-negative impact results.

## Reporting

Publish losses and limitations, not only wins. Every slice report states known unresolved
cases and ignored tests explicitly.
