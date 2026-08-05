"""
Background classification and flood-fill edge connectivity.
"""

import numpy as np

def extract_edge_connected_mask(matching_mask: np.ndarray) -> np.ndarray:
    """
    Given a boolean 2D mask of matching pixels, return a boolean mask containing
    only the connected component(s) touching the image boundary.
    """
    height, width = matching_mask.shape
    if height == 0 or width == 0:
        return np.zeros((height, width), dtype=bool)

    edge_mask = np.zeros((height, width), dtype=bool)
    visited = np.zeros((height, width), dtype=bool)

    # Collect boundary seed coordinates where matching_mask is True
    seeds = []
    # Top and bottom rows
    for x in range(width):
        if matching_mask[0, x]:
            seeds.append((0, x))
        if matching_mask[height - 1, x]:
            seeds.append((height - 1, x))

    # Left and right columns
    for y in range(height):
        if matching_mask[y, 0]:
            seeds.append((y, 0))
        if matching_mask[y, width - 1]:
            seeds.append((y, width - 1))

    # 4-connected BFS flood fill
    queue = list(set(seeds))
    for r, c in queue:
        visited[r, c] = True
        edge_mask[r, c] = True

    idx = 0
    while idx < len(queue):
        r, c = queue[idx]
        idx += 1

        for dr, dc in ((-1, 0), (1, 0), (0, -1), (0, 1)):
            nr, nc = r + dr, c + dc
            if 0 <= nr < height and 0 <= nc < width:
                if not visited[nr, nc] and matching_mask[nr, nc]:
                    visited[nr, nc] = True
                    edge_mask[nr, nc] = True
                    queue.append((nr, nc))

    return edge_mask
