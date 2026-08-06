# Palette Trace — Current Handoff

> This file records the current implementation state.
> `SPEC.md` remains the authoritative implementation contract.
> Verify this handoff against Git and the current code before relying on it.

## Session metadata

* **Last updated:** YYYY-MM-DD
* **Updated by:** Agent or developer name
* **Current branch:** `branch-name`
* **HEAD commit:** `commit hash or "uncommitted"`
* **Working tree:** Clean / modified
* **Current phase:** Phase 0 / 1 / 2 / 3 / 4 / 5
* **Primary objective:** Brief statement of the current task

## Start here

The next agent should begin by:

1. Running `git status`.
2. Reviewing the current diff.
3. Reading the specification sections listed below.
4. Opening the files in the working set.
5. Running the smallest relevant validation command before making further changes.

## Current objective

Describe the concrete behaviour currently being implemented.

Include the observable completion condition.

Example:

> Implement deterministic interpolation for linked Colour reach values and validate the five normative mapping anchors.

## Relevant specification

* `SPEC §X.Y — Section name`
* `SPEC §X.Y — Section name`
* `SPEC §33 — Applicable testing requirements`
* `SPEC §34.N — Applicable MVP acceptance criterion`
* `SPEC §35 — Applicable anti-shortcut requirement`

## Current status

### Completed

* Concrete behaviour that currently works.
* Concrete tests or evidence supporting it.

### In progress

* Work that has begun but is not yet complete.

### Blocked

* Blocking issue.
* Evidence or error.
* Required decision or missing capability.

### Not started but immediately relevant

* Work that logically follows the current task.

## Working set

Only these files are expected to be needed initially:

| File                           | Relevant symbols               | Why it matters    |
| ------------------------------ | ------------------------------ | ----------------- |
| `path/to/file.py`              | `ClassName`, `function_name()` | Brief explanation |
| `tests/unit/test_file.py`      | `test_specific_behaviour()`    | Brief explanation |
| `palette_trace/data/file.json` | Mapping or preset ID           | Brief explanation |

## Architecture and data flow

Describe only the architecture needed to resume this task.

Example:

```text
Reach slider
→ versioned reach mapping
→ channel tolerances
→ reserved-claim eligibility
→ normalized claim score
→ label-map ownership
```

Do not summarize the complete application.

## Changes made in the latest session

### `path/to/file`

* Changed:
* Reason:
* Specification reference:

### `path/to/test`

* Added or updated:
* Behaviour covered:

## Decisions made

| Decision       | Reason              | Evidence               | Revisit when |
| -------------- | ------------------- | ---------------------- | ------------ |
| Brief decision | Technical rationale | Test, spike, or source | Condition    |

Distinguish confirmed decisions from temporary assumptions.

## Validation performed

| Command         | Result                    | Notes           |
| --------------- | ------------------------- | --------------- |
| `exact command` | Passed / Failed / Skipped | Relevant output |
| `exact command` | Passed / Failed / Skipped | Relevant output |

Do not write “tests pass” without listing the command that was executed.

## Known failures

### Failure title

* **Observed behaviour:**
* **Expected behaviour:**
* **Relevant files:**
* **Reproduction command or steps:**
* **Error output:**
* **Likely cause:**
* **Attempts already made:**
* **Recommended next investigation:**

## Failed or rejected approaches

### Approach

* What was attempted:
* Why it was rejected:
* Evidence:
* Circumstances under which it may be worth reconsidering:

## Unverified assumptions

* [ ] Assumption requiring verification
* [ ] Platform-specific behaviour not yet tested
* [ ] Dependency capability not yet confirmed
* [ ] Specification interpretation requiring a decision

## Immediate next actions

1. The smallest concrete action the next agent should perform.
2. The next validation command.
3. The next implementation step after validation.

## User-visible behaviour

Describe what a user can currently observe.

Do not list internal files here unless they affect user behaviour.

## Handoff freshness checks

The next agent must consider this handoff stale when:

* the recorded HEAD does not match the current relevant history;
* files in the working set were substantially changed afterward;
* tests contradict the recorded status;
* the implementation phase changed;
* a referenced symbol no longer exists;
* `SPEC.md` changed in a relevant section.

## Latest session summary

* Maximum ten concise bullets.
* Include only information another agent needs to continue.
* Remove obsolete bullets during the next update.
