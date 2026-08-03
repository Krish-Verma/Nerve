"""An application object imported from another module.

`app` here is the same object `fastapi_app.py` created, and a human reads this as three more routes
on that application. Nerve does not record them, and the reason is the stated lower bound in plan
§5.1: the binding that proves `app` is a framework object lives in another file, and following it
means tracking a value across modules rather than reading a binding.

This is a **known limitation, counted as `app-not-local`** — not a silent miss. It is the honest
analogue of 9a's `sys.path` refusal, which also deliberately drops edges that would otherwise
resolve.
"""

from fastapi_app import app, router


@app.get("/imported")
def imported_app_route():
    return []


@router.post("/imported-router")
def imported_router_route():
    return []
