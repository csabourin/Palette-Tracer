# Palette Trace — Implementation Status

This document tracks durable implementation progress against `SPEC.md`.

`SPEC.md` is authoritative. This document records evidence; it does not redefine requirements.

**Assessed at:** 2026-08-07 — base commit `bfc779f` plus the uncommitted working tree described below. 235 tests pass; the Phase 0 gate passes 12/12.

## Status definitions

* **Not started:** No meaningful implementation exists.
* **In progress:** Implementation has begun but is incomplete.
* **Implemented, unverified:** The behaviour appears implemented but required validation has not been run.
* **Partially verified:** Some required validation has passed.
* **Verified:** The required implementation and applicable validation have passed.
* **Blocked:** Progress requires an unresolved capability, dependency, decision, or correction.
* **Deferred — SHOULD:** A `SHOULD` requirement was deferred with a documented technical reason.
* **Future / out of MVP:** Explicitly outside the current MVP.

## Current phase summary

| Phase | Status | Evidence | Remaining work |
| ----- | ------ | -------- | -------------- |
| Phase 0 — Engine and colour-model spike | **Verified** | `make phase0` → 12/12; `make conformance` evaluates 3 backends; ADR-0001 Accepted with measured results | None. Gate met 2026-08-07. |
| Phase 1 — Headless core | Partially verified | Full pipeline runs decode → claims → quantize → distribute → label map → cleanup → geometry → trace → assemble. 235 tests. | No golden-image tests (§33.3); no CLI development harness distinct from the standalone host; §22.4 clip/mask/filter handling absent; preview proxy scaling (§17.4) absent |
| Phase 2 — Portable interface | In progress | Loopback server, token-gated API, browser UI, standalone host with sidecar persistence and SVG export | UI does not yet expose per-channel tolerances, per-scan profiles, background mode, layer reordering, or the §27 source-changed dialog; single-threaded server; no cancellation of an in-flight trace; no progress reporting; accessibility unverified |
| Phase 3 — Inkscape integration | Implemented, unverified | INX, selection, `pt:` persistence, group replacement, transforms, EXIF, provenance attributes, error boundary | Never executed inside Inkscape; §27 recovery dialogs not built; §22.4 absent |
| Phase 4 — Presets and production validation | In progress | Destination presets drive geometry policy; all four §20 policies implemented; laser operation validation | Preset apply path, import/export, `presets/migrations.py`, screen-print and vinyl destination-specific validation beyond geometry |
| Phase 5 — Packaging and release | In progress | `README.md`, wheel builds and packages all subpackages and data, `palette-trace-web` entry point | `CHANGELOG.md`, `CONTRIBUTING.md`, `THIRD_PARTY_NOTICES.md`, licence review, accessibility review, user documentation, sample SVGs, release archive |

## Active implementation slice

* **Current phase:** Phase 2 — the interface is now the narrowest part of the product.
* **Current objective:** Expose in the browser UI the controls the core already supports.
* **Primary specification sections:** §9.2, §9.3, §13.2, §16, §18, §27, §29.
* **Associated handoff:** `.ai/HANDOFF.md`
* **Technical decision records:** `docs/decisions/ADR-0001-reference-tracing-backend.md`

## Requirement tracking

| Specification | Requirement | Status | Implementation evidence | Validation evidence | Notes |
| ------------- | ----------- | ------ | ----------------------- | ------------------- | ----- |
| §9.4 | Two application hosts drive one core | Verified | `palette_trace.py`, `palette_trace/standalone.py` | `test_standalone_host.py::TestHostSeparation` (subprocess-checked), `::TestHostParity` | Core is `inkex`-free; enforced by test, not convention |
| §9.4.3 | Standalone settings sidecar | Verified | `palette_trace/sidecar.py` | `test_standalone_host.py::TestSidecar` — 7 tests incl. malformed, wrong-version, orphaned | Advisory: never blocks opening |
| §9.4.4 | Standalone SVG output | Verified | `palette_trace/svg_writer.py` | `test_standalone_host.py::TestSvgWriter` — 10 tests | Atomic write via `sidecar.write_atomically` |
| §10.2 | Settings stored on the source image | Implemented, unverified | `document/settings_store.py` | `test_schema.py` | Untested against a live Inkscape document |
| §10.3 | Generated group attributes | Verified | `document/generated_groups.py`, `svg_writer.py` | `test_standalone_host.py::test_provenance_attributes_are_written` | All seven attributes now written by both hosts |
| §13.1 | Versioned deterministic reach interpolation | Verified | `color/reach.py`, `data/reach_mapping_v1.json` | `test_reach_calibration.py` — 18 tests | Five normative anchors, monotonicity, clamping |
| §13.2 | Per-channel tolerances separately editable | Partially verified | `color/claims.py` independent-channel mode | `test_claims_comprehensive.py::TestIndependentChannelMode` | Core supports it; no UI control |
| §13.3 | Hue wrapping | Verified | `color/conversion.py` | `test_color.py::test_shortest_hue_distance` | — |
| §13.4 | Low-chroma hue suppression | Verified | `color/reach.py` | `test_reach_calibration.py::TestHueConfidence`, `test_claims_comprehensive.py::TestNeutralColorGuardrail` | — |
| §14.1–14.3 | Reserved candidates, scoring, deterministic conflict resolution | Verified | `color/claims.py` | `test_claims_comprehensive.py` — 12 tests | Tie-break: priority index, then UUID |
| §14.4 | Unclaimed pixels | **Verified** | `color/assignment.py::distribute_unclaimed` | `test_assignment.py` — 22 tests; `test_pipeline.py` | **Fixed this session.** Previously every unclaimed pixel was assigned to the first automatic entry, so only one scan ever produced geometry |
| §14.5 | One pixel, at most one scan | Verified | `color/assignment.py`, `masks/geometry_policy.py` | `test_assignment.py::test_masks_are_disjoint`, `test_geometry_policy.py::test_every_pixel_belongs_to_exactly_one_scan` | — |
| §15 | Deterministic automatic palette | Verified | `color/quantizer.py`, `color/histogram.py` | `test_quantizer_determinism.py` — 14 tests | §15.5 minimum separation implemented, not separately asserted |
| §16 | Background kept, omitted or replaced | **Verified** | `color/background.py`, `pipeline/controller.py` | `test_background.py` — 16 tests; `test_pipeline.py::TestBackground` — 5 tests | **Implemented this session.** All three §16.1 matching modes and all three §16.2 output modes |
| §17.1 | One integer label map | Verified | `masks/label_map.py` | `test_masks.py::test_label_map` | — |
| §17.2–17.3 | Cleanup order and speckle removal | Verified | `masks/components.py`, `masks/morphology.py` | `test_masks.py` | — |
| §17.4 | Preview scaling | Not started | — | — | Pipeline always runs at intrinsic resolution |
| §17.5 | Thin-feature protection | **Verified** | `masks/thin_features.py` | Exercised by `test_pipeline.py`; conformance `thin_feature_retention` | **Fixed this session.** Previously restored every removed pixel unconditionally, which undid speckle removal entirely. Now restores only the residue of a morphological opening |
| §18 | Scans inherit or override trace profiles | **Verified** | `presets/profiles.py`, `pipeline/controller.py` | `test_trace_profiles.py` — 18 tests; `test_pipeline.py::TestTraceProfiles` | **Implemented this session.** Inherit, preset and override modes; unsupported settings reported, not ignored |
| §19 | Destination presets produce distinct policies | Partially verified | `presets/destination.py`, `data/destination_presets_v1.json`, controller geometry branch | `test_pipeline.py::TestGeometryPolicies` | Geometry differs per destination; destination-specific *validation* beyond §20.4 is not implemented |
| §20.1–20.2 | Stacked and stacked-trapped | Verified | `masks/geometry_policy.py` | `test_geometry_policy.py::TestUnderlap`, `::TestTrapping` | Silhouette preservation asserted |
| §20.3 | Exclusive layers | **Verified** | `masks/geometry_policy.py::enforce_exclusive_ownership` | `test_geometry_policy.py::TestExclusiveOwnership` | **Implemented this session.** Ownership is exclusive, union preserved, per-scan offsets disabled |
| §20.4 | Separate operations | **Verified** | `geometry_policy.py::validate_operation_masks` | `test_geometry_policy.py::TestOperationValidation`, `test_pipeline.py` | **Implemented this session.** Empty, undersized and duplicate operations reported |
| §21.1 | OKLCH working space | Verified | `color/conversion.py`, `image_source.py` | `test_color.py::test_oklab_roundtrip`; `make phase0` asserts round-trip < 1/255 | **Vectorized this session** — was one Python interpreter call per pixel |
| §22.1–22.2 | Embedded and linked local images | Implemented, unverified | `document/selection.py`, `image_source.py` | `test_selection.py`; `test_standalone_host.py::test_source_decodes_through_the_shared_representation` | Data-URI branch still untested |
| §22.3 | Transform mapping | Implemented, unverified | `document/transforms.py` | — | `viewBox` and non-uniform `preserveAspectRatio` not handled |
| §22.4 | Clips, masks and SVG filters | Not started | — | — | — |
| §22.5 | EXIF orientation | Implemented, unverified | `image_source.py` | — | `ImageOps.exif_transpose`; no fixture test |
| §23.1 | Canonical backend protocol | Verified | `tracing/protocol.py` | `make phase0` compares dataclass fields against §23.1 | — |
| §23.2 | Backend responsibilities | Verified | All five adapters | `test_backend_conformance.py` mandatory tier — 7 checks × 3 backends | — |
| §23.3–23.4 | Discovery and portable preference | Verified | `tracing/registry.py` | `test_registry_prefers_the_reference_backend`, `test_explicit_backend_selection_is_honoured` | Priority is a named constant |
| §23.6 | At least two candidates evaluated | **Verified** | `tracing/conformance/{fixtures,runner}.py` | `make conformance` — potrace, vtracer, python_contour; ADR-0001 records the numbers | **Unblocked this session.** Was 1 runnable backend and 0 executing tests |
| §24 Stage 7 | Background classification stage | Verified | `pipeline/controller.py` | `test_pipeline.py::TestBackground` | — |
| §24 Stage 13 | Atomic commit | Verified | `document/generated_groups.py`, `sidecar.write_atomically` | `test_standalone_host.py::TestAtomicWrite` | Group built in full before the old one is detached |
| §26 | Saved-preset schema | In progress | `schemas/user-preset-v1.schema.json`, `presets/user_presets.py` | — | `createdAt`/`updatedAt` still hard-coded; no load/apply path; no `migrations.py` |
| §27 | Source fingerprint change detection | Partially verified | `settings.py::source_has_changed`, both hosts set `session.source_changed` | `test_settings_provenance.py::TestFingerprint` — 4 tests | **Detection implemented this session.** The four recovery *choices* are not built; standalone prints a note, Inkscape sets a flag the UI ignores |
| §29 | Accessibility | Not started | Some `role`/`aria-label` in `web/index.html` | — | No `tests/accessibility/` |
| §30 | Error handling | Partially verified | `errors.py`, `diagnostics.py`, error boundary in `palette_trace.py` | — | Diagnostics report is now surfaced on failure; no test |
| §31 | Security | Partially verified | 127.0.0.1 + ephemeral port, 128-bit token with `compare_digest`, `no-store`, no external assets | Manual inspection | No `tests/security/` |
| §33.2 | Backend conformance tests | **Verified** | `tests/conformance/test_backend_conformance.py` | `make test-conformance` → 40 passed | **Fixed this session.** Was `collected 0 items` |
| §33.3 | Golden-image tests | Not started | — | — | `tests/golden/` does not exist |
| §33.4 | Accessibility tests | Not started | — | — | — |
| §33.5 | Cross-platform tests | Not started | — | — | Linux only |
| §35 | Anti-shortcut requirements | Partially verified | OKLCH matching, claims before quantization, explicit priority, versioned data files, registry indirection, no network assets | Full suite | Remaining gap is UI exposure, not shortcuts in the core |

## MVP acceptance criteria

| Criterion | Summary | Status | Evidence |
| --------: | ------- | ------ | -------- |
| 1 | Open one selected embedded or linked local bitmap | Partially verified | `document/selection.py`; standalone path verified by `test_standalone_host.py` |
| 2 | Restore settings stored on the image | Partially verified | `settings_store.py`; sidecar path verified by `TestSidecar` |
| 3 | Select 1–64 scans | Implemented, unverified | Schema bounds, `web/index.html` |
| 4 | Pick colours from the preview | Implemented, unverified | `server/api.py` `/api/sample_color` |
| 5 | Picked colours become exact output colours | Verified | `test_pipeline.py::test_picked_colours_are_exact_output_colours` |
| 6 | Every picked colour has Colour reach | Verified | `test_reach_calibration.py::TestReachClaimsIntegration` |
| 7 | Hue, chroma and lightness separately editable | In progress | Core supports it; no UI control |
| 8 | Neutral colours handle hue reliably | Verified | `TestHueConfidence`, `TestNeutralColorGuardrail` |
| 9 | Remaining scans are generated deterministically | **Verified** | `test_quantizer_determinism.py`; end-to-end run recovered all four source colours into four scans |
| 10 | Automatic colours account for pinned colours | Verified | Controller quantizes only the unclaimed mask; `test_pipeline.py` |
| 11 | Conflicting claims resolve deterministically | Verified | `TestTieBreaking` |
| 12 | Background kept, omitted or replaced | **Verified** | `test_pipeline.py::TestBackground` — 5 tests |
| 13 | Scans inherit or override trace profiles | **Verified** | `test_trace_profiles.py`, `test_pipeline.py::TestTraceProfiles` |
| 14 | Different scans use materially different profiles | **Verified** | `test_per_scan_profiles_change_the_output` |
| 15 | Destination presets produce distinct policies | Partially verified | Geometry policy differs per destination; validation is thinner |
| 16 | Stacked and trapped output work | Verified | `test_geometry_policy.py` |
| 17 | Exclusive-layer output maintains exclusive ownership | **Verified** | `TestExclusiveOwnership` |
| 18 | Laser output creates named operation groups | Partially verified | `separate_operations` policy and validation exist; group naming is generic, not operation-role-specific |
| 19 | A portable backend passes conformance tests | **Verified** | potrace passes all 15 checks; `make conformance` |
| 20 | Backend selection is abstracted | Verified | `tracing/registry.py`; controller never names a backend |
| 21 | Output is grouped and labelled | Verified | `test_scan_groups_are_labelled` |
| 22 | Settings stored on the source | Partially verified | Both hosts; Inkscape path untested live |
| 23 | Deleting the image deletes its settings | Implemented, unverified | Inherent to attribute storage; not applicable to standalone per §9.4.5 |
| 24 | Saved presets apply to another image | Not started | `user_presets.py` saves and lists only |
| 25 | Reapplying replaces the linked generated group | Implemented, unverified | `commit_generated_trace_group(existing_group_id=…)` |
| 26 | Manual-edit risk detected or warned | Partially verified | `detect_manual_edits` + `pt:settings-hash` now written | `test_recorded_settings_hash_detects_later_edits`; not wired into a user-facing warning |
| 27 | No source data leaves the machine | Partially verified | Loopback, token, no external assets | No `tests/security/` |
| 28 | Interface is keyboard accessible | Not started | — |
| 29 | Errors do not corrupt the document | Partially verified | Error boundary, group swap, atomic file write |
| 30 | Identical inputs produce deterministic results | Verified | `test_identical_inputs_produce_identical_results`; conformance determinism check per backend |

## Defects found and fixed this session

| Defect | Impact | Fix | Guard |
| ------ | ------ | --- | ----- |
| `PotraceAdapter` passed the mask to potrace without inverting it. Potrace treats samples *below* `blacklevel` as foreground, so the background was traced. | Every potrace-traced scan gained a path covering the whole image frame. This also polluted the Phase 0 measurements and led the superseded ADR to the wrong reference backend. | `potrace.Bitmap(mask_arr == 0)` | `test_solid_shape_does_not_produce_a_full_frame_path`, run against every registered backend |
| All unclaimed pixels were assigned to the first automatic entry. | The automatic palette was computed and then discarded; only one scan ever produced geometry. An end-to-end run of a four-colour image produced one scan. | `color/assignment.py::distribute_unclaimed` — nearest automatic centre in OKLab | `test_assignment.py` (22 tests), end-to-end run now yields four scans matching the four source colours |
| `preserve_thin_features` restored every removed pixel unconditionally. | Speckle removal was effectively a no-op whenever thin-feature protection was on, which is the default. | Restore only the residue of a morphological opening at half the feature width | `masks/thin_features.py::thin_structure_mask` |
| Conformance suite was invisible to pytest (`__init__` on a non-`Test*` class). | Zero conformance checks executed while the gate script reported the Phase 0 gate as met. | Checks moved to `tracing/conformance/runner.py`; pytest module parametrizes over discovered backends | `make test-conformance` → 40 passed; `test_every_check_was_evaluated` |
| `scripts/check_phase0.py` used only `__import__` and `os.path.exists`. | Reported "Phase 0 gate criteria MET" with one runnable backend and no executed tests. | Rewritten to execute pytest, run conformance, and assert measured results | `make phase0` |
| sRGB→OKLCH conversion was a per-pixel Python loop. | One interpreter call per pixel; dominated decode time on any real image. | `srgb_array_to_oklch` vectorized over the whole array | `make phase0` asserts round-trip accuracy < 1/255 |
| `pyproject.toml` declared `readme = "README.md"`, which did not exist. | Any source or wheel build failed. | `README.md` added | `python -m build --wheel` succeeds |
| Subpackages had no `__init__.py`. | Relied on namespace-package behaviour that `setuptools.packages.find` does not guarantee. | `__init__.py` added throughout; `package-data` declared | Wheel contains all nine subpackages and the data files |

## Deferred requirements

| Specification | Requirement | Reason | Observable impact | Proposed path |
| ------------- | ----------- | ------ | ----------------- | ------------- |
| §23.4 | WebAssembly backend preferred over a Python binding | The reference backend `potracer` is pure Python with no compiled extension, which satisfies the portability intent at far lower cost than building and bundling a WASM toolchain | None today; a WASM engine might trace faster | Revisit if runtime becomes a complaint. Recorded in ADR-0001 |

## Known blockers

| Blocker | Affected requirements | Evidence | Owner or next action |
| ------- | --------------------- | -------- | -------------------- |
| The browser UI exposes a fraction of what the core supports | §13.2, §16, §18, §27, criteria 7, 12, 13, 14 | `web/app.js` is 213 lines and posts a whole settings object; it has no controls for channel tolerances, per-scan profiles, background mode or layer order | The Phase 2 work described in the active slice |
| Never executed inside Inkscape | All Phase 3 requirements | `inkex` 1.4.1 is installed and importable, but `palette_trace.inx` has never been loaded by an Inkscape process | Install into a user extensions directory and run against a real document |
| No golden-image tests | §33.3, criterion 30 | `tests/golden/` does not exist. Determinism is asserted per-run, not against a stored reference | Capture golden SVGs for a small fixture set once the UI stabilises |

## Technical decisions

| Decision record | Summary | Status |
| --------------- | ------- | ------ |
| `docs/decisions/ADR-0001-reference-tracing-backend.md` | Potrace via the pure-Python `potracer` binding is the reference backend | **Accepted** — supported by executed conformance results for three candidates |

## Validation environments

| Environment | Status | Last verified | Notes |
| ----------- | ------ | ------------- | ----- |
| Windows | Not verified | — | — |
| macOS | Not verified | — | — |
| Linux | Partially verified | 2026-08-07 | WSL2, Python 3.12.3, pytest 9.1.1 — 235 tests pass, Phase 0 gate 12/12, wheel builds |
| Inkscape 1.4 | Not verified | — | `inkex` 1.4.1 present; extension never run inside Inkscape |

## Repository structure conformance (§32)

Present and specified: everything in §32 except the items below.

Still missing: `CHANGELOG.md`, `CONTRIBUTING.md`, `THIRD_PARTY_NOTICES.md`, `palette_trace/presets/migrations.py`, `palette_trace/pipeline/jobs.py`, `palette_trace/pipeline/cancellation.py`, `palette_trace/web/assets/`, `palette_trace/web/wasm/`, `tests/golden/`, `tests/accessibility/`, `tests/security/`, `tests/cross_platform/`.

`palette_trace/extension.py` is absent by design: the Inkscape entry point is the top-level `palette_trace.py`, which is what the INX descriptor invokes.

`palette_trace/image_source.py` moved out of `document/` this session because both hosts need it and `document/` is now defined as the Inkscape-only layer (§9.4.1). §32 was updated to match.

## Maintenance rules

* Update this document only when durable status changes.
* Link status to files, symbols, tests, commands, commits, or decision records.
* Do not mark placeholders or mocks as implemented.
* Do not mark skipped tests as passed.
* Do not mark an entire phase complete when only its structure exists.
* Remove claims that are disproven by the current implementation.
* Use `.ai/HANDOFF.md` for temporary, session-specific continuation details.
