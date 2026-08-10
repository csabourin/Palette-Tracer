"""
2D Integer Label Map for memory-efficient pixel classification.
"""

import numpy as np


class LabelMap:
    """
    Stores classified scan labels as a single 2D integer array (uint8 or uint16).
    """

    def __init__(self, height: int, width: int, entry_ids: list[str]):
        self.height = height
        self.width = width
        self.entry_ids = list(entry_ids)
        self.id_to_label = {eid: idx + 1 for idx, eid in enumerate(self.entry_ids)}
        self.label_to_id = {idx + 1: eid for idx, eid in enumerate(self.entry_ids)}

        dtype = np.uint16 if len(entry_ids) > 250 else np.uint8
        self.data = np.zeros((height, width), dtype=dtype)

    def set_claims_from_indices(self, winners: np.ndarray, entry_ids: list[str]):
        """
        Populate the label map from an index-per-pixel claim array.

        `winners` holds the position in `entry_ids` that won each pixel, or -1
        where nothing did. Indices rather than id strings keep this off an
        object array the size of the picture.
        """
        for index, entry_id in enumerate(entry_ids):
            label = self.id_to_label.get(entry_id, 0)
            if label:
                self.data[winners == index] = label

    def set_label_for_mask(self, mask: np.ndarray, entry_id: str):
        """Set label ID for pixels where mask is True."""
        lbl = self.id_to_label.get(entry_id, 0)
        self.data[mask] = lbl

    def get_binary_mask(self, entry_id: str) -> np.ndarray:
        """Returns boolean 2D array where label matches entry_id."""
        lbl = self.id_to_label.get(entry_id, 0)
        if lbl == 0:
            return np.zeros((self.height, self.width), dtype=bool)
        return self.data == lbl
