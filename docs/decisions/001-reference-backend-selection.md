"""
Technical Decision Record: Reference Tracing Backend Selection

Date: 2024-01-XX
Status: Proposed
Section: §36 Phase 0 Gate Criteria

---

## Context

Palette Trace requires a binary-mask vectorization backend that:
1. Produces deterministic output under identical inputs (§23.2)
2. Preserves holes in mask regions (§23.2)
3. Suppresses single-pixel noise (§25)
4. Supports per-scan profile configuration (corner sensitivity, smoothing) (§18)
5. Is available as a Python library without external CLI dependencies

## Candidates Evaluated

| Criterion | VTracer | Potrace | Python Contour |
|-----------|---------|---------|----------------|
| Deterministic | ✓ (seeded) | ✓ | ✓ |
| Hole support | ✓ (fill-rule) | ✓ | ✗ |
| Noise suppression | ✓ (configurable minArea) | ✓ (-t/-s params) | Partial |
| Profile configurability | ✓ (cornerSensitivity, curveSmoothing) | Limited | Minimal |
| Pure Python | ✓ (vtracer>=0.6.0) | ✗ (C binary required) | ✓ |
| License | GPL-3.0 | GPL-2.0 | MIT |
| Install complexity | pip install | system package | pip install |

## Decision

**VTracer is selected as the reference backend.**

### Rationale

1. **Determinism guarantee**: VTracer's `convert_image_to_svg_py` produces bit-identical output for identical inputs when called with the same parameters. This satisfies §23.2 and enables golden-file testing.

2. **Hole preservation**: VTracer correctly handles donut/shell geometries via SVG fill-rule (`nonzero`). Conformance test `test_donut_hole_preservation` validates this.

3. **Noise suppression**: VTracer's `minArea` parameter (configurable per profile in §18) allows aggressive noise filtering without manual pre-processing. Single-pixel noise is suppressed when `minArea >= 4`.

4. **Profile configurability**: VTracer exposes the three canonical settings required by §23.1:
   - `cornerSensitivity` (0–1): controls corner detection aggressiveness
   - `curveSmoothing` (0–1): balances smoothness vs fidelity  
   - `optimization` (int): post-processing simplification level

5. **Installation portability**: Pure Python package via pip, no system-level dependencies. This is critical for cross-platform Inkscape extension distribution (§37).

6. **License compatibility**: GPL-3.0 aligns with the project's own license.

### Risks and Mitigations

| Risk | Severity | Mitigation |
|------|----------|------------|
| VTracer not installed on user system | Medium | Fallback to Potrace in registry (§24). Error message guides user to install. |
| GPL-3.0 copyleft concerns | Low | Project is already GPL-3.0; no conflict. |
| Performance on large images | Low | Lazy mask generation (§25) limits memory; VTracer processes one mask at a time. |

## Conformance Thresholds (Phase 0 Gate)

The following numeric thresholds define "acceptable path quality" for Phase 0 sign-off:

| Test | Threshold | Rationale |
|------|-----------|----------|
| Solid rectangle node count | ≤ 20 paths | Simple geometry should not explode |
| Donut hole preservation | ≥ 1 path produced | Holes must be represented |
| Determinism | Exact tuple match (tolerance = 0.0) | §23.2 requirement |
| Single-pixel noise suppression | ≤ 0 paths | Isolated pixels must not become vectors |
| Sparse noise (≤4 pixels) | ≤ 2 paths | Minimal output for scattered noise |

## Verification

Run conformance suite:
```bash
python -m pytest tests/conformance/test_backend_conformance.py -v
```

All seven test categories must pass for the reference backend before Phase 1 begins.

## Alternatives Considered

- **Potrace as primary**: Rejected due to C binary dependency complicating distribution. Kept as fallback.
- **Python Contour as primary**: Rejected due to lack of hole support and minimal configurability.
- **Multi-backend voting**: Rejected for Phase 0 complexity; reserved as future optimization (§37).

## Approvals

Pending conformance test results from Phase 0 execution.
"