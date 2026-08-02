# ts-coverage fixture

A small TypeScript tree plus one hand-written LCOV report, used by
`crates/nerve-index/tests/coverage_precision.rs` to measure what the `coverage` extractor emits
against what a human read out of the report.

`coverage/lcov.info` is written by hand rather than generated, so that every `DA:` line is a
deliberate case:

- `src/math.ts` — `add` fully covered, `clamp` **partially** covered (one branch never taken),
  `neverRun` instrumented and never executed, so it must get **no edge at all**.
- `src/shapes.ts` — a constructor and two methods, one of them partially covered, plus lines
  outside every symbol (the `import`, the `class` line, the `interface`) which must be counted as
  unattributed rather than attached to something.

The expected edges are in `expected.json`. They are a **regression gate, not an accuracy claim**:
one hand-built corpus says nothing about how the mapping behaves on a real repository.
