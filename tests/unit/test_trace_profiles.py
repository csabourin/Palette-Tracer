"""
Trace-profile resolution tests (SPEC §18, MVP criteria 13-14).
"""

import pytest

from palette_trace.presets.profiles import (
    DEFAULT_PROFILE_ID,
    get_builtin_profile,
    list_profile_ids,
    merge_profiles,
    resolve_entry_profile,
    unsupported_settings,
)
from palette_trace.settings import create_default_settings


@pytest.fixture
def global_profile():
    return create_default_settings("test-uuid")["globalTraceProfile"]


class TestBuiltinProfiles:
    def test_every_spec_profile_is_present(self):
        """§18 lists seven initial profiles."""
        expected = {
            "default", "smooth_shapes", "sharp_details", "thin_line_art",
            "small_accents", "simplified_background", "fabrication_clean",
        }
        assert expected <= set(list_profile_ids())

    def test_profiles_are_materially_different(self):
        """Criterion 14: different scans must be able to trace differently."""
        sharp = get_builtin_profile("sharp_details")["vector"]
        smooth = get_builtin_profile("smooth_shapes")["vector"]
        assert sharp["cornerSensitivity"] > smooth["cornerSensitivity"]
        assert sharp["curveSmoothing"] < smooth["curveSmoothing"]

    def test_unknown_profile_falls_back_to_default(self):
        assert get_builtin_profile("no_such_profile") == get_builtin_profile(DEFAULT_PROFILE_ID)

    def test_returned_profile_is_a_copy(self):
        first = get_builtin_profile("default")
        first["vector"]["cornerSensitivity"] = 0.0
        assert get_builtin_profile("default")["vector"]["cornerSensitivity"] != 0.0


class TestMerge:
    def test_override_replaces_only_named_keys(self):
        base = {"vector": {"cornerSensitivity": 0.65, "optimization": 0.2}}
        merged = merge_profiles(base, {"vector": {"cornerSensitivity": 0.9}})
        assert merged["vector"]["cornerSensitivity"] == 0.9
        assert merged["vector"]["optimization"] == 0.2

    def test_base_is_not_mutated(self):
        base = {"vector": {"cornerSensitivity": 0.65}}
        merge_profiles(base, {"vector": {"cornerSensitivity": 0.1}})
        assert base["vector"]["cornerSensitivity"] == 0.65

    def test_empty_override_is_identity(self):
        base = {"mask": {"offsetPx": 3}}
        assert merge_profiles(base, {}) == base


class TestResolution:
    def test_inherit_returns_the_global_profile(self, global_profile):
        entry = {"traceProfile": {"mode": "inherit"}}
        assert resolve_entry_profile(entry, global_profile) == global_profile

    def test_missing_trace_profile_inherits(self, global_profile):
        assert resolve_entry_profile({}, global_profile) == global_profile

    def test_preset_mode_selects_a_builtin(self, global_profile):
        entry = {"traceProfile": {"mode": "preset", "profileId": "thin_line_art"}}
        assert resolve_entry_profile(entry, global_profile) == get_builtin_profile("thin_line_art")

    def test_override_merges_over_the_global_profile(self, global_profile):
        entry = {"traceProfile": {"mode": "override", "values": {"vector": {"optimization": 0.99}}}}
        resolved = resolve_entry_profile(entry, global_profile)
        assert resolved["vector"]["optimization"] == 0.99
        assert resolved["vector"]["cornerSensitivity"] == \
            global_profile["vector"]["cornerSensitivity"]

    def test_override_can_rebase_onto_a_preset(self, global_profile):
        entry = {"traceProfile": {
            "mode": "override",
            "profileId": "sharp_details",
            "values": {"mask": {"offsetPx": 2}},
        }}
        resolved = resolve_entry_profile(entry, global_profile)
        assert resolved["mask"]["offsetPx"] == 2
        assert resolved["vector"]["cornerSensitivity"] == \
            get_builtin_profile("sharp_details")["vector"]["cornerSensitivity"]

    def test_unknown_mode_inherits_rather_than_failing(self, global_profile):
        entry = {"traceProfile": {"mode": "some_future_mode"}}
        assert resolve_entry_profile(entry, global_profile) == global_profile

    def test_two_entries_resolve_to_different_profiles(self, global_profile):
        """Criterion 14, end to end."""
        sharp = resolve_entry_profile(
            {"traceProfile": {"mode": "preset", "profileId": "sharp_details"}}, global_profile)
        smooth = resolve_entry_profile(
            {"traceProfile": {"mode": "preset", "profileId": "smooth_shapes"}}, global_profile)
        assert sharp["vector"] != smooth["vector"]

    def test_resolution_does_not_mutate_the_global_profile(self, global_profile):
        original = global_profile["vector"]["optimization"]
        entry = {"traceProfile": {"mode": "override", "values": {"vector": {"optimization": 0.01}}}}
        resolve_entry_profile(entry, global_profile)
        assert global_profile["vector"]["optimization"] == original


class TestUnsupportedReporting:
    def test_unsupported_settings_are_named(self):
        """§18: a backend must report what it cannot honour."""
        profile = {"vector": {"cornerSensitivity": 0.5, "curveSmoothing": 0.5}}
        assert unsupported_settings(profile, frozenset(["cornerSensitivity"])) == ["curveSmoothing"]

    def test_fully_supported_profile_reports_nothing(self):
        profile = {"vector": {"cornerSensitivity": 0.5}}
        assert unsupported_settings(profile, frozenset(["cornerSensitivity"])) == []

    def test_structural_flags_are_not_engine_settings(self):
        profile = {"vector": {"requireClosedPaths": True}}
        assert unsupported_settings(profile, frozenset()) == []
