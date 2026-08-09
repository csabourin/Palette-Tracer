# Palette Trace — Current Handoff

> This file records the current implementation state.
> `SPEC.md` remains the authoritative implementation contract.
> Verify this handoff against Git and the current code before relying on it.

## Session metadata

* **Last updated:** 2026-08-09
* **Updated by:** Claude
* **Current branch:** `claude/color-picker-ux-api-yuag4d`, branched from `94e3614` (`origin/master`, PR #7 merged)
* **Current phase:** Phase 2 — Portable interface
* **Primary objective this session:** Make the palette the only statement of how many colours the trace has, and make picking from the picture the only way a colour gets into it.

## Start here

1. `git status` / `git log --oneline -5` — confirm this matches what's described below.
2. `uv sync && uv run --with pytest pytest tests/ -q --ignore=tests/unit/test_schema.py --ignore=tests/unit/test_selection.py` → expect `376 passed`. See "Environment limitation" for why those two are ignored.
3. Read `docs/IMPLEMENTATION_STATUS.md` — updated this session.

## Environment limitation (read before believing a red suite)

`inkex` cannot be installed in this container: `pip install inkex` fails at metadata generation with `Dependency 'girepository-2.0' is required but not found`. `tests/unit/test_schema.py` and `tests/unit/test_selection.py` import it at module level and therefore fail collection here.

Consequences, both environmental rather than regressions:

* the full `make test` is uncollectable; run it with those two modules ignored;
* `make phase0` reports **11/12**, its only failing check being the unit suite.

Both were already true before this session's changes — verified by stashing them and re-running.

## What changed this session (2026-08-09)

### The palette is the number of colours

The toolbar used to carry a `− 4 +` stepper that set `scanCount`, and an **Add**
button that opened a colour wheel. Between them they said two contradictory
things: that the number of colours was a figure to type, and that a colour could
be invented rather than found in the picture.

Both are gone. In their place, a **Colours** button opens `#sheet-palette`,
which lists every entry — picked and automatic alike — with its colour, its
coverage and a Remove button, and offers one way to add: *Add a colour from the
picture*, which hands the gesture to the existing picker.

* `scanCount` did not change meaning and the API did not change at all. The
  interface holds `scanCount === palette.entries.length` at every mutation
  instead of setting it directly. §10.4 already required the two to be equal.
* `addPinnedColour` now always appends. It used to consume the first automatic
  entry, which kept the total steady back when a separate number owned it; with
  the list as the count, an add that replaced a row would misreport itself.
* The removal logic moved out of the scan sheet into `removeEntry`, shared by
  both places, and refuses the last remaining entry (the schema requires ≥ 1).
* `#sheet-colour` survives, reachable only through **Change** on an entry that
  already exists — a screen print or a cut file is sometimes specified by an ink
  reference that no amount of aiming at a photograph will land on.
* Picking started from the palette sheet returns to it when picking ends
  (`state.pickReturnsToPalette`); picking started from the toolbar does not, so
  the toolbar never opens a sheet nobody asked for.

Known interaction, not a defect: Undo lives in the header, so undoing a removal
means closing the sheet first. That is what a modal `<dialog>` is for, and it is
true of every other sheet in this interface.

## What changed in the previous session (2026-08-08)

### The interface was rewritten

`web/index.html`, `web/app.js` and `web/styles.css` were replaced. The previous interface put the whole technical surface on screen at once and had no way to load a picture.

Three views now exist, one at a time: **load a picture**, **workspace**, **done**. Before a bitmap exists, the only thing on screen is the means of getting one.

* **Mobile-first CSS.** The narrow layout is the base; the two-panel desktop layout is a `min-width: 900px` enhancement. Touch targets are at least 44 px; `touch-action: none` on the canvas viewport so pan and pinch do not fight the page.
* **Naming.** Destinations are offered as "An illustration", "A screen print", "A vinyl or paper cut" with a sentence on what each does to the geometry. Controls state their result — "Your picture will be reduced to 4 flat colours", "45% of the picture". The default entry name changed from `Scan N` to `Colour N` in `settings.py`, which also changes SVG layer labels.
* **Progressive disclosure.** Backend, backdrop and geometry reset live behind a closed `Fine-tuning` disclosure. Backdrop matching/output modes appear only once a backdrop colour is chosen. Colour-reach controls appear only for pinned entries. Per-scan channel tolerances and trace profiles sit inside each colour's sheet, behind a further disclosure. Nothing was removed.
* **Picking is additive.** `addPickedColour` grows the palette (and `scanCount`) when no automatic entry is left, instead of overwriting a colour the user picked earlier. It also refuses to add a duplicate hex.

### The magnifier picker (§9.3)

Press and drag on the picture: a 13-source-pixel magnifier with a pixel grid and a marked centre pixel follows above the contact point, showing the hex live, and commits on release. Sample size — one pixel / 5×5 median / 15×15 dominant — is an on-screen segmented control, because a touchscreen has no modifier keys.

The live readout is computed client-side (`app.js::sampleLocally`, mirroring `server/api.py::sample_source_color`) so there is no network round trip inside a drag; the value actually committed still comes from the server.

Keyboard picking exists and was tested: arrows move the sample point one source pixel at a time, Shift by ten, Enter commits. Without it, picking would be pointer-only.

### Browser image loading (§9.4.2)

* `POST /api/load_image` takes a base64 data URI, decodes in memory (`server/uploads.py`), and never writes it to disk.
* `POST /api/export` returns the assembled SVG for download and deliberately **does not** end the session.
* `standalone.py` now takes the image path as optional (`nargs="?"`); `run_without_source` serves the loading screen.
* `AppSession.commit_target` resolves to `document` / `file` / `download`, and the interface names its primary button after it.
* Loading a picture in a session that was launched with a path clears `output_path`, so the result cannot be written over an unrelated file's output. Covered by `test_standalone_host.py::test_swapping_the_image_in_the_browser_leaves_the_named_output_alone`.
* Uploads above 4 MP are downscaled (`MAX_WORKING_PIXELS`) with a notice stating the traced dimensions, because §17.4 preview scaling does not exist and a 12 MP photo makes the pipeline look hung. Deterministic: fixed budget, explicit LANCZOS.

### Server-side supporting changes

* `MAX_REQUEST_BYTES` (48 MiB) in `app_server.py`, answered as 413 **without reading the body**. §9.1 always required a payload limit; image loading is what made it load-bearing.
* `sample_source_color` extracted from the endpoint and given the third (dominant) mode.
* `PipelineController` now reports `coveragePercent` per scan. `claims_stats` covers pinned entries only, so the interface previously had to show automatic colours as "0% of the picture", which reads as "found nothing". Reported only — no geometry depends on it.

### `SPEC.md`

§9.2 was rewritten (mobile-first, progressive disclosure, naming, revised preview modes and palette rows), §9.3 gained the sample-size and aiming requirements, and §9.4.2 gained a "Browser-supplied source bitmaps" subsection. §9.4.3 and §9.4.5 note that a browser-supplied bitmap has no sidecar.

Two deliberate reductions, both recorded in the spec text itself:

* the six preview modes became three — the quantized, vector and production modes rendered identical geometry, so offering them separately implied a difference the pipeline does not produce;
* "Preview quality" was dropped from the main controls, because §17.4 preview scaling is unimplemented and §9.2.1 forbids showing a control that does nothing.

## Validation performed this session (2026-08-09)

| Command | Result | Notes |
| ------- | ------ | ----- |
| `uv run --with pytest pytest tests/ --ignore=test_schema.py --ignore=test_selection.py` | Passed | `376 passed`, unchanged by this session's work — the change is entirely in `web/` |
| Scripted Chromium session, 390×844 and 1440×900 | Passed | 18 assertions: the stepper and the add-a-colour button are absent; the sheet lists four entries with colour and coverage; *Add from the picture* closes the sheet, picks, and returns with five; removal shortens the list and moves focus deliberately; the last entry's Remove is disabled; Escape closes; no console errors |
| Scripted Chromium session, second pass | Passed | The strip's `+` picks without opening the sheet; undo reverses both an add and a removal; the colour sheet is reachable only as "Change …" with an Apply button; Remove is reachable and usable from the keyboard |

## Validation performed in the previous session (2026-08-08)

| Command | Result | Notes |
| ------- | ------ | ----- |
| `pytest tests/ --ignore=test_schema.py --ignore=test_selection.py` | Passed | `337 passed` (277 → 337: `test_uploads.py` 19, `test_app_server.py` 12, plus additions to `test_api.py`, `test_standalone_host.py` and `test_pipeline.py`) |
| `scripts/check_phase0.py` | 11/12 | Only the unit-suite check fails, and only because `inkex` is uninstallable here — see above |
| Live API smoke test (`curl`) | Passed | Session with no image → load → sample (`15x15_dominant`) → export produced a 3,975-byte SVG with `pt:` provenance |
| Scripted Chromium session, iPhone 13 viewport | Passed | Load → magnifier pick → keyboard pick → stepper → lock exact → remove → backdrop selection → preset save/apply → download. Zero console errors |
| Scripted Chromium session, 1440×900 | Passed | Same flow; two-panel layout, panel scrolls internally (`documentElement.scrollHeight === innerHeight`) |

## Defects found by browser testing and fixed

None of these were caught by the Python suite, and none would have been caught by reading the code:

1. **`[hidden]` did nothing.** A class rule that sets `display` outranks the UA's `[hidden] { display: none }`, so the "Picture changed" badge, the picking bar and the backdrop detail controls were permanently visible. Fixed with a global `[hidden] { display: none !important }`.
2. **The zoom buttons were dead.** The viewport's `pointerdown` handler called `setPointerCapture`, which retargeted the follow-up `click` away from the HUD buttons inside it. Fixed by ignoring pointerdowns that originate on a `button`.
3. **Automatic colours reported "0% of the picture".** Led to the `coveragePercent` change above.
4. **The desktop layout scrolled the whole page** instead of scrolling the controls panel: `.workspace` had `flex: 1` inside a container with only `min-height`, so it sized to content and the explicit `height` was ignored.

Also fixed: duplicate backend warnings (one message repeated once per scan), and a `/favicon.ico` 404 in the console (an empty `data:` icon link, no remote asset).

## Unverified assumptions

* [ ] The scripted browser sessions used Chromium only. Safari on iOS is the platform where `dialog`, `touch-action`, `env(safe-area-inset-*)` and pointer capture most plausibly differ, and it has not been tried.
* [ ] Real-device touch was never used — Playwright's synthetic touch is not a fingertip, and the magnifier's offset above the contact point is tuned by eye.
* [ ] No accessibility audit was run this session. Keyboard paths were exercised; a screen reader was not.
* [ ] The Inkscape host's changes (`session.image_name`, `can_load_image` refusing uploads) are unexercised — `inkex` will not install here.
* [ ] Drag-to-reorder in the swatch list was implemented but not clicked; the Move back / Move forward buttons in each colour's sheet were.
* [ ] The palette sheet has no reordering of its own. Layer order is still changed from the strip or from a colour's sheet, and whether that separation reads as sensible has not been tried on anyone.
* [ ] A palette grown well past a handful of colours was never looked at: the sheet scrolls, but no one has picked twenty colours from a photograph to see what that list becomes.

## Immediate next actions

1. **Add `tests/ui/` with Playwright.** Still the highest-value gap in Phase 2, and this session widened it: the palette sheet was verified by a scripted browser session that is not in the repository, exactly like the interface work before it. The 18 assertions written for it are the outline of that suite.
2. Fix the container so `inkex` installs, or make those two test modules skip when it is absent, so the Phase 0 gate is meaningful again.
3. Accessibility pass with a real screen reader, plus axe-core, before calling §29 Verified.
4. §27 missing-source dialog — still requires deferring the Inkscape host's early `PaletteTraceError`.
5. Implement §17.4 preview scaling, which would let `MAX_WORKING_PIXELS` be raised or removed.

## Handoff freshness checks

The next agent must consider this handoff stale when:

* the recorded HEAD does not match the current relevant history;
* files in the working set were substantially changed afterward;
* tests contradict the recorded status;
* the implementation phase changed;
* a referenced symbol no longer exists;
* `SPEC.md` changed in a relevant section.
