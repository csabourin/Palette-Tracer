# Palette Trace — Current Handoff

> This file records the current implementation state.
> `SPEC.md` remains the authoritative implementation contract.
> Verify this handoff against Git and the current code before relying on it.

## Session metadata

* **Last updated:** 2026-08-07
* **Updated by:** Claude
* **Current branch:** `phase0-conformance-and-standalone-host` (unchanged; the prior session's commit on this branch was already merged to `master` via PR #2 before this session started — see "Git state" below)
* **Working tree:** Modified, not yet committed at the time of writing — see the diff for the exact file list
* **Current phase:** Phase 2 — Portable interface
* **Primary objective this session:** Fix `apply_destination_preset` never being called, then bring the browser interface up to what the headless core supports (channel tolerances, background controls, per-scan trace profiles, layer reordering, the §27 fingerprint-changed dialog, and saved-preset save/load).

## Start here

1. `git status` / `git log --oneline -5` — confirm this matches what's described below.
2. `make test` → expect `277 passed`.
3. `make phase0` → expect `12/12` and `Phase 0 gate criteria MET`.
4. Read `docs/IMPLEMENTATION_STATUS.md` for per-requirement state — it was updated this session.

## Git state (read before doing anything with branches)

`phase0-conformance-and-standalone-host`'s only commit (`302582d`) was already merged into `origin/master` via PR #2 (merge commit `c19c1f7`) before this session began. Local `master` has been fast-forwarded to `origin/master`. There was nothing to reconcile — no conflicting changes existed on `master` beyond that merge. This session's new work is uncommitted on top of `phase0-conformance-and-standalone-host`, which is otherwise identical to `master`.

## What changed this session

### Fixed: destination presets were never applied

`presets/destination.py::get_destination_preset` existed, was tested nowhere, and was called nowhere. Choosing a destination in the interface changed only `settings.destination.id`; geometry policy, trapping/underlap and the global trace profile stayed on the illustration defaults no matter what was picked. Added `apply_destination_preset(settings, dest_id)`, wired into two new endpoints:

* `POST /api/apply_destination` — called when the destination `<select>` changes.
* `POST /api/reset_destination_defaults` — SPEC §9.2's "Reset to destination defaults" button; re-applies the *current* destination's technical defaults, discarding manual geometry/profile tweaks.

Both leave palette entries, scan count and picked colours untouched — only destination-governed fields change. Verified live in-browser (destination → laser produced `geometry.policy: "separate_operations"` and fabrication-clean trace values in the actual API response) and in `tests/unit/test_destination_presets.py` / `tests/unit/test_api.py::TestDestinationEndpoints`.

### `custom`/`presetId` vs `override`/`profileId` naming

`SPEC.md` used `mode: "custom"` with a full `profile: TraceProfile` replacement; `presets/profiles.py` (and its existing, already-passing tests) used `mode: "override"` with a partial `values` merge and `profileId`. The code's convention won — a full-replacement `custom` mode would force every override to restate all twelve profile fields and would silently drop new `TraceProfile` fields added later. `SPEC.md` §11 and §18 were rewritten to match the code, with the rationale recorded in §18. Nothing in `presets/profiles.py` changed.

### Browser interface — the Phase 2 gap this closes

`web/index.html`, `web/app.js` and `web/styles.css` were rewritten (previously ~310 lines total; the interface exposed destination/scan-count/backend and a bare Colour-reach slider only). Added:

* Independent hue/chroma/lightness tolerance controls (§13.2), with a client-side `srgbToOklch` port of `color/conversion.py` driving the §13.4 low-chroma hue-default and the "Hue has little effect" hint.
* Background entry selector, matching-mode and output-mode controls (§16), kept in sync with per-entry `role`.
* Per-scan trace profile editor (§18): Inherit / named preset / Customize, with the Customize editor exposing the same 12 mask/vector fields the global profile has (field-level granularity — see the decision recorded in `SPEC.md` §18).
* Layer reordering (§9.2): keyboard-accessible Move up/down buttons (always present) plus native HTML5 drag-and-drop (pointer-only enhancement).
* The §27 fingerprint-changed recovery dialog, all four choices, via a new `POST /api/resolve_source_change`. The §27 *missing-source* dialog (3 choices) is **not** built — see Known blockers in `docs/IMPLEMENTATION_STATUS.md` for why that's a separate, larger change.
* Save/load/apply/delete for user presets (§26), via `presets/user_presets.py::build_configuration_patch`/`apply_configuration_patch` and `/api/user_presets*`. Also fixed hard-coded `createdAt`/`updatedAt` timestamps. Rename/duplicate/import/export are not built.

New/changed server endpoints, all in `palette_trace/server/api.py`: `/api/destination_presets`, `/api/apply_destination`, `/api/reset_destination_defaults`, `/api/trace_profiles`, `/api/resolve_source_change`, `/api/user_presets` (GET/POST), `/api/user_presets/apply`, `/api/user_presets/delete`. `/api/session` now also returns `sourceChanged`. Every mutating endpoint now returns the full updated `settings` object rather than a partial field set, so the client always re-syncs from one shape.

Accessibility (§29) was a hard constraint throughout, not a pass at the end — see the Accessibility notes in the session's final report to the user (not duplicated here; read `docs/IMPLEMENTATION_STATUS.md`'s §29 row for the tracked summary). No automated a11y suite exists yet; verification this session was manual (accessibility-tree inspection via the Claude Browser tool, plus interaction testing), not axe-core/Lighthouse/a real screen reader.

### Replit runnability

Added `.replit`, `replit.nix`, `scripts/replit_run.sh`, and `examples/sample.png` (a small generated fixture — Palette Trace has no other bundled sample image). `palette_trace/server/app_server.py::launch_palette_trace_app` and `palette_trace/standalone.py` gained `--host`/`--port` (defaulting to `127.0.0.1`/ephemeral per §9.1 unless `$PORT` is set, which Replit does and nothing else plausibly would). A `/` request with no `?token=` now 302s to the tokened URL — a UX nicety for Replit's webview, not a security change (`/api/*` still requires the token header regardless). Locally smoke-tested `$PORT=8080` → binds `0.0.0.0:8080`; not run inside an actual Replit container.

## Validation performed

| Command | Result | Notes |
| ------- | ------ | ----- |
| `make test` | Passed | `277 passed` (235 → 277: 42 new tests across `test_destination_presets.py`, `test_user_presets.py`, `test_api.py`, plus additions to `test_settings_provenance.py`) |
| `make phase0` | Passed | `12/12` |
| Manual browser verification | Passed | Standalone host launched against `examples/sample.png`; exercised destination change, colour picking (pin → API → re-render), per-scan trace-profile override editor, save/apply/delete preset round trip, and a full Apply → SVG + sidecar write, all via the Claude Browser tool. No console errors at any point. Accessibility tree confirmed programmatic label association, dialog focus-trapping (background content excluded from the tree while a `<dialog>` is open), and accessible names on icon-only buttons (e.g. "Move Scan 1 up", "Delete preset Test preset") |
| `$PORT`-driven bind smoke test | Passed | `PORT=8080 .venv/bin/python -m palette_trace.standalone ... ` → bound `0.0.0.0:8080`; bare `/` returned 302 to the tokened URL |

## Known failures

None. The suite is green.

## Bug found and fixed mid-session

`web/app.js::pinColor` called `pushSettings()` without awaiting it, then immediately called `announceStatus("Pinned … to …")`. Because `pushSettings()` itself calls `announceStatus("Settings updated.")` after its `await`, the generic message always overwrote the more useful specific one in the `aria-live` region. Fixed by awaiting `pushSettings()` before the specific announcement. Caught via live browser testing (the status region read "Settings updated." instead of the pin message), not by the Python test suite — there is no JS test harness in this repo.

## Unverified assumptions (new this session)

* [ ] Replit actually runs `scripts/replit_run.sh` the way `.replit` describes — only smoke-tested locally with `$PORT` set by hand, not inside a real Replit container.
* [ ] The manual accessibility verification (accessibility-tree inspection + interaction testing through an automation tool) is not a substitute for a real screen reader (NVDA/VoiceOver/JAWS) pass. Structure and semantics were checked; the actual spoken/braille experience was not.
* [ ] Drag-and-drop reordering was implemented but not exercised in the browser session (Move up/down buttons were). It's a pointer-only enhancement over an already-functional keyboard path, so the risk if it's subtly broken is low, but it hasn't been clicked.

## Immediate next actions

1. Real assistive-technology pass (NVDA or VoiceOver) over the interface, or at minimum an axe-core/Lighthouse run, before calling §29 "Verified" rather than "In progress".
2. §27 missing-source dialog — requires deferring the Inkscape host's early `PaletteTraceError` raise in `palette_trace.py::_run` so the web interface can launch and offer "Locate replacement" / "Keep existing vectors" / "Cancel" instead of failing before the UI ever opens.
3. Install the extension into an Inkscape user extensions directory and run it against a real document — still the largest unverified assumption in the whole project, unrelated to this session's work.
4. §26.4 preset rename/duplicate/import/export, if a user actually needs them — save/apply/delete cover the workflow described in this session's task.
5. Capture golden SVGs for a small fixture set (§33.3) — unrelated to this session, still open from before.

## Handoff freshness checks

The next agent must consider this handoff stale when:

* the recorded HEAD does not match the current relevant history;
* files in the working set were substantially changed afterward;
* tests contradict the recorded status;
* the implementation phase changed;
* a referenced symbol no longer exists;
* `SPEC.md` changed in a relevant section.
