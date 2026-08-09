"""
Unclaimed-pixel assignment tests (SPEC §14.4, §15.6, §35).
"""

import numpy as np
import pytest

from palette_trace.color.assignment import (
    DROP,
    NEAREST_AVAILABLE,
    assign_nearest,
    distribute_unclaimed,
    entry_centres,
    oklch_image_to_oklab,
)
from palette_trace.color.conversion import hex_to_srgb, srgb_array_to_oklch


def entry(entry_id, hex_value=None, anchor=None):
    return {
        "id": entry_id,
        "output": {"mode": "automatic_centroid",
                   "color": {"srgb": hex_value} if hex_value else None},
        "sourceAnchor": {"srgb": anchor} if anchor else None,
    }


def oklch_of(hex_values, height=4, width=4):
    """Builds a small OKLCH image whose left/right halves use two colours."""
    srgb = np.zeros((height, width, 3), dtype=np.float64)
    srgb[:, : width // 2] = hex_to_srgb(hex_values[0])
    srgb[:, width // 2:] = hex_to_srgb(hex_values[1])
    return srgb_array_to_oklch(srgb)


class TestCentres:
    def test_output_colour_is_used(self):
        ids, centres = entry_centres([entry("a", "#FF0000")])
        assert ids == ["a"]
        assert centres.shape == (1, 3)

    def test_source_anchor_is_the_fallback(self):
        ids, _ = entry_centres([entry("a", None, anchor="#00FF00")])
        assert ids == ["a"]

    def test_entries_without_a_colour_are_skipped(self):
        """Defaulting to black would silently attract unrelated pixels."""
        ids, centres = entry_centres([entry("a"), entry("b", "#FF0000")])
        assert ids == ["b"]
        assert centres.shape == (1, 3)

    def test_no_colours_yields_an_empty_array(self):
        ids, centres = entry_centres([entry("a")])
        assert ids == []
        assert centres.shape == (0, 3)


class TestConversion:
    def test_oklch_to_oklab_round_trips_chroma_and_hue(self):
        oklch = np.array([[[0.6, 0.1, 0.0]], [[0.6, 0.1, 90.0]]])
        oklab = oklch_image_to_oklab(oklch)
        assert oklab[0, 0, 1] == pytest.approx(0.1)
        assert oklab[0, 0, 2] == pytest.approx(0.0, abs=1e-9)
        assert oklab[1, 0, 1] == pytest.approx(0.0, abs=1e-9)
        assert oklab[1, 0, 2] == pytest.approx(0.1)

    def test_lightness_is_preserved(self):
        oklch = np.array([[[0.42, 0.2, 137.0]]])
        assert oklch_image_to_oklab(oklch)[0, 0, 0] == pytest.approx(0.42)


class TestAssignNearest:
    def test_unmasked_pixels_stay_unassigned(self):
        oklab = np.zeros((4, 4, 3))
        mask = np.zeros((4, 4), dtype=bool)
        mask[0, 0] = True
        result = assign_nearest(oklab, mask, np.array([[0.0, 0.0, 0.0]]))
        assert result[0, 0] == 0
        assert (result[1:, :] == -1).all()

    def test_each_pixel_takes_its_nearest_centre(self):
        oklch = oklch_of(["#FF0000", "#0000FF"])
        oklab = oklch_image_to_oklab(oklch)
        _, centres = entry_centres([entry("red", "#FF0000"), entry("blue", "#0000FF")])

        result = assign_nearest(oklab, np.ones((4, 4), dtype=bool), centres)
        assert (result[:, :2] == 0).all()
        assert (result[:, 2:] == 1).all()

    def test_no_centres_assigns_nothing(self):
        result = assign_nearest(np.zeros((3, 3, 3)), np.ones((3, 3), dtype=bool),
                                np.empty((0, 3)))
        assert (result == -1).all()

    def test_ties_resolve_to_the_lowest_index(self):
        """Determinism (§34.30): the result depends only on palette order."""
        oklab = np.zeros((2, 2, 3))
        centres = np.array([[0.0, 0.0, 0.0], [0.0, 0.0, 0.0]])
        assert (assign_nearest(oklab, np.ones((2, 2), dtype=bool), centres) == 0).all()

    def test_a_later_exact_tie_does_not_displace_the_winner(self):
        """
        The sharp edge of §34.30 for a scan that compares one centre at a time:
        an incumbent must be replaced only by something strictly closer. A
        later centre at exactly the same distance would otherwise steal the
        pixel, and the answer would depend on iteration order.
        """
        oklab = np.zeros((1, 1, 3))
        centres = np.array([
            [0.9, 0.0, 0.0],        # far
            [0.2, 0.0, 0.0],        # nearest, and first to be nearest
            [0.5, 0.0, 0.0],        # far
            [-0.2, 0.0, 0.0],       # exactly as near as index 1
        ])
        assert assign_nearest(oklab, np.ones((1, 1), dtype=bool), centres)[0, 0] == 1

    def test_it_agrees_with_the_brute_force_distance_table(self):
        """
        The reference the scan replaced: build the whole (n, k) table and take
        `argmin`. That is unaffordable at four megapixels — it is the 480 MB of
        temporaries this function exists to avoid — but at 40x30 it is the
        clearest possible statement of what the answer should be.
        """
        rng = np.random.default_rng(7)
        oklab = rng.uniform(-0.4, 1.0, size=(40, 30, 3)).astype(np.float32)
        centres = rng.uniform(-0.4, 1.0, size=(11, 3))
        mask = rng.random((40, 30)) > 0.25

        pixels = oklab[mask]
        distances = ((pixels[:, None, :] - centres[None, :, :]) ** 2).sum(axis=2)
        expected = np.full((40, 30), -1, dtype=np.int32)
        expected[mask] = np.argmin(distances, axis=1)

        assert np.array_equal(assign_nearest(oklab, mask, centres), expected)

    def test_it_handles_the_largest_supported_scan_count(self):
        """§13: at most 64 scans, so the per-centre scan must stay correct there."""
        rng = np.random.default_rng(11)
        oklab = rng.uniform(0.0, 1.0, size=(8, 8, 3)).astype(np.float32)
        centres = rng.uniform(0.0, 1.0, size=(64, 3))

        result = assign_nearest(oklab, np.ones((8, 8), dtype=bool), centres)
        pixels = oklab.reshape(-1, 3)
        distances = ((pixels[:, None, :] - centres[None, :, :]) ** 2).sum(axis=2)
        assert np.array_equal(result.reshape(-1), np.argmin(distances, axis=1))


class TestDistribute:
    def test_unclaimed_pixels_spread_across_automatic_entries(self):
        """The bug this fixes: every unclaimed pixel landed in one scan."""
        oklch = oklch_of(["#FF0000", "#0000FF"])
        entries = [entry("red", "#FF0000"), entry("blue", "#0000FF")]

        result = distribute_unclaimed(oklch, np.ones((4, 4), dtype=bool), entries, [])
        assert set(result) == {"red", "blue"}
        assert result["red"].sum() == 8
        assert result["blue"].sum() == 8

    def test_masks_are_disjoint(self):
        """§14.5: one pixel belongs to at most one scan."""
        oklch = oklch_of(["#FF0000", "#0000FF"])
        entries = [entry("red", "#FF0000"), entry("blue", "#0000FF")]
        result = distribute_unclaimed(oklch, np.ones((4, 4), dtype=bool), entries, [])

        total = np.zeros((4, 4), dtype=int)
        for mask in result.values():
            total += mask.astype(int)
        assert total.max() == 1

    def test_assignment_is_confined_to_the_unclaimed_mask(self):
        oklch = oklch_of(["#FF0000", "#0000FF"])
        unclaimed = np.zeros((4, 4), dtype=bool)
        unclaimed[0, :] = True
        result = distribute_unclaimed(
            oklch, unclaimed, [entry("red", "#FF0000"), entry("blue", "#0000FF")], [])

        for mask in result.values():
            assert not np.any(mask & ~unclaimed)

    def test_empty_mask_assigns_nothing(self):
        oklch = oklch_of(["#FF0000", "#0000FF"])
        assert distribute_unclaimed(
            oklch, np.zeros((4, 4), dtype=bool), [entry("a", "#FF0000")], []) == {}

    def test_nearest_available_falls_back_to_pinned(self):
        """§14.4: with no automatic scans, unclaimed pixels go to the nearest pinned."""
        oklch = oklch_of(["#FF0000", "#0000FF"])
        result = distribute_unclaimed(
            oklch, np.ones((4, 4), dtype=bool), [],
            [entry("pin", "#FF0000")], NEAREST_AVAILABLE)
        assert set(result) == {"pin"}
        assert result["pin"].sum() == 16

    def test_drop_policy_leaves_pixels_unassigned(self):
        oklch = oklch_of(["#FF0000", "#0000FF"])
        result = distribute_unclaimed(
            oklch, np.ones((4, 4), dtype=bool), [], [entry("pin", "#FF0000")], DROP)
        assert result == {}

    def test_default_policy_is_nearest_available(self):
        oklch = oklch_of(["#FF0000", "#0000FF"])
        assert distribute_unclaimed(
            oklch, np.ones((4, 4), dtype=bool), [], [entry("pin", "#FF0000")]) != {}

    def test_result_is_deterministic(self):
        oklch = oklch_of(["#FF0000", "#0000FF"])
        entries = [entry("red", "#FF0000"), entry("blue", "#0000FF")]
        first = distribute_unclaimed(oklch, np.ones((4, 4), dtype=bool), entries, [])
        second = distribute_unclaimed(oklch, np.ones((4, 4), dtype=bool), entries, [])
        assert set(first) == set(second)
        assert all(np.array_equal(first[k], second[k]) for k in first)
