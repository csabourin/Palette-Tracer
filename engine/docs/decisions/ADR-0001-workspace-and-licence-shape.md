# ADR-0001 — Workspace and licence shape

**Status:** Accepted; licence selection superseded by ADR-0004
**Date:** 2026-08-10
**Resolves:** `SPEC.md` §40 open question 1 ("Repository/license shape")
**Requirements:** PTE-LIC-001..005, §37.1, §37.3

## Context

At the time of this decision, `SPEC.md` §37.1 said the new Rust workspace
SHOULD use `MIT OR Apache-2.0`, and
§37.3 observes that the repository hosting it is GPL-3.0-or-later. §40 lists the
choice between "a separate permissive engine repository" and "a permissive
subproject hosted by the GPL application" as explicitly unresolved.

The engine is not yet useful standalone, and its first consumer is the Python
application in this repository. Splitting repositories now would cost a release
and vendoring pipeline before there is anything to release.

## Decision

The engine lives at `engine/` **inside this repository**. This ADR originally
selected `MIT OR Apache-2.0`; ADR-0004 supersedes that part of the decision and
selects MIT only. The repository root remains GPL-3.0-or-later.

* `engine/LICENSE-MIT` states the engine's terms.
* `engine/SPEC.md` is authoritative for `engine/` and only for `engine/`. The
  root `SPEC.md` remains authoritative for the Python application. The two use
  disjoint requirement-identifier schemes (`PTE-TOPO-004` versus `§20.3`), so a
  citation is never ambiguous about which contract it means.
* `engine/AGENTS.md` scopes the contributor contract to the subtree.
* Nothing in `engine/` links, imports, or is derived from anything under the
  repository root. The dependency arrow points one way only: the Python
  application may one day call the engine; the engine never calls back.

## Consequences

**Positive.** One repository, one issue tracker, one CI configuration. The
engine can be extracted to its own repository later by moving the directory,
because it has no upward dependency.

**Negative.** A reader who clones the repository sees two licences. This is
mitigated by the licence sitting beside the code it governs and by both
`README.md` files saying so.

**Compatibility.** None yet -- there is no released engine artefact.

**Test impact.** `cargo deny check` (PTE-SEC-011) enforces that the permissive
closure stays permissive. The Python test suite is untouched.

**Reversal cost.** Low. Extracting `engine/` to a separate repository is a
directory move plus a CI file; re-licensing the engine to GPL would be a header
sweep and a `deny.toml` edit.

## Alternatives considered

1. **Separate permissive repository now.** Correct end state, premature today.
   Adds cross-repository version coordination before the engine produces
   output anyone consumes.
2. **License the engine GPL-3.0-or-later to match the root.** Simplest, and it
   would permit porting the existing Python algorithms directly. Rejected: it
   contradicts §37.1 and forecloses embedding the engine in the permissive
   WASM and native hosts the specification targets (§22, §23).
