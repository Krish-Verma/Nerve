// A module with both a named export and a default export.

export function named(): number {
  return 1;
}

function secret(): number {
  return 2;
}

export default secret;
