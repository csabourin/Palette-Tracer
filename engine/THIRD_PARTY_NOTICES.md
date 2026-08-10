# Third-party notices

SPEC §37.4 requires this file. It lists every crate the Palette Tracer Engine
links, why it is here, and the licence review that admitted it
(PTE-ARCH-012, PTE-SEC-011, PTE-SEC-013).

The engine is licensed `MIT OR Apache-2.0`. Every direct dependency must be
compatible with both halves of that, must build for `wasm32-unknown-unknown`,
and must not pull a GPL or LGPL closure (PTE-LIC-003).

## Direct dependencies

| Crate | Licence | Used by | Why | Review note |
|---|---|---|---|---|
| `blake3` | CC0-1.0 OR Apache-2.0 OR Apache-2.0-with-LLVM-exception | `palette-tracer-core` | Semantic digest hash, Appendix F.4 | `default-features = false` disables `rayon` and `std`-only accelerations, so the build stays single-threaded and deterministic across targets (PTE-DET-001). Public-domain-equivalent primary licence; no copyleft closure. |
| `kurbo` | MIT OR Apache-2.0 | `palette-tracer-geometry` | Bézier evaluation, flattening, and curve/point distance queries used by the §11 fitter | Hidden entirely behind `palette_tracer_core::ir::PathSegment`; no `kurbo` type appears in any public signature outside the geometry crate (PTE-ARCH-011). Pure Rust, no build script, builds for wasm32. |
| `serde` | MIT OR Apache-2.0 | `core`, `cli`, `wasm` | Config and report schemas (§6.3, §6.5) | Derives are applied only to stable schema types. `default-features = false` keeps `std` explicit. |
| `serde_json` | MIT OR Apache-2.0 | `core`, `cli`, `wasm` | JSON form of the config and the trace report (Appendix A) | Arbitrary-precision and preserve-order features are off. |

## Development-only dependencies

| Crate | Licence | Why |
|---|---|---|
| `proptest` | MIT OR Apache-2.0 | Property tests required by §27.2 (topology invariants, curve-fit bounds, palette tie order). Never linked into a release artefact. |

## Deliberately absent

* **No renderer.** `resvg` is not a dependency. The seam and coverage tests use
  a first-party dev-only rasteriser in `palette-tracer-svg/tests`. The
  consequence is recorded honestly: the §18.7 cross-renderer compatibility
  matrix is **not** satisfied by this build. See
  `docs/IMPLEMENTATION_STATUS.md`.
* **No parallelism crate.** `rayon` is not a dependency; §19.6 parallelism is
  unimplemented, and single-thread execution is the conformance target
  (PTE-PERF-009, PTE-NO-038).
* **No image codec.** The core takes decoded pixels only (PTE-ARCH-001). The
  CLI reads a minimal subset of PNG with a first-party decoder rather than
  taking on a codec dependency; unsupported encodings return a typed decode
  error rather than a guess.
* **No GPL or LGPL code, in any crate, at any depth** (PTE-LIC-002/003). See
  `docs/decisions/ADR-0003-clean-room-provenance.md`.
