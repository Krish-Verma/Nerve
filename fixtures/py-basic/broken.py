"""A syntax error mid-file. The parse degrades to a partial one; the index does not abort."""


def before() -> int:
    return 1


def broken(:
    return 2


def after() -> int:
    return 3
