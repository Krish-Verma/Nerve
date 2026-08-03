"""A file that does not parse cleanly, with a readable route above the damage.

A syntax error must not lose the routes the tree did read, and must not be reported as a route
either. `tree-sitter` recovers, so the rule sees a valid `decorated_definition` for `before_damage`
and an ERROR node afterwards.
"""

from fastapi import FastAPI

app = FastAPI()


@app.get("/before-damage")
def before_damage():
    return []


def broken(:
    return
