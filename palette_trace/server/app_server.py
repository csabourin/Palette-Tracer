"""
Local HTTP Server bound strictly to 127.0.0.1 on an ephemeral port.
Serves static frontend assets and API endpoints.
"""

import os
import json
import threading
import webbrowser
from http.server import ThreadingHTTPServer, BaseHTTPRequestHandler
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
        body = self._parse_body(raw_body)

        headers = {k: v for k, v in self.headers.items()}
        status, res = handle_api_request(self.session, self.path.split("?")[0], "POST", body, headers)
        self._send_json(status, res)

    def _parse_body(self, raw_body: bytes) -> dict:
        """
        Reads a request body as either JSON or a bitmap's own bytes.

        `Content-Type` decides when it says something useful, and the body
        itself decides when it does not: a body that will not parse as JSON was
        never going to be JSON, whatever a header claimed on its behalf. That
        matters because the header is the one part of this request that
        something in the middle — a reverse proxy, a rewritten fetch — can
        quietly change, and losing it should not turn a picture into an error
        message about data URIs.
        """
        content_type = (self.headers.get("Content-Type") or "").split(";")[0].strip().lower()
        binary = {"rawBody": raw_body, "contentType": content_type}

        if content_type and content_type != "application/json":
            return binary

        try:
            parsed = json.loads(raw_body.decode("utf-8")) if raw_body else {}
        except (UnicodeDecodeError, ValueError):
            return binary

        return parsed if isinstance(parsed, dict) else binary

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
                self.send_header("Cache-Control", "no-store")
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
        # The interface and the server it talks to are one program, released
        # together and served from a port that gets reused. A cached app.js
        # against a freshly started server is two different versions of that
        # program negotiating an API neither of them agrees on.
        self.send_header("Cache-Control", "no-store")
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

    # Threaded, because a trace takes seconds and a single-request loop spends
    # every one of them refusing to accept a connection. Nothing else was
    # listening, so a reverse proxy in front of the server — Replit's, for one —
    # would give up waiting and answer the browser with its own error page,
    # which the interface then had to report as a reply that did not come from
    # Palette Trace at all. `api.handle_api_request` serialises the work that
    # touches session state, so concurrency here buys responsiveness, not races.
    server = ThreadingHTTPServer((host, port), PaletteTraceRequestHandler)
    bound_port = server.server_port
    path = f"/index.html?token={session.session_token}"

    if open_browser:
        webbrowser.open(f"http://127.0.0.1:{bound_port}{path}")
    elif host in ("0.0.0.0", "::"):
        # Bound to every interface, which is what a container platform needs.
        # This used to print a 127.0.0.1 URL regardless, which says the opposite
        # of what the socket is doing: it reads as "loopback only, not reachable
        # from outside" at the exact moment the operator is trying to find out
        # whether their host is publishing it.
        print(f"Palette Trace is listening on every interface, port {bound_port}.")
        print("Open the address your host publishes for that port. No token is")
        print("needed in the address: a request without one is redirected to an")
        print("authenticated page.")
        print(f"On this machine that is http://127.0.0.1:{bound_port}{path}")
    else:
        print(f"Palette Trace interface: http://{host}:{bound_port}{path}")

    # Serve until Apply or Cancel. The loop runs on its own thread and this one
    # waits for the session to reach a terminal state, rather than checking the
    # flags between requests: `handle_request` blocks until a request arrives,
    # so the old loop only noticed Apply once some *later* request happened to
    # come in. `shutdown` stops the accept loop without touching the requests
    # already being served, and `server_close` joins them, so nothing is cut off
    # mid-response — including the very response that reported Apply.
    serving = threading.Thread(target=server.serve_forever, name="palette-trace-http")
    serving.start()
    try:
        session.finished.wait()
    except KeyboardInterrupt:
        session.is_cancelled = True
    finally:
        server.shutdown()
        serving.join()
        server.server_close()

    return session.is_applied
