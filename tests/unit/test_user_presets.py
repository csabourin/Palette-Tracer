"""
Saved user preset tests (SPEC §26, §8.3).
"""

import pytest

from palette_trace.presets.user_presets import (
    apply_configuration_patch,
    build_configuration_patch,
    delete_user_preset,
    get_user_presets_dir,
    list_user_presets,
    load_user_preset,
    save_user_preset,
)
from palette_trace.settings import create_default_settings


@pytest.fixture(autouse=True)
def isolated_presets_dir(tmp_path, monkeypatch):
    """Every test gets its own preset directory instead of the real ~/.config."""
    monkeypatch.setenv("HOME", str(tmp_path))
    monkeypatch.setattr("pathlib.Path.home", lambda: tmp_path)
    yield tmp_path


@pytest.fixture
def pinned_settings():
    settings = create_default_settings("source-image-uuid")
    settings["destination"]["id"] = "laser"
    settings["scanCount"] = 3
    settings["palette"]["entries"] = settings["palette"]["entries"][:3]
    entry = settings["palette"]["entries"][0]
    entry["kind"] = "pinned"
    entry["sourceAnchor"] = {"srgb": "#AA3300"}
    entry["output"] = {"mode": "exact", "color": {"srgb": "#AA3300"}}
    entry["role"] = "outline"
    return settings


class TestDirectoryIsolation:
    def test_presets_dir_is_under_the_fake_home(self, tmp_path):
        assert str(get_user_presets_dir()).startswith(str(tmp_path))


class TestSaveAndList:
    def test_save_creates_a_readable_preset(self, pinned_settings):
        preset = save_user_preset("Brand palette", "for laser jobs", pinned_settings, scope="full")
        assert preset["name"] == "Brand palette"
        assert preset["schemaVersion"] == 1
        assert preset["createdAt"] == preset["updatedAt"]
        assert preset["createdAt"]  # not the old hard-coded literal

        listed = list_user_presets()
        assert len(listed) == 1
        assert listed[0]["presetUuid"] == preset["presetUuid"]

    def test_two_presets_get_distinct_timestamps_or_at_least_distinct_ids(self, pinned_settings):
        first = save_user_preset("A", "", pinned_settings, scope="full")
        second = save_user_preset("B", "", pinned_settings, scope="full")
        assert first["presetUuid"] != second["presetUuid"]

    def test_load_returns_none_for_unknown_id(self):
        assert load_user_preset("00000000-0000-0000-0000-000000000000") is None

    def test_delete_removes_the_file(self, pinned_settings):
        preset = save_user_preset("Temp", "", pinned_settings, scope="full")
        assert delete_user_preset(preset["presetUuid"]) is True
        assert list_user_presets() == []

    def test_delete_unknown_id_returns_false(self):
        assert delete_user_preset("00000000-0000-0000-0000-000000000000") is False


class TestConfigurationPatchScopes:
    def test_structure_scope_excludes_exact_colours(self, pinned_settings):
        patch = build_configuration_patch(pinned_settings, "structure")
        assert "entries" not in patch
        assert patch["destination"]["id"] == "laser"
        assert patch["entryStructure"][0]["role"] == "outline"
        assert "sourceAnchor" not in patch["entryStructure"][0]

    def test_palette_scope_excludes_destination_and_geometry(self, pinned_settings):
        patch = build_configuration_patch(pinned_settings, "palette")
        assert "destination" not in patch
        assert "geometry" not in patch
        assert patch["entries"][0]["sourceAnchor"] == {"srgb": "#AA3300"}

    def test_full_scope_includes_both(self, pinned_settings):
        patch = build_configuration_patch(pinned_settings, "full")
        assert patch["destination"]["id"] == "laser"
        assert patch["entries"][0]["sourceAnchor"] == {"srgb": "#AA3300"}


class TestApplyConfigurationPatch:
    def test_full_patch_round_trips_onto_a_fresh_document(self, pinned_settings):
        patch = build_configuration_patch(pinned_settings, "full")
        target = create_default_settings("a-different-image-uuid")

        apply_configuration_patch(target, patch)

        assert target["destination"]["id"] == "laser"
        assert target["scanCount"] == 3
        assert len(target["palette"]["entries"]) == 3
        assert target["palette"]["entries"][0]["sourceAnchor"] == {"srgb": "#AA3300"}
        # imageUuid must survive untouched -- a preset must not overwrite identity.
        assert target["imageUuid"] == "a-different-image-uuid"

    def test_applied_entries_get_fresh_ids_not_the_source_documents_ids(self, pinned_settings):
        patch = build_configuration_patch(pinned_settings, "full")
        source_ids = {e["id"] for e in pinned_settings["palette"]["entries"]}
        target = create_default_settings("other-uuid")

        apply_configuration_patch(target, patch)

        applied_ids = {e["id"] for e in target["palette"]["entries"]}
        assert applied_ids.isdisjoint(source_ids)
        assert target["palette"]["layerOrder"] == [e["id"] for e in target["palette"]["entries"]]

    def test_structure_patch_carries_roles_without_colours(self, pinned_settings):
        patch = build_configuration_patch(pinned_settings, "structure")
        target = create_default_settings("other-uuid")

        apply_configuration_patch(target, patch)

        assert target["palette"]["entries"][0]["role"] == "outline"
        assert target["palette"]["entries"][0]["kind"] == "automatic"
        assert target["palette"]["entries"][0]["sourceAnchor"] is None

    def test_scan_count_can_differ_from_the_target_documents_original(self, pinned_settings):
        target = create_default_settings("other-uuid")
        assert target["scanCount"] == 4
        patch = build_configuration_patch(pinned_settings, "structure")  # scanCount 3

        apply_configuration_patch(target, patch)

        assert target["scanCount"] == 3
        assert len(target["palette"]["entries"]) == 3
