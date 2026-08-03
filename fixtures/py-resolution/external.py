"""Names that leave the repository, plus one that names an indexed module without naming
anything in it. Each is recorded as unresolved, never dropped.

`importlib.import_module` and `__import__` are the exception: `py-structural` already records
each of them as an import finding, so re-reporting one as a call would count a single statement
twice. They are counted as unmodelled call sites instead — the same treatment `ts-js-reference`
gives `require('./x')`.
"""

import importlib
import json
from abc import ABC
from json import dumps
from pkg.util import missing


class Abstract(ABC):
    """`ABC` comes from the standard library, which is not indexed.

    The edge is `EXTENDS`, not `IMPLEMENTS`: Python has no `implements` keyword, and an
    abstract base class in a base list states inheritance exactly as any other base does.
    """

    def encode(self) -> str:
        return dumps({})


def encode_via_module(payload) -> str:
    """The receiver is a module Nerve did not index."""
    return json.dumps(payload)


def call_missing() -> int:
    """`pkg/util.py` is indexed and defines no `missing`."""
    return missing()


def report(value: int) -> None:
    """A builtin: nothing in the scope chain binds it."""
    print(value)


def load(name: str):
    """The runtime import hook."""
    return importlib.import_module(name)


def load_builtin(name: str):
    """The other dynamic form."""
    return __import__(name)
