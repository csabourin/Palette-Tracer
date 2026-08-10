"""
Reach tolerance mapping and neutral color guardrail calculations.
"""

import json
from pathlib import Path

_DATA_DIR = Path(__file__).parent.parent / "data"
_REACH_FILE = _DATA_DIR / "reach_mapping_v1.json"

_REACH_DATA = None

def _load_reach_data():
    global _REACH_DATA
    if _REACH_DATA is None:
        with open(_REACH_FILE, encoding="utf-8") as f:
            _REACH_DATA = json.load(f)["anchors"]
    return _REACH_DATA

def get_tolerances_for_reach(overall_reach: int) -> dict:
    """
    Interpolate linked channel tolerances for an integer overallReach in [0, 100].
    Returns dict with keys: hue (deg), chroma, lightness.
    """
    anchors = _load_reach_data()
    reach = max(0, min(100, int(overall_reach)))

    # Find bounding anchors
    for i in range(len(anchors) - 1):
        a1 = anchors[i]
        a2 = anchors[i + 1]
        if a1["reach"] <= reach <= a2["reach"]:
            span = float(a2["reach"] - a1["reach"])
            t = (reach - a1["reach"]) / span if span else 0.0
            return {
                "hue": a1["hue"] + t * (a2["hue"] - a1["hue"]),
                "chroma": a1["chroma"] + t * (a2["chroma"] - a1["chroma"]),
                "lightness": a1["lightness"] + t * (a2["lightness"] - a1["lightness"]),
            }
    # Fallback for 100
    last = anchors[-1]
    return {"hue": last["hue"], "chroma": last["chroma"], "lightness": last["lightness"]}

def calculate_hue_confidence(chroma: float) -> float:
    """
    Computes hue confidence multiplier: clamp(chroma / 0.05, 0.0, 1.0).
    """
    if chroma <= 0.0:
        return 0.0
    return max(0.0, min(1.0, chroma / 0.05))
