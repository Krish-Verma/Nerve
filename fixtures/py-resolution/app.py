"""Every cross-module import form `py-reference` resolves.

`import pkg.util` binds `pkg`, and the target of `pkg.util.scale` is found by taking the
**longest prefix that is an indexed module** — never by matching a basename anywhere in the
tree. `from pkg import Engine` follows `pkg/__init__.py`'s own module-scope import, which is a
Python fact and not a name coincidence.
"""

from pkg.util import scale
from pkg import Engine
import pkg.util
import pkg.core as core

DEFAULT = scale(1)
ALIAS = scale


def direct(value: int) -> int:
    """`from pkg.util import scale`."""
    return scale(value)


def via_dotted_module(value: int) -> int:
    """`import pkg.util` binds `pkg`; `pkg.util` is the longest indexed prefix of the callee."""
    return pkg.util.scale(value)


def via_module_alias() -> "core.Engine":
    """`import pkg.core as core` binds the module under its alias."""
    return core.Engine()


def via_package_re_export() -> Engine:
    """`pkg/__init__.py` binds `Engine` from `.core`, so `pkg.Engine` is `pkg.core.Engine`."""
    return Engine()


def annotated(engine: Engine) -> Engine:
    """A type position is a reference position."""
    return engine


class Derived(core.Engine):
    """A base named through a module alias."""
