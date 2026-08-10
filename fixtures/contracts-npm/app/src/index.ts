export function greet(name: string): string {
  return `hello ${name}`;
}

export function main(): string {
  return greet("app");
}
