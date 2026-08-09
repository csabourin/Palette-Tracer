"""
Every colour Palette Trace reports is a colour the picture contains (§34.5).

A K-Means centroid is an average and a per-channel median is a construction:
both routinely name a colour that appears nowhere in the source. That is the
one thing a colour picker must never do, so the automatic palette and every
sampling mode are pinned here against the pixels they came from.
"""

import numpy as np
import pytest
from PIL import Image

from palette_trace.color.conversion import srgb_to_hex
from palette_trace.color.histogram import snap_to_source_colors, unique_source_colors
from palette_trace.image_source import DecodedImageSource
from palette_trace.pipeline.controller import PipelineController
from palette_trace.server.api import nearest_source_color, sample_source_color
from palette_trace.settings import reset_settings_to_destination_defaults


def hexes_present(pixels: np.ndarray) -> set:
    """Every colour that literally occurs in an (h, w, 3|4) uint8 array."""
    flat = pixels[..., :3].reshape(-1, 3)
    return {srgb_to_hex(*(channel / 255.0 for channel in row)) for row in np.unique(flat, axis=0)}


class TestSampling:
    def gradient(self):
        """A horizontal ramp: every 5×5 window straddles several values."""
        row = np.linspace(0, 255, 40).astype(np.uint8)
        return np.stack([np.tile(row, (40, 1))] * 3, axis=-1).astype(np.float32) / 255.0

    @pytest.mark.parametrize("mode", ["exact", "5x5_median", "15x15_dominant"])
    def test_every_mode_returns_a_colour_the_picture_has(self, mode):
        srgb = self.gradient()
        present = hexes_present((srgb * 255).astype(np.uint8))

        for x, y in ((0, 0), (7, 12), (20, 20), (39, 39)):
            sampled = srgb_to_hex(*sample_source_color(srgb, x, y, mode))
            assert sampled in present, f"{mode} at {x},{y} invented {sampled}"

    def test_an_edge_is_not_averaged_across(self):
        """The classic failure: a black/white edge sampling as mid grey."""
        srgb = np.zeros((20, 20, 3), dtype=np.float32)
        srgb[:, 10:] = 1.0

        for x in (9, 10, 11):
            sampled = srgb_to_hex(*sample_source_color(srgb, x, 10, "5x5_median"))
            assert sampled in ("#000000", "#FFFFFF")

    def test_the_same_window_always_answers_the_same_way(self):
        srgb = self.gradient()
        first = sample_source_color(srgb, 15, 15, "15x15_dominant")
        assert all(sample_source_color(srgb, 15, 15, "15x15_dominant") == first
                   for _ in range(5))


class TestAutomaticPalette:
    def photograph(self):
        """
        Two flat colours with a soft seam between them. The seam gives K-Means
        every opportunity to settle on a blend that is in neither block.
        """
        pixels = np.zeros((40, 40, 4), dtype=np.uint8)
        pixels[..., 3] = 255
        pixels[:, :18] = (210, 30, 40, 255)
        pixels[:, 22:] = (25, 60, 190, 255)
        ramp = np.linspace(0, 1, 4)[None, :, None]
        pixels[:, 18:22, :3] = (
            (1 - ramp) * np.array([210, 30, 40]) + ramp * np.array([25, 60, 190])
        ).astype(np.uint8)
        return pixels

    def test_every_automatic_colour_exists_in_the_picture(self):
        pixels = self.photograph()
        source = DecodedImageSource(Image.fromarray(pixels, mode="RGBA"))
        settings = reset_settings_to_destination_defaults("image-uuid", "illustration")

        output = PipelineController(source, settings).run_pipeline()
        present = hexes_present(pixels)

        for entry in output["palette_entries"]:
            assert entry["output"]["color"]["srgb"] in present

    def test_snapping_picks_the_nearest_real_colour(self):
        pixels = np.zeros((4, 4, 3), dtype=np.uint8)
        pixels[:, :2] = (255, 0, 0)
        pixels[:, 2:] = (0, 0, 255)
        rgb255, counts, oklab = unique_source_colors(
            pixels.astype(np.float32) / 255.0, np.ones((4, 4), dtype=bool)
        )

        # A centroid halfway between the two, nudged towards red.
        centre = tuple(oklab[np.argmax(rgb255[:, 0])] * 0.6 + oklab[np.argmax(rgb255[:, 2])] * 0.4)
        assert snap_to_source_colors([centre], rgb255, counts, oklab) == ["#FF0000"]

    def test_a_flat_picture_keeps_its_exact_colour(self):
        pixels = np.full((16, 16, 4), 255, dtype=np.uint8)
        pixels[..., :3] = (137, 42, 219)
        source = DecodedImageSource(Image.fromarray(pixels, mode="RGBA"))
        settings = reset_settings_to_destination_defaults("image-uuid", "illustration")

        output = PipelineController(source, settings).run_pipeline()
        drawn = {s["color"] for s in output["scan_results"] if s["pathDatas"]}
        assert drawn == {"#892ADB"}


class TestNearestSourceColor:
    def test_a_typed_colour_snaps_onto_the_picture(self):
        pixels = np.zeros((8, 8, 3), dtype=np.float32)
        pixels[:, :4] = np.array([0.8, 0.1, 0.1])
        pixels[:, 4:] = np.array([0.1, 0.1, 0.8])

        snapped = srgb_to_hex(*nearest_source_color(pixels, 0.75, 0.15, 0.05))
        assert snapped == srgb_to_hex(0.8, 0.1, 0.1)

    def test_a_colour_already_in_the_picture_is_left_alone(self):
        pixels = np.full((8, 8, 3), 0.25, dtype=np.float32)
        assert nearest_source_color(pixels, 0.25, 0.25, 0.25) == pytest.approx(
            (0.25, 0.25, 0.25), abs=1 / 255.0
        )
