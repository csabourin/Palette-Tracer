"""
Destination preset application tests (SPEC §19, §9.2 "Reset to destination
defaults"). Regression coverage for the defect where `get_destination_preset`
was loaded but never called, so changing the destination in the interface
never changed generated geometry.
"""

import pytest

from palette_trace.presets.destination import (
    apply_destination_preset,
    get_destination_preset,
    list_destination_ids,
    resolve_destination_id,
)
from palette_trace.settings import create_default_settings


class TestListingAndResolution:
    def test_every_spec_destination_is_present(self):
        expected = {
            "illustration", "logo_branding", "screen_printing",
            "vinyl_paper", "laser", "custom",
        }
        assert expected <= set(list_destination_ids())

    def test_unknown_id_resolves_to_illustration(self):
        assert resolve_destination_id("no_such_destination") == "illustration"

    def test_known_id_resolves_to_itself(self):
        assert resolve_destination_id("laser") == "laser"

    def test_get_destination_preset_returns_a_copy(self):
        first = get_destination_preset("laser")
        first["geometry"]["policy"] = "mutated"
        assert get_destination_preset("laser")["geometry"]["policy"] != "mutated"


class TestApplyDestinationPreset:
    def test_geometry_changes_with_destination(self):
        """The regression this closes: switching destination must change geometry."""
        settings = create_default_settings("test-uuid")
        assert settings["geometry"]["policy"] == "stacked"  # illustration default

        apply_destination_preset(settings, "laser")
        assert settings["geometry"]["policy"] == "separate_operations"
        assert settings["destination"]["id"] == "laser"

    def test_global_trace_profile_changes_with_destination(self):
        settings = create_default_settings("test-uuid")
        apply_destination_preset(settings, "vinyl_paper")
        # vinyl_paper's globalTraceProfileId is fabrication_clean (§19.4)
        assert settings["globalTraceProfile"]["mask"]["minimumRegionAreaPx2"] == 16

    def test_unknown_destination_falls_back_but_still_applies(self):
        settings = create_default_settings("test-uuid")
        apply_destination_preset(settings, "not_a_real_destination")
        assert settings["destination"]["id"] == "illustration"
        assert settings["geometry"]["policy"] == "stacked"

    def test_palette_entries_are_untouched(self):
        """Switching destinations must not discard a user's palette choices."""
        settings = create_default_settings("test-uuid")
        settings["palette"]["entries"][0]["kind"] = "pinned"
        settings["palette"]["entries"][0]["sourceAnchor"] = {"srgb": "#FF0000"}
        original_entries = settings["palette"]["entries"]

        apply_destination_preset(settings, "screen_printing")

        assert settings["palette"]["entries"] is original_entries
        assert settings["palette"]["entries"][0]["kind"] == "pinned"

    def test_scan_count_is_untouched(self):
        settings = create_default_settings("test-uuid")
        settings["scanCount"] = 12
        apply_destination_preset(settings, "logo_branding")
        assert settings["scanCount"] == 12

    def test_returns_the_same_object_it_mutates(self):
        settings = create_default_settings("test-uuid")
        result = apply_destination_preset(settings, "laser")
        assert result is settings
