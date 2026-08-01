export function relocate(): string {
  const parts = ['alpha', 'beta', 'gamma'];
  return parts.join('-');
}

export function annotate(value: string): string {
  return value.trim().toUpperCase();
}
