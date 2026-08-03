"""Parsing — the middle layer, and the caller a depth-2 edge must be attributed to."""

from src.lex import classify, tokenize

DEFAULTS = tokenize("a b")


def parse(text):
    """Tokenise, then classify each token."""
    tokens = tokenize(text)
    return [classify(token) for token in tokens]


def reload_lex():
    """A lazy import, so a resolved caller can reach a module-body frame."""
    import src.lex

    return src.lex


class Parser:
    """One method, so a method endpoint is exercised as well as a function."""

    def parse_all(self, lines):
        """Parse every line."""
        return [parse(line) for line in lines]
