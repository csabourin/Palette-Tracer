# Handoff

**Session date:** 2026-08-10
**Branch:** `claude/tracing-engine-apidpj`
**Working slice:** added `engine/`, a new Rust raster-to-SVG engine built to the
Palette Tracer Engine specification. Nothing under `palette_trace/` was touched.

## What exists now

`engine/` is a nine-crate Cargo workspace implementing Phases 0–2 of
`engine/SPEC.md` plus the SVG output path, end to end. It traces
`examples/sample.png` to a valid, seam-free, deterministic SVG.

Read `engine/docs/IMPLEMENTATION_STATUS.md` first. It is the durable record and
it has a *Gaps* section listing ten things that are not built.

## Commands run, and what they actually returned

```
cd engine && cargo test --workspace
    292 passed, 0 failed

cd engine && cargo clippy --workspace --all-targets -- -D warnings
    clean

cd engine && cargo check --workspace --target wasm32-unknown-unknown
    clean

make engine-trace
    pte: 5 faces, 10 shared edges, 29115 bytes,
    digest pte-semantic-v1-blake3:83a2eacc9329e79473674cac3d3ff02d44f32c83986914d0e4db1255ecb71950
```

The Python suite was not re-run in this session; nothing under `palette_trace/`,
`tests/` or `pyproject.toml` changed. `.gitignore`, `Makefile`, `README.md` and
`AGENTS.md` gained engine-related lines only.

## The largest thing still missing

**Curve fitting (§11).** Boundaries are grid-aligned polylines: the sample above
contains 4022 line segments where a fitted result would contain a small
fraction. The output is correct and it is not compact. §31.5's complexity gates
are not met and are not claimed. This is the next thing to build, and §39.9 is
the order to build it in — line and cubic fitting with *bidirectional*
validation, keeping the polyline as a fallback.

Subpixel reconstruction (§10) is the other half. The mixture estimator already
exists in `palette-tracer-color::mixture` and is used by fringe detection; what
is missing is §10.3's coverage-to-position inversion and the boundary
optimisation around it.

## Things tried that did not work, so the next session does not repeat them

* **A 4-connected partition makes §9.4 dead code.** With 4-connectivity the
  pixels of a one-pixel diagonal line are never adjacent, so each becomes its
  own region and the label map never contains the `A B / B A` pattern the
  ambiguity resolver exists for. The partition is now 8-connected (§8.2 permits
  either). The *region adjacency graph* stays 4-connected, because a shared
  boundary is a shared raster interface and a corner touch shares no length.
* **The first ambiguity energy had continuity backwards.** Asking only "does
  this diagonal continue past the cell" lets the background win every time, because
  the background continues everywhere. It now asks first whether the connection
  *matters* — whether the two pixels are already four-connected nearby — and only
  then whether it continues.
* **Vertex degree is not always even.** Three labels meeting produce a degree-3
  T-junction, in the interior and on the border alike. An earlier node rule
  looked only for degree 4 and split the image border into chains whose reverses
  were not chains, which surfaced as `half_edge_without_twin`.
* **A cache hit is not a claim.** The assignment cache stored only the label, so
  every hit was counted as inside the palette's reach and the §26.4
  "outside every reach" metric read zero. It stores the claim flag too.
* **The default reach never reached the effective config.** With an automatic
  palette it was consumed during generation, so neither the digest nor the
  session cache key could see a change to it. It is now a field on
  `EffectivePalette`.

## Unverified assumptions

* Every threshold is an engineering choice, not a measured optimum. There is no
  reference corpus, so §31's gates are calibrated against nothing.
* The seam gate uses a first-party rasteriser. §18.7's cross-renderer matrix is
  not satisfied, and no cross-renderer claim is made.
* `cargo check --target wasm32-unknown-unknown` proves the engine is host-free.
  It does not prove native and WASM produce the same digest; the corpus has not
  been run in a browser.
* `deny.toml` is written and reviewed. Nothing runs `cargo deny`.

## If you are wiring the engine into the Python application

Do not, without settling the licence question first. The engine is
`MIT OR Apache-2.0` and was written clean-room against
`palette_trace/**/*.py` (`engine/docs/decisions/ADR-0003-clean-room-provenance.md`).
A GPL host calling a permissive engine through a process boundary or a C ABI is
straightforward; linking them is a decision, not an implementation detail.
