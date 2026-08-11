# Handoff

**Session date:** 2026-08-11
**Branch:** `agent/fix-cli-config-thin-features`, from merged PR #19's `master`
**Working slice:** fixed the two configuration defects recorded by the prior
session: CLI flags now override config files regardless of token order, and
`segmentation.protectThinFeatures` now controls the merge penalty. No Python
implementation source was read or changed.

## Configuration correctness

PTE-API-015 defines precedence by source, not by argument position. The CLI
previously mutated one `TraceConfig` while scanning tokens, so a later
`--config` erased an earlier `--profile`. `ConfigOverrides` now collects every
supported configuration flag and applies those values after all config files
have been parsed. `a_profile_flag_before_the_config_file_still_wins` proves the
profile wins while an unrelated file field remains in force.

`segmentation.protectThinFeatures=false` previously changed only the semantic
digest: §8.7's role penalty was always active. `merge_cost` now includes that
penalty only when the resolved switch is true, and the protected-outcome census
uses the same gate. `disabling_thin_feature_protection_changes_the_merge_decision`
holds every other input constant and proves the switch changes the production
merge decision. The default remains true, so the 12-fixture corpus is
unchanged by the census; no byte-for-byte comparison with the parent commit was
run.

## Licence decision

The engine is now MIT-only. ADR-0004 supersedes ADR-0001's former dual
`MIT OR Apache-2.0` choice, the Cargo workspace and fixture manifests declare
`MIT`, and `engine/LICENSE-APACHE` has been removed.

"Boundary" now has an explicit meaning: the engine remains independently
reusable under MIT, is written clean-room, and never depends on GPL application
code. A future adapter belongs on the GPL side and may call the engine through
the CLI, a C ABI, an in-process extension, or WASM. Process isolation is not a
licensing requirement; the packaging choice must preserve the MIT notice and
the one-way dependency/provenance rule.

## Merge resolution

PR #18's implementation is the retained architecture: it has a separate
`palette-tracer-aa` crate, exact square/half-plane inversion, multi-sample
normals, confidence, bounded §10.5 optimisation, and passing analytic §31.2
gates. The conflicting PR #17 geometry-local implementation was not merged on
top because it is both less complete and materially worse on the same corpus
(8/7 circle faces and 11 star faces versus PR #18's 4/3 and 2). The salvaged
fixed-point pass then takes the final circles to the expected 2/2 faces.

Two independent improvements from PR #17 were ported instead:

* §8.6 fringe reassignment repeats to a deterministic fixed point, and its test
  now requires a real reversible audit record rather than accepting an empty
  vector;
* automatic SVG precision reserves at most `0.05 px` of displacement when
  subpixel reconstruction is active, and the report uses the same selector.

The conformance table now records the encoded-sRGB transfer hypothesis as a
deliberate, tested compatibility extension and marks PTE-AA-001 partly rather
than claiming literal conformance to its linear-light fitting MUST.

## Start here: the diagnosis changed the plan

The previous handoff recommended §10 on the strength of the census — a circle
tracing to 8 faces, a star to 47 — and attributed both to §10.3's missing
coverage-to-position inversion. The inversion was indeed missing. **It was not
what produced the extra faces.** The §8.6 fringe pass was running and being
defeated by two separate things, and the first would have quietly corrupted
§10.3 had it been built on top.

**The mixture model was in the wrong blend space.** Instrumenting
`reassign_fringe`, the star's fringe was rejected 77 to 3 on residual, with
residuals in a band of 0.040–0.056 against a threshold of 0.035. A band that
tight is not noise. §10.2 fits a straight line in linear-premultiplied space;
most rasterisers composite antialiasing on the transfer-encoded bytes, and
`make_fixtures.py` does the same. A blend that is straight in encoded
coordinates is a *curve* in linear coordinates. Closed form predicts the
maximum residual at **0.0236** for the circle's colour pair and **0.0570** for
the star's; measured maxima were 0.024 and 0.056. The threshold sat exactly
between them — which is the entire reason one fixture absorbed 90 of 90 fringe
regions and the other rejected 77 of 80.

This mattered far more than the face counts. §10.3 inverts the same coverage
the fringe gate rejects on, and on encoded-blend input the linear-only
estimator is biased by **+0.22 at a true coverage of 0.5**. Fed to
`A_square⁻¹` that is a fifth of a pixel — *larger than the grid error §10
exists to remove*.

**§8.7's shape veto did not belong in §8.6.** The gate skipped any region
scoring above 0.5 for thin-feature protection, which inverts §8.6's own logic
(§8.6 treats thinness as evidence *for* fringe, and its four conditions contain
no shape veto) and misfired on the shape antialiasing produces: under the
8-connected partition a three-pixel chain touching only at corners has
perimeter 12 against area 3, so `perimeter²/(16·area)` reads 3.0 where a
straight one-pixel strip of the same length reads 1.33. The proxy is calibrated
for 4-connected blobs.

`engine/docs/notes/subpixel-antialias.md` §1 is the full write-up.

## What changed

**§10 exists.** New crate `palette-tracer-aa`, 23 tests, wired as stage E.5
between topology extraction and §11 fitting:

* **§10.2** — `two_color_best` estimates the compositing transfer from the
  evidence instead of assuming it. Every residual compared and reported is
  linear-light premultiplied; the encoded-sRGB parameter fit is the deliberate
  PTE-AA-001 compatibility deviation recorded above.
* **§10.3** — exact piecewise analytic square/half-plane inversion, the first
  of §10.3's three options and the *cheapest*. One `sqrt`, no iteration, so the
  decision is bit-identical across targets (PTE-DET-004). Monotone and
  reversal-symmetric exactly, not to within a tolerance.
* **§10.4** — normals from boundary tangents over a five-sample window, with a
  stability measure feeding PTE-AA-005's confidence.
* **§10.5** — the objective reduced to one unknown per sample (displacement
  along the normal), solved by eight Gauss-Seidel sweeps. Endpoints pinned and
  trust region hard, because an infinite penalty is a constraint.
* **PTE-AA-009** — the report's census now counts each edge's actual evidence
  class. It used to derive it from the profile, so `coverage_reconstructed` was
  hard-coded to zero.

**§31.2's gates are measured and met**, in
`crates/palette-tracer/tests/subpixel_gates.rs`, on the analytic circles
regenerated from their descriptions (PTE-TEST-004 commits the generator, not
the rasters):

| Metric | Gate | circle-0 | circle-1 |
|---|---:|---:|---:|
| Median boundary normal error | `≤ 0.10 px` | 0.028 | 0.055 |
| p95 | `≤ 0.35 px` | 0.155 | 0.262 |
| Max (no junctions to exclude) | `≤ 0.75 px` | 0.391 | 0.537 |
| Circle centre error | `≤ 0.20 px` | 0.001 | 0.032 |
| Relative radius error | `≤ 1%` or `0.20 px` | 0.092% | 0.106% |

**Corpus, before and after the whole session:**

```
                      faces  lines cubics  bytes        faces  lines cubics  bytes
circle-subpixel-0         8    146     16   1917   ->       2      6      6    619
circle-subpixel-1         9    158     22   2167   ->       2      6      6    625
rounded-rectangle        14    184      4   2181   ->       2     12      8    616
star-acute-corners       47    556     12   6160   ->       2     20     20   1104
```

Every other fixture is byte-identical, including `pixel-art/diagonals` and
`topology/one-pixel-bridge`.

## Commands run, and what they actually returned

```
cd engine && cargo test --workspace          400 passed, 0 failed
cd engine && cargo clippy --workspace --all-targets -- -D warnings   clean
cd engine && cargo fmt --check                                       clean
cd engine && cargo check --workspace --target wasm32-unknown-unknown clean
make engine-deny        advisories ok, bans ok, licenses ok, sources ok
make engine-parity      12 fixture(s), native and wasm32 agree
make engine-corpus      the census above
```

For this configuration slice specifically:

```
cargo test -p palette-tracer-cli --test cli
    23 passed, 0 failed
cargo test -p palette-tracer-segment
    28 passed, 0 failed
make engine-test        400 passed, 0 failed
make engine-lint        fmt clean; Clippy clean with -D warnings
make engine-wasm        workspace check clean for wasm32-unknown-unknown
make engine-deny        advisories ok, bans ok, licenses ok, sources ok
make engine-corpus      12 fixtures; census unchanged
make engine-parity      12 fixtures; native and wasm32 agree
git diff --check        clean
```

Both new regression tests were run before the implementation and failed for
the intended reason: the effective profile was `flat-illustration` instead of
`logo`, and disabling thin-feature protection still produced zero merges.
After the fixes both pass. The first `make engine-parity` retry stopped before
comparison with `node: not found` because the Rust-only `PATH` omitted Node;
adding the installed Node 22 directory made the command pass.

The MIT-only documentation session additionally ran:

```
make engine-lint        cargo fmt and workspace/all-target Clippy clean
make engine-deny        advisories ok, bans ok, licenses ok, sources ok
cargo metadata          all 10 workspace packages report license = MIT
make_fixtures.py        all 12 generated manifest entries report first-party MIT
git diff --check        clean
```

`cargo install cargo-deny --locked` failed once with a `libc` compile error and
succeeded on a plain retry; if it fails for you, just run it again.

The Python suite was not re-run; nothing under `palette_trace/`, `tests/` or
`pyproject.toml` changed.

## Things tried that did not work, so the next session does not repeat them

* **Ranking transfer hypotheses by smaller residual is a coin flip on
  near-neutral colours.** When the two sides differ mainly in luminance, all
  three channels scale together, the encoded blend curve lies within
  quantisation of the encoded segment, and both hypotheses fit to within a code
  point — while their coverages differ by about 0.07. The first draft did
  exactly this and returned 0.919 where the truth was 0.845. The fix is an
  asymmetric rule: linear light is §10.2's stated model and stays the default
  unless it *visibly* fails (residual above a `4e-3` quantisation floor) and
  the encoded hypothesis explains the remainder materially better (half the
  residual).
* **A saturated pixel is a perfect mixture and locates nothing.** Coverage 0 or
  1, residual zero, so it wins any contest ranked by residual — and its
  inverted offset is the pixel's own edge. The first draft admitted them and
  reconstructed *nothing*, pinning every boundary back onto the grid.
  Candidates are now restricted to coverages at least 0.03 inside `[0, 1]`.
* **A per-edge evidence quorum throws away corners.** Requiring half an edge's
  samples to carry coverage before using any of them lost the corners of
  `curves/rounded-rectangle` entirely, because its sides are axis-aligned runs
  of saturated pixels. The right unit of decision is the sample, not the edge;
  unconstrained samples simply do not move.
* **§31.2's gates and `geometry.curveTolerancePx` are budgets for the same
  quantity.** The fitter is *permitted* to deviate by up to the tolerance, so a
  profile above a gate cannot meet it however exact the reconstruction beneath
  is. `flat-illustration`'s 0.6 px default is six times §31.2's median target,
  and at that setting circle-1 measures a median of 0.103 and a p95 of 0.392.
  The gates are therefore measured at 0.10 px and the interaction is recorded
  in `the_gates_are_a_budget_shared_with_the_fitters_tolerance`. **No profile
  default was changed and no gate was loosened** (§31, `AGENTS.md`).
* **Signs in the §10.5 data term are easy to get backwards and hard to see.**
  The normal from `normal::normals` points into the *left* face; §10.3's
  convention needs it pointing *out* of the region whose coverage is measured.
  The end-to-end vertical-edge test at known subpixel positions is what caught
  it; the unit tests on the inversion alone all passed with the sign wrong.

## What to build next

**§10.4's weights have nothing to consume them.** Junction positions are
PTE-TOPO-011/012/013 and are not implemented, so §10 pins every chain endpoint.
This remains §10's largest gap and the natural next slice — and unlike this
session's starting point, the barycentric estimator it needs already exists and
is tested. The analytic circles have no junctions and therefore cannot validate
that requirement; their final 2/2 face count removes the former misleading
attribution.

After that, §11.7 primitives (a circle is still cubics, not a circle) would
make `curves/circle-subpixel-*` say what they are, and PTE-GEO-011's "recognized
primitives MUST remain represented semantically in the IR" would have something
to represent.

## Unverified assumptions

* The corpus exists; nothing is calibrated against it. Every threshold is still
  an engineering choice, including §10's new ones — the 0.35 smoothness weight,
  the eight sweeps, the one-pixel trust radius, the 0.03 coverage band, the
  `4e-3` quantisation floor and the 0.5 transfer margin.
* The transfer hypothesis is chosen per pixel and never pooled over an edge or
  an image, which would be strictly stronger evidence.
* The seam gate still uses a first-party rasteriser. §18.7's cross-renderer
  matrix is not satisfied, and no cross-renderer claim is made.
* Parity is proven for `wasm32-wasip1` under Node's V8. A `wasm-bindgen`
  binding does not exist and no other browser engine has been tried.
* No baseline. The segment counts above have nothing to be compared against
  until PTE-BASE-001's benchmarks exist.

## If you are wiring the engine into the Python application

The licence question is settled, but the integration is not implemented. Read
ADR-0004 before choosing the mechanism. Put the adapter on the GPL side, depend
one-way on the engine's public contract, retain the engine's MIT notice, and do
not use `palette_trace/**/*.py` as implementation input for the engine
(ADR-0003). The CLI, C ABI, in-process extension and WASM remain engineering
options; none is selected by this decision.
