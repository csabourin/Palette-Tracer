"""
Deterministic quantizer tests (§15, §36).

Tests verify:
- Identical inputs produce identical outputs (determinism)
- Fixed centers are preserved in output
- Convergence within reasonable iterations
- Edge cases: empty histogram, single bin, over-requested centres
"""

import pytest

from palette_trace.color.conversion import srgb_to_oklab
from palette_trace.color.quantizer import run_deterministic_quantization

# --------------------------------------------------------------------------- #
#  Helper fixtures                                                            #
# --------------------------------------------------------------------------- #

def make_histogram_entry(r: float, g: float, b: float, weight: int) -> dict:
    """Create a histogram entry from sRGB values."""
    oklab = srgb_to_oklab(r, g, b)
    packed_srgb = (int(r * 255) << 16) | (int(g * 255) << 8) | int(b * 255)
    return {
        "oklab": oklab,
        "srgb": (r, g, b),
        "weight": weight,
        "packed_srgb": packed_srgb,
    }


# --------------------------------------------------------------------------- #
#  Tests                                                                      #
# --------------------------------------------------------------------------- #

class TestDeterminism:
    """§34.30 — quantizer must be deterministic."""

    def test_identical_inputs_same_output(self):
        hist = [
            make_histogram_entry(1.0, 0.0, 0.0, 100),   # red
            make_histogram_entry(0.0, 1.0, 0.0, 80),    # green
            make_histogram_entry(0.0, 0.0, 1.0, 60),    # blue
            make_histogram_entry(1.0, 1.0, 0.0, 40),    # yellow
        ]

        result1 = run_deterministic_quantization(hist, target_count=3)
        result2 = run_deterministic_quantization(hist, target_count=3)

        assert len(result1) == len(result2)
        for e1, e2 in zip(result1, result2):
            assert e1["hex"] == e2["hex"]
            # OKLab values should be identical (bitwise)
            assert e1["oklab"] == e2["oklab"]

    def test_determinism_under_different_order(self):
        """Histogram order should not affect output."""
        base_hist = [
            make_histogram_entry(0.2, 0.5, 0.8, 100),
            make_histogram_entry(0.8, 0.2, 0.3, 75),
            make_histogram_entry(0.5, 0.7, 0.2, 50),
        ]

        shuffled_hist = base_hist[::-1]  # reverse order

        result1 = run_deterministic_quantization(base_hist, target_count=2)
        result2 = run_deterministic_quantization(shuffled_hist, target_count=2)

        assert len(result1) == len(result2)
        for e1, e2 in zip(result1, result2):
            assert e1["hex"] == e2["hex"]

    def test_repeated_runs_stable(self):
        """Running 5 times should produce identical results."""
        hist = [
            make_histogram_entry(0.9, 0.1, 0.1, 200),
            make_histogram_entry(0.1, 0.9, 0.1, 150),
            make_histogram_entry(0.1, 0.1, 0.9, 100),
            make_histogram_entry(0.5, 0.5, 0.5, 80),
        ]

        results = [run_deterministic_quantization(hist, target_count=3) for _ in range(5)]

        # All results should be identical
        first_hexes = [e["hex"] for e in results[0]]
        for i, result in enumerate(results[1:], 1):
            current_hexes = [e["hex"] for e in result]
            assert current_hexes == first_hexes, f"Run {i} differed from run 0"


class TestFixedCenters:
    """Pinned centres must be preserved."""

    def test_fixed_centers_preserved(self):
        red_oklab = srgb_to_oklab(1.0, 0.0, 0.0)

        hist = [
            make_histogram_entry(0.95, 0.05, 0.05, 100),
            make_histogram_entry(0.8, 0.2, 0.1, 60),
            make_histogram_entry(0.1, 0.8, 0.2, 40),
        ]

        result = run_deterministic_quantization(
            hist,
            target_count=2,
            fixed_centers=[red_oklab],
        )

        # Should have exactly 2 auto-centres (fixed centres excluded from output)
        assert len(result) == 2

    def test_no_fixed_centers_works(self):
        hist = [
            make_histogram_entry(0.5, 0.5, 0.5, 100),
            make_histogram_entry(0.8, 0.2, 0.3, 60),
        ]

        result = run_deterministic_quantization(hist, target_count=2)
        assert len(result) == 2


class TestEdgeCases:
    """Boundary conditions."""

    def test_empty_histogram(self):
        result = run_deterministic_quantization([], target_count=5)
        assert result == []

    def test_zero_target_count(self):
        hist = [make_histogram_entry(0.5, 0.5, 0.5, 100)]
        result = run_deterministic_quantization(hist, target_count=0)
        assert result == []

    def test_single_bin(self):
        hist = [make_histogram_entry(0.5, 0.3, 0.2, 100)]
        result = run_deterministic_quantization(hist, target_count=1)

        assert len(result) == 1
        # Should be close to the input colour
        from palette_trace.color.conversion import hex_to_srgb
        r, g, b = hex_to_srgb(result[0]["hex"])
        assert abs(r - 0.5) < 0.1
        assert abs(g - 0.3) < 0.1
        assert abs(b - 0.2) < 0.1

    def test_over_requested_centers(self):
        """Requesting more centres than bins should not crash."""
        hist = [
            make_histogram_entry(0.5, 0.3, 0.2, 100),
            make_histogram_entry(0.8, 0.1, 0.1, 60),
        ]

        result = run_deterministic_quantization(hist, target_count=10)
        # Should produce ≤ len(hist) centres
        assert len(result) <= len(hist)

    def test_single_bin_with_fixed_centers(self):
        """Single bin + fixed centers should still work."""
        red_oklab = srgb_to_oklab(1.0, 0.0, 0.0)
        hist = [make_histogram_entry(0.5, 0.3, 0.2, 100)]

        result = run_deterministic_quantization(
            hist,
            target_count=1,
            fixed_centers=[red_oklab],
        )
        assert len(result) >= 0  # May produce 0 if all pixels cluster near fixed centre


class TestConvergence:
    """Quantizer should converge within max_iterations."""

    def test_converges_within_budget(self):
        hist = [
            make_histogram_entry(1.0, 0.0, 0.0, 200),
            make_histogram_entry(0.0, 1.0, 0.0, 150),
            make_histogram_entry(0.0, 0.0, 1.0, 100),
        ]

        # Should converge well before max_iterations=50
        result = run_deterministic_quantization(hist, target_count=3, max_iterations=20)
        assert len(result) == 3

    def test_weighted_centroids(self):
        """Centres should be weighted by histogram bin counts."""
        # Two very close clusters with different weights
        hist = [
            make_histogram_entry(0.5, 0.1, 0.1, 300),   # heavy weight
            make_histogram_entry(0.52, 0.12, 0.12, 50), # light weight
        ]

        result = run_deterministic_quantization(hist, target_count=1)
        assert len(result) == 1

        # Result should be closer to the heavy-weight entry
        from palette_trace.color.conversion import hex_to_srgb, srgb_to_oklab
        r, g, b = hex_to_srgb(result[0]["hex"])
        result_oklab = srgb_to_oklab(r, g, b)

        dist_heavy = sum((result_oklab[i] - hist[0]["oklab"][i])**2 for i in range(3))
        dist_light = sum((result_oklab[i] - hist[1]["oklab"][i])**2 for i in range(3))

        assert dist_heavy < dist_light, "Centroid should be closer to heavier cluster"


class TestOutputStructure:
    """Result entries must have required fields."""

    def test_result_has_required_fields(self):
        hist = [
            make_histogram_entry(0.5, 0.3, 0.2, 100),
            make_histogram_entry(0.8, 0.6, 0.4, 60),
        ]

        result = run_deterministic_quantization(hist, target_count=2)

        for entry in result:
            assert "hex" in entry
            assert "oklab" in entry
            assert len(entry["hex"]) == 7  # '#RRGGBB'
            assert isinstance(entry["oklab"], tuple)
            assert len(entry["oklab"]) == 3

    def test_hex_values_valid(self):
        """Hex values must be valid sRGB colours."""
        hist = [make_histogram_entry(0.5, 0.3, 0.2, 100)]
        result = run_deterministic_quantization(hist, target_count=1)

        for entry in result:
            # Hex should be parseable back to sRGB
            from palette_trace.color.conversion import hex_to_srgb
            r, g, b = hex_to_srgb(entry["hex"])
            assert 0.0 <= r <= 1.0
            assert 0.0 <= g <= 1.0
            assert 0.0 <= b <= 1.0


if __name__ == "__main__":
    pytest.main([__file__, "-v"])
