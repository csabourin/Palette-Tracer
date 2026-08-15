# Handoff

**Session date:** 2026-08-14
**Branch:** `codex/engine-blind-svg-baseline`, based on `master` at `0581928`
(`Merge pull request #25 from
csabourin/claude/corpus-sample-files-push-5qkh3s`)
**Working slice:** blind arbitrary-SVG scoring plus the first independent
renderer/external-tracer measurement.

## What changed

The engine now has a standard-library blind scorer at
`engine/tools/svg_scorer.py`. It parses arbitrary SVG without consulting PTE IR
or reports, refuses active/external content, records tool and input digests,
and measures reconstruction, topology, raster boundary and editability at
1×/4×/16× over transparent, white, black and diagnostic backgrounds.

Four analytic evaluation fixtures now have machine-readable topology truth in
`engine/fixtures/manifests/evaluation-corpus-v1.json`. The corpus validator
recomputes those signatures from the committed raster. Negative controls cover
holes, rare accents, seams, active/external SVG and topology-truth drift.

The first PTE-BASE-001 vertical slice is recorded in
`engine/baselines/vtracer-0.6.15/`. The pinned VTracer stable wrapper uses the
MIT `vtracer` 0.6.15 Python package already present in `.venv`; it is evaluation
tooling, not a Cargo or engine runtime dependency. The Inkscape adapter refuses
versions other than 1.4.2 and maps the scorer's four background names to
explicit export colors/opacity. `Makefile` exposes `engine-svg-score` and
`engine-vtracer-stable-baseline`; `engine/baselines/README.md` has exact
reproduction commands.

The measured fixture is `eval/logo/flat-exact-palette`, with no preprocessing,
no color cap and no fixed palette (the stable VTracer Python API does not expose
the latter two constraints). VTracer emitted 8 paths, 165 cubics, 13 lines, 508
control points and 10,538 SVG bytes in 0.007551 s, at 25,997,312 peak RSS bytes.
Inkscape 1.4.2 rendered all twelve passes. At 1× transparent: PSNR 26.757687 dB,
global SSIM 0.991442559, alpha MAE 0.002133681, missing-patch fraction
0.000013021. Boundary F-score is 0.996300211 at 1 px, symmetric Chamfer
0.123806 px, approximate Hausdorff 1.414214 px. The current hard topology gate
fails: teal is 4 components rather than 2, and 706 rendered pixels are outside
the declared classification distance. Do not turn this into an aggregate
quality claim; boundary antialias classification has not been calibrated.

## Validation actually run

```text
python3 -m py_compile engine/tools/svg_scorer.py \
  engine/tools/validate_evaluation_corpus.py \
  engine/tools/test_svg_scorer.py engine/tools/test_evaluation_corpus.py \
  engine/tools/vtracer_stable_baseline.py engine/tools/inkscape_renderer.py \
  engine/tools/test_external_baseline_tools.py
    success, no output

make engine-evaluation-corpus
    evaluation corpus: 18 valid fixtures (6/6/6)
    13 analytic fixtures regenerate byte-identically; 5 fixed digests
    18 tests passed in 5.886 s

.venv/bin/python engine/tools/vtracer_stable_baseline.py --version
    VTracer 0.6.15 (stable)

.venv/bin/python engine/tools/vtracer_stable_baseline.py \
  --input engine/fixtures/synthetic/evaluation/logos/flat-logo.png \
  --output engine/baselines/vtracer-0.6.15/eval-logo-flat-exact-palette.svg \
  --report engine/baselines/vtracer-0.6.15/eval-logo-flat-exact-palette-trace.json
    completed in 0.007551 s

python3 engine/tools/inkscape_renderer.py \
  --inkscape '/mnt/c/Program Files/Inkscape/bin/inkscape.exe' --version
    Inkscape 1.4.2 (f4327f4, 2025-05-13)

# Exact full scorer command is in engine/baselines/README.md.
# It completed all 12 passes and wrote the recorded score JSON.

make -n engine-vtracer-stable-baseline \
  ENGINE_INKSCAPE='/mnt/c/Program Files/Inkscape/bin/inkscape.exe'
    command expansion and quoting are correct

git diff --check -- engine Makefile
    clean
```

Rust was not rerun: `cargo`, `rustc`, and `cargo-deny` are absent, and this
slice changes no Rust, Cargo metadata, engine algorithm, strict synthetic
fixture, or threshold. Current inherited Rust evidence remains 423 tests plus
2 doctests, clean Clippy/fmt/WASM, and 13-fixture native/WASI parity.

## Failed or incidental attempts

- Direct Windows-path updates through the patch helper repeatedly returned
  `helper_unknown_error`; updates were expressed as patch files and applied
  with `git apply`. Temporary patch/reject files were removed.
- A combined Chrome/Edge version probe opened Chrome instead of behaving like
  a CLI query. Its WSL launcher was terminated. Chrome and Edge were not used
  for evidence.
- The full pure-Python 16× scorer run took roughly nine minutes while staying
  compute-bound around 123 MiB RSS. This was observed, not instrumented, and is
  not a calibrated performance result.
- Passing the Inkscape path through nested PowerShell/WSL quoting failed once.
  The recorded run used the exact temporary symlink bootstrap documented in
  `engine/baselines/README.md`; the Make target's quoted direct path was checked
  with `make -n`.

## Next work

1. Inspect/calibrate topology classification around antialiased boundaries
   before treating the 706 unclassified pixels as a tracer defect. Do not tune
   from holdout data.
2. Run the full licensed corpus for VTracer stable, then VTracer 1.0 alpha.
3. Add PTE, Potrace, AutoTrace and ImageTracerJS using the same scorer and
   Inkscape renderer. Unsupported semantics stay explicit, not aggregate
   failures.
4. Add a second renderer before any cross-renderer claim and instrument scorer
   wall time/peak memory before setting budgets.

Do not read `palette_trace/**/*.py` while implementing the MIT engine. The root
Python adapter is outside this clean-room boundary.
