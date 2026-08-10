"""
SVG path data normalization and scaling.

This is the whole of the backend output boundary (§31): an adapter hands back
path `d` strings and nothing else, so no backend-authored SVG fragment is ever
inserted into a document and there is no markup here left to sanitize.
"""

import re

#: Every path command, and which of its parameters are lengths.
#:
#: For all but the arc, every parameter is a coordinate or a length and scales
#: with the drawing. `A` takes `rx ry x-axis-rotation large-arc sweep x y`: the
#: rotation is an angle and the two flags are booleans, so scaling them would
#: turn a quarter-circle into something else entirely. Relative commands scale
#: the same way as absolute ones — a displacement is a length too.
_ARC_LENGTH_PARAMETERS = (0, 1, 5, 6)
_ARC_PARAMETER_COUNT = 7

_TOKEN = re.compile(r"[A-Za-z]|[-+]?(?:\d*\.\d+|\d+\.?)(?:[eE][-+]?\d+)?")

def normalize_svg_path_data(path_d: str) -> str:
    """Sanitizes and normalizes raw SVG path string."""
    if not path_d:
        return ""

    # Remove unexpected HTML tags or script injection attempts
    cleaned = re.sub(r"<[^>]*>", "", path_d)
    # Ensure whitespace around command letters
    cleaned = re.sub(r"([a-zA-Z])", r" \1 ", cleaned)
    return re.sub(r"\s+", " ", cleaned).strip()

def scale_svg_path_data(path_d: str, factor: float) -> str:
    """
    Rescales every length in a path, leaving arc angles and flags alone.

    A preview traces a reduced copy of the bitmap, so the geometry that comes
    back is in preview pixels. The rest of the program — the preview panel, the
    SVG writer, the Inkscape commit — is written against source pixels and has
    no idea a preview happened, which is the way to keep it: the coordinate
    system is converted once, here, rather than by a transform that every
    consumer would have to remember to apply.

    Numbers are re-emitted at three decimals, matching what the backends
    produce. `factor` of exactly 1.0 returns the path untouched, so a
    full-resolution trace is not reformatted for nothing.
    """
    if not path_d or factor == 1.0:
        return path_d or ""

    parts = []
    command = ""
    parameter_index = 0

    for token in _TOKEN.findall(path_d):
        if token.isalpha():
            command = token
            parameter_index = 0
            parts.append(token)
            continue

        value = float(token)
        if command in ("A", "a"):
            scales = (parameter_index % _ARC_PARAMETER_COUNT) in _ARC_LENGTH_PARAMETERS
        else:
            scales = True
        parameter_index += 1

        if scales:
            value *= factor
        # Trailing zeros are noise, and the backends already round to three
        # decimals, so this changes precision for nobody.
        parts.append(f"{value:.3f}".rstrip("0").rstrip("."))

    return " ".join(parts)
