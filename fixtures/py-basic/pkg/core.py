"""Functions, classes, methods, nesting and decorators."""

import functools

from .util import scale


@functools.lru_cache(maxsize=None)
def tune(value: int) -> int:
    """A decorated module-level function.

    The decorator is structural metadata on this symbol, not a call edge: `@functools.lru_cache`
    says something about `tune`, and what it *does* at runtime is Slice 10's problem.
    """

    def inner(step: int) -> int:
        """Nested inside `tune`; its scope path names the enclosing function."""
        return step + 1

    return inner(scale(value))


class Engine:
    """A class whose `__init__` is an ordinary Method, with nothing special about it."""

    limit = 10

    def __init__(self, name: str) -> None:
        self.name = name

    @staticmethod
    def make() -> "Engine":
        return Engine("default")

    @property
    def label(self) -> str:
        return self.name

    async def start(self) -> None:
        return None


class Turbo(Engine):
    """A subclass. `EXTENDS` belongs to Slice 9b; 9a records the class and its methods only."""

    def start(self) -> None:
        return None
