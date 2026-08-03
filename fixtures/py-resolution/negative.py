"""Negative fixtures: a shadowed name, a same-named local, and a same-named attribute on a
different class. None of them may produce an edge.

Every line here is the plausible wrong answer for a resolver that matched names instead of
resolving them.
"""

from pkg.core import Turbo
from pkg.util import scale


def helper() -> int:
    """A module-level function whose name a class body below also uses."""
    return 1


def shadowed_by_parameter(scale):
    """The parameter shadows the import. `scale(1)` calls whatever was passed in."""
    return scale(1)


def shadowed_by_local() -> int:
    """A local binding established before the use."""
    scale = 3
    return scale


def assigned_anywhere_is_local() -> int:
    """Python binds a name for the **whole** function body, not from the assignment onwards.

    `scale` is local here even on the line above the assignment — CPython raises
    UnboundLocalError rather than reaching the import — so no edge to the import may exist.
    """
    total = scale
    scale = 2
    return total + scale


class Other:
    """A different class with a method named exactly as `pkg/core.py#Engine.start` is."""

    def start(self, value: int) -> int:
        return value


def call_other_start(other: Other) -> int:
    """The receiver is a parameter, so the call is unresolved.

    It is in particular not `Other.start` because an annotation said so — an annotation is a
    claim about a value, not a resolution of one — and not `Engine.start` because the names
    happen to match.
    """
    return other.start(1)


def inherited_member(value: int) -> int:
    """`Turbo` is indexed and declares no `start`; it inherits one.

    Walking the MRO to find the declaration stops being a syntax fact as soon as one base is
    unresolved, so Nerve refuses uniformly rather than sometimes.
    """
    return Turbo.start(value)


class Scoped:
    """A class body **is** a scope for the code that runs in it and is **not** an enclosing
    scope for the methods defined in it. Both halves are exercised below."""

    helper = 2
    doubled = helper * 2

    def use(self) -> int:
        """`helper` here is the module function: a class body is not an enclosing scope."""
        return helper()
