// Same-module resolution, plus the named and default exports every other module pulls on.

export function add(a: number, b: number): number {
  return normalize(a) + normalize(b);
}

function normalize(value: number): number {
  return value;
}

export function scale(value: number, factor: number): number {
  return value * factor;
}

export default add;
