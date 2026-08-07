"""
Local web API tests (SPEC §9, §19, §26, §27).

Exercises `handle_api_request` directly against a real `AppSession`, the same
way both hosts drive it, rather than mocking the dispatcher.
"""

import numpy as np
import pytest
from PIL import Image

from palette_trace.image_source import DecodedImageSource
from palette_trace.server.api import handle_api_request
from palette_trace.server.session import AppSession
from palette_trace.settings import create_default_settings


def two_colour_source(width=20, height=20):
    arr = np.zeros((height, width, 4), dtype=np.uint8)
    arr[:, : width // 2] = [200, 40, 40, 255]
    arr[:, width // 2:] = [40, 40, 200, 255]
    return DecodedImageSource(Image.fromarray(arr, mode="RGBA"))


@pytest.fixture
def session():
    s = AppSession()
    s.image_source = two_colour_source()
    s.settings = create_default_settings("test-image-uuid")
    return s


def call(session, path, method, body=None):
    headers = {"X-Session-Token": session.session_token}
    return handle_api_request(session, path, method, body or {}, headers)


class TestAuth:
    def test_missing_token_is_rejected(self, session):
        status, body = handle_api_request(session, "/api/session", "GET", {}, {})
        assert status == 401

    def test_wrong_token_is_rejected(self, session):
        status, body = handle_api_request(
            session, "/api/session", "GET", {}, {"X-Session-Token": "wrong"}
        )
        assert status == 401


class TestSession:
    def test_session_reports_source_changed_flag(self, session):
        session.source_changed = True
        status, body = call(session, "/api/session", "GET")
        assert status == 200
        assert body["sourceChanged"] is True


class TestDestinationEndpoints:
    def test_destination_presets_are_listed(self, session):
        status, body = call(session, "/api/destination_presets", "GET")
        assert status == 200
        assert "laser" in body["presets"]
        assert "laser" in body["order"]

    def test_apply_destination_changes_geometry_and_reruns_pipeline(self, session):
        status, body = call(session, "/api/apply_destination", "POST", {"destinationId": "laser"})
        assert status == 200
        assert body["settings"]["geometry"]["policy"] == "separate_operations"
        assert "scanResults" in body

    def test_reset_destination_defaults_uses_the_current_destination(self, session):
        session.settings["destination"]["id"] = "screen_printing"
        session.settings["geometry"]["policy"] = "stacked"  # simulate a manual tweak

        status, body = call(session, "/api/reset_destination_defaults", "POST")

        assert status == 200
        assert body["settings"]["geometry"]["policy"] == "stacked_trapped"


class TestTraceProfileEndpoint:
    def test_profiles_are_listed_with_bodies(self, session):
        status, body = call(session, "/api/trace_profiles", "GET")
        assert status == 200
        assert "sharp_details" in body["profiles"]
        assert body["profiles"]["sharp_details"]["vector"]["cornerSensitivity"] > 0


class TestSourceChangeResolution:
    def test_recalculate_clears_the_flag_without_changing_settings(self, session):
        session.source_changed = True
        original_scan_count = session.settings["scanCount"]

        status, body = call(session, "/api/resolve_source_change", "POST", {"action": "recalculate"})

        assert status == 200
        assert session.source_changed is False
        assert body["settings"]["scanCount"] == original_scan_count

    def test_reset_automatic_preserves_pinned_entries(self, session):
        session.source_changed = True
        entry = session.settings["palette"]["entries"][0]
        entry["kind"] = "pinned"
        entry["sourceAnchor"] = {"srgb": "#AA3300"}

        status, body = call(session, "/api/resolve_source_change", "POST", {"action": "reset_automatic"})

        assert status == 200
        assert body["settings"]["palette"]["entries"][0]["sourceAnchor"] == {"srgb": "#AA3300"}

    def test_destination_defaults_action_resets_the_whole_document(self, session):
        session.source_changed = True
        session.settings["palette"]["entries"][0]["kind"] = "pinned"
        session.settings["destination"]["id"] = "laser"

        status, body = call(session, "/api/resolve_source_change", "POST", {"action": "destination_defaults"})

        assert status == 200
        assert all(e["kind"] == "automatic" for e in body["settings"]["palette"]["entries"])
        assert body["settings"]["geometry"]["policy"] == "separate_operations"

    def test_cancel_sets_the_cancelled_flag_and_does_not_rerun(self, session):
        status, body = call(session, "/api/resolve_source_change", "POST", {"action": "cancel"})
        assert status == 200
        assert session.is_cancelled is True
        assert "settings" not in body

    def test_unknown_action_is_rejected(self, session):
        status, body = call(session, "/api/resolve_source_change", "POST", {"action": "not_a_real_action"})
        assert status == 400


class TestUserPresetEndpoints:
    @pytest.fixture(autouse=True)
    def isolated_presets_dir(self, tmp_path, monkeypatch):
        monkeypatch.setenv("HOME", str(tmp_path))
        monkeypatch.setattr("pathlib.Path.home", lambda: tmp_path)

    def test_save_list_apply_delete_round_trip(self, session):
        session.settings["palette"]["entries"][0]["kind"] = "pinned"
        session.settings["palette"]["entries"][0]["sourceAnchor"] = {"srgb": "#00FF00"}

        status, body = call(session, "/api/user_presets", "POST", {
            "name": "Green pin", "description": "", "scope": "palette",
        })
        assert status == 200
        preset_uuid = body["preset"]["presetUuid"]

        status, body = call(session, "/api/user_presets", "GET")
        assert status == 200
        assert len(body["presets"]) == 1

        # Applying to a differently-initialised session's settings should
        # still bring the pinned colour across (§8.3).
        session.settings = create_default_settings("another-image-uuid")
        status, body = call(session, "/api/user_presets/apply", "POST", {"presetUuid": preset_uuid})
        assert status == 200
        assert body["settings"]["palette"]["entries"][0]["sourceAnchor"] == {"srgb": "#00FF00"}

        status, body = call(session, "/api/user_presets/delete", "POST", {"presetUuid": preset_uuid})
        assert status == 200
        assert body["presets"] == []

    def test_save_without_a_name_is_rejected(self, session):
        status, body = call(session, "/api/user_presets", "POST", {"name": "", "scope": "full"})
        assert status == 400

    def test_apply_unknown_preset_is_a_404(self, session):
        status, body = call(session, "/api/user_presets/apply", "POST", {"presetUuid": "no-such-id"})
        assert status == 404
