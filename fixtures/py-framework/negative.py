"""Framework-shaped code that declares no route. Every endpoint emitted from this file is a
false positive.

The point of the file is that `.get`, `.post` and `.route` are ordinary method names. A rule that
matches on the decorator's spelling rather than on what the receiver is bound to would find five
routes here.
"""


class Cache:
    """A cache has a `get`. It is not a web framework."""

    def get(self, key):
        return None

    def post(self, key, value):
        return None


cache = Cache()


@cache.get("/not-a-route")
def looks_like_a_handler():
    """`cache` is bound to `Cache()`, which is not a framework constructor. Nerve does not know
    that `@cache.get(...)` is a route, so it emits nothing — and counts nothing either, because a
    tally of missed routes here would itself be a false claim."""
    return None


class Router:
    """Even the name is suggestive. Still not `fastapi.APIRouter`."""

    def get(self, path):
        def wrap(fn):
            return fn

        return wrap


router = Router()


@router.get("/also-not-a-route")
def another_handler():
    return None


def get(path):
    """A bare module-level function named `get`, used as a decorator factory."""

    def wrap(fn):
        return fn

    return wrap


@get("/bare-decorator")
def third_handler():
    return None


# An ordinary call that happens to look like Express. Python, not JavaScript, and `app` here is a
# plain dict.
app = {"get": lambda path, handler: None}
app["get"]("/subscript", third_handler)
