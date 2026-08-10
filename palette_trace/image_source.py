"""
Image decoding, sRGB conversion, EXIF orientation, and SHA-256 fingerprinting.
"""

import base64
import contextlib
import hashlib
import io

import numpy as np
from PIL import Image, ImageOps

from palette_trace.color.conversion import srgb_array_to_oklch
from palette_trace.errors import ImageSourceError


class DecodedImageSource:
    """Encapsulates decoded raster data, dimensions, and content fingerprint."""

    def __init__(self, pil_image: Image.Image, mime_type: str = "image/png"):
        # Malformed EXIF is common in the wild and is never a reason to refuse
        # a picture that decoded: an un-rotated trace beats no trace at all.
        with contextlib.suppress(Exception):
            pil_image = ImageOps.exif_transpose(pil_image)

        self.mime_type = mime_type
        self.intrinsic_width, self.intrinsic_height = pil_image.size

        # Convert to RGBA sRGB numpy array
        rgba_img = pil_image.convert("RGBA")
        self.rgba_data = np.array(rgba_img, dtype=np.uint8)

        # Separate sRGB floats (0..1) and alpha (0..1)
        self.srgb = self.rgba_data[:, :, :3].astype(np.float32) / 255.0
        self.alpha = self.rgba_data[:, :, 3].astype(np.float32) / 255.0

        self._oklch = None
        self._working_views = {}

        # Calculate SHA-256 fingerprint
        hasher = hashlib.sha256()
        hasher.update(self.rgba_data.tobytes())
        self.fingerprint = hasher.hexdigest()

    @property
    def oklch(self) -> np.ndarray:
        """
        The OKLCH working representation (§21.1), computed on first use.

        Vectorised — the per-pixel Python loop this replaces cost one
        interpreter call per pixel. It is deferred because only the pipeline
        needs it: decoding a bitmap so it can be put on screen should not wait
        on a colour-space conversion of every pixel that nothing has asked for
        yet.

        `self.srgb` is float32, and the conversion now works in the precision it
        is handed, so the result already has the stored dtype — the cast that
        used to sit here was undoing a widening that no longer happens.
        """
        if self._oklch is None:
            self._oklch = srgb_array_to_oklch(self.srgb)
        return self._oklch

    def working_view(self, scale: float) -> "DecodedImageSource":
        """
        This source reduced to `scale`, for tracing a preview (§17.4).

        Returns `self` at 1.0 and never enlarges, so the full-resolution path
        costs nothing and stays literally the same object. Views are cached per
        scale exactly as `oklch` is: an interactive session previews at one
        factor over and over, and the OKLCH conversion of the reduced copy — the
        expensive part — should happen once, not once per settings change.

        LANCZOS is named rather than left to Pillow's default so that the same
        image and the same factor always produce the same pixels (§34.30), which
        is the same reason `uploads._resize_within_pixel_budget` names it.
        """
        if scale >= 1.0:
            return self

        target = (
            max(1, int(self.intrinsic_width * scale)),
            max(1, int(self.intrinsic_height * scale)),
        )
        if target == (self.intrinsic_width, self.intrinsic_height):
            return self

        view = self._working_views.get(target)
        if view is None:
            reduced = Image.fromarray(self.rgba_data, mode="RGBA").resize(
                target, Image.LANCZOS
            )
            view = DecodedImageSource(reduced, self.mime_type)
            self._working_views[target] = view
        return view


def load_image_source(href_or_data: str, is_data_uri: bool) -> DecodedImageSource:
    """Loads and decodes image from local path or data URI."""
    try:
        if is_data_uri:
            # Parse data URI: data:image/png;base64,...
            header, base64_str = href_or_data.split(",", 1)
            mime_type = header.split(";")[0].replace("data:", "")
            raw_bytes = base64.b64decode(base64_str)
            pil_img = Image.open(io.BytesIO(raw_bytes))
        else:
            mime_type = "image/png"
            pil_img = Image.open(href_or_data)

        return DecodedImageSource(pil_img, mime_type)
    except Exception as e:
        raise ImageSourceError(f"Failed to decode raster image: {e}")
