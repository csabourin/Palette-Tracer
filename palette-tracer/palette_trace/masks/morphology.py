"""
Morphological operations (dilation, erosion, smoothing, preview scaling).
"""

import numpy as np

def dilate_mask(binary_mask: np.ndarray, radius_px: float) -> np.ndarray:
    """Dilates a boolean mask by a given pixel radius."""
    if radius_px <= 0:
        return binary_mask.copy()

    r = int(np.ceil(radius_px))
    height, width = binary_mask.shape
    output = binary_mask.copy()

    # Circular kernel
    y_coords, x_coords = np.ogrid[-r:r+1, -r:r+1]
    kernel = (x_coords**2 + y_coords**2) <= (radius_px**2)

    fore_y, fore_x = np.where(binary_mask)

    for cy, cx in zip(fore_y, fore_x):
        min_y = max(0, cy - r)
        max_y = min(height, cy + r + 1)
        min_x = max(0, cx - r)
        max_x = min(width, cx + r + 1)

        ky_min = min_y - (cy - r)
        ky_max = ky_min + (max_y - min_y)
        kx_min = min_x - (cx - r)
        kx_max = kx_min + (max_x - min_x)

        sub_kernel = kernel[ky_min:ky_max, kx_min:kx_max]
        output[min_y:max_y, min_x:max_x] |= sub_kernel

    return output

def erode_mask(binary_mask: np.ndarray, radius_px: float) -> np.ndarray:
    """Erodes a boolean mask by a given pixel radius."""
    if radius_px <= 0:
        return binary_mask.copy()
    inverted = ~binary_mask
    dilated_inv = dilate_mask(inverted, radius_px)
    return ~dilated_inv

def apply_mask_offset(binary_mask: np.ndarray, offset_px: float) -> np.ndarray:
    """Applies mask offset (positive = dilate, negative = erode)."""
    if offset_px > 0:
        return dilate_mask(binary_mask, offset_px)
    elif offset_px < 0:
        return erode_mask(binary_mask, abs(offset_px))
    return binary_mask.copy()
