"""
REST API handlers for Palette Trace local web interface.
"""

import base64
import io
import sys
import uuid
from urllib.parse import unquote

import numpy as np
from PIL import Image

from palette_trace.color.conversion import hex_to_srgb, srgb_to_hex, srgb_to_oklab
from palette_trace.color.histogram import unique_source_colors
from palette_trace.errors import PaletteTraceError
from palette_trace.pipeline.controller import PipelineController
from palette_trace.presets.destination import (
    apply_destination_preset,
    get_destination_preset,
    list_destination_ids,
)
from palette_trace.presets.preview_scaling import (
    PREVIEW_BUDGET_PIXELS,
    preview_scale_for,
)
from palette_trace.presets.profiles import get_builtin_profile, list_profile_ids
from palette_trace.presets.user_presets import (
    apply_configuration_patch,
    delete_user_preset,
    list_user_presets,
    load_user_preset,
    save_user_preset,
)
from palette_trace.server.session import ORIGIN_UPLOAD
from palette_trace.server.uploads import (
    MAX_WORKING_PIXELS,
    decode_bytes,
    decode_upload,
    display_name,
)
from palette_trace.settings import (
    compute_settings_hash,
    record_generation_provenance,
    reset_automatic_entries_preserving_pinned,
    reset_settings_to_destination_defaults,
)
from palette_trace.svg_writer import build_svg_document

#: Picker sample sizes (§9.3). The 5×5 median is the default; exact-pixel
#: sampling MUST be available; a larger dominant-colour sample MAY be offered.
#: All three are exposed as an explicit control rather than as modifier keys,
#: which do not exist on a touchscreen.
SAMPLE_EXACT = "exact"
SAMPLE_MEDIAN_5X5 = "5x5_median"
SAMPLE_DOMINANT_15X15 = "15x15_dominant"
SAMPLE_MODES = (SAMPLE_EXACT, SAMPLE_MEDIAN_5X5, SAMPLE_DOMINANT_15X15)


def _representative_pixel(window: np.ndarray) -> tuple[float, float, float]:
    """
    The pixel of `window` closest to its per-channel median.

    The median itself is computed channel by channel, so on an even-sized run —
    or on any window where the three channels disagree about which pixel sits in
    the middle — it is a value no pixel in the window actually has. Returning the
    nearest real pixel instead keeps the median's resistance to outliers while
    guaranteeing the answer is a colour the picture contains (§34.5).
    """
    median = np.median(window, axis=0)
    distances = ((window - median) ** 2).sum(axis=1)
    # `argmin` takes the first minimum, and the window is scanned in raster
    # order, so the same window always answers with the same pixel (§34.30).
    pixel = window[int(np.argmin(distances))]
    return float(pixel[0]), float(pixel[1]), float(pixel[2])


def sample_source_color(srgb, x: int, y: int, mode: str) -> tuple[float, float, float]:
    """
    Samples one colour from the decoded source at (x, y).

    Every mode returns a colour that is literally present in the picture. §9.3
    forbids an arithmetic mean because averaging across an edge invents a colour
    that appears nowhere — but a per-channel median can invent one too, so the
    neighbourhood modes report the real pixel nearest to that median rather than
    the median itself.
    """
    height, width = srgb.shape[:2]

    if mode == SAMPLE_EXACT:
        pixel = srgb[y, x]
        return float(pixel[0]), float(pixel[1]), float(pixel[2])

    radius = 2 if mode == SAMPLE_MEDIAN_5X5 else 7
    window = srgb[
        max(0, y - radius):min(height, y + radius + 1),
        max(0, x - radius):min(width, x + radius + 1),
    ].reshape(-1, 3)

    if mode == SAMPLE_MEDIAN_5X5:
        return _representative_pixel(window)

    # Dominant: bucket the window at 5 bits per channel, take the most
    # populated bucket, and report a real pixel from within it. The bucket
    # centre would be a colour the image does not contain.
    buckets = (window * 31.0).round().astype(np.int32)
    keys = buckets[:, 0] * 1024 + buckets[:, 1] * 32 + buckets[:, 2]
    unique, counts = np.unique(keys, return_counts=True)
    # np.unique sorts, so ties resolve to the lowest key — deterministic.
    dominant = window[keys == unique[int(np.argmax(counts))]]
    return _representative_pixel(dominant)


def nearest_source_color(srgb, r: float, g: float, b: float) -> tuple[float, float, float]:
    """
    The colour in the picture perceptually closest to the one asked for.

    Backs the "snap to the picture" option on typed-in colours: a value copied
    out of a brand guide is rarely a value the scan actually contains, and a
    palette entry anchored to a colour that is not there claims nothing.
    """
    target = np.asarray(srgb_to_oklab(r, g, b), dtype=np.float64)
    rgb255, _, oklab = unique_source_colors(srgb, np.ones(srgb.shape[:2], dtype=bool))
    if rgb255.shape[0] == 0:
        return r, g, b

    closest = int(np.argmin(((oklab - target) ** 2).sum(axis=1)))
    red, green, blue = rgb255[closest]
    return red / 255.0, green / 255.0, blue / 255.0


def _header(headers: dict, name: str) -> str:
    """Case-insensitive header lookup; `http.server` preserves the sent casing."""
    lowered = name.lower()
    for key, value in headers.items():
        if key.lower() == lowered:
            return value
    return ""


def _header_file_name(headers: dict) -> str:
    """
    The uploaded file's name, sent percent-encoded so it survives a header.

    Header values are Latin-1 by the letter of HTTP, and a filename is whatever
    the user's filesystem allows, so the browser encodes it on the way out.
    """
    raw = _header(headers, "X-File-Name")
    try:
        return unquote(raw)
    except (UnicodeDecodeError, ValueError):
        return ""


def _flag(body: dict, headers: dict, key: str, header_name: str) -> bool:
    """A boolean sent either in a JSON body or, for a raw upload, as a header."""
    if key in body:
        return bool(body[key])
    return _header(headers, header_name).strip().lower() in ("1", "true", "yes")


def _client_bitmap_size(body: dict, headers: dict) -> tuple[int, int] | None:
    """The size of the bitmap the client already holds, when it reported one."""
    reported = body.get("clientBitmap")
    if isinstance(reported, dict):
        values = (reported.get("width"), reported.get("height"))
    else:
        values = (_header(headers, "X-Client-Width"), _header(headers, "X-Client-Height"))

    try:
        width, height = int(values[0]), int(values[1])
    except (TypeError, ValueError):
        return None
    return (width, height) if width > 0 and height > 0 else None


def _preview_scale(session) -> float:
    """
    The factor an interactive re-trace of this session's bitmap runs at (§17.4).

    Previewing is the whole reason §17.4 exists: a settings change should answer
    while the user is still looking at the control they moved. What is committed
    is never previewed — see `_run_pipeline`.
    """
    source = session.image_source
    if source is None:
        return 1.0
    return preview_scale_for(
        source.intrinsic_width, source.intrinsic_height, PREVIEW_BUDGET_PIXELS
    )


def _run_pipeline(session, preview: bool = False) -> dict:
    """
    Re-runs the controller for the session's current settings.

    `preview` decides how much bitmap is traced, never what comes back: the
    controller converts the pixel-denominated settings on the way in and the
    geometry on the way out, so both answers are in source pixels either way
    (§17.4).
    """
    scale = _preview_scale(session) if preview else 1.0

    # Let go of the previous run before starting the next. Both hold traced
    # geometry and per-scan masks for the whole bitmap, and the full-resolution
    # run behind an export is the largest of them — keeping a preview alive
    # across it peaks at the sum of the two for no benefit, since the run about
    # to happen replaces it either way. On a small container that difference is
    # the process being killed, which the browser sees as a reply from the proxy
    # rather than from this server.
    session.controller = None
    session.pipeline_output = None

    if not preview:
        src = session.image_source
        print(
            f"Palette Trace: running full-resolution pipeline "
            f"({src.intrinsic_width}\u00d7{src.intrinsic_height})\u2026",
            file=sys.stderr,
            flush=True,
        )

    session.controller = PipelineController(session.image_source, session.settings, scale)
    output = session.controller.run_pipeline()

    if not preview:
        print("Palette Trace: full-resolution pipeline done.", file=sys.stderr, flush=True)

    output["previewScale"] = session.controller.effective_scale
    session.pipeline_output = output
    session.pipeline_output_is_preview = session.controller.is_preview
    return output


def _settings_response(session, status: str = "success") -> dict:
    """
    The common response shape after any mutation that re-runs the pipeline.

    Every caller of this is a settings change the user is waiting on, so it
    previews. The two endpoints that produce a deliverable — `/api/export` and
    the Apply path — deliberately do not go through here.
    """
    output = _run_pipeline(session, preview=True)
    session.settings["palette"]["entries"] = output["palette_entries"]
    return {
        "status": status,
        "settings": session.settings,
        "paletteEntries": output["palette_entries"],
        "claimStats": output["claims_stats"],
        "scanResults": output["scan_results"],
        "warnings": output.get("warnings", []),
        # What the geometry in `scanResults` was traced from, as a fraction of
        # the source. The interface states it rather than leaving the user to
        # wonder why a preview is smoother than the file they downloaded.
        "previewScale": output["previewScale"],
    }


def _source_preview_data_uri(session) -> str:
    """Base64 PNG of the decoded source, for the interface's canvas."""
    img = Image.fromarray((session.image_source.srgb * 255).astype(np.uint8))
    buf = io.BytesIO()
    img.save(buf, format="PNG")
    return "data:image/png;base64," + base64.b64encode(buf.getvalue()).decode("ascii")


def _session_state(session) -> dict:
    """
    What the interface needs to decide what to show.

    The interface opens on an image-loading screen and reveals everything else
    only once there is something to trace, so `hasImage` — not the presence of
    settings — is what gates the workspace (§9.2).
    """
    source = session.image_source
    return {
        "sessionToken": session.session_token,
        "hasImage": session.has_image,
        "imageName": session.image_name,
        "imageWidth": source.intrinsic_width if source else 0,
        "imageHeight": source.intrinsic_height if source else 0,
        "settings": session.settings,
        # Names the primary action: commit into the open document, write the
        # configured SVG file, or hand the result back as a download (§9.4.2).
        "commitTarget": session.commit_target,
        "canLoadImage": session.can_load_image,
        "resizeNotice": session.resize_notice,
        # §27: the interface must offer recovery choices when true.
        "sourceChanged": bool(getattr(session, "source_changed", False)),
        # The pixel budget the server will resize an upload down to. Told to the
        # client so it can do that scaling itself before sending: the same
        # bitmap arrives, but a phone photograph stops spending seconds being
        # encoded, transferred and decoded at a size nothing will ever use.
        "maxWorkingPixels": MAX_WORKING_PIXELS,
    }


def _download_name(session) -> str:
    """Suggested filename for a downloaded result."""
    stem = (session.image_name or "palette-trace").rsplit(".", 1)[0] or "palette-trace"
    return f"{stem}.palette-trace.svg"


def _build_result_svg(session) -> str:
    """
    Assembles the SVG for the current pipeline output and records provenance.

    Shared by the download path and the Inkscape/CLI commit path so that a
    downloaded result carries the same `pt:` provenance as a written one
    (§9.4.4, §10.3).

    Always traced at full resolution. A cached preview is geometry from a
    reduced copy of the bitmap: correct to look at, and not what anyone asked to
    be given. §17.4 makes interaction cheap by previewing — it does not make the
    delivered file a preview.
    """
    # Read through the session rather than into a local first: a local
    # reference to the cached preview would keep it alive for the duration of
    # the full-resolution run that is meant to replace it.
    if session.pipeline_output is None or session.pipeline_output_is_preview:
        _run_pipeline(session)
    output = session.pipeline_output
    backend = output.get("backend", {})
    source = session.image_source

    record_generation_provenance(
        session.settings,
        source_hash=source.fingerprint,
        backend_id=backend.get("id", ""),
        backend_version=backend.get("version", ""),
    )

    return build_svg_document(
        output["scan_results"],
        source.intrinsic_width,
        source.intrinsic_height,
        session.settings["imageUuid"],
        provenance={
            "settingsHash": compute_settings_hash(session.settings),
            "sourceHash": source.fingerprint,
            "backendId": backend.get("id", ""),
            "backendVersion": backend.get("version", ""),
        },
    )


#: Endpoints answered without taking the session's pipeline lock. Everything
#: else is serialised, which is the safe default: a new endpoint has to be added
#: here deliberately, and the cost of forgetting is a wait rather than a race.
#:
#: The bar for membership is that the response shares no mutable session state
#: at all. The two preset listings are built from static preset data, and
#: `/api/cancel` answers a constant after raising a flag — and is precisely the
#: request that must not queue behind the slow trace the user is abandoning.
#:
#: `/api/session` is deliberately *not* here, tempting though it is: its
#: response embeds `session.settings` by reference, and the caller serialises
#: that dictionary after this function has returned. Answering it outside the
#: lock would mean walking the settings tree while a destination change or an
#: export writes into it. It is fetched once when the page loads, so it has
#: nothing to gain and a torn response to lose.
_WITHOUT_PIPELINE_LOCK = frozenset({
    "/api/destination_presets",
    "/api/trace_profiles",
    "/api/cancel",
})


#: Endpoints that operate on a bitmap. Requesting one before an image has been
#: loaded is a client bug rather than a user error, so it gets a plain 400
#: instead of being folded into each handler.
_REQUIRES_IMAGE = frozenset({
    "/api/sample_color",
    "/api/nearest_source_color",
    "/api/update_settings",
    "/api/preview_source",
    "/api/apply_destination",
    "/api/reset_destination_defaults",
    "/api/resolve_source_change",
    "/api/user_presets/apply",
    "/api/export",
    "/api/apply",
})


def handle_api_request(
    session, path: str, method: str, body: dict, headers: dict
) -> tuple[int, dict]:
    """
    Dispatches HTTP API requests for the Web UI.
    Returns (status_code, response_json_dict).
    """
    # 1. Validate session token header
    token = headers.get("X-Session-Token") or headers.get("x-session-token")
    if not token or not session.validate_token(token):
        return (401, {"error": "Invalid or missing session token."})

    if path in _REQUIRES_IMAGE and not session.has_image:
        return (400, {"error": "Load an image first."})

    if path in _WITHOUT_PIPELINE_LOCK:
        return _dispatch(session, path, method, body, headers)

    # The server answers requests concurrently, but the session is one mutable
    # document: two traces running at once would interleave their writes to its
    # settings, palette and cached output, and the result would depend on which
    # finished first. Serialising them here is what makes the concurrency above
    # a responsiveness change rather than a behaviour change.
    with session.pipeline_lock:
        return _dispatch(session, path, method, body, headers)


def _dispatch(session, path: str, method: str, body: dict, headers: dict) -> tuple[int, dict]:
    """The endpoint table itself; see `handle_api_request` for the gates."""
    if path == "/api/session" and method == "GET":
        return (200, _session_state(session))

    if path == "/api/load_image" and method == "POST":
        # §9.4.2: the browser is the only image chooser. There is deliberately
        # no endpoint that lists or reads arbitrary local paths (§9.1) — the
        # bytes come up in the request body and are decoded in memory.
        if not session.can_load_image:
            return (400, {"error": "This host supplies its own image and cannot load another."})

        try:
            if body.get("rawBody") is not None:
                source, resize_notice = decode_bytes(
                    body["rawBody"], body.get("contentType", "")
                )
                file_name = _header_file_name(headers)
            else:
                source, resize_notice = decode_upload(body.get("dataUri", ""))
                file_name = body.get("fileName", "")
        except PaletteTraceError as exc:
            return (400, {"error": str(exc)})

        session.image_source = source
        session.image_name = display_name(file_name)
        session.origin = ORIGIN_UPLOAD
        session.output_path = None
        session.resize_notice = resize_notice
        session.source_changed = False

        # A new bitmap invalidates every pinned colour, claim and layer order
        # from the previous one, so the settings restart from the destination
        # that was already chosen rather than from whatever the last image left
        # behind (§27's "start from destination defaults", applied implicitly
        # because there is no prior configuration for *this* image to recover).
        previous_destination = (
            (session.settings or {}).get("destination", {}).get("id", "illustration")
        )
        session.settings = reset_settings_to_destination_defaults(
            str(uuid.uuid4()), previous_destination
        )
        session.settings.setdefault("source", {}).update({
            "intrinsicWidth": source.intrinsic_width,
            "intrinsicHeight": source.intrinsic_height,
            "mimeType": source.mime_type,
        })

        # The trace is the slow half of loading a picture. A client that intends
        # to show the bitmap first and ask for colours second says so, and gets
        # the decode on its own — the interface stops being blank a lot sooner.
        if _flag(body, headers, "deferTrace", "X-Defer-Trace"):
            response = {"status": "success"}
        else:
            response = _settings_response(session)
        response.update(_session_state(session))

        # Re-encoding the source to base64 PNG costs seconds on a phone
        # photograph, and the browser that just chose the file already has the
        # pixels. It is only sent when the decode changed the picture's size —
        # an EXIF rotation, or a resize down to the working budget — because
        # that is when the client's copy is no longer what is being traced.
        client_size = _client_bitmap_size(body, headers)
        if client_size == (source.intrinsic_width, source.intrinsic_height):
            response["usesClientBitmap"] = True
        else:
            response["dataUri"] = _source_preview_data_uri(session)
        return (200, response)

    if path == "/api/nearest_source_color" and method == "POST":
        try:
            red, green, blue = hex_to_srgb(str(body.get("hex", "")))
        except ValueError:
            return (400, {"error": "That is not a colour this can look up."})

        red, green, blue = nearest_source_color(session.image_source.srgb, red, green, blue)
        return (200, {
            "hex": srgb_to_hex(red, green, blue), "r": red, "g": green, "b": blue,
        })

    if path == "/api/export" and method == "POST":
        # Deliberately does not end the session: a download is a checkpoint,
        # not a commitment, and the user may well adjust and download again.
        return (200, {
            "svg": _build_result_svg(session),
            "fileName": _download_name(session),
        })

    if path == "/api/sample_color" and method == "POST":
        x = int(body.get("x", 0))
        y = int(body.get("y", 0))
        mode = body.get("mode", SAMPLE_MEDIAN_5X5)

        if mode not in SAMPLE_MODES:
            return (400, {"error": f"Unknown sample mode: {mode}"})

        w = session.image_source.intrinsic_width
        h = session.image_source.intrinsic_height

        if not (0 <= x < w and 0 <= y < h):
            return (400, {"error": "Sample coordinates out of bounds."})

        r, g, b = sample_source_color(session.image_source.srgb, x, y, mode)
        return (200, {"hex": srgb_to_hex(r, g, b), "r": r, "g": g, "b": b, "mode": mode})

    if path == "/api/update_settings" and method == "POST":
        session.settings = body.get("settings", {})
        return (200, _settings_response(session))

    if path == "/api/preview_source" and method == "GET":
        return (200, {"dataUri": _source_preview_data_uri(session)})

    if path == "/api/destination_presets" and method == "GET":
        ids = list_destination_ids()
        return (200, {
            "order": ids,
            "presets": {dest_id: get_destination_preset(dest_id) for dest_id in ids},
        })

    if path == "/api/apply_destination" and method == "POST":
        dest_id = body.get("destinationId", "")
        apply_destination_preset(session.settings, dest_id)
        return (200, _settings_response(session))

    if path == "/api/reset_destination_defaults" and method == "POST":
        # §9.2 "Reset to destination defaults": re-apply the *current*
        # destination's technical defaults, discarding manual geometry or
        # trace-profile tweaks made since (§19). Palette entries survive.
        current_id = session.settings.get("destination", {}).get("id", "illustration")
        apply_destination_preset(session.settings, current_id)
        return (200, _settings_response(session))

    if path == "/api/trace_profiles" and method == "GET":
        ids = list_profile_ids()
        return (200, {
            "order": ids,
            "profiles": {profile_id: get_builtin_profile(profile_id) for profile_id in ids},
        })

    if path == "/api/resolve_source_change" and method == "POST":
        # §27: the four recovery choices offered when the recorded source
        # fingerprint no longer matches the decoded bitmap.
        action = body.get("action", "recalculate")

        if action == "cancel":
            session.is_cancelled = True
            return (200, {"status": "cancelled"})

        if action == "reset_automatic":
            reset_automatic_entries_preserving_pinned(session.settings)
        elif action == "destination_defaults":
            image_uuid = session.settings.get("imageUuid", "")
            dest_id = session.settings.get("destination", {}).get("id", "illustration")
            session.settings = reset_settings_to_destination_defaults(image_uuid, dest_id)
        elif action != "recalculate":
            return (400, {"error": f"Unknown source-change action: {action}"})

        session.source_changed = False
        return (200, _settings_response(session))

    if path == "/api/user_presets" and method == "GET":
        return (200, {"presets": list_user_presets()})

    if path == "/api/user_presets" and method == "POST":
        name = (body.get("name") or "").strip()
        if not name:
            return (400, {"error": "A preset name is required."})
        description = body.get("description", "")
        scope = body.get("scope", "full")
        if scope not in ("structure", "palette", "full"):
            return (400, {"error": f"Unknown preset scope: {scope}"})
        preset = save_user_preset(name, description, session.settings, scope)
        return (200, {"preset": preset, "presets": list_user_presets()})

    if path == "/api/user_presets/delete" and method == "POST":
        preset_uuid = body.get("presetUuid", "")
        deleted = delete_user_preset(preset_uuid)
        if not deleted:
            return (404, {"error": "No such preset."})
        return (200, {"status": "deleted", "presets": list_user_presets()})

    if path == "/api/user_presets/apply" and method == "POST":
        preset_uuid = body.get("presetUuid", "")
        preset = load_user_preset(preset_uuid)
        if not preset:
            return (404, {"error": "No such preset."})
        apply_configuration_patch(session.settings, preset.get("configurationPatch", {}))
        return (200, _settings_response(session))

    if path == "/api/apply" and method == "POST":
        # Both hosts commit `session.pipeline_output` directly — into the open
        # Inkscape document, or into the configured SVG file. After a settings
        # change that output is a preview (§17.4), so it is re-traced at full
        # resolution first. This is the one request where the wait is the point:
        # the user has asked for the result, not for a picture of it.
        if session.pipeline_output is None or session.pipeline_output_is_preview:
            output = _run_pipeline(session)
            # A preview resolves its automatic colours from the reduced copy,
            # and those are the entries currently in the settings document. The
            # full-resolution run has just resolved them again from the real
            # pixels, so the settings — and the sidecar written from them — say
            # the same thing as the file being committed.
            session.settings["palette"]["entries"] = output["palette_entries"]
        session.is_applied = True
        return (200, {"status": "applied"})

    if path == "/api/cancel" and method == "POST":
        session.is_cancelled = True
        return (200, {"status": "cancelled"})

    return (404, {"error": "API endpoint not found."})
