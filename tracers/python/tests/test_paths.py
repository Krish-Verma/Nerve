"""Relativisation: what reaches an artifact, and what is dropped before it can."""

from __future__ import annotations

import os
import tempfile
import unittest

from nerve_trace.paths import PathScope
from nerve_trace.record import MAX_STRING_BYTES


class PathScopeTests(unittest.TestCase):
    def setUp(self):
        self._temporary = tempfile.TemporaryDirectory()
        self.root = os.path.realpath(self._temporary.name)
        self.scope = PathScope(self.root)

    def tearDown(self):
        self._temporary.cleanup()

    def test_a_file_inside_the_root_becomes_a_relative_path(self):
        inside = os.path.join(self.root, "src", "parse.py")
        self.assertEqual(self.scope.relative(inside), "src/parse.py")

    def test_the_root_itself_is_not_a_file_in_the_repository(self):
        self.assertIsNone(self.scope.relative(self.root))

    def test_a_file_outside_the_root_is_dropped_and_counted(self):
        parent = os.path.dirname(self.root)
        self.assertIsNone(self.scope.relative(os.path.join(parent, "elsewhere.py")))
        self.assertEqual(self.scope.outside_root, 1)

    def test_a_sibling_whose_name_merely_starts_with_the_root_is_dropped(self):
        # A prefix comparison without the separator would accept `/tmp/rootEXTRA/x.py` for the root
        # `/tmp/root`, which is a different directory.
        self.assertIsNone(self.scope.relative(self.root + "EXTRA/x.py"))

    def test_the_standard_library_is_outside_the_root(self):
        self.assertIsNone(self.scope.relative(os.__file__))

    def test_generated_code_has_no_path_to_relativise(self):
        for filename in ("<string>", "<stdin>", "<frozen importlib._bootstrap>"):
            self.assertIsNone(self.scope.relative(filename))

    def test_a_traversal_spelling_never_survives(self):
        # `realpath` collapses it; the result is outside the root, so it is dropped rather than
        # emitted with a `..` segment Nerve would refuse as a traversal.
        traversal = os.path.join(self.root, "..", "escaped.py")
        self.assertIsNone(self.scope.relative(traversal))

    def test_a_traversal_that_lands_back_inside_is_relativised_without_dots(self):
        inside = os.path.join(self.root, "src", "..", "src", "parse.py")
        self.assertEqual(self.scope.relative(inside), "src/parse.py")

    def test_no_answer_is_ever_absolute_or_contains_a_dot_dot_segment(self):
        candidates = [
            os.path.join(self.root, "a.py"),
            os.path.join(self.root, "deep", "deeper", "b.py"),
            os.path.join(self.root, "..", "c.py"),
            os.__file__,
            "<string>",
            "",
        ]
        for candidate in candidates:
            answer = self.scope.relative(candidate)
            if answer is None:
                continue
            self.assertFalse(answer.startswith("/"), answer)
            self.assertFalse(answer.startswith("\\"), answer)
            self.assertNotIn("..", answer.split("/"))
            self.assertNotIn("\\", answer)

    def test_a_control_character_in_a_filename_is_dropped(self):
        self.assertIsNone(self.scope.relative(os.path.join(self.root, "a\nb.py")))

    def test_a_path_longer_than_the_readers_string_bound_is_dropped(self):
        segment = "d" * 200
        deep = os.path.join(self.root, segment, segment, segment, "x.py")
        self.assertGreater(len(deep) - len(self.root), MAX_STRING_BYTES)
        self.assertIsNone(self.scope.relative(deep))

    def test_the_empty_and_non_string_filenames_are_dropped(self):
        self.assertIsNone(self.scope.relative(""))
        self.assertIsNone(self.scope.relative(None))

    def test_the_answer_is_memoised(self):
        inside = os.path.join(self.root, "src", "parse.py")
        first = self.scope.relative(inside)
        second = self.scope.relative(inside)
        self.assertEqual(first, second)
        # Counted once per distinct filename, so a hot loop cannot inflate the report.
        outside = os.path.join(os.path.dirname(self.root), "e.py")
        self.scope.relative(outside)
        self.scope.relative(outside)
        self.assertEqual(self.scope.outside_root, 1)

    def test_a_symlinked_root_and_a_real_path_agree(self):
        real = os.path.join(self.root, "real")
        os.makedirs(real)
        link = os.path.join(self.root, "link")
        try:
            os.symlink(real, link)
        except (OSError, NotImplementedError):
            self.skipTest("this platform does not allow symlinks")
        scope = PathScope(real)
        self.assertEqual(scope.relative(os.path.join(link, "x.py")), "x.py")


if __name__ == "__main__":
    unittest.main()
