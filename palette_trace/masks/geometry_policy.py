"""
Destination geometry policy implementations (stacked, trapped, exclusive, operations).
"""

import numpy as np
from palette_trace.masks.morphology import dilate_mask

def apply_underlap_to_mask(
    mask: np.ndarray,
    subject_silhouette: np.ndarray,
    underlap_px: float,
    preserve_outer_silhouette: bool = True,
) -> np.ndarray:
    """
    Expands lower scan mask by underlap_px while constraining to outer subject silhouette.
    """
    if underlap_px <= 0:
        return mask.copy()

    dilated = dilate_mask(mask, underlap_px)

    if preserve_outer_silhouette:
        return dilated & subject_silhouette

    return dilated

def apply_trapping_to_mask(
    mask: np.ndarray,
    subject_silhouette: np.ndarray,
    trapping_px: float,
) -> np.ndarray:
    """
    Applies trapping expansion to screen print color layer masks.
    """
    if trapping_px <= 0:
        return mask.copy()

    dilated = dilate_mask(mask, trapping_px)
    return dilated & subject_silhouette
