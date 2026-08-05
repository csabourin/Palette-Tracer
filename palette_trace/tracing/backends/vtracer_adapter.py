"""
VTracer backend adapter.
"""

import re
import numpy as np
from palette_trace.tracing.protocol import (
    TraceBackend,
    BackendCapabilities,
    TraceRequest,
    TraceResult,
)
from palette_trace.tracing.normalization import normalize_svg_path_data

class VTracerAdapter(TraceBackend):
    """Adapter for VTracer engine."""

    def __init__(self):
        self._has_module = False
        try:
            import vtracer
            self._has_module = True
        except ImportError:
            pass

    def capabilities(self) -> BackendCapabilities:
        return BackendCapabilities(
            backend_id="vtracer",
            version="0.6.15",
            supports_binary_masks=True,
            supports_holes=True,
            supports_cancellation=False,
            deterministic=True,
            supported_canonical_settings=frozenset([
                "cornerSensitivity",
                "curveSmoothing",
                "optimization",
            ]),
        )

    def is_available(self) -> bool:
        return self._has_module

    def trace_mask(
        self,
        request: TraceRequest,
        cancellation_token: object = None,
    ) -> TraceResult:
        if not self._has_module:
            raise RuntimeError("VTracer module is not available.")

        import vtracer
        import tempfile
        import os
        from PIL import Image

        mask_arr = np.frombuffer(request.packed_binary_mask, dtype=np.uint8).reshape(
            (request.height, request.width)
        )

        # Build 4-channel RGBA image
        rgba = np.zeros((request.height, request.width, 4), dtype=np.uint8)
        rgba[mask_arr > 0] = [0, 0, 0, 255]
        rgba[mask_arr == 0] = [255, 255, 255, 0]

        img = Image.fromarray(rgba, mode="RGBA")

        with tempfile.NamedTemporaryFile(suffix=".png", delete=False) as f_in:
            in_path = f_in.name
            img.save(in_path)

        with tempfile.NamedTemporaryFile(suffix=".svg", delete=False) as f_out:
            out_path = f_out.name

        try:
            vtracer.convert_image_to_svg_py(in_path, out_path)
            with open(out_path, "r", encoding="utf-8") as f_svg:
                svg_str = f_svg.read()
        finally:
            if os.path.exists(in_path):
                os.unlink(in_path)
            if os.path.exists(out_path):
                os.unlink(out_path)

        # Extract d="..." path attributes from returned SVG XML string
        paths = re.findall(r'd="([^"]+)"', svg_str)
        normalized_paths = [normalize_svg_path_data(p) for p in paths if p.strip()]

        return TraceResult(
            svg_path_data=tuple(normalized_paths),
            fill_rule="nonzero",
            warnings=(),
            statistics={"path_count": len(normalized_paths)},
        )
