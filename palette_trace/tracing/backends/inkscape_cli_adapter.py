"""
Inkscape CLI action backend adapter (`object-trace`).
"""

import shutil
from palette_trace.tracing.protocol import (
    TraceBackend,
    BackendCapabilities,
    TraceRequest,
    TraceResult,
)

class InkscapeCliAdapter(TraceBackend):
    """Adapter invoking Inkscape CLI actions."""

    def __init__(self):
        self._exe = shutil.which("inkscape") or r"C:\Program Files\Inkscape\bin\inkscape.exe"

    def capabilities(self) -> BackendCapabilities:
        return BackendCapabilities(
            backend_id="inkscape_cli",
            version="1.4+",
            supports_binary_masks=True,
            supports_holes=True,
            supports_cancellation=False,
            deterministic=True,
            supported_canonical_settings=frozenset([]),
        )

    def is_available(self) -> bool:
        import os
        return os.path.exists(self._exe)

    def trace_mask(
        self,
        request: TraceRequest,
        cancellation_token: object = None,
    ) -> TraceResult:
        # Inkscape CLI single-scan batch adapter placeholder
        return TraceResult(
            svg_path_data=(),
            fill_rule="evenodd",
            warnings=("Inkscape CLI object-trace action is limited.",),
            statistics={},
        )
