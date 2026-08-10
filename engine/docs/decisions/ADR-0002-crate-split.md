# ADR-0002 — Crate split: types in `core`, orchestration in the facade

**Status:** Accepted
**Date:** 2026-08-10
**Requirements:** PTE-ARCH-001..012, §5.1, §5.2, §6.2

## Context

`SPEC.md` §5.1 describes `palette-tracer-core` as "Host-free orchestration,
config, IR, reports" and lists twelve sibling crates, several of which
(`-color`, `-segment`, `-topology`, `-geometry`, `-svg`) implement pipeline
stages whose inputs and outputs are the config and IR types.

Those two roles cannot live in one crate. If `core` owns `TraceConfig` and
`VectorDocument`, then `palette-tracer-color` must depend on `core` to name its
own arguments. If `core` also orchestrates, it must depend on
`palette-tracer-color`. Cargo rejects the cycle.

## Decision

Keep every crate name from §5.1 and split only the *role* of `core`:

* **`palette-tracer-core`** owns types and contracts, and depends on no sibling
  crate. It holds `ImageView`/`PixelFormat` (§6.1), `TraceConfig` /
  `EffectiveConfig` / `Profile` (§6.3), the vector IR (§6.4), `TraceReport`
  (§6.5), the `TraceError` taxonomy (PTE-ARCH-007), `ResourceLimits` (§24.1),
  `TraceControl` and `WorkBudget` (PTE-API-006), the semantic digest
  (Appendix F), and the shared determinism utilities.
* **Stage crates** (`-color`, `-segment`, `-topology`, `-geometry`, `-svg`)
  depend on `core` and on nothing else in the workspace. A stage crate never
  depends on another stage crate; the facade wires them.
* **`palette-tracer`** is the facade. It depends on all of the above and
  provides `Engine::{validate_config, analyze, segment, vectorize, trace}`
  exactly as §6.2 specifies.
* **`palette-tracer-cli`** and **`palette-tracer-wasm`** are host adapters and
  depend only on the facade.

## Consequences

The §5.2 dependency rules are preserved and are now mechanically checkable: a
`cargo tree` that shows `palette-tracer-core` depending on any sibling is a
defect. Host isolation (PTE-ARCH-001) holds because `core` has no host
dependency to inherit.

The names `palette-tracer-gradients`, `-fabrication`, `-codecs`, `-capi`, and
`-bench` are reserved for their §5.1 meanings and slot in beside their siblings
without renaming anything that exists.

The one visible cost is that the crate called "core" does not contain the
`trace` function. `engine/README.md` says so in its first section.

**Reversal cost.** Low, and localised: merging the facade back into `core`
would be a file move if the cycle ever became representable.

## Alternatives considered

1. **A separate `palette-tracer-ir` crate below `core`.** Works, but adds a
   crate the specification does not name and leaves `core` as a thin
   re-export. Rejected as more surface for no isolation gain.
2. **One crate for everything, modules instead of crates.** Cheapest to build,
   but §5.2's dependency rules become unenforceable -- nothing would stop a
   colour routine from reaching into the SVG serialiser. Rejected: the
   boundaries are the point.
