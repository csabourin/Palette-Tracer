# Palette Trace — Current Handoff

> This file records the current implementation state.
> `SPEC.md` remains the authoritative implementation contract.
> Verify this handoff against Git and the current code before relying on it.

## Session metadata

* **Last updated:** 2026-08-10
* **Updated by:** Claude
* **Current branch:** `claude/svg-settings-save-error-719b3p`, branched from `d20edaf` (`origin/master`, PR #13 merged)
* **Current phase:** Phase 2 — Portable interface
* **Primary objective this session:** A reported failure on Replit — saving the SVG or saving settings answered instantly with `/api/user_presets was answered by something other than Palette Trace (HTTP 404)`.

## Start here

1. `git status` / `git log --oneline -5` — confirm this matches what's described below.
2. `python -m pytest tests -q --ignore=tests/unit/test_schema.py --ignore=tests/unit/test_selection.py` → expect `433 passed`. See "Environment limitation" for why those two are ignored.
3. `ruff check .` → expect `All checks passed!`.
4. Read the new "Defects found and fixed" row in `docs/IMPLEMENTATION_STATUS.md` before touching `server/app_server.py`.

## Environment limitation (read before believing a red suite)

`inkex` cannot be installed in this container, so `tests/unit/test_schema.py` and `tests/unit/test_selection.py` fail collection with `ModuleNotFoundError: No module named 'inkex'`. Unchanged this session; the previous handoff's diagnosis (a `scour`-versus-setuptools build failure, not a missing system library) was not re-tested.

Note the baseline count moved from `434 passed` to `433 passed` **before** this session's work: that is the previous session's arithmetic, not a deleted test. This session added 8 tests, so the number to expect now is `433`, measured, not inferred.

## What changed this session (2026-08-10)

### The reported fault: an API request that was never answered at all

`do_GET` and `do_POST` called `handle_api_request` unguarded. `BaseHTTPRequestHandler` answers a handler that raises by **dropping the connection** — the traceback goes to the console and *nothing is sent*. Only `PaletteTraceError` inside `/api/load_image` was ever caught.

That is the one thing this API must never do. Every `/api/` reply is JSON, and `web/app.js::api()` reads an unparseable reply as "answered by something other than Palette Trace". So a fault **inside** the application produced the one message that says the fault is **outside** it: on loopback the browser sees an empty reply, and behind Replit's reverse proxy it sees the proxy's own HTML 404 — the exact message reported, blaming an address that was correct.

Reproduced directly, before any change, with a handler patched to raise:

```
curl -X POST .../api/user_presets  →  curl exit 52 (Empty reply from server)
```

Fixed in `PaletteTraceRequestHandler._api_response`, which wraps every dispatch:

* `PaletteTraceError` → **400** with its own sentence, since those are written for the user;
* `MemoryError` → **500** naming memory, because the full-resolution trace behind an export is the largest allocation a session makes and a small container is where it fails;
* anything else → **500** naming the **exception type only**, with the full traceback printed to the console. §9.1 keeps local filesystem paths out of browser-visible data and an exception message routinely carries one — `test_the_message_the_browser_gets_carries_no_local_path` pins that.

`log_message` stays silenced for ordinary requests; `_log_failure` prints only for a handler that raised, where it is the only record of why.

### What was most likely raising

Not established — the container the fault occurred in was not available. `user_presets.get_user_presets_dir()` calls `mkdir` under `Path.home()` on every save, and a container is exactly where that fails (read-only image, a `$HOME` the process does not own). It now raises `PaletteTraceError` with the OS's own reason instead of a bare `OSError`, and so does the preset write itself.

End-to-end proof of the pairing, with `$HOME` pointed at a regular file so `mkdir` raises `NotADirectoryError`:

* before: `POST /api/user_presets` → empty reply, curl exit 52;
* after: `400 {"error": "Saved settings live in a folder this machine will not let Palette Trace create (Not a directory). Nothing was saved."}`, and the session keeps serving.

**This is a plausible cause, not a confirmed one.** What is confirmed is the mechanism: whatever raised, the user could not have been told what it was.

### Export no longer holds a preview across the run that replaces it

`_build_result_svg` took a local reference to the cached preview output, then ran the full-resolution pipeline, so both were resident at once. `_run_pipeline` now clears `session.controller` and `session.pipeline_output` before constructing the new controller, and `_build_result_svg` reads through the session rather than into a local first.

This is the second mechanism that produces the reported symptom — an OOM kill leaves the port empty, and Replit's proxy answers on its own behalf with a 404, which is documented in `.replit` from an earlier session. The `~466 MB` peak in the `MAX_WORKING_PIXELS` blocker was measured *with* that overlap.

### The client message now points somewhere useful

`api()`'s non-JSON message said "check the address you opened, and anything sitting in front of the server". Now that the server answers JSON for its own failures too, an unparseable reply almost always means the server is gone, so the message says that and asks for a console check and a reload.

## Validation performed this session (2026-08-10)

| Command | Result | Notes |
| ------- | ------ | ----- |
| `python -m pytest tests -q --ignore=tests/unit/test_schema.py --ignore=tests/unit/test_selection.py` | Passed | `433 passed in 22.48s`, including the 8 added this session |
| `ruff check .` | Passed | `All checks passed!` |
| Live server, happy path | Passed | `python -m palette_trace.standalone --no-browser` on `PORT=8126`: raw-bytes `load_image` 200, `POST /api/user_presets` 200, `POST /api/export` 200 with a 2 437-byte SVG, `GET /api/user_presets` 200, empty preset name still 400 |
| Live server, unwritable `$HOME` | Passed | `PORT=8127` with `$HOME` on a regular file: the 400 quoted above, and `/api/session` still 200 afterwards |
| Live server, handler patched to raise (pre-change) | Reproduced the fault | curl exit 52, empty reply, traceback in the console only |

Not run this session: `make conformance`, `scripts/check_phase0.py`. Nothing changed in the pipeline, the backends or the geometry path — the diff is the HTTP layer, preset storage, one client message and two documents.

## Unverified assumptions

Carried forward, none of them touched this session:

* [ ] Safari on iOS has never been tried; the scripted browser sessions were Chromium only.
* [ ] Real-device touch was never used.
* [ ] No accessibility audit has been run with a real screen reader.
* [ ] The Inkscape host is unexercised — `inkex` will not install here.
* [ ] Drag-to-reorder in the swatch list was implemented but never clicked.
* [ ] A palette grown well past a handful of colours was never looked at.

New this session:

* [ ] **The Replit failure was never reproduced in a Replit container.** Both the mechanism and its fix were demonstrated locally. What reached the user's browser is consistent with either an unhandled exception or an OOM kill, and both are now addressed, but neither was observed there.
* [ ] **The preset-directory theory is unconfirmed.** `mkdir` under `Path.home()` is the most likely thing to raise on a container, and it was the only candidate testable from here. If the fault recurs, the console now names the real one — ask for that traceback first.
* [ ] **The lower export peak is not measured.** Releasing the preview before the full-resolution run must reduce the `~466 MB` figure; by how much, nobody has measured.

## Immediate next actions

1. **If the fault recurs on Replit, read the console.** The traceback is now printed there and the browser message names the exception type. That is the whole point of this session's change — do not re-theorise without it.
2. **Add `tests/ui/` with Playwright.** Still the highest-value gap in Phase 2.
3. **Make `test_schema.py` and `test_selection.py` skip when `inkex` is absent** rather than fail collection. A module-level `pytest.importorskip("inkex")` would make `make test` and the Phase 0 gate meaningful immediately; the gate has reported 11/12 for an environmental reason across several sessions.
4. Re-measure peak RSS on a real Replit container now that preview and export no longer overlap, and decide whether `MAX_WORKING_PIXELS` can be raised.
5. Accessibility pass with a real screen reader, plus axe-core, before calling §29 Verified.
6. §27 missing-source dialog — still requires deferring the Inkscape host's early `PaletteTraceError`.
7. **Decide whether §28 manual-edit detection is wanted.** Criterion 26 is honestly "Not built"; the hash is already on the group.

## Handoff freshness checks

The next agent must consider this handoff stale when:

* the recorded HEAD does not match the current relevant history;
* files in the working set were substantially changed afterward;
* tests contradict the recorded status;
* the implementation phase changed;
* a referenced symbol no longer exists;
* `SPEC.md` changed in a relevant section.
