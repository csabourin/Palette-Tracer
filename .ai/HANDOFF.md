# Palette Trace — Current Handoff

> This file records the current implementation state.
> `SPEC.md` remains the authoritative implementation contract.
> Verify this handoff against Git and the current code before relying on it.

## Session metadata

* **Last updated:** 2026-08-10
* **Updated by:** Claude
* **Current branch:** `claude/code-cleanup-dedup-hb6ken` at `9567430`, branched from `392a0d6` (`origin/master`, PR #12 merged)
* **Current phase:** Phase 2 — Portable interface
* **Primary objective this session:** Remove dead code and duplication. No feature work, no interface work, no behaviour change that was not itself the fix.

## Start here

1. `git status` / `git log --oneline -5` — confirm this matches what's described below.
2. `python -m pytest tests -q --ignore=tests/unit/test_schema.py --ignore=tests/unit/test_selection.py` → expect `434 passed`. See "Environment limitation" for why those two are ignored.
3. `ruff check .` → expect `All checks passed!`. The configuration is new this session; see below.
4. Read `docs/IMPLEMENTATION_STATUS.md` — criterion 26 was corrected this session.

## Environment limitation (read before believing a red suite)

`inkex` cannot be installed in this container, so `tests/unit/test_schema.py` and `tests/unit/test_selection.py` fail collection with `ModuleNotFoundError: No module named 'inkex'`.

**The failure mode has changed since the last handoff and the old description is wrong.** It no longer fails on `Dependency 'girepository-2.0' is required but not found`. On this container (Python 3.11.15, setuptools 68.1.2) `pip install inkex` now fails earlier, building its `scour` dependency:

```
Building wheel for scour (setup.py): finished with status 'error'
    raise AttributeError(attr)
AttributeError: install_layout. Did you mean: 'install_platlib'?
```

That is a `scour`-versus-setuptools incompatibility, not a missing system library — which means the next action here is probably pinning or patching `scour`, not installing GObject headers. Do not budget for the old diagnosis.

Consequences, both environmental rather than regressions:

* the full `make test` is uncollectable; run it with those two modules ignored;
* `make phase0` reports **11/12**, its only failing check being the unit suite. Verified this session by reading the failing check's own output (`[FAIL] Unit and integration tests pass — 2 errors in 0.52s`), which is the two collection errors above and nothing else.

## What changed this session (2026-08-10)

One commit, `9567430`: 207 files, +317 / −17,032. Nothing in `web/`. No API change, no schema change, no `SPEC.md` change.

### The repository contained a second copy of itself

`palette-tracer/` was a complete duplicate of the project — 127 tracked files, its own `SPEC.md`, `app.js`, `pyproject.toml` and test suite. It arrived in `d55f18e` ("Import Palette-Tracer from GitHub…") as an accidental nested clone alongside the tree that already existed, and was never touched again while the root tree kept moving. Nothing referenced it. Deleted.

If you are reading old session notes or old grep output, be aware that until this commit **every search in this repository returned two answers**, one of them months stale. That is worth knowing before trusting any file:line reference written before 2026-08-10.

Committed `__pycache__/*.pyc` and `scripts/tsconfig.tsbuildinfo` went with it. `.gitignore` had covered all of them already; they were tracked from before it did.

### Two backends reported themselves available and then traced nothing

`AutoTraceAdapter` and `InkscapeCliAdapter` were placeholders whose `trace_mask` returned an empty `svg_path_data` tuple. Both `is_available()` implementations returned true whenever the corresponding binary was on `PATH`, and both sat **ahead of `python_contour`** in `BACKEND_PRIORITY`.

So on a machine with `autotrace` or `inkscape` installed and neither potrace nor vtracer importable, `get_backend("auto")` resolved to a backend that silently produced empty scans — and the pipeline has no way to tell an empty scan from an empty mask, so nothing would have reported it. Not reachable in this container, which is why no test caught it.

Both adapters are deleted and removed from `BACKEND_PRIORITY`. `ADR-0001` records why and what reintroducing one requires. The registry now builds from a single `_ADAPTERS` tuple instead of five copy-pasted `if is_available()` blocks.

### Dead code removed

All verified uncalled by grep across `*.py` and `*.js` before deletion:

* `palette_trace/document/svg_sanitizer.py` — a module whose only content was a one-line pass-through to `normalization.normalize_svg_path_data`, plus an unused import.
* `palette_trace/pipeline/cache.py` — `PipelineCache` was never instantiated anywhere.
* `normalization.sanitize_svg_fragment` — imported only by the dead stub above. Before deleting it I traced the backend output boundary: every adapter returns path `d` strings and `svg_writer` / `generated_groups` consume only those, so no backend-authored SVG fragment is ever inserted into a document and §31's fragment sanitization has no live consumer on that path. The module docstring now says so, because it used to advertise sanitization this module no longer contains.
* `LabelMap.set_claims` — superseded by `set_claims_from_indices`.
* `errors.SchemaValidationError` — never raised.
* `fixtures.ALL_FIXTURES` — its docstring claimed the conformance runner iterated it "in report order"; the runner names each fixture individually and never read the dict.

### `detect_manual_edits` removed — read this if you were counting on it

`generated_groups.detect_manual_edits` implemented SPEC §28 / §34.26 correctly, but **no host ever called it and no test exercised it**. `docs/IMPLEMENTATION_STATUS.md` credited it with `test_recorded_settings_hash_detects_later_edits`, which actually covers `compute_settings_hash` and never touches the detector.

Removed, and criterion 26 corrected from "Partially verified" to **"Not built"**. The `pt:settings-hash` attribute is still written onto the generated group, so the evidence a detector needs is still recorded — what does not exist is anything comparing it before replacing a group. That was already true; the function only made it look otherwise.

This is the one deletion a reviewer might reasonably reverse. It is in `9567430` if you want it back.

### Duplicated declarations collapsed

* `settings.py` declared background-matching, background-output and geometry-policy vocabularies as unused literal tuples, duplicating the named constants in `color.background` and `masks.geometry_policy` and the enums in `schemas/image-settings-v1.schema.json`. Removed; a comment records where they actually live.
* The version string `"1.0.0"` was written out in `__init__.py`, `settings.py` and `diagnostics.py`. `EXTENSION_VERSION` now derives from `palette_trace.__version__` and `diagnostics` imports it.

### `requires-python` was wrong and the package could not import below 3.11

`pyproject.toml` declared `>=3.9`. But `color/conversion.py` calls `math.cbrt`, added in **3.11**, and annotations throughout use PEP 604 unions (`str | None`) that are evaluated at definition time and need **3.10**. Anyone installing on 3.9 or 3.10 would have hit an `ImportError` or `TypeError` on first import.

Corrected to `>=3.11` in `pyproject.toml` and in `README.md`. This matches what the deployment already used — `.replit` pins `python-3.12`, `replit.nix` pins `python312`.

### Lint configuration added

`[tool.ruff]` in `pyproject.toml`: `line-length = 100`, `target-version = "py311"`, selecting `F,E,W,I,UP,SIM,C4,RET,PIE,ERA`. Everything it found is fixed — unused imports, deprecated `typing` aliases, unsorted imports, redundant `open(..., "r")` modes, 17 over-length lines.

One deliberate per-file ignore, with its reasoning in the config: `E741` in `color/conversion.py`, where `l`, `m` and `s` are the LMS cone responses the OKLab transform is published in terms of. Renaming them would make that code harder to check against the reference, not easier.

Two module-level `import` probes that loaded a whole tracing engine just to discover whether it exists now use `importlib.util.find_spec` (`capabilities.py`, `vtracer_adapter.py`). `vtracer` itself is still imported lazily inside `trace_mask`, because it remains an optional dependency.

## Validation performed this session (2026-08-10)

| Command | Result | Notes |
| ------- | ------ | ----- |
| `python -m pytest tests -q --ignore=tests/unit/test_schema.py --ignore=tests/unit/test_selection.py` | Passed | `434 passed in 20.08s` — identical to the pre-change baseline captured before any edit. No test was added, removed or rewritten for behaviour; the only test edits were unused imports, four `== True`/`== False` comparisons and line wrapping |
| `ruff check .` | Passed | `All checks passed!` |
| `python scripts/check_phase0.py` | 11/12 | Only the unit-suite check fails, and only for the `inkex` reason above — confirmed by reading that check's output, not assumed |
| `python -m palette_trace.tracing.conformance.runner` | Passed | `potrace (1.16): REFERENCE-ELIGIBLE`; `vtracer (0.6.15)` and `python_contour (1.0.0)` both `VALID (quality checks failed)`, which is their documented standing in ADR-0001. `Reference-eligible: potrace` |
| `BackendRegistry()` smoke test | Passed | `available: ['potrace', 'vtracer', 'python_contour']`, `auto -> potrace` — the removed adapters are gone and selection is unchanged |
| `generate_diagnostic_report()` smoke test | Passed | Returns `has_pil`/`has_numpy`/`has_vtracer`/`has_potrace` all true with real versions, and `extension_version: 1.0.0` from the single source |
| `python -m palette_trace.standalone --help` | Passed | Entry point still resolves after the import changes |

## Unverified assumptions

Carried forward from previous sessions and still unverified — none of this session's work touched any of them:

* [ ] The scripted browser sessions used Chromium only. Safari on iOS is the platform where `dialog`, `touch-action`, `env(safe-area-inset-*)` and pointer capture most plausibly differ, and it has not been tried.
* [ ] Real-device touch was never used — Playwright's synthetic touch is not a fingertip, and the magnifier's offset above the contact point is tuned by eye.
* [ ] No accessibility audit has been run. Keyboard paths were exercised in earlier sessions; a screen reader was not.
* [ ] The Inkscape host is unexercised — `inkex` will not install here, so `document/` is covered by nothing.
* [ ] Drag-to-reorder in the swatch list was implemented but never clicked.
* [ ] A palette grown well past a handful of colours was never looked at.
* [ ] Replit has not been exercised since PR #12. Nothing this session touched `.replit` or `scripts/replit_run.sh`.

New this session:

* [ ] **The removed adapters were never observed failing** — the empty-trace path is unreachable in this container, since neither `autotrace` nor `inkscape` is installed. The defect was read out of the code, not reproduced. It is a deletion of code that provably returned `svg_path_data=()`, so the reasoning does not depend on reproducing it, but no test demonstrates the old behaviour.
* [ ] **`>=3.11` is inferred, not measured.** `math.cbrt` and PEP 604 fix the floor at 3.11 by inspection; the suite was not run under 3.11.0, only under this container's 3.11.15.

## Immediate next actions

1. **Add `tests/ui/` with Playwright.** Still the highest-value gap in Phase 2. Unchanged by this session, which added no interface coverage because it changed no interface code.
2. **Make `test_schema.py` and `test_selection.py` skip when `inkex` is absent**, rather than fail collection. This is now the cheapest of the two options — the container fix needs a `scour`/setuptools resolution (see the corrected diagnosis above), whereas a module-level `pytest.importorskip("inkex")` would make `make test` and the Phase 0 gate meaningful immediately. The gate has been reporting 11/12 for an environmental reason across at least three sessions.
3. Accessibility pass with a real screen reader, plus axe-core, before calling §29 Verified.
4. §27 missing-source dialog — still requires deferring the Inkscape host's early `PaletteTraceError`.
5. Measure peak memory on a real Replit container and decide whether `MAX_WORKING_PIXELS` can be raised. See the requalified blocker in `docs/IMPLEMENTATION_STATUS.md`.
6. **Decide whether §28 manual-edit detection is wanted.** Criterion 26 is now honestly "Not built". If it is wanted, the hash is already on the group and the check is a few lines in `commit_generated_trace_group`, at the branch where an `existing` group is found and removed.

## Handoff freshness checks

The next agent must consider this handoff stale when:

* the recorded HEAD does not match the current relevant history;
* files in the working set were substantially changed afterward;
* tests contradict the recorded status;
* the implementation phase changed;
* a referenced symbol no longer exists;
* `SPEC.md` changed in a relevant section.
