"""
System capability detection for the Python runtime, Pillow, NumPy and the
tracing backends. Feeds the diagnostic report (§30).
"""

import importlib
import importlib.util
import platform
import sys


def _version_of(module_name: str) -> str | None:
    """The installed module's version, or None when it is not importable."""
    if importlib.util.find_spec(module_name) is None:
        return None
    return getattr(importlib.import_module(module_name), "__version__", "unknown")


def _is_installed(module_name: str) -> bool:
    """Whether the module can be imported, without importing it."""
    return importlib.util.find_spec(module_name) is not None


def detect_environment() -> dict:
    """Reports the runtime and the optional libraries this install can reach."""
    pil_version = _version_of("PIL")
    numpy_version = _version_of("numpy")

    return {
        "python_version": sys.version,
        "platform": platform.platform(),
        "has_pil": pil_version is not None,
        "pil_version": pil_version,
        "has_numpy": numpy_version is not None,
        "numpy_version": numpy_version,
        # Backends are probed rather than imported: importing a tracing engine
        # to ask whether it exists costs the load on every diagnostic run.
        "has_vtracer": _is_installed("vtracer"),
        "has_potrace": _is_installed("potrace"),
    }
