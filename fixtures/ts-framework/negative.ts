// Express-shaped code that declares no route. Every endpoint from this file is a false positive.
//
// `.get` and `.post` are ordinary method names. A rule matching the decorator or call spelling
// rather than what the receiver is bound to would find four routes here.

class Cache {
  get(key: string, fallback: unknown): unknown {
    void key;
    return fallback;
  }
  post(key: string, value: unknown): void {
    void key;
    void value;
  }
}

const cache = new Cache();

export function handler(): void {}

cache.get("/not-a-route", handler);
cache.post("/also-not-a-route", handler);

// A local class named Router is still not express.Router.
class Router {
  get(path: string, fn: unknown): void {
    void path;
    void fn;
  }
}

const router = new Router();
router.get("/still-not-a-route", handler);

// A plain object with a `get` property.
const app = { get: (path: string, fn: unknown) => { void path; void fn; } };
app.get("/object-literal", handler);

// A Map has a `get`, with one argument.
const registry = new Map<string, unknown>();
registry.get("/map-get");
