"""
Connected component analysis, speckle removal, and hole filling.
"""

import numpy as np

# Use scipy's C-level labeling when available — it is orders of magnitude
# faster than the pure-Python BFS for images larger than a few hundred pixels.
try:
    from scipy.ndimage import label as _scipy_label

    # 4-connectivity structure (no diagonals), matching the original BFS.
    _STRUCT_4 = np.array([[0, 1, 0], [1, 1, 1], [0, 1, 0]], dtype=bool)

    def label_connected_components(binary_mask: np.ndarray) -> tuple[np.ndarray, list[int]]:
        """
        4-connected component labeling on a boolean 2D mask.
        Returns (labels_array, list_of_component_sizes).
        """
        labeled, num_features = _scipy_label(binary_mask, structure=_STRUCT_4)
        sizes = [0]  # index 0 = background
        for i in range(1, num_features + 1):
            sizes.append(int(np.count_nonzero(labeled == i)))
        return labeled, sizes

except ImportError:
    from collections import deque

    def label_connected_components(binary_mask: np.ndarray) -> tuple[np.ndarray, list[int]]:
        """
        4-connected component labeling on a boolean 2D mask (pure-Python fallback).
        Returns (labels_array, list_of_component_sizes).
        """
        height, width = binary_mask.shape
        labels = np.zeros((height, width), dtype=np.int32)
        current_label = 0
        sizes = [0]  # index 0 represents background (0 size)

        for y in range(height):
            for x in range(width):
                if binary_mask[y, x] and labels[y, x] == 0:
                    current_label += 1
                    count = 0
                    queue = deque([(y, x)])
                    labels[y, x] = current_label

                    while queue:
                        cy, cx = queue.popleft()
                        count += 1
                        for dy, dx in ((-1, 0), (1, 0), (0, -1), (0, 1)):
                            ny, nx = cy + dy, cx + dx
                            if 0 <= ny < height and 0 <= nx < width:
                                if binary_mask[ny, nx] and labels[ny, nx] == 0:
                                    labels[ny, nx] = current_label
                                    queue.append((ny, nx))

                    sizes.append(count)

        return labels, sizes

def remove_small_speckles(binary_mask: np.ndarray, min_area_px2: float) -> np.ndarray:
    """Removes connected foreground components with area < min_area_px2."""
    if min_area_px2 <= 1:
        return binary_mask.copy()

    labels, sizes = label_connected_components(binary_mask)
    cleaned = binary_mask.copy()

    for label_id, size in enumerate(sizes):
        if label_id == 0:
            continue
        if size < min_area_px2:
            cleaned[labels == label_id] = False

    return cleaned

def fill_small_holes(binary_mask: np.ndarray, max_hole_area_px2: float) -> np.ndarray:
    """Fills background holes enclosed within foreground where hole area <= max_hole_area_px2."""
    if max_hole_area_px2 <= 0:
        return binary_mask.copy()

    inverted = ~binary_mask
    labels, sizes = label_connected_components(inverted)
    cleaned = binary_mask.copy()

    height, width = binary_mask.shape
    touching_edge = set()

    # Find background components touching boundary (exterior background)
    for x in range(width):
        if labels[0, x] > 0:
            touching_edge.add(labels[0, x])
        if labels[height - 1, x] > 0:
            touching_edge.add(labels[height - 1, x])

    for y in range(height):
        if labels[y, 0] > 0:
            touching_edge.add(labels[y, 0])
        if labels[y, width - 1] > 0:
            touching_edge.add(labels[y, width - 1])

    # Any inverted component not touching edge and size <= max_hole_area_px2 is a hole to fill
    for label_id, size in enumerate(sizes):
        if label_id == 0 or label_id in touching_edge:
            continue
        if size <= max_hole_area_px2:
            cleaned[labels == label_id] = True

    return cleaned
