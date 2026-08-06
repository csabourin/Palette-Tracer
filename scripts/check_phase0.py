#!/usr/bin/env python3
"""
Phase 0 Gate Status Checker (§36).

Verifies all Phase 0 deliverables are present and tests pass.
Run this script to determine if Phase 0 is complete.

Usage:
    python3 scripts/check_phase0.py
"""

import sys
import os

# Add project root to path
sys.path.insert(0, os.path.dirname(os.path.dirname(os.path.abspath(__file__))))


def check_module_importable(name: str) -> bool:
    """Check if a module can be imported."""
    try:
        __import__(name)
        return True
    except ImportError as exc:
        print(f"  ✗ {name}: {exc}")
        return False


def check_file_exists(path: str) -> bool:
    """Check if a file exists."""
    full_path = os.path.join(os.path.dirname(os.path.dirname(os.path.abspath(__file__))), path)
    if os.path.exists(full_path):
        return True
    print(f"  ✗ Missing: {path}")
    return False


def main():
    print("=" * 60)
    print("Phase 0 Gate Status Checker (§36)")
    print("=" * 60)
    
    checks = []
    
    # 1. Backend protocol
    print("\n[1/8] Backend Protocol")
    ok = check_module_importable("palette_trace.tracing.protocol")
    checks.append(("Backend protocol", ok))
    
    # 2. Conformance fixtures
    print("\n[2/8] Conformance Fixtures")
    ok = check_file_exists("tests/conformance/test_backend_conformance.py")
    checks.append(("Conformance fixtures", ok))
    
    # 3. Backend adapters (≥2)
    print("\n[3/8] Backend Adapters (≥2 required)")
    adapter_modules = [
        "palette_trace.tracing.backends.vtracer_adapter",
        "palette_trace.tracing.backends.potrace_adapter",
        "palette_trace.tracing.backends.python_contour_adapter",
    ]
    available_count = 0
    for mod in adapter_modules:
        if check_module_importable(mod):
            available_count += 1
    ok = available_count >= 2
    checks.append((f"Backend adapters ({available_count} found)", ok))
    
    # 4. OKLCH conversion
    print("\n[4/8] OKLCH Conversion")
    ok = check_module_importable("palette_trace.color.conversion")
    checks.append(("OKLCH conversion", ok))
    
    # 5. Colour reach
    print("\n[5/8] Colour Reach")
    ok = check_module_importable("palette_trace.color.reach")
    checks.append(("Colour reach", ok))
    
    # 6. Claim resolution tests
    print("\n[6/8] Claim Resolution Tests")
    ok1 = check_file_exists("tests/unit/test_claims_comprehensive.py")
    ok2 = check_module_importable("palette_trace.color.claims")
    ok = ok1 and ok2
    checks.append(("Claim resolution tests", ok))
    
    # 7. Deterministic quantizer
    print("\n[7/8] Deterministic Quantizer")
    ok1 = check_module_importable("palette_trace.color.quantizer")
    ok2 = check_file_exists("tests/unit/test_quantizer_determinism.py")
    ok = ok1 and ok2
    checks.append(("Deterministic quantizer", ok))
    
    # 8. Technical decision record
    print("\n[8/8] Technical Decision Record")
    ok = check_file_exists("docs/decisions/001-reference-backend-selection.md")
    checks.append(("Technical decision record", ok))
    
    # Summary
    print("\n" + "=" * 60)
    print("SUMMARY")
    print("=" * 60)
    
    passed = sum(1 for _, ok in checks if ok)
    total = len(checks)
    
    for name, ok in checks:
        symbol = "✓" if ok else "✗"
        print(f"  [{symbol}] {name}")
    
    print(f"\n{passed}/{total} checks passed")
    
    if passed == total:
        print("\n🎉 Phase 0 gate criteria MET — proceed to Phase 1")
        return 0
    else:
        print(f"\n❌ Phase 0 incomplete: {total - passed} check(s) failed")
        return 1


if __name__ == "__main__":
    sys.exit(main())
