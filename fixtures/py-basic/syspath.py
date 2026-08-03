"""A module that mutates `sys.path`.

Absolute import resolution consults `sys.path`, so once a module rewrites it, no absolute
specifier in that module means what the repository layout says it means. `pkg.util` below
would otherwise resolve; here it must not. Relative imports are unaffected — they are resolved
from `__package__`, not from `sys.path` — which is why the poison is narrow rather than total.
"""

import sys

sys.path.append("vendor")

from pkg.util import scale


def widened(value: int) -> int:
    return scale(value)
