# Handoff

**Session date:** 2026-08-11
**Branch:** `claude/engine-project-gaps-rby09i`
**Working slice:** closed three of the ten gaps in `engine/`, and built §11 curve
fitting. Nothing under `palette_trace/` was touched, by decision — the licence
question in the previous handoff is still open and still unanswered.

## What changed

**§11 curve fitting exists.** `palette-tracer-geometry` was a one-line
placeholder; it is now the largest crate's worth of the build's largest gap.
Chain preparation, multi-scale corner classification with hysteresis, line and
cubic models with bidirectional error validation, and dynamic-programming
segmentation, wired into `Engine::vectorize` as stage F with the post-fit
revalidation PTE-GEO-014 requires.

On `examples/sample.png`: **4022 segments → 252** (184 lines, 68 cubics),
**29115 bytes → 3353**. Faces, shared edges and holes unchanged.

**Three gaps closed:**

* *No committed CLI test suite* → `crates/palette-tracer-cli/tests/cli.rs`, 22
  tests against the real binary.
* *No `cargo deny` in a command* → `make engine-deny`, and the policy it runs
  now passes.
* *No native-versus-WASM parity evidence* → `make engine-parity`, 13 fixtures,
  digests identical.

**A corpus exists.** `make engine-fixtures` generates 12 synthetic fixtures from
analytic descriptions with §25.2 manifests (PTE-TEST-004: the generator is
committed, the rasters are not). `make engine-corpus` traces them and prints the
§26.7 census.

## Commands run, and what they actually returned

```
cd engine && cargo test --workspace          362 passed, 0 failed
cd engine && cargo clippy --workspace --all-targets -- -D warnings   clean
cd engine && cargo fmt --check                                       clean
cd engine && cargo check --workspace --target wasm32-unknown-unknown clean
make engine-deny        advisories ok, bans ok, licenses ok, sources ok
make engine-parity      12 fixture(s), native and wasm32 agree
make engine-trace       pte: 5 faces, 10 shared edges, 3353 bytes
```

Toolchain moved from `rustc 1.94.1` to `1.97.1`; the newer clippy found one
pre-existing lint in `palette-tracer-color`, fixed here.

The Python suite was not re-run; nothing under `palette_trace/`, `tests/` or
`pyproject.toml` changed.

## Defects found by writing the tests, not by reading the code

* **`pte trace --report - <in> -` concatenated an SVG and a JSON report on one
  stream.** `main.rs` documented the rule and nothing enforced it. Now refused
  before any work is done.
* **`cargo deny` had never been executed and did not pass.** `arrayref`,
  reached through `blake3`, is BSD-2-Clause and only that. Admitted with a
  review note; the speculative `Zlib` allowance, which matched nothing, removed.
* **`THIRD_PARTY_NOTICES.md` claimed the CLI decodes PNG.** It reads Netpbm and
  refuses PNG by name.
* **The coloring-book duplicate-interface gate keyed on endpoints only.** It
  passed before because polylines differ in their intermediate points. Once
  chains became single cubics it started reporting false duplicates — the two
  ways round a one-pixel hole share both junctions and differ only in their
  control points. The key now covers full geometry, which is strictly stronger.

## Things tried that did not work, so the next session does not repeat them

* **A chord is not a tangent, and least squares over the same window does not
  fix it.** The chord over arclength `h` on an arc of radius `r` is rotated
  `h/2r` from the tangent — a *systematic* bias, 1.8° at `r = 40`, costing
  0.25 px over a span. Richardson extrapolation over two chords removes it
  (2.6e-2 rad → 4.5e-5 rad), but only in its general form: both windows snap to
  whole samples, so they are not in a 2:1 ratio and the familiar `2·d1 − d2`
  under-corrects by a third.
* **Extrapolation amplifies staircase jitter.** It removes a systematic bias and
  doubles a random one. Guarded to apply only when the two chords agree within
  8°, which separates a circular arc from a stair step cleanly.
* **Skipping §11.4's Newton reparameterisation costs more than it looks.**
  Chord-length parameterisation biases `α` by 1.4%, which is 0.16 px of sag at a
  quarter circle's midpoint — on its own past the §31.2 median gate. Four Newton
  passes take the measured circle error to 0.0072 px.
* **The first candidate budget starved the normal case.** It was
  `8 × samples`, which is far below the search's own worst case of
  `2 × (ladder + 1) × samples`, so ordinary chains hit the fallback. Derive the
  budget from the search, not from a round number.
* **Distance measurement was `O(m·F)` and dominated everything** — 32 s for the
  accuracy suite. Both sequences traverse the same boundary in the same
  direction, so a monotone sweep with bounded lookahead makes it `O(m + F)`
  (6 s). Where the assumption fails it over-estimates, which can reject a good
  candidate but never accept a bad one.
* **`kurbo` was declared for this crate and is not used.** Nearest-point on a
  cubic is a quintic root solve whose iteration count is not part of any API
  contract. PTE-DET-004 needs the same *decision* on every target. Removed from
  the manifest rather than left declared over unused code.

## What the corpus says to build next

The census in `engine/docs/IMPLEMENTATION_STATUS.md` makes the case on its own:
a circle traces to **eight faces**, a five-pointed star to **47**. Antialiased
input becomes thin fringe regions because §10's coverage-to-position inversion
does not exist, and those regions are also where almost every "already minimal"
chain comes from. **§10 is the next thing to build**, and unlike last session's
recommendation this one is a measurement rather than a judgement.

§10.5's boundary optimisation is the same work's other half and would lift the
one real limitation of the fitter: split points sit on source samples, so a 45°
staircase cannot become one line below a tolerance of `1/√2`.

## Unverified assumptions

* The corpus exists; nothing is calibrated against it. Every threshold is still
  an engineering choice, including the new ones (corner scales, the 8°
  extrapolation limit, the turning limit).
* The seam gate still uses a first-party rasteriser. §18.7's cross-renderer
  matrix is not satisfied, and no cross-renderer claim is made.
* Parity is proven for `wasm32-wasip1` under Node's V8. A `wasm-bindgen`
  binding does not exist and no other browser engine has been tried.
* No baseline. "252 segments" has nothing to be compared against until
  PTE-BASE-001's benchmarks exist.

## If you are wiring the engine into the Python application

Unchanged, and deliberately untouched this session. Do not, without settling the
licence question first. The engine is `MIT OR Apache-2.0` and was written
clean-room against `palette_trace/**/*.py`
(`engine/docs/decisions/ADR-0003-clean-room-provenance.md`). A GPL host calling a
permissive engine through a process boundary or a C ABI is straightforward;
linking them is a decision, not an implementation detail.
