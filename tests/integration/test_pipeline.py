"""
End-to-end pipeline integration tests (SPEC §24).
"""

import copy

import numpy as np
import pytest
from PIL import Image

from palette_trace.image_source import DecodedImageSource
from palette_trace.pipeline.controller import PipelineController
from palette_trace.settings import create_default_settings


def two_colour_source(width=30, height=30):
    """Red left half, blue right half, fully opaque."""
    arr = np.zeros((height, width, 4), dtype=np.uint8)
    arr[:, : width // 2] = [255, 0, 0, 255]
    arr[:, width // 2:] = [0, 0, 255, 255]
    return DecodedImageSource(Image.fromarray(arr, mode="RGBA"))


def pinned_entry(entry_id, name, hex_colour, **overrides):
    entry = {
        "id": entry_id,
        "name": name,
        "enabled": True,
        "kind": "pinned",
        "sourceAnchor": {"srgb": hex_colour},
        "output": {"mode": "exact", "color": {"srgb": hex_colour}},
        "assignment": {
            "mode": "reserve_within_reach",
            "overallReach": 25,
            "channels": {"mode": "linked"},
        },
        "role": "primary_fill",
        "traceProfile": {"mode": "inherit"},
    }
    entry.update(overrides)
    return entry


@pytest.fixture
def source():
    return two_colour_source()


@pytest.fixture
def settings():
    config = create_default_settings("test-img-uuid")
    config["scanCount"] = 2
    config["palette"]["entries"] = [
        pinned_entry("e_red", "Red Scan", "#FF0000"),
        pinned_entry("e_blue", "Blue Scan", "#0000FF"),
    ]
    config["palette"]["layerOrder"] = ["e_red", "e_blue"]
    return config


class TestBasicExecution:
    def test_pipeline_produces_one_result_per_scan(self, source, settings):
        output = PipelineController(source, settings).run_pipeline()
        assert len(output["scan_results"]) == 2

    def test_pinned_colour_claims_its_half(self, source, settings):
        output = PipelineController(source, settings).run_pipeline()
        assert output["claims_stats"]["e_red"] > 40.0

    def test_picked_colours_are_exact_output_colours(self, source, settings):
        """Criterion 5."""
        output = PipelineController(source, settings).run_pipeline()
        colours = {s["entryId"]: s["color"] for s in output["scan_results"]}
        assert colours["e_red"] == "#FF0000"
        assert colours["e_blue"] == "#0000FF"

    def test_scans_produce_geometry(self, source, settings):
        output = PipelineController(source, settings).run_pipeline()
        assert all(s["pathDatas"] for s in output["scan_results"])

    def test_every_scan_reports_the_share_it_owns(self, source, settings):
        """
        The interface reports coverage for every colour, not only pinned ones:
        `claims_stats` covers pinned entries alone, so an automatic colour
        would otherwise have to be shown as 0%, which reads as "found nothing".
        """
        output = PipelineController(source, settings).run_pipeline()
        coverage = {s["entryId"]: s["coveragePercent"] for s in output["scan_results"]}

        assert coverage["e_red"] == pytest.approx(50.0, abs=2.0)
        assert coverage["e_blue"] == pytest.approx(50.0, abs=2.0)
        assert sum(coverage.values()) == pytest.approx(100.0, abs=2.0)

    def test_an_automatic_entry_also_reports_coverage(self, source):
        config = create_default_settings("test-img-uuid")
        config["scanCount"] = 2
        output = PipelineController(source, config).run_pipeline()

        assert not output["claims_stats"]          # nothing is pinned
        assert all(s["coveragePercent"] > 0 for s in output["scan_results"])

    def test_backend_provenance_is_reported(self, source, settings):
        """§10.3 requires the backend id and version on the generated group."""
        output = PipelineController(source, settings).run_pipeline()
        assert output["backend"]["id"]
        assert output["backend"]["version"]

    def test_results_are_ordered_bottom_to_top(self, source, settings):
        settings["palette"]["layerOrder"] = ["e_blue", "e_red"]
        output = PipelineController(source, settings).run_pipeline()
        assert [s["entryId"] for s in output["scan_results"]] == ["e_blue", "e_red"]

    def test_identical_inputs_produce_identical_results(self, settings):
        """Criterion 30: determinism across independently constructed runs."""
        first = PipelineController(two_colour_source(), copy.deepcopy(settings)).run_pipeline()
        second = PipelineController(two_colour_source(), copy.deepcopy(settings)).run_pipeline()
        assert [s["pathDatas"] for s in first["scan_results"]] == \
               [s["pathDatas"] for s in second["scan_results"]]
        assert first["claims_stats"] == second["claims_stats"]


class TestGeometryPolicies:
    @pytest.mark.parametrize("policy", [
        "stacked", "stacked_trapped", "exclusive_layers", "separate_operations",
    ])
    def test_every_policy_runs_end_to_end(self, source, settings, policy):
        """Criteria 16-18: all four §20 policies must be reachable."""
        settings["geometry"]["policy"] = policy
        output = PipelineController(source, settings).run_pipeline()
        assert len(output["scan_results"]) == 2

    def test_exclusive_policy_disables_per_scan_offsets(self, source, settings):
        """§20.3: an offset is exactly what breaks exclusive ownership."""
        settings["globalTraceProfile"]["mask"]["offsetPx"] = 3

        settings["geometry"]["policy"] = "exclusive_layers"
        exclusive = PipelineController(source, settings).run_pipeline()

        settings["geometry"]["policy"] = "stacked"
        settings["geometry"]["underlap"]["value"] = 0.0
        stacked = PipelineController(source, settings).run_pipeline()

        assert [s["pathDatas"] for s in exclusive["scan_results"]] != \
               [s["pathDatas"] for s in stacked["scan_results"]]

    def test_separate_operations_validates_layers(self, source, settings):
        settings["geometry"]["policy"] = "separate_operations"
        settings["palette"]["entries"].append(
            pinned_entry("e_absent", "Never matches", "#00FF00"))
        settings["scanCount"] = 3
        output = PipelineController(source, settings).run_pipeline()
        assert any(w["code"] == "empty_operation" for w in output["warnings"])


class TestTraceProfiles:
    def test_per_scan_profiles_change_the_output(self):
        """
        Criteria 13-14: scans can use materially different profiles.

        Uses a source with a small detached speck, because a plain rectangle
        traces identically under every profile — the difference only shows where
        a profile's cleanup thresholds actually bite.
        """
        arr = np.zeros((30, 30, 4), dtype=np.uint8)
        arr[:, :15] = [255, 0, 0, 255]
        arr[:, 15:] = [0, 0, 255, 255]
        arr[2:5, 20:23] = [255, 0, 0, 255]      # 9 px² detached red speck
        source = DecodedImageSource(Image.fromarray(arr, mode="RGBA"))

        settings = create_default_settings("profile-uuid")
        settings["scanCount"] = 2
        settings["palette"]["entries"] = [
            pinned_entry("e_red", "Red", "#FF0000"),
            pinned_entry("e_blue", "Blue", "#0000FF"),
        ]
        settings["palette"]["layerOrder"] = ["e_red", "e_blue"]

        # default keeps regions >= 4 px²; simplified_background requires 64 px².
        baseline = PipelineController(source, settings).run_pipeline()
        settings["palette"]["entries"][0]["traceProfile"] = {
            "mode": "preset", "profileId": "simplified_background",
        }
        overridden = PipelineController(source, settings).run_pipeline()

        before = {s["entryId"]: s["pathDatas"] for s in baseline["scan_results"]}
        after = {s["entryId"]: s["pathDatas"] for s in overridden["scan_results"]}

        assert len(before["e_red"]) > len(after["e_red"]), \
            "the coarser profile should have dropped the speck"
        assert before["e_blue"] == after["e_blue"], "the other scan must be unaffected"

    def test_unsupported_settings_are_reported_not_ignored(self, source, settings):
        """§18: an adapter must map, emulate or report a setting."""
        settings["palette"]["entries"][0]["traceProfile"] = {
            "mode": "override",
            "values": {"vector": {"noSuchEngineSetting": 1}},
        }
        output = PipelineController(source, settings).run_pipeline()
        assert any(w["code"] == "unsupported_trace_setting" for w in output["warnings"])


class TestBackground:
    def test_background_can_be_omitted(self, source, settings):
        """Criterion 12."""
        settings["palette"]["backgroundEntryId"] = "e_blue"
        settings["output"]["backgroundOutput"] = "omit"
        output = PipelineController(source, settings).run_pipeline()
        assert [s["entryId"] for s in output["scan_results"]] == ["e_red"]

    def test_background_can_be_kept(self, source, settings):
        settings["palette"]["backgroundEntryId"] = "e_blue"
        settings["output"]["backgroundOutput"] = "keep_paths"
        output = PipelineController(source, settings).run_pipeline()
        assert len(output["scan_results"]) == 2

    def test_background_can_be_replaced_with_a_rectangle(self, source, settings):
        settings["palette"]["backgroundEntryId"] = "e_blue"
        settings["output"]["backgroundOutput"] = "replace_with_rectangle"
        output = PipelineController(source, settings).run_pipeline()

        background = next(s for s in output["scan_results"] if s["entryId"] == "e_blue")
        assert background["pathDatas"] == [
            f"M 0 0 L {source.intrinsic_width} 0 "
            f"L {source.intrinsic_width} {source.intrinsic_height} "
            f"L 0 {source.intrinsic_height} Z"
        ]

    def test_background_scan_is_marked(self, source, settings):
        settings["palette"]["backgroundEntryId"] = "e_blue"
        output = PipelineController(source, settings).run_pipeline()
        background = next(s for s in output["scan_results"] if s["entryId"] == "e_blue")
        assert background["isBackground"] is True
        assert background["role"] == "background"

    def test_edge_connected_matching_preserves_enclosed_regions(self):
        """§16.1: an island of the backdrop colour is not the backdrop."""
        arr = np.zeros((24, 24, 4), dtype=np.uint8)
        arr[:, :] = [255, 255, 255, 255]        # white field
        arr[6:18, 6:18] = [255, 0, 0, 255]      # red block
        arr[10:14, 10:14] = [255, 255, 255, 255]  # enclosed white island
        source = DecodedImageSource(Image.fromarray(arr, mode="RGBA"))

        settings = create_default_settings("bg-uuid")
        settings["scanCount"] = 2
        settings["palette"]["entries"] = [
            pinned_entry("e_white", "White", "#FFFFFF"),
            pinned_entry("e_red", "Red", "#FF0000"),
        ]
        settings["palette"]["layerOrder"] = ["e_white", "e_red"]
        settings["palette"]["backgroundEntryId"] = "e_white"
        settings["palette"]["backgroundMatching"] = "edge_connected"
        settings["output"]["backgroundOutput"] = "keep_paths"

        output = PipelineController(source, settings).run_pipeline()
        white = next(s for s in output["scan_results"] if s["entryId"] == "e_white")

        # The enclosed island is excluded, so the white scan is no longer the
        # full frame minus the block — it has strictly fewer pixels than the
        # all-matching interpretation would give.
        settings["palette"]["backgroundMatching"] = "all_matching"
        all_matching = PipelineController(source, settings).run_pipeline()
        white_all = next(s for s in all_matching["scan_results"] if s["entryId"] == "e_white")

        assert white["pathDatas"] != white_all["pathDatas"]


class TestDisabledEntries:
    def test_disabled_entries_produce_no_scan(self, source, settings):
        settings["palette"]["entries"][1]["enabled"] = False
        output = PipelineController(source, settings).run_pipeline()
        assert [s["entryId"] for s in output["scan_results"]] == ["e_red"]
