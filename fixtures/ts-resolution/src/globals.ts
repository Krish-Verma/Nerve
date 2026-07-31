// AMBIGUOUS: `console` is a host global. Nerve records the gap rather than guessing.

export function report(value: number): void {
  console.log(value);
}
