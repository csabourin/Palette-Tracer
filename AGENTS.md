# Working instructions for agents

This file tells a coding agent how to work in this repository. It is not the specification.

**`SPEC.md` is the authoritative implementation contract.** This file never restates, summarizes or overrides it. Where the two appear to conflict, `SPEC.md` wins and this file is wrong.

**Except under `engine/`.** That directory holds the Palette Tracer Engine, a separate Rust project with its own specification (`engine/SPEC.md`), its own contributor contract (`engine/AGENTS.md`) and its own licence (`MIT OR Apache-2.0`). Neither this file nor the root `SPEC.md` applies there. The two specifications use disjoint requirement identifiers -- `PTE-TOPO-004` in the engine, `§20.3` here -- so a citation always says which contract it means. The engine is not wired into the Python pipeline; see `engine/docs/IMPLEMENTATION_STATUS.md`.

---

## 1. Orientation

Read in this order before changing anything:

1. `.ai/HANDOFF.md` — the current working slice. Check its freshness rules first; a stale handoff is worse than none.
2. `docs/IMPLEMENTATION_STATUS.md` — durable per-requirement state and known blockers.
3. The `SPEC.md` sections named by the handoff. Do not read the whole specification to make a small change.
4. `docs/decisions/` — technical decision records. These bind you.

Then run `git status`, review the diff, and run the smallest relevant validation command before editing.

## 2. Environment

```bash
python3 -m venv .venv
.venv/bin/python -m pip install -e ".[inkscape,backends,test]"
```

| Command | Purpose |
| ------- | ------- |
| `make test` | Full suite |
| `make test-unit` | Unit tests only |
| `make test-conformance` | Backend conformance under pytest |
| `make conformance` | Human-readable per-backend conformance report |
| `make phase0` | Phase 0 gate (§36) — executes the suites, does not check for files |
| `make verify-env` | Interpreter, pytest, inkex and discovered backends |
| `make web IMAGE=path/to.png` | Standalone host |

If `make test` fails with a `Permission denied` or `Exec format error` on `.venv/bin/python`, the virtual environment is corrupt — delete and recreate it. Do not work around it by setting `PYTHONPATH`.

## 3. Rules that come from the specification

These are the ones agents break most often. They are not style preferences.

* **§35 anti-shortcuts are binding.** Read that section before touching colour matching, palette generation, claim priority or backend selection. In particular: never use RGB Euclidean distance as the matching model, never treat layer order as claim priority, never re-run random clustering on dialog open, and never hard-code one tracing engine into the pipeline.
* **§9.4.1 host separation.** The headless core must not import `inkex`. Only `palette_trace/document/` and `palette_trace.py` may. `tests/unit/test_standalone_host.py` enforces this in a subprocess — if you add an `inkex` import to a core module, that test fails and the fix is to move the code, not to relax the test.
* **Backends go through the registry.** `palette_trace/tracing/registry.py` is the only module that names a backend. `REFERENCE_BACKEND_ID` is the single place the default is chosen, and changing it requires re-running conformance and updating `docs/decisions/ADR-0001-reference-tracing-backend.md`.
* **Versioned data files.** Reach mappings, trace profiles and destination presets live in `palette_trace/data/*_v1.json`. Do not inline these values into code, and do not edit a `_v1` file's semantics — add a `_v2`.
* **No network, ever.** The interface loads no remote assets and makes no external requests (§31). No CDN links, no web fonts.

## 4. Documentation duties

Two documents track progress. They serve different purposes and both must stay honest.

### `docs/IMPLEMENTATION_STATUS.md` — durable

Update when a **durable** status changes: a requirement becomes implemented or verified, a blocker appears or clears, a phase advances.

* Use only the status vocabulary defined at the top of that file.
* Every status must cite evidence: a file, a symbol, a test name, a command, a commit or a decision record.
* "Verified" requires an executed validation. If you did not run it, the status is "Implemented, unverified".
* Never mark a placeholder, mock or stub as implemented.
* Never mark a skipped test as passed.
* Never mark a phase complete when only its structure exists.
* Delete claims the current code disproves.

### `.ai/HANDOFF.md` — session-scoped

Rewrite at the end of any session that changed the working state. It is a handoff to the next agent, not a changelog.

* Record the exact commands you ran and their real results, including failures. "Tests pass" without a command is not a validation record.
* Record what you tried that did not work and why, so the next agent does not repeat it.
* Record unverified assumptions explicitly.
* Remove bullets that are no longer true. This file is pruned, not appended to.

### Honesty rules

These matter more than completeness:

* Report what the code does, not what it is named after. A module called `background.py` that nothing calls is not implemented background handling.
* A passing gate script is not evidence. Verify that the script executes real work before citing it — one in this repository previously reported "Phase 0 gate criteria MET" while zero conformance tests were running.
* If a status document contradicts the code, the code is right. Fix the document.
* State a partial result as partial. Do not round up.

## 5. Making changes

* **Scope.** Do what was asked. If you find an adjacent defect, fix it only if it blocks the task; otherwise record it in the handoff.
* **Tests.** New behaviour needs tests that would fail without it. A test asserting that a function returns something is not a test of the requirement.
* **Spec references.** When code implements a specific requirement, cite the section in a comment — `§20.3`, not "for exclusive layers". The next reader needs to find the rule.
* **Comments.** Explain why, not what. The interesting comment is the one recording a non-obvious constraint, a measured threshold or a defect that a change guards against.
* **Determinism.** Anything affecting geometry must produce identical output for identical input. If you introduce a dict iteration, a set, a timestamp or an unsorted glob into the geometry path, you have probably broken §34.30.

## 6. Conformance and thresholds

Backend conformance has two tiers, and the distinction is load-bearing:

* **Mandatory** checks encode the `MUST` list in §23.2. Every registered backend must pass. A failure means the backend should not be registered.
* **Quality** checks encode the §23.6 evaluation criteria and decide which backend is fit to be the reference. Backends may fail these and still ship as alternatives.

Thresholds live in `palette_trace/tracing/conformance/runner.py` with the rationale beside each one. If you change a threshold, re-run `make conformance`, record the new measurements in the ADR, and explain why the old threshold was wrong. Loosening a threshold to make a test pass is a defect, not a fix.

## 7. What not to do

* Do not add a dependency without checking licence compatibility with GPL-3.0-or-later and recording it.
* Do not commit unless asked.
* Do not restate `SPEC.md` in another file. Link to the section.
* Do not create speculative status rows for requirements nobody has started.
* Do not silently widen a schema. Settings and presets are versioned (§35).
* Do not disable or skip a failing test to make a suite green.
