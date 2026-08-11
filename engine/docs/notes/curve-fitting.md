# Curve fitting

Design note for `palette-tracer-geometry`, with the §34.2 contents.

**Requirements:** PTE-GEO-001 through PTE-GEO-009, PTE-GEO-012, PTE-GEO-013,
PTE-GEO-014. §11.1–§11.6, §11.8. Consumed by §31.5's complexity gates.

**Not covered here:** §11.7 primitive recognition and the §11.4 circular arc
candidate. Complete-circle recognition now has its own design note,
`primitive-recognition.md`; the arc candidate remains unimplemented and
`Profile::expand` refuses `geometry.allowArcs` by name.

---

## What it does

The topology extractor emits one `CurveChain` per shared edge, as a polyline
with one line segment per pixel step. This crate replaces each chain with the
smallest set of lines and cubic Béziers that stays inside the profile's error
budget.

On `examples/sample.png` that is 4022 segments down to 252 (184 lines, 68
cubics) and 29115 bytes down to 3353, with the face count, shared-edge count
and hole count unchanged.

## Mathematics and conventions

Coordinates are `f64` in the image domain, `y` down (`core::ir::geom`). All
tolerances are in source pixels before physical scaling (PTE-GEO-008).

### Cubic model

§11.4's constrained form. With unit tangents `t0`, `t1` in the direction of
travel,

```text
P1 = P0 + α·t0        P2 = P3 − β·t1
B(t) = (1−t)³P0 + 3(1−t)²t·P1 + 3(1−t)t²·P2 + t³·P3
```

`α` and `β` come from the 2×2 normal equations of the least-squares problem
over a chord-length parameterisation, then up to four Newton reparameterisation
passes. The endpoints are exact by construction, which is what lets a span be
pinned to a junction.

**Why reparameterisation is not optional.** The chord-length parameterisation
assumes the cubic's parameter advances in proportion to arclength. It does not.
On a quarter circle the resulting `α` is 21.79 against an optimum of 22.09 —
1.4% — and 1.4% of tangent length is 0.16 px of sag at the midpoint. That alone
exceeds the 0.10 px median of §31.2. Newton removes it: measured error on an
analytic circle falls to 0.0072 px.

### Flattening bound

Distances are measured against a flattened polyline, so the flattening error
enters every gate. The bound is proven rather than estimated, from the linear
precision of the Bernstein basis:

```text
‖B(t) − L(t)‖ ≤ max( ‖P1 − L(⅓)‖, ‖P2 − L(⅔)‖ )
```

Midpoint subdivision scales it by ¼, so `d` uniform subdivisions give
`bound / 4^d`. `Cubic::flatten` picks the smallest such `d`, capped at 10, and
returns `None` at the cap — PTE-GEO-007's "conservative cap plus rejection".
The bound is *added* to every measured distance before comparison, so a
candidate accepted here is accepted under the true distance too.

### Tangent estimation

A chord is not a tangent. On an arc of radius `r` the chord over arclength `h`
is rotated by `h/2r` from the tangent at its base — a bias linear in the window
that no least-squares fit over the same window removes. At `r = 40` and
`h = 2.5` it is 1.8°, and 1.8° of endpoint tangent error costs 0.25 px over the
span: twenty times the §31.2 median gate, arising entirely from the estimator.

Two chords and Richardson extrapolation remove the linear term:

```text
T ≈ d1 + (d1 − d2)·h1/(h2 − h1)
```

in its general form, not the familiar `2·d1 − d2`. Both windows snap to whole
samples, so a request for 2.5 and 5.0 px on a chain sampled every 1.05 px gives
3.14 and 5.24 — a ratio of 1.67, which the constant form under-corrects by a
third. Measured tangent error on the analytic circle: 0.026 rad before,
4.5 × 10⁻⁵ rad after.

Extrapolation removes a *systematic* bias and amplifies a *random* one. On a
staircase the two chords differ by the step phase, which is noise, and doubling
it is worse than accepting the bias. It is therefore applied only when the two
chords agree within 8°, which a circular arc does down to a radius of about nine
pixels and a stair step does not.

### Corner classification

For each interior sample and each scale in {2, 4, 8} px, the turning angle
between the chords to the samples that far away. The statistic is the
**minimum** over supported scales: a sample must look like a corner at *every*
scale to be one.

That is the mechanism for PTE-GEO-002. On a 45° staircase the fine-scale angle
alternates ±90° while both coarse chords lie along the diagonal, so the minimum
is near zero. On a real 90° corner every scale agrees.

Hysteresis (PTE-GEO-004) is Canny's, along the chain: samples at or above
`threshold + hysteresis/2` seed a region, the region grows while the statistic
stays above `threshold − hysteresis/2`, and the strongest sample in each region
becomes the corner. A numeric wobble moves a corner by a sample at most; it
cannot split one corner into two or flip a run.

### Error metrics (§11.5)

Every candidate is measured in both directions:

* **source → curve** — the largest distance from a source sample to the
  candidate;
* **curve → source** — the largest distance from a point on the candidate to
  the source polyline.

The second is the one a sample-only fitter cannot see, and it is what rejects a
Bézier that "can pass near samples while ballooning between them". Their
maximum is the approximate bidirectional Hausdorff distance, and it is what the
tolerance gates. Cusps and loops are rejected before any distance is measured,
by total tangent turning: a span between two pins that turns more than π is
looping whatever its endpoint error says.

### Segmentation (§11.6)

`D[j] = min over i<j and models M of D[i] + cost(M, i, j)`, run independently
between consecutive pins so pins are honoured by construction. Candidates
failing the hard tests never enter the graph.

## Tie rules

§11.1 gives a soft objective with weights `λn`, `λp`, `λr`; §11.6 gives a strict
tie-break order. Choosing weights would make the tie-break order an emergent
property of three arbitrary constants, and PTE-DET-003 forbids a bare float from
deciding anything. So the cost *is* the order, as an additive lexicographic
tuple:

1. segment count;
2. free parameters (line 2, cubic 6);
3. model simplicity (line 0, arc 1, cubic 2);
4. total error, quantised at `GEOMETRY_SCALE`;
5. earliest split, realised by preferring the smaller predecessor on a tie.

Every component is additive, so the recurrence keeps optimal substructure. Rule
1 of §11.6, "lower hard-error class", is not a component because the graph
contains no failing candidate to be worse than a passing one.

Fewer segments therefore always beats a smaller error, which is what §31.5 asks
for: a straight boundary is one line, not four that fit marginally better.

## Invariants

* **Endpoints never move.** Every model is built with its span's own endpoints;
  the chain's first and last samples are its junction vertices. `fit_topology`
  touches `Topology::chains` and nothing else — vertices, half-edges, faces and
  cycles come out identical (`fitting_changes_only_the_chains`).
* **One chain, fitted once** (PTE-GEO-012, PTE-AA-008). There is one chain per
  shared edge and both faces read it through `oriented_chain`, so "independent
  control points for the two sides" stays unrepresentable.
* **Reverse equivalence** (§25.4). Fitting a chain and reversing it gives
  exactly what reversing and fitting gives. Least squares is not symmetric under
  reversal, so this is obtained rather than hoped for: each chain is rotated
  into a canonical direction — start from the lexicographically smaller endpoint
  in raster order — fitted, and rotated back. The two traversals of one
  interface are literally the same computation
  (`fitting_a_reversed_chain_gives_the_reversed_fit_exactly`, asserted on exact
  equality).
* **Never worse.** A fit that does not reduce the segment count is discarded and
  the polyline stands.
* **No NaN control point** (PTE-GEO-006). A degenerate solve falls back to
  `α = β = chord/3`; a non-positive tangent length is refused outright.
* **Revalidation** (PTE-GEO-014). Fitting changes shared geometry, so
  `Engine::vectorize` reruns the topology validator afterwards.

## Complexity

`O(n·K)` candidate fits for a chain of `n` samples, where `K` is the
predecessor ladder (`max_span_candidates`, capped at 64). Predecessors are
proposed on a geometric ladder — 1, 2, 3, 4, 6, 9, 13, … samples back — plus the
block's first sample, always. The ladder is what keeps generation near-linear;
including the block start unconditionally is what keeps "a straight boundary is
*one* line" representable however long the run.

Distance measurement is the inner cost and was `O(m·F)` for a span of `m`
samples against a flattening of `F` points — the dominant cost of the whole
fitter, 32 s for the accuracy suite. Both sequences traverse the same boundary
in the same direction, so the nearest segment advances monotonically and a
sweep with a bounded lookahead makes it `O(m + F)`; the suite now takes 6 s.
Where the monotone assumption fails the sweep over-estimates, which can reject a
good candidate but never accept a bad one.

## Cancellation and resource bounds

`fit_topology` calls `check_cancel` once per chain. Within a chain, the
candidate budget is `2·(K+1)·n`, derived from the search's own worst case rather
than picked: exceeding it means the search is not behaving as analysed, and the
chain keeps its polyline. Newton is capped at four passes and flattening at
depth 10, so no single candidate can consume unbounded work (PTE-GEO-005).

## Known limitations

1. **Hard-edged split points remain source samples.** §10.5 moves samples only
   when trusted antialias coverage provides a position. A hard-edged 45°
   staircase carries only §10.6's uncertainty interval, so its off-diagonal
   samples remain `1/√2 ≈ 0.707` px from the ideal chord and a tolerance below
   that still splits a boundary that is "really" one line. The generated
   `curves/shallow-staircase` fixture is deliberately this case. Optimising
   crisp evidence inside its uncertainty strip remains unimplemented.
2. **Subpixel evidence is conditional.** Trusted two-colour coverage is
   reconstructed and optimised before fitting; crisp and low-confidence samples
   stay on pixel-cell interfaces. `BoundaryEvidence` and the report distinguish
   those outcomes. See `subpixel-antialias.md` and the measured §31.2 gates.
3. **No generic arcs; primitive recognition is circle-only.** §11.4's arc
   candidate is absent. §11.7 can retain a sufficiently supported complete
   circle semantically, but rectangles, ellipses, rounded rectangles, polygons
   and repeated radii remain unimplemented. See `primitive-recognition.md`.
4. **Corner detection needs about six pixels of chain.** Fewer than five samples
   or fewer than two supported scales means no corner is declared, because what
   remains is the three-point angle PTE-GEO-002 forbids. Short chains are fitted
   without pins; the error gates still bind.
5. **The thresholds are engineering choices.** The corner scales, the 8°
   extrapolation limit, the sweep lookahead and the turning limit are reasoned
   but have not been selected by a corpus parameter sweep. The synthetic corpus
   now measures their outcomes; it is not yet a calibration study.

## Alternatives considered

* **`kurbo` for evaluation and nearest-point queries.** Declared in the
  workspace manifest for this purpose; not used. Nearest-point on a cubic is a
  quintic root solve, and a library's iteration count and convergence test are
  not part of its API contract. "Accurate to 1e-9" is not the same as "the same
  decision on every target", and the semantic digest is a decision (PTE-DET-004).
  Everything here is closed-form or a fixed bounded loop. The dependency was
  removed from the manifest rather than left declared and unused.
* **Schneider's recursive refit.** PTE-GEO-005 permits it with a bound. A
  dynamic program was chosen instead because §11.6 asks for global segmentation
  with a stated tie order, which recursive halving cannot give: its split points
  come from where the error happens to peak.
* **Weights instead of a lexicographic tuple.** Rejected under PTE-DET-003; see
  *Tie rules*.
* **Total-least-squares tangents over a window.** Same first-order bias as a
  chord, and more arithmetic. Richardson over two chords removes the bias
  instead of averaging it.

## Tests

| Claim | Test |
|---|---|
| The chord bound is an upper bound | `the_chord_bound_is_never_exceeded_by_the_real_curve` |
| Flattening honours its tolerance | `a_flattened_polyline_stays_within_its_tolerance` |
| Flattening rejects rather than degrades | `an_impossible_tolerance_is_rejected_rather_than_subdivided_forever` |
| Endpoints survive flattening exactly | `flattening_preserves_the_endpoints_exactly` |
| A staircase is not a corner (PTE-GEO-002) | `a_forty_five_degree_staircase_has_no_corners`, `a_shallow_staircase_has_no_corners` |
| A real corner is found once | `a_right_angle_is_one_corner_at_the_turn`, `a_square_has_exactly_four_corners` |
| Curvature is not a corner | `a_circle_has_no_corners` |
| Hysteresis does not multiply corners | `a_near_threshold_corner_stays_a_single_corner` |
| Duplicates removed, junctions kept (PTE-GEO-001) | `duplicate_samples_are_removed_but_the_endpoints_survive` |
| Ballooning is rejected (§11.5) | `a_ballooning_cubic_is_rejected_though_it_passes_through_the_samples` |
| No NaN control point (PTE-GEO-006) | `a_degenerate_span_never_produces_a_nan_control_point` |
| A straight run is one line (§31.5) | `a_long_straight_run_becomes_exactly_one_line` |
| A rectangle is four lines (§31.5) | `a_rectangle_becomes_four_lines`, `a_rectangle_is_four_lines` |
| Looser tolerance never costs more segments | `a_looser_tolerance_never_needs_more_segments`, `loosening_the_tolerance_never_costs_more_segments_or_breaks_the_budget` |
| Reverse equivalence (§25.4) | `fitting_a_reversed_chain_gives_the_reversed_fit_exactly` |
| Junctions do not move (PTE-GEO-013) | `fitting_never_moves_a_chain_endpoint`, `fitting_changes_only_the_chains` |
| Already-fitted chains are not refitted | `a_chain_containing_a_cubic_is_left_alone` |
| Pixel art is untouched (§15) | `a_zero_tolerance_leaves_every_chain_untouched` |
| Cancellation (PTE-API-006) | `fitting_can_be_cancelled` |
| Bounded work (PTE-GEO-005/009) | `exhausting_the_candidate_budget_falls_back_to_the_polyline`, `the_predecessor_ladder_is_bounded_and_reaches_the_block_start`, `an_adversarial_noisy_chain_stays_bounded` |
| Measured accuracy against analytic truth | `a_circle_is_reproduced_to_well_under_a_tenth_of_a_pixel`, `an_ellipse_is_reproduced_within_its_tolerance`, `an_s_curve_keeps_its_inflection_without_looping` |

`crates/palette-tracer-geometry/tests/accuracy.rs` carries an explicit note on
what its numbers do and do not claim: they bound the *fitter's own* contribution
to the error budget, given exact samples. They are not §31.2's subpixel gates,
which measure the whole path from antialiased pixels and need §10.

## Provenance

Written from §11 and the standard mathematics it cites. The Bernstein linear
precision and convex hull properties are textbook (Farin, *Curves and Surfaces
for CAGD*, §4.3). The multi-scale chord-angle corner statistic is the classical
Rosenfeld–Johnston / Chetverikov family. The hysteresis is Canny's, applied to a
one-dimensional chain. Richardson extrapolation is standard numerical analysis.
No implementation was consulted, and `palette_trace/**/*.py` was not read
(PTE-LIC-002, ADR-0003).
