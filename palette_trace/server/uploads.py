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

#: Above this pixel count the upload is resized before tracing.
#:
#: This used to stand on interaction cost: every mask operation ran at full
#: source resolution, so a 12-megapixel phone photo made each settings change
#: slow enough that the interface looked hung. §17.4 preview scaling removed
#: that reason — interaction now traces a copy inside
#: `presets.preview_scaling.PREVIEW_BUDGET_PIXELS` whatever the source is.
#:
#: What the cap now stands on is the other half, which preview scaling does not
#: touch: committing and exporting trace the source itself, and the decoded
#: source stays resident for the whole session. Measured at this limit, the
#: decoded arrays are 128 MB (16 rgba + 48 sRGB + 16 alpha + 48 OKLCH) and a
#: session that loads, previews and exports peaks at about 466 MB of RSS. Both
#: figures scale linearly with pixel count, so the ceiling is now the memory of
#: the smallest machine this is expected to run on rather than the clock —
#: raising it is a measurement on that machine, not a judgement call.
#:
#: The limit is a fixed constant rather than a heuristic so that the same upload
#: always produces the same geometry (§34.30).
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
        raise ImageSourceError(
            "No picture arrived with that request. If this keeps happening, "
            "reload the page — the interface and the server it is talking to "
            "may be from different versions."
        )

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


#: Leading bytes that identify a format, for when the declared type is missing
#: or wrong. A `Content-Type` survives a browser, an XHR and a socket, but not
#: reliably a reverse proxy — and the bytes themselves always say what they are.
_MAGIC_NUMBERS = (
    (b"\x89PNG\r\n\x1a\n", "image/png"),
    (b"\xff\xd8\xff", "image/jpeg"),
    (b"GIF87a", "image/gif"),
    (b"GIF89a", "image/gif"),
    (b"BM", "image/bmp"),
    (b"II*\x00", "image/tiff"),
    (b"MM\x00*", "image/tiff"),
    (b"P6", "image/x-portable-pixmap"),
    (b"P5", "image/x-portable-graymap"),
)


def sniff_image_type(raw: bytes) -> str | None:
    """The format `raw` actually is, by its leading bytes, or None."""
    for signature, mime_type in _MAGIC_NUMBERS:
        if raw.startswith(signature):
            return mime_type
    # WebP is a RIFF container; the format only appears at byte 8.
    if raw[:4] == b"RIFF" and raw[8:12] == b"WEBP":
        return "image/webp"
    return None


def decode_bytes(raw: bytes, mime_type: str) -> tuple[DecodedImageSource, str | None]:
    """
    Decodes a browser-supplied bitmap sent as raw bytes.

    The `data:` URI form costs a third again in transfer and a full base64 pass
    on both ends, which on a phone photograph is most of the wait before the
    picture appears. Posting the file's own bytes skips all of it; this is the
    same decode, without the envelope.

    The declared type is a hint, not the answer: what the bytes begin with wins
    whenever the two disagree, so an upload still loads when a proxy has
    rewritten the header or a picker gave the file no type at all.
    """
    declared = (mime_type or "").split(";")[0].strip().lower()
    sniffed = sniff_image_type(raw)

    mime_type = sniffed or declared
    if mime_type not in ACCEPTED_MIME_TYPES:
        raise ImageSourceError(
            f"{declared or 'That file'} is not an image format Palette Trace can trace."
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
