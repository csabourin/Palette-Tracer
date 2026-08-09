"""
Backdrop colour appearing in the foreground (SPEC §16.1).

An `edge_connected` backdrop deliberately leaves enclosed patches of its own
colour out of the backdrop — the white of an eye when the backdrop is white.
Those patches still have to end up somewhere. A patch surrounded by one colour
is a hole in that colour; a patch that a hole cannot express gets a layer of its
own, in front, in the backdrop's colour.
"""

import numpy as np
import pytest
from PIL import Image

from palette_trace.color.background import (
    ISLAND_HOLE,
    ISLAND_IGNORE,
    ISLAND_LAYER,
    classify_background_islands,
)
from palette_trace.image_source import DecodedImageSource
from palette_trace.pipeline.controller import PipelineController
from palette_trace.settings import reset_settings_to_destination_defaults


def scene():
    """
    A white backdrop, a blue block beside a green one, and two white patches:
    one enclosed by blue alone, one straddling the blue/green seam.
    """
    pixels = np.full((60, 60, 4), 255, dtype=np.uint8)
    pixels[10:50, 10:30] = (20, 20, 200, 255)
    pixels[10:50, 30:50] = (20, 200, 20, 255)
    pixels[20:26, 15:21] = (255, 255, 255, 255)     # enclosed by blue
    pixels[34:40, 25:36] = (255, 255, 255, 255)     # blue on one side, green on the other
    return pixels


def pinned_settings(policy=None):
    settings = reset_settings_to_destination_defaults("image-uuid", "illustration")
    settings["scanCount"] = 3
    entries = settings["palette"]["entries"][:3]
    settings["palette"]["entries"] = entries
    settings["palette"]["layerOrder"] = [entry["id"] for entry in entries]

    for entry, hex_value in zip(entries, ("#FFFFFF", "#1414C8", "#14C814")):
        entry["kind"] = "pinned"
        entry["sourceAnchor"] = {"srgb": hex_value}
        entry["output"] = {"mode": "exact", "color": {"srgb": hex_value}}
        entry["assignment"] = {
            "mode": "reserve_within_reach",
            "overallReach": 25,
            "channels": {"mode": "linked"},
        }

    settings["palette"]["backgroundEntryId"] = entries[0]["id"]
    settings["palette"]["backgroundMatching"] = "edge_connected"
    if policy is not None:
        settings["palette"]["backgroundForegroundPolicy"] = policy
    return settings


def run(policy=None):
    source = DecodedImageSource(Image.fromarray(scene(), mode="RGBA"))
    settings = pinned_settings(policy)
    return PipelineController(source, settings).run_pipeline(), settings


class TestClassification:
    """The decision itself, away from the pipeline."""

    def labelled(self):
        # Scan 1 owns the left half, scan 2 the right; one island sits inside
        # scan 1, another straddles the seam between them.
        labels = np.zeros((20, 20), dtype=np.uint8)
        labels[:, :10] = 1
        labels[:, 10:] = 2

        islands = np.zeros((20, 20), dtype=bool)
        islands[3:6, 3:6] = True        # inside scan 1
        islands[14:17, 8:13] = True     # across both
        labels[islands] = 0
        return islands, labels

    def test_a_patch_inside_one_colour_becomes_a_hole_in_it(self):
        islands, labels = self.labelled()
        result = classify_background_islands(islands, labels)

        assert result.hole_count == 1
        assert 1 in result.holes
        assert result.holes[1][4, 4]

    def test_a_patch_across_two_colours_gets_its_own_layer(self):
        islands, labels = self.labelled()
        result = classify_background_islands(islands, labels)

        assert result.layer_count == 1
        assert result.layer_mask[15, 10]
        assert not result.layer_mask[4, 4]

    def test_a_patch_running_off_the_frame_cannot_be_a_hole(self):
        labels = np.ones((20, 20), dtype=np.uint8)
        islands = np.zeros((20, 20), dtype=bool)
        islands[0:4, 0:4] = True
        labels[islands] = 0

        result = classify_background_islands(islands, labels)
        assert result.hole_count == 0
        assert result.layer_count == 1

    def test_a_patch_bordering_unassigned_pixels_cannot_be_a_hole(self):
        """Nothing is drawn there, so subtracting the patch would show nothing."""
        labels = np.ones((20, 20), dtype=np.uint8)
        labels[10:, :] = 0
        islands = np.zeros((20, 20), dtype=bool)
        islands[8:12, 8:12] = True
        labels[islands] = 0

        result = classify_background_islands(islands, labels)
        assert result.hole_count == 0
        assert result.layer_count == 1

    @pytest.mark.parametrize("policy,holes,layers", [
        (ISLAND_HOLE, 1, 0),
        (ISLAND_LAYER, 0, 2),
        (ISLAND_IGNORE, 0, 0),
    ])
    def test_the_policy_is_respected(self, policy, holes, layers):
        islands, labels = self.labelled()
        result = classify_background_islands(islands, labels, policy)
        assert (result.hole_count, result.layer_count) == (holes, layers)

    def test_nothing_in_the_foreground_is_reported_as_nothing(self):
        result = classify_background_islands(
            np.zeros((10, 10), dtype=bool), np.ones((10, 10), dtype=np.uint8)
        )
        assert result.total == 0
        assert result.summary() == ""


class TestThroughThePipeline:
    def test_the_enclosing_colour_is_traced_with_a_hole(self):
        """
        Two subpaths where an unbroken block would have one: the outline of the
        block, and the outline of the patch inside it.
        """
        output, settings = run()
        blue = next(s for s in output["scan_results"] if s["color"] == "#1414C8")
        green = next(s for s in output["scan_results"] if s["color"] == "#14C814")

        assert len(blue["pathDatas"]) == 2
        assert len(green["pathDatas"]) == 1

    def test_the_straddling_patch_becomes_a_layer_of_its_own(self):
        output, settings = run()
        derived = [s for s in output["scan_results"] if s["derived"]]

        assert len(derived) == 1
        assert derived[0]["color"] == "#FFFFFF"
        assert derived[0]["pathDatas"]

    def test_the_extra_layer_is_drawn_in_front_of_everything(self):
        output, _ = run()
        assert output["scan_results"][-1]["derived"] is True

    def test_the_extra_layer_is_not_written_into_the_palette(self):
        """It follows from the picture; there is nothing for a user to edit."""
        output, settings = run()
        assert len(output["palette_entries"]) == 3
        assert all("::foreground" not in e["id"] for e in output["palette_entries"])

    def test_what_happened_is_reported(self):
        output, _ = run()
        messages = [w["message"] for w in output["warnings"]
                    if w["code"] == "background_in_foreground"]
        assert messages == [
            "2 patches of the backdrop colour sit in the foreground — "
            "1 became a hole, 1 got a layer of its own."
        ]

    def test_ignoring_them_leaves_the_previous_behaviour(self):
        output, _ = run(ISLAND_IGNORE)
        assert not any(s["derived"] for s in output["scan_results"])
        assert not any(w["code"] == "background_in_foreground" for w in output["warnings"])


class TestBorderShare:
    def test_the_backdrop_owns_the_frame(self):
        output, _ = run()
        backdrop = next(s for s in output["scan_results"] if s["isBackground"])
        blue = next(s for s in output["scan_results"] if s["color"] == "#1414C8")

        assert backdrop["borderSharePercent"] == pytest.approx(100.0)
        assert blue["borderSharePercent"] == 0.0
