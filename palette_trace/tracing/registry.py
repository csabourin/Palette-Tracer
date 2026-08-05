"""
Tracing Backend Discovery and Registry Manager.
"""

from typing import Dict, Optional, List
from palette_trace.tracing.protocol import TraceBackend
from palette_trace.tracing.backends.potrace_adapter import PotraceAdapter
from palette_trace.tracing.backends.vtracer_adapter import VTracerAdapter
from palette_trace.tracing.backends.python_contour_adapter import PythonContourAdapter
from palette_trace.tracing.backends.autotrace_adapter import AutoTraceAdapter
from palette_trace.tracing.backends.inkscape_cli_adapter import InkscapeCliAdapter
from palette_trace.errors import BackendNotFoundError

class BackendRegistry:
    """Manages discovery and instantiation of tracing backend adapters."""

    def __init__(self):
        self._backends: dict[str, TraceBackend] = {}
        self._register_default_backends()

    def _register_default_backends(self):
        # Register available adapters
        potrace = PotraceAdapter()
        if potrace.is_available():
            self._backends["potrace"] = potrace

        vtracer = VTracerAdapter()
        if vtracer.is_available():
            self._backends["vtracer"] = vtracer

        py_contour = PythonContourAdapter()
        if py_contour.is_available():
            self._backends["python_contour"] = py_contour

        autotrace = AutoTraceAdapter()
        if autotrace.is_available():
            self._backends["autotrace"] = autotrace

        ink_cli = InkscapeCliAdapter()
        if ink_cli.is_available():
            self._backends["inkscape_cli"] = ink_cli

    def get_backend(self, preferred_id: str = "auto") -> TraceBackend:
        """Returns requested or best available backend adapter."""
        if preferred_id != "auto" and preferred_id in self._backends:
            return self._backends[preferred_id]

        # Priority selection: potrace -> vtracer -> python_contour
        for candidate in ["potrace", "vtracer", "python_contour"]:
            if candidate in self._backends:
                return self._backends[candidate]

        if self._backends:
            return next(iter(self._backends.values()))

        raise BackendNotFoundError("No compatible tracing backend available.")

    def list_available_backends(self) -> list[dict]:
        """Returns metadata list of available backends."""
        info = []
        for bid, b in self._backends.items():
            caps = b.capabilities()
            info.append({
                "id": caps.backend_id,
                "version": caps.version,
                "supportsHoles": caps.supports_holes,
                "deterministic": caps.deterministic,
            })
        return info
