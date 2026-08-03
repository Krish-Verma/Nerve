// Registrations Nerve declines to record, each for a stated reason, each counted by form.

import express from "express";

const app = express();
const PREFIX = "/computed";

// path-not-literal: the address depends on a module-level name.
app.get(PREFIX + "/items", handlerA);

// path-not-literal: a template literal with a substitution.
app.get(`/users/${PREFIX}`, handlerA);

// handler-not-a-symbol: an inline arrow is not a declared symbol.
app.get("/inline", (req: unknown, res: unknown) => {
  void req;
  void res;
});

// handler-not-a-symbol: an inline function expression.
app.post("/inline-expression", function (req: unknown, res: unknown) {
  void req;
  void res;
});

// handler-not-a-symbol: a member expression is a callable the source does not name locally.
const handlers = { list: handlerA };
app.get("/member", handlers.list);

export function handlerA(req: unknown, res: unknown): void {
  void req;
  void res;
}
