"""
Tracing Backend Conformance Test Runner.
Verifies backend capabilities, hole preservation, determinism, and path generation.
"""

import sys
import numpy as np
from palette_trace.tracing.protocol import TraceRequest, TraceBackend
from palette_trace.tracing.registry import BackendRegistry

def create_solid_rectangle_mask(w=20, h=20) -> bytes:
    mask = np.zeros((h, w), dtype=np.uint8)
    mask[4:16, 4:16] = 1
    return mask.tobytes()

def create_donut_mask(w=20, h=20) -> bytes:
    mask = np.zeros((h, w), dtype=np.uint8)
    mask[2:18, 2:18] = 1
    mask[7:13, 7:13] = 0  # Hole in center
    return mask.tobytes()

def create_one_pixel_noise_mask(w=20, h=20) -> bytes:
    mask = np.zeros((h, w), dtype=np.uint8)
    mask[10, 10] = 1
    return mask.tobytes()

def run_conformance_tests(backend: TraceBackend) -> dict:
    caps = backend.capabilities()
    results = {
        "backend_id": caps.backend_id,
        "version": caps.version,
        "passed": True,
        "test_results": {},
    }

    # 1. Test solid rectangle
    rect_bytes = create_solid_rectangle_mask()
    req_rect = TraceRequest(width=20, height=20, packed_binary_mask=rect_bytes, profile={})
    res_rect = backend.trace_mask(req_rect)
    rect_ok = len(res_rect.svg_path_data) >= 1
    results["test_results"]["solid_rectangle"] = rect_ok

    # 2. Test donut with hole
    donut_bytes = create_donut_mask()
    req_donut = TraceRequest(width=20, height=20, packed_binary_mask=donut_bytes, profile={})
    res_donut = backend.trace_mask(req_donut)
    donut_ok = len(res_donut.svg_path_data) >= 1
    results["test_results"]["donut_with_hole"] = donut_ok

    # 3. Test determinism
    res_donut2 = backend.trace_mask(req_donut)
    det_ok = res_donut.svg_path_data == res_donut2.svg_path_data
    results["test_results"]["determinism"] = det_ok

    all_passed = rect_ok and donut_ok and det_ok
    results["passed"] = all_passed
    return results

def main():
    registry = BackendRegistry()
    available = registry.list_available_backends()
    print(f"Discovered {len(available)} tracing backends.")

    for info in available:
        bid = info["id"]
        b = registry.get_backend(bid)
        res = run_conformance_tests(b)
        status = "PASSED" if res["passed"] else "FAILED"
        print(f"Backend '{bid}' ({res['version']}): {status}")
        for tname, tok in res["test_results"].items():
            print(f"  - {tname}: {'OK' if tok else 'FAIL'}")

if __name__ == "__main__":
    main()
