"""
Tracing-backend conformance runner (SPEC §23.2, §23.6, §33.2).

Two tiers of check are evaluated, and the distinction matters:

**Mandatory** checks encode the `MUST` responsibilities of SPEC §23.2. A backend
that fails one is not a legitimate backend and must not be registered.

**Quality** checks encode the §23.6 evaluation criteria used to *choose* a
reference backend. Failing one does not make a backend invalid — it makes it
unsuitable as the reference. `python_contour`, for instance, is a correctness
fallback that is expected to fail the node-count budgets.

Thresholds are numeric and derived from measured behaviour of the candidate
engines, with margin. `tests/conformance/test_backend_conformance.py` asserts on
these results; `python -m palette_trace.tracing.conformance.runner` prints them.
"""

import argparse
import json
import re
import sys
import time
from dataclasses import dataclass, field
from typing import Callable, Optional

from palette_trace.tracing.conformance import fixtures
from palette_trace.tracing.protocol import TraceBackend, TraceResult
from palette_trace.tracing.registry import BackendRegistry

# --------------------------------------------------------------------------- #
#  Thresholds                                                                  #
# --------------------------------------------------------------------------- #

#: A solid rectangle is four corners. Anything above this is node explosion.
MAX_NODES_SOLID_RECTANGLE = 32

#: A donut needs an outer boundary and a hole boundary.
MIN_SUBPATHS_DONUT = 2

#: A plus has twelve corners; a backend that collapses it to a bounding box
#: would emit four or five nodes.
MIN_NODES_PLUS = 8

#: A plus that is not being curve-fitted at all would emit one node per boundary
#: pixel.
MAX_NODES_PLUS = 64

#: A disc should be curve-fitted, not emitted as a polygon.
MAX_NODES_CIRCLE = 40

#: An isolated pixel must not become geometry.
MAX_PATHS_SINGLE_PIXEL = 0

#: Four scattered pixels may merge, but must stay minimal.
MAX_PATHS_SPARSE_NOISE = 2

#: A one-pixel diagonal stroke is a legitimate thin feature, not noise.
MIN_PATHS_DIAGONAL_LINE = 1

#: Node budget for a 512x512 annulus.
MAX_NODES_LARGE_DONUT = 256

#: Wall-clock ceiling for one 512x512 mask, in seconds. Generous — this catches
#: pathological behaviour, not slow-but-usable engines.
MAX_SECONDS_LARGE_DONUT = 5.0

_COMMAND_RE = re.compile(r"[MLCSQTAHVZmlcsqtahvz]")
_SUBPATH_RE = re.compile(r"[Mm]")
_NUMBER_RE = re.compile(r"-?\d+(?:\.\d+)?(?:[eE][-+]?\d+)?")


def count_nodes(paths) -> int:
    """Total SVG path commands across every path."""
    return sum(len(_COMMAND_RE.findall(p)) for p in paths)


def count_subpaths(paths) -> int:
    """Total moveto commands — one per closed boundary, holes included."""
    return sum(len(_SUBPATH_RE.findall(p)) for p in paths)


# --------------------------------------------------------------------------- #
#  Results                                                                     #
# --------------------------------------------------------------------------- #

@dataclass
class CheckResult:
    name: str
    tier: str          # "mandatory" | "quality"
    passed: bool
    detail: str = ""
    measured: dict = field(default_factory=dict)


@dataclass
class ConformanceReport:
    backend_id: str
    version: str
    checks: list = field(default_factory=list)
    error: Optional[str] = None

    def _by_tier(self, tier: str):
        return [c for c in self.checks if c.tier == tier]

    @property
    def mandatory_passed(self) -> bool:
        return self.error is None and all(c.passed for c in self._by_tier("mandatory"))

    @property
    def quality_passed(self) -> bool:
        return self.error is None and all(c.passed for c in self._by_tier("quality"))

    @property
    def passed(self) -> bool:
        return self.mandatory_passed and self.quality_passed

    def get(self, name: str) -> CheckResult:
        for c in self.checks:
            if c.name == name:
                return c
        raise KeyError(f"No conformance check named {name!r}")

    def to_dict(self) -> dict:
        return {
            "backendId": self.backend_id,
            "version": self.version,
            "error": self.error,
            "mandatoryPassed": self.mandatory_passed,
            "qualityPassed": self.quality_passed,
            "checks": [
                {
                    "name": c.name,
                    "tier": c.tier,
                    "passed": c.passed,
                    "detail": c.detail,
                    "measured": c.measured,
                }
                for c in self.checks
            ],
        }


# --------------------------------------------------------------------------- #
#  Evaluation                                                                  #
# --------------------------------------------------------------------------- #

def _trace(backend: TraceBackend, fixture_fn: Callable, profile=None):
    """Traces one fixture, returning (result, elapsed_seconds)."""
    request = fixtures.as_request(fixture_fn(), profile)
    started = time.perf_counter()
    result = backend.trace_mask(request)
    return result, time.perf_counter() - started


def _paths_are_structurally_valid(result: TraceResult) -> tuple[bool, str]:
    for index, path in enumerate(result.svg_path_data):
        if not isinstance(path, str):
            return False, f"path {index} is {type(path).__name__}, not str"
        if not path.strip():
            return False, f"path {index} is empty"
        if not _COMMAND_RE.search(path):
            return False, f"path {index} contains no SVG command"
        for token in _NUMBER_RE.findall(path):
            value = float(token)
            if value != value or value in (float("inf"), float("-inf")):
                return False, f"path {index} contains non-finite coordinate {token}"
    if result.fill_rule not in ("nonzero", "evenodd"):
        return False, f"fill_rule {result.fill_rule!r} is not a legal SVG fill-rule"
    return True, f"{len(result.svg_path_data)} path(s), fill-rule {result.fill_rule}"


def run_conformance_suite(backend: TraceBackend) -> ConformanceReport:
    """Evaluates one backend against every mandatory and quality check."""
    caps = backend.capabilities()
    report = ConformanceReport(backend_id=caps.backend_id, version=caps.version)

    def record(name, tier, passed, detail="", **measured):
        report.checks.append(CheckResult(name, tier, bool(passed), detail, measured))

    # -- mandatory: SPEC §23.2 ---------------------------------------------- #

    for required in (
        "backend_id", "version", "supports_binary_masks", "supports_holes",
        "supports_cancellation", "deterministic", "supported_canonical_settings",
    ):
        if not hasattr(caps, required):
            record("capabilities_reporting", "mandatory", False,
                   f"missing capability field {required!r}")
            break
    else:
        ok = bool(caps.backend_id) and bool(caps.version)
        record("capabilities_reporting", "mandatory", ok,
               f"id={caps.backend_id}, version={caps.version}")

    try:
        rect, rect_seconds = _trace(backend, fixtures.solid_rectangle)
    except Exception as exc:                                    # noqa: BLE001
        report.error = f"{type(exc).__name__}: {exc}"
        record("binary_mask_input", "mandatory", False, report.error)
        return report

    record("binary_mask_input", "mandatory", len(rect.svg_path_data) >= 1,
           f"{len(rect.svg_path_data)} path(s) for a solid rectangle",
           pathCount=len(rect.svg_path_data), seconds=round(rect_seconds, 4))

    valid, detail = _paths_are_structurally_valid(rect)
    record("path_data_structure", "mandatory", valid, detail)

    donut_result, _ = _trace(backend, fixtures.donut)
    donut_subpaths = count_subpaths(donut_result.svg_path_data)
    record("hole_preservation", "mandatory", donut_subpaths >= MIN_SUBPATHS_DONUT,
           f"{donut_subpaths} subpath(s), need >= {MIN_SUBPATHS_DONUT} "
           f"(fill-rule {donut_result.fill_rule})",
           subpaths=donut_subpaths, fillRule=donut_result.fill_rule)

    repeat_result, _ = _trace(backend, fixtures.donut)
    identical = donut_result.svg_path_data == repeat_result.svg_path_data
    record("determinism", "mandatory", identical,
           "identical across runs" if identical else "output differs between runs")

    # §23.1 fixes the signature: every backend accepts a cancellation token,
    # whether or not it acts on one.
    try:
        backend.trace_mask(fixtures.as_request(fixtures.solid_rectangle()), None)
        record("cancellation_signature", "mandatory", True,
               f"accepts a cancellation token (honoured: {caps.supports_cancellation})")
    except TypeError as exc:
        record("cancellation_signature", "mandatory", False,
               f"rejects the cancellation-token argument: {exc}")

    large_result, large_seconds = _trace(backend, fixtures.large_donut)
    record("large_mask", "mandatory", len(large_result.svg_path_data) >= 1,
           f"{len(large_result.svg_path_data)} path(s) from 512x512 in {large_seconds:.3f}s",
           pathCount=len(large_result.svg_path_data), seconds=round(large_seconds, 4))

    # -- quality: SPEC §23.6 ------------------------------------------------ #

    rect_nodes = count_nodes(rect.svg_path_data)
    record("node_budget_solid_rectangle", "quality", rect_nodes <= MAX_NODES_SOLID_RECTANGLE,
           f"{rect_nodes} nodes, budget {MAX_NODES_SOLID_RECTANGLE}", nodes=rect_nodes)

    plus_result, _ = _trace(backend, fixtures.plus)
    plus_nodes = count_nodes(plus_result.svg_path_data)
    record("sharp_corners", "quality", MIN_NODES_PLUS <= plus_nodes <= MAX_NODES_PLUS,
           f"{plus_nodes} nodes for a 12-corner plus, want "
           f"{MIN_NODES_PLUS}..{MAX_NODES_PLUS}", nodes=plus_nodes)

    circle_result, _ = _trace(backend, fixtures.circle)
    circle_nodes = count_nodes(circle_result.svg_path_data)
    record("smooth_curves", "quality", circle_nodes <= MAX_NODES_CIRCLE,
           f"{circle_nodes} nodes for a disc, budget {MAX_NODES_CIRCLE}", nodes=circle_nodes)

    pixel_result, _ = _trace(backend, fixtures.single_pixel)
    record("noise_suppression", "quality",
           len(pixel_result.svg_path_data) <= MAX_PATHS_SINGLE_PIXEL,
           f"{len(pixel_result.svg_path_data)} path(s) for one isolated pixel, "
           f"budget {MAX_PATHS_SINGLE_PIXEL}", pathCount=len(pixel_result.svg_path_data))

    sparse_result, _ = _trace(backend, fixtures.sparse_noise)
    record("sparse_noise_suppression", "quality",
           len(sparse_result.svg_path_data) <= MAX_PATHS_SPARSE_NOISE,
           f"{len(sparse_result.svg_path_data)} path(s) for four scattered pixels, "
           f"budget {MAX_PATHS_SPARSE_NOISE}", pathCount=len(sparse_result.svg_path_data))

    diagonal_result, _ = _trace(backend, fixtures.diagonal_line)
    record("thin_feature_retention", "quality",
           len(diagonal_result.svg_path_data) >= MIN_PATHS_DIAGONAL_LINE,
           f"{len(diagonal_result.svg_path_data)} path(s) for a 1px diagonal, "
           f"need >= {MIN_PATHS_DIAGONAL_LINE}", pathCount=len(diagonal_result.svg_path_data))

    large_nodes = count_nodes(large_result.svg_path_data)
    record("node_budget_large_mask", "quality", large_nodes <= MAX_NODES_LARGE_DONUT,
           f"{large_nodes} nodes, budget {MAX_NODES_LARGE_DONUT}", nodes=large_nodes)

    record("runtime_large_mask", "quality", large_seconds <= MAX_SECONDS_LARGE_DONUT,
           f"{large_seconds:.3f}s for 512x512, ceiling {MAX_SECONDS_LARGE_DONUT}s",
           seconds=round(large_seconds, 4))

    return report


def evaluate_all(registry: Optional[BackendRegistry] = None) -> list:
    """Runs the suite against every discovered backend."""
    registry = registry or BackendRegistry()
    reports = []
    for info in registry.list_available_backends():
        backend_id = info["id"]
        try:
            reports.append(run_conformance_suite(registry.get_backend(backend_id)))
        except Exception as exc:                                # noqa: BLE001
            reports.append(ConformanceReport(
                backend_id=backend_id,
                version=info.get("version", "?"),
                error=f"{type(exc).__name__}: {exc}",
            ))
    return reports


# --------------------------------------------------------------------------- #
#  CLI                                                                         #
# --------------------------------------------------------------------------- #

def _print_report(report: ConformanceReport) -> None:
    if report.error and not report.checks:
        print(f"\n{report.backend_id} ({report.version}): ERROR — {report.error}")
        return

    verdict = "REFERENCE-ELIGIBLE" if report.passed else (
        "VALID (quality checks failed)" if report.mandatory_passed else "INVALID"
    )
    print(f"\n{report.backend_id} ({report.version}): {verdict}")
    for tier in ("mandatory", "quality"):
        print(f"  {tier}:")
        for check in report.checks:
            if check.tier != tier:
                continue
            print(f"    [{'PASS' if check.passed else 'FAIL'}] {check.name}: {check.detail}")


def main(argv=None) -> int:
    parser = argparse.ArgumentParser(description="Run tracing-backend conformance checks.")
    parser.add_argument("--json", action="store_true", help="emit machine-readable JSON")
    parser.add_argument("--backend", help="evaluate only this backend id")
    args = parser.parse_args(argv)

    registry = BackendRegistry()
    if args.backend:
        reports = [run_conformance_suite(registry.get_backend(args.backend))]
    else:
        reports = evaluate_all(registry)

    if args.json:
        json.dump([r.to_dict() for r in reports], sys.stdout, indent=2)
        print()
    else:
        print(f"Discovered {len(reports)} tracing backend(s).")
        for report in reports:
            _print_report(report)
        eligible = [r.backend_id for r in reports if r.passed]
        print(f"\nReference-eligible: {', '.join(eligible) if eligible else 'none'}")

    return 0 if any(r.passed for r in reports) else 1


if __name__ == "__main__":
    sys.exit(main())
