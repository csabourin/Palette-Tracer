"""
Preview scaling end to end (SPEC §17.4).

`test_masks.py::TestPreviewThresholdScaling` covers the arithmetic §17.4
mandates. This covers the machinery built on top of it: the reduced working
view, the geometry conversion back into source pixels, and the rule that
decides which requests preview and which do not.
"""

import numpy as np
import pytest
from PIL import Image

from palette_trace.image_source import DecodedImageSource
from palette_trace.pipeline.controller import PipelineController
from palette_trace.presets.preview_scaling import (
    PREVIEW_BUDGET_PIXELS,
    preview_scale_for,
    scale_measurement,
)
from palette_trace.settings import create_default_settings
from palette_trace.tracing.normalization import scale_svg_path_data


def source(width=64, height=48) -> DecodedImageSource:
    """A four-colour bitmap in quadrants, large enough to trace."""
    pixels = np.zeros((height, width, 4), dtype=np.uint8)
    pixels[..., 3] = 255
    half_h, half_w = height // 2, width // 2
    pixels[:half_h, :half_w, :3] = (220, 40, 40)
    pixels[:half_h, half_w:, :3] = (40, 180, 90)
    pixels[half_h:, :half_w, :3] = (40, 80, 220)
    pixels[half_h:, half_w:, :3] = (240, 230, 60)
    return DecodedImageSource(Image.fromarray(pixels, mode="RGBA"), "image/png")


class TestPreviewScaleFor:
    def test_an_image_inside_the_budget_is_not_reduced(self):
        """Never enlarge, and never reduce what does not need it."""
        assert preview_scale_for(100, 100, 1_000_000) == 1.0

    def test_an_image_over_the_budget_is_reduced_to_it(self):
        scale = preview_scale_for(4000, 2000, 1_000_000)
        assert scale == pytest.approx(0.3535, abs=1e-4)
        assert (4000 * scale) * (2000 * scale) == pytest.approx(1_000_000)

    def test_the_same_image_always_gets_the_same_factor(self):
        """§34.30: derived from a fixed budget, not chosen per image."""
        assert preview_scale_for(3000, 2000, PREVIEW_BUDGET_PIXELS) == \
            preview_scale_for(3000, 2000, PREVIEW_BUDGET_PIXELS)

    @pytest.mark.parametrize("width,height", [(0, 100), (100, 0)])
    def test_a_degenerate_size_does_not_divide_by_zero(self, width, height):
        assert preview_scale_for(width, height, 1_000_000) == 1.0


class TestWorkingView:
    def test_full_scale_returns_the_same_object(self):
        """The delivered path must not pay for a feature it does not use."""
        original = source()
        assert original.working_view(1.0) is original

    def test_a_reduced_view_has_reduced_dimensions(self):
        view = source(64, 48).working_view(0.5)
        assert (view.intrinsic_width, view.intrinsic_height) == (32, 24)
        assert view.oklch.shape == (24, 32, 3)

    def test_the_view_is_cached_per_factor(self):
        """
        The OKLCH conversion of the reduced copy is the expensive part, and an
        interactive session asks for the same factor over and over.
        """
        original = source()
        assert original.working_view(0.5) is original.working_view(0.5)
        assert original.working_view(0.5) is not original.working_view(0.25)

    def test_it_never_enlarges(self):
        original = source()
        assert original.working_view(2.0) is original


class TestGeometryComesBackInSourcePixels:
    def test_a_preview_reports_the_same_frame_as_the_source(self):
        original = source(64, 48)
        preview = PipelineController(original, create_default_settings("u"), 0.5)

        assert preview.is_preview
        assert preview.source.intrinsic_width == 32
        # The controller traced 32x24 but must answer about 64x48.
        assert preview.requested_source.intrinsic_width == 64

    def test_preview_geometry_lands_inside_the_source_frame(self):
        """
        Not "close to the full-resolution geometry" — a reduced trace is a
        different trace. What must hold is that the coordinates describe the
        source's frame and not a quarter of it in the corner, which is what
        forgetting the conversion looks like.
        """
        original = source(64, 48)
        output = PipelineController(original, create_default_settings("u"), 0.5).run_pipeline()

        numbers = []
        for scan in output["scan_results"]:
            for path in scan["pathDatas"]:
                numbers += [float(t) for t in path.split() if not t.isalpha()]

        assert numbers, "the preview produced no geometry to check"
        assert max(numbers) > 40, "geometry never left the reduced frame"
        assert max(numbers) <= 64.5

    def test_the_full_resolution_run_is_not_a_preview(self):
        controller = PipelineController(source(), create_default_settings("u"))
        assert not controller.is_preview
        assert controller.effective_scale == 1.0
        assert controller.source is controller.requested_source


class TestScaleSvgPathData:
    def test_every_coordinate_scales(self):
        assert scale_svg_path_data("M 10 20 L 30 40 Z", 2.0) == "M 20 40 L 60 80 Z"

    def test_a_factor_of_one_returns_the_path_untouched(self):
        """A full-resolution trace must not be reformatted for nothing."""
        path = "M 1.000 2.000 C 3.500 4.500, 5.000 6.000, 7.000 8.000 Z"
        assert scale_svg_path_data(path, 1.0) is path

    def test_curves_and_relative_commands_scale(self):
        """A displacement is a length too, so `c` scales exactly like `C`."""
        assert scale_svg_path_data("m 4 4 c 2 2, 4 4, 6 6", 0.5) == \
            "m 2 2 c 1 1 2 2 3 3"

    def test_single_axis_commands_scale(self):
        assert scale_svg_path_data("M 0 0 H 10 V 20", 3.0) == "M 0 0 H 30 V 60"

    def test_an_arc_scales_its_radii_and_endpoint_but_not_its_angle_or_flags(self):
        """
        `A rx ry x-axis-rotation large-arc sweep x y`. Scaling the rotation or
        either flag turns a quarter circle into something else — the flags are
        booleans, and a `1` that became `0.5` is not even valid path data.
        """
        assert scale_svg_path_data("M 0 0 A 10 20 45 1 0 30 40", 0.5) == \
            "M 0 0 A 5 10 45 1 0 15 20"

    def test_repeated_arc_parameter_sets_stay_aligned(self):
        """SVG lets one `A` carry several arcs; the flags repeat every seven."""
        assert scale_svg_path_data("A 10 10 0 1 0 20 20 10 10 0 0 1 40 40", 0.5) == \
            "A 5 5 0 1 0 10 10 5 5 0 0 1 20 20"

    def test_an_empty_path_stays_empty(self):
        assert scale_svg_path_data("", 0.5) == ""


class TestScaleMeasurement:
    def test_a_geometry_measurement_scales_and_keeps_its_unit(self):
        """
        Trapping and underlap (§20) are linear distances applied to the mask.
        `mm` converts to source pixels downstream, so both units scale here.
        """
        assert scale_measurement({"value": 2.0, "unit": "mm"}, 0.5) == \
            {"value": 1.0, "unit": "mm"}

    def test_a_full_scale_run_leaves_it_alone(self):
        measurement = {"value": 2.0, "unit": "source_px"}
        assert scale_measurement(measurement, 1.0) is measurement

    def test_a_missing_or_non_numeric_value_is_passed_through(self):
        assert scale_measurement({"unit": "mm"}, 0.5) == {"unit": "mm"}
        assert scale_measurement(None, 0.5) is None
