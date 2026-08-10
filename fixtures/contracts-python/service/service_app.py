"""The service being scanned. Its manifest is the subject; this module is here so the repository
has something to index, which is what makes the neighbour check `available` rather than
`partially_indexed`."""


def handle(request: str) -> str:
    return request.strip()
