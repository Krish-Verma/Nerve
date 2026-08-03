"""Classes, inheritance, and the two receivers that decide Python's unresolved rate.

`Engine.start(...)` names a class Nerve indexed, so the method it names is resolvable.
`self.start()` is not: `self` is a parameter name, not a language keyword, and treating it as
the enclosing class would be type inference by convention. Nerve records that site as
unresolved and counts it rather than guessing at it.
"""

from .util import scale


class Engine:
    """A base class."""

    limit = 10

    def start(self, value: int) -> int:
        """An imported call inside a method."""
        return scale(value)

    def restart(self, value: int) -> int:
        """`self.start` is unresolved. See the module docstring."""
        return self.start(value)

    def ratio(self, value: int) -> int:
        """A module symbol in value position, then a call through the local it was bound to."""
        doubler = scale
        return doubler(value)

    @staticmethod
    def make() -> "Engine":
        """The class name in value position, inside one of the class's own methods."""
        return Engine()


class Turbo(Engine):
    """`class Turbo(Engine)` states inheritance, so Nerve records `EXTENDS`.

    It never records `IMPLEMENTS`: Python has no `implements` keyword, and an abstract base
    class in a base list states inheritance exactly as any other base does.
    """

    def boost(self, value: int) -> int:
        """An explicit class receiver: the class is named, so the method is named too."""
        return Engine.start(self, value)
