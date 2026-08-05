"""
Diagnostic reporting utilities for Palette Trace.
"""

from palette_trace.capabilities import detect_environment

def generate_diagnostic_report(error=None):
    env = detect_environment()
    report = {
        "extension_version": "1.0.0",
        "environment": env,
        "error_details": str(error) if error else None,
    }
    return report
