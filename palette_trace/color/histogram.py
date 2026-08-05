"""
Color histogram reduction module (6-bit channel quantizing for fast OKLab clustering).
"""

import numpy as np
from palette_trace.color.conversion import srgb_to_oklab, oklab_to_srgb, srgb_to_hex

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
    divisor = 1 << shift

    # Mask valid unclaimed pixels
    valid_coords = np.where(unclaimed_mask)
    if len(valid_coords[0]) == 0:
        return []

    r_vals = srgb_image[:, :, 0][valid_coords]
    g_vals = srgb_image[:, :, 1][valid_coords]
    b_vals = srgb_image[:, :, 2][valid_coords]

    # Convert to integer 0..255
    r_int = (r_vals * 255.0).astype(np.uint32)
    g_int = (g_vals * 255.0).astype(np.uint32)
    b_int = (b_vals * 255.0).astype(np.uint32)

    # Packed 6-bit bins
    r_bin = r_int >> shift
    g_bin = g_int >> shift
    b_bin = b_int >> shift

    packed_bins = (r_bin << (2 * bits_per_channel)) | (g_bin << bits_per_channel) | b_bin

    unique_bins, counts = np.unique(packed_bins, return_counts=True)

    histogram = []
    scale = (1 << bits_per_channel) - 1

    for bin_code, count in zip(unique_bins, counts):
        r_b = (bin_code >> (2 * bits_per_channel)) & scale
        g_b = (bin_code >> bits_per_channel) & scale
        b_b = bin_code & scale

        r_f = r_b / scale
        g_f = g_b / scale
        b_f = b_b / scale

        oklab = srgb_to_oklab(r_f, g_f, b_f)

        packed_srgb = (int(r_f * 255) << 16) | (int(g_f * 255) << 8) | int(b_f * 255)

        histogram.append({
            "oklab": oklab,
            "srgb": (r_f, g_f, b_f),
            "weight": int(count),
            "packed_srgb": packed_srgb,
        })

    return histogram
