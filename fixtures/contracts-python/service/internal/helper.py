"""A directory inside `service` that `{ path = "./internal" }` points at. It holds no pyproject.toml
and no `.nerve/`, so the declaration resolves inside the repository being scanned."""


def helper() -> int:
    return 1
