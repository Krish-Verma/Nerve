"""A module in a directory with no `__init__.py`.

That directory is a *namespace package*: its contents are assembled at runtime from every
`sys.path` entry that holds a directory of the same name, so `from nspkg import orphan` names
something Nerve cannot claim to have found.
"""


def orphaned() -> int:
    return 1
