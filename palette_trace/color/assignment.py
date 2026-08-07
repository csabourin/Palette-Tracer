"""
Nearest-entry assignment for unclaimed pixels (SPEC §14.4, §15.6).

Pinned entries reserve pixels within their reach (§14.1-14.3). Everything left
over belongs to the automatic scans, and each such pixel goes to the cluster it
is nearest to. Without this step the automatic palette would be computed and then
ignored, and every unclaimed pixel would land in a single scan.

Distances are measured in OKLab, where Euclidean distance is perceptually
meaningful — plain RGB distance is explicitly forbidden (§35).
"""

import numpy as np

from palette_trace.color.conversion import hex_to_srgb, srgb_to_oklab

#: §14.4 policies for pixels no automatic scan can take.
NEAREST_AVAILABLE = "nearest_available"
DROP = "drop"


def oklch_image_to_oklab(oklch: np.ndarray) -> np.ndarray:
    """Converts a stored (h, w, 3) OKLCH image to OKLab."""
    lightness = oklch[..., 0]
    chroma = oklch[..., 1]
    hue = np.radians(oklch[..., 2])
    return np.stack([lightness, chroma * np.cos(hue), chroma * np.sin(hue)], axis=-1)


def entry_centres(entries: list) -> tuple:
    """
    Returns (entry_ids, centres) for entries that have a resolved output colour.

    Entries without a colour are skipped rather than defaulted to black, which
    would silently pull unrelated pixels towards them.
    """
    ids = []
    centres = []
    for entry in entries:
        colour = (entry.get("output") or {}).get("color") or {}
        hex_value = colour.get("srgb") or (entry.get("sourceAnchor") or {}).get("srgb")
        if not hex_value:
            continue
        ids.append(entry["id"])
        centres.append(srgb_to_oklab(*hex_to_srgb(hex_value)))

    return ids, np.asarray(centres, dtype=np.float64) if centres else np.empty((0, 3))


def assign_nearest(
    oklab_image: np.ndarray,
    mask: np.ndarray,
    centres: np.ndarray,
) -> np.ndarray:
    """
    Returns an int array of centre indices for masked pixels, -1 elsewhere.

    Ties resolve to the lowest index, which makes the result depend only on
    palette order and therefore deterministic (§34.30).
    """
    height, width = mask.shape
    assignment = np.full((height, width), -1, dtype=np.int32)

    if centres.shape[0] == 0 or not np.any(mask):
        return assignment

    pixels = oklab_image[mask]                       # (n, 3)
    # (n, k) squared distances; k is the scan count, at most 64.
    distances = ((pixels[:, None, :] - centres[None, :, :]) ** 2).sum(axis=2)
    assignment[mask] = np.argmin(distances, axis=1).astype(np.int32)
    return assignment


def distribute_unclaimed(
    oklch_image: np.ndarray,
    unclaimed_mask: np.ndarray,
    automatic_entries: list,
    pinned_entries: list,
    policy: str = NEAREST_AVAILABLE,
) -> dict:
    """
    Maps entry id to the mask of unclaimed pixels it takes (§14.4).

    Automatic entries absorb the unclaimed pixels. When there are none, the
    policy decides: `nearest_available` falls back to the nearest enabled pinned
    entry, `drop` leaves the pixels unassigned and transparent.
    """
    if not np.any(unclaimed_mask):
        return {}

    targets = automatic_entries
    if not targets:
        if policy == DROP:
            return {}
        targets = pinned_entries

    ids, centres = entry_centres(targets)
    if not ids:
        return {}

    oklab_image = oklch_image_to_oklab(oklch_image)
    assignment = assign_nearest(oklab_image, unclaimed_mask, centres)

    return {
        entry_id: (assignment == index)
        for index, entry_id in enumerate(ids)
        if np.any(assignment == index)
    }
