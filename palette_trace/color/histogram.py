"""
Color histogram reduction module (6-bit channel quantizing for fast OKLab clustering).
"""

import numpy as np
from palette_trace.color.conversion import srgb_array_to_oklch, srgb_to_hex


def build_color_histogram(
    srgb_image: np.ndarray,
    unclaimed_mask: np.ndarray,
    bits_per_channel: int = 6,
) -> list[dict]:
    """
    Builds a weighted histogram of unclaimed sRGB pixels.
    Returns list of dicts: [{'oklab': (L,a,b), 'srgb': (r,g,b), 'weight': count, 'packed_srgb': int}]
    """
    shift = 8 - bits_per_channel
    scale = (1 << bits_per_channel) - 1

    mask = np.asarray(unclaimed_mask, dtype=bool)
    if not mask.any():
        return []

    pixels = np.asarray(srgb_image)[mask]
    channels = (pixels * 255.0).astype(np.uint32) >> shift

    packed_bins = (
        (channels[:, 0].astype(np.int64) << (2 * bits_per_channel))
        | (channels[:, 1].astype(np.int64) << bits_per_channel)
        | channels[:, 2].astype(np.int64)
    )

    unique_bins, counts = np.unique(packed_bins, return_counts=True)

    bins = np.stack([
        (unique_bins >> (2 * bits_per_channel)) & scale,
        (unique_bins >> bits_per_channel) & scale,
        unique_bins & scale,
    ], axis=1) / float(scale)

    # One vectorised conversion for the whole histogram: OKLab per bin used to
    # be a Python call per bin, and a 6-bit histogram can hold 262 144 of them.
    oklch = srgb_array_to_oklch(bins)
    hue = np.radians(oklch[:, 2])
    oklab = np.stack(
        [oklch[:, 0], oklch[:, 1] * np.cos(hue), oklch[:, 1] * np.sin(hue)], axis=1
    )

    packed_srgb = (
        (bins[:, 0] * 255).astype(np.int64) << 16
        | (bins[:, 1] * 255).astype(np.int64) << 8
        | (bins[:, 2] * 255).astype(np.int64)
    )

    return [
        {
            "oklab": (float(lab[0]), float(lab[1]), float(lab[2])),
            "srgb": (float(rgb[0]), float(rgb[1]), float(rgb[2])),
            "weight": int(count),
            "packed_srgb": int(packed),
        }
        for lab, rgb, count, packed in zip(oklab, bins, counts, packed_srgb)
    ]


def unique_source_colors(
    srgb_image: np.ndarray,
    mask: np.ndarray,
) -> tuple[np.ndarray, np.ndarray, np.ndarray]:
    """
    Every colour that literally occurs under `mask`, at full 8-bit precision.

    Returns (rgb255, counts, oklab). Unlike :func:`build_color_histogram` this
    does not bin anything: these are the colours the picture actually contains,
    which is what a palette entry has to end up holding (§34.5).
    """
    selected = np.asarray(mask, dtype=bool)
    if not selected.any():
        empty = np.empty((0, 3))
        return empty.astype(np.uint8), np.empty(0, dtype=np.int64), empty

    pixels = np.rint(np.asarray(srgb_image)[selected] * 255.0).astype(np.int64)
    packed = (pixels[:, 0] << 16) | (pixels[:, 1] << 8) | pixels[:, 2]

    unique_packed, counts = np.unique(packed, return_counts=True)
    rgb255 = np.stack([
        (unique_packed >> 16) & 0xFF,
        (unique_packed >> 8) & 0xFF,
        unique_packed & 0xFF,
    ], axis=1).astype(np.uint8)

    oklch = srgb_array_to_oklch(rgb255.astype(np.float64) / 255.0)
    hue = np.radians(oklch[:, 2])
    oklab = np.stack(
        [oklch[:, 0], oklch[:, 1] * np.cos(hue), oklch[:, 1] * np.sin(hue)], axis=1
    )

    return rgb255, counts, oklab


def snap_to_source_colors(
    centres: list[tuple[float, float, float]],
    rgb255: np.ndarray,
    counts: np.ndarray,
    oklab: np.ndarray,
) -> list[str]:
    """
    Replaces each OKLab centroid with the nearest colour the picture contains.

    A K-Means centroid is an average of a cluster, so it is very often a colour
    that appears nowhere in the source — a muddy blend across an edge. Snapping
    each centroid to its nearest real colour keeps the palette's arrangement
    (the clusters are unchanged) while guaranteeing that every swatch is a
    colour someone could point at in the picture.

    Ties are broken towards the more common colour, then towards the lower
    packed sRGB value, so the result depends only on the image (§34.30).
    """
    if not centres or oklab.shape[0] == 0:
        return []

    targets = np.asarray(centres, dtype=np.float64)
    # (k, n) — k is the scan count, at most 64.
    distances = ((targets[:, None, :] - oklab[None, :, :]) ** 2).sum(axis=2)

    packed = (
        rgb255[:, 0].astype(np.int64) << 16
        | rgb255[:, 1].astype(np.int64) << 8
        | rgb255[:, 2].astype(np.int64)
    )

    snapped = []
    for row in distances:
        closest = np.flatnonzero(row <= row.min() + 1e-12)
        if closest.size > 1:
            best = counts[closest]
            closest = closest[best == best.max()]
            closest = closest[np.argmin(packed[closest])]
        else:
            closest = closest[0]
        r, g, b = rgb255[closest]
        snapped.append(srgb_to_hex(r / 255.0, g / 255.0, b / 255.0))

    return snapped
