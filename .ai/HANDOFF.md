# Palette Trace — Current Handoff

> This file records the current implementation state.
> `SPEC.md` remains the authoritative implementation contract.
> Verify this handoff against Git and the current code before relying on it.

## Session metadata

* **Last updated:** 2026-08-07
* **Updated by:** Claude
* **Current branch:** `master`
* **HEAD commit:** `bfc779f` — **the work described here is uncommitted**
* **Working tree:** Modified (24 files changed, 12 added, 3 renamed)
* **Current phase:** Phase 2 — Portable interface
* **Primary objective:** Close the Phase 0 gate with real evidence, then advance the headless core.

## Start here

1. `git status` — everything below is uncommitted.
2. `make test` → expect `235 passed`.
3. `make phase0` → expect `12/12` and `Phase 0 gate criteria MET`.
4. Read `AGENTS.md` §3 before touching colour matching, backends or host separation.
5. Read `docs/IMPLEMENTATION_STATUS.md` for per-requirement state.

The virtual environment was recreated this session — the old one had `IntxLNK` symlink stubs and could not execute. `make test` works now; do not reintroduce `PYTHONPATH` workarounds.

## Current objective

Bring the browser interface up to what the headless core already supports.

Observable completion condition:

> A user can, from the interface: edit hue/chroma/lightness tolerances independently; choose a background entry, matching mode and output mode; assign a trace profile per scan; reorder layers; and respond to a changed-source prompt. Each control round-trips through `/api/update_settings` and changes the preview.

The core supports all of this already. The gap is `web/app.js` (213 lines) and `web/index.html`, which post a whole settings object with no controls for any of it.

## Relevant specification

* `SPEC §9.2 — Interface layout`
* `SPEC §9.3 — Picked-colour behaviour`
* `SPEC §13.2 — Advanced channel controls`
* `SPEC §16 — Background handling`
* `SPEC §18 — Trace profiles`
* `SPEC §27 — Source changes and fingerprints`
* `SPEC §29 — Accessibility requirements`
* `SPEC §34.7, §34.12, §34.13, §34.14, §34.28`

## Current status

### Completed

**Phase 0 is genuinely closed.** `make phase0` executes the test suites and the conformance runner rather than checking that files exist; it passes 12/12.

* **Conformance harness rebuilt.** `palette_trace/tracing/conformance/{fixtures/,runner.py}` hold eight fixtures and fifteen checks in two tiers — mandatory (§23.2, every backend must pass) and quality (§23.6, decides reference eligibility). `make test-conformance` runs 40 tests where previously pytest collected zero.
* **Three backends evaluated.** potrace 1.16, vtracer 0.6.15 and python_contour 1.0.0 all pass the mandatory tier. Only potrace passes the quality tier.
* **ADR-0001 rewritten and Accepted**, selecting potrace with measured numbers. This reverses the superseded draft.
* **Background handling (§16)** — all three matching modes and all three output modes, wired into the controller.
* **Per-scan trace profiles (§18)** — `presets/profiles.py`; inherit, preset and override modes; unsupported settings reported rather than ignored.
* **Exclusive-layer and separate-operations geometry (§20.3, §20.4)** — including operation validation for laser output.
* **Fingerprint and provenance (§27, §10.3)** — both hosts record the source hash, settings hash, backend id and version; all seven `pt:` group attributes are written.
* **Standalone host (§9.4)** — `palette_trace/standalone.py`, `sidecar.py`, `svg_writer.py`. `palette-trace-web image.png` traces a file and writes an SVG plus a settings sidecar, with no Inkscape involved.
* **SPEC extended** with §9.4 (application hosts, host contract, standalone requirements, persistence, output, host parity table) and §10.5 (sidecar), plus updates to §5, §32, §34 preamble and Phase 2.
* **AGENTS.md rewritten** as agent working instructions. It was previously a byte-identical copy of `SPEC.MD`.

### In progress

* Browser interface — see Current objective.
* Saved presets — `user_presets.py` writes and lists but nothing loads or applies them, and `createdAt`/`updatedAt` are hard-coded strings.

### Blocked

Nothing is blocked.

### Not started but immediately relevant

* Golden-image tests (§33.3). Determinism is asserted per-run but never against a stored reference.
* `tests/accessibility/`, `tests/security/`, `tests/cross_platform/` (§33.4, §33.5).
* §22.4 clips, masks and SVG filters.
* §17.4 preview proxy scaling — the pipeline always runs at intrinsic resolution.

## Working set

| File | Relevant symbols | Why it matters |
| ---- | ---------------- | -------------- |
| `palette_trace/web/app.js` | `refreshPipeline()`, `renderPalette()` | Where the missing controls go |
| `palette_trace/web/index.html` | control panel markup | Needs the §9.2 layout and §29 semantics |
| `palette_trace/server/api.py` | `handle_api_request` | Accepts a whole settings object; new controls need no new endpoint |
| `palette_trace/settings.py` | `create_default_settings` | The field names the UI must drive, including `palette.backgroundMatching` |
| `palette_trace/presets/profiles.py` | `resolve_entry_profile`, `list_profile_ids` | Populates a per-scan profile dropdown |
| `palette_trace/server/session.py` | `AppSession.source_changed` | Set by both hosts; the UI never reads it (§27) |

## Architecture and data flow

```text
Host (palette_trace.py | palette_trace/standalone.py)
→ image_source.py            decode, EXIF, sRGB + alpha + OKLCH, SHA-256
→ settings.py                host-neutral schema (pt:settings | sidecar)
→ pipeline/controller.py     claims → quantize → distribute → label map
                             → cleanup → geometry policy → trace
→ tracing/registry.py        resolve backend (REFERENCE_BACKEND_ID = potrace)
→ generated_groups.py | svg_writer.py   commit
```

The host boundary is the important invariant: only `palette_trace/document/` and `palette_trace.py` may import `inkex` (§9.4.1). `tests/unit/test_standalone_host.py::TestHostSeparation` enforces this in a subprocess, so an `inkex` import already loaded by another test cannot mask a violation.

## Changes made in the latest session

### Renames

* `SPEC.MD` → `SPEC.md`
* `.ai/PROJECT_HANDOFF.md` → `.ai/HANDOFF.md`
* `docs/decisions/001-reference-backend-selection.md` → `docs/decisions/ADR-0001-reference-tracing-backend.md`
* `palette_trace/document/image_source.py` → `palette_trace/image_source.py` (both hosts need it; `document/` is now Inkscape-only)

### `palette_trace/tracing/backends/potrace_adapter.py`

* Changed: `potrace.Bitmap(mask_arr > 0)` → `potrace.Bitmap(mask_arr == 0)`.
* Reason: potrace treats samples *below* `blacklevel` as foreground, so the adapter was tracing the background and emitting a full-frame path for every mask.
* Specification reference: §23.2.

### `palette_trace/color/assignment.py` (new)

* Added: `distribute_unclaimed`, assigning each unclaimed pixel to its nearest automatic centre in OKLab.
* Reason: the controller assigned every unclaimed pixel to the first automatic entry, so the automatic palette was computed and discarded. An end-to-end run of a four-colour image produced one scan; it now produces four, matching the four source colours.
* Specification reference: §14.4, §15.6.

### `palette_trace/masks/thin_features.py`

* Changed: restore only the residue of a morphological opening at half the feature width, instead of every removed pixel.
* Reason: the previous version undid speckle removal entirely whenever thin-feature protection was on, which is the default.
* Specification reference: §17.5.

### Other implementation

* `color/background.py` — §16.1 matching modes, §16.2 rectangle path; edge-connected fill rewritten as constrained dilation.
* `masks/geometry_policy.py` — `enforce_exclusive_ownership`, `validate_operation_masks`.
* `presets/profiles.py` (new) — §18 resolution.
* `settings.py` (new), `sidecar.py` (new), `svg_writer.py` (new), `standalone.py` (new).
* `color/conversion.py` — `srgb_array_to_oklch`, replacing a per-pixel Python loop.
* `document/generated_groups.py` — all seven §10.3 attributes; `detect_manual_edits`.
* `palette_trace.py` — error boundary, provenance recording, fingerprint check.
* `scripts/check_phase0.py` — rewritten to execute rather than inspect.
* `pyproject.toml` — `potracer` promoted to a runtime dependency, `inkex` moved to an `inkscape` extra, `palette-trace-web` entry point, package data.
* `README.md` (new), `Makefile` (conformance/phase0/web targets), `__init__.py` across all subpackages.

### Tests

* `tests/conformance/test_backend_conformance.py` — rewritten; 40 tests.
* `tests/unit/test_geometry_policy.py`, `test_background.py`, `test_trace_profiles.py`, `test_settings_provenance.py`, `test_standalone_host.py`, `test_assignment.py` — new.
* `tests/integration/test_pipeline.py` — rewritten; 19 tests covering all four geometry policies, background modes, per-scan profiles and determinism.

Total: 59 → 235 tests.

## Decisions made

| Decision | Reason | Evidence | Revisit when |
| -------- | ------ | -------- | ------------ |
| Potrace, not VTracer, is the reference backend | VTracer discards a one-pixel diagonal entirely (0 paths); `potracer` is pure Python while VTracer ships a compiled Rust extension, so it is the more portable candidate under §23.4 | `make conformance`; ADR-0001 | A WASM candidate is prototyped, or potrace fails a future check |
| Conformance splits into mandatory and quality tiers | Every backend must satisfy §23.2, but §23.6 quality thresholds are about *choosing* a reference. Without the split, registering a deliberate low-quality fallback would turn the suite red | `tracing/conformance/runner.py` | — |
| Exclusive ownership resolves ties to the topmost layer | Matches what a stacked render would show, so exclusive output looks like stacked output without double coverage | `test_topmost_entry_wins_contested_pixels` | A destination needs a different rule |
| `image_source.py` moved out of `document/` | Both hosts decode images; `document/` is the Inkscape-only layer | `TestHostSeparation` | — |
| Trapping in millimetres converts at 96 dpi | Document DPI is not plumbed through yet; 96 is the SVG user-unit default | `controller._to_source_pixels` | Document DPI becomes available — this is a temporary assumption |

## Validation performed

| Command | Result | Notes |
| ------- | ------ | ----- |
| `make test` | Passed | `235 passed in 5.78s` |
| `make phase0` | Passed | `12/12 checks passed` / `Phase 0 gate criteria MET` |
| `make test-conformance` | Passed | `40 passed` (previously `collected 0 items`) |
| `make conformance` | Passed | 3 backends; `Reference-eligible: potrace` |
| `make verify-env` | Passed | `backends: ['potrace', 'vtracer', 'python_contour']` |
| `python -m build --wheel` | Passed | `palette_trace-1.0.0-py3-none-any.whl`; verified it contains all nine subpackages, the `data/*.json` files, `web/*` and the entry point |
| Standalone host, end to end | Passed | Traced a synthetic 120×120 four-colour image; wrote a 4-scan SVG whose labels are `#FBF7E7`, `#D22831`, `#141414`, `#1C2879` — exactly the four source colours |

## Known failures

None. The suite is green.

## Failed or rejected approaches

### Restoring the existing `.venv` by `chmod +x`

* Attempted: `chmod +x .venv/bin/*`.
* Rejected: the entries were not merely non-executable; `.venv/bin/python` was an `IntxLNK` symlink stub, so it failed with `Exec format error`. The venv had been copied across a filesystem boundary.
* Resolution: recreated the venv. The old `site-packages` was copied into the new one because `inkex` pulls in PyGObject → pycairo, which needs system cairo development headers that are not installed here.

### Asserting quality thresholds against every backend

* Attempted: one parametrized assertion per check per backend.
* Rejected: `python_contour` is a deliberate zero-dependency fallback and fails the node budgets by design (157 nodes for a rectangle against a budget of 32). Asserting quality on it would either turn the suite red or force the thresholds to be meaningless.
* Resolution: the two-tier split described above.

## Unverified assumptions

* [ ] The extension runs at all inside Inkscape. `palette_trace.py` has never been invoked through the INX descriptor.
* [ ] `document/transforms.py` maps correctly for images with a `viewBox` or non-uniform `preserveAspectRatio`.
* [ ] `ImageOps.exif_transpose` covers every orientation §22.5 requires — no fixture test.
* [ ] A single-threaded `handle_request()` loop is sufficient. A browser that opens a second connection or prefetches could stall it.
* [ ] 96 dpi is an acceptable interim conversion for millimetre trapping.
* [ ] `potracer`'s reported version string is `1.16`; the adapter hard-codes it rather than reading it from the package.

## Immediate next actions

1. Add per-scan trace-profile and background controls to `web/app.js` and `web/index.html`, driven by `list_profile_ids()` and `BACKGROUND_MATCHING_MODES`.
2. Add independent hue/chroma/lightness tolerance controls (§13.2, criterion 7).
3. Surface `session.source_changed` as the four §27 recovery choices.
4. Install the extension into an Inkscape user extensions directory and run it against a real document — this is the largest remaining unknown.
5. Capture golden SVGs for a small fixture set (§33.3).

## User-visible behaviour

**Standalone host — works today.** `palette-trace-web image.png` decodes the image, opens the interface on `127.0.0.1`, and on Apply writes `image.palette-trace.svg` with one labelled group per scan plus an `image.png.palettetrace.json` settings sidecar. Verified end to end this session. Refuses to overwrite an existing output without `--force`.

**Inkscape host — never run inside Inkscape.** The code path is complete: select a bitmap, run Extensions → Raster → Palette Trace, configure, Apply, and a labelled group appears beside the image with settings stored on the image element. Failures are caught and reported without modifying the document.

Available in the interface: preview, destination, scan count, backend choice, per-entry Colour reach, click-to-sample colour picking, Apply/Cancel.

Not yet in the interface, though the core supports all of it: independent channel tolerances, background entry and mode, per-scan trace profiles, layer reordering, and the changed-source prompt.

## Handoff freshness checks

The next agent must consider this handoff stale when:

* the recorded HEAD does not match the current relevant history;
* files in the working set were substantially changed afterward;
* tests contradict the recorded status;
* the implementation phase changed;
* a referenced symbol no longer exists;
* `SPEC.md` changed in a relevant section.

## Latest session summary

* Phase 0 gate is genuinely met: 12/12, with the checker executing suites rather than checking for files.
* Conformance went from 0 executing tests and 1 runnable backend to 40 tests across 3 backends.
* Reference backend changed from VTracer to potrace on measured evidence; ADR-0001 is Accepted.
* Three real defects fixed: potrace mask polarity, unclaimed-pixel distribution, thin-feature restoration.
* Standalone web application host implemented and verified end to end; SPEC §9.4 and §10.5 added to specify it.
* `AGENTS.md` rewritten as agent instructions; it was a byte-identical copy of the spec.
* Tests: 59 → 235. Wheel now builds and packages correctly.
* Everything is uncommitted on `master`.
* The narrowest part of the product is now the browser UI, not the core.
