"""Routes Nerve declines to record, each for a stated reason, each counted by form.

These are not silent misses. Every construct here increments a named counter that the fixture gate
asserts, so the precision denominator is auditable and a silently growing set of unreadable forms
fails the build. Same discipline as Slice 9b's unmodelled-site tally.
"""

from fastapi import FastAPI
from flask import Flask

app = FastAPI()
site = Flask(__name__)

PREFIX = "/computed"


# path-not-literal: the address depends on a module-level name.
@app.get(PREFIX + "/items")
def computed_path():
    return []


# path-not-literal: an f-string's value depends on runtime state.
@app.get(f"/users/{PREFIX}")
def interpolated_path():
    return []


# path-not-literal: no positional argument at all.
@app.get()
def no_path():
    return []


# handler-not-a-symbol: a lambda is not a declared symbol, so there is no entity to serve.
app.get("/lambda")(lambda: [])


# methods-not-literal: `methods` is a name rather than a list of string literals.
#
# It has to be a *Flask* object. FastAPI has no `route` decorator at all, so `@app.route(...)` on a
# FastAPI app is not a route in the first place and is correctly not counted as one missed — the
# first draft of this fixture put it there and the extractor was right to disagree.
METHODS = ["GET"]


@site.route("/dynamic-methods", methods=METHODS)
def dynamic_methods():
    return "ok"


# A subscript decorator. Nothing is emitted and **nothing is counted**: Nerve has no reason to
# believe a dict lookup was meant to register a route, and 9a already counts decorator expressions
# whose form is not a dotted name. It stays in the fixture as a must-never-appear case.
HANDLERS = {"get": app.get}


@HANDLERS["get"]("/subscripted")
def subscripted_decorator():
    return []
