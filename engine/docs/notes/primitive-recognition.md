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
only when it replaces at least four generic segments, making description
complexity materially smaller. If the opposite face is opaque, that face
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

Residual is radial displacement. The maximum must not exceed the resolved
`geometry.curveTolerancePx`, and p95 must not exceed half that tolerance. These
are hard gates, not objective weights. In particular, the analytic radius-20
diagnostic exposed an arbitrary loop-closure sample displaced by `0.65 px`;
the recognizer correctly refused it under logo's `0.45 px` bound instead of
loosening the profile. The committed radius-28 fixture passes when measured at
`0.60 px`, with its centre and radius still inside §31.2's `0.20 px` gates.

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

The end-to-end tests regenerate `curves/circle-subpixel-1`'s analytic raster
(centre `(50.37, 40.61)`, radius `28`, 16×16 box supersampling) and traverse
the real segmentation, topology, §10 reconstruction, fitter, typed document,
semantic digest and SVG writer.

## Known limitations

The recognizer is circle-only and requires a complete one-edge face boundary.
It does not pool repeated radii, infer hidden support, recognise ellipses or
regular polygons, or convert a generic partial arc to §11.4's arc model. Its
thresholds are engineering choices measured by the analytic fixture, not a
corpus calibration. The seam diagnostic is still first-party; §18.7's
cross-renderer matrix remains unclaimed.

## Provenance and licence

The algebra is the standard Kåsa circle fit described in the open literature;
the SVG endpoint-arc conversion follows W3C SVG 1.1 Appendix F.6.5, already a
specification citation in `engine/SPEC.md`. The implementation was written
from those mathematical descriptions and §11.7. No external implementation or
`palette_trace/**/*.py` was consulted (PTE-LIC-002, ADR-0003). The workspace
remains MIT-only and adds no dependency.
