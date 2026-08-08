"""Deterministic constrained K-Means clustering in OKLab coordinates."""

import math

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
    candidate_pool = list(sorted_histogram)

    # Farthest-point initialization for automatic centres.
    while len(centers) < fixed_count + actual_target and candidate_pool:
        if not centers:
            selected_index = 0
        else:
            selected_index = None
            best_weighted_distance = -1.0
            best_packed_srgb = float("inf")

            for index, item in enumerate(candidate_pool):
                point = item["oklab"]

                minimum_distance = min(
                    oklab_distance_sq(point, center)
                    for center in centers
                )

                # Do not initialize a centre too close to an existing centre.
                if minimum_distance < min_separation_sq:
                    continue

                weighted_distance = minimum_distance * item["weight"]

                is_better_distance = (
                    weighted_distance > best_weighted_distance
                )
                is_deterministic_tie_break = (
                    math.isclose(
                        weighted_distance,
                        best_weighted_distance,
                        abs_tol=1e-9,
                    )
                    and item["packed_srgb"] < best_packed_srgb
                )

                if is_better_distance or is_deterministic_tie_break:
                    selected_index = index
                    best_weighted_distance = weighted_distance
                    best_packed_srgb = item["packed_srgb"]

            if selected_index is None:
                # No remaining candidate satisfies the minimum separation.
                break

        # Removing the selected candidate prevents duplicate initialization.
        selected_item = candidate_pool.pop(selected_index)
        centers.append(selected_item["oklab"])

    auto_centers = centers[fixed_count:]

    if not auto_centers:
        return []

    def assign_clusters(
        current_centers: list[tuple[float, float, float]],
    ) -> tuple[list[list[dict]], list[int | float]]:
        clusters: list[list[dict]] = [
            [] for _ in current_centers
        ]
        cluster_weights: list[int | float] = [
            0 for _ in current_centers
        ]

        for item in histogram:
            point = item["oklab"]
            weight = item["weight"]

            # Including the index in the key provides an explicit,
            # deterministic tie-break.
            best_index = min(
                range(len(current_centers)),
                key=lambda index: (
                    oklab_distance_sq(
                        point,
                        current_centers[index],
                    ),
                    index,
                ),
            )

            clusters[best_index].append(item)
            cluster_weights[best_index] += weight

        return clusters, cluster_weights

    # K-Means iterations.
    for _ in range(max(0, max_iterations)):
        clusters, cluster_weights = assign_clusters(centers)

        max_shift = 0.0
        new_auto_centers: list[tuple[float, float, float]] = []

        for auto_index, old_center in enumerate(auto_centers):
            center_index = fixed_count + auto_index
            items = clusters[center_index]
            total_weight = cluster_weights[center_index]

            if total_weight > 0 and items:
                proposed_center = (
                    sum(
                        item["oklab"][0] * item["weight"]
                        for item in items
                    )
                    / total_weight,
                    sum(
                        item["oklab"][1] * item["weight"]
                        for item in items
                    )
                    / total_weight,
                    sum(
                        item["oklab"][2] * item["weight"]
                        for item in items
                    )
                    / total_weight,
                )
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
                "oklab": center,
                "count": final_cluster_weights[center_index],
            }
        )

    return result_entries