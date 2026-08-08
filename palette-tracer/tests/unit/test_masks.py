"""
Unit tests for Masks Representation & Cleanup (label_map, components, morphology, geometry).
"""

import pytest
import numpy as np
from palette_trace.masks.label_map import LabelMap
from palette_trace.masks.components import remove_small_speckles, fill_small_holes
from palette_trace.masks.morphology import dilate_mask, erode_mask
from palette_trace.masks.geometry_policy import apply_underlap_to_mask

def test_label_map():
    lm = LabelMap(10, 10, ["e1", "e2"])
    mask_e1 = np.zeros((10, 10), dtype=bool)
    mask_e1[2:5, 2:5] = True
    lm.set_label_for_mask(mask_e1, "e1")

    res = lm.get_binary_mask("e1")
    assert np.array_equal(res, mask_e1)

def test_speckle_removal():
    mask = np.zeros((10, 10), dtype=bool)
    mask[1, 1] = True  # 1 px speckle
    mask[4:8, 4:8] = True  # 16 px region

    cleaned = remove_small_speckles(mask, min_area_px2=4)
    assert cleaned[1, 1] == False
    assert cleaned[5, 5] == True

def test_hole_filling():
    mask = np.zeros((10, 10), dtype=bool)
    mask[1:9, 1:9] = True
    mask[4, 4] = False  # 1 px hole

    filled = fill_small_holes(mask, max_hole_area_px2=4)
    assert filled[4, 4] == True

def test_morphology():
    mask = np.zeros((10, 10), dtype=bool)
    mask[4:6, 4:6] = True

    dilated = dilate_mask(mask, 1.0)
    assert dilated[3, 4] == True

    eroded = erode_mask(dilated, 1.0)
    assert np.array_equal(eroded, mask)
