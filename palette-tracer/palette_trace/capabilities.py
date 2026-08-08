"""
System capability detection for Python runtime, Pillow, NumPy, and Tracing Backends.
"""

import sys
import platform

def detect_environment():
    capabilities = {
        "python_version": sys.version,
        "platform": platform.platform(),
        "has_pil": False,
        "pil_version": None,
        "has_numpy": False,
        "numpy_version": None,
        "has_vtracer": False,
        "has_potrace": False,
    }

    try:
        import PIL
        capabilities["has_pil"] = True
        capabilities["pil_version"] = getattr(PIL, "__version__", "unknown")
    except ImportError:
        pass

    try:
        import numpy
        capabilities["has_numpy"] = True
        capabilities["numpy_version"] = getattr(numpy, "__version__", "unknown")
    except ImportError:
        pass

    try:
        import vtracer
        capabilities["has_vtracer"] = True
    except ImportError:
        pass

    try:
        import potrace
        capabilities["has_potrace"] = True
    except ImportError:
        pass

    return capabilities
