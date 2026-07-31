// Forms Nerve does not model. Each is counted; none produces an edge.

export class Base {}

export function mixin(base: unknown): unknown {
  return base;
}

// UNMODELLED: the superclass is computed, so there is no EXTENDS edge to Base.
// The `mixin(Base)` call itself is still an ordinary call site.
export class Mixed extends mixin(Base) {}

export function unmodelledForms(
  table: Record<string, () => void>,
  key: string,
  make: () => () => void,
): void {
  table[key]();
  make()();
  (function () {
    return 1;
  })();
}
