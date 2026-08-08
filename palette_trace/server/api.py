"""
REST API handlers for Palette Trace local web interface.
"""

import io
import base64
import uuid

import numpy as np
from PIL import Image

from palette_trace.color.conversion import srgb_to_hex
from palette_trace.errors import PaletteTraceError
from palette_trace.pipeline.controller import PipelineController
from palette_trace.presets.destination import (
    apply_destination_preset,
    get_destination_preset,
    list_destination_ids,
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
from palette_trace.server.uploads import decode_upload, display_name
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


def sample_source_color(srgb, x: int, y: int, mode: str) -> tuple[float, float, float]:
    """
    Samples one colour from the decoded source at (x, y).

    §9.3 forbids an arithmetic mean: averaging across an edge invents a colour
    that appears nowhere in the image, which is exactly the colour a user
    aiming at an edge does not want. The median returns a real neighbourhood
    colour, and the dominant mode returns the most common one.
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
        return tuple(float(np.median(window[:, channel])) for channel in range(3))

    # Dominant: bucket the window at 5 bits per channel, take the most
    # populated bucket, and return the median *within* it. Reporting the
    # bucket centre instead would return a colour the image does not contain.
    buckets = (window * 31.0).round().astype(np.int32)
    keys = buckets[:, 0] * 1024 + buckets[:, 1] * 32 + buckets[:, 2]
    unique, counts = np.unique(keys, return_counts=True)
    # np.unique sorts, so ties resolve to the lowest key — deterministic.
    dominant = window[keys == unique[int(np.argmax(counts))]]
    return tuple(float(np.median(dominant[:, channel])) for channel in range(3))


def _run_pipeline(session) -> dict:
    """Re-runs the controller for the session's current settings."""
    session.controller = PipelineController(session.image_source, session.settings)
    session.pipeline_output = session.controller.run_pipeline()
    return session.pipeline_output


def _settings_response(session, status: str = "success") -> dict:
    """The common response shape after any mutation that re-runs the pipeline."""
    output = _run_pipeline(session)
    session.settings["palette"]["entries"] = output["palette_entries"]
    return {
        "status": status,
        "settings": session.settings,
        "paletteEntries": output["palette_entries"],
        "claimStats": output["claims_stats"],
        "scanResults": output["scan_results"],
        "warnings": output.get("warnings", []),
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
    """
    output = session.pipeline_output or _run_pipeline(session)
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


#: Endpoints that operate on a bitmap. Requesting one before an image has been
#: loaded is a client bug rather than a user error, so it gets a plain 400
#: instead of being folded into each handler.
_REQUIRES_IMAGE = frozenset({
    "/api/sample_color",
    "/api/update_settings",
    "/api/preview_source",
    "/api/apply_destination",
    "/api/reset_destination_defaults",
    "/api/resolve_source_change",
    "/api/user_presets/apply",
    "/api/export",
    "/api/apply",
})


def handle_api_request(session, path: str, method: str, body: dict, headers: dict) -> tuple[int, dict]:
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

    if path == "/api/session" and method == "GET":
        return (200, _session_state(session))

    elif path == "/api/load_image" and method == "POST":
        # §9.4.2: the browser is the only image chooser. There is deliberately
        # no endpoint that lists or reads arbitrary local paths (§9.1) — the
        # bytes come up in the request body and are decoded in memory.
        if not session.can_load_image:
            return (400, {"error": "This host supplies its own image and cannot load another."})

        try:
            source, resize_notice = decode_upload(body.get("dataUri", ""))
        except PaletteTraceError as exc:
            return (400, {"error": str(exc)})

        session.image_source = source
        session.image_name = display_name(body.get("fileName", ""))
        session.origin = ORIGIN_UPLOAD
        session.output_path = None
        session.resize_notice = resize_notice
        session.source_changed = False

        # A new bitmap invalidates every pinned colour, claim and layer order
        # from the previous one, so the settings restart from the destination
        # that was already chosen rather than from whatever the last image left
        # behind (§27's "start from destination defaults", applied implicitly
        # because there is no prior configuration for *this* image to recover).
        previous_destination = (session.settings or {}).get("destination", {}).get("id", "illustration")
        session.settings = reset_settings_to_destination_defaults(
            str(uuid.uuid4()), previous_destination
        )
        session.settings.setdefault("source", {}).update({
            "intrinsicWidth": source.intrinsic_width,
            "intrinsicHeight": source.intrinsic_height,
            "mimeType": source.mime_type,
        })

        response = _settings_response(session)
        response.update(_session_state(session))
        response["dataUri"] = _source_preview_data_uri(session)
        return (200, response)

    elif path == "/api/export" and method == "POST":
        # Deliberately does not end the session: a download is a checkpoint,
        # not a commitment, and the user may well adjust and download again.
        return (200, {
            "svg": _build_result_svg(session),
            "fileName": _download_name(session),
        })

    elif path == "/api/sample_color" and method == "POST":
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

    elif path == "/api/update_settings" and method == "POST":
        session.settings = body.get("settings", {})
        return (200, _settings_response(session))

    elif path == "/api/preview_source" and method == "GET":
        return (200, {"dataUri": _source_preview_data_uri(session)})

    elif path == "/api/destination_presets" and method == "GET":
        ids = list_destination_ids()
        return (200, {
            "order": ids,
            "presets": {dest_id: get_destination_preset(dest_id) for dest_id in ids},
        })

    elif path == "/api/apply_destination" and method == "POST":
        dest_id = body.get("destinationId", "")
        apply_destination_preset(session.settings, dest_id)
        return (200, _settings_response(session))

    elif path == "/api/reset_destination_defaults" and method == "POST":
        # §9.2 "Reset to destination defaults": re-apply the *current*
        # destination's technical defaults, discarding manual geometry or
        # trace-profile tweaks made since (§19). Palette entries survive.
        current_id = session.settings.get("destination", {}).get("id", "illustration")
        apply_destination_preset(session.settings, current_id)
        return (200, _settings_response(session))

    elif path == "/api/trace_profiles" and method == "GET":
        ids = list_profile_ids()
        return (200, {
            "order": ids,
            "profiles": {profile_id: get_builtin_profile(profile_id) for profile_id in ids},
        })

    elif path == "/api/resolve_source_change" and method == "POST":
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

    elif path == "/api/user_presets" and method == "GET":
        return (200, {"presets": list_user_presets()})

    elif path == "/api/user_presets" and method == "POST":
        name = (body.get("name") or "").strip()
        if not name:
            return (400, {"error": "A preset name is required."})
        description = body.get("description", "")
        scope = body.get("scope", "full")
        if scope not in ("structure", "palette", "full"):
            return (400, {"error": f"Unknown preset scope: {scope}"})
        preset = save_user_preset(name, description, session.settings, scope)
        return (200, {"preset": preset, "presets": list_user_presets()})

    elif path == "/api/user_presets/delete" and method == "POST":
        preset_uuid = body.get("presetUuid", "")
        deleted = delete_user_preset(preset_uuid)
        if not deleted:
            return (404, {"error": "No such preset."})
        return (200, {"status": "deleted", "presets": list_user_presets()})

    elif path == "/api/user_presets/apply" and method == "POST":
        preset_uuid = body.get("presetUuid", "")
        preset = load_user_preset(preset_uuid)
        if not preset:
            return (404, {"error": "No such preset."})
        apply_configuration_patch(session.settings, preset.get("configurationPatch", {}))
        return (200, _settings_response(session))

    elif path == "/api/apply" and method == "POST":
        session.is_applied = True
        return (200, {"status": "applied"})

    elif path == "/api/cancel" and method == "POST":
        session.is_cancelled = True
        return (200, {"status": "cancelled"})

    return (404, {"error": "API endpoint not found."})
