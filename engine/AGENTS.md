# Working instructions for agents, in `engine/`

`SPEC.md` in this directory is the authoritative implementation contract for
everything under `engine/`. Where this file and that one appear to conflict,
`SPEC.md` wins and this file is wrong.

The repository root has its own `SPEC.md` and `AGENTS.md`, which govern the
Python application. They do not apply here, and this one does not apply there.
The two use disjoint identifier schemes — `PTE-TOPO-004` here, `§20.3` there —
so a citation is never ambiguous.

## Orientation

1. `docs/IMPLEMENTATION_STATUS.md` — what is built, what is not, and what has
   actually been measured. Read this before believing anything else.
2. `docs/decisions/` — the ADRs bind you.
3. The `SPEC.md` sections your change touches. Do not read all 3,690 lines to
   make a small change.
4. `docs/notes/` — design notes for individual algorithms (§34.2).

## Commands

```bash
cargo test --workspace                                    # 417 tests + 2 doctests
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --check
cargo check --workspace --target wasm32-unknown-unknown   # PTE-ARCH-003
cargo deny check                                          # PTE-LIC-005
```

From the repository root, `make engine-test`, `make engine-lint`,
`make engine-wasm` and `make engine-deny` run the same things. `cargo deny`
needs `cargo install cargo-deny --locked` once.

## Rules that come from the specification

These are not style preferences.

* **§0.2, the evidence rule.** Code is not completion. A requirement is done
  when it has an implementation, tests at the right levels, and a row in
  `docs/IMPLEMENTATION_STATUS.md` citing a test name. Words like "fast",
  "seam-free" and "exact" need a named metric and a reproducible command.
* **PTE-NO-042: refuse, do not ignore.** A setting this build cannot honour
  returns `ConfigError::UnsupportedInThisBuild` naming the governing
  requirement. Accepting it and producing something else is the failure mode the
  whole §33 list exists to prevent.
* **PTE-TOPO-001: one boundary, fitted once.** If you find yourself giving a
  face its own copy of a shared chain, stop. The validator will catch it, but
  the design should make it unwriteable.
* **PTE-NO-043: no unordered iteration in an output path.** `BTreeMap` and
  sorted `Vec`, never `HashMap`, wherever the result can reach the output or
  the digest.
* **PTE-DET-002/003: no bare float comparison decides anything.** Use
  `core::determinism::QuantKey`. `clippy::float_cmp` is denied; a test module
  that needs exact equality opts out locally and says why.
* **PTE-LIC-002, the clean-room boundary.** Do not read `palette_trace/**/*.py`
  while writing code here. `ADR-0003` explains what that costs and why it is
  worth it.
* **§19.4's forbidden allocation shapes.** No `N × K` buffer, no mask per
  palette entry, no unbounded memoisation. The label plane is the only
  per-pixel allocation the engine keeps.

## Documentation duties

Update `docs/IMPLEMENTATION_STATUS.md` when a durable status changes. Every
status cites evidence — a test name, a command, a file, an ADR. "Conforming"
means the requirement's tests exist and pass; if you did not run them, it is not
conforming. Never mark a stub as implemented, and delete a claim the code
disproves.

A nontrivial algorithm needs a design note in `docs/notes/` before it merges,
with the §34.2 contents: requirement identifiers, the mathematics and its
conventions, invariants and failure modes, complexity, tie rules, cancellation
points, citations and provenance, alternatives, tests, and known limitations.

## What not to do

* Do not add a dependency without a `THIRD_PARTY_NOTICES.md` row, a `deny.toml`
  entry, and a check that it builds for `wasm32-unknown-unknown`.
* Do not loosen a threshold to make a test pass. §31 permits loosening a gate
  only with written evidence that the original was invalid — not that it was
  difficult.
* Do not restate `SPEC.md` here. Cite the section.
* Do not claim a gate passes that you have not measured. The status document
  has a *Gaps* section precisely so that "not yet" has somewhere to live.
