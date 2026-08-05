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
        new_settings = body.get("settings", {})
        session.settings = new_settings

        # Re-run pipeline controller
        session.controller = PipelineController(session.image_source, session.settings)
        session.pipeline_output = session.controller.run_pipeline()

        return (200, {
            "status": "success",
            "paletteEntries": session.pipeline_output["palette_entries"],
            "claimStats": session.pipeline_output["claims_stats"],
            "scanResults": session.pipeline_output["scan_results"],
        })

    elif path == "/api/preview_source" and method == "GET":
        if not session.image_source:
            return (400, {"error": "No image source."})
        # Return base64 PNG of decoded image source
        img = Image.fromarray((session.image_source.srgb * 255).astype(np.uint8))
        buf = io.BytesIO()
        img.save(buf, format="PNG")
        b64 = base64.b64encode(buf.getvalue()).decode("ascii")
        return (200, {"dataUri": f"data:image/png;base64,{b64}"})

    elif path == "/api/apply" and method == "POST":
        session.is_applied = True
        return (200, {"status": "applied"})

    elif path == "/api/cancel" and method == "POST":
        session.is_cancelled = True
        return (200, {"status": "cancelled"})

    return (404, {"error": "API endpoint not found."})
