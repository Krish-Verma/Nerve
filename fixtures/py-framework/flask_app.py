"""Flask routes, including the two forms `route` takes and the method-shorthand form.

`@app.route(...)` carries its methods in a keyword argument; the shorthands carry it in the
decorator name. Both are read here. Flask's own documented default when `methods` is absent is
`GET`, and that default is the framework's rather than Nerve's guess.
"""

from flask import Blueprint, Flask

app = Flask(__name__)
bp = Blueprint("admin", __name__)


@app.route("/health")
def health():
    """No `methods=`, so Flask's documented default applies: GET."""
    return "ok"


@app.route("/submit", methods=["POST"])
def submit():
    return "", 201


@app.route("/both", methods=["GET", "POST"])
def both():
    """Two methods in one decorator are two declared endpoints at one path."""
    return "ok"


@app.get("/shorthand")
def shorthand():
    """Flask 2.0 added the method shorthands FastAPI already had."""
    return "ok"


@bp.post("/admin/purge")
def purge():
    """A Blueprint is a framework constructor too, and the prefix it may be registered under is
    not composed in — same rule as APIRouter."""
    return "", 204
