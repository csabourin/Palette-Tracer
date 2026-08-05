"""
Image decoding, sRGB conversion, EXIF orientation, and SHA-256 fingerprinting.
"""

import io
import base64
import hashlib
import numpy as np
from PIL import Image, ImageOps
from palette_trace.errors import ImageSourceError
from palette_trace.color.conversion import srgb_to_oklch

class DecodedImageSource:
    """Encapsulates decoded raster data, dimensions, and content fingerprint."""

    def __init__(self, pil_image: Image.Image, mime_type: str = "image/png"):
        # Apply EXIF orientation
        try:
            pil_image = ImageOps.exif_transpose(pil_image)
        except Exception:
            pass

        self.mime_type = mime_type
        self.intrinsic_width, self.intrinsic_height = pil_image.size

        # Convert to RGBA sRGB numpy array
        rgba_img = pil_image.convert("RGBA")
        self.rgba_data = np.array(rgba_img, dtype=np.uint8)

        # Separate sRGB floats (0..1) and alpha (0..1)
        self.srgb = self.rgba_data[:, :, :3].astype(np.float32) / 255.0
        self.alpha = self.rgba_data[:, :, 3].astype(np.float32) / 255.0

        # Compute precalculated OKLCH representation
        r = self.srgb[:, :, 0]
        g = self.srgb[:, :, 1]
        b = self.srgb[:, :, 2]

        # Vectorized sRGB to OKLCH
        self.oklch = np.zeros((self.intrinsic_height, self.intrinsic_width, 3), dtype=np.float32)
        for y in range(self.intrinsic_height):
            for x in range(self.intrinsic_width):
                self.oklch[y, x] = srgb_to_oklch(r[y, x], g[y, x], b[y, x])

        # Calculate SHA-256 fingerprint
        hasher = hashlib.sha256()
        hasher.update(self.rgba_data.tobytes())
        self.fingerprint = hasher.hexdigest()


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
