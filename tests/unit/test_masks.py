"""
Unit tests for Masks Representation & Cleanup (label_map, components, morphology, geometry).
"""

import pytest
import numpy as np
from palette_trace.masks.label_map import LabelMap
from palette_trace.masks.components import remove_small_speckles, fill_small_holes
from palette_trace.masks.morphology import dilate_mask, erode_mask
from palette_trace.masks.geometry_policy import apply_underlap_to_mask
from palette_trace.presets.preview_scaling import (
    pixel_exponent,
    scale_profile,
    scale_value,
)

def test_label_map():
    lm = LabelMap(10, 10, ["e1", "e2"])
    mask_e1 = np.zeros((10, 10), dtype=bool)
    mask_e1[2:5, 2:5] = True
    lm.set_label_for_mask(mask_e1, "e1")

    res = lm.get_binary_mask("e1")
    assert np.array_equal(res, mask_e1)

def test_speckle_removal():
    mask = np.zeros((10, 10), dtype=bool)
    mask[1, 1] = True  # 1 px speckle
    mask[4:8, 4:8] = True  # 16 px region

    cleaned = remove_small_speckles(mask, min_area_px2=4)
    assert cleaned[1, 1] == False
    assert cleaned[5, 5] == True

def test_hole_filling():
    mask = np.zeros((10, 10), dtype=bool)
    mask[1:9, 1:9] = True
    mask[4, 4] = False  # 1 px hole

    filled = fill_small_holes(mask, max_hole_area_px2=4)
    assert filled[4, 4] == True

def test_morphology():
    mask = np.zeros((10, 10), dtype=bool)
    mask[4:6, 4:6] = True

    dilated = dilate_mask(mask, 1.0)
    assert dilated[3, 4] == True

    eroded = erode_mask(dilated, 1.0)
    assert np.array_equal(eroded, mask)


class TestPreviewThresholdScaling:
    """
    SPEC §17.4, stated as the case it forbids: *a preview MUST NOT apply a
    full-resolution 16-pixel-area threshold as 16 preview pixels.*

    These work on the cleanup functions directly rather than through the
    pipeline, so what is being asserted is the arithmetic §17.4 mandates, not a
    particular image happening to survive resampling.
    """

    HALF = 0.5
    FULL_RESOLUTION_THRESHOLD = 16          # the figure §17.4 names

    def detail(self, side: int, canvas: int = 40) -> np.ndarray:
        """A solid square of `side`x`side` in the middle of an empty canvas."""
        mask = np.zeros((canvas, canvas), dtype=bool)
        start = canvas // 2
        mask[start:start + side, start:start + side] = True
        return mask

    def test_a_detail_just_over_the_threshold_survives_at_full_resolution(self):
        """The control: 25 px² against a 16 px² threshold is kept."""
        survivor = remove_small_speckles(self.detail(5), self.FULL_RESOLUTION_THRESHOLD)
        assert survivor.any()

    def test_the_unconverted_threshold_is_what_would_delete_it(self):
        """
        The same detail as it appears in a half-scale preview — about 6 px²
        where it was 25 — judged against the unconverted 16. This is the bug
        §17.4 exists to forbid, and it has to be demonstrated or the test below
        proves nothing.
        """
        previewed = self.detail(int(5 * self.HALF) + 1, canvas=20)     # 3x3 = 9 px²
        assert not remove_small_speckles(
            previewed, self.FULL_RESOLUTION_THRESHOLD
        ).any(), "the unscaled threshold was expected to delete the preview's detail"

    def test_the_converted_threshold_keeps_it(self):
        """§17.4: an area threshold scales by the *square* of the factor."""
        previewed = self.detail(int(5 * self.HALF) + 1, canvas=20)
        converted = scale_value(self.FULL_RESOLUTION_THRESHOLD, self.HALF, 2)

        assert converted == 4.0
        assert remove_small_speckles(previewed, converted).any()

    def test_a_profile_scales_areas_by_the_square_and_lengths_by_the_factor(self):
        profile = {
            "mask": {
                "minimumRegionAreaPx2": 16,
                "fillHolesAreaPx2": 8,
                "closeGapsRadiusPx": 2,
                "smoothingRadiusPx": 1.2,
                "minimumFeatureWidthPx": 3,
                "offsetPx": -4,
                "preserveThinFeatures": True,
            },
            "vector": {"minimumPathAreaPx2": 4, "cornerSensitivity": 0.65},
        }
        scaled = scale_profile(profile, 0.5)

        assert scaled["mask"]["minimumRegionAreaPx2"] == 4.0        # 16 x 0.25
        assert scaled["mask"]["fillHolesAreaPx2"] == 2.0            # 8 x 0.25
        assert scaled["vector"]["minimumPathAreaPx2"] == 1.0        # 4 x 0.25
        assert scaled["mask"]["closeGapsRadiusPx"] == 1.0           # 2 x 0.5
        assert scaled["mask"]["smoothingRadiusPx"] == 0.6
        assert scaled["mask"]["minimumFeatureWidthPx"] == 1.5
        assert scaled["mask"]["offsetPx"] == -2.0                   # sign survives

        # Everything that is not a pixel measurement is left exactly alone.
        assert scaled["mask"]["preserveThinFeatures"] is True
        assert scaled["vector"]["cornerSensitivity"] == 0.65

    def test_the_area_suffix_is_tested_before_the_linear_one(self):
        """
        `fillHolesAreaPx2` also ends in the letters of `…Px`. Getting this
        ordering wrong scales every area linearly, which is precisely the
        half-right answer §17.4 rules out — and it would look plausible.
        """
        assert pixel_exponent("fillHolesAreaPx2") == 2
        assert pixel_exponent("closeGapsRadiusPx") == 1
        assert pixel_exponent("preserveThinFeatures") == 0
        assert pixel_exponent("cornerSensitivity") == 0

    def test_every_pixel_setting_the_profiles_ship_is_recognised(self):
        """
        The suffix rule is only safe if the shipped fields really do carry it.
        A future field named `minimumRegionArea` would be silently left at full
        resolution, so the data file is checked against the rule rather than
        trusted to follow it.
        """
        from palette_trace.presets.profiles import get_builtin_profile, list_profile_ids

        for profile_id in list_profile_ids():
            profile = get_builtin_profile(profile_id)
            for section in ("mask", "vector"):
                for key, value in profile.get(section, {}).items():
                    if isinstance(value, bool) or not isinstance(value, (int, float)):
                        continue
                    assert pixel_exponent(key) > 0 or not key.lower().endswith(
                        ("area", "radius", "width", "offset")
                    ), f"{profile_id}.{section}.{key} looks like a pixel measurement"
