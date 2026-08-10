// A directory inside `app` that `file:./internal` points at. It holds no package.json and no
// `.nerve/`, so the declaration resolves inside the repository being scanned rather than at a
// neighbour — which is a different fact from "that path is unregistered".
export function localHelper(): number {
  return 1;
}
