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
    assert calculate_hue_confidence(0.02) == 0.4
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
