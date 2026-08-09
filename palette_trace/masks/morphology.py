"""
Morphological operations (dilation, erosion, smoothing, preview scaling).

The structuring element is a disc of the requested radius, so a dilation grows
a shape evenly in every direction rather than squaring off its corners. The
work itself goes through `scipy.ndimage`: the loop-per-foreground-pixel version
this replaces cost one Python iteration per set pixel, which on a full-frame
backdrop mask meant millions of them per scan.
"""

import numpy as np
from scipy import ndimage


def disc_structure(radius_px: float) -> np.ndarray:
    """Boolean disc of `radius_px`, sized to the smallest odd square that fits."""
    r = int(np.ceil(radius_px))
    y_coords, x_coords = np.ogrid[-r:r + 1, -r:r + 1]
    return (x_coords ** 2 + y_coords ** 2) <= (radius_px ** 2)


def dilate_mask(binary_mask: np.ndarray, radius_px: float) -> np.ndarray:
    """Dilates a boolean mask by a given pixel radius."""
    mask = np.asarray(binary_mask, dtype=bool)
    if radius_px <= 0 or not mask.any():
        return mask.copy()

    return ndimage.binary_dilation(mask, structure=disc_structure(radius_px))


def erode_mask(binary_mask: np.ndarray, radius_px: float) -> np.ndarray:
    """
    Erodes a boolean mask by a given pixel radius.

    Expressed as the complement of a dilation of the complement, which is what
    keeps the image border neutral: a shape running off the edge of the frame is
    not eaten away from a boundary that is a crop, not an outline.
    """
    if radius_px <= 0:
        return np.asarray(binary_mask, dtype=bool).copy()
    inverted = ~np.asarray(binary_mask, dtype=bool)
    return ~dilate_mask(inverted, radius_px)


def apply_mask_offset(binary_mask: np.ndarray, offset_px: float) -> np.ndarray:
    """Applies mask offset (positive = dilate, negative = erode)."""
    if offset_px > 0:
        return dilate_mask(binary_mask, offset_px)
    elif offset_px < 0:
        return erode_mask(binary_mask, abs(offset_px))
    return np.asarray(binary_mask, dtype=bool).copy()
