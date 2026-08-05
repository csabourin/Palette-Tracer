"""
Destination presets loader and policy definitions.
"""

import json
from pathlib import Path

_DATA_DIR = Path(__file__).parent.parent / "data"
_DEST_FILE = _DATA_DIR / "destination_presets_v1.json"

_DEST_PRESETS = None

def get_destination_preset(dest_id: str) -> dict:
    """Returns destination configuration dict for dest_id."""
    global _DEST_PRESETS
    if _DEST_PRESETS is None:
        with open(_DEST_FILE, "r", encoding="utf-8") as f:
            _DEST_PRESETS = json.load(f)["destinations"]
    return _DEST_PRESETS.get(dest_id, _DEST_PRESETS["illustration"])
