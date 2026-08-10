#!/usr/bin/env python3
"""
Phase 0 gate checker (SPEC §36).

Phase 0 is the engine and colour-model spike. Its point is to produce *evidence*
that a backend is fit for purpose before a full UI is built on top of it, so this
script executes the conformance suite rather than checking that files exist.

An earlier version of this script reported the gate as met using only
`__import__` and `os.path.exists`, which passed while zero conformance checks
were running and only one backend was installed. Every check below must either
execute real work or assert on a measured result.

Usage:
    python3 scripts/check_phase0.py [--verbose]

Exit status is 0 when the gate is met, 1 otherwise.
"""

import argparse
import os
import re
import subprocess
import sys

REPO_ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
sys.path.insert(0, REPO_ROOT)


class Gate:
    def __init__(self, verbose: bool = False):
        self.results: list[tuple[str, bool, str]] = []
        self.verbose = verbose

    def record(self, name: str, passed: bool, detail: str = "") -> bool:
        self.results.append((name, bool(passed), detail))
        symbol = "PASS" if passed else "FAIL"
        print(f"  [{symbol}] {name}" + (f" — {detail}" if detail else ""))
        return bool(passed)


def check_backend_protocol(gate: Gate) -> None:
    """§23.1 — the protocol must declare every mandated field."""
    print("\n[1/8] Backend protocol (§23.1)")
    try:
        from palette_trace.tracing.protocol import (
            BackendCapabilities,
            TraceBackend,
            TraceRequest,
            TraceResult,
        )
    except ImportError as exc:
        gate.record("Backend protocol", False, str(exc))
        return

    expected = {
        BackendCapabilities: {
            "backend_id", "version", "supports_binary_masks", "supports_holes",
            "supports_cancellation", "deterministic", "supported_canonical_settings",
        },
        TraceRequest: {"width", "height", "packed_binary_mask", "profile", "transform_hint"},
        TraceResult: {"svg_path_data", "fill_rule", "warnings", "statistics"},
    }
    missing = []
    for cls, fields in expected.items():
        actual = set(getattr(cls, "__dataclass_fields__", {}))
        missing += [f"{cls.__name__}.{f}" for f in sorted(fields - actual)]

    has_methods = all(hasattr(TraceBackend, m) for m in ("capabilities", "trace_mask"))
    gate.record(
        "Backend protocol matches §23.1",
        not missing and has_methods,
        "all dataclass fields and protocol methods present" if not missing
        else f"missing {', '.join(missing)}",
    )


def check_conformance_executes(gate: Gate) -> None:
    """§33.2 — the conformance suite must actually run, not merely exist."""
    print("\n[2/8] Conformance suite executes (§33.2)")
    result = subprocess.run(
        [sys.executable, "-m", "pytest", "tests/conformance/", "-q"],
        cwd=REPO_ROOT, capture_output=True, text=True,
    )
    tail = (result.stdout or result.stderr).strip().splitlines()
    summary = tail[-1] if tail else "no output"
    collected_nothing = "no tests ran" in result.stdout or "collected 0 items" in result.stdout
    gate.record(
        "Conformance tests run under pytest",
        result.returncode == 0 and not collected_nothing,
        summary,
    )
    if gate.verbose and result.returncode != 0:
        print(result.stdout)


def check_backend_candidates(gate: Gate) -> None:
    """§23.6 — at least two candidates must be evaluated, not merely importable."""
    print("\n[3/8] Backend candidates evaluated (§23.6)")
    try:
        from palette_trace.tracing.conformance.runner import evaluate_all
    except ImportError as exc:
        gate.record("Backend candidates", False, str(exc))
        return

    reports = evaluate_all()
    ids = [r.backend_id for r in reports]
    gate.record(
        "At least two candidates evaluated",
        len(reports) >= 2,
        f"{len(reports)} evaluated: {', '.join(ids) if ids else 'none'}"
        + ("" if len(reports) >= 2 else " — run: pip install -e '.[backends]'"),
    )

    invalid = [r.backend_id for r in reports if not r.mandatory_passed]
    gate.record(
        "Every registered backend satisfies §23.2",
        not invalid,
        "all backends pass the mandatory checks" if not invalid
        else f"failing: {', '.join(invalid)}",
    )

    eligible = [r.backend_id for r in reports if r.passed]
    gate.record(
        "A backend passes the full quality suite",
        bool(eligible),
        f"reference-eligible: {', '.join(eligible)}" if eligible
        else "no backend passed every quality check",
    )


def check_reference_backend(gate: Gate) -> None:
    """The selected reference backend must be one that passes."""
    print("\n[4/8] Reference backend selection")
    try:
        from palette_trace.tracing.conformance.runner import run_conformance_suite
        from palette_trace.tracing.registry import REFERENCE_BACKEND_ID, BackendRegistry
    except ImportError as exc:
        gate.record("Reference backend", False, str(exc))
        return

    registry = BackendRegistry()
    try:
        report = run_conformance_suite(registry.get_backend(REFERENCE_BACKEND_ID))
    except Exception as exc:                                    # noqa: BLE001
        gate.record("Reference backend passes conformance", False,
                    f"{REFERENCE_BACKEND_ID}: {type(exc).__name__}: {exc}")
        return

    failed = [c.name for c in report.checks if not c.passed]
    gate.record(
        "Reference backend passes conformance",
        report.passed,
        f"{REFERENCE_BACKEND_ID} passes all checks" if report.passed
        else f"{REFERENCE_BACKEND_ID} failed: {', '.join(failed)}",
    )

    resolved = registry.get_backend("auto").capabilities().backend_id
    gate.record(
        "Automatic selection resolves to the reference backend",
        resolved == REFERENCE_BACKEND_ID,
        f"auto -> {resolved}",
    )


def check_colour_model(gate: Gate) -> None:
    """§21.1, §13.4 — OKLCH conversion must round-trip and behave at low chroma."""
    print("\n[5/8] Colour model (§21.1, §13.4)")
    try:
        from palette_trace.color.conversion import (
            hex_to_srgb,
            oklab_to_srgb,
            oklch_to_oklab,
            srgb_to_oklch,
        )
    except ImportError as exc:
        gate.record("OKLCH conversion", False, str(exc))
        return

    worst = 0.0
    for hex_value in ("#000000", "#ffffff", "#ff0000", "#00ff00", "#0000ff", "#7f7f7f", "#123456"):
        r, g, b = hex_to_srgb(hex_value)
        back = oklab_to_srgb(*oklch_to_oklab(*srgb_to_oklch(r, g, b)))
        worst = max(worst, max(abs(a - c) for a, c in zip((r, g, b), back)))

    gate.record("OKLCH round-trip within 1/255", worst < 1.0 / 255.0,
                f"worst channel error {worst:.6f}")


def check_colour_reach(gate: Gate) -> None:
    """§13.1 — reach mapping must be versioned and monotonic across its range."""
    print("\n[6/8] Colour reach (§13.1)")
    try:
        from palette_trace.color.reach import get_tolerances_for_reach
    except ImportError as exc:
        gate.record("Colour reach", False, str(exc))
        return

    tolerances = [get_tolerances_for_reach(v) for v in range(101)]
    monotonic = all(
        b["hue"] >= a["hue"] and b["chroma"] >= a["chroma"] and b["lightness"] >= a["lightness"]
        for a, b in zip(tolerances, tolerances[1:])
    )
    gate.record("Reach mapping is monotonic over 0..100", monotonic,
                f"reach 0 -> {tolerances[0]}, reach 100 -> {tolerances[100]}")


def check_unit_suite(gate: Gate) -> None:
    """§33.1 — claim resolution and quantizer determinism must be under test."""
    print("\n[7/8] Unit suite (§33.1)")
    result = subprocess.run(
        [sys.executable, "-m", "pytest", "tests/unit/", "tests/integration/", "-q"],
        cwd=REPO_ROOT, capture_output=True, text=True,
    )
    tail = (result.stdout or result.stderr).strip().splitlines()
    gate.record("Unit and integration tests pass", result.returncode == 0,
                tail[-1] if tail else "no output")
    if gate.verbose and result.returncode != 0:
        print(result.stdout)


def check_decision_record(gate: Gate) -> None:
    """§36 — the spike must end in a decision record that is no longer provisional."""
    print("\n[8/8] Technical decision record (§36)")
    path = os.path.join(REPO_ROOT, "docs", "decisions",
                        "ADR-0001-reference-tracing-backend.md")
    if not os.path.exists(path):
        gate.record("Decision record exists", False, f"missing {path}")
        return

    with open(path, encoding="utf-8") as handle:
        text = handle.read()

    gate.record("Decision record exists", True, os.path.relpath(path, REPO_ROOT))

    # Tolerates the markdown emphasis the record uses, e.g. "* **Status:** Accepted".
    accepted = re.search(r"status[:*\s]*[:\s]\s*\**\s*accepted\b", text, re.IGNORECASE)
    gate.record(
        "Decision record is accepted",
        bool(accepted),
        "status is Accepted" if accepted
        else "status is still provisional — record the measured conformance results",
    )


def main() -> int:
    parser = argparse.ArgumentParser(description="Check the Phase 0 gate (§36).")
    parser.add_argument("--verbose", action="store_true",
                        help="show full pytest output for failing steps")
    args = parser.parse_args()

    print("=" * 68)
    print("Phase 0 gate checker (§36) — engine and colour-model spike")
    print("=" * 68)

    gate = Gate(verbose=args.verbose)
    for step in (
        check_backend_protocol,
        check_conformance_executes,
        check_backend_candidates,
        check_reference_backend,
        check_colour_model,
        check_colour_reach,
        check_unit_suite,
        check_decision_record,
    ):
        step(gate)

    passed = sum(1 for _, ok, _ in gate.results if ok)
    total = len(gate.results)

    print("\n" + "=" * 68)
    print(f"SUMMARY — {passed}/{total} checks passed")
    print("=" * 68)
    for name, ok, detail in gate.results:
        print(f"  [{'PASS' if ok else 'FAIL'}] {name}")

    if passed == total:
        print("\nPhase 0 gate criteria MET — proceed to Phase 1.")
        return 0

    print(f"\nPhase 0 gate NOT met: {total - passed} check(s) failed.")
    return 1


if __name__ == "__main__":
    sys.exit(main())
