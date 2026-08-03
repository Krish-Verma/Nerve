"""Top-level module: absolute in-repo imports, one that leaves the repository, and one that
names a directory with no `__init__.py`."""

import os
from pkg.util import scale
from pkg import bootstrap
import pkg.sub.deep
from nspkg import orphan


def run(value: int) -> int:
    """A module-level function."""
    return scale(value)
