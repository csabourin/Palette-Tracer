# ADR-0004 — MIT licence and GPL-host integration boundary

**Status:** Accepted
**Date:** 2026-08-11
**Supersedes:** ADR-0001's `MIT OR Apache-2.0` licence selection only
**Requirements:** PTE-LIC-001..005, §37.1–§37.4, PTE-ARCH-001..003

## Context

The repository contains two separately governed components. The existing
Python application is GPL-3.0-or-later. The Rust engine under `engine/` was
created clean-room as a permissive component and was initially declared
`MIT OR Apache-2.0` while the repository owner considered the final licence.

The repository owner has selected MIT. The remaining ambiguity is what
"licence boundary" means when the GPL application eventually consumes the MIT
engine. Treating the boundary as necessarily meaning a subprocess would mix a
packaging choice with the actual provenance and dependency constraints.

## Decision

Everything authored as part of the first-party engine under `engine/` is
licensed under the MIT License in `engine/LICENSE-MIT`. Cargo packages declare
the SPDX expression `MIT`; generated first-party fixtures use the same licence.
The former Apache-2.0 option and `engine/LICENSE-APACHE` are removed.

The repository root remains GPL-3.0-or-later and governs the Python
application. Files carrying their own licence or third-party notice retain
those terms.

The integration boundary has three enforceable parts:

1. **Provenance.** Engine implementation is written from `engine/SPEC.md`, its
   public references, and independent tests. GPL implementation source is not
   copied, translated, or used as an implementation template (ADR-0003,
   PTE-LIC-002/004).
2. **Dependency direction.** The MIT engine and its dependency closure do not
   import, link to, or call GPL application code. A GPL-side adapter may depend
   on the engine's public API. The engine never calls back into the host.
3. **Identification and notices.** A standalone engine release remains an
   identifiable MIT component and includes its copyright and permission
   notice. A distribution that also contains the GPL application preserves the
   engine's MIT notice and the root application's GPL terms, plus every
   third-party notice.

This decision does **not** select an integration mechanism. A subprocess CLI,
C ABI, in-process extension, or WASM adapter may all be evaluated. The chosen
adapter belongs on the GPL side, and its ADR must document the packaging,
copying, lifecycle, error, and release consequences. Process isolation is not
imposed merely to make the licence diagram look simpler.

This ADR records project policy and provenance. It is not a general legal
opinion about unrelated combinations or distributions; a materially different
distribution model requires review.

## Consequences

**Positive.** The engine has one short, familiar licence and can be embedded in
permissive or proprietary hosts subject to the MIT notice. The GPL application
can consume the MIT component without changing the engine's clean-room source
licence.

**Trade-off.** Apache-2.0's explicit patent grant and termination language are
no longer offered for first-party engine code. Third-party dependencies retain
their own licences, including Apache-2.0 where applicable.

**Contributor policy.** Contributions intentionally submitted under
`engine/` are accepted for distribution under MIT unless a file and its review
explicitly establish another compatible licence.

**Testing.** `cargo metadata` must report `MIT` for every workspace package;
`cargo deny check` remains release-blocking and continues to validate the
dependency closure rather than replacing third-party licences with MIT.

**Integration.** Selecting an adapter is now an architecture task, not a
blocked licence choice. Until that task is implemented, the Python application
and engine remain unwired.

## Alternatives considered

1. **Keep `MIT OR Apache-2.0`.** Broader downstream choice and an explicit
   patent grant. Rejected because the repository owner selected MIT only.
2. **License the engine GPL-3.0-or-later.** Simplifies a monolithic GPL
   distribution but prevents the intended permissive standalone engine.
3. **Require subprocess isolation.** Keeps packaging visibly separate, but
   imposes runtime and deployment costs without being an engine requirement.
   Rejected as a mandatory rule; it remains an available adapter design.
