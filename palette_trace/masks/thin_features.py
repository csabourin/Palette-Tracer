"""
Thin-feature protection (SPEC §17.5).

Speckle removal and hole filling cannot tell a stray pixel from a hairline: both
are small. This module restores what cleanup removed, but *only* where the lost
pixels belong to a genuinely thin structure — a whisker, a hairline serif, a
one-pixel outline. Isolated noise, which is what cleanup is for, stays removed.
"""

import numpy as np

from palette_trace.masks.morphology import dilate_mask, erode_mask


def thin_structure_mask(mask: np.ndarray, min_feature_width_px: float) -> np.ndarray:
    """
    Returns the parts of `mask` narrower than `min_feature_width_px`.

    Computed as the residue of a morphological opening: anything that does not
    survive an erode-then-dilate at half the feature width is, by definition,
    narrower than that width.
    """
    if min_feature_width_px <= 0:
        return np.zeros_like(mask, dtype=bool)

    radius = min_feature_width_px / 2.0
    opened = dilate_mask(erode_mask(mask.astype(bool), radius), radius)
    return mask.astype(bool) & ~opened


def preserve_thin_features(
    original_mask: np.ndarray,
    cleaned_mask: np.ndarray,
    min_feature_width_px: float,
) -> np.ndarray:
    """
    Restores thin features that cleanup removed from `original_mask`.

    Returns `cleaned_mask` unchanged when the feature width is zero or when
    nothing thin was lost. Only pixels that are both lost *and* part of a thin
    structure come back, so this cannot undo legitimate speckle removal.
    """
    original = original_mask.astype(bool)
    cleaned = cleaned_mask.astype(bool)

    if min_feature_width_px <= 0:
        return cleaned.copy()

    lost = original & ~cleaned
    if not np.any(lost):
        return cleaned.copy()

    thin = thin_structure_mask(original, min_feature_width_px)
    restorable = lost & thin
    if not np.any(restorable):
        return cleaned.copy()

    return cleaned | restorable
