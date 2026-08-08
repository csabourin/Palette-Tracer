"""
SVG Sanitizer module for enforcing security constraints on imported SVG content.
"""

from palette_trace.tracing.normalization import sanitize_svg_fragment, normalize_svg_path_data

def sanitize_path_element(d_attribute: str) -> str:
    """Validates SVG path data."""
    return normalize_svg_path_data(d_attribute)
