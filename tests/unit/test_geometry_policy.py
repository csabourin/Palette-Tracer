"""
Geometry-policy tests (SPEC §20, MVP criteria 16-18).
"""

import numpy as np

from palette_trace.masks.geometry_policy import (
    EXCLUSIVE_LAYERS,
    SEPARATE_OPERATIONS,
    STACKED,
    STACKED_TRAPPED,
    allows_overlap,
    apply_trapping_to_mask,
    apply_underlap_to_mask,
    enforce_exclusive_ownership,
    validate_operation_masks,
)


def block(height=20, width=20, rows=slice(0, 10), cols=slice(0, 20)):
    mask = np.zeros((height, width), dtype=bool)
    mask[rows, cols] = True
    return mask


class TestPolicyClassification:
    def test_stacked_policies_allow_overlap(self):
        assert allows_overlap(STACKED)
        assert allows_overlap(STACKED_TRAPPED)

    def test_exclusive_policies_forbid_overlap(self):
        assert not allows_overlap(EXCLUSIVE_LAYERS)
        assert not allows_overlap(SEPARATE_OPERATIONS)


class TestUnderlap:
    def test_zero_underlap_is_identity(self):
        mask = block()
        assert np.array_equal(apply_underlap_to_mask(mask, np.ones_like(mask), 0.0), mask)

    def test_underlap_expands_within_the_silhouette(self):
        mask = block(rows=slice(5, 10))
        silhouette = np.ones_like(mask)
        expanded = apply_underlap_to_mask(mask, silhouette, 2.0)
        assert np.count_nonzero(expanded) > np.count_nonzero(mask)

    def test_underlap_never_grows_the_outer_silhouette(self):
        """§20.1: expansion is intersected with the classified subject pixels."""
        mask = block(rows=slice(5, 10))
        silhouette = block(rows=slice(4, 12))
        expanded = apply_underlap_to_mask(mask, silhouette, 5.0, preserve_outer_silhouette=True)
        assert not np.any(expanded & ~silhouette)

    def test_silhouette_preservation_can_be_disabled(self):
        mask = block(rows=slice(5, 10))
        silhouette = block(rows=slice(5, 10))
        expanded = apply_underlap_to_mask(mask, silhouette, 3.0, preserve_outer_silhouette=False)
        assert np.any(expanded & ~silhouette)


class TestTrapping:
    def test_zero_trapping_is_identity(self):
        mask = block()
        assert np.array_equal(apply_trapping_to_mask(mask, np.ones_like(mask), 0.0), mask)

    def test_trapping_stays_inside_the_silhouette(self):
        mask = block(rows=slice(5, 10))
        silhouette = block(rows=slice(4, 12))
        trapped = apply_trapping_to_mask(mask, silhouette, 4.0)
        assert not np.any(trapped & ~silhouette)


class TestExclusiveOwnership:
    def test_every_pixel_belongs_to_exactly_one_scan(self):
        """§20.3 / criterion 17: exclusive raster ownership."""
        overlapping = {
            "a": block(rows=slice(0, 12)),
            "b": block(rows=slice(8, 20)),
        }
        exclusive = enforce_exclusive_ownership(overlapping, ["a", "b"])

        stacked = np.zeros((20, 20), dtype=int)
        for mask in exclusive.values():
            stacked += mask.astype(int)
        assert stacked.max() <= 1

    def test_topmost_entry_wins_contested_pixels(self):
        overlapping = {
            "lower": block(rows=slice(0, 12)),
            "upper": block(rows=slice(8, 20)),
        }
        exclusive = enforce_exclusive_ownership(overlapping, ["lower", "upper"])
        contested = block(rows=slice(8, 12))
        assert np.all(exclusive["upper"][contested])
        assert not np.any(exclusive["lower"][contested])

    def test_union_is_preserved(self):
        """Exclusivity must reassign contested pixels, not discard them."""
        overlapping = {
            "a": block(rows=slice(0, 12)),
            "b": block(rows=slice(8, 20)),
        }
        expected = overlapping["a"] | overlapping["b"]
        exclusive = enforce_exclusive_ownership(overlapping, ["a", "b"])
        combined = np.zeros_like(expected)
        for mask in exclusive.values():
            combined |= mask
        assert np.array_equal(combined, expected)

    def test_entries_missing_from_layer_order_are_still_handled(self):
        masks = {"a": block(rows=slice(0, 5)), "orphan": block(rows=slice(10, 15))}
        exclusive = enforce_exclusive_ownership(masks, ["a"])
        assert set(exclusive) == {"a", "orphan"}
        assert np.any(exclusive["orphan"])


class TestOperationValidation:
    def test_empty_operation_is_reported(self):
        warnings = validate_operation_masks({"a": np.zeros((10, 10), dtype=bool)})
        assert [w["code"] for w in warnings] == ["empty_operation"]

    def test_operation_below_minimum_area_is_reported(self):
        tiny = np.zeros((10, 10), dtype=bool)
        tiny[0, 0] = True
        warnings = validate_operation_masks({"a": tiny}, minimum_area_px2=4)
        assert [w["code"] for w in warnings] == ["operation_below_minimum_area"]

    def test_duplicate_operations_are_reported(self):
        mask = block(rows=slice(0, 10))
        warnings = validate_operation_masks({"a": mask, "b": mask.copy()})
        assert any(w["code"] == "duplicate_operation" for w in warnings)

    def test_distinct_valid_operations_produce_no_warnings(self):
        masks = {"a": block(rows=slice(0, 8)), "b": block(rows=slice(12, 20))}
        assert validate_operation_masks(masks) == []
