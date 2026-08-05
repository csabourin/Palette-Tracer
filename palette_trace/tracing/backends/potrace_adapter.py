"""
Potrace tracing backend adapter (supports potracer Python binding and potrace CLI).
"""

import numpy as np
from palette_trace.tracing.protocol import (
    TraceBackend,
    BackendCapabilities,
    TraceRequest,
    TraceResult,
)
from palette_trace.tracing.normalization import normalize_svg_path_data

class PotraceAdapter(TraceBackend):
    """Adapter for Potrace vectorization engine."""

    def __init__(self):
        self._has_module = False
        try:
            import potrace
            self._has_module = True
        except ImportError:
            pass

    def capabilities(self) -> BackendCapabilities:
        return BackendCapabilities(
            backend_id="potrace",
            version="1.16",
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
            raise RuntimeError("Potrace module is not available.")

        import potrace

        mask_arr = np.frombuffer(request.packed_binary_mask, dtype=np.uint8).reshape(
            (request.height, request.width)
        )

        # Potrace bitmap expects boolean or uint32 matrix
        bmp = potrace.Bitmap(mask_arr > 0)

        # Map profile options
        profile = request.profile or {}
        vec_cfg = profile.get("vector", {})

        turdsized = int(vec_cfg.get("minimumPathAreaPx2", 2))
        alphamax = max(0.0, min(1.3333, vec_cfg.get("cornerSensitivity", 0.65) * 1.3333))
        opttoler = max(0.0, min(1.0, vec_cfg.get("optimization", 0.2)))

        path = bmp.trace(
            turdsize=turdsized,
            alphamax=alphamax,
            opttolerance=opttoler,
        )

        svg_paths = []
        for curve in path.curves:
            d_parts = []
            start = curve.start_point
            d_parts.append(f"M {start.x:.3f} {start.y:.3f}")

            for segment in curve.segments:
                if segment.is_corner:
                    c = segment.c
                    end = segment.end_point
                    d_parts.append(f"L {c.x:.3f} {c.y:.3f} L {end.x:.3f} {end.y:.3f}")
                else:
                    c1 = segment.c1
                    c2 = segment.c2
                    end = segment.end_point
                    d_parts.append(
                        f"C {c1.x:.3f} {c1.y:.3f}, {c2.x:.3f} {c2.y:.3f}, {end.x:.3f} {end.y:.3f}"
                    )

            d_parts.append("Z")
            svg_paths.append(normalize_svg_path_data(" ".join(d_parts)))

        return TraceResult(
            svg_path_data=tuple(svg_paths),
            fill_rule="evenodd",
            warnings=(),
            statistics={"curve_count": len(svg_paths)},
        )
