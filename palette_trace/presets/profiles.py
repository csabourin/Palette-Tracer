"""
Trace-profile resolution (SPEC §18).

Profiles live in a versioned data file. A palette entry either inherits the
document's global profile or overrides it — by naming a built-in profile, or by
supplying explicit values that are merged over the inherited profile.

Without this module the per-entry `traceProfile` field is written to settings and
never read, which is what MVP criteria 13 and 14 require.
"""

import copy
import json
from pathlib import Path

_DATA_FILE = Path(__file__).parent.parent / "data" / "trace_profiles_v1.json"

_PROFILES: dict | None = None

#: The profile applied when nothing else resolves.
DEFAULT_PROFILE_ID = "default"


def _load() -> dict:
    global _PROFILES
    if _PROFILES is None:
        with open(_DATA_FILE, encoding="utf-8") as handle:
            _PROFILES = json.load(handle)["profiles"]
    return _PROFILES


def list_profile_ids() -> list[str]:
    """Every built-in profile id, in data-file order."""
    return list(_load())


def get_builtin_profile(profile_id: str) -> dict:
    """Returns a deep copy of a built-in profile, falling back to the default."""
    profiles = _load()
    return copy.deepcopy(profiles.get(profile_id, profiles[DEFAULT_PROFILE_ID]))


def merge_profiles(base: dict, override: dict) -> dict:
    """
    Merges `override` over `base` one section at a time.

    Only the `mask` and `vector` sections are merged, and only keys present in
    the override are replaced — a partial override must not blank out the
    settings it does not mention.
    """
    merged = copy.deepcopy(base) if base else {}
    for section in ("mask", "vector"):
        if section not in override:
            continue
        values = override.get(section) or {}
        if not isinstance(values, dict):
            continue
        merged.setdefault(section, {}).update(values)
    return merged


def resolve_entry_profile(entry: dict, global_profile: dict) -> dict:
    """
    Resolves the effective trace profile for one palette entry (§18).

    Supported `traceProfile.mode` values:

    * ``inherit`` — use the document's global profile unchanged.
    * ``preset`` — use the built-in profile named by ``profileId``.
    * ``override`` — merge ``values`` over the inherited or named profile.

    An unrecognised mode inherits, so that a future settings version widening
    this field degrades rather than fails.
    """
    base = (
        copy.deepcopy(global_profile) if global_profile
        else get_builtin_profile(DEFAULT_PROFILE_ID)
    )
    config = entry.get("traceProfile") or {}
    mode = config.get("mode", "inherit")

    if mode == "preset":
        return get_builtin_profile(config.get("profileId", DEFAULT_PROFILE_ID))

    if mode == "override":
        # An override may also re-base onto a named preset before merging.
        profile_id = config.get("profileId")
        if profile_id:
            base = get_builtin_profile(profile_id)
        return merge_profiles(base, config.get("values") or {})

    return base


def unsupported_settings(profile: dict, supported: frozenset) -> list[str]:
    """
    Names the vector settings a backend does not claim to support (§18).

    Unsupported settings must be reported rather than silently ignored, so the
    pipeline surfaces these as warnings on the scan result.
    """
    requested = set((profile or {}).get("vector", {}))
    # Structural flags are pipeline-level concerns, not engine parameters.
    requested -= {"requireClosedPaths"}
    return sorted(requested - set(supported))
