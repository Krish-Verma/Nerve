"""Relative imports at two depths, and one that climbs past the repository root."""

from ..util import scale
from ..core import Engine
from . import missing_sibling
from .... import nothing


def descend(value: int) -> int:
    return scale(value)
