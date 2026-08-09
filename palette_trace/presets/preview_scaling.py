"""
Preview scaling for pixel-denominated settings (SPEC §17.4).

A preview traces a smaller copy of the bitmap so that changing a setting
answers quickly. Every threshold expressed in pixels then means something
different: §17.4 requires linear dimensions to scale by the preview scale
factor, area dimensions by its square, and forbids applying a full-resolution
16-pixel-area threshold as 16 preview pixels. Left unconverted, a preview would
delete detail the delivered file keeps — an aperçu that lies about the result is
worse than a slow one.

The conversion is driven by the field-name suffix rather than by a list of
field names. The seven settings concerned already say which they are:

* ``…Px``  — a linear dimension: ``closeGapsRadiusPx``, ``minimumFeatureWidthPx``,
  ``offsetPx``, ``smoothingRadiusPx``
* ``…Px2`` — an area:            ``fillHolesAreaPx2``, ``minimumPathAreaPx2``,
  ``minimumRegionAreaPx2``

A field added later is converted by construction instead of by remembering to
add it here, which is the failure this module exists to make impossible.
"""

import copy

#: Checked before the linear suffix — `…Px2` also ends in `…Px`'s letters if
#: the test is written the other way round, and the area would silently scale
#: linearly. That is the one ordering bug this whole approach can have.
AREA_SUFFIX = "Px2"
LINEAR_SUFFIX = "Px"

#: Sections of a resolved trace profile that hold pixel-denominated settings.
_SCALED_SECTIONS = ("mask", "vector")

#: How much bitmap a preview is allowed to trace. Modelled on
#: `uploads.MAX_WORKING_PIXELS`: a fixed constant rather than a per-image
#: heuristic, so the same picture always previews at the same scale (§34.30).
#:
#: Set from measurement, not from taste. The pipeline's cost is close to linear
#: in pixel count, and a settings change on four megapixels took about three
#: seconds; a quarter of that is the difference between a control that responds
#: and one the user waits on — and, on a hosted deployment, between answering
#: the reverse proxy and being replaced by its error page.
PREVIEW_BUDGET_PIXELS = 1_000_000


def pixel_exponent(field_name: str) -> int:
    """
    How a field's units scale with the preview factor: 2, 1 or 0.

    `0` means the field is not expressed in pixels — a ratio, a flag, an id —
    and must be passed through untouched.
    """
    if field_name.endswith(AREA_SUFFIX):
        return 2
    if field_name.endswith(LINEAR_SUFFIX):
        return 1
    return 0


def scale_value(value, scale: float, exponent: int):
    """
    Converts one measurement to preview units, leaving non-numbers alone.

    Booleans are excluded deliberately: `bool` is a subclass of `int` in Python,
    so `preserveThinFeatures` would otherwise become `0.25` on a half-scale
    preview and read as False.
    """
    if exponent == 0 or isinstance(value, bool) or not isinstance(value, (int, float)):
        return value
    return value * (scale ** exponent)


def scale_profile(profile: dict, scale: float) -> dict:
    """
    Returns a copy of a resolved trace profile expressed in preview pixels.

    Values stay fractional. Rounding a scaled threshold up would remove detail
    that full resolution keeps, which is exactly what §17.4 forbids, and the
    consumers — `remove_small_speckles`, `fill_small_holes`, `apply_mask_offset`,
    `preserve_thin_features` — all take a float area or radius already.

    A factor of exactly 1.0 hands the profile straight back, as the other two
    conversions here do: the full-resolution path is the delivered one and
    should not pay a deep copy per entry per run to be told nothing changed.
    """
    if scale == 1.0:
        return profile if profile else {}

    scaled = copy.deepcopy(profile) if profile else {}
    for section in _SCALED_SECTIONS:
        values = scaled.get(section)
        if not isinstance(values, dict):
            continue
        for key, value in values.items():
            values[key] = scale_value(value, scale, pixel_exponent(key))

    return scaled


def scale_measurement(measurement: dict, scale: float) -> dict:
    """
    Converts a `{value, unit}` geometry measurement to preview units.

    Underlap and trapping (§20) are linear distances applied to the mask, so a
    preview that left them at their source-pixel value would trap twice as far
    at half scale. They carry their unit in a field rather than in their name,
    so the suffix rule above cannot see them — but `mm` converts to source
    pixels downstream, which means both units scale the same way here.
    """
    if not isinstance(measurement, dict) or scale == 1.0:
        return measurement
    value = measurement.get("value")
    if isinstance(value, bool) or not isinstance(value, (int, float)):
        return measurement
    return {**measurement, "value": value * scale}


def preview_scale_for(width: int, height: int, budget_pixels: int) -> float:
    """
    The factor to trace a preview of a `width` x `height` bitmap at.

    Derived from a pixel budget on the model of `uploads.MAX_WORKING_PIXELS`, so
    the same image always previews at the same scale (§34.30) rather than at
    whatever a per-image heuristic decided. Never enlarges: a picture already
    inside the budget is previewed at full resolution, where the preview and the
    result are the same thing and §17.4 has nothing to convert.
    """
    pixels = int(width) * int(height)
    if pixels <= 0 or budget_pixels <= 0 or pixels <= budget_pixels:
        return 1.0
    return (budget_pixels / float(pixels)) ** 0.5
