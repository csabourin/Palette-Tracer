"""
Destination presets loader and policy definitions (SPEC §19).

`get_destination_preset` alone does not change anything — a caller must apply
the returned preset onto a settings document. `apply_destination_preset` is
that caller, wired into `server/api.py` for both the destination dropdown and
the "Reset to destination defaults" control (§9.2).
"""

import copy
import json
from pathlib import Path

from palette_trace.presets.profiles import DEFAULT_PROFILE_ID, get_builtin_profile

_DATA_DIR = Path(__file__).parent.parent / "data"
_DEST_FILE = _DATA_DIR / "destination_presets_v1.json"

_DEST_PRESETS: dict | None = None

_FALLBACK_ID = "illustration"


def _load() -> dict:
    global _DEST_PRESETS
    if _DEST_PRESETS is None:
        with open(_DEST_FILE, "r", encoding="utf-8") as f:
            _DEST_PRESETS = json.load(f)["destinations"]
    return _DEST_PRESETS


def list_destination_ids() -> list[str]:
    """Every built-in destination id, in data-file order."""
    return list(_load())


def resolve_destination_id(dest_id: str) -> str:
    """An unknown destination id resolves to the fallback rather than failing."""
    presets = _load()
    return dest_id if dest_id in presets else _FALLBACK_ID


def get_destination_preset(dest_id: str) -> dict:
    """Returns a deep copy of the destination configuration dict for dest_id."""
    presets = _load()
    resolved = resolve_destination_id(dest_id)
    return copy.deepcopy(presets[resolved])


def apply_destination_preset(settings: dict, dest_id: str) -> dict:
    """
    Applies a destination preset's technical defaults onto `settings` in place
    (§19), used by both the destination dropdown and "Reset to destination
    defaults" (§9.2).

    This overwrites geometry, the global trace profile and
    `output.hideSourceImageAfterApply` — the fields a destination actually
    governs. It deliberately leaves `scanCount`, palette entries and picked
    colours untouched: switching destinations must not discard a user's
    palette choices, and a picked colour's output stays exact regardless of
    destination (§34.5).
    """
    resolved_id = resolve_destination_id(dest_id)
    preset = get_destination_preset(resolved_id)

    settings["destination"] = {
        "id": resolved_id,
        "presetVersion": preset.get("presetVersion", 1),
    }
    settings["geometry"] = copy.deepcopy(preset["geometry"])
    settings["globalTraceProfile"] = get_builtin_profile(
        preset.get("globalTraceProfileId", DEFAULT_PROFILE_ID)
    )
    settings.setdefault("output", {})["hideSourceImageAfterApply"] = preset.get(
        "hideSourceImageAfterApply", False
    )
    return settings
