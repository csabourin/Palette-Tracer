"""
Deterministic constrained K-Means clustering in OKLab coordinates.
"""

import math
import numpy as np
from palette_trace.color.conversion import oklab_to_srgb, srgb_to_hex

def oklab_distance_sq(p1: tuple[float, float, float], p2: tuple[float, float, float]) -> float:
    dL = p1[0] - p2[0]
    da = p1[1] - p2[1]
    db = p1[2] - p2[2]
    return dL * dL + da * da + db * db

def run_deterministic_quantization(
    histogram: list[dict],
    target_count: int,
    fixed_centers: list[tuple[float, float, float]] = None,
    min_separation_oklab: float = 0.02,
    max_iterations: int = 50,
    convergence_threshold: float = 1e-5,
) -> list[dict]:
    """
    Runs deterministic K-Means clustering on OKLab histogram entries.
    Returns list of generated automatic centroid dicts: [{'hex': '#RRGGBB', 'oklab': (L,a,b), 'count': int}].
    """
    if not histogram or target_count <= 0:
        return []

    fixed_centers = fixed_centers or []
    fixed_count = len(fixed_centers)

    # Cap target_count to number of available histogram entries
    actual_target = min(target_count, len(histogram))

    # 1. Initialize centers
    # Sort histogram by weight descending, then packed_srgb ascending for determinism
    sorted_hist = sorted(histogram, key=lambda item: (-item["weight"], item["packed_srgb"]))

    centers = list(fixed_centers)

    # Farthest-point initialization for remaining centers
    while len(centers) < fixed_count + actual_target and sorted_hist:
        if not centers:
            # First center: most frequent
            first = sorted_hist[0]
            centers.append(first["oklab"])
        else:
            best_cand = None
            max_min_dist = -1.0
            best_packed_srgb = float("inf")

            for item in sorted_hist:
                pt = item["oklab"]
                # Min distance to existing centers
                min_d = min(oklab_distance_sq(pt, c) for c in centers)
                weighted_d = min_d * item["weight"]

                if weighted_d > max_min_dist or (
                    abs(weighted_d - max_min_dist) < 1e-9 and item["packed_srgb"] < best_packed_srgb
                ):
                    max_min_dist = weighted_d
                    best_cand = pt
                    best_packed_srgb = item["packed_srgb"]

            if best_cand is not None:
                centers.append(best_cand)
            else:
                break

    auto_centers = centers[fixed_count:]
    if not auto_centers:
        return []

    # 2. Iterate K-Means
    for iteration in range(max_iterations):
        # Assign histogram items to nearest center
        clusters = [[] for _ in range(len(centers))]
        cluster_weights = [0 for _ in range(len(centers))]

        for item in histogram:
            pt = item["oklab"]
            w = item["weight"]
            best_idx = 0
            best_dist = float("inf")
            for idx, c in enumerate(centers):
                d = oklab_distance_sq(pt, c)
                if d < best_dist:
                    best_dist = d
                    best_idx = idx

            clusters[best_idx].append(item)
            cluster_weights[best_idx] += w

        # Recalculate auto centers
        max_shift = 0.0
        new_auto_centers = []

        for auto_idx in range(len(auto_centers)):
            c_idx = fixed_count + auto_idx
            items = clusters[c_idx]
            tot_w = cluster_weights[c_idx]

            if tot_w > 0 and items:
                sum_L = sum(it["oklab"][0] * it["weight"] for it in items)
                sum_a = sum(it["oklab"][1] * it["weight"] for it in items)
                sum_b = sum(it["oklab"][2] * it["weight"] for it in items)
                new_c = (sum_L / tot_w, sum_a / tot_w, sum_b / tot_w)
            else:
                new_c = auto_centers[auto_idx]

            shift = math.sqrt(oklab_distance_sq(auto_centers[auto_idx], new_c))
            max_shift = max(max_shift, shift)
            new_auto_centers.append(new_c)

        auto_centers = new_auto_centers
        centers[fixed_count:] = auto_centers

        if max_shift < convergence_threshold:
            break

    # 3. Format results
    result_entries = []
    for auto_idx, c in enumerate(auto_centers):
        c_idx = fixed_count + auto_idx
        r, g, b = oklab_to_srgb(c[0], c[1], c[2])
        hex_c = srgb_to_hex(r, g, b)
        result_entries.append({
            "hex": hex_c,
            "oklab": c,
            "weight": cluster_weights[c_idx],
        })

    return result_entries
