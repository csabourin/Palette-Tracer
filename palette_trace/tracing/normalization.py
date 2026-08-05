"""
SVG Path data normalization and SVG sanitization.
"""

import re

def normalize_svg_path_data(path_d: str) -> str:
    """Sanitizes and normalizes raw SVG path string."""
    if not path_d:
        return ""

    # Remove unexpected HTML tags or script injection attempts
    cleaned = re.sub(r"<[^>]*>", "", path_d)
    # Ensure whitespace around command letters
    cleaned = re.sub(r"([a-zA-Z])", r" \1 ", cleaned)
    cleaned = re.sub(r"\s+", " ", cleaned).strip()
    return cleaned

def sanitize_svg_fragment(svg_string: str) -> str:
    """Strips executable elements (scripts, event handlers) from backend SVG outputs."""
    if not svg_string:
        return ""
    # Strip <script...>...</script>
    cleaned = re.sub(r"<script[^>]*>.*?</script>", "", svg_string, flags=re.DOTALL | re.IGNORECASE)
    # Strip event attributes on*="..."
    cleaned = re.sub(r"\s+on[a-z]+\s*=\s*([\"']).*?\1", "", cleaned, flags=re.IGNORECASE)
    return cleaned
