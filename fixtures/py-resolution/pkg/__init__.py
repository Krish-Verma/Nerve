"""A package whose `__init__` re-exports one name.

`from pkg import Engine` therefore has a chain to follow, and following it is a Python fact
rather than a guess: `pkg.Engine` **is** `pkg.core.Engine`, because `pkg/__init__.py` binds it
from `.core` unconditionally at module scope.
"""

from .core import Engine

__all__ = ["Engine"]
