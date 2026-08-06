# Palette Trace — Implementation Status

This document tracks durable implementation progress against `SPEC.md`.

`SPEC.md` is authoritative. This document records evidence; it does not redefine requirements.

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

| Phase                                       | Status      | Evidence | Remaining work                                                                                            |
| ------------------------------------------- | ----------- | -------- | --------------------------------------------------------------------------------------------------------- |
| Phase 0 — Engine and colour-model spike     | Not started | —        | Backend protocol, conformance harness, backend comparison, colour model and deterministic quantizer spike |
| Phase 1 — Headless core                     | Not started | —        | —                                                                                                         |
| Phase 2 — Portable interface                | Not started | —        | —                                                                                                         |
| Phase 3 — Inkscape integration              | Not started | —        | —                                                                                                         |
| Phase 4 — Presets and production validation | Not started | —        | —                                                                                                         |
| Phase 5 — Packaging and release             | Not started | —        | —                                                                                                         |

## Active implementation slice

* **Current phase:**
* **Current objective:**
* **Primary specification sections:**
* **Associated handoff:** `.ai/HANDOFF.md`
* **Technical decision records:**

## Requirement tracking

| Specification | Requirement                                                         | Status      | Implementation evidence | Validation evidence | Notes |
| ------------- | ------------------------------------------------------------------- | ----------- | ----------------------- | ------------------- | ----- |
| §13.1         | Linked Colour reach uses versioned deterministic interpolation      | Not started | —                       | —                   | —     |
| §13.4         | Low-chroma colours suppress unreliable hue contribution             | Not started | —                       | —                   | —     |
| §14.3         | Conflicting pinned claims resolve deterministically                 | Not started | —                       | —                   | —     |
| §15           | Automatic palette generation is deterministic                       | Not started | —                       | —                   | —     |
| §17.1         | Classification uses one integer label map                           | Not started | —                       | —                   | —     |
| §23.1         | Backends implement the canonical protocol                           | Not started | —                       | —                   | —     |
| §23.6         | At least two backend candidates are evaluated                       | Not started | —                       | —                   | —     |
| §29           | Interface follows accessibility requirements                        | Not started | —                       | —                   | —     |
| §31           | Local interface and backend processing follow security requirements | Not started | —                       | —                   | —     |

Add rows as implementation begins. Do not create hundreds of speculative rows before work reaches them.

## MVP acceptance criteria

| Criterion | Summary                                                       | Status      | Evidence |
| --------: | ------------------------------------------------------------- | ----------- | -------- |
|         1 | Open one selected embedded or linked local bitmap             | Not started | —        |
|         2 | Restore settings stored on the image                          | Not started | —        |
|         3 | Select 1–64 scans                                             | Not started | —        |
|         4 | Pick colours from the preview                                 | Not started | —        |
|         5 | Picked colours become exact output colours                    | Not started | —        |
|         6 | Every picked colour has Colour reach                          | Not started | —        |
|         7 | Hue, chroma and lightness tolerances are separately editable  | Not started | —        |
|         8 | Neutral colours handle hue reliably                           | Not started | —        |
|         9 | Remaining scans are generated deterministically               | Not started | —        |
|        10 | Automatic colours account for pinned colours                  | Not started | —        |
|        11 | Conflicting claims resolve deterministically                  | Not started | —        |
|        12 | Background can be kept, omitted or replaced                   | Not started | —        |
|        13 | Scans inherit or override trace profiles                      | Not started | —        |
|        14 | Different scans can use materially different trace profiles   | Not started | —        |
|        15 | Destination presets produce distinct policies                 | Not started | —        |
|        16 | Stacked and trapped output work                               | Not started | —        |
|        17 | Exclusive-layer output maintains exclusive raster ownership   | Not started | —        |
|        18 | Laser output creates named operation groups                   | Not started | —        |
|        19 | A portable or cross-platform backend passes conformance tests | Not started | —        |
|        20 | Backend selection is abstracted                               | Not started | —        |
|        21 | Output is grouped and labelled                                | Not started | —        |
|        22 | Settings are stored on the source image                       | Not started | —        |
|        23 | Deleting the image deletes image-specific settings            | Not started | —        |
|        24 | Saved presets apply to another image                          | Not started | —        |
|        25 | Reapplying can replace the linked generated group             | Not started | —        |
|        26 | Manual-edit risk is detected or warned                        | Not started | —        |
|        27 | No source data leaves the machine                             | Not started | —        |
|        28 | Interface is keyboard accessible                              | Not started | —        |
|        29 | Errors do not corrupt the document                            | Not started | —        |
|        30 | Identical inputs produce deterministic results                | Not started | —        |

## Deferred requirements

| Specification | Requirement | Reason | Observable impact | Proposed path |
| ------------- | ----------- | ------ | ----------------- | ------------- |
| —             | —           | —      | —                 | —             |

## Known blockers

| Blocker | Affected requirements | Evidence | Owner or next action |
| ------- | --------------------- | -------- | -------------------- |
| —       | —                     | —        | —                    |

## Technical decisions

| Decision record                                        | Summary                                                 | Status  |
| ------------------------------------------------------ | ------------------------------------------------------- | ------- |
| `docs/decisions/ADR-0001-reference-tracing-backend.md` | Select the reference backend using conformance evidence | Pending |

## Validation environments

| Environment  | Status       | Last verified | Notes |
| ------------ | ------------ | ------------- | ----- |
| Windows      | Not verified | —             | —     |
| macOS        | Not verified | —             | —     |
| Linux        | Not verified | —             | —     |
| Inkscape 1.4 | Not verified | —             | —     |

## Maintenance rules

* Update this document only when durable status changes.
* Link status to files, symbols, tests, commands, commits, or decision records.
* Do not mark placeholders or mocks as implemented.
* Do not mark skipped tests as passed.
* Do not mark an entire phase complete when only its structure exists.
* Remove claims that are disproven by the current implementation.
* Use `.ai/HANDOFF.md` for temporary, session-specific continuation details.
