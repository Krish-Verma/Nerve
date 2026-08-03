"""Comprehension scopes, `global` and `nonlocal`: three places where Python's scope chain is
not the one a C-family reader expects.

Each function below is chosen so that **removing** the rule changes the answer. A fixture that
would pass with the rule deleted measures nothing.
"""

from pkg.util import scale


def target() -> int:
    """A module-level function whose name a comprehension variable also uses."""
    return 1


def comprehension_binds_its_own_name(items):
    """The comprehension's `target` is local to the comprehension, so the module function is
    shadowed inside it and no edge may be produced."""
    return [target for target in items]


def comprehension_body_reaches_outward(items):
    """Nothing local binds `scale`, so the comprehension body reaches the import."""
    return [scale(item) for item in items]


def rebinds_a_global() -> int:
    """`global target` says the assignment below rebinds the module-level name.

    Without the declaration Python would make `target` local to this whole function, and the
    call above the assignment would reach nothing at all.
    """
    global target
    result = target()
    target = None
    return result


def outer() -> int:
    """`nonlocal` binds the enclosing function's name.

    Without the declaration, `helper = None` would make `helper` local to `replace` for the
    whole of its body, and the call above it would reach nothing.
    """

    def helper() -> int:
        return 1

    def replace() -> int:
        nonlocal helper
        value = helper()
        helper = None
        return value

    return replace()
