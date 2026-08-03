"""A leaf module with no imports.

The positive resolution target for an absolute (`pkg.util`) and a relative (`..util`)
specifier alike.
"""


def scale(value: int) -> int:
    """Multiply by two. Named `scale` on purpose: `negative.py` writes a bare `import util`,
    and a resolver that searched by basename would wrongly land here."""
    return value * 2
