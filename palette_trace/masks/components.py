"""
Connected component analysis, speckle removal, and hole filling.

Every operation here runs once per scan on a full-resolution mask, so a
per-pixel Python loop is not an implementation detail — it is the difference
between an interface that answers and one that looks hung. The labelling is
delegated to `scipy.ndimage`, which is already a hard dependency, and the
per-component decisions are made with `np.bincount` rather than by walking the
label array again.
"""

import numpy as np
from scipy import ndimage

#: 4-connectivity, matching the flood fill this module used to implement by
#: hand. A diagonal touch is not connectivity (see tests/unit/test_masks.py).
_FOUR_CONNECTED = np.array([[0, 1, 0], [1, 1, 1], [0, 1, 0]], dtype=bool)


def label_connected_components(binary_mask: np.ndarray) -> tuple[np.ndarray, list[int]]:
    """
    4-connected component labeling on a boolean 2D mask.
    Returns (labels_array, list_of_component_sizes).

    `sizes[0]` is 0 and stands for the background, so `sizes[label]` is the
    area of that label — the indexing every caller here relies on.
    """
    mask = np.asarray(binary_mask, dtype=bool)
    if mask.size == 0:
        return np.zeros(mask.shape, dtype=np.int32), [0]

    labels, count = ndimage.label(mask, structure=_FOUR_CONNECTED)
    labels = labels.astype(np.int32, copy=False)

    sizes = np.bincount(labels.ravel(), minlength=count + 1)
    sizes[0] = 0
    return labels, [int(size) for size in sizes]


def component_sizes(labels: np.ndarray, count: int) -> np.ndarray:
    """Area of every label as an array, with index 0 zeroed (background)."""
    sizes = np.bincount(labels.ravel(), minlength=count + 1)
    sizes[0] = 0
    return sizes


def remove_small_speckles(binary_mask: np.ndarray, min_area_px2: float) -> np.ndarray:
    """Removes connected foreground components with area < min_area_px2."""
    mask = np.asarray(binary_mask, dtype=bool)
    if min_area_px2 <= 1 or not mask.any():
        return mask.copy()

    labels, count = ndimage.label(mask, structure=_FOUR_CONNECTED)
    sizes = component_sizes(labels, count)

    # A lookup table indexed by label turns "which components survive" into one
    # vectorised gather instead of one boolean pass per component.
    keep = sizes >= min_area_px2
    keep[0] = False
    return keep[labels]


def fill_small_holes(binary_mask: np.ndarray, max_hole_area_px2: float) -> np.ndarray:
    """Fills background holes enclosed within foreground where hole area <= max_hole_area_px2."""
    mask = np.asarray(binary_mask, dtype=bool)
    if max_hole_area_px2 <= 0 or mask.size == 0:
        return mask.copy()

    inverted = ~mask
    if not inverted.any():
        return mask.copy()

    labels, count = ndimage.label(inverted, structure=_FOUR_CONNECTED)
    sizes = component_sizes(labels, count)

    # Anything reachable from the border is exterior, not a hole.
    touching_edge = np.zeros(count + 1, dtype=bool)
    for border in (labels[0, :], labels[-1, :], labels[:, 0], labels[:, -1]):
        touching_edge[np.unique(border)] = True

    fill = (sizes <= max_hole_area_px2) & ~touching_edge
    fill[0] = False
    if not fill.any():
        return mask.copy()

    return mask | fill[labels]
