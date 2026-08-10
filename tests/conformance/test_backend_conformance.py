"""
Backend conformance tests (SPEC §23.2, §23.6, §33.2, §34.19).

The checks themselves live in `palette_trace.tracing.conformance.runner` so that
pytest and the command-line runner evaluate exactly the same thing. This module
is the assertion layer.

Structure mirrors the two tiers in the runner:

* Every discovered backend must pass every **mandatory** check — those are the
  `MUST` responsibilities of §23.2. A backend failing one should not be
  registered at all.
* The **quality** checks decide which backend is fit to be the reference
  backend (§23.6). Backends are allowed to fail these; at least one must pass
  them all, and the configured reference backend must be one that does.
"""

import pytest

from palette_trace.tracing.conformance.runner import (
    ConformanceReport,
    evaluate_all,
)
from palette_trace.tracing.registry import REFERENCE_BACKEND_ID, BackendRegistry

MANDATORY_CHECKS = [
    "capabilities_reporting",
    "binary_mask_input",
    "path_data_structure",
    "hole_preservation",
    "determinism",
    "cancellation_signature",
    "large_mask",
]

QUALITY_CHECKS = [
    "node_budget_solid_rectangle",
    "sharp_corners",
    "smooth_curves",
    "noise_suppression",
    "sparse_noise_suppression",
    "thin_feature_retention",
    "node_budget_large_mask",
    "runtime_large_mask",
]


# Evaluated once — tracing a 512x512 mask per backend is not free.
_REPORTS: dict[str, ConformanceReport] = {r.backend_id: r for r in evaluate_all()}
_BACKEND_IDS = sorted(_REPORTS)


@pytest.fixture(scope="session")
def reports() -> dict:
    return _REPORTS


# --------------------------------------------------------------------------- #
#  Discovery gates (SPEC §23.6, §34.19)                                        #
# --------------------------------------------------------------------------- #

def test_at_least_two_candidates_are_evaluated():
    """§23.6 requires the spike to evaluate at least two backend candidates."""
    assert len(_BACKEND_IDS) >= 2, (
        f"only {len(_BACKEND_IDS)} backend(s) discovered ({', '.join(_BACKEND_IDS) or 'none'}). "
        "Install the backend candidates with: pip install -e '.[backends]'"
    )


def test_a_portable_backend_passes_full_conformance():
    """§34.19: a portable or cross-platform backend passes conformance tests."""
    eligible = [bid for bid, r in _REPORTS.items() if r.passed]
    assert eligible, (
        "no backend passed every mandatory and quality check. "
        f"Results: { {bid: (r.mandatory_passed, r.quality_passed) for bid, r in _REPORTS.items()} }"
    )


def test_reference_backend_is_conformance_eligible():
    """The configured reference backend must be one that actually passes."""
    report = _REPORTS.get(REFERENCE_BACKEND_ID)
    assert report is not None, (
        f"reference backend {REFERENCE_BACKEND_ID!r} was not discovered; "
        f"available: {', '.join(_BACKEND_IDS) or 'none'}"
    )
    failed = [c.name for c in report.checks if not c.passed]
    assert report.passed, (
        f"reference backend {REFERENCE_BACKEND_ID!r} failed: {', '.join(failed)}"
    )


def test_registry_prefers_the_reference_backend():
    """§23.3/§23.4: automatic selection must resolve to the reference backend."""
    registry = BackendRegistry()
    assert registry.get_backend("auto").capabilities().backend_id == REFERENCE_BACKEND_ID


def test_explicit_backend_selection_is_honoured():
    """§23.3 item 1: an explicitly selected backend wins over the priority order."""
    registry = BackendRegistry()
    for backend_id in _BACKEND_IDS:
        assert registry.get_backend(backend_id).capabilities().backend_id == backend_id


# --------------------------------------------------------------------------- #
#  Mandatory checks — every backend (SPEC §23.2)                               #
# --------------------------------------------------------------------------- #

@pytest.mark.parametrize("backend_id", _BACKEND_IDS)
@pytest.mark.parametrize("check_name", MANDATORY_CHECKS)
def test_mandatory_check(backend_id, check_name, reports):
    report = reports[backend_id]
    assert report.error is None, f"{backend_id} raised during evaluation: {report.error}"
    check = report.get(check_name)
    assert check.passed, f"{backend_id} failed {check_name}: {check.detail}"


@pytest.mark.parametrize("backend_id", _BACKEND_IDS)
def test_every_check_was_evaluated(backend_id, reports):
    """Guards against a check silently disappearing from the runner."""
    evaluated = {c.name for c in reports[backend_id].checks}
    missing = set(MANDATORY_CHECKS + QUALITY_CHECKS) - evaluated
    assert not missing, f"{backend_id} did not evaluate: {', '.join(sorted(missing))}"


# --------------------------------------------------------------------------- #
#  Quality checks — reference backend only (SPEC §23.6)                        #
# --------------------------------------------------------------------------- #

@pytest.mark.parametrize("check_name", QUALITY_CHECKS)
def test_reference_backend_quality_check(check_name, reports):
    report = reports.get(REFERENCE_BACKEND_ID)
    if report is None:
        pytest.fail(f"reference backend {REFERENCE_BACKEND_ID!r} was not discovered")
    check = report.get(check_name)
    assert check.passed, f"{REFERENCE_BACKEND_ID} failed {check_name}: {check.detail}"


# --------------------------------------------------------------------------- #
#  Regression guards for defects found during the Phase 0 spike                #
# --------------------------------------------------------------------------- #

@pytest.mark.parametrize("backend_id", _BACKEND_IDS)
def test_solid_shape_does_not_produce_a_full_frame_path(backend_id):
    """
    Regression: PotraceAdapter passed the mask to potrace without inverting it.
    Potrace treats samples *below* blacklevel as foreground, so the background
    was traced and every scan gained a path covering the whole image.
    """

    from palette_trace.tracing.conformance import fixtures

    registry = BackendRegistry()
    backend = registry.get_backend(backend_id)

    mask = fixtures.solid_rectangle()
    height, width = mask.shape
    result = backend.trace_mask(fixtures.as_request(mask))

    numbers = []
    for path in result.svg_path_data:
        numbers.extend(float(t) for t in __import__("re").findall(
            r"-?\d+(?:\.\d+)?", path))
    assert numbers, f"{backend_id} produced no coordinates for a solid rectangle"

    # The fixture inset is a fifth of each edge, so a correctly traced rectangle
    # never reaches the frame corners.
    touches_frame = (
        min(numbers) <= 0.5
        and max(numbers) >= min(width, height) - 0.5
    )
    assert not touches_frame, (
        f"{backend_id} traced geometry spanning the full {width}x{height} frame for an "
        "inset rectangle — the mask polarity is probably inverted"
    )
