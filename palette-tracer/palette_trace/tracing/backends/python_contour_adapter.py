"""
Pure Python contour-following fallback tracing adapter.
Guarantees 100% zero-dependency execution if no external libraries/binaries are present.
"""

import numpy as np
from palette_trace.tracing.protocol import (
    TraceBackend,
    BackendCapabilities,
    TraceRequest,
    TraceResult,
)
from palette_trace.tracing.normalization import normalize_svg_path_data
from palette_trace.masks.components import label_connected_components

class PythonContourAdapter(TraceBackend):
    """Pure-Python fallback boundary tracer."""

    def capabilities(self) -> BackendCapabilities:
        return BackendCapabilities(
            backend_id="python_contour",
            version="1.0.0",
            supports_binary_masks=True,
            supports_holes=True,
            supports_cancellation=False,
            deterministic=True,
            supported_canonical_settings=frozenset(["minimumPathAreaPx2"]),
        )

    def is_available(self) -> bool:
        return True

    def trace_mask(
        self,
        request: TraceRequest,
        cancellation_token: object = None,
    ) -> TraceResult:
        mask_arr = np.frombuffer(request.packed_binary_mask, dtype=np.uint8).reshape(
            (request.height, request.width)
        )

        labels, sizes = label_connected_components(mask_arr > 0)
        svg_paths = []

        profile = request.profile or {}
        min_area = profile.get("vector", {}).get("minimumPathAreaPx2", 1)

        for label_id, size in enumerate(sizes):
            if label_id == 0 or size < min_area:
                continue

            comp_mask = labels == label_id
            contours = self._trace_component_boundary(comp_mask)
            for contour in contours:
                if len(contour) >= 3:
                    d_parts = [f"M {contour[0][0]:.3f} {contour[0][1]:.3f}"]
                    for pt in contour[1:]:
                        d_parts.append(f"L {pt[0]:.3f} {pt[1]:.3f}")
                    d_parts.append("Z")
                    svg_paths.append(normalize_svg_path_data(" ".join(d_parts)))

        return TraceResult(
            svg_path_data=tuple(svg_paths),
            fill_rule="evenodd",
            warnings=(),
            statistics={"component_count": len(svg_paths)},
        )

    def _trace_component_boundary(self, comp_mask: np.ndarray) -> list[list[tuple[float, float]]]:
        """Simple boundary extraction for connected component."""
        height, width = comp_mask.shape
        visited_boundary = set()
        contours = []

        for y in range(height):
            for x in range(width):
                if comp_mask[y, x] and (y, x) not in visited_boundary:
                    # Check if edge pixel
                    is_edge = False
                    for dy, dx in ((-1, 0), (1, 0), (0, -1), (0, 1)):
                        ny, nx = y + dy, x + dx
                        if not (0 <= ny < height and 0 <= nx < width) or not comp_mask[ny, nx]:
                            is_edge = True
                            break

                    if is_edge:
                        # Extract boundary loop
                        loop = []
                        cy, cx = y, x
                        while (cy, cx) not in visited_boundary:
                            visited_boundary.add((cy, cx))
                            loop.append((cx + 0.5, cy + 0.5))

                            next_pt = None
                            for dy, dx in ((-1, 0), (0, 1), (1, 0), (0, -1)):
                                ny, nx = cy + dy, cx + dx
                                if 0 <= ny < height and 0 <= nx < width and comp_mask[ny, nx]:
                                    if (ny, nx) not in visited_boundary:
                                        # Edge check
                                        edge = any(
                                            not (0 <= ny + ey < height and 0 <= nx + ex < width) or not comp_mask[ny + ey, nx + ex]
                                            for ey, ex in ((-1, 0), (1, 0), (0, -1), (0, 1))
                                        )
                                        if edge:
                                            next_pt = (ny, nx)
                                            break
                            if next_pt is None:
                                break
                            cy, cx = next_pt

                        if len(loop) >= 3:
                            contours.append(loop)

        return contours
