# External baseline evidence

This directory contains evaluation output from independent tracing tools. The
SVGs and JSON reports are evidence under `SPEC.md` §29; they are not engine
implementation source and are never linked into the MIT crates.

## VTracer stable 0.6.15

`vtracer-0.6.15/eval-logo-flat-exact-palette*` is the first measured vertical
slice for PTE-BASE-001. It uses the MIT-licensed `vtracer` Python package from
PyPI, version-refused and digest-recorded by `tools/vtracer_stable_baseline.py`, with no
input preprocessing. The configuration, unsupported semantics, wall time and
peak RSS are recorded in the trace JSON.

The SVG is reconstructed by Inkscape 1.4.2 through
`tools/inkscape_renderer.py`. The score JSON records the Inkscape executable,
adapter and Python digests. This local WSL run made the renderer path
space-free with:

```bash
ln -sf '/mnt/c/Program Files/Inkscape/bin/inkscape.exe' \
  /tmp/pte-inkscape-1.4.2.exe
```

Reproduce both stages from the repository root:

```bash
.venv/bin/python engine/tools/vtracer_stable_baseline.py \
  --input engine/fixtures/synthetic/evaluation/logos/flat-logo.png \
  --output engine/baselines/vtracer-0.6.15/eval-logo-flat-exact-palette.svg \
  --report engine/baselines/vtracer-0.6.15/eval-logo-flat-exact-palette-trace.json

python3 engine/tools/svg_scorer.py \
  --fixture eval/logo/flat-exact-palette \
  --svg engine/baselines/vtracer-0.6.15/eval-logo-flat-exact-palette.svg \
  --renderer-command \
    'python3 engine/tools/inkscape_renderer.py --inkscape /tmp/pte-inkscape-1.4.2.exe {svg} {output} {width} {height} {background}' \
  --renderer-version-command \
    'python3 engine/tools/inkscape_renderer.py --inkscape /tmp/pte-inkscape-1.4.2.exe --version' \
  --output engine/baselines/vtracer-0.6.15/eval-logo-flat-exact-palette-score.json
```

This one analytic train fixture is not the PTE-BASE-001 corpus comparison. It
does not cover VTracer 1.0 alpha, other tracers, PTE, other profiles, a second
renderer, enforced resource limits, or aggregate ranking.
