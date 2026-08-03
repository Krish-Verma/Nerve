"""Negative fixture: specifiers that must not resolve, though something similar exists.

Every line here is the plausible wrong answer for a resolver that searched by basename, walked
into a package it was not told to, or guessed that an imported name is a submodule.
"""

import util
from core import Engine
import pkg.util.scale
from pkg import core


def local_scale(value: int) -> int:
    return value
