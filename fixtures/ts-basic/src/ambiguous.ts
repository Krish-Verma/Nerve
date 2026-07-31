// Two identically named functions in different scopes, plus a dynamic import whose
// argument is not a literal. The dynamic import names no specifier, so it must produce
// no edge at all rather than an invented one.

export function outerA(): string {
  function shared(): string {
    return 'a';
  }
  return shared();
}

export function outerB(): string {
  function shared(): string {
    return 'b';
  }
  return shared();
}

export async function loadDynamic(specifier: string): Promise<unknown> {
  return import(specifier);
}
