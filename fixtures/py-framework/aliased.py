"""Aliased imports, stacked decorators, methods as handlers, and a duplicate address.

Every case here must work through ordinary binding lookup rather than by matching the text
`FastAPI`. A rule that compared spellings would miss all of them.
"""

import functools

from fastapi import FastAPI as WebApp
from flask import Flask as WebServer

api = WebApp()
site = WebServer(__name__)


@api.get("/aliased")
def aliased_constructor():
    """The constructor was renamed at the import. The binding still leads to `fastapi.FastAPI`."""
    return []


@site.route("/aliased-flask")
def aliased_flask():
    return "ok"


def trace(fn):
    """An ordinary decorator, stacked with the route decorator below."""

    @functools.wraps(fn)
    def wrapper(*args, **kwargs):
        return fn(*args, **kwargs)

    return wrapper


@api.get("/wrapped-below")
@trace
def wrapper_below_route():
    """The route decorator is outermost. `@trace` sits between it and the function."""
    return []


@trace
@api.get("/wrapped-above")
def wrapper_above_route():
    """The route decorator is innermost. Whether the wrapper preserves the handler's runtime
    identity is not knowable from the source — which is exactly why `SERVED_BY` states a
    declaration and not a dispatch."""
    return []


class Views:
    """A method can be a handler. The `SERVED_BY` target is then a `Method`, not a `Function`."""

    @api.get("/method-handler")
    def as_method(self):
        return []


# duplicate-address: the same method and path declared twice in one module is ambiguous. Both edges
# are kept — the source really does say this twice — and the ambiguity is flagged rather than
# resolved by picking one.
@api.get("/twice")
def first_declaration():
    return []


@api.get("/twice")
def second_declaration():
    return []
