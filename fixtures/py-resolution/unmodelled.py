"""Callee and base-class shapes Nerve declines to model. Each is counted, never guessed."""

from pkg.core import Engine


def make_base():
    """Returns a class. Which class it returns on any given run is a runtime fact."""
    return Engine


class Computed(make_base()):
    """A computed base. No `EXTENDS` edge is invented, and the call itself is still a real call
    site that is recorded."""


class Boxed(Engine[int]):
    """A subscripted base, and Nerve does **not** strip it to `Engine` the way it strips
    TypeScript's `extends Base<number>`.

    The asymmetry is the languages': a TypeScript type argument is erased at run time, while
    `Engine[int]` calls `Engine.__class_getitem__(int)` and the base is whatever that returns.
    The name `Engine` is still referenced, and that much is recorded.
    """


class Rocket(Engine):
    """Callee shapes with no name to record."""

    def go(self, table, index, factory) -> int:
        super().go()
        table[index]()
        factory()()
        (lambda: 1)()
        return 0
