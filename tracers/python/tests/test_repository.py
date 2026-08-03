"""Reading the repository's name and commit without running `git`.

`git_commit` is half of what makes an artifact *bind* to a repository state
(`docs/plans/slice-11a-trace-ingestion.md` §5), so getting it wrong has two failure modes and they are
not symmetric. Returning `None` when a commit exists downgrades the binding to `unverified`, which is
honest but weaker. Returning the *wrong* commit would make a trace of state A look like evidence for
state B, which is the thing the binding exists to prevent — so every malformed case here must return
`None` rather than a guess.
"""

from __future__ import annotations

import os
import tempfile
import unittest

from nerve_trace.repository import git_commit, root_name

SHA = "a1ddbdf" + "0" * 33
OTHER = "b" * 40


def write(path, text):
    os.makedirs(os.path.dirname(path), exist_ok=True)
    with open(path, "w", encoding="utf-8", newline="\n") as handle:
        handle.write(text)


class RootNameTests(unittest.TestCase):
    def setUp(self):
        self._temporary = tempfile.TemporaryDirectory()
        self.root = os.path.realpath(self._temporary.name)

    def tearDown(self):
        self._temporary.cleanup()

    def test_the_final_segment_is_the_name(self):
        nested = os.path.join(self.root, "my-repo")
        os.makedirs(nested)
        self.assertEqual(root_name(nested), "my-repo")

    def test_a_trailing_separator_does_not_change_the_answer(self):
        nested = os.path.join(self.root, "my-repo")
        os.makedirs(nested)
        self.assertEqual(root_name(nested + os.sep), "my-repo")

    def test_the_filesystem_root_has_no_name_to_compare(self):
        """`read_header` refuses a root name containing a separator, so there is nothing to send."""
        self.assertIsNone(root_name("/"))


class GitCommitTests(unittest.TestCase):
    def setUp(self):
        self._temporary = tempfile.TemporaryDirectory()
        self.root = os.path.realpath(self._temporary.name)

    def tearDown(self):
        self._temporary.cleanup()

    def git(self, *parts):
        return os.path.join(self.root, ".git", *parts)

    def test_no_git_directory_is_not_an_error(self):
        self.assertIsNone(git_commit(self.root))

    def test_a_symbolic_head_is_read_through_a_loose_ref(self):
        write(self.git("HEAD"), "ref: refs/heads/main\n")
        write(self.git("refs", "heads", "main"), SHA + "\n")
        self.assertEqual(git_commit(self.root), SHA)

    def test_a_loose_ref_that_is_absent_falls_back_to_packed_refs(self):
        write(self.git("HEAD"), "ref: refs/heads/main\n")
        write(
            self.git("packed-refs"),
            "# pack-refs with: peeled fully-peeled sorted\n"
            + OTHER
            + " refs/heads/other\n"
            + SHA
            + " refs/heads/main\n",
        )
        self.assertEqual(git_commit(self.root), SHA)

    def test_a_peeled_tag_line_in_packed_refs_is_skipped(self):
        write(self.git("HEAD"), "ref: refs/heads/main\n")
        write(
            self.git("packed-refs"),
            SHA + " refs/heads/main\n^" + OTHER + "\n",
        )
        self.assertEqual(git_commit(self.root), SHA)

    def test_a_detached_head_is_the_commit_itself(self):
        write(self.git("HEAD"), SHA + "\n")
        self.assertEqual(git_commit(self.root), SHA)

    def test_a_worktree_pointer_is_followed(self):
        real = os.path.join(self.root, "actual-git-dir")
        write(os.path.join(real, "HEAD"), SHA + "\n")
        write(os.path.join(self.root, ".git"), "gitdir: actual-git-dir\n")
        self.assertEqual(git_commit(self.root), SHA)

    def test_a_crafted_head_cannot_read_an_arbitrary_file(self):
        write(self.git("HEAD"), "ref: ../../../../etc/passwd\n")
        self.assertIsNone(git_commit(self.root))

    def test_an_absolute_ref_is_refused(self):
        write(self.git("HEAD"), "ref: /etc/passwd\n")
        self.assertIsNone(git_commit(self.root))

    def test_anything_that_is_not_a_commit_is_no_commit_at_all(self):
        for content in (
            "not-a-sha\n",
            "\n",
            SHA[:39] + "\n",
            SHA + "0\n",
            "A" * 40 + "\n",
            "g" * 40 + "\n",
            # 64 hex is a valid object id to git, and is not a value `git_commit` may carry: the
            # reader requires exactly 40 lowercase hex characters.
            "0" * 64 + "\n",
        ):
            write(self.git("HEAD"), content)
            self.assertIsNone(git_commit(self.root), content.strip())

    def test_an_uppercase_commit_is_refused_rather_than_lowercased(self):
        """`optional_hex_field` requires lowercase; normalising here would hide a strange git."""
        write(self.git("HEAD"), SHA.upper() + "\n")
        self.assertIsNone(git_commit(self.root))

    def test_a_readable_answer_is_always_the_shape_the_reader_accepts(self):
        write(self.git("HEAD"), "ref: refs/heads/main\n")
        write(self.git("refs", "heads", "main"), SHA + "\n")
        answer = git_commit(self.root)
        self.assertEqual(len(answer), 40)
        self.assertEqual(answer, answer.lower())
        self.assertTrue(all(character in "0123456789abcdef" for character in answer))


if __name__ == "__main__":
    unittest.main()
