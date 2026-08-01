export function helper(): number {
  return 41;
}

export function secondary(): number {
  return helper() + 1;
}
