"""
Canonical binary-mask fixtures for backend conformance evaluation (SPEC §23.6, §33.2).

Every fixture returns a ``numpy`` array of shape ``(height, width)`` and dtype
``uint8`` containing only 0 and 1. Use :func:`as_request` to wrap one in a
:class:`~palette_trace.tracing.protocol.TraceRequest`.

Fixtures are deliberately small and analytically describable so that conformance
thresholds can be reasoned about rather than tuned against a specific engine.
"""

import numpy as np

from palette_trace.tracing.protocol import TraceRequest

DEFAULT_SIZE = 64
LARGE_SIZE = 512


def solid_rectangle(w: int = DEFAULT_SIZE, h: int = DEFAULT_SIZE) -> np.ndarray:
    """Axis-aligned filled rectangle. Four corners, no holes."""
    mask = np.zeros((h, w), dtype=np.uint8)
    mask[h // 5: h - h // 5, w // 5: w - w // 5] = 1
    return mask


def donut(w: int = DEFAULT_SIZE, h: int = DEFAULT_SIZE) -> np.ndarray:
    """Square ring. Requires two boundaries — one outer, one hole."""
    mask = np.zeros((h, w), dtype=np.uint8)
    mask[h // 8: h - h // 8, w // 8: w - w // 8] = 1
    mask[h // 3: h - h // 3, w // 3: w - w // 3] = 0
    return mask


def plus(w: int = DEFAULT_SIZE, h: int = DEFAULT_SIZE) -> np.ndarray:
    """Plus/cross shape. Twelve right-angle corners, four of them concave."""
    mask = np.zeros((h, w), dtype=np.uint8)
    mask[3 * h // 8: 5 * h // 8, w // 8: w - w // 8] = 1
    mask[h // 8: h - h // 8, 3 * w // 8: 5 * w // 8] = 1
    return mask


def circle(w: int = DEFAULT_SIZE, h: int = DEFAULT_SIZE) -> np.ndarray:
    """Filled disc. Purely curved boundary with no true corners."""
    yy, xx = np.mgrid[0:h, 0:w]
    cy, cx, r = h / 2.0, w / 2.0, min(w, h) * 0.375
    return (((yy - cy) ** 2 + (xx - cx) ** 2) <= r * r).astype(np.uint8)


def single_pixel(w: int = DEFAULT_SIZE, h: int = DEFAULT_SIZE) -> np.ndarray:
    """One isolated pixel. Should not become vector geometry."""
    mask = np.zeros((h, w), dtype=np.uint8)
    mask[h // 2, w // 2] = 1
    return mask


def sparse_noise(w: int = DEFAULT_SIZE, h: int = DEFAULT_SIZE) -> np.ndarray:
    """Four scattered isolated pixels."""
    mask = np.zeros((h, w), dtype=np.uint8)
    for fy, fx in ((0.15, 0.15), (0.80, 0.80), (0.50, 0.15), (0.15, 0.80)):
        mask[int(h * fy), int(w * fx)] = 1
    return mask


def diagonal_line(w: int = DEFAULT_SIZE, h: int = DEFAULT_SIZE) -> np.ndarray:
    """One-pixel-wide diagonal stroke. Thin-feature retention (SPEC §17.5)."""
    mask = np.zeros((h, w), dtype=np.uint8)
    for i in range(min(w, h)):
        mask[i, i] = 1
    return mask


def large_donut(w: int = LARGE_SIZE, h: int = LARGE_SIZE) -> np.ndarray:
    """Large annulus. Exercises runtime and node-count behaviour at scale."""
    yy, xx = np.mgrid[0:h, 0:w]
    cy, cx = h / 2.0, w / 2.0
    d2 = (yy - cy) ** 2 + (xx - cx) ** 2
    mask = (d2 <= (min(w, h) * 0.39) ** 2).astype(np.uint8)
    mask[d2 <= (min(w, h) * 0.156) ** 2] = 0
    return mask


def as_request(mask: np.ndarray, profile: dict | None = None) -> TraceRequest:
    """Wraps a fixture mask in a TraceRequest."""
    h, w = mask.shape
    return TraceRequest(
        width=w,
        height=h,
        packed_binary_mask=mask.astype(np.uint8).tobytes(),
        profile=profile or {},
    )
