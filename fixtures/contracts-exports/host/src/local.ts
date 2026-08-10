// A relative import target inside `host` itself. `./local` is not a package specifier, so it is
// never a C2 declaration — the oracle records it under `skipped` so that absence is stated.
export const helper = "local";
