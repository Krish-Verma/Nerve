"""Two facts about the traced repository: what to call it, and which commit it was on.

Both go in the header, and the commit is half of what makes an artifact *bind* to a repository state
rather than float free of one (`docs/plans/slice-11a-trace-ingestion.md` §5).

`git` is **not** executed to obtain it. `crates/nerve-index/src/gitinfo.rs` reads the same plumbing
files directly and records why: a repository can ship a `.git/config` that turns `git` into an
arbitrary program, and spawning anything in a directory whose contents are untrusted is a needless
surface. The tracer runs inside the user's test process, which makes it a *worse* place to spawn
something, not a better one. Every failure degrades to `None`; an exotic git layout is not an error.

The reader is stricter than git's own plumbing: `trace.rs::optional_hex_field` requires exactly 40
**lowercase** hex characters for `git_commit`, so anything else is reported as no commit at all
rather than as a commit Nerve will refuse the header over.
"""

from __future__ import annotations

import os

_HEX = frozenset("0123456789abcdef")


def root_name(root):
    """The final path segment of `root`, or `None` if it has none Nerve would accept.

    `repository_root_name` is compared against the index's own root name to decide whether the
    artifact is even about this repository, and `read_header` refuses a value containing a path
    separator. A root with no usable final segment — the filesystem root itself — has no name to
    compare, so the answer is `None` and the caller declines to trace rather than inventing one.
    """
    resolved = os.path.realpath(root)
    name = os.path.basename(resolved)
    if not name or "/" in name or "\\" in name:
        return None
    if any(ord(character) < 0x20 for character in name):
        return None
    return name


def _is_commit(text):
    return len(text) == 40 and all(character in _HEX for character in text)


def _git_dir(root):
    dot_git = os.path.join(root, ".git")
    if os.path.isdir(dot_git):
        return dot_git
    if os.path.isfile(dot_git):
        # A worktree or submodule: `.git` is a file holding `gitdir: <path>`.
        text = _read(dot_git)
        if text is None or not text.startswith("gitdir:"):
            return None
        target = text[len("gitdir:") :].strip()
        if not target:
            return None
        resolved = target if os.path.isabs(target) else os.path.join(root, target)
        if os.path.isdir(resolved):
            return resolved
    return None


def _read(path):
    try:
        with open(path, "r", encoding="utf-8", errors="strict") as handle:
            return handle.read(4096).strip()
    except (OSError, UnicodeDecodeError):
        return None


def _packed_ref(git_dir, ref_name):
    text = _read(os.path.join(git_dir, "packed-refs"))
    if text is None:
        return None
    for line in text.splitlines():
        line = line.strip()
        if not line or line.startswith("#") or line.startswith("^"):
            continue
        parts = line.split(" ", 1)
        if len(parts) != 2:
            continue
        object_id, name = parts[0], parts[1].strip()
        if name == ref_name and _is_commit(object_id):
            return object_id
    return None


def git_commit(root):
    """The commit `HEAD` points at, or `None` when there is no readable git state."""
    git_dir = _git_dir(root)
    if git_dir is None:
        return None
    head = _read(os.path.join(git_dir, "HEAD"))
    if head is None:
        return None
    if head.startswith("ref:"):
        ref_name = head[len("ref:") :].strip()
        # A crafted `HEAD` must not become a read of an arbitrary file.
        if not ref_name or ".." in ref_name or ref_name.startswith("/"):
            return None
        loose = _read(os.path.join(git_dir, *ref_name.split("/")))
        if loose is not None and _is_commit(loose):
            return loose
        return _packed_ref(git_dir, ref_name)
    return head if _is_commit(head) else None
