"""
Settings hashing, provenance and fingerprint tests (SPEC §10.3, §27, criteria 22-26).
"""

import copy

from palette_trace.settings import (
    compute_settings_hash,
    create_default_settings,
    record_generation_provenance,
    reset_automatic_entries_preserving_pinned,
    reset_settings_to_destination_defaults,
    source_has_changed,
    stored_fingerprint,
)


class TestSettingsHash:
    def test_hash_is_stable_across_calls(self):
        settings = create_default_settings("uuid-1")
        assert compute_settings_hash(settings) == compute_settings_hash(settings)

    def test_hash_is_independent_of_key_order(self):
        settings = create_default_settings("uuid-1")
        reordered = dict(reversed(list(settings.items())))
        assert compute_settings_hash(settings) == compute_settings_hash(reordered)

    def test_hash_changes_when_configuration_changes(self):
        settings = create_default_settings("uuid-1")
        before = compute_settings_hash(settings)
        settings["scanCount"] = 8
        assert compute_settings_hash(settings) != before

    def test_generation_block_is_excluded(self):
        """The block records the result of a run, so it cannot feed its own hash."""
        settings = create_default_settings("uuid-1")
        before = compute_settings_hash(settings)
        settings["generation"]["lastGeneratedGroupId"] = "palette-trace-result-abcdef"
        assert compute_settings_hash(settings) == before


class TestFingerprint:
    def test_new_settings_have_no_fingerprint(self):
        assert stored_fingerprint(create_default_settings("uuid-1")) == ""

    def test_unapplied_image_is_not_reported_as_changed(self):
        """§27: no recorded fingerprint means never applied, not changed."""
        assert not source_has_changed(create_default_settings("uuid-1"), "abc123")

    def test_matching_fingerprint_is_not_a_change(self):
        settings = create_default_settings("uuid-1")
        record_generation_provenance(settings, "abc123", "potrace", "1.16")
        assert not source_has_changed(settings, "abc123")

    def test_differing_fingerprint_is_a_change(self):
        settings = create_default_settings("uuid-1")
        record_generation_provenance(settings, "abc123", "potrace", "1.16")
        assert source_has_changed(settings, "def456")


class TestProvenance:
    def test_provenance_is_recorded(self):
        settings = create_default_settings("uuid-1")
        record_generation_provenance(settings, "src-hash", "potrace", "1.16", "group-1")

        generation = settings["generation"]
        assert generation["lastSourceHash"] == "src-hash"
        assert generation["lastBackendId"] == "potrace"
        assert generation["lastBackendVersion"] == "1.16"
        assert generation["lastGeneratedGroupId"] == "group-1"
        assert generation["lastSettingsHash"]

    def test_fingerprint_is_written_into_source_block(self):
        settings = create_default_settings("uuid-1")
        record_generation_provenance(settings, "src-hash", "potrace", "1.16")
        assert settings["source"]["fingerprint"]["value"] == "src-hash"
        assert settings["source"]["fingerprint"]["algorithm"] == "sha256"

    def test_group_id_is_preserved_when_not_supplied(self):
        settings = create_default_settings("uuid-1")
        settings["generation"]["lastGeneratedGroupId"] = "existing"
        record_generation_provenance(settings, "src-hash", "potrace", "1.16")
        assert settings["generation"]["lastGeneratedGroupId"] == "existing"

    def test_recorded_settings_hash_detects_later_edits(self):
        """Criterion 26: manual-edit risk is detectable."""
        settings = create_default_settings("uuid-1")
        record_generation_provenance(settings, "src-hash", "potrace", "1.16")
        recorded = settings["generation"]["lastSettingsHash"]

        edited = copy.deepcopy(settings)
        edited["palette"]["entries"][0]["name"] = "Renamed"
        assert compute_settings_hash(edited) != recorded


class TestSourceChangeRecovery:
    """§27 recovery choices offered when the source fingerprint has changed."""

    def test_reset_automatic_preserves_pinned_entries(self):
        settings = create_default_settings("uuid-1")
        pinned = settings["palette"]["entries"][1]
        pinned["kind"] = "pinned"
        pinned["sourceAnchor"] = {"srgb": "#123456"}
        pinned["name"] = "My red"

        reset_automatic_entries_preserving_pinned(settings)

        assert settings["palette"]["entries"][1]["sourceAnchor"] == {"srgb": "#123456"}
        assert settings["palette"]["entries"][1]["name"] == "My red"

    def test_reset_automatic_replaces_non_pinned_entries(self):
        settings = create_default_settings("uuid-1")
        settings["palette"]["entries"][0]["name"] = "Stale automatic"
        settings["palette"]["entries"][0]["output"] = {
            "mode": "automatic_centroid", "color": {"srgb": "#FFFFFF"},
        }

        reset_automatic_entries_preserving_pinned(settings)

        assert settings["palette"]["entries"][0]["name"] == "Scan 1"
        assert settings["palette"]["entries"][0]["output"]["color"] is None

    def test_reset_automatic_preserves_entry_ids_and_count(self):
        settings = create_default_settings("uuid-1")
        original_ids = [e["id"] for e in settings["palette"]["entries"]]

        reset_automatic_entries_preserving_pinned(settings)

        assert [e["id"] for e in settings["palette"]["entries"]] == original_ids

    def test_reset_to_destination_defaults_discards_palette_and_applies_geometry(self):
        settings = create_default_settings("uuid-1")
        settings["palette"]["entries"][0]["kind"] = "pinned"

        fresh = reset_settings_to_destination_defaults("uuid-1", "laser")

        assert fresh["imageUuid"] == "uuid-1"
        assert fresh["destination"]["id"] == "laser"
        assert fresh["geometry"]["policy"] == "separate_operations"
        assert all(e["kind"] == "automatic" for e in fresh["palette"]["entries"])
