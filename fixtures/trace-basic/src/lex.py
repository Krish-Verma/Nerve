"""Tokenising — the innermost layer of the fixture's call stack."""


def tokenize(text):
    """Split on spaces, dropping empties."""
    return [piece for piece in text.split(" ") if piece]


def classify(token):
    """Name a token's kind."""
    if token.isdigit():
        return "number"
    return "word"


def unobserved(text):
    """No artifact in this fixture reaches this, so it must acquire no edge at all."""
    return tokenize(text)
