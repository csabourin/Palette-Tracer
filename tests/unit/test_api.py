"""
Local web API tests (SPEC §9, §19, §26, §27).

Exercises `handle_api_request` directly against a real `AppSession`, the same
way both hosts drive it, rather than mocking the dispatcher.
"""

import base64
import io

import numpy as np
import pytest
from PIL import Image

from palette_trace.image_source import DecodedImageSource
from palette_trace.server.api import handle_api_request, sample_source_color
from palette_trace.server.session import AppSession
from palette_trace.settings import create_default_settings


def two_colour_array(width=20, height=20):
    arr = np.zeros((height, width, 4), dtype=np.uint8)
    arr[:, : width // 2] = [200, 40, 40, 255]
    arr[:, width // 2:] = [40, 40, 200, 255]
    return arr


def two_colour_source(width=20, height=20):
    return DecodedImageSource(Image.fromarray(two_colour_array(width, height), mode="RGBA"))


def two_colour_data_uri(width=20, height=20):
    buf = io.BytesIO()
    Image.fromarray(two_colour_array(width, height), mode="RGBA").save(buf, format="PNG")
    return "data:image/png;base64," + base64.b64encode(buf.getvalue()).decode("ascii")


@pytest.fixture
def session():
    s = AppSession()
    s.image_source = two_colour_source()
    s.image_name = "sample.png"
    s.settings = create_default_settings("test-image-uuid")
    return s


@pytest.fixture
def empty_session():
    """A standalone host launched with no image, showing the loading screen."""
    s = AppSession()
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

    def test_session_with_an_image_says_so(self, session):
        status, body = call(session, "/api/session", "GET")
        assert body["hasImage"] is True
        assert body["imageName"] == "sample.png"
        assert (body["imageWidth"], body["imageHeight"]) == (20, 20)

    def test_session_without_an_image_says_so(self, empty_session):
        """The interface opens on its loading screen when this is false."""
        status, body = call(empty_session, "/api/session", "GET")
        assert status == 200
        assert body["hasImage"] is False
        assert body["canLoadImage"] is True

    def test_a_session_with_nowhere_to_write_commits_by_download(self, empty_session):
        assert call(empty_session, "/api/session", "GET")[1]["commitTarget"] == "download"

    def test_a_session_bound_to_an_output_file_commits_to_it(self, session, tmp_path):
        session.output_path = tmp_path / "out.svg"
        assert call(session, "/api/session", "GET")[1]["commitTarget"] == "file"

    def test_the_inkscape_host_commits_to_the_document_and_cannot_swap_images(self, session):
        session.image_elem = object()   # stands in for the selected <image>
        status, body = call(session, "/api/session", "GET")
        assert body["commitTarget"] == "document"
        assert body["canLoadImage"] is False


class TestImageRequiredEndpoints:
    @pytest.mark.parametrize("path", [
        "/api/sample_color",
        "/api/update_settings",
        "/api/apply_destination",
        "/api/export",
        "/api/apply",
    ])
    def test_they_are_refused_before_an_image_exists(self, empty_session, path):
        status, body = call(empty_session, path, "POST")
        assert status == 400
        assert "Load an image first" in body["error"]

    def test_the_session_endpoint_still_answers(self, empty_session):
        assert call(empty_session, "/api/session", "GET")[0] == 200


class TestLoadImage:
    def test_loading_a_picture_returns_a_ready_workspace(self, empty_session):
        status, body = call(empty_session, "/api/load_image", "POST", {
            "dataUri": two_colour_data_uri(30, 18), "fileName": "holiday.png",
        })

        assert status == 200
        assert body["hasImage"] is True
        assert body["imageName"] == "holiday.png"
        assert (body["imageWidth"], body["imageHeight"]) == (30, 18)
        # The first pipeline run comes back with the response, so the interface
        # has something to draw without a second round trip.
        assert body["scanResults"]
        assert body["dataUri"].startswith("data:image/png;base64,")

    def test_the_chosen_destination_survives_a_new_picture(self, empty_session):
        empty_session.settings["destination"]["id"] = "laser"
        status, body = call(empty_session, "/api/load_image", "POST", {
            "dataUri": two_colour_data_uri(), "fileName": "a.png",
        })
        assert body["settings"]["destination"]["id"] == "laser"
        assert body["settings"]["geometry"]["policy"] == "separate_operations"

    def test_a_new_picture_discards_the_previous_one_s_pinned_colours(self, session):
        """
        Pinned colours, claims and layer order all describe the *old* bitmap.
        Carrying them over would silently apply one picture's art direction to
        another (§27).
        """
        session.settings["palette"]["entries"][0]["kind"] = "pinned"
        session.settings["palette"]["entries"][0]["sourceAnchor"] = {"srgb": "#AA3300"}

        status, body = call(session, "/api/load_image", "POST", {
            "dataUri": two_colour_data_uri(), "fileName": "b.png",
        })

        assert status == 200
        assert all(e["kind"] == "automatic" for e in body["settings"]["palette"]["entries"])

    def test_loading_switches_the_session_to_the_download_path(self, session, tmp_path):
        """
        A bitmap chosen in the browser is not the file named on the command
        line, so its result must not be written over that file's output.
        """
        session.output_path = tmp_path / "out.svg"
        status, body = call(session, "/api/load_image", "POST", {
            "dataUri": two_colour_data_uri(), "fileName": "b.png",
        })
        assert body["commitTarget"] == "download"
        assert session.output_path is None

    def test_a_bad_upload_is_rejected_without_disturbing_the_session(self, session):
        original = session.image_source
        status, body = call(session, "/api/load_image", "POST", {
            "dataUri": "data:image/png;base64,####", "fileName": "b.png",
        })
        assert status == 400
        assert session.image_source is original

    def test_the_inkscape_host_refuses_to_swap_its_image(self, session):
        session.image_elem = object()
        status, body = call(session, "/api/load_image", "POST", {
            "dataUri": two_colour_data_uri(), "fileName": "b.png",
        })
        assert status == 400


class TestRawUpload:
    """
    §9.4.2: the bytes may also arrive as themselves, not wrapped in a data URI.

    A phone photograph spends a third again on base64 plus a full encode and
    decode pass on each end, all of it before the picture can appear.
    """

    def png_bytes(self, width=30, height=18):
        buf = io.BytesIO()
        Image.fromarray(two_colour_array(width, height), mode="RGBA").save(buf, format="PNG")
        return buf.getvalue()

    def post_raw(self, session, raw, extra_headers=None):
        headers = {"X-Session-Token": session.session_token, **(extra_headers or {})}
        body = {"rawBody": raw, "contentType": "image/png"}
        return handle_api_request(session, "/api/load_image", "POST", body, headers)

    def test_raw_bytes_load_the_same_picture_a_data_uri_would(self, empty_session):
        status, body = self.post_raw(
            empty_session, self.png_bytes(), {"X-File-Name": "holiday%20snap.png"}
        )

        assert status == 200
        assert (body["imageWidth"], body["imageHeight"]) == (30, 18)
        assert body["imageName"] == "holiday snap.png"

    def test_bytes_that_are_not_an_image_are_refused(self, empty_session):
        status, body = self.post_raw(empty_session, b"not a picture")
        assert status == 400

    @pytest.mark.parametrize("declared", ["", "application/octet-stream", "application/pdf"])
    def test_the_bytes_decide_when_the_declared_type_is_no_help(self, empty_session, declared):
        """
        A proxy can rewrite a Content-Type and a file picker can omit one, but
        a PNG still begins with a PNG's bytes.
        """
        headers = {"X-Session-Token": empty_session.session_token}
        status, body = handle_api_request(
            empty_session, "/api/load_image", "POST",
            {"rawBody": self.png_bytes(), "contentType": declared}, headers,
        )

        assert status == 200
        assert (body["imageWidth"], body["imageHeight"]) == (30, 18)

    def test_something_that_is_not_an_image_is_still_refused(self, empty_session):
        headers = {"X-Session-Token": empty_session.session_token}
        status, body = handle_api_request(
            empty_session, "/api/load_image", "POST",
            {"rawBody": b"%PDF-1.4 not a picture", "contentType": "application/pdf"}, headers,
        )
        assert status == 400
        assert "application/pdf" in body["error"]

    def test_a_client_holding_the_same_pixels_is_not_sent_them_back(self, empty_session):
        status, body = self.post_raw(
            empty_session, self.png_bytes(), {"X-Client-Width": "30", "X-Client-Height": "18"},
        )
        assert body["usesClientBitmap"] is True
        assert "dataUri" not in body

    def test_a_client_whose_pixels_no_longer_match_gets_the_preview(self, empty_session):
        """A rotation or a resize during decode makes the client's copy wrong."""
        status, body = self.post_raw(
            empty_session, self.png_bytes(), {"X-Client-Width": "300", "X-Client-Height": "180"},
        )
        assert body.get("usesClientBitmap") is not True
        assert body["dataUri"].startswith("data:image/png;base64,")

    def test_deferring_the_trace_returns_the_picture_without_it(self, empty_session):
        status, body = self.post_raw(empty_session, self.png_bytes(), {"X-Defer-Trace": "1"})

        assert status == 200
        assert body["hasImage"] is True
        assert "scanResults" not in body

    def test_the_deferred_trace_is_what_the_next_update_produces(self, empty_session):
        self.post_raw(empty_session, self.png_bytes(), {"X-Defer-Trace": "1"})
        status, body = call(empty_session, "/api/update_settings", "POST",
                            {"settings": empty_session.settings})

        assert status == 200
        assert body["scanResults"]


class TestNearestSourceColor:
    def test_a_typed_colour_is_snapped_onto_the_picture(self, session):
        status, body = call(session, "/api/nearest_source_color", "POST", {"hex": "#CC2020"})
        assert status == 200
        assert body["hex"] == "#C82828"

    def test_something_that_is_not_a_colour_is_refused(self, session):
        assert call(session, "/api/nearest_source_color", "POST", {"hex": "teal"})[0] == 400

    def test_it_needs_a_picture_to_look_in(self, empty_session):
        assert call(empty_session, "/api/nearest_source_color", "POST", {"hex": "#000000"})[0] == 400


class TestExport:
    def test_export_returns_a_downloadable_svg(self, session):
        call(session, "/api/update_settings", "POST", {"settings": session.settings})
        status, body = call(session, "/api/export", "POST")

        assert status == 200
        assert body["svg"].startswith("<?xml")
        assert "<svg " in body["svg"]
        assert body["fileName"] == "sample.palette-trace.svg"

    def test_export_records_provenance_the_same_way_a_written_file_does(self, session):
        call(session, "/api/update_settings", "POST", {"settings": session.settings})
        call(session, "/api/export", "POST")

        generation = session.settings["generation"]
        assert generation["lastSourceHash"] == session.image_source.fingerprint
        assert generation["lastSettingsHash"]

    def test_export_does_not_end_the_session(self, session):
        """Downloading is a checkpoint; the user may adjust and download again."""
        call(session, "/api/update_settings", "POST", {"settings": session.settings})
        call(session, "/api/export", "POST")
        assert session.is_applied is False
        assert session.is_cancelled is False


class TestSampling:
    """§9.3: three sample sizes, and never an arithmetic mean."""

    def test_exact_returns_the_pixel_under_the_crosshair(self, session):
        status, body = call(session, "/api/sample_color", "POST", {"x": 2, "y": 2, "mode": "exact"})
        assert status == 200
        assert body["hex"].upper() == "#C82828"

    def test_the_default_is_the_five_by_five_median(self, session):
        status, body = call(session, "/api/sample_color", "POST", {"x": 2, "y": 2})
        assert body["mode"] == "5x5_median"
        assert body["hex"].upper() == "#C82828"

    def test_out_of_bounds_coordinates_are_rejected(self, session):
        assert call(session, "/api/sample_color", "POST", {"x": 999, "y": 0})[0] == 400

    def test_an_unknown_sample_mode_is_rejected(self, session):
        status, body = call(session, "/api/sample_color", "POST", {"x": 1, "y": 1, "mode": "mean"})
        assert status == 400

    @pytest.mark.parametrize("mode", ["exact", "5x5_median", "15x15_dominant"])
    def test_no_mode_invents_a_colour_that_is_not_in_the_picture(self, mode):
        """
        Sampled on the seam between two flat colours, an arithmetic mean would
        return the purple that lies between red and blue — a colour that
        appears nowhere in the image. Every supported mode must return one of
        the two real colours instead.
        """
        source = two_colour_source(20, 20)
        r, g, b = sample_source_color(source.srgb, 10, 10, mode)
        assert (round(r * 255), round(g * 255), round(b * 255)) in {(200, 40, 40), (40, 40, 200)}

    def test_the_dominant_mode_returns_the_majority_colour_near_an_edge(self):
        """Two pixels into the blue half, a 15×15 window is mostly blue."""
        source = two_colour_source(40, 40)
        r, g, b = sample_source_color(source.srgb, 22, 20, "15x15_dominant")
        assert (round(r * 255), round(g * 255), round(b * 255)) == (40, 40, 200)


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


class TestPreviewScaling:
    """
    §17.4: interaction traces a reduced copy, delivery does not.

    The session fixture's bitmap is well inside the preview budget, so these
    force the question with an explicit budget rather than a large image — the
    rule under test is which endpoints preview, not what the budget is.
    """

    def large_session(self, monkeypatch, session, budget=64):
        from palette_trace.server import api as api_module

        monkeypatch.setattr(api_module, "PREVIEW_BUDGET_PIXELS", budget)
        return session

    def test_a_settings_change_previews(self, session, monkeypatch):
        self.large_session(monkeypatch, session)
        status, body = call(session, "/api/update_settings", "POST",
                            {"settings": session.settings})

        assert status == 200
        assert body["previewScale"] < 1.0
        assert session.pipeline_output_is_preview

    def test_a_small_picture_is_previewed_at_full_resolution(self, session):
        """Below the budget there is nothing to reduce, and §17.4 has no work."""
        status, body = call(session, "/api/update_settings", "POST",
                            {"settings": session.settings})

        assert status == 200
        assert body["previewScale"] == 1.0
        assert not session.pipeline_output_is_preview

    def test_export_retraces_at_full_resolution(self, session, monkeypatch):
        """
        The delivered file is never a preview. A cached preview is geometry from
        a reduced copy of the bitmap: fine to look at, not what was asked for.
        """
        self.large_session(monkeypatch, session)
        call(session, "/api/update_settings", "POST", {"settings": session.settings})
        assert session.pipeline_output_is_preview

        status, body = call(session, "/api/export", "POST")

        assert status == 200
        assert not session.pipeline_output_is_preview
        assert f'width="{session.image_source.intrinsic_width}"' in body["svg"]

    def test_applying_retraces_at_full_resolution(self, session, monkeypatch):
        """Both hosts commit `pipeline_output` straight into a document."""
        self.large_session(monkeypatch, session)
        call(session, "/api/update_settings", "POST", {"settings": session.settings})

        assert call(session, "/api/apply", "POST")[0] == 200
        assert session.is_applied
        assert not session.pipeline_output_is_preview

    def test_applying_leaves_the_settings_agreeing_with_the_committed_file(
        self, session, monkeypatch
    ):
        """
        The sidecar is written from the settings, the SVG from the output. If
        Apply re-resolved the palette at full resolution and did not write it
        back, the two would disagree about the colours.
        """
        self.large_session(monkeypatch, session)
        call(session, "/api/update_settings", "POST", {"settings": session.settings})
        call(session, "/api/apply", "POST")

        committed = [
            ((e.get("output") or {}).get("color") or {}).get("srgb")
            for e in session.pipeline_output["palette_entries"]
        ]
        stored = [
            ((e.get("output") or {}).get("color") or {}).get("srgb")
            for e in session.settings["palette"]["entries"]
        ]
        assert stored == committed

    def test_a_previewed_result_still_describes_the_source_frame(
        self, session, monkeypatch
    ):
        """§17.4 converts thresholds in and geometry out; the frame never moves."""
        self.large_session(monkeypatch, session)
        _, body = call(session, "/api/update_settings", "POST",
                       {"settings": session.settings})

        width = session.image_source.intrinsic_width
        for scan in body["scanResults"]:
            for path in scan["pathDatas"]:
                for token in path.split():
                    if not token.isalpha():
                        assert float(token) <= width + 0.5
