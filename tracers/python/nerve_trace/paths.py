"""Relativising a frame's filename against the repository root, or dropping it.

Two rules, and both are about what an artifact must never contain rather than about convenience.

**Never an absolute path.** An artifact naming `/Users/someone/project/src/x.py` leaks a filesystem
layout to whoever reads the artifact — and Nerve refuses it anyway, as `path-refused`, through the
shared traversal check Slice 8b-i introduced. Relativising in the producer is the only place the leak
can be prevented rather than detected.

**Never a `..` segment, and never a backslash spelling.** A frame outside the repository root is
**dropped and counted**, not emitted as `../../site-packages/pytest/python.py`. Most frames in a real
run are outside the root — the standard library, pytest itself, every installed package — so this is
the common path, not an edge case, and it is why the resolved answer is memoised on `co_filename`:
without the cache a `realpath` syscall would run on every frame the interpreter enters.

Dropping is silent in the artifact by design. A frame in `site-packages` is not a gap in the trace,
it is a file Nerve has no row for; recording it as a limitation would put the standard library's
existence in the `producer_limitations` tally. The count is reported to the user in pytest's terminal
summary instead, so a root that was wrong — everything dropped — is loud rather than invisible.
"""

from __future__ import annotations

import os

from .record import MAX_STRING_BYTES


class PathScope:
    """Maps a frame's `co_filename` to a repository-relative path, or to `None`.

    `None` means *not in this repository*, which is the only reason this class ever declines. It is
    not a judgement about the file.
    """

    def __init__(self, root):
        resolved = os.path.realpath(root)
        self.root = resolved
        self._prefix = resolved if resolved.endswith(os.sep) else resolved + os.sep
        self._cache = {}
        #: Distinct `co_filename` values that resolved outside the root. Distinct rather than total,
        #: because the cache means each filename is judged once and the interesting number is how
        #: many *files* were out of scope.
        self.outside_root = 0

    def relative(self, filename):
        """The repository-relative path for `filename`, or `None` if it is not under the root."""
        cached = self._cache.get(filename, False)
        if cached is not False:
            return cached
        value = self._resolve(filename)
        self._cache[filename] = value
        if value is None:
            self.outside_root += 1
        return value

    def _resolve(self, filename):
        if not isinstance(filename, str) or not filename:
            return None
        # `<string>`, `<frozen importlib._bootstrap>`, `<stdin>`: code with no file to name. The
        # `generated-code` limitation covers the class; there is no path to relativise.
        if filename.startswith("<"):
            return None
        if any(ord(character) < 0x20 for character in filename):
            return None
        try:
            resolved = os.path.realpath(filename)
        except OSError:
            return None
        if not resolved.startswith(self._prefix):
            return None
        relative = resolved[len(self._prefix) :]
        if not relative:
            return None
        relative = relative.replace(os.sep, "/")
        if os.altsep:
            relative = relative.replace(os.altsep, "/")
        # `realpath` has already collapsed `..` and made the path absolute, so these are assertions
        # about the result rather than parsing of the input — the belt to `realpath`'s braces.
        if relative.startswith("/") or "\\" in relative:
            return None
        if "" in relative.split("/") or ".." in relative.split("/"):
            return None
        if len(relative.encode("utf-8")) > MAX_STRING_BYTES:
            # Nerve would refuse the record as `string-too-long`; emitting it would spend a refusal
            # counter on a path this producer could see was too long.
            return None
        return relative
