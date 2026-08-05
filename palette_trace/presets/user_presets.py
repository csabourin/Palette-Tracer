"""
User preset persistence manager (JSON file storage in user config directory).
"""

import json
import os
import uuid
from pathlib import Path
from typing import List, Dict, Optional

def get_user_presets_dir() -> Path:
    """Returns directory path for user-saved presets."""
    home = Path.home()
    preset_dir = home / ".config" / "palette-trace" / "presets"
    preset_dir.mkdir(parents=True, exist_ok=True)
    return preset_dir

def list_user_presets() -> list[dict]:
    """Lists saved user presets."""
    p_dir = get_user_presets_dir()
    presets = []
    for fpath in p_dir.glob("*.json"):
        try:
            with open(fpath, "r", encoding="utf-8") as f:
                data = json.load(f)
                if data.get("schemaVersion") == 1:
                    presets.append(data)
        except Exception:
            pass
    return presets

def save_user_preset(name: str, description: str, patch_data: dict, scope: str = "full") -> dict:
    """Saves a new user preset file."""
    p_dir = get_user_presets_dir()
    puuid = str(uuid.uuid4())
    preset_obj = {
        "schemaVersion": 1,
        "presetUuid": puuid,
        "name": name,
        "description": description,
        "createdAt": "2026-08-05T12:00:00Z",
        "updatedAt": "2026-08-05T12:00:00Z",
        "scope": scope,
        "includes": {
            "destination": True,
            "scanCount": True,
            "geometry": True,
            "globalTraceProfile": True,
            "perScanProfiles": True,
            "paletteRoles": True,
            "matchingSettings": True,
            "exactPaletteColors": True,
            "outputSettings": True,
        },
        "configurationPatch": patch_data,
    }
    file_path = p_dir / f"{puuid}.json"
    with open(file_path, "w", encoding="utf-8") as f:
        json.dump(preset_obj, f, indent=2)
    return preset_obj
