# ADR-0001 — Reference tracing backend

* **Status:** Accepted
* **Date:** 2026-08-07
* **Specification:** §23.2 Backend responsibilities, §23.4 Portable backend preference, §23.6 Engine-selection spike, §36 Phase 0
* **Supersedes:** the provisional draft that selected VTracer without executed evidence

## Decision

**Potrace, via the pure-Python `potracer` binding, is the reference tracing backend.**

`REFERENCE_BACKEND_ID` in `palette_trace/tracing/registry.py` is the single place this is encoded. VTracer is retained as the first alternative; `python_contour` is retained as a zero-dependency correctness fallback that is explicitly *not* reference-eligible.

## Context

§23.6 requires a conformance harness and evaluation of at least two candidates — one portable/WASM or Python, one CLI or native — before the full UI is built. §23.4 prefers a portable implementation: WebAssembly bundled with the interface, or a Python binding with maintained wheels. §35 forbids hard-coding one engine into the palette pipeline, so this decision is about which backend is the *default*, not which is the only one.

## Reproducing the evidence

```bash
python -m palette_trace.tracing.conformance.runner
```

Add `--json` for machine-readable output. The same checks run under `pytest tests/conformance/`.

Checks are split into two tiers. **Mandatory** checks encode the `MUST` list in §23.2 — a backend that fails one is not a legitimate backend. **Quality** checks encode the §23.6 evaluation criteria and decide reference eligibility; a backend may fail these and still ship as an alternative.

## Measured results

Measured 2026-08-07 on Linux (WSL2), Python 3.12.3. Fixtures are defined in `palette_trace/tracing/conformance/fixtures/`; thresholds and their rationale are in `palette_trace/tracing/conformance/runner.py`.

### Mandatory (§23.2)

| Check | Threshold | potrace 1.16 | vtracer 0.6.15 | python_contour 1.0.0 |
| ----- | --------- | ------------ | -------------- | -------------------- |
| Capabilities reporting | All §23.1 fields present | Pass | Pass | Pass |
| Binary-mask input | ≥ 1 path for a solid rectangle | Pass (1) | Pass (1) | Pass (1) |
| Path-data structure | Valid commands, finite coordinates, legal fill-rule | Pass | Pass | Pass |
| Hole preservation | ≥ 2 subpaths for a donut | Pass (2, evenodd) | Pass (2, nonzero) | Pass (5, evenodd) |
| Determinism | Byte-identical across runs | Pass | Pass | Pass |
| Cancellation signature | Accepts a cancellation token | Pass | Pass | Pass |
| Large mask | Completes on 512×512 | Pass | Pass | Pass |

All three candidates are legitimate backends.

### Quality (§23.6)

| Check | Threshold | potrace | vtracer | python_contour |
| ----- | --------- | ------: | ------: | -------------: |
| Node budget, solid rectangle | ≤ 32 nodes | **10** | **6** | 157 ✗ |
| Sharp corners (12-corner plus) | 8–64 nodes | **26** | **14** | 190 ✗ |
| Smooth curves (disc) | ≤ 40 nodes | **6** | **9** | 88 ✗ |
| Noise suppression (1 pixel) | 0 paths | **0** | **0** | **0** |
| Sparse noise (4 pixels) | ≤ 2 paths | **0** | **0** | **0** |
| Thin-feature retention (1px diagonal) | ≥ 1 path | **1** | 0 ✗ | 0 ✗ |
| Node budget, 512×512 annulus | ≤ 256 nodes | **25** | **28** | 1016 ✗ |
| Runtime, 512×512 annulus | ≤ 5.0 s | **0.037 s** | **0.018 s** | **0.165 s** |

**Only potrace passes every quality check.**

### Licence and distribution

| | potrace (`potracer`) | vtracer | python_contour |
| --- | --- | --- | --- |
| Licence | GPL-2.0-or-later | MIT | Project-internal |
| Compatible with GPL-3.0-or-later project | Yes (the "or later" clause permits GPL-3.0) | Yes | n/a |
| Implementation | Pure Python, `numpy` only — no compiled extension | Rust extension, prebuilt wheels | Pure Python |
| Windows / macOS / Linux | Yes — one pure-Python wheel | Yes, subject to a wheel existing per platform and Python version | Yes |
| Offline install | Yes | Yes | Yes (vendored) |

Verified: the installed `potracer` 0.0.4 package contains only `__init__.py` and `potrace.py`, with no `.so`, `.pyd` or `.dylib`.

## Rationale

1. **Thin-feature retention is the deciding criterion.** A one-pixel diagonal stroke is a legitimate feature in line art, hand-inked scans and logo work, and §17.5 requires thin features to survive cleanup. VTracer discards it entirely — 0 paths. Losing geometry silently is worse than emitting slightly more of it, and no downstream stage can recover a feature the backend never emitted.

2. **Potrace is the more portable candidate, contrary to the superseded draft.** That draft rejected Potrace for requiring a C binary. This is wrong for the `potracer` PyPI package, which is a pure-Python port depending only on `numpy`. Under §23.4 it is strictly more portable than VTracer, which ships a compiled Rust extension and therefore depends on a matching wheel for each platform and Python version.

3. **Quality is otherwise comparable.** VTracer produces marginally tighter node counts (6 versus 10 on a rectangle) and is about twice as fast on a 512×512 mask. Both are far inside budget; neither difference is material at the scale this tool operates on.

4. **Licence compatibility holds either way.** GPL-2.0-or-later permits relicensing under GPL-3.0, which the project already uses. VTracer's MIT licence would also have been compatible.

5. **The decision is cheap to revisit.** The pipeline resolves backends through `BackendRegistry` only; `REFERENCE_BACKEND_ID` is one constant, and the conformance suite re-runs in about a second.

## Consequences

* `potracer` moves from an optional extra to a runtime dependency, since the default path now depends on it.
* `PotraceAdapter` currently maps `cornerSensitivity`, `optimization` and `minimumPathAreaPx2` onto `alphamax`, `opttolerance` and `turdsize`. `curveSmoothing` is declared in its capabilities but not yet mapped — see the open issue below.
* `python_contour` guarantees the extension still produces output with no third-party tracer installed, at visibly lower quality. It must never be selected automatically while a conformance-eligible backend is present.

## Defect found during this spike

`PotraceAdapter.trace_mask` built its bitmap as `potrace.Bitmap(mask_arr > 0)`. Potrace treats samples **below** `blacklevel` (default 0.5) as foreground, so this traced the *background*: every mask produced a spurious path covering the entire image frame, and every traced scan would have been filled edge to edge.

Fixed by inverting the mask — `potrace.Bitmap(mask_arr == 0)`. Guarded by `test_solid_shape_does_not_produce_a_full_frame_path`, which runs against every registered backend.

This defect is the direct reason the superseded draft's conclusion was wrong: Potrace's measured output was polluted by the frame path, which inflated its path counts and hid its quality.

## Open issues

* `VTracerAdapter` declares support for `cornerSensitivity`, `curveSmoothing` and `optimization` but passes none of them to `vtracer.convert_image_to_svg_py`. Its capabilities currently overstate what it honours.
* `PotraceAdapter` does not map `curveSmoothing`.
* Both adapters hard-code their reported version string rather than reading it from the installed package.
* AutoTrace and the Inkscape CLI adapter were not evaluated — neither is installable in this environment. §23.7 marks the Inkscape CLI adapter as optional and experimental.
* A WebAssembly backend, which §23.4 prefers over a Python binding, has not been prototyped.

## Alternatives considered

* **VTracer as reference.** Rejected on thin-feature retention and on portability, both measured above. Retained as the first alternative and as a hedge if a WASM path is later built around it.
* **python_contour as reference.** Rejected: 157 nodes for a rectangle and 1016 for a 512×512 annulus, roughly 15–40× the other candidates. Retained as a fallback so the extension degrades rather than fails.
* **Multi-backend voting or per-scan backend choice.** Rejected as unnecessary complexity for the MVP; §37 keeps it available as future work.
