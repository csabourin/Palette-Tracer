"""
REST API handlers for Palette Trace local web interface.
"""

import io
import json
import base64
import numpy as np
from PIL import Image
from palette_trace.color.conversion import srgb_to_hex
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
from palette_trace.settings import (
    reset_automatic_entries_preserving_pinned,
    reset_settings_to_destination_defaults,
)


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


def handle_api_request(session, path: str, method: str, body: dict, headers: dict) -> tuple[int, dict]:
    """
    Dispatches HTTP API requests for the Web UI.
    Returns (status_code, response_json_dict).
    """
    # 1. Validate session token header
    token = headers.get("X-Session-Token") or headers.get("x-session-token")
    if not token or not session.validate_token(token):
        return (401, {"error": "Invalid or missing session token."})

    if path == "/api/session" and method == "GET":
        return (200, {
            "sessionToken": session.session_token,
            "imageWidth": session.image_source.intrinsic_width if session.image_source else 0,
            "imageHeight": session.image_source.intrinsic_height if session.image_source else 0,
            "settings": session.settings,
            # §27: the interface must offer recovery choices when true.
            "sourceChanged": bool(getattr(session, "source_changed", False)),
        })

    elif path == "/api/sample_color" and method == "POST":
        x = int(body.get("x", 0))
        y = int(body.get("y", 0))
        mode = body.get("mode", "5x5_median")

        if not session.image_source:
            return (400, {"error": "No image source loaded."})

        w = session.image_source.intrinsic_width
        h = session.image_source.intrinsic_height

        if not (0 <= x < w and 0 <= y < h):
            return (400, {"error": "Sample coordinates out of bounds."})

        if mode == "exact":
            r, g, b = session.image_source.srgb[y, x]
        else:
            # 5x5 neighborhood median
            min_y = max(0, y - 2)
            max_y = min(h, y + 3)
            min_x = max(0, x - 2)
            max_x = min(w, x + 3)

            sub_srgb = session.image_source.srgb[min_y:max_y, min_x:max_x]
            r = float(np.median(sub_srgb[:, :, 0]))
            g = float(np.median(sub_srgb[:, :, 1]))
            b = float(np.median(sub_srgb[:, :, 2]))

        hex_c = srgb_to_hex(r, g, b)
        return (200, {"hex": hex_c, "r": r, "g": g, "b": b})

    elif path == "/api/update_settings" and method == "POST":
        session.settings = body.get("settings", {})
        return (200, _settings_response(session))

    elif path == "/api/preview_source" and method == "GET":
        if not session.image_source:
            return (400, {"error": "No image source."})
        # Return base64 PNG of decoded image source
        img = Image.fromarray((session.image_source.srgb * 255).astype(np.uint8))
        buf = io.BytesIO()
        img.save(buf, format="PNG")
        b64 = base64.b64encode(buf.getvalue()).decode("ascii")
        return (200, {"dataUri": f"data:image/png;base64,{b64}"})

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
