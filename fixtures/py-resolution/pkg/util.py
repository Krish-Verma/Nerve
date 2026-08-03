"""A leaf module. Nothing here imports anything, so every edge out of it is same-module."""


def scale(value: int) -> int:
    """Multiply by two."""
    return value * 2


def normalize(value: int) -> int:
    """The simplest edge there is, and the one every cross-module case builds on."""
    return scale(value)
