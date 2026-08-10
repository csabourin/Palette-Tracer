"""
Unit tests for settings schema loading and default initialization.
"""

import inkex

from palette_trace.document.settings_store import (
    create_default_settings,
    load_or_initialize_settings,
)


def test_default_settings():
    settings = create_default_settings("test-uuid-1234")
    assert settings["schemaVersion"] == 1
    assert settings["scanCount"] == 4
    assert len(settings["palette"]["entries"]) == 4

def test_load_settings_initialization():
    elem = inkex.Image()
    settings = load_or_initialize_settings(elem)
    assert settings["schemaVersion"] == 1
    assert elem.get("{https://christiansabourin.com/ns/palette-trace/1}image-uuid") is not None
