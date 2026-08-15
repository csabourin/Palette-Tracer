# Blind SVG scoring

## Problem and requirements

`SPEC.md` §39 issue 1 requires a scorer that can grade an arbitrary SVG before
PTE is compared with external tracers. This note covers §25.1–§25.2, §26.1–
§26.4, §26.7, §28.2, §29.1–§29.2, PTE-TEST-005/006/012/013/014/015 and the
evidence rule in §0.2.

The scorer must not trust PTE-specific semantic IR or trace-report fields. Its
inputs are the same candidate SVG bytes an external tracer produces, the
licensed evaluation raster, machine-readable fixture truth, and pixels from an
independent renderer.

## Conventions and metrics

Source raster pixel `(x, y)` covers `[x, x+1) × [y, y+1)`. At scale `s`, the
reference value at rendered pixel `(u, v)` is source pixel
`(floor(u/s), floor(v/s))`. This nearest-neighbour reference preserves the
actual raster evidence instead of inventing subpixel truth the corpus does not
contain.

Encoded sRGB channels are decoded with §7.1's transfer function before RGB
error is accumulated. Transparent comparisons use premultiplied linear RGB and
measure alpha separately. Opaque diagnostic passes composite both images onto
the named encoded-sRGB background, then decode the result for measurement.
`ΔE_OK` is Euclidean distance in OKLab. Percentiles use a fixed `0.0005`-wide
histogram so memory does not scale with the number of rendered pixels.

The reconstruction report contains linear-RGB MSE/PSNR, whole-image luminance
SSIM, alpha mean absolute error, a missing-patch fraction and p50/p95/max
`ΔE_OK`. Whole-image SSIM is named `globalSsim`; it is not represented as the
windowed SSIM metric used by some image-quality suites.

Topology truth names a finite ordered palette and the expected component, hole
and Euler-characteristic counts for each protected label. Candidate pixels are
assigned to the nearest `(OKLab, alpha)` tuple when it lies within the
manifest's declared classification distance; equal distances choose the first
manifest label. Components and holes use four-connectivity. An unclassified
pixel is a hard topology failure. The four v1 truth fixtures cover a central
hole, a rare two-component accent, transparent/partial-alpha regions, and open
versus closed line work.

Boundary samples are classified raster cells adjacent to a different label.
Symmetric Chamfer, approximate Hausdorff and tolerance precision/recall/F-score
use an eight-neighbour distance graph with orthogonal cost `1` and diagonal
cost `sqrt(2)`. The output says `approximateHausdorffPx`; it does not claim an
exact continuous-curve bound.

SVG complexity is parsed independently with Python's XML stack. The census
includes visible elements, paths, line/arc/quadratic/cubic segments, control
points, primitives, strokes, gradients/stops, groups, raw bytes and
deterministic gzip bytes. DTDs, entities, scripts, `foreignObject`, external
references and external CSS are refused before a renderer is invoked.

## Inputs, outputs and invariants

`tools/svg_scorer.py` accepts one manifest fixture ID, one SVG and an explicit
renderer command template. The renderer receives only the SVG, output path,
dimensions, scale and background; it never receives the reference raster. The
JSON report records SHA-256 digests of the scorer, manifest, SVG and renderer
executable plus renderer version output.

The principal invariants are:

- no PTE report, semantic digest or typed IR is an input;
- the renderer command runs without a shell and has a bounded timeout;
- every requested render has exactly the expected dimensions;
- topology truth is validated against the committed reference before it can
  grade a candidate;
- metric output is deterministic for identical SVG, renderer and corpus bytes;
- naturalistic generated inputs remain comparative-only and cannot become
  calibration truth through this tool.

Failures are explicit: malformed or active SVG, unknown fixtures, unsupported
reference formats, absent renderer placeholders/executables, renderer failure
or timeout, malformed topology truth, unexpected PNG dimensions and unsupported
PNG encodings all return a non-zero status. There is no silent metric omission
for a fixture that declares topology truth.

## Complexity and work limits

For `N = width × height × scale²`, reconstruction metrics take `O(N)` time and
`O(N)` decoded-pixel memory. Histogram storage is constant. At source scale,
classification and connected-component analysis are `O(width × height)`;
the boundary distance calculation is `O(N log N)` time and `O(N)` memory due
to its deterministic priority queue. XML work is linear in the bounded SVG
size apart from path-token storage.

The first slice caps SVG input at 256 MiB, scales at 16, and each renderer
invocation with a caller-controlled timeout (60 seconds by default). Renderer
invocations are the cancellation boundaries. Pixel loops do not yet accept a
cooperative cancellation token; the CLI process remains externally
terminable, and this limitation must be addressed before the scorer is used as
an untrusted long-running service.

## Provenance and alternatives

The implementation was written independently from `engine/SPEC.md` and the
matrices and test assets already published in this MIT engine. It reads no GPL
application implementation and adds no dependency. Python's standard library
provides XML, hashing, compression, process and data-structure support.

Alternatives rejected for this slice:

- the engine's first-party IR rasterizer, because it cannot parse arbitrary
  external SVG and would not be an independent renderer;
- an embedded SVG renderer dependency, because no renderer decision, licence
  review or cross-target check has been recorded yet;
- PSNR/SSIM-only scoring, because PTE-TEST-005/006 make protected topology a
  hard, separate dimension;
- source-vector access in the renderer, because PTE-TEST-014 requires equal
  information for PTE and competitors.

## Tests and fixtures

`tools/test_svg_scorer.py` covers exact reconstruction, a missing hole, a
missing tiny accent, a diagnostic-background seam, independent SVG complexity,
malformed/active/external SVG refusals, renderer identity evidence and an
end-to-end exact topology pass on `eval/logo/flat-exact-palette`.
`tools/test_evaluation_corpus.py` includes a negative control proving that
declared topology truth cannot drift away from its raster. The corpus validator
recomputes all four machine-readable signatures on every run.

## Known limitations

- No independent renderer is bundled. The first external baseline uses a
  version-refusing adapter around a locally installed Inkscape 1.4.2 and the
  score records the adapter, interpreter and Inkscape binary digests. Other
  environments must supply their reviewed executable explicitly.
- Corpus v1's generated assets are comparative-only. The scorer currently
  decodes the 8-bit non-interlaced PNG reference subset, not its one JPEG.
- `globalSsim` is not windowed SSIM.
- Boundary distances are raster approximations and do not replace analytic
  arclength sampling when future fixtures provide source vectors.
- SVG style inheritance and reusable-element expansion are not normalized into
  semantic editability counts. Counts describe the serialized candidate.
- Peak memory/runtime, browser-renderer parity and calibrated quality gates are
  not established by this slice.
