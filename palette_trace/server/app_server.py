"""
Local HTTP Server bound strictly to 127.0.0.1 on an ephemeral port.
Serves static frontend assets and API endpoints.
"""

import os
import json
import webbrowser
from http.server import HTTPServer, BaseHTTPRequestHandler
from pathlib import Path
from palette_trace.server.api import handle_api_request

WEB_DIR = Path(__file__).parent.parent / "web"

#: §9.1 requires a restricted payload size. The ceiling is driven by
#: `/api/load_image`, whose body is a base64 data URI of the chosen bitmap:
#: base64 costs a third on top of `uploads.MAX_UPLOAD_BYTES`, and the rest is
#: JSON framing. Every other endpoint sends a few kilobytes of settings.
MAX_REQUEST_BYTES = 48 * 1024 * 1024


class PaletteTraceRequestHandler(BaseHTTPRequestHandler):
    session = None

    def log_message(self, format, *args):
        # Silence default HTTP server logging
        pass

    def do_GET(self):
        if self.path.startswith("/api/"):
            headers = {k: v for k, v in self.headers.items()}
            status, res = handle_api_request(self.session, self.path.split("?")[0], "GET", {}, headers)
            self._send_json(status, res)
        else:
            self._serve_static_file(self.path)

    def do_POST(self):
        try:
            length = int(self.headers.get("Content-Length", 0))
        except ValueError:
            self._send_json(400, {"error": "Malformed Content-Length header."})
            return

        if length > MAX_REQUEST_BYTES:
            # Answer without draining the body: reading it is precisely the
            # allocation the limit exists to refuse.
            self._send_json(413, {"error": "That request is too large to accept."})
            return

        raw_body = self.rfile.read(length) if length > 0 else b"{}"
        try:
            body = json.loads(raw_body.decode("utf-8")) if raw_body else {}
        except Exception:
            body = {}

        headers = {k: v for k, v in self.headers.items()}
        status, res = handle_api_request(self.session, self.path.split("?")[0], "POST", body, headers)
        self._send_json(status, res)

    def _send_json(self, status: int, data: dict):
        body_bytes = json.dumps(data).encode("utf-8")
        self.send_response(status)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(body_bytes)))
        self.send_header("Cache-Control", "no-store")
        self.end_headers()
        self.wfile.write(body_bytes)

    def _serve_static_file(self, req_path: str):
        path_only, _, query = req_path.partition("?")
        rel_path = path_only.lstrip("/")
        if not rel_path or rel_path == "index.html":
            if "token=" not in query and self.session is not None:
                # Convenience redirect so a bare `/` (e.g. a Replit webview
                # that has no way to know the session token) still lands on
                # an authenticated page. Purely a UX nicety: /api/* still
                # requires the token header regardless of how index.html was
                # reached, so this does not widen what §9.1 protects.
                self.send_response(302)
                self.send_header("Location", f"/index.html?token={self.session.session_token}")
                self.end_headers()
                return
            target_path = WEB_DIR / "index.html"
            content_type = "text/html"
        elif rel_path == "app.js":
            target_path = WEB_DIR / "app.js"
            content_type = "application/javascript"
        elif rel_path == "styles.css":
            target_path = WEB_DIR / "styles.css"
            content_type = "text/css"
        else:
            self.send_error(404, "File not found")
            return

        if not target_path.exists():
            self.send_error(404, "File not found")
            return

        with open(target_path, "rb") as f:
            content = f.read()

        self.send_response(200)
        self.send_header("Content-Type", content_type)
        self.send_header("Content-Length", str(len(content)))
        self.end_headers()
        self.wfile.write(content)


def launch_palette_trace_app(
    session, open_browser: bool = True, host: str = "127.0.0.1", port: int = 0
) -> bool:
    """
    Serves the local interface (§9.1, §31).

    §9.1 requires binding only to 127.0.0.1 or ::1 on an ephemeral port, and
    that remains the default for every caller that does not pass `host`/
    `port` explicitly. The standalone CLI's `--host`/`--port` flags (and its
    `PORT`-env-var detection for running under a container platform such as
    Replit, where the desktop loopback assumption does not hold) are the only
    callers that widen this, and only when a caller opts in explicitly.

    Blocks until the user applies or cancels. Returns True when applied.

    `open_browser` is False for headless runs and for the standalone host's
    `--no-browser` mode, where the URL is printed instead.
    """
    PaletteTraceRequestHandler.session = session

    server = HTTPServer((host, port), PaletteTraceRequestHandler)
    bound_port = server.server_port
    display_host = host if host not in ("0.0.0.0", "::") else "127.0.0.1"
    url = f"http://{display_host}:{bound_port}/index.html?token={session.session_token}"

    if open_browser:
        webbrowser.open(url)
    else:
        print(f"Palette Trace interface: {url}")

    # Event loop until Apply or Cancel
    try:
        while not session.is_applied and not session.is_cancelled:
            server.handle_request()
    finally:
        server.server_close()

    return session.is_applied
