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
    height, width, _ = oklch_image.shape
    total_pixels = height * width

    evaluators = [
        PinnedClaimEvaluator(entry, idx)
        for idx, entry in enumerate(pinned_entries)
    ]

    claims = np.full((height, width), None, dtype=object)
    claim_counts = {e["id"]: 0 for e in pinned_entries}

    if not evaluators:
        return claims, {e["id"]: 0.0 for e in pinned_entries}

    L_arr = oklch_image[:, :, 0]
    C_arr = oklch_image[:, :, 1]
    h_arr = oklch_image[:, :, 2]

    EPSILON = 1e-6

    for y in range(height):
        for x in range(width):
            L, C, h = L_arr[y, x], C_arr[y, x], h_arr[y, x]

            best_eval = None
            best_score = float("inf")

            for ev in evaluators:
                eligible, score = ev.evaluate_pixel(L, C, h)
                if not eligible:
                    continue

                if score < best_score - EPSILON:
                    best_score = score
                    best_eval = ev
                elif abs(score - best_score) <= EPSILON:
                    # Tie breakers: explicit claim priority -> UUID lexical order
                    if best_eval is None:
                        best_eval = ev
                        best_score = score
                    elif ev.priority_index < best_eval.priority_index:
                        best_eval = ev
                    elif ev.priority_index == best_eval.priority_index:
                        if ev.id < best_eval.id:
                            best_eval = ev

            if best_eval is not None:
                claims[y, x] = best_eval.id
                claim_counts[best_eval.id] += 1

    stats = {
        eid: (count / float(total_pixels)) * 100.0
        for eid, count in claim_counts.items()
    }
    return claims, stats
