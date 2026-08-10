"""Deterministic constrained K-Means clustering in OKLab coordinates."""

import math

import numpy as np

from palette_trace.color.conversion import oklab_to_srgb, srgb_to_hex


def oklab_distance_sq(
    p1: tuple[float, float, float],
    p2: tuple[float, float, float],
) -> float:
    d_l = p1[0] - p2[0]
    d_a = p1[1] - p2[1]
    d_b = p1[2] - p2[2]

    return d_l * d_l + d_a * d_a + d_b * d_b


def run_deterministic_quantization(
    histogram: list[dict],
    target_count: int,
    fixed_centers: list[tuple[float, float, float]] | None = None,
    min_separation_oklab: float = 0.02,
    max_iterations: int = 50,
    convergence_threshold: float = 1e-5,
) -> list[dict]:
    """
    Run deterministic K-Means clustering on OKLab histogram entries.

    Returns automatic centroid dictionaries:

    [
        {
            "hex": "#RRGGBB",
            "oklab": (L, a, b),
            "count": int,
        }
    ]
    """
    if not histogram or target_count <= 0:
        return []

    fixed_centers = fixed_centers or []
    fixed_count = len(fixed_centers)
    min_separation_sq = max(0.0, min_separation_oklab) ** 2

    actual_target = min(target_count, len(histogram))

    # Sort by weight descending and packed sRGB ascending so initialization
    # remains deterministic.
    sorted_histogram = sorted(
        histogram,
        key=lambda item: (-item["weight"], item["packed_srgb"]),
    )

    centers = list(fixed_centers)

    # Farthest-point initialization for automatic centres.
    #
    # The distance every candidate has to its nearest centre only ever falls,
    # so it is carried between rounds and lowered by the newest centre alone,
    # rather than recomputed against all of them. On a 6-bit histogram that is
    # the difference between one pass and sixty-four.
    candidates = np.asarray([item["oklab"] for item in sorted_histogram], dtype=np.float64)
    candidate_weights = np.asarray(
        [item["weight"] for item in sorted_histogram], dtype=np.float64
    )
    candidate_packed = np.asarray(
        [item["packed_srgb"] for item in sorted_histogram], dtype=np.int64
    )

    available = np.ones(len(sorted_histogram), dtype=bool)
    nearest = np.full(len(sorted_histogram), np.inf)
    for center in centers:
        nearest = np.minimum(nearest, ((candidates - np.asarray(center)) ** 2).sum(axis=1))

    while len(centers) < fixed_count + actual_target and available.any():
        if not centers:
            selected_index = 0
        else:
            # Ties are broken towards the lowest packed sRGB value, which makes
            # the choice depend on the colour rather than on iteration order.
            eligible = np.flatnonzero(available & (nearest >= min_separation_sq))
            if eligible.size == 0:
                # No remaining candidate satisfies the minimum separation.
                break

            weighted = nearest[eligible] * candidate_weights[eligible]
            best = weighted.max()
            tied = eligible[weighted >= best - max(1e-9, abs(best) * 1e-9)]
            selected_index = int(tied[np.argmin(candidate_packed[tied])])

        available[selected_index] = False
        chosen = tuple(candidates[selected_index])
        centers.append(chosen)
        nearest = np.minimum(
            nearest, ((candidates - candidates[selected_index]) ** 2).sum(axis=1)
        )

    auto_centers = centers[fixed_count:]

    if not auto_centers:
        return []

    # The assignment step runs once per iteration over the whole histogram —
    # up to 262 144 bins for a 6-bit histogram — so it is done in numpy. The
    # points and weights are lifted out of the dictionaries once, in histogram
    # order, which is the order the tie-break depends on.
    points = np.asarray([item["oklab"] for item in histogram], dtype=np.float64)
    weights = np.asarray([item["weight"] for item in histogram], dtype=np.float64)
    weighted_points = points * weights[:, None]

    def assign_clusters(
        current_centers: list[tuple[float, float, float]],
    ) -> tuple[np.ndarray, np.ndarray]:
        """
        Returns (assignment, cluster_weights).

        `assignment[i]` is the index of the centre that bin `i` belongs to.
        `np.argmin` returns the first minimum, which is the same explicit
        tie-break towards the lowest centre index the scalar form used.
        """
        centers = np.asarray(current_centers, dtype=np.float64)
        distances = ((points[:, None, :] - centers[None, :, :]) ** 2).sum(axis=2)
        assignment = np.argmin(distances, axis=1)
        cluster_weights = np.bincount(
            assignment, weights=weights, minlength=len(current_centers)
        )
        return assignment, cluster_weights

    # K-Means iterations.
    for _ in range(max(0, max_iterations)):
        assignment, cluster_weights = assign_clusters(centers)

        # Weighted sum per cluster, all three channels at once.
        sums = np.zeros((len(centers), 3), dtype=np.float64)
        np.add.at(sums, assignment, weighted_points)

        max_shift = 0.0
        new_auto_centers: list[tuple[float, float, float]] = []

        for auto_index, old_center in enumerate(auto_centers):
            center_index = fixed_count + auto_index
            total_weight = cluster_weights[center_index]

            if total_weight > 0:
                proposed_center = tuple(sums[center_index] / total_weight)
            else:
                proposed_center = old_center

            # Compare against:
            # - fixed centres;
            # - already accepted automatic centres;
            # - automatic centres not yet updated this iteration.
            #
            # This preserves the separation established at initialization.
            other_centers = [
                *fixed_centers,
                *new_auto_centers,
                *auto_centers[auto_index + 1 :],
            ]

            violates_separation = any(
                oklab_distance_sq(proposed_center, other_center)
                < min_separation_sq
                for other_center in other_centers
            )

            new_center = (
                old_center
                if violates_separation
                else proposed_center
            )

            shift = math.sqrt(
                oklab_distance_sq(old_center, new_center)
            )
            max_shift = max(max_shift, shift)

            new_auto_centers.append(new_center)

        auto_centers = new_auto_centers
        centers[fixed_count:] = auto_centers

        if max_shift < convergence_threshold:
            break

    # The previous assignments were calculated before the final centre update.
    # Reassign once so the returned cluster counts match the final centres.
    _, final_cluster_weights = assign_clusters(centers)

    result_entries = []

    for auto_index, center in enumerate(auto_centers):
        center_index = fixed_count + auto_index

        red, green, blue = oklab_to_srgb(*center)

        result_entries.append(
            {
                "hex": srgb_to_hex(red, green, blue),
                "oklab": tuple(float(value) for value in center),
                "count": int(final_cluster_weights[center_index]),
            }
        )

    return result_entries
