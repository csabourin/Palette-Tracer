"""
Host-neutral image settings (SPEC §11).

This module must not import `inkex`. Both hosts depend on it: the Inkscape
extension stores these settings as a `pt:settings` attribute on the source
`<image>` (§10.2), and the standalone application stores them in a sidecar file
(§9.4.3). The schema is identical either way.
"""

import hashlib
import json
import uuid

from palette_trace import __version__

#: Custom XML namespace (§10.1). An identifier, not a network endpoint.
PT_NAMESPACE = "https://christiansabourin.com/ns/palette-trace/1"
PT_SCHEMA_VERSION = 1

#: The version stamped into settings provenance (§10.3) and the diagnostic
#: report (§30). One name for it, so a release cannot half-happen.
EXTENSION_VERSION = __version__

#: The background matching (§16.1), background output (§16.2) and geometry
#: policy (§20) vocabularies are *not* repeated here. Each one is named by the
#: module that acts on it — `color.background` and `masks.geometry_policy` —
#: and constrained by `schemas/image-settings-v1.schema.json`. A third copy of
#: the same literals would only give them somewhere to drift apart.

DEFAULT_SCAN_COUNT = 4


def create_palette_entry(index: int, entry_id: str | None = None) -> dict:
    """Builds one default automatic palette entry."""
    return {
        "id": entry_id or str(uuid.uuid4()),
        # "Scan" is this specification's term for the concept (§6.2), but it is
        # not a word a user brings with them. The default *name* is the one
        # they see in the interface and in the output's layer labels, so it
        # says what the thing is rather than what the model calls it.
        "name": f"Colour {index + 1}",
        "enabled": True,
        "kind": "automatic",
        "sourceAnchor": None,
        "output": {
            "mode": "automatic_centroid",
            "color": None,
        },
        "assignment": {
            "mode": "automatic",
            "overallReach": 25,
            "channels": {
                "mode": "linked",
                "hue": {"enabled": True, "tolerance": 10.0, "weight": 1.0},
                "chroma": {"enabled": True, "tolerance": 0.035, "weight": 1.0},
                "lightness": {"enabled": True, "tolerance": 0.06, "weight": 1.0},
            },
        },
        "role": "primary_fill",
        "traceProfile": {"mode": "inherit"},
    }


def create_default_settings(image_uuid: str) -> dict:
    """Creates a default PaletteTraceImageSettingsV1 configuration."""
    entries = [create_palette_entry(i) for i in range(DEFAULT_SCAN_COUNT)]

    return {
        "schemaVersion": PT_SCHEMA_VERSION,
        "extensionVersion": EXTENSION_VERSION,
        "imageUuid": image_uuid,
        "source": {
            "sourceMode": "intrinsic",
            "fingerprint": {"algorithm": "sha256", "value": ""},
            "intrinsicWidth": 0,
            "intrinsicHeight": 0,
            "mimeType": "image/png",
            "traceClipEffects": False,
            "traceSvgFilters": False,
        },
        "destination": {"id": "illustration", "presetVersion": 1},
        "scanCount": DEFAULT_SCAN_COUNT,
        "colorProcessing": {
            "inputSpace": "srgb",
            "comparisonSpace": "oklch",
            "quantizer": "deterministic_constrained_kmeans",
            "automaticPalette": {
                "histogramBitsPerChannel": 6,
                "maxHistogramEntries": 32768,
                "maxIterations": 50,
                "convergenceThreshold": 1e-5,
                "minimumSeparation": 0.02,
            },
            "unmatchedPixelPolicy": "nearest_available",
            "reachMappingVersion": 1,
            "lowChromaHuePolicyVersion": 1,
        },
        "palette": {
            "entries": entries,
            "layerOrder": [e["id"] for e in entries],
            "backgroundEntryId": None,
            "backgroundMatching": "all_matching",
            # §16.1: what becomes of a patch of the backdrop colour that the
            # matching mode leaves in the foreground. The default keeps it as a
            # hole where a hole says the right thing, and gives it a layer of
            # its own where one would not.
            "backgroundForegroundPolicy": "hole_or_layer",
        },
        "alpha": {
            "fullyTransparentPolicy": "ignore",
            "partialAlphaPolicy": "composite",
            "ignoreBelow": 0.05,
            "matte": {"mode": "white"},
            "outputOpacity": "opaque",
        },
        "globalTraceProfile": {
            "mask": {
                "minimumRegionAreaPx2": 4,
                "fillHolesAreaPx2": 4,
                "closeGapsRadiusPx": 1,
                "smoothingRadiusPx": 0.5,
                "preserveThinFeatures": True,
                "minimumFeatureWidthPx": 1,
                "offsetPx": 0,
            },
            "vector": {
                "cornerSensitivity": 0.65,
                "curveSmoothing": 0.50,
                "optimization": 0.20,
                "requireClosedPaths": False,
                "minimumPathAreaPx2": 1,
            },
        },
        "geometry": {
            "policy": "stacked",
            "underlap": {"value": 0.5, "unit": "source_px"},
            "trapping": {"value": 0.0, "unit": "mm"},
            "preserveOuterSilhouette": True,
            "validateClosedPaths": False,
            "detectDuplicateGeometry": False,
        },
        "backend": {
            "preferredBackendId": "auto",
            "requireExactBackend": False,
            "options": {},
        },
        "output": {
            "updateExistingResult": True,
            "hideSourceImageAfterApply": False,
            "backgroundOutput": "keep_paths",
            "groupScans": True,
            "labelLayers": True,
        },
        "generation": {
            "lastGeneratedGroupId": None,
            "lastSourceHash": None,
            "lastSettingsHash": None,
            "lastBackendId": None,
            "lastBackendVersion": None,
        },
    }


def compute_settings_hash(settings: dict) -> str:
    """
    Stable SHA-256 over the settings that affect generated geometry (§10.3).

    The `generation` block is excluded: it records the result of a run, so
    including it would make the hash depend on its own previous value.
    """
    payload = {k: v for k, v in settings.items() if k != "generation"}
    canonical = json.dumps(payload, sort_keys=True, separators=(",", ":"))
    return hashlib.sha256(canonical.encode("utf-8")).hexdigest()


def record_generation_provenance(
    settings: dict,
    source_hash: str,
    backend_id: str,
    backend_version: str,
    generated_group_id: str | None = None,
) -> dict:
    """Writes the post-apply provenance block (§10.3, §27, §34.26)."""
    generation = settings.setdefault("generation", {})
    generation["lastSourceHash"] = source_hash
    generation["lastSettingsHash"] = compute_settings_hash(settings)
    generation["lastBackendId"] = backend_id
    generation["lastBackendVersion"] = backend_version
    if generated_group_id is not None:
        generation["lastGeneratedGroupId"] = generated_group_id

    settings.setdefault("source", {}).setdefault(
        "fingerprint", {"algorithm": "sha256", "value": ""}
    )["value"] = source_hash
    return settings


def reset_automatic_entries_preserving_pinned(settings: dict) -> dict:
    """
    §27 "Reset automatic palette while preserving pinned colours" — one of the
    four recovery choices offered when the source fingerprint has changed.

    Pinned entries (and their id, so layer order and background selection
    survive) are left untouched. Automatic entries are replaced with fresh
    defaults at the same position, so a stale automatic output colour from the
    previous image cannot linger.
    """
    entries = settings.get("palette", {}).get("entries", [])
    rebuilt = [
        entry if entry.get("kind") == "pinned" else create_palette_entry(i, entry.get("id"))
        for i, entry in enumerate(entries)
    ]
    settings.setdefault("palette", {})["entries"] = rebuilt
    return settings


def reset_settings_to_destination_defaults(image_uuid: str, dest_id: str) -> dict:
    """
    §27 "Start from destination defaults" — the strongest of the four source-
    change recovery choices. Unlike `apply_destination_preset`, this also
    discards palette entries and picked colours; it is a full restart under
    the chosen destination, not a resync of destination-governed fields.
    """
    # Imported locally to avoid a settings.py <-> presets.destination import
    # cycle: destination.py's apply_destination_preset needs nothing from
    # here at import time, but create_default_settings lives in this module.
    from palette_trace.presets.destination import apply_destination_preset

    settings = create_default_settings(image_uuid)
    apply_destination_preset(settings, dest_id)
    return settings


def stored_fingerprint(settings: dict) -> str:
    """Returns the recorded source fingerprint, or an empty string (§27)."""
    return settings.get("source", {}).get("fingerprint", {}).get("value", "") or ""


def source_has_changed(settings: dict, current_fingerprint: str) -> bool:
    """
    True when a fingerprint was recorded and no longer matches (§27).

    A missing recorded fingerprint means the image has never been applied, which
    is not a change — the caller should proceed normally.
    """
    recorded = stored_fingerprint(settings)
    return bool(recorded) and recorded != current_fingerprint
