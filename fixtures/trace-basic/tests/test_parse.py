"""The fixture's tests. Nerve never runs them: every artifact here is hand-written."""

from src.parse import Parser, parse, reload_lex


def test_basic():
    """One depth-1 call from the test body, whose callee calls twice more."""
    assert parse("a 1") == ["word", "number"]


def test_method():
    """A depth-1 call to a method, which reaches `parse` at depth 2."""
    assert Parser().parse_all(["a", "b"]) == [["word"], ["word"]]


def test_lazy_import():
    """Reaches a module-body frame, which no symbol contains."""
    assert reload_lex() is not None


def test_partial():
    """The test the interrupted run stopped inside."""
    assert parse("x") == ["word"]
