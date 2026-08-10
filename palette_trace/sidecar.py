"""
Standalone settings persistence (SPEC §9.4.3, §10.5).

The Inkscape host stores settings as an attribute on the source `<image>`, so
deleting the image deletes them. A standalone host has no document to store them
in and uses a sidecar file beside the image instead.

The sidecar is advisory: anything wrong with it falls back to defaults rather
than failing, because losing a configuration is an annoyance and refusing to open
an image is a broken tool.
"""

import json
import os
import tempfile
import uuid
from pathlib import Path

from palette_trace.settings import PT_SCHEMA_VERSION, create_default_settings

SIDECAR_SUFFIX = ".palettetrace.json"


def sidecar_path(image_path) -> Path:
    """Returns the sidecar path for a source image."""
    path = Path(image_path)
    return path.with_name(path.name + SIDECAR_SUFFIX)


def load_settings(image_path) -> dict:
    """
    Loads settings for `image_path`, or returns defaults.

    A missing, unreadable, malformed or wrong-version sidecar yields defaults
    (§9.4.3).
    """
    path = sidecar_path(image_path)
    try:
        with open(path, encoding="utf-8") as handle:
            settings = json.load(handle)
    except (OSError, ValueError):
        return create_default_settings(str(uuid.uuid4()))

    if not isinstance(settings, dict) or settings.get("schemaVersion") != PT_SCHEMA_VERSION:
        return create_default_settings(str(uuid.uuid4()))

    settings.setdefault("imageUuid", str(uuid.uuid4()))
    return settings


def save_settings(image_path, settings: dict) -> Path:
    """
    Writes the sidecar atomically beside the source image.

    A partial write would be indistinguishable from corruption on the next open,
    so the file is written to a temporary neighbour and moved into place.
    """
    settings["schemaVersion"] = PT_SCHEMA_VERSION
    path = sidecar_path(image_path)
    write_atomically(path, json.dumps(settings, separators=(",", ":")))
    return path


def write_atomically(path, text: str, encoding: str = "utf-8") -> None:
    """
    Writes `text` to `path` via a temporary file in the same directory.

    Same-directory placement matters: `os.replace` is only atomic within one
    filesystem, and a temp directory may be on another one.
    """
    path = Path(path)
    path.parent.mkdir(parents=True, exist_ok=True)

    handle, temp_path = tempfile.mkstemp(
        dir=str(path.parent), prefix=f".{path.name}.", suffix=".tmp"
    )
    try:
        with os.fdopen(handle, "w", encoding=encoding) as stream:
            stream.write(text)
            stream.flush()
            os.fsync(stream.fileno())
        os.replace(temp_path, path)
    except BaseException:
        if os.path.exists(temp_path):
            os.unlink(temp_path)
        raise
