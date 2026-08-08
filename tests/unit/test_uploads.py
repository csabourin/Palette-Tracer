"""
Browser-supplied source bitmaps (SPEC §9.1, §9.4.2).
"""

import base64
import io

import numpy as np
import pytest
from PIL import Image

from palette_trace.errors import ImageSourceError
from palette_trace.server.uploads import (
    MAX_UPLOAD_BYTES,
    MAX_WORKING_PIXELS,
    decode_upload,
    display_name,
    parse_data_uri,
)


def png_bytes(width=16, height=16, colour=(200, 40, 40, 255)) -> bytes:
    arr = np.zeros((height, width, 4), dtype=np.uint8)
    arr[:, :] = colour
    buf = io.BytesIO()
    Image.fromarray(arr, mode="RGBA").save(buf, format="PNG")
    return buf.getvalue()


def data_uri(raw: bytes, mime="image/png") -> str:
    return f"data:{mime};base64," + base64.b64encode(raw).decode("ascii")


class TestParseDataUri:
    def test_accepts_a_base64_png(self):
        mime, raw = parse_data_uri(data_uri(png_bytes()))
        assert mime == "image/png"
        assert raw.startswith(b"\x89PNG")

    @pytest.mark.parametrize("value", [
        "",
        "not-a-uri",
        "https://example.com/photo.png",
        "data:image/png,notbase64",
    ])
    def test_rejects_anything_that_is_not_a_base64_data_uri(self, value):
        with pytest.raises(ImageSourceError):
            parse_data_uri(value)

    def test_rejects_a_type_that_is_not_a_traceable_image(self):
        """A source must be a bitmap; SVG and PDF are not decoded as one."""
        with pytest.raises(ImageSourceError, match="not an image format"):
            parse_data_uri(data_uri(b"<svg/>", mime="image/svg+xml"))

    def test_rejects_a_payload_over_the_upload_limit(self):
        """§9.1: accepted payload size is restricted."""
        oversized = "data:image/png;base64," + "A" * (MAX_UPLOAD_BYTES * 2)
        with pytest.raises(ImageSourceError, match="upload limit"):
            parse_data_uri(oversized)

    def test_the_limit_is_checked_before_decoding(self):
        """
        Decoding an oversized payload to discover it is oversized would
        allocate exactly what the limit exists to refuse. The guard therefore
        has to reject deliberately invalid base64 that is merely too long,
        rather than failing on the decode.
        """
        with pytest.raises(ImageSourceError, match="upload limit"):
            parse_data_uri("data:image/png;base64," + "!" * (MAX_UPLOAD_BYTES * 2))


class TestDecodeUpload:
    def test_decodes_to_a_usable_source(self):
        source, notice = decode_upload(data_uri(png_bytes(24, 12)))
        assert (source.intrinsic_width, source.intrinsic_height) == (24, 12)
        assert notice is None
        assert source.fingerprint

    def test_rejects_bytes_that_are_not_an_image(self):
        with pytest.raises(ImageSourceError):
            decode_upload(data_uri(b"this is not a png at all"))

    def test_oversized_pictures_are_resized_and_say_so(self):
        """
        A phone photo runs the full-resolution pipeline for minutes. Tracing a
        downscaled copy is the usable behaviour; doing it silently is not.
        """
        side = int(MAX_WORKING_PIXELS ** 0.5) + 400
        source, notice = decode_upload(data_uri(png_bytes(side, side)))

        assert source.intrinsic_width * source.intrinsic_height <= MAX_WORKING_PIXELS
        assert notice is not None
        assert f"{side} × {side}" in notice
        assert f"{source.intrinsic_width} × {source.intrinsic_height}" in notice

    def test_resizing_preserves_aspect_ratio(self):
        source, _ = decode_upload(data_uri(png_bytes(4000, 1000)))
        ratio = source.intrinsic_width / source.intrinsic_height
        assert ratio == pytest.approx(4.0, rel=0.02)

    def test_the_same_upload_always_resizes_to_the_same_pixels(self):
        """§34.30: identical input must produce identical geometry."""
        raw = data_uri(png_bytes(3000, 2000))
        first, _ = decode_upload(raw)
        second, _ = decode_upload(raw)
        assert first.fingerprint == second.fingerprint


class TestDisplayName:
    @pytest.mark.parametrize("given,expected", [
        ("photo.png", "photo.png"),
        ("/etc/passwd", "passwd"),
        ("..\\..\\windows\\system32\\a.png", "a.png"),
        ("", "image"),
        ("   ", "image"),
    ])
    def test_reduces_to_a_safe_basename(self, given, expected):
        assert display_name(given) == expected

    def test_truncates_absurd_names(self):
        assert len(display_name("x" * 500)) == 120
