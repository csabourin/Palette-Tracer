"""
Diagnostic reporting utilities for Palette Trace (SPEC §30).
"""

from palette_trace.capabilities import detect_environment
from palette_trace.settings import EXTENSION_VERSION


def generate_diagnostic_report(error=None) -> dict:
    """Builds the environment-and-error report offered when something fails."""
    return {
        "extension_version": EXTENSION_VERSION,
        "environment": detect_environment(),
        "error_details": str(error) if error else None,
    }
