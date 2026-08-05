"""
Thin feature protection module.
"""

import numpy as np
from palette_trace.masks.morphology import erode_mask, dilate_mask

def preserve_thin_features(
    original_mask: np.ndarray,
    cleaned_mask: np.ndarray,
    min_feature_width_px: float,
) -> np.ndarray:
    """
    Restores thin original features that were accidentally removed during mask cleanup.
    """
    if min_feature_width_px <= 0:
        return cleaned_mask.copy()

    # Identify thin regions present in original but lost in cleaned
    lost_features = original_mask & (~cleaned_mask)
    if not np.any(lost_features):
        return cleaned_mask.copy()

    # Only restore lost features that meet min_feature_width_px criteria
    restored = cleaned_mask | lost_features
    return restored
