# Shared subpixel junction optimisation

§34.2 design note for the junction pass in `palette-tracer-aa`.

## Requirements and scope

This pass implements the part of §9.5 and §10.4 that the per-chain subpixel
solver cannot implement:

* PTE-TOPO-011 — one position is shared by every incident curve and optimised
  once;
* PTE-TOPO-012 — independent curve smoothing keeps endpoints pinned, followed
  by a topology-preserving joint solve of the shared vertex;
* PTE-TOPO-013 — a legal move preserves incident cyclic order, crosses no
  non-incident edge, and stays inside a local displacement bound;
* PTE-AA-006 — three- and four-colour barycentric weights are evidence for the
  joint solve, not permission to change topology.

It runs after §10.5's per-chain optimisation and before §11 fitting. The fitter
therefore still sees pinned endpoints (§11.3, PTE-GEO-013); it simply sees the
subpixel shared endpoint chosen here instead of the original grid corner.

## Why barycentric weights are not a position

For an observed colour `C` and incident face colours `A_k`, §10.4 solves

```text
min_w ||C - Σ w_k A_k||²,  w_k ≥ 0,  Σw_k = 1.
```

Those weights estimate area coverage inside one pixel. They do not identify
where the several boundary lines meet: many different arrangements have the
same set of areas. Treating a weight as an `x` or `y` coordinate would add a
geometric assumption that is not in the evidence.

The position instead comes from the two-colour constraints already recovered
along each incident edge. Near a junction, incident edge `i` supplies

```text
n_i · x = t_i,
```

with confidence `c_i`. The shared position is the weighted least-squares
intersection

```text
argmin_x Σ c_i ||n_i · x - t_i||².
```

This is one deterministic 2×2 solve. Parallel evidence is rejected by a
quantised determinant gate. There is no convergence loop.

The barycentric estimate remains essential: at least one source pixel touching
the grid junction must have three active weights and an acceptable residual.
It gates the solve and scales the line confidences. Thus a crisp three-region
grid corner, whose samples are all one- or two-colour, stays pinned rather than
being moved by an accidental extrapolation.

## Evidence retained across the two passes

The independent chain pass keeps the nearest usable line constraint at each
endpoint together with the original first interior sample. The later joint
pass needs both:

* the line constraints locate the candidate after the interiors have moved;
* the original rays define the incident cyclic order that the candidate must
  preserve.

Only vertices of degree at least three with three or four distinct interior
face colours are eligible. A domain-border T junction includes the exterior,
which has no colour evidence, and remains pinned.

## Topology is a hard constraint

The candidate is accepted only if all of these hold:

1. its displacement from the extracted vertex is at most the §10.5 one-pixel
   trust radius;
2. every incident first segment remains non-degenerate;
3. every pair of original incident rays keeps its quantised orientation, and
   an opposite collinear pair cannot fold onto the same side;
4. every changed first segment avoids every segment not incident to the old
   shared vertex.

The comparisons that decide these gates use `QuantKey`. Incident edges and
face colours are visited in arena or `BTreeSet` order, so no allocation or hash
iteration order affects the result (PTE-DET-002/003/004).

On acceptance, the vertex is changed once. The same `Point` value is then
copied to the start or end of every incident `CurveChain`. The chains remain
the single shared geometry used by both half-edges (PTE-TOPO-001,
PTE-AA-008); no face-local copy exists.

## Tests and corpus evidence

`a_multicolour_junction_is_optimized_once_and_shared_exactly` renders a
linear-light, three-colour analytic T junction at `(10.35, 10.62)`. The grid
vertex begins at `(10, 11)`, the solve recovers the truth within `0.08 px`, and
all three chain endpoints compare exactly equal to the one moved vertex.

`a_four_colour_junction_is_reconstructed_as_one_shared_vertex` repeats the
same analytic gate for a four-region X junction and checks all four endpoints.

`a_junction_move_that_reorders_incident_edges_is_rejected` and
`a_junction_move_that_crosses_a_nonincident_edge_is_rejected` exercise the two
topological refusal paths directly.

The §25.3 corpus now includes `topology/subpixel-t-junction`, generated from an
analytic three-region partition at `(7.35, 7.62)`. Its protected topology is
three faces and one degree-three interior junction. `make engine-corpus`
reports three faces, six shared edges, and no fitting fallback; its three
interior boundaries are classified as coverage reconstructed.

## Declared limits

* The multi-colour gate currently uses §10.4's literal linear-light
  barycentric model. The encoded-sRGB compatibility hypothesis used for
  two-colour edges is not generalised to three or four colours.
* The line evidence is local extrapolation from the nearest constrained sample.
  A junction with barycentric evidence but fewer than two independent incident
  line constraints is reported as low confidence and stays pinned.
* The crossing guard operates on the pre-fit polylines, which is the complete
  representation at this stage. The existing validator still reruns after §10
  and again after §11 as required by PTE-GEO-014.
