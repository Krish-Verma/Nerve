"""FastAPI routes that a rule can read from the source alone.

Every route here is a positive case: the application object is bound at module scope in this file
to a call of a constructor imported from `fastapi`, and the path is a plain string literal.
"""

from fastapi import APIRouter, FastAPI

app = FastAPI()
router = APIRouter(prefix="/v1")


@app.get("/users")
def list_users():
    return []


@app.get("/users/{user_id}")
def read_user(user_id: int):
    return {"id": user_id}


@app.post("/users")
async def create_user(body: dict):
    return body


@app.delete("/users/{user_id}")
def delete_user(user_id: int):
    return None


# The prefix is NOT composed into the recorded address. `APIRouter(prefix="/v1")` means the
# deployed path is `/v1/items` only if someone calls `include_router`, which is a fact in another
# file. The declared path is `/items`, and that is what is recorded.
@router.get("/items")
def list_items():
    return []


def not_a_route():
    """No decorator, so no endpoint. Present so the fixture has a control."""
    return 1
