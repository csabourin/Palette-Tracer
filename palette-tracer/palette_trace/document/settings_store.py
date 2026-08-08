"""
Read and write the pt: settings attributes on SVG <image> elements (SPEC §10.2).

This is the Inkscape-host binding only. The settings themselves — schema,
defaults and hashing — live in `palette_trace.settings`, which is host-neutral
so that the standalone application can reuse them.
"""

import json

import inkex

from palette_trace.settings import (
    EXTENSION_VERSION,
    PT_NAMESPACE,
    PT_SCHEMA_VERSION,
    compute_settings_hash,
    create_default_settings,
    create_palette_entry,
    record_generation_provenance,
    source_has_changed,
    stored_fingerprint,
)

__all__ = [
    "PT_NAMESPACE",
    "PT_SCHEMA_VERSION",
    "EXTENSION_VERSION",
    "compute_settings_hash",
    "create_default_settings",
    "create_palette_entry",
    "load_or_initialize_settings",
    "record_generation_provenance",
    "save_settings",
    "source_has_changed",
    "stored_fingerprint",
]


def load_or_initialize_settings(image_elem: inkex.Image) -> dict:
    """Reads pt:settings from the image element, or initializes defaults."""
    import uuid

    img_uuid = image_elem.get(f"{{{PT_NAMESPACE}}}image-uuid") or str(uuid.uuid4())
    image_elem.set(f"{{{PT_NAMESPACE}}}image-uuid", img_uuid)

    raw_settings = image_elem.get(f"{{{PT_NAMESPACE}}}settings")
    if raw_settings:
        try:
            settings = json.loads(raw_settings)
            if settings.get("schemaVersion") == PT_SCHEMA_VERSION:
                return settings
        except (ValueError, TypeError):
            pass

    return create_default_settings(img_uuid)


def save_settings(image_elem: inkex.Image, settings: dict) -> None:
    """
    Serializes settings into minified JSON on the image element (§10.2).

    Settings live on the image so that deleting the image deletes them (§34.23).
    """
    settings["schemaVersion"] = PT_SCHEMA_VERSION
    raw_json = json.dumps(settings, separators=(",", ":"))
    image_elem.set(f"{{{PT_NAMESPACE}}}schema-version", str(PT_SCHEMA_VERSION))
    image_elem.set(f"{{{PT_NAMESPACE}}}settings", raw_json)
