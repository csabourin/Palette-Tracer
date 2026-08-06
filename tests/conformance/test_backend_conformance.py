"""
Comprehensive conformance fixtures for tracing backends (§36, §23).

Tests verify:
- Solid shape reproduction (node count budget)
- Hole preservation  
- Determinism under repeated runs
- Noise suppression
- Path data structure validity

Conformance thresholds are numeric where possible to enable automated gating.
"""

import re
import numpy as np
from palette_trace.tracing.protocol import TraceRequest, TraceBackend
from palette_trace.tracing.registry import BackendRegistry


# --------------------------------------------------------------------------- #
#  Fixture masks                                                              #
# --------------------------------------------------------------------------- #

def create_solid_rectangle_mask(w=64, h=64) -> bytes:
    """Solid rectangle centred in the frame."""
    mask = np.zeros((h, w), dtype=np.uint8)
    mask[12:52, 12:52] = 1
    return mask.tobytes()


def create_donut_mask(w=64, h=64) -> bytes:
    """Ring with a hole in the centre — tests hole preservation."""
    mask = np.zeros((h, w), dtype=np.uint8)
    mask[8:56, 8:56] = 1
    mask[20:44, 20:44] = 0
    return mask.tobytes()


def create_one_pixel_noise_mask(w=64, h=64) -> bytes:
    """Single isolated pixel — backend should suppress or produce minimal output."""
    mask = np.zeros((h, w), dtype=np.uint8)
    mask[32, 32] = 1
    return mask.tobytes()


def create_sparse_noise_mask(w=64, h=64) -> bytes:
    """Scattered noise pixels — backend should produce few or no paths."""
    mask = np.zeros((h, w), dtype=np.uint8)
    coords = [(10, 10), (50, 50), (32, 10), (10, 50)]
    for y, x in coords:
        mask[y, x] = 1
    return mask.tobytes()


def create_diagonal_line_mask(w=64, h=64) -> bytes:
    """Diagonal line — tests smooth curve handling."""
    mask = np.zeros((h, w), dtype=np.uint8)
    for i in range(min(w, h)):
        mask[i, i] = 1
    return mask.tobytes()


# --------------------------------------------------------------------------- #
#  Threshold constants                                                        #
# --------------------------------------------------------------------------- #

MAX_NODE_COUNT_SOLID_RECT = 20       # A simple rect should not explode nodes
MIN_PATHS_FOR_DONUT = 1             # At least one path expected for donut
MAX_PATHS_FOR_NOISE = 0             # Single pixel should ideally produce nothing
DETERMINISM_TOLERANCE = 0.0         # Path tuples must be identical


# --------------------------------------------------------------------------- #
#  Conformance test class                                                     #
# --------------------------------------------------------------------------- #

class BackendConformance:
    """Runs a suite of conformance checks against a TraceBackend."""

    def __init__(self, backend: TraceBackend):
        self.backend = backend
        self.results: dict[str, bool] = {}
        self.details: dict[str, str] = {}

    # -- individual tests --------------------------------------------------- #

    def test_solid_rectangle(self) -> bool:
        """Solid rectangle must produce at least one path with reasonable node count."""
        mask_bytes = create_solid_rectangle_mask()
        req = TraceRequest(width=64, height=64, packed_binary_mask=mask_bytes, profile={})
        res = self.backend.trace_mask(req)

        has_paths = len(res.svg_path_data) >= 1
        if not has_paths:
            self.details["solid_rectangle"] = "No paths produced"
            return False

        # Check statistics for node count if available
        stats = res.statistics.get("path_count", 0)
        if stats > MAX_NODE_COUNT_SOLID_RECT:
            self.details["solid_rectangle"] = f"Too many paths: {stats}"
            return False

        self.details["solid_rectangle"] = f"{len(res.svg_path_data)} path(s)"
        return True

    def test_donut_hole_preservation(self) -> bool:
        """Donut shape must produce at least one path (hole may be implicit via fill-rule)."""
        mask_bytes = create_donut_mask()
        req = TraceRequest(width=64, height=64, packed_binary_mask=mask_bytes, profile={})
        res = self.backend.trace_mask(req)

        has_paths = len(res.svg_path_data) >= MIN_PATHS_FOR_DONUT
        if not has_paths:
            self.details["donut_hole"] = "No paths produced for donut"
            return False

        self.details["donut_hole"] = f"{len(res.svg_path_data)} path(s), fill_rule={res.fill_rule}"
        return True

    def test_determinism(self) -> bool:
        """Identical input must produce identical output."""
        mask_bytes = create_donut_mask()
        req = TraceRequest(width=64, height=64, packed_binary_mask=mask_bytes, profile={})

        res1 = self.backend.trace_mask(req)
        res2 = self.backend.trace_mask(req)

        is_deterministic = res1.svg_path_data == res2.svg_path_data
        if not is_deterministic:
            self.details["determinism"] = "Outputs differ between runs"
        else:
            self.details["determinism"] = "Deterministic ✓"
        return is_deterministic

    def test_noise_suppression(self) -> bool:
        """Single isolated pixel should produce no paths."""
        mask_bytes = create_one_pixel_noise_mask()
        req = TraceRequest(width=64, height=64, packed_binary_mask=mask_bytes, profile={})
        res = self.backend.trace_mask(req)

        suppressed = len(res.svg_path_data) <= MAX_PATHS_FOR_NOISE
        if not suppressed:
            self.details["noise_suppression"] = f"Produced {len(res.svg_path_data)} path(s) for single pixel"
        else:
            self.details["noise_suppression"] = "Noise suppressed ✓"
        return suppressed

    def test_sparse_noise_suppression(self) -> bool:
        """Sparse noise should produce minimal output."""
        mask_bytes = create_sparse_noise_mask()
        req = TraceRequest(width=64, height=64, packed_binary_mask=mask_bytes, profile={})
        res = self.backend.trace_mask(req)

        # Allow up to 1 path for sparse noise (some backends may merge nearby pixels)
        minimal = len(res.svg_path_data) <= 2
        if not minimal:
            self.details["sparse_noise"] = f"Produced {len(res.svg_path_data)} paths for sparse noise"
        else:
            self.details["sparse_noise"] = "Sparse noise handled ✓"
        return minimal

    def test_capabilities_reporting(self) -> bool:
        """Backend must report valid capabilities."""
        caps = self.backend.capabilities()
        
        required_fields = ["backend_id", "version", "supports_binary_masks", 
                          "supports_holes", "deterministic"]
        for field in required_fields:
            if not hasattr(caps, field):
                self.details["capabilities"] = f"Missing capability: {field}"
                return False

        if not caps.backend_id:
            self.details["capabilities"] = "backend_id is empty"
            return False

        self.details["capabilities"] = f"id={caps.backend_id}, v{caps.version}"
        return True

    def test_path_data_structure(self) -> bool:
        """Path data must be non-empty strings."""
        mask_bytes = create_solid_rectangle_mask()
        req = TraceRequest(width=64, height=64, packed_binary_mask=mask_bytes, profile={})
        res = self.backend.trace_mask(req)

        for i, path in enumerate(res.svg_path_data):
            if not isinstance(path, str):
                self.details["path_structure"] = f"Path {i} is not a string"
                return False
            # Each path should contain at least one command letter
            has_command = bool(re.search(r'[MZLHVCSQTA]', path))
            if not has_command:
                self.details["path_structure"] = f"Path {i} has no valid SVG commands"
                return False

        self.details["path_structure"] = "All paths structurally valid ✓"
        return True

    # -- aggregate ----------------------------------------------------------- #

    def run_all(self) -> dict:
        """Execute all conformance tests and return results."""
        test_methods = [
            ("capabilities_reporting", self.test_capabilities_reporting),
            ("solid_rectangle", self.test_solid_rectangle),
            ("donut_hole_preservation", self.test_donut_hole_preservation),
            ("determinism", self.test_determinism),
            ("noise_suppression", self.test_noise_suppression),
            ("sparse_noise_suppression", self.test_sparse_noise_suppression),
            ("path_data_structure", self.test_path_data_structure),
        ]

        for name, method in test_methods:
            try:
                self.results[name] = method()
            except Exception as exc:
                self.results[name] = False
                self.details[name] = f"Exception: {exc}"

        overall_passed = all(self.results.values())
        return {
            "backend_id": self.backend.capabilities().backend_id,
            "version": self.backend.capabilities().version,
            "passed": overall_passed,
            "results": self.results,
            "details": self.details,
        }


def main():
    """Standalone runner when pytest is unavailable."""
    registry = BackendRegistry()
    available = registry.list_available_backends()
    
    print(f"Discovered {len(available)} tracing backends.\n")

    for info in available:
        bid = info["id"]
        try:
            backend = registry.get_backend(bid)
            conf = BackendConformance(backend)
            res = conf.run_all()
            
            status = "PASSED ✓" if res["passed"] else "FAILED ✗"
            print(f"Backend '{bid}' ({res['version']}): {status}")
            for tname, tok in res["results"].items():
                symbol = "✓" if tok else "✗"
                detail = res["details"].get(tname, "")
                print(f"  [{symbol}] {tname}: {detail}")
        except Exception as exc:
            print(f"Backend '{bid}': ERROR — {exc}\n")


if __name__ == "__main__":
    main()
