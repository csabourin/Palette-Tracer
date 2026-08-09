"""
Pixel claim matching and conflict resolution algorithms.
"""

from typing import List, Dict, Optional
import numpy as np
from palette_trace.color.conversion import srgb_to_oklch, shortest_hue_distance
from palette_trace.color.reach import get_tolerances_for_reach, calculate_hue_confidence

class PinnedClaimEvaluator:
    """Evaluates whether source pixels fall within reach of pinned palette entries."""

    def __init__(self, entry_dict: dict, priority_index: int):
        self.id = entry_dict["id"]
        self.priority_index = priority_index
        self.assignment = entry_dict.get("assignment", {})
        self.mode = self.assignment.get("mode", "reserve_within_reach")

        # Source anchor color
        anchor = entry_dict.get("sourceAnchor", {})
        if anchor and "srgb" in anchor:
            hex_c = anchor["srgb"]
            r = int(hex_c[1:3], 16) / 255.0
            g = int(hex_c[3:5], 16) / 255.0
            b = int(hex_c[5:7], 16) / 255.0
            self.anchor_L, self.anchor_C, self.anchor_h = srgb_to_oklch(r, g, b)
        else:
            self.anchor_L, self.anchor_C, self.anchor_h = (0.5, 0.0, 0.0)

        # Tolerances
        channels = self.assignment.get("channels", {})
        mode = channels.get("mode", "linked")

        if mode == "linked":
            overall = self.assignment.get("overallReach", 25)
            tols = get_tolerances_for_reach(overall)
            self.hue_tol = tols["hue"]
            self.chroma_tol = tols["chroma"]
            self.light_tol = tols["lightness"]

            self.hue_enabled = self.anchor_C >= 0.02  # Neutral color guardrail
            self.chroma_enabled = True
            self.light_enabled = True

            self.hue_weight = 1.0
            self.chroma_weight = 1.0
            self.light_weight = 1.0
        else:
            hue_cfg = channels.get("hue", {})
            self.hue_enabled = hue_cfg.get("enabled", True) and (self.anchor_C >= 0.02)
            self.hue_tol = hue_cfg.get("tolerance", 10.0)
            self.hue_weight = hue_cfg.get("weight", 1.0)

            chroma_cfg = channels.get("chroma", {})
            self.chroma_enabled = chroma_cfg.get("enabled", True)
            self.chroma_tol = chroma_cfg.get("tolerance", 0.035)
            self.chroma_weight = chroma_cfg.get("weight", 1.0)

            light_cfg = channels.get("lightness", {})
            self.light_enabled = light_cfg.get("enabled", True)
            self.light_tol = light_cfg.get("tolerance", 0.06)
            self.light_weight = light_cfg.get("weight", 1.0)

    def evaluate_array(
        self,
        L: np.ndarray,
        C: np.ndarray,
        h: np.ndarray,
    ) -> tuple[np.ndarray, np.ndarray]:
        """
        Whole-image form of :meth:`evaluate_pixel`.

        Returns (eligible, score) arrays shaped like `L`. Ineligible pixels
        carry a score of `inf`, so a caller can take a running minimum without
        having to mask first. The arithmetic is identical to the scalar path —
        the scalar path is what the unit tests pin, and this is what actually
        runs on an image, so the two must not be allowed to drift.
        """
        shape = L.shape
        if self.mode != "reserve_within_reach":
            return (np.zeros(shape, dtype=bool), np.full(shape, np.inf))

        eligible = np.ones(shape, dtype=bool)
        weighted_sq_sum = np.zeros(shape, dtype=np.float64)
        weights_sum = np.zeros(shape, dtype=np.float64)

        if self.light_enabled:
            d_L = np.abs(L - self.anchor_L)
            tol_L = max(1e-6, self.light_tol)
            eligible &= d_L <= tol_L
            weighted_sq_sum += self.light_weight * (d_L / tol_L) ** 2
            weights_sum += self.light_weight

        if self.chroma_enabled:
            d_C = np.abs(C - self.anchor_C)
            tol_C = max(1e-6, self.chroma_tol)
            eligible &= d_C <= tol_C
            weighted_sq_sum += self.chroma_weight * (d_C / tol_C) ** 2
            weights_sum += self.chroma_weight

        if self.hue_enabled:
            # Hue confidence fades out as a pixel approaches neutral (§13.4), so
            # the weight — and therefore whether hue is consulted at all — is a
            # per-pixel quantity rather than a per-entry one.
            confidence = np.clip(C / 0.05, 0.0, 1.0)
            confidence = np.where(C <= 0.0, 0.0, confidence)
            effective = self.hue_weight * confidence
            consulted = effective > 1e-4

            difference = np.abs(h - self.anchor_h) % 360.0
            d_h = np.minimum(difference, 360.0 - difference)
            tol_h = max(1e-6, self.hue_tol)

            eligible &= ~consulted | (d_h <= tol_h)
            contribution = effective * (d_h / tol_h) ** 2
            weighted_sq_sum += np.where(consulted, contribution, 0.0)
            weights_sum += np.where(consulted, effective, 0.0)

        score = np.where(
            weights_sum <= 1e-6,
            0.0,
            weighted_sq_sum / np.where(weights_sum <= 1e-6, 1.0, weights_sum),
        )
        return (eligible, np.where(eligible, score, np.inf))

    def evaluate_pixel(self, L: float, C: float, h: float) -> tuple[bool, float]:
        """
        Returns (is_eligible, normalized_score).
        """
        if self.mode != "reserve_within_reach":
            return (False, float("inf"))

        weights_sum = 0.0
        weighted_sq_sum = 0.0

        # Lightness check
        if self.light_enabled:
            d_L = abs(L - self.anchor_L)
            tol_L = max(1e-6, self.light_tol)
            if d_L > tol_L:
                return (False, float("inf"))
            w = self.light_weight
            weighted_sq_sum += w * ((d_L / tol_L) ** 2)
            weights_sum += w

        # Chroma check
        if self.chroma_enabled:
            d_C = abs(C - self.anchor_C)
            tol_C = max(1e-6, self.chroma_tol)
            if d_C > tol_C:
                return (False, float("inf"))
            w = self.chroma_weight
            weighted_sq_sum += w * ((d_C / tol_C) ** 2)
            weights_sum += w

        # Hue check
        if self.hue_enabled:
            hue_conf = calculate_hue_confidence(C)
            effective_hue_weight = self.hue_weight * hue_conf

            if effective_hue_weight > 1e-4:
                d_h = shortest_hue_distance(h, self.anchor_h)
                tol_h = max(1e-6, self.hue_tol)
                if d_h > tol_h:
                    return (False, float("inf"))
                weighted_sq_sum += effective_hue_weight * ((d_h / tol_h) ** 2)
                weights_sum += effective_hue_weight

        if weights_sum <= 1e-6:
            return (True, 0.0)

        score = weighted_sq_sum / weights_sum
        return (True, score)


#: Two scores this close are the same score, and the tie goes to the entry that
#: comes first in the palette (§14.3).
CLAIM_EPSILON = 1e-6


def resolve_claim_indices(
    oklch_image: np.ndarray,
    pinned_entries: list[dict],
) -> tuple[np.ndarray, dict[str, float]]:
    """
    Classifies image pixels into pinned entry *positions*.

    Returns an int32 array holding the index into `pinned_entries` that won each
    pixel, or -1 where nothing claimed it, plus the per-entry claim percentages.

    This is the form the pipeline wants: an index array is one contiguous
    allocation, where the array of id strings that :func:`resolve_claimed_pixels`
    returns is one Python object per pixel.
    """
    height, width, _ = oklch_image.shape
    total_pixels = float(height * width) or 1.0

    winners = np.full((height, width), -1, dtype=np.int32)
    if not pinned_entries:
        return winners, {}

    evaluators = [
        PinnedClaimEvaluator(entry, idx)
        for idx, entry in enumerate(pinned_entries)
    ]

    L_arr = oklch_image[:, :, 0]
    C_arr = oklch_image[:, :, 1]
    h_arr = oklch_image[:, :, 2]

    best_score = np.full((height, width), np.inf)

    # Sequential, in palette order, exactly as the per-pixel version was: an
    # entry only displaces the incumbent by beating it outright, so a tie always
    # resolves to whichever entry was reached first.
    for index, evaluator in enumerate(evaluators):
        eligible, score = evaluator.evaluate_array(L_arr, C_arr, h_arr)
        better = eligible & (score < best_score - CLAIM_EPSILON)
        if not np.any(better):
            continue
        winners[better] = index
        best_score[better] = score[better]

    counts = np.bincount(
        winners.ravel() + 1, minlength=len(evaluators) + 1
    )[1:]
    stats = {
        entry["id"]: float(counts[index]) / total_pixels * 100.0
        for index, entry in enumerate(pinned_entries)
    }
    return winners, stats


def resolve_claimed_pixels(
    oklch_image: np.ndarray,
    pinned_entries: list[dict],
) -> tuple[np.ndarray, dict[str, float]]:
    """
    Classifies image pixels into claimed pinned entry IDs.
    Returns:
      claims_map: string array (or index array) of claimed palette entry IDs or None.
      stats: dict mapping entry_id -> percentage of image claimed.
    """
    winners, stats = resolve_claim_indices(oklch_image, pinned_entries)

    claims = np.full(winners.shape, None, dtype=object)
    for index, entry in enumerate(pinned_entries):
        claims[winners == index] = entry["id"]

    if not pinned_entries:
        return claims, {}
    return claims, stats
