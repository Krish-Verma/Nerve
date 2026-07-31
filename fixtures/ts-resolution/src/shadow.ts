// Shadowing. Every call here has an obvious wrong answer that Nerve must not give.

import { add } from './math';

// NEGATIVE: the local `const add` shadows the import.
export function shadowedByConst(): number {
  const add = 1;
  return add;
}

export function target(): number {
  return 1;
}

// NEGATIVE: the parameter shadows the module function `target`.
export function shadowedByParameter(target: () => number): number {
  return target();
}

// POSITIVE: each `helper()` must reach the `helper` in its own scope, not the sibling's.
export function sameNameDifferentScopes(): number {
  function helper(): number {
    return 1;
  }
  return helper();
}

export function otherScope(): number {
  function helper(): number {
    return 2;
  }
  return helper();
}

// NEGATIVE: `scale` is exported by math.ts, but this one is a local.
export function localNameMatchesForeignExport(): number {
  const scale = 5;
  return scale;
}
