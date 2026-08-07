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
        length = int(self.headers.get("Content-Length", 0))
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
        rel_path = req_path.lstrip("/").split("?")[0]
        if not rel_path or rel_path == "index.html":
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


def launch_palette_trace_app(session, open_browser: bool = True) -> bool:
    """
    Serves the local interface on 127.0.0.1 and an ephemeral port (§9.1, §31).

    Blocks until the user applies or cancels. Returns True when applied.

    `open_browser` is False for headless runs and for the standalone host's
    `--no-browser` mode, where the URL is printed instead.
    """
    PaletteTraceRequestHandler.session = session

    server = HTTPServer(("127.0.0.1", 0), PaletteTraceRequestHandler)
    port = server.server_port
    url = f"http://127.0.0.1:{port}/index.html?token={session.session_token}"

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
