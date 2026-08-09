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


def _fast_findnext(bitmap: np.ndarray):
    """
    Bottom-most, then left-most set pixel of `bitmap`, or None.

    Identical in result to `potrace.potrace.findnext`, which the pure-Python
    potracer calls once per path it discovers and answers with a full
    `np.nonzero` of the whole bitmap — quadratic in the number of paths, and on
    a noisy photograph the single most expensive thing in a trace. Collapsing
    rows first turns each call into two linear scans of much smaller arrays.
    """
    rows = bitmap.any(axis=1)
    if not rows.any():
        return None
    y = int(np.flatnonzero(rows)[-1])
    return y, int(np.argmax(bitmap[y]))


def _install_fast_findnext() -> None:
    """
    Swaps potracer's path search for the equivalent above.

    Guarded on the symbol still existing and still being the one we mean to
    replace, so a future potracer that has fixed this itself is left alone.
    """
    try:
        from potrace import potrace as potrace_impl
    except ImportError:
        return

    existing = getattr(potrace_impl, "findnext", None)
    if existing is None or getattr(existing, "_palette_trace_patched", False):
        return

    _fast_findnext._palette_trace_patched = True
    potrace_impl.findnext = _fast_findnext


class PotraceAdapter(TraceBackend):
    """Adapter for Potrace vectorization engine."""

    def __init__(self):
        self._has_module = False
        try:
            import potrace  # noqa: F401
            self._has_module = True
            _install_fast_findnext()
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

        # Potrace treats samples BELOW blacklevel (default 0.5) as foreground, so the
        # mask must be inverted: selected pixels become 0, unselected become 1.
        # Passing the mask directly traces the background and yields a full-frame path.
        bmp = potrace.Bitmap(mask_arr == 0)

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
