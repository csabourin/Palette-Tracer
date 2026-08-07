"""
Saved user preset persistence (SPEC §26).

Presets are JSON files in a per-user config directory. A preset stores a
`configurationPatch` scoped by `includes` (§26.1-26.3) rather than a full
settings document, so it can be applied to a different image (§8.3) without
carrying over that image's identity, generation state or (for a structure
preset) its exact colours.
"""

import copy
import json
import uuid
from datetime import datetime, timezone
from pathlib import Path

from palette_trace.settings import create_palette_entry

_INCLUDES_BY_SCOPE = {
    "structure": {
        "destination": True, "scanCount": True, "geometry": True,
        "globalTraceProfile": True, "perScanProfiles": True, "paletteRoles": True,
        "matchingSettings": False, "exactPaletteColors": False, "outputSettings": True,
    },
    "palette": {
        "destination": False, "scanCount": False, "geometry": False,
        "globalTraceProfile": False, "perScanProfiles": True, "paletteRoles": True,
        "matchingSettings": True, "exactPaletteColors": True, "outputSettings": False,
    },
    "full": {
        "destination": True, "scanCount": True, "geometry": True,
        "globalTraceProfile": True, "perScanProfiles": True, "paletteRoles": True,
        "matchingSettings": True, "exactPaletteColors": True, "outputSettings": True,
    },
}


def get_user_presets_dir() -> Path:
    """Returns directory path for user-saved presets."""
    home = Path.home()
    preset_dir = home / ".config" / "palette-trace" / "presets"
    preset_dir.mkdir(parents=True, exist_ok=True)
    return preset_dir


def list_user_presets() -> list[dict]:
    """Lists saved user presets, newest first."""
    p_dir = get_user_presets_dir()
    presets = []
    for fpath in p_dir.glob("*.json"):
        try:
            with open(fpath, "r", encoding="utf-8") as f:
                data = json.load(f)
                if data.get("schemaVersion") == 1:
                    presets.append(data)
        except (OSError, ValueError):
            pass
    presets.sort(key=lambda p: p.get("updatedAt", ""), reverse=True)
    return presets


def load_user_preset(preset_uuid: str) -> dict | None:
    """Loads one saved preset by id, or None if it does not exist or is unreadable."""
    file_path = get_user_presets_dir() / f"{preset_uuid}.json"
    try:
        with open(file_path, "r", encoding="utf-8") as f:
            data = json.load(f)
    except (OSError, ValueError):
        return None
    return data if data.get("schemaVersion") == 1 else None


def delete_user_preset(preset_uuid: str) -> bool:
    """Deletes a saved preset file. Returns whether a file was actually removed."""
    file_path = get_user_presets_dir() / f"{preset_uuid}.json"
    try:
        file_path.unlink()
        return True
    except FileNotFoundError:
        return False


def _structural_entry(entry: dict) -> dict:
    """A structure-scope entry: role and trace profile pattern, no colour (§26.1)."""
    return {
        "enabled": entry.get("enabled", True),
        "role": entry.get("role"),
        "traceProfile": entry.get("traceProfile"),
    }


def _palette_entry(entry: dict) -> dict:
    """A palette-scope entry: exact colours, anchors, reach and role (§26.2)."""
    return {
        "kind": entry.get("kind"),
        "sourceAnchor": entry.get("sourceAnchor"),
        "output": entry.get("output"),
        "assignment": entry.get("assignment"),
        "role": entry.get("role"),
        "traceProfile": entry.get("traceProfile"),
    }


def build_configuration_patch(settings: dict, scope: str) -> dict:
    """
    Builds a `configurationPatch` from the live settings document for the
    given scope (§26.1-26.3). Source-specific identity (`imageUuid`,
    `source`) and `generation` state are never included — a preset must be
    reusable against a different image (§8.3).
    """
    palette = settings.get("palette", {})
    entries = palette.get("entries", [])
    patch: dict = {}

    if scope in ("structure", "full"):
        patch["destination"] = copy.deepcopy(settings.get("destination"))
        patch["scanCount"] = settings.get("scanCount")
        patch["geometry"] = copy.deepcopy(settings.get("geometry"))
        patch["globalTraceProfile"] = copy.deepcopy(settings.get("globalTraceProfile"))
        patch["output"] = copy.deepcopy(settings.get("output"))

    if scope in ("palette", "full"):
        patch["entries"] = [_palette_entry(e) for e in entries]
        patch["backgroundMatching"] = palette.get("backgroundMatching")
        patch["colorProcessing"] = copy.deepcopy(settings.get("colorProcessing"))
        patch["alpha"] = copy.deepcopy(settings.get("alpha"))
    elif scope == "structure":
        patch["entryStructure"] = [_structural_entry(e) for e in entries]

    return patch


def apply_configuration_patch(settings: dict, patch: dict) -> dict:
    """
    Merges a saved preset's `configurationPatch` onto `settings` in place
    (§8.3). Palette entry ids are always regenerated: a preset may be applied
    to an image with a different scan count, and reusing ids from the
    document the preset was saved from would collide with this image's own
    entries.
    """
    for key in ("destination", "geometry", "globalTraceProfile", "output", "colorProcessing", "alpha"):
        if patch.get(key) is not None:
            settings[key] = copy.deepcopy(patch[key])

    if patch.get("scanCount") is not None:
        settings["scanCount"] = patch["scanCount"]

    palette = settings.setdefault("palette", {})
    if patch.get("backgroundMatching") is not None:
        palette["backgroundMatching"] = patch["backgroundMatching"]

    if patch.get("entries") is not None:
        new_entries = []
        for i, saved in enumerate(patch["entries"]):
            entry = create_palette_entry(i)
            entry["kind"] = saved.get("kind", entry["kind"])
            entry["sourceAnchor"] = saved.get("sourceAnchor")
            entry["output"] = saved.get("output", entry["output"])
            entry["assignment"] = saved.get("assignment", entry["assignment"])
            entry["role"] = saved.get("role", entry["role"])
            entry["traceProfile"] = saved.get("traceProfile", entry["traceProfile"])
            new_entries.append(entry)
        palette["entries"] = new_entries
        palette["layerOrder"] = [e["id"] for e in new_entries]
        palette["backgroundEntryId"] = None
        settings["scanCount"] = len(new_entries)
    elif patch.get("entryStructure") is not None:
        new_entries = []
        for i, saved in enumerate(patch["entryStructure"]):
            entry = create_palette_entry(i)
            entry["enabled"] = saved.get("enabled", True)
            entry["role"] = saved.get("role", entry["role"])
            entry["traceProfile"] = saved.get("traceProfile", entry["traceProfile"])
            new_entries.append(entry)
        palette["entries"] = new_entries
        palette["layerOrder"] = [e["id"] for e in new_entries]
        palette["backgroundEntryId"] = None
        settings["scanCount"] = len(new_entries)

    return settings


def save_user_preset(name: str, description: str, settings: dict, scope: str = "full") -> dict:
    """Builds a configuration patch from `settings` and saves it as a new preset file."""
    p_dir = get_user_presets_dir()
    puuid = str(uuid.uuid4())
    now = datetime.now(timezone.utc).isoformat()
    preset_obj = {
        "schemaVersion": 1,
        "presetUuid": puuid,
        "name": name,
        "description": description,
        "createdAt": now,
        "updatedAt": now,
        "scope": scope,
        "includes": _INCLUDES_BY_SCOPE.get(scope, _INCLUDES_BY_SCOPE["full"]),
        "configurationPatch": build_configuration_patch(settings, scope),
    }
    file_path = p_dir / f"{puuid}.json"
    with open(file_path, "w", encoding="utf-8") as f:
        json.dump(preset_obj, f, indent=2)
    return preset_obj
