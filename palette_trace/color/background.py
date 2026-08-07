"""
Background classification (SPEC §16).

Background is a *role*, not a separate palette type (§16.3): any entry, pinned or
automatic, may be designated as background. What differs is how its pixels are
selected (§16.1) and how they are emitted (§16.2).
"""

import numpy as np

#: Matching modes from §16.1.
ALL_MATCHING = "all_matching"
EDGE_CONNECTED = "edge_connected"
TRANSPARENT = "transparent"

#: Output modes from §16.2.
KEEP_PATHS = "keep_paths"
OMIT = "omit"
REPLACE_WITH_RECTANGLE = "replace_with_rectangle"


def extract_edge_connected_mask(matching_mask: np.ndarray) -> np.ndarray:
    """
    Returns only the components of `matching_mask` that touch the image boundary.

    Enclosed regions of the same colour are preserved as non-background (§16.1),
    so a white eye inside a character does not become part of a white backdrop.

    Implemented as an iterative binary dilation constrained to the matching mask,
    which is equivalent to a 4-connected flood fill from the border but stays in
    numpy rather than a per-pixel Python queue.
    """
    height, width = matching_mask.shape
    if height == 0 or width == 0:
        return np.zeros((height, width), dtype=bool)

    matching = matching_mask.astype(bool)

    reached = np.zeros((height, width), dtype=bool)
    reached[0, :] = matching[0, :]
    reached[-1, :] = matching[-1, :]
    reached[:, 0] = matching[:, 0]
    reached[:, -1] = matching[:, -1]

    while True:
        grown = reached.copy()
        grown[1:, :] |= reached[:-1, :]
        grown[:-1, :] |= reached[1:, :]
        grown[:, 1:] |= reached[:, :-1]
        grown[:, :-1] |= reached[:, 1:]
        grown &= matching

        if np.array_equal(grown, reached):
            return reached
        reached = grown


def classify_background(
    matching_mask: np.ndarray,
    alpha: np.ndarray,
    mode: str = ALL_MATCHING,
    alpha_threshold: float = 0.05,
) -> np.ndarray:
    """
    Applies a §16.1 matching mode to a candidate background mask.

    `matching_mask` is the set of pixels the background entry would otherwise
    claim. `alpha` is the source alpha channel in 0..1.
    """
    if mode == TRANSPARENT:
        return alpha < alpha_threshold

    matching = matching_mask.astype(bool)
    if mode == EDGE_CONNECTED:
        return extract_edge_connected_mask(matching)

    return matching


def background_rectangle_path(width: int, height: int) -> str:
    """
    SVG path data covering the complete intrinsic source bounds (§16.2).

    Rectangle replacement uses the full image bounds rather than the traced
    background extent, and is placed with the same source-to-document transform
    as every other generated path.
    """
    return f"M 0 0 L {width} 0 L {width} {height} L 0 {height} Z"
