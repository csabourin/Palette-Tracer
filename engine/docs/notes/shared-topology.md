# Design note — label map to shared half-edges

**Requirements:** PTE-TOPO-001..010, PTE-TOPO-014/015, PTE-NO-009, PTE-NO-012,
PTE-NO-013, PTE-NO-014. §9, Appendix B, Appendix D.3.
**Status:** conforming (see `docs/IMPLEMENTATION_STATUS.md`).
**Code:** `crates/palette-tracer-topology/src/{grid,ambiguity,extract,validate}.rs`

## Problem

Given an exclusive label per pixel, produce a planar subdivision in which the
interface between any two regions is *one* geometric object, referenced by both
sides. Everything the engine claims about seams follows from this being true by
construction rather than by care.

## Conventions

Pixel `(x, y)` occupies `[x, x+1] × [y, y+1]`; grid vertices are the integer
points; `y` increases downward. A *dart* is a directed elementary segment, and
its left side is `(dy, -dx)` — travelling east, north is on the left. Every face
cycle is walked with the face on the left, which is what makes the twin of a
half-edge exactly the reverse traversal.

## Algorithm

1. **Decide the ambiguous cells first.** A 2×2 neighbourhood reading `A B / B A`
   admits two non-crossing connections. Each is decided by the §9.4 energy
   before any traversal, so the traversal is a lookup rather than a judgement.
2. **Find the split points.** A stub exists where the two pixels flanking it
   differ, so a vertex's degree is 0, 2, 3 or 4 — never 1. Degree 2 cannot
   change the face pair (with two stubs, the other two flanking pairs are
   equal), so a chain splits exactly at degree ≠ 2, minus the ambiguous cells,
   which the §9.4 decision turns into pass-throughs.
3. **Trace chains.** From each split point, follow the tight-turn rule — the
   first stub clockwise from the arrival that still has the same face on its
   left — until another split point. Closed loops with no split point start at
   their lexicographically smallest dart.
4. **Pair, then build.** Chain `i`'s twin starts at the reverse of chain `i`'s
   last dart. Half-edge `next` is whatever chain the dart rule continues into.
   Cycles fall out of `next`; the sign of the shoelace area separates outer
   boundaries from holes.

## Invariants

Appendix B, implemented in `validate.rs` and run unconditionally after
extraction. The load-bearing one is `shared_edge_is_one_chain`: for every
half-edge, `oriented_chain(h) == oriented_chain(twin(h)).reversed()`. If a
future fitter ever gave one side its own geometry, this fails immediately.

## Complexity

`O(N)` for the ambiguity and degree passes, `O(S)` for the chain traversal where
`S` is the boundary length, `O(E)` for cycles. Memory is `O(R + E + S)`. The
maps are keyed by grid position, so nothing is proportional to `N × K`.

## Tie rules

* Chain numbering follows grid order of the starting dart, so identifiers are a
  property of the raster (PTE-API-010).
* An exact tie in the §9.4 energy goes to the diagonal with the smaller minimum
  label identifier — a property of the labels, which survives reflection and
  rotation (PTE-TEST-010). It is emphatically not a turn direction
  (PTE-NO-012).

## Why the ambiguity energy is shaped the way it is

Continuity is the strongest term, and it asks two questions rather than one:
*does the connection matter* (are the two pixels already four-connected
nearby?) and *does the diagonal continue past the cell*. The first question is
what stops a background label, which is connected everywhere, from winning the
connection away from a one-pixel line. Where both diagonals need the connection
— a checkerboard, or a line that cuts the background in two inside the window —
continuity is silent and Kopf and Lischinski's sparse-pixel preference decides:
the rarer label connects.

## Alternatives considered

* **Marching-squares with a fixed turn.** Simplest, and exactly what PTE-NO-012
  forbids: `the_anti_diagonal_resolves_the_mirrored_way` fails under it.
* **A full DCEL with explicit vertex rings.** More general than a square grid
  needs; the four-direction stub order *is* the vertex ring.
* **Splitting at every grid vertex.** Would make every chain one segment and
  remove the need for the tight-turn rule, at the cost of one shared edge per
  pixel step and a topology that says nothing about structure.

## Limitations

* Junction positions are not optimised (§9.5). They sit where extraction put
  them. Nothing moves them, so "optimised once and shared" holds vacuously.
* A region that is diagonally self-touching can produce a face with two outer
  cycles. The validator permits it and the serialiser emits both subpaths; the
  specification does not discuss the case.
* No tiling, so no seam stitching (§8.8).
