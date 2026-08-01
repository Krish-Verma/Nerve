// Negative fixture. Same symbol names as src/movable.ts, entirely different bodies.
//
// Deleting src/movable.ts while this file appears must propose no identity link. A name match
// is not evidence (ADR-0002); the body digest is what carries the claim, and these bodies share
// nothing with the originals.

export function relocate(): string {
  throw new Error('unrelated implementation that happens to share a name');
}

export function annotate(value: string): string {
  return `${value.length}`;
}
