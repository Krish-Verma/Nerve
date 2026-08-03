"""Import forms whose effect is not statically knowable.

Each one is recorded as an `Unresolved` **value**, never as an omission. The module a statement
names may still resolve; what does not resolve is *which names it binds*.
"""

import importlib

from pkg.util import *

try:
    from pkg.core import Engine
except ImportError:  # pragma: no cover
    Engine = None

if True:
    from pkg import bootstrap


def load(name: str):
    """A dynamic import: the module is chosen at runtime."""
    return importlib.import_module(name)


def load_builtin(name: str):
    """The other dynamic form."""
    return __import__(name)
