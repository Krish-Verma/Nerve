// Express routes a rule can read from the source alone.
//
// Every route here is a positive case: the application object is bound at module scope in this file
// to a call of a constructor imported from `express`, and the path is a plain string literal.

import express from "express";

const app = express();
const router = express.Router();

export function listUsers(req: unknown, res: unknown): void {
  void req;
  void res;
}

export function readUser(req: unknown, res: unknown): void {
  void req;
  void res;
}

export function createUser(req: unknown, res: unknown): void {
  void req;
  void res;
}

export function notARoute(): number {
  return 1;
}

app.get("/users", listUsers);
app.get("/users/:id", readUser);
app.post("/users", createUser);
app.delete("/users/:id", readUser);

// A router's mount prefix is decided by `app.use("/v1", router)` — a separate fact, and not
// composed in. The declared path is `/items`.
router.get("/items", listUsers);

// `all` is not an HTTP method, and Nerve does not expand it into eight endpoints.
app.all("/anything", listUsers);
