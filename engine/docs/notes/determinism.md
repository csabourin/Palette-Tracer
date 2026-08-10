# Design note — determinism

**Requirements:** PTE-DET-001..004, PTE-NO-043, §20, Appendix F.
**Status:** partly conforming — cross-target parity is not established.
**Code:** `crates/palette-tracer-core/src/{determinism,digest}.rs`

## Problem

§20.2 lists eight sources of nondeterminism. Three of them are structural and
are handled by types rather than by discipline; the rest are absent by
construction in a single-threaded engine with no clock and no random seed.

## The three that need machinery

1. **Unstable sorting of equal keys.** `MinHeap<T: Ord>` pops in an order
   defined entirely by `T`, and every entry type includes its own tie-break. The
   declared one for merging is `(cost_bin, min_region_key, max_region_key)`.
2. **Hash-map iteration order.** No `HashMap` appears in any path that reaches
   the output or the digest. The one hash in the engine is the assignment
   cache's, which is unseeded *and* exact-keyed: a hit returns what a miss would
   have computed, so residency cannot change a label.
3. **Target-specific floating point crossing a threshold.** `QuantKey` turns a
   `f64` into an integer decision key. A difference of one ulp cannot flip a
   branch; a difference that matters always does. Edge weights are quantised to
   `u16` before sorting for the same reason.

## The digest

Appendix F, over tagged length-prefixed tokens rather than serialised JSON — so
a field rename is not a digest change and map ordering is not part of the
contract. Coordinates are quantised at 10⁻⁶ px. Closed paths rotate to their
lexicographically smallest start, preserving winding, so two runs that began a
walk at different corners agree while a reversed traversal does not.

Serialisation policy is deliberately **excluded**: indentation, metadata and
decimal precision do not change the digest, because §20.1 defines it over the
quantised typed IR and Appendix E.1 classifies a precision change as
invalidating serialisation only.

## What is measured

`crates/palette-tracer/tests/determinism.rs`: repeats, concurrent traces, byte
stability, alpha-zero randomisation, palette permutation, integer translation,
and equivalent pixel formats. Plus a negative: a real change must change the
digest, or the test proves nothing.

## What is not

Cross-target parity. `cargo check --target wasm32-unknown-unknown` proves the
engine is host-free; it does not run the corpus. PTE-NO-049 is not discharged
and `docs/IMPLEMENTATION_STATUS.md` says so.

Tile-size invariance is vacuous: there is no tiling.
