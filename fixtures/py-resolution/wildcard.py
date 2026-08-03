"""A wildcard import binds a set of names that lives in the imported module's runtime namespace.

Slice 9a records that set as an `Unresolved` value, and 9b does not contradict it by resolving
a name through it. The missing edge below is therefore a **stated false negative** rather than
a forbidden one: at run time `scale` really is `pkg.util.scale`, and Nerve declines to say so
because what `import *` binds is not a property of this file.
"""

from pkg.util import *


def use_wildcard(value: int) -> int:
    return scale(value)
