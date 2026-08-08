"""
Reach calibration validation tests (§13, §36).

Tests verify:
- Interpolation correctness at all anchor points and midpoints
- Low-chroma suppression behaviour (§13.3–§13.4)
- Edge cases at reach=0 (exact) and reach=100 (aggressive)
"""

import pytest
from palette_trace.color.reach import (
    get_tolerances_for_reach,
    calculate_hue_confidence,
    _load_reach_data,
)
from palette_trace.color.conversion import srgb_to_oklch


# --------------------------------------------------------------------------- #
#  Anchor point tests                                                         #
# --------------------------------------------------------------------------- #

class TestAnchorPoints:
    """Tolerances at exact anchor values must match the JSON table."""

    def test_reach_0_exact(self):
        tols = get_tolerances_for_reach(0)
        assert pytest.approx(tols["hue"]) == 0.5, "reach=0 hue should be 0.5°"
        assert pytest.approx(tols["chroma"]) == 0.002
        assert pytest.approx(tols["lightness"]) == 0.002

    def test_reach_25_narrow(self):
        tols = get_tolerances_for_reach(25)
        assert pytest.approx(tols["hue"]) == 10.0
        assert pytest.approx(tols["chroma"]) == 0.035
        assert pytest.approx(tols["lightness"]) == 0.060

    def test_reach_50_similar(self):
        tols = get_tolerances_for_reach(50)
        assert pytest.approx(tols["hue"]) == 25.0
        assert pytest.approx(tols["chroma"]) == 0.080
        assert pytest.approx(tols["lightness"]) == 0.160

    def test_reach_75_broad(self):
        tols = get_tolerances_for_reach(75)
        assert pytest.approx(tols["hue"]) == 60.0
        assert pytest.approx(tols["chroma"]) == 0.160
        assert pytest.approx(tols["lightness"]) == 0.350

    def test_reach_100_aggressive(self):
        tols = get_tolerances_for_reach(100)
        assert pytest.approx(tols["hue"]) == 180.0
        assert pytest.approx(tols["chroma"]) == 0.400
        assert pytest.approx(tols["lightness"]) == 1.000


# --------------------------------------------------------------------------- #
#  Interpolation tests                                                        #
# --------------------------------------------------------------------------- #

class TestInterpolation:
    """Midpoint interpolation must be linear between anchors."""

    def test_midpoint_12_5(self):
        """reach=12.5 → halfway between reach=0 and reach=25."""
        tols = get_tolerances_for_reach(12)
        # Linear interpolation: (0.5 + 10.0) / 2 ≈ 5.25 for hue
        expected_hue = 0.5 + 0.48 * (10.0 - 0.5)  # t=12/25≈0.48
        assert pytest.approx(tols["hue"], abs=0.5) == expected_hue

    def test_monotonically_increasing(self):
        """All tolerance channels must increase with reach."""
        prev = get_tolerances_for_reach(0)
        for reach in range(1, 101):
            curr = get_tolerances_for_reach(reach)
            assert curr["hue"] >= prev["hue"], f"Hue decreased at reach={reach}"
            assert curr["chroma"] >= prev["chroma"], f"Chroma decreased at reach={reach}"
            assert curr["lightness"] >= prev["lightness"], f"Lightness decreased at reach={reach}"
            prev = curr

    def test_clamped_at_0(self):
        """Negative reach values should clamp to 0."""
        tols_neg = get_tolerances_for_reach(-10)
        tols_0 = get_tolerances_for_reach(0)
        assert tols_neg == tols_0

    def test_clamped_at_100(self):
        """Reach > 100 should clamp to 100."""
        tols_over = get_tolerances_for_reach(150)
        tols_100 = get_tolerances_for_reach(100)
        assert tols_over == tols_100


# --------------------------------------------------------------------------- #
#  Hue confidence tests                                                       #
# --------------------------------------------------------------------------- #

class TestHueConfidence:
    """§13.3 — hue confidence multiplier behaviour."""

    def test_zero_chroma_no_hue(self):
        assert calculate_hue_confidence(0.0) == pytest.approx(0.0)

    def test_half_threshold(self):
        # chroma=0.025 → 0.025/0.05 = 0.5
        assert calculate_hue_confidence(0.025) == pytest.approx(0.5)

    def test_full_threshold(self):
        assert calculate_hue_confidence(0.05) == pytest.approx(1.0)

    def test_above_threshold_clamped(self):
        assert calculate_hue_confidence(0.10) == pytest.approx(1.0)
        assert calculate_hue_confidence(0.50) == pytest.approx(1.0)

    def test_negative_chroma_handled(self):
        # Should not crash; clamp to 0
        assert calculate_hue_confidence(-0.01) == pytest.approx(0.0)


# --------------------------------------------------------------------------- #
#  Data integrity tests                                                       #
# --------------------------------------------------------------------------- #

class TestReachDataIntegrity:
    """Reach mapping JSON must be well-formed."""

    def test_anchors_loaded(self):
        anchors = _load_reach_data()
        assert len(anchors) == 5, "Expected exactly 5 anchor points"

    def test_anchor_fields_present(self):
        anchors = _load_reach_data()
        required_keys = {"reach", "hue", "chroma", "lightness"}
        for anchor in anchors:
            assert required_keys.issubset(anchor.keys()), f"Missing keys in {anchor}"

    def test_anchor_reaches_ordered(self):
        anchors = _load_reach_data()
        reaches = [a["reach"] for a in anchors]
        assert reaches == sorted(reaches), "Anchors must be ordered by reach value"

    def test_reach_range_complete(self):
        """First anchor at 0, last at 100."""
        anchors = _load_reach_data()
        assert anchors[0]["reach"] == 0
        assert anchors[-1]["reach"] == 100


# --------------------------------------------------------------------------- #
#  Integration: reach + claims together                                       #
# --------------------------------------------------------------------------- #

class TestReachClaimsIntegration:
    """Higher reach → more pixels claimed."""

    def test_higher_reach_claims_more(self):
        from palette_trace.color.claims import resolve_claimed_pixels
        import numpy as np

        L, C, h = srgb_to_oklch(1.0, 0.0, 0.0)  # red anchor
        
        # Create gradient image: exact red → slightly shifted
        oklch_img = np.zeros((4, 4, 3), dtype=np.float64)
        for i in range(4):
            delta = i * 0.02
            oklch_img[:, i] = [L + delta, C, h]

        entry_low_reach = {
            "id": "e1",
            "kind": "pinned",
            "sourceAnchor": {"srgb": "#FF0000"},
            "assignment": {
                "mode": "reserve_within_reach",
                "overallReach": 25,
                "channels": {"mode": "linked"},
            },
        }

        entry_high_reach = {
            "id": "e1",
            "kind": "pinned",
            "sourceAnchor": {"srgb": "#FF0000"},
            "assignment": {
                "mode": "reserve_within_reach",
                "overallReach": 75,
                "channels": {"mode": "linked"},
            },
        }

        _, stats_low = resolve_claimed_pixels(oklch_img, [entry_low_reach])
        _, stats_high = resolve_claimed_pixels(oklch_img, [entry_high_reach])

        assert stats_high["e1"] >= stats_low["e1"], \
            "Higher reach should claim more pixels"


if __name__ == "__main__":
    pytest.main([__file__, "-v"])
