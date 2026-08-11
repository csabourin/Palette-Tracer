# ADR-0003 — Clean-room provenance for the permissive engine

**Status:** Accepted
**Date:** 2026-08-10
**Requirements:** PTE-LIC-002, PTE-LIC-004, §37.2, §37.3, PTE-NO-044

## Context

The engine is MIT (ADR-0004). The Python application in the
same repository is GPL-3.0-or-later and implements overlapping ideas: OKLCH
colour reaches, pinned palette entries, deterministic constrained
quantisation, destination presets.

§37.3 gives three ways to reconcile this: obtain rights-holder agreement for
dual-licensed reuse, re-express the behaviour clean-room, or license the
successor under GPL. PTE-LIC-002 forbids copying, translating, mechanically
porting, or using GPL source as a line-by-line template for the permissive
core. PTE-NO-044 restates the same rule as a forbidden shortcut.

The repository owner selected the clean-room route.

## Decision

Engine code is written from `engine/SPEC.md` and from the public references it
cites (§42), and **not** from the Python implementation.

Concretely, while authoring anything under `engine/`:

* `palette_trace/**/*.py` is not read, quoted, transliterated, or consulted for
  structure, constants, thresholds, or naming.
* The mathematics comes from `engine/SPEC.md`, which states the sRGB transfer
  function (§7.1), the OKLab matrices (§7.1), the reach score (§7.4), the merge
  identity (§8.5), the coverage inversion (§10.2--10.3), and the fitting
  objectives (§11) explicitly and in full. Where the specification cites a
  paper, the paper is the source (§42).
* Every algorithm module carries a provenance line in its rustdoc naming the
  specification section and any paper that informed it, and stating that the
  implementation was written independently (PTE-LIC-004).
* No dependency may introduce a GPL or LGPL closure. `deny.toml` enforces the
  allowlist and `cargo deny check` is release-blocking.

Before an integration adapter exists, the only contact between the trees is
documentation: the root `README.md` and `SPEC.md` each carry a pointer to
`engine/`, and this ADR describes the Python side from its published
specification, not its source. A future adapter may live on the GPL side and
call the engine's public interface. That runtime dependency does not permit the
engine implementation to read, copy, import, or call back into GPL code; the
dependency and provenance arrows remain one-way (ADR-0004).

## Consequences

Some work is redone that could have been transliterated -- the colour
conversions and the reach model in particular. That is the cost of the licence
boundary, and it is small relative to the engine's genuinely new parts (shared
topology, coverage inversion, bidirectional curve validation), which have no
Python counterpart to copy anyway.

The two implementations may diverge in edge-case behaviour. That is acceptable
and expected: they answer to different specifications. Nothing in this
repository asserts that the engine reproduces the Python tool's output.

**Reversal cost.** High and one-way. If the rights holder later grants
dual-licensed reuse of the Python code, that is a new ADR and the boundary can
be relaxed going forward -- but code already written clean-room stays clean.
Going the other direction, after a port, would require rewriting.

## Alternatives considered

1. **Rights-holder grant for dual-licensed reuse.** Available in principle --
   the repository has a single principal author -- but it requires a
   contributor inventory first (§37.3 makes Phase 0 responsible for one), and
   it was not the direction chosen.
2. **License the engine GPL and port freely.** Covered and rejected in
   ADR-0001.
