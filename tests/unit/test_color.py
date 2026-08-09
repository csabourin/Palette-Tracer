"""
Unit tests for Color Engine (sRGB, OKLab, OKLCH, Reach, Claims, Quantizer).
"""

import pytest
import numpy as np
from palette_trace.color.conversion import (
    hex_to_srgb,
    srgb_to_hex,
    srgb_to_oklab,
    oklab_to_srgb,
    srgb_array_to_oklch,
    srgb_to_oklch,
    shortest_hue_distance,
)
from palette_trace.color.reach import get_tolerances_for_reach, calculate_hue_confidence
from palette_trace.color.claims import PinnedClaimEvaluator, resolve_claimed_pixels
from palette_trace.color.quantizer import run_deterministic_quantization

def test_hex_conversion():
    r, g, b = hex_to_srgb("#FF0000")
    assert (r, g, b) == (1.0, 0.0, 0.0)
    assert srgb_to_hex(1.0, 0.0, 0.0) == "#FF0000"

def test_oklab_roundtrip():
    L, a, b = srgb_to_oklab(0.2, 0.5, 0.8)
    r2, g2, b2 = oklab_to_srgb(L, a, b)
    assert pytest.approx(r2, abs=1e-3) == 0.2
    assert pytest.approx(g2, abs=1e-3) == 0.5
    assert pytest.approx(b2, abs=1e-3) == 0.8

def test_shortest_hue_distance():
    assert shortest_hue_distance(10.0, 350.0) == 20.0
    assert shortest_hue_distance(0.0, 180.0) == 180.0
    assert shortest_hue_distance(40.0, 50.0) == 10.0

def test_reach_mapping():
    tols_0 = get_tolerances_for_reach(0)
    assert tols_0["hue"] == 0.5
    assert tols_0["chroma"] == 0.002

    tols_25 = get_tolerances_for_reach(25)
    assert tols_25["hue"] == 10.0
    assert tols_25["chroma"] == 0.035

def test_neutral_color_guardrails():
    assert calculate_hue_confidence(0.0) == 0.0
    assert pytest.approx(calculate_hue_confidence(0.02), abs=1e-6) == 0.4
    assert calculate_hue_confidence(0.05) == 1.0
    assert calculate_hue_confidence(0.10) == 1.0

def test_pinned_claim_evaluator():
    entry = {
        "id": "e1",
        "kind": "pinned",
        "sourceAnchor": { "srgb": "#FF0000" },
        "assignment": {
            "mode": "reserve_within_reach",
            "overallReach": 25,
            "channels": { "mode": "linked" },
        },
    }
    ev = PinnedClaimEvaluator(entry, 0)
    # Red pixel (exact match)
    L, C, h = srgb_to_oklch(1.0, 0.0, 0.0)
    eligible, score = ev.evaluate_pixel(L, C, h)
    assert eligible is True
    assert pytest.approx(score, abs=1e-4) == 0.0

def test_deterministic_quantizer():
    hist = [
        {"oklab": (0.5, 0.1, 0.1), "srgb": (0.5, 0.1, 0.1), "weight": 100, "packed_srgb": 1234},
        {"oklab": (0.8, -0.1, 0.2), "srgb": (0.8, 0.0, 0.2), "weight": 50, "packed_srgb": 5678},
    ]
    res1 = run_deterministic_quantization(hist, 2)
    res2 = run_deterministic_quantization(hist, 2)
    assert len(res1) == 2
    assert res1[0]["hex"] == res2[0]["hex"]


class TestVectorisedOklch:
    """
    `srgb_array_to_oklch` is the whole cost of the first trace, so its working
    precision follows its input instead of forcing float64 (§21.1 fixes the
    working space, not the storage width). These pin down what that is allowed
    to mean.
    """

    def sample(self, dtype):
        values = np.array([
            [[0.0, 0.0, 0.0], [1.0, 1.0, 1.0], [0.04045, 0.0405, 0.03]],
            [[0.2, 0.5, 0.8], [1.0, 0.0, 0.0], [0.5, 0.5, 0.5]],
        ], dtype=dtype)
        return srgb_array_to_oklch(values)

    def test_a_float32_image_stays_float32(self):
        """A decoded bitmap is float32 and is stored as float32; widening in
        between bought nothing and cost every pixel a double-precision trip."""
        assert self.sample(np.float32).dtype == np.float32

    def test_a_float64_input_keeps_its_precision(self):
        """The histogram's per-bin arrays are small and float64. Nothing about
        the image path should quietly coarsen them."""
        assert self.sample(np.float64).dtype == np.float64

    def test_an_integer_input_is_promoted_rather_than_truncated(self):
        result = srgb_array_to_oklch(np.zeros((1, 1, 3), dtype=np.uint8))
        assert result.dtype == np.float64

    def test_it_matches_the_scalar_conversion(self):
        """
        §5: correctness must not depend on NumPy, so the vectorised path is only
        ever an optimisation of :func:`srgb_to_oklch` and has to answer the same.
        Both branches of the piecewise transfer function are covered — the
        fractional power is now evaluated only above the knee.
        """
        # L and C live in 0..1; hue is an angle in degrees, so it carries its
        # own tolerance rather than borrowing one on a different scale.
        for dtype, tolerance, hue_tolerance in (
            (np.float64, 1e-12, 1e-9),
            (np.float32, 1e-5, 1e-2),
        ):
            flat = self.sample(dtype).reshape(-1, 3)
            source = np.array([
                [0.0, 0.0, 0.0], [1.0, 1.0, 1.0], [0.04045, 0.0405, 0.03],
                [0.2, 0.5, 0.8], [1.0, 0.0, 0.0], [0.5, 0.5, 0.5],
            ], dtype=dtype)
            for index, (r, g, b) in enumerate(source):
                expected = srgb_to_oklch(float(r), float(g), float(b))
                assert flat[index, 0] == pytest.approx(expected[0], abs=tolerance)
                assert flat[index, 1] == pytest.approx(expected[1], abs=tolerance)
                # Hue is meaningless at zero chroma and wraps at 360, so it is
                # compared only where there is a hue to compare.
                if expected[1] > 1e-4:
                    assert flat[index, 2] == pytest.approx(expected[2], abs=hue_tolerance)

    def test_the_knee_of_the_transfer_function_is_on_the_linear_side(self):
        """
        sRGB's piecewise split is `<= 0.04045`, and the power is now gated on
        the complement. An off-by-one on that boundary would be invisible on a
        photograph and wrong on a test chart.
        """
        knee = np.array([[[0.04045, 0.04045, 0.04045]]], dtype=np.float64)
        assert srgb_array_to_oklch(knee)[0, 0, 0] == pytest.approx(
            srgb_to_oklch(0.04045, 0.04045, 0.04045)[0], abs=1e-12
        )

