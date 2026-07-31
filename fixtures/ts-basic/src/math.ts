// Named functions, a nested named function, a declarator-bound arrow, and a default export.

export function add(a: number, b: number): number {
  function normalize(value: number): number {
    return Number.isFinite(value) ? value : 0;
  }
  return normalize(a) + normalize(b);
}

export function subtract(a: number, b: number): number {
  return add(a, -b);
}

export const scale = (value: number, factor: number): number => value * factor;

export default add;
