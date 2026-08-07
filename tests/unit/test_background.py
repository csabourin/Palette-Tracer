"""
Background classification tests (SPEC §16, MVP criterion 12).
"""

import numpy as np

from palette_trace.color.background import (
    ALL_MATCHING,
    EDGE_CONNECTED,
    TRANSPARENT,
    background_rectangle_path,
    classify_background,
    extract_edge_connected_mask,
)


def framed_mask():
    """A border of matching pixels plus an enclosed matching square."""
    mask = np.zeros((20, 20), dtype=bool)
    mask[0, :] = mask[-1, :] = True
    mask[:, 0] = mask[:, -1] = True
    mask[8:12, 8:12] = True          # enclosed island of the same colour
    return mask


class TestEdgeConnected:
    def test_border_component_is_kept(self):
        result = extract_edge_connected_mask(framed_mask())
        assert result[0, 0] and result[0, 10] and result[19, 19]

    def test_enclosed_region_of_the_same_colour_is_preserved(self):
        """§16.1: enclosed regions must not be swallowed by the backdrop."""
        result = extract_edge_connected_mask(framed_mask())
        assert not result[9, 9]

    def test_non_matching_pixels_are_never_included(self):
        mask = framed_mask()
        result = extract_edge_connected_mask(mask)
        assert not np.any(result & ~mask)

    def test_mask_with_no_border_pixels_yields_nothing(self):
        mask = np.zeros((10, 10), dtype=bool)
        mask[4:6, 4:6] = True
        assert not np.any(extract_edge_connected_mask(mask))

    def test_fully_matching_mask_is_fully_connected(self):
        mask = np.ones((8, 8), dtype=bool)
        assert np.all(extract_edge_connected_mask(mask))

    def test_empty_image_is_handled(self):
        assert extract_edge_connected_mask(np.zeros((0, 0), dtype=bool)).shape == (0, 0)

    def test_diagonal_connection_does_not_bridge(self):
        """Flood fill is 4-connected, so a diagonal touch is not connectivity."""
        mask = np.zeros((10, 10), dtype=bool)
        mask[0, 0] = True
        mask[1, 1] = True
        result = extract_edge_connected_mask(mask)
        assert result[0, 0]
        assert not result[1, 1]


class TestMatchingModes:
    def test_all_matching_returns_the_input(self):
        mask = framed_mask()
        alpha = np.ones((20, 20), dtype=float)
        assert np.array_equal(classify_background(mask, alpha, ALL_MATCHING), mask)

    def test_edge_connected_drops_the_island(self):
        mask = framed_mask()
        alpha = np.ones((20, 20), dtype=float)
        result = classify_background(mask, alpha, EDGE_CONNECTED)
        assert result[0, 0]
        assert not result[9, 9]

    def test_transparent_mode_ignores_the_matching_mask(self):
        mask = np.zeros((10, 10), dtype=bool)
        alpha = np.ones((10, 10), dtype=float)
        alpha[0:3, :] = 0.0
        result = classify_background(mask, alpha, TRANSPARENT, alpha_threshold=0.05)
        assert np.all(result[0:3, :])
        assert not np.any(result[3:, :])

    def test_unknown_mode_falls_back_to_all_matching(self):
        mask = framed_mask()
        alpha = np.ones((20, 20), dtype=float)
        assert np.array_equal(classify_background(mask, alpha, "future_mode"), mask)


class TestRectangleReplacement:
    def test_rectangle_covers_the_full_intrinsic_bounds(self):
        """§16.2: replacement uses the source bounds, not the traced extent."""
        path = background_rectangle_path(640, 480)
        assert path == "M 0 0 L 640 0 L 640 480 L 0 480 Z"

    def test_rectangle_is_closed(self):
        assert background_rectangle_path(10, 10).rstrip().endswith("Z")
