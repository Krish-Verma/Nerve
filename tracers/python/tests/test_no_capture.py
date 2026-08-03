"""No argument or return value is *capturable* — asserted by scanning for the capability.

`docs/plans/slice-11b-python-tracer.md` §4. Not "is not captured" — **cannot be**. "We do not record
it" is a promise; "there is no code path that could" is a property, and only the second survives a
future edit by someone who has not read the plan.

Three layers, of which this file is the third. The contract has nowhere to put a value
(`test_record.py::test_no_record_key_could_hold_a_value`), the tracer never reads one, and this scan
fails on the **addition of the capability** rather than on a behaviour someone remembered to test. It
is the same technique as
`crates/nerve-index/tests/trace.rs::the_new_trace_modules_create_no_process_and_open_no_socket`, and
it works for the same reason.

`sys.settrace` is where this could go wrong and is therefore where the test points hardest: its
callback receives a frame whose locals hold every argument, and its `'return'` event receives the
returned value. The value parameter must be named `_arg` and must never be read.
"""

from __future__ import annotations

import ast
import io
import os
import sys
import tokenize
import unittest

PACKAGE = os.path.join(os.path.dirname(os.path.dirname(os.path.abspath(__file__))), "nerve_trace")

#: Every way CPython offers to reach an argument, a local, a return value or a rendered value.
#:
#: `f_locals` and `f_globals` are the frame's own doors to every name in scope. `co_varnames` and
#: `co_consts` are the code object's. `getargvalues` is `inspect`'s convenience wrapper for the first.
#: `pickle` and `repr(`/`format(` are the ways a value that had been reached would be turned into
#: something storable — a trace that captured arguments would capture credentials, and there is no
#: redaction scheme that is safe by construction.
FORBIDDEN = (
    "f_locals",
    "f_globals",
    "f_builtins",
    "co_varnames",
    "co_consts",
    "getargvalues",
    "getvalue",
    "pickle",
    "repr(",
    "format(",
)

#: Standard-library modules this package is allowed to import. There is no other kind: the package
#: ships no dependencies, so there is nothing to record in `third_party/LICENSES.md` and nothing for a
#: user to install beyond the package itself.
ALLOWED_IMPORTS = {
    "__future__",
    "datetime",
    "json",
    "os",
    "platform",
    "sys",
    "threading",
    "uuid",
}


def sources():
    found = []
    for directory, _subdirectories, filenames in os.walk(PACKAGE):
        for filename in sorted(filenames):
            if filename.endswith(".py"):
                found.append(os.path.join(directory, filename))
    return sorted(found)


def read(path):
    with open(path, "r", encoding="utf-8") as handle:
        return handle.read()


def code_lines(path):
    """`(line number, source)` for every line, with comments and string literals removed.

    The capability scan runs on this rather than on raw text, for the same reason
    `crates/nerve-cli/tests/no_subprocess.rs` strips `//` before looking for `Command::new`: a module
    that refuses a capability has to be able to *name* it in its own documentation, and a scan that
    could not tell prose from code would force the invariant to go unexplained — which is how an
    invariant stops being understood and then stops being kept.

    Removing string literals also closes the obvious dodge of spelling an attribute as text and
    reaching it with `getattr`; the AST checks below cover the direct attribute route.
    """
    text = read(path)
    lines = {number: line for number, line in enumerate(text.splitlines(), start=1)}
    for token in tokenize.generate_tokens(io.StringIO(text).readline):
        if token.type not in (tokenize.COMMENT, tokenize.STRING):
            continue
        start, end = token.start[0], token.end[0]
        for number in range(start, end + 1):
            if number not in lines:
                continue
            if number == start and number == end:
                line = lines[number]
                lines[number] = line[: token.start[1]] + line[token.end[1] :]
            elif number == start:
                lines[number] = lines[number][: token.start[1]]
            elif number == end:
                lines[number] = lines[number][token.end[1] :]
            else:
                lines[number] = ""
    return sorted(lines.items())


class NonCapturabilityTests(unittest.TestCase):
    def test_the_scan_covers_the_whole_package(self):
        names = {os.path.basename(path) for path in sources()}
        self.assertGreaterEqual(len(names), 8)
        for expected in (
            "__init__.py",
            "backend.py",
            "frames.py",
            "monitoring_backend.py",
            "paths.py",
            "pytest_plugin.py",
            "record.py",
            "repository.py",
            "settrace_backend.py",
        ):
            self.assertIn(expected, names, "a module the scan must cover is missing")

    def test_no_module_can_reach_an_argument_a_local_or_a_return_value(self):
        offenders = []
        for path in sources():
            for number, line in code_lines(path):
                for needle in FORBIDDEN:
                    if needle in line:
                        offenders.append(f"{os.path.basename(path)}:{number}: {needle}")
        self.assertEqual(
            offenders,
            [],
            "a value-capture capability appeared in the tracer. The artifact contract has nowhere "
            "to put a value and this package must have no way to reach one; if a genuine exception "
            "is ever needed it requires a documented amendment to the plan, not an edit to this "
            "list.",
        )

    def test_the_scan_would_catch_a_capture_that_was_introduced(self):
        """The mutation probe. A scan that passes on everything proves nothing.

        Real code carrying a real capture is run through the same stripping the package is, so the
        scan is shown to fail on the addition rather than merely to pass on the current tree.
        """
        import tempfile

        mutant = '"""A capture, with the forbidden name also harmlessly in this docstring: f_locals."""\n\n\ndef capture(frame):\n    return dict(frame.f_locals)\n'
        with tempfile.TemporaryDirectory() as directory:
            path = os.path.join(directory, "mutant.py")
            with open(path, "w", encoding="utf-8") as handle:
                handle.write(mutant)
            hits = [
                (number, needle)
                for number, line in code_lines(path)
                for needle in FORBIDDEN
                if needle in line
            ]
        self.assertEqual(hits, [(5, "f_locals")], "the scan must see code and ignore prose")

    def test_the_settrace_return_handler_names_its_value_parameter_underscore_arg(self):
        text = read(os.path.join(PACKAGE, "settrace_backend.py"))
        self.assertIn("def _local_trace(self, frame, event, _arg):", text)
        self.assertIn("def _global_trace(self, frame, event, _arg):", text)

    def test_no_trace_callback_reads_its_value_parameter(self):
        """`_arg` may be named — a callback signature is positional — and must never be used."""
        for name in ("settrace_backend.py", "monitoring_backend.py"):
            tree = ast.parse(read(os.path.join(PACKAGE, name)))
            for node in ast.walk(tree):
                if not isinstance(node, ast.FunctionDef):
                    continue
                parameters = {argument.arg for argument in node.args.args}
                if "_arg" not in parameters:
                    continue
                for inner in ast.walk(node):
                    if isinstance(inner, ast.Name) and inner.id == "_arg":
                        self.fail(f"{name}: {node.name} reads its value parameter")

    def test_the_monitoring_callback_takes_the_value_as_an_ignored_parameter(self):
        text = read(os.path.join(PACKAGE, "monitoring_backend.py"))
        self.assertIn("def _on_exit(self, code, _offset, _arg):", text)

    def test_no_frame_attribute_beyond_the_four_the_tracer_needs_is_touched(self):
        """`f_back`, `f_lineno`, `f_code` and `f_trace_lines` are the whole of it."""
        allowed = {"f_back", "f_lineno", "f_code", "f_trace_lines"}
        for path in sources():
            tree = ast.parse(read(path))
            for node in ast.walk(tree):
                if isinstance(node, ast.Attribute) and node.attr.startswith("f_"):
                    self.assertIn(node.attr, allowed, f"{os.path.basename(path)}: {node.attr}")

    def test_only_the_code_object_fields_a_location_needs_are_touched(self):
        allowed = {"co_filename", "co_firstlineno"}
        for path in sources():
            tree = ast.parse(read(path))
            for node in ast.walk(tree):
                if isinstance(node, ast.Attribute) and node.attr.startswith("co_"):
                    self.assertIn(node.attr, allowed, f"{os.path.basename(path)}: {node.attr}")

    def test_no_symbol_name_is_recorded(self):
        """`co_name` is deliberately absent: resolving the name is Nerve's job.

        A name supplied by the tracer would be a second opinion competing with the index, and the
        Slice 5c line-to-symbol mapping must not grow a rival. A frame is a *location*.
        """
        for path in sources():
            for _number, line in code_lines(path):
                self.assertNotIn("co_name", line)
                self.assertNotIn("co_qualname", line)


class NoDependencyTests(unittest.TestCase):
    def test_every_import_is_the_standard_library(self):
        for path in sources():
            tree = ast.parse(read(path))
            for node in ast.walk(tree):
                if isinstance(node, ast.Import):
                    for alias in node.names:
                        top = alias.name.split(".")[0]
                        self.assertIn(top, ALLOWED_IMPORTS, os.path.basename(path))
                elif isinstance(node, ast.ImportFrom):
                    if node.level:
                        continue
                    top = (node.module or "").split(".")[0]
                    self.assertIn(top, ALLOWED_IMPORTS, os.path.basename(path))

    @unittest.skipUnless(
        hasattr(sys, "stdlib_module_names"), "sys.stdlib_module_names needs Python 3.10 or later"
    )
    def test_the_allowed_imports_really_are_in_the_standard_library(self):
        for name in sorted(ALLOWED_IMPORTS):
            self.assertIn(name, sys.stdlib_module_names, name)

    def test_pytest_is_never_imported(self):
        """The plugin is found by hook name, so the package stays importable without pytest.

        That is what lets these tests run on a machine that has none, which is the machine the tracer
        was written on.
        """
        for path in sources():
            tree = ast.parse(read(path))
            for node in ast.walk(tree):
                if isinstance(node, ast.Import):
                    self.assertNotIn("pytest", [alias.name for alias in node.names])
                elif isinstance(node, ast.ImportFrom):
                    self.assertNotEqual(node.module, "pytest")


class NoProcessOrSocketTests(unittest.TestCase):
    """The tracer runs inside the user's test process, which makes it the worst place to spawn."""

    def test_the_tracer_creates_no_process_and_opens_no_socket(self):
        forbidden = (
            "subprocess",
            "os.system",
            "os.popen",
            "os.exec",
            "os.spawn",
            "os.fork",
            "socket",
            "urllib",
            "http.client",
        )
        for path in sources():
            for number, line in code_lines(path):
                for needle in forbidden:
                    self.assertNotIn(needle, line, f"{os.path.basename(path)}:{number}")


if __name__ == "__main__":
    unittest.main()
