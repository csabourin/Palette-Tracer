"""
Destination geometry policies (SPEC §20).

Four policies decide how per-scan masks relate to each other before tracing:

* ``stacked`` — scans are independent and painted bottom to top; lower scans are
  expanded by an underlap so no background shows through at the seams (§20.1).
* ``stacked_trapped`` — the same idea using the destination's trapping
  measurement, for screen printing (§20.2).
* ``exclusive_layers`` — every pixel belongs to exactly one scan, no expansion at
  all, for cutting workflows where overlap means a double cut (§20.3).
* ``separate_operations`` — exclusive ownership plus per-operation validation and
  grouping, for laser workflows (§20.4).
"""

import numpy as np

from palette_trace.masks.morphology import dilate_mask

STACKED = "stacked"
STACKED_TRAPPED = "stacked_trapped"
EXCLUSIVE_LAYERS = "exclusive_layers"
SEPARATE_OPERATIONS = "separate_operations"

#: Policies under which a pixel may belong to more than one scan.
OVERLAPPING_POLICIES = (STACKED, STACKED_TRAPPED)

#: Policies that require exclusive raster ownership (§20.3, §20.4, §34.17).
EXCLUSIVE_POLICIES = (EXCLUSIVE_LAYERS, SEPARATE_OPERATIONS)


def allows_overlap(policy: str) -> bool:
    """Whether the policy permits masks to be expanded into each other."""
    return policy in OVERLAPPING_POLICIES


def apply_underlap_to_mask(
    mask: np.ndarray,
    subject_silhouette: np.ndarray,
    underlap_px: float,
    preserve_outer_silhouette: bool = True,
) -> np.ndarray:
    """
    Expands a lower scan mask by `underlap_px` (§20.1).

    The expansion is intersected with the union of classified subject pixels, so
    underlap never grows the exterior silhouette unless that is explicitly
    disabled.
    """
    if underlap_px <= 0:
        return mask.copy()

    dilated = dilate_mask(mask, underlap_px)
    if preserve_outer_silhouette:
        return dilated & subject_silhouette
    return dilated


def apply_trapping_to_mask(
    mask: np.ndarray,
    subject_silhouette: np.ndarray,
    trapping_px: float,
    preserve_outer_silhouette: bool = True,
) -> np.ndarray:
    """Applies trapping expansion to a screen-print colour separation (§20.2)."""
    if trapping_px <= 0:
        return mask.copy()

    dilated = dilate_mask(mask, trapping_px)
    if preserve_outer_silhouette:
        return dilated & subject_silhouette
    return dilated


def enforce_exclusive_ownership(masks: dict, layer_order: list) -> dict:
    """
    Removes overlap between scan masks, resolving ties by layer order (§20.3).

    `masks` maps entry id to a boolean array; `layer_order` lists entry ids from
    bottom to top. Later entries win, matching the painter's-algorithm result a
    stacked render would produce, so exclusive output looks like stacked output
    without the double coverage.

    Returns a new dict; the input is not modified.
    """
    ordered = [eid for eid in layer_order if eid in masks]
    ordered += [eid for eid in masks if eid not in ordered]

    exclusive = {}
    claimed = None
    # Walk top to bottom so the topmost scan keeps every pixel it owns.
    for entry_id in reversed(ordered):
        mask = masks[entry_id].astype(bool)
        if claimed is None:
            claimed = np.zeros_like(mask, dtype=bool)
        owned = mask & ~claimed
        exclusive[entry_id] = owned
        claimed |= owned

    return {eid: exclusive[eid] for eid in ordered}


def validate_operation_masks(
    masks: dict,
    minimum_area_px2: int = 4,
) -> list:
    """
    Validates operation layers for fabrication destinations (§20.4).

    Reports, rather than silently correcting:

    * empty operation layers;
    * operations whose total area is below the minimum;
    * duplicate operations covering identical pixels.

    Closed-path and self-intersection validation belongs to the vector stage,
    which is where path data exists; those are reported by the pipeline.
    """
    warnings = []
    areas = {eid: int(np.count_nonzero(mask)) for eid, mask in masks.items()}

    for entry_id, area in areas.items():
        if area == 0:
            warnings.append({
                "code": "empty_operation",
                "entryId": entry_id,
                "message": "Operation layer contains no pixels and will produce no output.",
            })
        elif area < minimum_area_px2:
            warnings.append({
                "code": "operation_below_minimum_area",
                "entryId": entry_id,
                "message": (
                    f"Operation layer covers {area} px², below the "
                    f"{minimum_area_px2} px² minimum for this destination."
                ),
            })

    entry_ids = [eid for eid, area in areas.items() if area > 0]
    for index, first in enumerate(entry_ids):
        for second in entry_ids[index + 1:]:
            if np.array_equal(masks[first], masks[second]):
                warnings.append({
                    "code": "duplicate_operation",
                    "entryId": second,
                    "message": (
                        f"Operation layer is identical to {first!r} and would cut "
                        "the same geometry twice."
                    ),
                })

    return warnings
