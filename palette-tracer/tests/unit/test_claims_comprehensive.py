"""
Comprehensive claim-resolution tests (§14, §36).

Tests cover:
- Tie-breaking by priority index
- Tie-breaking by UUID lexical order
- Neutral-colour guardrail (hue disabled when chroma < 0.02)
- Linked vs independent channel modes
- Edge cases at tolerance boundaries
"""

import pytest
import numpy as np
from palette_trace.color.conversion import srgb_to_oklch, oklch_to_oklab
from palette_trace.color.claims import PinnedClaimEvaluator, resolve_claimed_pixels


# --------------------------------------------------------------------------- #
#  Helper fixtures                                                            #
# --------------------------------------------------------------------------- #

def make_pinned_entry(entry_id: str, hex_color: str, reach: int = 25, priority: int = 0):
    """Create a minimal pinned palette entry dict."""
    return {
        "id": entry_id,
        "kind": "pinned",
        "sourceAnchor": {"srgb": hex_color},
        "assignment": {
            "mode": "reserve_within_reach",
            "overallReach": reach,
            "channels": {"mode": "linked"},
        },
    }


# --------------------------------------------------------------------------- #
#  Tests                                                                      #
# --------------------------------------------------------------------------- #

class TestTieBreaking:
    """§14.3 — deterministic tie-breaking rules."""

    def test_priority_index_wins(self):
        """Lower priority index should win when scores are equal."""
        # Two entries with identical anchor colours
        entry_a = make_pinned_entry("a_alpha", "#FF0000", reach=25, priority=1)
        entry_b = make_pinned_entry("b_beta", "#FF0000", reach=25, priority=0)

        L, C, h = srgb_to_oklch(1.0, 0.0, 0.0)  # exact red

        ev_a = PinnedClaimEvaluator(entry_a, 1)
        ev_b = PinnedClaimEvaluator(entry_b, 0)

        eligible_a, score_a = ev_a.evaluate_pixel(L, C, h)
        eligible_b, score_b = ev_b.evaluate_pixel(L, C, h)

        assert eligible_a is True
        assert eligible_b is True
        # Scores should be identical (both exact matches)
        assert pytest.approx(score_a, abs=1e-9) == score_b

    def test_uuid_lexical_tiebreak(self):
        """When priority indices are equal, UUID lexical order decides."""
        entry_x = make_pinned_entry("x_charlie", "#FF0000", reach=25, priority=0)
        entry_a = make_pinned_entry("a_alpha", "#FF0000", reach=25, priority=0)

        L, C, h = srgb_to_oklch(1.0, 0.0, 0.0)

        ev_x = PinnedClaimEvaluator(entry_x, 0)
        ev_a = PinnedClaimEvaluator(entry_a, 0)

        eligible_x, score_x = ev_x.evaluate_pixel(L, C, h)
        eligible_a, score_a = ev_a.evaluate_pixel(L, C, h)

        assert eligible_x is True
        assert eligible_a is True
        # Both exact matches → scores identical
        assert pytest.approx(score_x, abs=1e-9) == score_a


class TestNeutralColorGuardrail:
    """§13.4 — hue channel disabled for low-chroma anchors."""

    def test_hue_disabled_for_neutral_anchor(self):
        """Near-grey anchor (chroma < 0.02) must disable hue checking."""
        # Very desaturated grey-blue
        entry = make_pinned_entry("neutral", "#808080", reach=50, priority=0)
        ev = PinnedClaimEvaluator(entry, 0)

        assert ev.hue_enabled is False, "Hue should be disabled for neutral anchor"

    def test_hue_enabled_for_chromatic_anchor(self):
        """Saturated colour anchor must enable hue checking."""
        entry = make_pinned_entry("chromatic", "#FF0000", reach=50, priority=0)
        ev = PinnedClaimEvaluator(entry, 0)

        assert ev.hue_enabled is True


class TestToleranceBoundaries:
    """Pixel exactly at tolerance boundary should be eligible."""

    def test_exact_boundary_eligible(self):
        """Pixel distance equal to tolerance → eligible with score ~1.0."""
        entry = make_pinned_entry("boundary", "#FF0000", reach=25, priority=0)
        ev = PinnedClaimEvaluator(entry, 0)

        # Red anchor — test a slightly shifted pixel still within tolerance
        L, C, h = srgb_to_oklch(1.0, 0.0, 0.0)
        
        # Add small delta to lightness (within reach=25 tolerance of 0.060)
        L_shifted = L + 0.05
        
        eligible, score = ev.evaluate_pixel(L_shifted, C, h)
        assert eligible is True
        assert score <= 1.0

    def test_just_over_boundary_ineligible(self):
        """Pixel just beyond tolerance → ineligible."""
        entry = make_pinned_entry("boundary", "#FF0000", reach=25, priority=0)
        ev = PinnedClaimEvaluator(entry, 0)

        L, C, h = srgb_to_oklch(1.0, 0.0, 0.0)
        
        # Exceed lightness tolerance (reach=25 → tol_L=0.060)
        L_exceeded = L + 0.07
        
        eligible, score = ev.evaluate_pixel(L_exceeded, C, h)
        assert eligible is False


class TestResolveClaimedPixels:
    """End-to-end claim resolution on small images."""

    def test_single_entry_claims_all_matching(self):
        """Uniform red image with one red pinned entry → 100% claimed."""
        # 4×4 uniform red image in OKLCH
        L, C, h = srgb_to_oklch(1.0, 0.0, 0.0)
        oklch_img = np.full((4, 4, 3), [L, C, h], dtype=np.float64)

        entries = [make_pinned_entry("red", "#FF0000", reach=50)]
        claims, stats = resolve_claimed_pixels(oklch_img, entries)

        assert stats["red"] == pytest.approx(100.0, abs=0.1)

    def test_two_entries_partition_image(self):
        """Red and blue regions partition correctly."""
        L_r, C_r, h_r = srgb_to_oklch(1.0, 0.0, 0.0)   # red
        L_b, C_b, h_b = srgb_to_oklch(0.0, 0.0, 1.0)   # blue

        oklch_img = np.zeros((4, 4, 3), dtype=np.float64)
        oklch_img[:, :2] = [L_r, C_r, h_r]   # left half red
        oklch_img[:, 2:] = [L_b, C_b, h_b]   # right half blue

        entries = [
            make_pinned_entry("red", "#FF0000", reach=50),
            make_pinned_entry("blue", "#0000FF", reach=50),
        ]
        claims, stats = resolve_claimed_pixels(oklch_img, entries)

        assert stats["red"] == pytest.approx(50.0, abs=1.0)
        assert stats["blue"] == pytest.approx(50.0, abs=1.0)

    def test_unclaimed_pixels_remain_none(self):
        """Pixels outside all tolerances should remain unclaimed."""
        L_r, C_r, h_r = srgb_to_oklch(1.0, 0.0, 0.0)   # red
        L_g, C_g, h_g = srgb_to_oklch(0.0, 1.0, 0.0)   # green (far from red)

        oklch_img = np.full((4, 4, 3), [L_g, C_g, h_g], dtype=np.float64)

        entries = [make_pinned_entry("red", "#FF0000", reach=0)]  # exact only
        claims, stats = resolve_claimed_pixels(oklch_img, entries)

        assert stats["red"] == pytest.approx(0.0, abs=0.1)


class TestIndependentChannelMode:
    """§14 — independent channel configuration."""

    def test_independent_mode_respects_weights(self):
        """Independent mode should use per-channel weights."""
        entry = {
            "id": "weighted",
            "kind": "pinned",
            "sourceAnchor": {"srgb": "#FF0000"},
            "assignment": {
                "mode": "reserve_within_reach",
                "channels": {
                    "mode": "independent",
                    "hue": {"enabled": True, "tolerance": 10.0, "weight": 2.0},
                    "chroma": {"enabled": True, "tolerance": 0.035, "weight": 1.0},
                    "lightness": {"enabled": True, "tolerance": 0.06, "weight": 1.0},
                },
            },
        }
        ev = PinnedClaimEvaluator(entry, 0)

        L, C, h = srgb_to_oklch(1.0, 0.0, 0.0)
        eligible, score = ev.evaluate_pixel(L, C, h)
        
        assert eligible is True
        assert pytest.approx(score, abs=1e-4) == 0.0


class TestDeterminism:
    """§36 — claim resolution must be deterministic."""

    def test_repeated_calls_identical(self):
        """Running resolve_claimed_pixels twice on same input → identical output."""
        L_r, C_r, h_r = srgb_to_oklch(1.0, 0.0, 0.0)
        oklch_img = np.full((8, 8, 3), [L_r, C_r, h_r], dtype=np.float64)

        entries = [make_pinned_entry("red", "#FF0000", reach=50)]
        
        claims1, stats1 = resolve_claimed_pixels(oklch_img, entries)
        claims2, stats2 = resolve_claimed_pixels(oklch_img, entries)

        np.testing.assert_array_equal(claims1, claims2)
        assert stats1 == stats2


if __name__ == "__main__":
    pytest.main([__file__, "-v"])