"""
Browser-supplied source bitmaps (SPEC §9.4.2).

The standalone host can be handed an image path on the command line, but that
is not a usable way to start on a phone, and it is not usable at all when the
interface is reached from a device that is not the machine running the server.
This module decodes a bitmap the user chose in the browser instead.

The bytes arrive in the request body and are decoded in memory. Nothing here
writes the upload to disk, so there is no temporary file to leak at session
completion (§9.1) and no filesystem path to expose.
"""

import base64
import binascii
import io

from PIL import Image

from palette_trace.errors import ImageSourceError
from palette_trace.image_source import DecodedImageSource

#: Largest decoded upload accepted, before any resize. Chosen to be comfortably
#: larger than any phone camera JPEG while still bounding what one request can
#: allocate (§9.1 "restrict accepted payload size").
MAX_UPLOAD_BYTES = 32 * 1024 * 1024

#: Above this pixel count the upload is resized before tracing. The pipeline
#: runs every mask operation at full source resolution (§17.4 preview scaling
#: is not implemented), so a 12-megapixel phone photo does not merely take
#: longer — it takes long enough that the interface looks hung. Tracing a
#: downscaled copy and saying so is better than either outcome. The limit is a
#: fixed constant rather than a heuristic so that the same upload always
#: produces the same geometry (§34.30).
MAX_WORKING_PIXELS = 4_000_000

#: Formats that make sense as a trace source. An upload whose type is not on
#: this list is rejected before Pillow sees it, so the decoder is never asked
#: to guess at arbitrary bytes.
ACCEPTED_MIME_TYPES = frozenset({
    "image/png",
    "image/jpeg",
    "image/gif",
    "image/bmp",
    "image/tiff",
    "image/webp",
    "image/x-portable-pixmap",
    "image/x-portable-graymap",
})


def parse_data_uri(data_uri: str) -> tuple[str, bytes]:
    """
    Splits a `data:` URI into its MIME type and decoded bytes.

    Raises `ImageSourceError` for anything that is not a base64 `data:` URI of
    an accepted image type, or whose payload exceeds `MAX_UPLOAD_BYTES`.
    """
    if not isinstance(data_uri, str) or not data_uri.startswith("data:"):
        raise ImageSourceError("The uploaded image was not sent as a data URI.")

    header, separator, payload = data_uri.partition(",")
    if not separator:
        raise ImageSourceError("The uploaded image data URI is malformed.")

    if ";base64" not in header:
        raise ImageSourceError("The uploaded image must be base64-encoded.")

    mime_type = header[len("data:"):].split(";")[0].strip().lower()
    if mime_type not in ACCEPTED_MIME_TYPES:
        raise ImageSourceError(
            f"{mime_type or 'That file type'} is not an image format Palette Trace can trace."
        )

    # Check the encoded length first: decoding to find out it was too big would
    # mean allocating the very thing the limit exists to prevent.
    if len(payload) > MAX_UPLOAD_BYTES // 3 * 4 + 4:
        raise ImageSourceError("That image is larger than the 32 MB upload limit.")

    try:
        raw = base64.b64decode(payload, validate=True)
    except (binascii.Error, ValueError) as exc:
        raise ImageSourceError(f"The uploaded image could not be decoded: {exc}") from exc

    if len(raw) > MAX_UPLOAD_BYTES:
        raise ImageSourceError("That image is larger than the 32 MB upload limit.")

    return mime_type, raw


def decode_upload(data_uri: str) -> tuple[DecodedImageSource, str | None]:
    """
    Decodes a browser-supplied bitmap sent as a base64 `data:` URI.

    Returns the decoded source and, when the image had to be resized to stay
    traceable, a human-readable notice describing what happened. The caller is
    expected to show that notice — silently tracing something other than what
    the user handed over would be a lie about the output's resolution.
    """
    mime_type, raw = parse_data_uri(data_uri)
    return decode_bytes(raw, mime_type)


def decode_bytes(raw: bytes, mime_type: str) -> tuple[DecodedImageSource, str | None]:
    """
    Decodes a browser-supplied bitmap sent as raw bytes.

    The `data:` URI form costs a third again in transfer and a full base64 pass
    on both ends, which on a phone photograph is most of the wait before the
    picture appears. Posting the file's own bytes skips all of it; this is the
    same decode, without the envelope.
    """
    mime_type = (mime_type or "").split(";")[0].strip().lower()
    if mime_type not in ACCEPTED_MIME_TYPES:
        raise ImageSourceError(
            f"{mime_type or 'That file type'} is not an image format Palette Trace can trace."
        )

    if len(raw) > MAX_UPLOAD_BYTES:
        raise ImageSourceError("That image is larger than the 32 MB upload limit.")

    try:
        image = Image.open(io.BytesIO(raw))
        image.load()
    except (OSError, ValueError) as exc:
        raise ImageSourceError(f"That file could not be read as an image: {exc}") from exc

    notice = None
    original_width, original_height = image.size
    if original_width * original_height > MAX_WORKING_PIXELS:
        image = _resize_within_pixel_budget(image)
        notice = (
            f"{original_width} × {original_height} is large enough to make tracing very slow, "
            f"so it was resized to {image.width} × {image.height}."
        )

    return DecodedImageSource(image, mime_type=mime_type), notice


def _resize_within_pixel_budget(image: Image.Image) -> Image.Image:
    """
    Scales `image` down to at most `MAX_WORKING_PIXELS`, preserving aspect ratio.

    The scale factor is derived from the pixel budget rather than being chosen
    per-image, and LANCZOS is named explicitly, so the same upload resizes to
    the same pixels on every run and on every Pillow default change.
    """
    width, height = image.size
    scale = (MAX_WORKING_PIXELS / float(width * height)) ** 0.5
    target = (max(1, int(width * scale)), max(1, int(height * scale)))
    return image.resize(target, Image.LANCZOS)


def display_name(file_name: str) -> str:
    """
    Reduces a browser-supplied filename to something safe to echo back.

    The browser sends only a basename, but it is still user-controlled text
    that ends up in the interface and in the suggested download name, so path
    separators and control characters are stripped here rather than trusted.
    """
    name = (file_name or "").replace("\\", "/").rsplit("/", 1)[-1]
    name = "".join(char for char in name if char.isprintable() and char not in '<>:"|?*')
    return name.strip()[:120] or "image"
