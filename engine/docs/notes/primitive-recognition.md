# Complete-circle primitive recognition

Design note for the first §11.7 vertical slice, with the §34.2 contents.

**Requirements:** PTE-GEO-010, PTE-GEO-011, §11.7, §31.2 and §31.5. The
shared-neighbour lowering also preserves PTE-TOPO-001 and PTE-SVG-009.

**Scope:** complete circles only. Rectangles, rounded rectangles, ellipses,
regular polygons, repeated radii and the §11.4 arc candidate remain outside
this slice and are named in `docs/IMPLEMENTATION_STATUS.md`.

## Placement and semantic contract

Recognition reads §10's reconstructed polylines before §11's generic fitter
replaces them. It does not mutate `Topology`: a `PrimitiveRecognition` records
the source face, shared edge, source sweep and a typed `Circle { center,
radius }`. That record crosses the geometry/SVG crate boundary and becomes
`Element::Primitive(Primitive::Circle)` only in lowering. The semantic digest
therefore hashes a circle, not a cubic approximation (PTE-GEO-011).

The fitted generic chain is still computed. Lowering accepts the recognition
only when the generic description costs at least twice the primitive's in
emitted coordinates — three for a circle's `cx`, `cy`, `r`. Counting *segments*
instead made recognition non-monotonic in tolerance: a looser bound lets the
fitter compress a circle to three cubics, and a three-segment chain then failed
a `< 4` test, so widening the tolerance turned a semantic circle back into
curves. Three cubics is twenty coordinates against three;
`a_looser_tolerance_never_withdraws_a_recognized_circle` holds the property. If the opposite face is opaque, that face
traverses the same recognition as two exact SVG circular arcs. It is not left
on the independently approximated cubics. The semantic `<circle>` and its
neighbour therefore share one analytic boundary even though their SVG element
types differ.

## Mathematics and coordinates

Coordinates are `f64` source pixels, y down, as in `core::ir::geom`. The chain
must be one complete closed edge carrying the only outer cycle of an interior
face and contain at least 16 distinct support samples.

For samples `(x_i, y_i)`, first subtract their centroid and divide by their RMS
distance. In those dimensionless coordinates `(u_i, v_i)`, fit Kåsa's circle

```text
u² + v² + D·u + E·v + F = 0
```

Because `Σu = Σv = 0`, the centre terms are the 2×2 system

```text
[Σu²  Σuv] [D] = [-Σu(u²+v²)]
[Σuv  Σv²] [E]   [-Σv(u²+v²)]
```

and the normalised centre is `(-D/2, -E/2)`. The emitted radius is the mean
geometric distance to that centre, transformed back to source pixels. No
iteration or pivot selection is involved. The scale-free determinant gate is
`det(A)/(trace(A)²) >= 1e-3`.

Support is measured by the signed angles of successive centre-to-sample rays.
Acceptance requires at least `0.95` turn, at most `1.10` total absolute turn,
and no individual angular gap above `0.10` of a revolution. That last gate is
what prevents a partial arc closed by one chord from impersonating a full
circle.

Residual is radial displacement, gated three ways: p95 at half the resolved
`geometry.curveTolerancePx`, p99 at the tolerance itself, and the maximum at
half again beyond it. These are hard gates, not objective weights.

The top one is trimmed on purpose. Binding the *maximum* straight to the
tolerance lets one sample veto the shape, and it did: the committed
`curves/circle-subpixel-0` has p95 `0.134 px` over 224 samples and a single
loop-closure sample at `0.617 px`, so under logo's `0.45 px` the engine refused
its own analytic circle on the strength of 1 sample in 224. PTE-GEO-010 binds
displacement to the resolved tolerance, which p99 does; the maximum stays as a
backstop so no sample may wander far even though one may sit outside the bound.
`one_outlying_sample_does_not_veto_an_otherwise_exact_circle` and
`a_sample_far_outside_the_bound_still_refuses_the_circle` hold both halves.
The fixture is now recognised at logo's own default, and the §25.3 census
records it: `curves/circle-subpixel-0` is one primitive and two neighbour arcs,
473 bytes against 619 for the generic form.

All branch decisions above compare `QuantKey` values at `GEOMETRY_SCALE`.
Face, edge and element order remains the existing raster-derived order.

## Invariants and failure modes

* Topology is never changed by recognition.
* Only a one-edge complete outer cycle is eligible. Partial arcs, holes,
  multi-edge boundaries and open chains are refused.
* A singular/non-finite solve, insufficient support, excessive radial error or
  insufficient complexity reduction leaves the generic fit untouched.
* Both incident faces consume one `PrimitiveRecognition`; an opaque neighbour
  is lowered as exact arcs and never as the old cubic boundary.
* Both are *written* from one set of numbers. The serialiser snaps an accepted
  circle onto the chosen decimal grid and derives the neighbour's arc endpoints
  and radii from the snapped `cx`, `cy`, `r`, so `cx' ± r'` are exact multiples
  of the grid. Rounding the two elements independently pulled them apart: the
  circle's centre landed on one value while the arc endpoints rounded to a pair
  with another midpoint, and — the arcs being exact semicircles, so their chord
  *is* the diameter — the rounded chord exceeded twice the rounded radius in
  about a quarter of cases, at which point SVG 1.1 F.6.6.2 obliges the renderer
  to scale the radii up and draw a circle that is not the one beside it.
* Negative/non-finite primitive radii fail `VectorDocument` validation before
  serialisation.
* Reversing the evidence preserves quantised centre and radius; only source
  sweep flips.

## Complexity, cancellation and budget

For `n` samples on a candidate face, fitting, support and residual measurement
are each one linear pass; sorting residuals is `O(n log n)`. Memory is `O(n)`
for the residual vector and `O(p)` for `p` accepted recognitions. Lowering
builds `BTreeMap`s keyed by face and edge, so matching is `O(log p)` per face
or boundary rather than the quadratic full scan found during PR #22's QA.

Cancellation is checked once per face. Each tested face charges one
`CURVE_CANDIDATE` plus one `BOUNDARY_SEGMENT` per sample to the global work
budget. `primitive_recognition_charges_the_work_budget` proves exhaustion is
reported from stage `fit`.

## Alternatives considered

* **Recognise from fitted cubics.** Rejected: flattening them would measure the
  fitter's approximation and discard the denser reconstructed evidence.
* **Replace only the foreground face.** Rejected: an opaque neighbour would
  retain cubics and create a crack or overlap against the exact circle.
* **Always permit the §31.2 maximum of 0.75 px.** Rejected: PTE-GEO-010 binds
  displacement to the resolved profile tolerance; a clean-looking fixture is
  not permission to override it.
* **Non-linear geometric least squares.** Deferred. A fixed-iteration refinement
  could reduce algebraic bias, but complete circles already meet the analytic
  centre/radius gates and the closed-form solve has a smaller determinism and
  denial-of-service surface.
* **A sibling-crate dependency from SVG to geometry.** Rejected under ADR-0002.
  The paint-free semantic record lives in the contracts crate instead.

## Tests and measured fixtures

| Claim | Evidence |
|---|---|
| Closed-form recovery | `a_complete_circle_is_recovered_in_source_coordinates` |
| Partial support refused | `a_partial_arc_is_not_misrepresented_as_a_complete_circle` |
| Non-circular shape refused | `a_square_fails_the_radial_displacement_gate` |
| Reverse equivalence | `recognizing_a_reversed_circle_is_semantically_equivalent` |
| Work budget | `primitive_recognition_charges_the_work_budget` |
| Real extractor to semantic IR/SVG | `the_real_extractor_emits_a_semantic_circle` |
| Opaque shared neighbour has no crack/overlap | `the_opaque_neighbour_reuses_the_exact_circle_as_arcs` |
| Invalid radii rejected | `an_invalid_circle_radius_fails_the_finiteness_gate` |
| The written arcs and the written circle are one boundary | `the_written_arcs_and_the_written_circle_are_the_same_numbers` |
| The seam diagnostic can actually fail | `a_circle_that_disagrees_with_its_neighbour_is_detected` |
| Recognition is monotone in tolerance | `a_looser_tolerance_never_withdraws_a_recognized_circle` |
| One outlier does not veto a circle | `one_outlying_sample_does_not_veto_an_otherwise_exact_circle` |
| A far sample still does | `a_sample_far_outside_the_bound_still_refuses_the_circle` |
| Refused where it cannot apply | `recognizing_primitives_in_a_coloring_book_is_refused` |

The end-to-end tests raster the same analytic circle as
`curves/circle-subpixel-1` (centre `(50.37, 40.61)`, radius `28`, 16×16 box
supersampling) but under the *opposite* pixel-centre convention:
`tools/make_fixtures.py` shades pixel `px` over `[px, px+1]`, these tests over
`[px−0.5, px+0.5]`, which is why they carry a `0.5` correction. They are not
the committed fixture and do not stand in for it. They traverse the real
segmentation, topology, §10 reconstruction, fitter, typed document, semantic
digest and SVG writer.

## Known limitations

The recognizer is circle-only and requires a complete one-edge face boundary.
It does not pool repeated radii, infer hidden support, recognise ellipses or
regular polygons, or convert a generic partial arc to §11.4's arc model. Its
thresholds are engineering choices measured by the analytic fixture, not a
corpus calibration. The seam diagnostic is still first-party; §18.7's
cross-renderer matrix remains unclaimed.

Two limits are worth naming precisely, because they decide how often the
feature fires at all.

**Segmentation can remove the candidate before recognition sees it.** The
precondition is one outer cycle carried by one closed shared edge.
`curves/circle-subpixel-1` meets it under `flat-illustration` (2 faces) and
*not* under `logo` (3 faces / 4 edges), where an antialias fringe ring
survives — so the profile that switches recognition on is also the one whose
segmentation can block it, and that fixture is never recognised at any
tolerance. Nothing here changes segmentation to suit recognition; a future
slice should decide whether a fringe ring around a recognised primitive is
absorbed or tolerated.

**Recognition still misses many analytic circles.** Sweeping 24 circles (six
sub-pixel centres × four radii) at `0.8 px`: 14 recognised, 10 not — most of
the misses being the segmentation case above rather than the residual gates.

## Provenance and licence

The algebra is the standard Kåsa circle fit described in the open literature;
the SVG endpoint-arc conversion follows W3C SVG 1.1 Appendix F.6.5, already a
specification citation in `engine/SPEC.md`. The implementation was written
from those mathematical descriptions and §11.7. No external implementation or
`palette_trace/**/*.py` was consulted (PTE-LIC-002, ADR-0003). The workspace
remains MIT-only and adds no dependency.
