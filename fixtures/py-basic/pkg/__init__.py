"""A directory holding an `__init__.py` is a package.

Nerve records that on this module's `meta`, not as a new `EntityKind`: a vocabulary member
would touch the UI mirror, `EntityKind::path_role`, and every exhaustiveness test, for a fact
that is already 1:1 with a file Nerve indexes.
"""

from .core import Engine

__all__ = ["bootstrap", "Engine", "not_defined_here"]


def bootstrap() -> None:
    """The only name in `__all__` that resolves to a symbol this module itself defines."""
    return None
