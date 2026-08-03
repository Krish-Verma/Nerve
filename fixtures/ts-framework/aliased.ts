// Aliased and destructured imports, a method handler, and a duplicate address.
//
// Every case must work through ordinary binding lookup rather than by matching the text `express`.

import makeServer, { Router as MakeRouter } from "express";

const site = makeServer();
const api = MakeRouter();

export function aliasedHandler(): void {}
export function routerHandler(): void {}
export function firstDeclaration(): void {}
export function secondDeclaration(): void {}

site.get("/aliased", aliasedHandler);
api.post("/aliased-router", routerHandler);

export class Views {
  static handle(): void {}
}

// A static method is a declared symbol, so it can be a handler.
site.get("/method-handler", Views.handle);

// duplicate-address: the same method and path declared twice in one module is ambiguous. Both edges
// are kept — the source really does say this twice — and the ambiguity is flagged.
site.get("/twice", firstDeclaration);
site.get("/twice", secondDeclaration);
