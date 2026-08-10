"""
Tracing backend discovery and registry (SPEC §23).
"""

from palette_trace.errors import BackendNotFoundError
from palette_trace.tracing.backends.potrace_adapter import PotraceAdapter
from palette_trace.tracing.backends.python_contour_adapter import PythonContourAdapter
from palette_trace.tracing.backends.vtracer_adapter import VTracerAdapter
from palette_trace.tracing.protocol import TraceBackend

#: Reference backend selected by the Phase 0 engine spike (§23.6).
#: See docs/decisions/ADR-0001-reference-tracing-backend.md for the measured
#: conformance evidence behind this choice. Changing it requires re-running
#: `python -m palette_trace.tracing.conformance.runner` and updating that record.
REFERENCE_BACKEND_ID = "potrace"

#: Automatic-selection order (§23.3). Portable Python bindings come before
#: compiled-wheel engines, which come before the correctness-only fallback.
BACKEND_PRIORITY = ("potrace", "vtracer", "python_contour")

#: Every adapter the registry knows how to construct, in no particular order —
#: `BACKEND_PRIORITY` decides what "auto" resolves to. An adapter belongs here
#: only once it can actually trace: one that reports itself available and then
#: returns no geometry is worse than one that is absent, because the pipeline
#: has no way to tell an empty scan from an empty mask.
_ADAPTERS = (PotraceAdapter, VTracerAdapter, PythonContourAdapter)


class BackendRegistry:
    """Manages discovery and instantiation of tracing backend adapters."""

    def __init__(self):
        self._backends: dict[str, TraceBackend] = {}
        for adapter_class in _ADAPTERS:
            adapter = adapter_class()
            if adapter.is_available():
                self._backends[adapter.capabilities().backend_id] = adapter

    def get_backend(self, preferred_id: str = "auto") -> TraceBackend:
        """Returns requested or best available backend adapter."""
        if preferred_id != "auto" and preferred_id in self._backends:
            return self._backends[preferred_id]

        for candidate in BACKEND_PRIORITY:
            if candidate in self._backends:
                return self._backends[candidate]

        if self._backends:
            return next(iter(self._backends.values()))

        raise BackendNotFoundError("No compatible tracing backend available.")

    def list_available_backends(self) -> list[dict]:
        """Returns metadata list of available backends."""
        info = []
        for backend in self._backends.values():
            caps = backend.capabilities()
            info.append({
                "id": caps.backend_id,
                "version": caps.version,
                "supportsHoles": caps.supports_holes,
                "deterministic": caps.deterministic,
            })
        return info
