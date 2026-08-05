"""
Read and write pt:settings attribute on SVG <image> elements.
"""

import json
import uuid
import inkex
from palette_trace.errors import SchemaValidationError

PT_NAMESPACE = "https://christiansabourin.com/ns/palette-trace/1"
PT_SCHEMA_VERSION = 1

def load_or_initialize_settings(image_elem: inkex.Image) -> dict:
    """
    Reads pt:settings from the image element or initializes defaults.
    """
    img_uuid = image_elem.get(f"{{{PT_NAMESPACE}}}image-uuid") or str(uuid.uuid4())
    image_elem.set(f"{{{PT_NAMESPACE}}}image-uuid", img_uuid)

    raw_settings = image_elem.get(f"{{{PT_NAMESPACE}}}settings")
    if raw_settings:
        try:
            settings = json.loads(raw_settings)
            if settings.get("schemaVersion") == 1:
                return settings
        except Exception:
            pass

    # Initialize new defaults
    return create_default_settings(img_uuid)

def save_settings(image_elem: inkex.Image, settings: dict):
    """
    Serializes settings dict into minified JSON on image element.
    """
    settings["schemaVersion"] = PT_SCHEMA_VERSION
    raw_json = json.dumps(settings, separators=(",", ":"))
    image_elem.set(f"{{{PT_NAMESPACE}}}schema-version", str(PT_SCHEMA_VERSION))
    image_elem.set(f"{{{PT_NAMESPACE}}}settings", raw_json)

def create_default_settings(image_uuid: str) -> dict:
    """Creates default PaletteTraceImageSettingsV1 configuration."""
    scan_count = 4

    default_entries = []
    layer_order = []
    for i in range(scan_count):
        eid = str(uuid.uuid4())
        layer_order.append(eid)
        default_entries.append({
            "id": eid,
            "name": f"Scan {i+1}",
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
                    "hue": { "enabled": True, "tolerance": 10.0, "weight": 1.0 },
                    "chroma": { "enabled": True, "tolerance": 0.035, "weight": 1.0 },
                    "lightness": { "enabled": True, "tolerance": 0.06, "weight": 1.0 },
                },
            },
            "role": "primary_fill",
            "traceProfile": { "mode": "inherit" },
        })

    return {
        "schemaVersion": 1,
        "extensionVersion": "1.0.0",
        "imageUuid": image_uuid,
        "source": {
            "sourceMode": "intrinsic",
            "fingerprint": { "algorithm": "sha256", "value": "" },
            "intrinsicWidth": 0,
            "intrinsicHeight": 0,
            "mimeType": "image/png",
            "traceClipEffects": False,
            "traceSvgFilters": False,
        },
        "destination": { "id": "illustration", "presetVersion": 1 },
        "scanCount": scan_count,
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
            "entries": default_entries,
            "layerOrder": layer_order,
            "backgroundEntryId": None,
        },
        "alpha": {
            "fullyTransparentPolicy": "ignore",
            "partialAlphaPolicy": "composite",
            "ignoreBelow": 0.05,
            "matte": { "mode": "white" },
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
            "underlap": { "value": 0.5, "unit": "source_px" },
            "trapping": { "value": 0.0, "unit": "mm" },
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
