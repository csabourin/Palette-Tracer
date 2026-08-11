# Subpixel antialias reconstruction

§34.2 design note for `palette-tracer-aa` and the §10.2 additions to
`palette-tracer-color::mixture`.

## Requirements

`PTE-AA-001` through `PTE-AA-009` (§10.1–§10.6). Adjacent: `PTE-SEG-015/016`
(§8.6 fringe absorption, which shares the mixture estimator), `PTE-GEO-013`
(a chain's endpoints do not move), `PTE-TOPO-001` and `PTE-AA-008` (one
boundary, fitted once, inherited by both faces), `PTE-DET-002/003/004`
(no bare float decides anything; the same decision on every target),
`PTE-AA-009` (the report distinguishes four boundary sources).

Gates: §31.2's synthetic subpixel geometry table.

---

## 1. What the diagnosis found, before any of this was written

The corpus census reported an antialiased circle tracing to eight faces and a
five-pointed star to 47. The standing assumption was that §10.3's
coverage-to-position inversion was missing. It was — but that is not what
produced the extra faces. The §8.6 fringe pass was running and being defeated
by two separate things, and the first of them would also have quietly
corrupted §10.3 had it been built on top.

### 1.1 The mixture model was in the wrong blend space

Instrumenting `reassign_fringe`, the star's fringe regions were rejected 77 to
3 on mixture residual, with residuals in a band of 0.040–0.056 against a
threshold of 0.035. A band that tight is not noise.

§10.2's estimator fits a straight line in linear-premultiplied space. Most 2D
rasterisers do not composite antialiasing there; they blend the
transfer-encoded bytes directly, and `tools/make_fixtures.py` does the same
because that is what the input the engine will see looks like. A blend that is
straight in encoded coordinates is a *curve* in linear coordinates, so a
perfectly antialiased pixel carries a systematic residual: the distance from
that curve to the chord.

The size of the residual is predictable in closed form, and predicting it is
what turned a suspicion into a diagnosis. Its maximum over coverage is

| Endpoints | Max linear-space residual |
|---|---:|
| `#202634` / `#f7f4ec` (the circle) | 0.0236 |
| `#c63e36` / `#f7f4ec` (the star) | 0.0570 |

against measured maxima of 0.024 and 0.056. The threshold of 0.035 sat exactly
between the two colour pairs. That is the whole explanation for why one fixture
absorbed 90 of 90 fringe regions and the other rejected 77 of 80: not a
difference in the shapes, but in how saturated their colours were.

**This mattered far more than the face counts.** §10.3 inverts the same
coverage the fringe gate rejects on. On encoded-blend input the linear-only
estimator's coverage is biased by **+0.22 at a true coverage of 0.5** for the
circle's colours and **+0.18** for the star's. Fed into `A_square⁻¹` that is
roughly a fifth of a pixel of position error — *larger than the grid error §10
exists to remove*, and several times §31.2's median gate. Building §10.3 on the
old estimator would have made geometry measurably worse on exactly the input
§10 targets.

### 1.2 §8.7's shape veto did not belong in §8.6

The fringe gate skipped any region whose thin-feature protection score exceeded
0.5. That inverts §8.6's own logic — §8.6 lists thinness as evidence *for* a
region being fringe, and its four conditions contain no shape veto — and it
misfired on precisely the shape antialiasing produces. Under the 8-connected
partition a three-pixel chain touching only at corners has perimeter 12 against
area 3, so `perimeter²/(16·area)` reads 3.0 where the same length of straight
one-pixel strip reads 1.33. The proxy is calibrated for 4-connected blobs and
over-scores diagonal chains by more than twice. Four such chains were what held
the circle at eight faces after everything else had been absorbed.

What separates a hairline from a fringe is colour, not shape: a dark line
between two regions is not a mixture of them, and the residual gate rejects it.

---

## 2. Estimating the compositing transfer (§10.2)

`mixture::two_color_best` scores two hypotheses and keeps the one that explains
the observation.

**Linear light** — §10.2's literal model, `C = αA + (1−α)B`, projected in
linear premultiplied coordinates.

**Encoded sRGB** — the source composited on transfer-encoded values. The
observation is then linear in *encoded* coordinates, so the coverage is the
projection there; the prediction is decoded back before the residual is taken.

`PTE-AA-001` holds throughout. Every residual compared, and every residual
reported, is linear-light premultiplied. Only the parameter search for the
encoded hypothesis happens in encoded coordinates, because that is what the
hypothesis *is*; the error metric that judges it never leaves linear light.

### 2.1 The asymmetry, and why it is not caution

The two hypotheses are not peers. §10.2 states the linear model, so it is the
default and carries no burden of proof; the encoded hypothesis is a departure
from the specification's model and has to be paid for.

That asymmetry is doing real work. When the two side colours differ mainly in
luminance and sit near the neutral axis, all three channels scale almost
together, the encoded blend curve lies within quantisation of the encoded
straight segment, and **both hypotheses fit to within a code point**. Their
coverages do not agree even so — for a near-neutral pair separated by a factor
of forty in luminance they differ by about 0.07 at mid-coverage. Picking the
smaller of two residuals that differ only by rounding would make that a coin
flip, and §10.3 turns coverage straight into position.

This was not hypothetical: it is what the first draft did, and the synthetic
vertical-edge test caught it returning 0.919 where the truth was 0.845.

So the encoded hypothesis wins only when

* the linear residual is above a quantisation floor of `4e-3` — one 8-bit code
  point in linear light near mid-range — so the linear model has *visibly*
  failed rather than merely failed by rounding; and
* the encoded residual is below half the linear one, so it explains what is
  left materially better.

Both comparisons go through `QuantKey`, never bare floats (PTE-DET-002/003).
A tie keeps linear light. This is PTE-AA-002's rule — ill-conditioned means
fall back to the declared estimate, not amplify — applied to *model selection*
rather than to the fit.

---

## 3. Inverting coverage to a position (§10.3)

### 3.1 Conventions

The pixel is the unit square `[−½, ½]²` centred on the origin. `n` is the unit
normal pointing **out of `A`**, so the `A` side is `{x : n·x ≤ d}` and coverage
increases with `d`. `d = 0` is a line through the pixel centre covering exactly
half.

In image space, the raster pixel `(px, py)` occupies the cell `[px, px+1] ×
[py, py+1]`, so its centre is at `(px + ½, py + ½)`. The corpus generator's
convention differs by half a pixel on both axes — it samples pixel `(px, py)`
over `[px − ½, px + ½]` — and `tests/subpixel_gates.rs` states and applies that
shift in exactly one place, because getting it wrong moves the measured centre
by `0.71 px` and fails every gate for the wrong reason.

### 3.2 The closed form

§10.3 offers three implementations: exact piecewise analytic intersection, a
monotone precomputed table with interpolation, or a bounded root solve. This is
the first, and it is the *cheapest* of the three rather than the dearest.

The unit square is invariant under the dihedral group of order eight, so the
area cut off by a half-plane depends only on the sorted absolute values of the
normal's components. With `a = max(|nₓ|,|n_y|)`, `b = min(|nₓ|,|n_y|)`,
`a² + b² = 1`:

```
                 0                                d ≤ −(a+b)/2
A(d) =   (d + (a+b)/2)² / (2ab)         −(a+b)/2 ≤ d ≤ −(a−b)/2
         ½ + d/a                        −(a−b)/2 ≤ d ≤  (a−b)/2
         1 − ((a+b)/2 − d)² / (2ab)      (a−b)/2 ≤ d ≤  (a+b)/2
                 1                                d ≥  (a+b)/2
```

Two corner triangles and a trapezoid. Each piece inverts in closed form:

```
d(α) =  −(a+b)/2 + √(2ab·α)                    α ≤ b/(2a)
         a(α − ½)                     b/(2a) ≤ α ≤ 1 − b/(2a)
         (a+b)/2 − √(2ab(1−α))        1 − b/(2a) ≤ α
```

One square root, no iteration. That matters beyond speed: PTE-DET-004 needs the
same *decision* on every target, and neither a table's interpolation nor a root
solve's termination test is guaranteed to land identically on two platforms.
IEEE-754 requires `sqrt` to be correctly rounded, so this inverse is
bit-identical wherever it runs.

Monotonicity and reversal symmetry (PTE-AA-003) hold *exactly*, by
construction, rather than to within a tolerance: each branch is increasing, the
branches agree at their shared bounds, and `d(1−α) = −d(α)` because the corner
branches are reflections and the middle branch is odd about `α = ½`.

When `b = 0` the normal is axis-aligned and `2ab = 0`, but both outer branches
are unreachable because `b/(2a) = 0`. Testing the branch bounds rather than the
denominator makes the degenerate case fall out instead of being special-cased.

### 3.3 Bounding is a property, not a clamp

The result always lies within `±(a+b)/2`, the pixel's own extent along the
normal, which is at most `1/√2`. PTE-AA-005's "recovered offsets MUST be
bounded to the local pixel support" is therefore satisfied by the formula
rather than by a clamp applied afterwards.

---

## 4. Normals (PTE-AA-004)

`PTE-AA-004` forbids inferring the normal "from one noisy pixel alone" and
permits "multi-pixel edge evidence or current boundary tangents". This build
takes the second: §10 runs after topology extraction, so the boundary already
exists as a polyline and the tangent is the chord `x_{i+w} − x_{i−w}`, rotated
a quarter turn.

The window `w = 2` is the smallest for which a 45° staircase reads as the
diagonal rather than as alternating axis directions — and a 45° edge is exactly
where grid snapping costs the most. `a_one_sample_window_would_alternate_
between_axis_directions` measures what `w = 1` would do, so the requirement is
demonstrably doing work rather than restating what the code would do anyway.

At a chain end the window is truncated rather than wrapped: the endpoint is a
junction shared with other chains, and the samples beyond it belong to a
different boundary (PTE-TOPO-001).

**Stability** is the agreement between the wide chord's normal and a narrow
one's, floored at zero. Where they disagree the boundary is turning inside the
window and the tangent is not a reliable description of it. This is PTE-AA-005's
"normal stability" term, and it is the same guard the §11 fitter applies to
Richardson extrapolation, for the same reason.

---

## 5. Boundary optimisation (§10.5)

§10.5 minimises

```
E = Σ cᵢ ρ(nᵢ·xᵢ − dᵢ) + λ_s Σ ‖x_{i−1} − 2xᵢ + x_{i+1}‖² + λ_t E_topology + λ_p E_pins
```

### 5.1 Two terms are constraints, not penalties

`E_topology` and `E_pins` are enforced as hard constraints. Chain endpoints are
junctions shared with other chains and do not move at all (PTE-GEO-013; and
PTE-TOPO-011's junction optimisation is not implemented, so a moved endpoint
would silently tear the boundary apart at the junction). Every interior point
is confined to a trust region of one pixel around its source position
(PTE-AA-007).

An infinite penalty is a constraint, and a constraint is cheaper and cannot be
traded away by a large enough smoothness term.

### 5.2 One unknown per sample

Points move only along their own normals — a sample sliding *along* the
boundary carries no information and would only redistribute the
parameterisation — so the unknown at sample `i` is one scalar `sᵢ`. The
objective is then quadratic with tridiagonal-plus-diagonal structure and is
solved by **eight Gauss-Seidel sweeps**.

A fixed count, not a convergence test. "Iterate until the residual stops
changing" is exactly the kind of rule that lands differently under a different
floating-point evaluation order, and PTE-DET-004 needs the same decision on
every target. Eight sweeps propagates the smoothness term across the tangent
window, which is as far as any single sample's evidence is relevant; the
homogeneous decay per sample is about 0.13, so the boundary layer around a
pinned endpoint is spent within two samples.

### 5.3 The robust `ρ` is the weight

§10.5's `ρ` is realised by the confidence `cᵢ` rather than by a redescending
loss. A sample whose mixture residual is large, whose colours barely separate,
or whose normal is unstable gets a small weight and cannot drag the boundary.
That keeps the objective quadratic, which keeps the solve closed and the
iteration count honest.

### 5.4 Two rules that came from being wrong first

**A saturated pixel is a perfect mixture and locates nothing.** A pixel lying
wholly on one side fits at coverage 0 or 1 with residual zero, so it wins any
contest ranked by residual — and its inverted offset is the pixel's own edge.
Admitting it pins the boundary back onto the grid, reproducing exactly what
§10.1 calls insufficient. Candidates are therefore restricted to coverages at
least `0.03` inside `[0, 1]`, a band that also clears 8-bit quantisation. The
first draft did not do this and reconstructed nothing at all.

**There is no per-edge quorum.** An earlier draft required half an edge's
samples to carry coverage before any of them were used. That threw away the
corners of `curves/rounded-rectangle`: its sides are axis-aligned runs of
saturated pixels with no coverage to read, so the whole boundary — corners
included — fell back to the grid. Each constrained sample is justified on its
own and bounded by its own trust region, so the right unit of decision is the
sample, not the edge. Unconstrained samples simply do not move, which is the
correct answer for a boundary that really is on a cell interface.

---

## 6. Reporting (PTE-AA-009)

Three outcomes per edge, decided from per-sample evidence, and they are not
interchangeable:

* `coverage_reconstructed` — at least one sample's coverage was inverted.
* `low_confidence_fallback` — a mixture was found and then failed a residual,
  confidence, or trust test. §10.6's crisp estimate stands.
* `crisp_grid` — no pixel here is a mixture of the two sides at all. The
  boundary is genuinely crisp, and the grid is the right answer rather than a
  fallback.
* `pixel_art_policy` — set by the profile; §10 never runs (PTE-TOPO-010, §15).

"No antialiasing here" and "antialiasing I could not trust" are different facts
about the input, and only the second is a reason to doubt the boundary.

The report's census now counts these from the edges themselves. It previously
derived them from the configured profile, which could only ever restate the
configuration — `coverage_reconstructed` was hard-coded to zero.

An edge's confidence is the mean over its **interior** samples, not over its
constrained ones. An edge where two samples of forty carried coverage is barely
reconstructed, and `0.05` reports that; averaging over the two would report
`0.98` and hide it.

---

## 7. Complexity and cancellation

Reconstruction is `O(S)` in boundary samples: each sample examines at most four
pixels, and the solve is a fixed eight sweeps. Memory is one displacement and
one constraint per sample of the chain currently being processed — nothing
proportional to the image, and none of §19.4's forbidden shapes.

`check_cancel` runs once per shared edge. Chains are bounded by the topology's
own limits, so the interval between checks is bounded.

---

## 8. Results

§31.2's gates, on `curves/circle-subpixel-0` and `-1` regenerated from their
analytic descriptions, at a curve tolerance of `0.10 px` (see §9):

| Metric | Gate | circle-0 | circle-1 |
|---|---:|---:|---:|
| Median boundary normal error | `≤ 0.10 px` | 0.028 | 0.055 |
| p95 boundary normal error | `≤ 0.35 px` | 0.155 | 0.262 |
| Max error (no junctions to exclude) | `≤ 0.75 px` | 0.391 | 0.537 |
| Circle centre error | `≤ 0.20 px` | 0.001 | 0.032 |
| Circle relative radius error | `≤ 1%` or `0.20 px` | 0.092% | 0.106% |

The circles have one closed boundary and no junctions at all, so the max gate
applies with nothing excluded — the strictest reading available.

Corpus effect, over the whole §25.2 suite:

```
                        before §10          after §10
circle-subpixel-0    52 lines + 26 cubics   16 lines +  4 cubics   1285 -> 709 bytes
circle-subpixel-1    66 lines + 14 cubics    8 lines +  6 cubics   1107 -> 677 bytes
star-acute-corners  172 lines + 12 cubics   20 lines + 20 cubics   1636 -> 1104 bytes
rounded-rectangle    58 lines +  8 cubics   12 lines +  8 cubics    892 -> 616 bytes
```

Every other fixture is byte-identical.

---

## 9. Known limitations

1. **§31.2's gates and `geometry.curveTolerancePx` are budgets for the same
   quantity.** The fitter is *permitted* to deviate from its samples by up to
   the tolerance, so a profile configured above a gate cannot meet that gate
   however exact the reconstruction underneath it is. `flat-illustration`
   defaults to `0.6 px`, six times §31.2's median target, and at that setting
   circle-1 measures a median of 0.103 and a p95 of 0.392 — both breached, by
   the fitter's licence rather than by reconstruction error. The centre and
   radius gates survive it, because fitting error is signed and averages out
   over a closed curve; the pointwise gates do not. The gates are therefore
   measured at `0.10 px`, and
   `the_gates_are_a_budget_shared_with_the_fitters_tolerance` records the
   interaction with numbers rather than leaving it implied. **No profile
   default was changed and no gate was loosened.**

2. **A crisp staircase is still a staircase.** `curves/shallow-staircase` is
   generated without antialiasing, so there is no coverage to invert and §10
   correctly leaves it alone — it remains 8 segments. Limitation 1 in
   `curve-fitting.md` is therefore *narrowed* rather than removed: split points
   no longer sit on source samples where antialias evidence exists, but a
   genuinely hard-edged 45° staircase still cannot become one line below a
   tolerance of `1/√2`. §10 fixes the antialiased case, which is the common
   one, and cannot fix the un-antialiased case, which carries no information.

3. **The transfer is estimated per pixel, not per image.** Two neighbouring
   pixels on the same boundary can in principle choose different hypotheses.
   The margin rule makes that rare, and the §10.5 smoothness term absorbs a
   single disagreeing sample, but nothing yet pools the decision over an edge
   or an image, which would be strictly stronger evidence.

4. **Only the two-colour case.** §10.4's barycentric junction weights exist in
   `color::mixture` and are tested, but reconstruction does not consult them:
   junction *positions* are PTE-TOPO-011/012/013, which is not implemented, so
   there is nothing yet to feed them to. Chain endpoints are pinned.

5. **Chains that are already curves are skipped.** §10 runs before §11, so
   every chain is a polyline in practice. A cubic chain is left alone rather
   than approximated, because moving control points is a different problem with
   a different error bound.

6. **`λ_s = 0.35`, eight sweeps, a trust radius of one pixel, a coverage band
   of `0.03`, a quantisation floor of `4e-3` and a margin of `0.5` are
   engineering choices**, chosen against the corpus and reasoned about above,
   not measured optima. They join the list in
   `docs/IMPLEMENTATION_STATUS.md`'s "No calibration" gap.

---

## 10. Provenance

The square/half-plane intersection is elementary plane geometry; the octant
reduction, the piecewise inverse, the transfer-hypothesis test and its margin
rule, and §10.5's reduction to one unknown per sample are derived here from
§10's statement of the problem. §10.5's objective is quoted from the
specification. No third-party geometry or colour library is used (PTE-ARCH-011),
and nothing under `palette_trace/` was read (PTE-LIC-002, ADR-0003).

## 11. Tests

`crates/palette-tracer-aa`, 23 tests:

* `the_inverse_recovers_the_offset_that_produced_the_coverage`,
  `the_forward_map_recovers_the_coverage_that_produced_the_offset` — PTE-AA-003
  composition, over all octants.
* `the_inverse_is_monotone_in_every_octant`,
  `reversing_the_labels_mirrors_the_offset` — PTE-AA-003 monotonicity and
  symmetry.
* `every_offset_lies_within_the_pixel_support` — PTE-AA-005 bounding.
* `half_coverage_is_the_pixel_centre_for_every_normal`,
  `an_axis_aligned_normal_gives_the_elementary_answer`,
  `a_diagonal_normal_is_two_corner_triangles_with_no_trapezoid` — the cases
  whose answers can be written down independently.
* `a_forty_five_degree_staircase_reads_as_a_diagonal_not_as_axis_steps`,
  `a_one_sample_window_would_alternate_between_axis_directions` — PTE-AA-004.
* `stability_falls_where_the_boundary_turns_a_corner` — PTE-AA-005.
* `reversing_the_chain_negates_every_normal` — PTE-TOPO-001.
* `a_subpixel_vertical_edge_is_recovered_to_its_true_position`,
  `reconstruction_beats_the_grid_position_it_replaces` — end to end on a case
  whose answer is known exactly.
* `no_sample_leaves_its_trust_region` — PTE-AA-007.
* `the_endpoints_of_a_chain_never_move` — PTE-GEO-013.
* `indistinguishable_sides_produce_no_constraint` — PTE-AA-002.

`crates/palette-tracer-color/src/mixture.rs`:

* `an_encoded_space_blend_is_recognised_and_its_coverage_recovered`
* `the_linear_hypothesis_alone_is_biased_by_a_fifth_of_a_pixel`
* `a_linear_light_blend_still_chooses_the_linear_hypothesis`
* `the_transfer_choice_is_symmetric_under_swapping_the_sides`
* `inseparable_endpoints_have_no_estimate_in_either_space`
* `a_transparent_endpoint_falls_back_to_the_linear_hypothesis`

`crates/palette-tracer-segment/src/rag.rs`:

* `an_antialiased_hairline_is_not_absorbed_because_it_is_not_a_mixture`

`crates/palette-tracer/tests/subpixel_gates.rs`, the §31.2 measurement.
