# Palette Tracer Engine

A deterministic raster-to-SVG engine in Rust, built to `SPEC.md` in this
directory.

Its defining property is in the first executive decision of that specification:
**one raster partition, one shared topology.** Neighbouring filled regions share
a single geometric boundary, stored once and referenced by both sides. A tracer
that fits one contour per colour cannot avoid seams; this one cannot produce
them, because the two sides of an interface are the same numbers in opposite
order.

```bash
# From the repository root
make engine-test          # cargo test --workspace
make engine-lint          # fmt + clippy -D warnings
make engine-wasm          # cargo check --target wasm32-unknown-unknown
make engine-evaluation-corpus  # validate corpus truth and scorer controls
python3 engine/tools/svg_scorer.py --help  # blindly grade an arbitrary SVG

# Reproduce the first external baseline (quoted path may contain spaces)
make engine-vtracer-stable-baseline \
  ENGINE_INKSCAPE='/path/to/pinned/Inkscape-1.4.2/inkscape'

# Trace the repository's own sample image
python3 engine/tools/png_to_ppm.py examples/sample.png /tmp/sample.pam
cd engine && cargo run -p palette-tracer-cli -- trace \
    --profile flat-illustration --report /tmp/report.json \
    /tmp/sample.pam /tmp/sample.svg
```

The SVG and the report are two machine-readable artefacts, so they cannot share
one stream; sending both to `-` is refused rather than concatenated.

## Status

**Early.** Read `docs/IMPLEMENTATION_STATUS.md` before forming an expectation;
it is written to be believed rather than to be encouraging.

The short version: the colour pipeline, segmentation, shared topology, §10
subpixel reconstruction, shared multi-colour junction optimization, §11 curve
fitting and the SVG serialiser are built and tested. The analytic §31.2 circle
gates pass; both corpus circles and the antialiased star have the expected two
faces. Complete supported circles can remain semantic `<circle>` primitives,
with an opaque neighbour traversing the same boundary as exact arcs.

Generic partial arcs, non-circular primitives, centreline strokes, gradients
and fabrication are not built. Requests for unsupported behavior return an
error naming the requirement rather than quietly producing something else.
See `docs/IMPLEMENTATION_STATUS.md` for the exact claims and gaps.

The first external measurement is VTracer stable 0.6.15 on one analytic logo,
rendered by Inkscape 1.4.2. It is evidence for the harness, not completion of
the §29 matrix; see `baselines/README.md` and the recorded JSON reports.

## Layout

| Crate | What it owns |
|---|---|
| `palette-tracer-core` | Types and contracts: pixel views, config, vector and topology IR, report, digest, limits, determinism utilities. Depends on no sibling crate. |
| `palette-tracer-color` | §7 colour spaces, palettes, reaches, assignment, automatic palettes, mixtures |
| `palette-tracer-segment` | §8 edge field, minimum-spanning-forest partition, region graph, merging, fringe |
| `palette-tracer-topology` | §9 label map to shared half-edges, 2×2 ambiguity, validator |
| `palette-tracer-aa` | §10 coverage inversion, normals, confidence and bounded boundary optimisation |
| `palette-tracer-geometry` | §11 line/cubic fitting |
| `palette-tracer-svg` | §18 lowering, serialisation, and a diagnostic rasteriser for the seam gate |
| `palette-tracer` | The facade: `Engine::{validate_config, analyze, segment, vectorize, trace}` |
| `palette-tracer-cli` | `pte` |
| `palette-tracer-wasm` | The host-free session a WebAssembly binding would wrap |

The crate called `core` does not contain `trace`. See
`docs/decisions/ADR-0002-crate-split.md` for why: §5.1's assignment of both the
IR and the orchestration to one crate is circular.

## Licence

`MIT`. The repository root is GPL-3.0-or-later and governs the Python
application; this directory is a separately licensed MIT component. A future
adapter belongs on the GPL side and may call the engine through its public API;
the engine must not copy or depend on the GPL implementation. See
`docs/decisions/ADR-0004-mit-licence-and-integration-boundary.md` and
`ADR-0003-clean-room-provenance.md`.

MSRV 1.90, edition 2024. Built and tested with `rustc 1.97.1`.
