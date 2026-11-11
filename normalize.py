def normalize(text: str) -> str:
    """Trim leading and trailing whitespace and collapse internal whitespace."""
    return " ".join(text.split())
