"""
Background classification (SPEC §16).

Background is a *role*, not a separate palette type (§16.3): any entry, pinned or
automatic, may be designated as background. What differs is how its pixels are
selected (§16.1) and how they are emitted (§16.2).
"""

import numpy as np

from palette_trace.masks.components import label_connected_components

#: Matching modes from §16.1.
ALL_MATCHING = "all_matching"
EDGE_CONNECTED = "edge_connected"
TRANSPARENT = "transparent"

#: Output modes from §16.2.
KEEP_PATHS = "keep_paths"
OMIT = "omit"
REPLACE_WITH_RECTANGLE = "replace_with_rectangle"


def extract_edge_connected_mask(matching_mask: np.ndarray) -> np.ndarray:
    """
    Returns only the components of `matching_mask` that touch the image boundary.

    Enclosed regions of the same colour are preserved as non-background (§16.1),
    so a white eye inside a character does not become part of a white backdrop.

    A 4-connected labelling answers this in one pass: a component is backdrop
    exactly when one of its pixels lies on the frame.
    """
    height, width = matching_mask.shape
    if height == 0 or width == 0:
        return np.zeros((height, width), dtype=bool)

    matching = np.asarray(matching_mask, dtype=bool)
    if not matching.any():
        return np.zeros((height, width), dtype=bool)

    labels, sizes = label_connected_components(matching)
    on_border = np.zeros(len(sizes), dtype=bool)
    for border in (labels[0, :], labels[-1, :], labels[:, 0], labels[:, -1]):
        on_border[np.unique(border)] = True
    on_border[0] = False

    return on_border[labels]


def classify_background(
    matching_mask: np.ndarray,
    alpha: np.ndarray,
    mode: str = ALL_MATCHING,
    alpha_threshold: float = 0.05,
) -> np.ndarray:
    """
    Applies a §16.1 matching mode to a candidate background mask.

    `matching_mask` is the set of pixels the background entry would otherwise
    claim. `alpha` is the source alpha channel in 0..1.
    """
    if mode == TRANSPARENT:
        return alpha < alpha_threshold

    matching = matching_mask.astype(bool)
    if mode == EDGE_CONNECTED:
        return extract_edge_connected_mask(matching)

    return matching


#: What to do with a patch of the backdrop colour that the matching mode has
#: ruled out of the backdrop — the white of an eye when the backdrop is white,
#: say. `hole_or_layer` is the default: keep it as a hole wherever a hole can
#: express it, and give it a layer of its own where one cannot.
ISLAND_HOLE_OR_LAYER = "hole_or_layer"
ISLAND_HOLE = "hole"
ISLAND_LAYER = "layer"
ISLAND_IGNORE = "ignore"
ISLAND_POLICIES = (ISLAND_HOLE_OR_LAYER, ISLAND_HOLE, ISLAND_LAYER, ISLAND_IGNORE)


class BackgroundIslands:
    """
    The verdict for every patch of the backdrop colour sitting in the foreground.

    * ``holes`` maps a label-map label to the mask to subtract from that entry's
      geometry. The patch is fully surrounded by that one colour, so a hole in
      it says exactly what the picture says.
    * ``layer_mask`` is everything a hole cannot express: a patch that touches
      the frame, or one that straddles two colours, would leave a bite out of
      whichever shape it was subtracted from. Those get a layer of their own,
      drawn above, in the backdrop colour.
    """

    def __init__(self, holes: dict, layer_mask: np.ndarray, hole_count: int, layer_count: int):
        self.holes = holes
        self.layer_mask = layer_mask
        self.hole_count = hole_count
        self.layer_count = layer_count

    @property
    def total(self) -> int:
        return self.hole_count + self.layer_count

    def summary(self) -> str:
        """One sentence for the interface, or an empty string when nothing happened."""
        if not self.total:
            return ""

        if self.total == 1:
            subject = "One patch of the backdrop colour sits in the foreground"
            outcome = "it became a hole" if self.hole_count else "it got a layer of its own"
            return f"{subject} — {outcome}."

        subject = f"{self.total} patches of the backdrop colour sit in the foreground"
        if self.hole_count and self.layer_count:
            holes = "1 became a hole" if self.hole_count == 1 else f"{self.hole_count} became holes"
            layers = (
                "1 got a layer of its own" if self.layer_count == 1
                else f"{self.layer_count} got layers of their own"
            )
            return f"{subject} — {holes}, {layers}."
        if self.hole_count:
            return f"{subject} — they became holes."
        return f"{subject} — they got layers of their own."


def classify_background_islands(
    island_mask: np.ndarray,
    entry_labels: np.ndarray,
    policy: str = ISLAND_HOLE_OR_LAYER,
) -> BackgroundIslands:
    """
    Decides, per patch, whether it can be a hole or needs a layer of its own.

    `island_mask` is the set of pixels that matched the backdrop colour but were
    excluded from the backdrop by the §16.1 matching mode. `entry_labels` is the
    label map for every other scan; islands themselves must already be zeroed
    there, so a patch never counts itself as its own neighbour.

    A patch qualifies as a hole when every pixel around it belongs to one and
    the same scan and it does not run off the edge of the frame. Anything else —
    a patch bridging two colours, one that leaks into unassigned pixels, one
    reaching the border — cannot be subtracted from a single shape without
    lying about what is underneath, so it becomes a layer.
    """
    empty = np.zeros(island_mask.shape, dtype=bool)
    if policy == ISLAND_IGNORE or not np.any(island_mask):
        return BackgroundIslands({}, empty, 0, 0)

    components, sizes = label_connected_components(island_mask)
    count = len(sizes) - 1
    if count <= 0:
        return BackgroundIslands({}, empty, 0, 0)

    if policy == ISLAND_LAYER:
        return BackgroundIslands({}, island_mask.copy(), 0, count)

    stride = int(entry_labels.max()) + 1

    # Which scan sits on the far side of each island pixel, in all four
    # directions. Gathered as (component, neighbouring scan) pairs so the whole
    # question is answered by one `np.unique` rather than a walk per patch.
    keys = []
    slices = (
        ((slice(1, None), slice(None)), (slice(None, -1), slice(None))),
        ((slice(None, -1), slice(None)), (slice(1, None), slice(None))),
        ((slice(None), slice(1, None)), (slice(None), slice(None, -1))),
        ((slice(None), slice(None, -1)), (slice(None), slice(1, None))),
    )
    for here, there in slices:
        source = components[here]
        neighbour = entry_labels[there]
        selected = (source > 0) & ~island_mask[there]
        if not np.any(selected):
            continue
        keys.append(source[selected].astype(np.int64) * stride + neighbour[selected])

    neighbours_of = {}
    if keys:
        unique_pairs = np.unique(np.concatenate(keys))
        for component_id, scan_label in zip(unique_pairs // stride, unique_pairs % stride):
            neighbours_of.setdefault(int(component_id), set()).add(int(scan_label))

    on_border = np.zeros(count + 1, dtype=bool)
    for border in (components[0, :], components[-1, :], components[:, 0], components[:, -1]):
        on_border[np.unique(border)] = True

    owner_of = np.zeros(count + 1, dtype=np.int64)
    becomes_layer = np.zeros(count + 1, dtype=bool)

    for component_id in range(1, count + 1):
        surrounding = neighbours_of.get(component_id, set())
        enclosed = (
            not on_border[component_id]
            and len(surrounding) == 1
            and 0 not in surrounding
        )
        if enclosed:
            owner_of[component_id] = next(iter(surrounding))
        elif policy == ISLAND_HOLE_OR_LAYER:
            becomes_layer[component_id] = True

    owners = owner_of[components]
    holes = {
        int(scan_label): owners == scan_label
        for scan_label in np.unique(owner_of)
        if scan_label > 0
    }
    layer_mask = becomes_layer[components]

    return BackgroundIslands(
        holes,
        layer_mask,
        int(np.count_nonzero(owner_of[1:])),
        int(np.count_nonzero(becomes_layer[1:])),
    )


def background_rectangle_path(width: int, height: int) -> str:
    """
    SVG path data covering the complete intrinsic source bounds (§16.2).

    Rectangle replacement uses the full image bounds rather than the traced
    background extent, and is placed with the same source-to-document transform
    as every other generated path.
    """
    return f"M 0 0 L {width} 0 L {width} {height} L 0 {height} Z"
