"""
Local HTTP server behaviour (SPEC §9.1).

Exercises a real server on a loopback ephemeral port rather than mocking the
handler, because the things §9.1 requires — the bind address, the token gate,
the payload ceiling, the refusal of other methods — are properties of the
server, not of `handle_api_request`.
"""

import http.client
import json
import threading

import pytest
from http.server import HTTPServer

from palette_trace.server.app_server import (
    MAX_REQUEST_BYTES,
    PaletteTraceRequestHandler,
)
from palette_trace.server.session import AppSession
from palette_trace.settings import create_default_settings


@pytest.fixture
def server():
    session = AppSession()
    session.settings = create_default_settings("test-uuid")

    PaletteTraceRequestHandler.session = session
    httpd = HTTPServer(("127.0.0.1", 0), PaletteTraceRequestHandler)
    thread = threading.Thread(target=httpd.serve_forever, daemon=True)
    thread.start()
    try:
        yield httpd, session
    finally:
        httpd.shutdown()
        httpd.server_close()
        thread.join(timeout=5)


def connect(httpd):
    return http.client.HTTPConnection("127.0.0.1", httpd.server_port, timeout=10)


class TestPayloadLimit:
    def test_an_oversized_body_is_refused(self, server):
        """
        §9.1: accepted payload size is restricted. Before browser image
        loading, every request body was a few kilobytes of settings and the
        limit was theoretical; `/api/load_image` makes it the difference
        between a bounded allocation and an unbounded one.
        """
        httpd, session = server
        conn = connect(httpd)
        conn.putrequest("POST", "/api/load_image")
        conn.putheader("Content-Type", "application/json")
        conn.putheader("X-Session-Token", session.session_token)
        conn.putheader("Content-Length", str(MAX_REQUEST_BYTES + 1))
        conn.endheaders()
        # Deliberately never send the body: a server that answers 413 without
        # reading it is the point of the test.
        response = conn.getresponse()

        assert response.status == 413
        conn.close()

    def test_a_malformed_content_length_is_refused(self, server):
        httpd, session = server
        conn = connect(httpd)
        conn.putrequest("POST", "/api/session")
        conn.putheader("X-Session-Token", session.session_token)
        conn.putheader("Content-Length", "not-a-number")
        conn.endheaders()
        assert conn.getresponse().status == 400
        conn.close()

    def test_a_normal_body_still_goes_through(self, server):
        httpd, session = server
        conn = connect(httpd)
        conn.request(
            "POST", "/api/load_image",
            body=json.dumps({"dataUri": "not-a-data-uri"}),
            headers={"Content-Type": "application/json", "X-Session-Token": session.session_token},
        )
        response = conn.getresponse()
        # Rejected by the decoder, not by the size guard — the request itself
        # was accepted and dispatched.
        assert response.status == 400
        assert "data URI" in json.loads(response.read())["error"]
        conn.close()


class TestTokenGate:
    def test_api_requests_without_a_token_are_refused(self, server):
        httpd, _ = server
        conn = connect(httpd)
        conn.request("GET", "/api/session")
        assert conn.getresponse().status == 401
        conn.close()

    def test_the_interface_itself_is_served(self, server):
        httpd, session = server
        conn = connect(httpd)
        conn.request("GET", f"/index.html?token={session.session_token}")
        response = conn.getresponse()
        assert response.status == 200
        assert b"Palette Trace" in response.read()
        conn.close()

    def test_a_bare_root_redirects_to_the_tokened_url(self, server):
        httpd, session = server
        conn = connect(httpd)
        conn.request("GET", "/")
        response = conn.getresponse()
        assert response.status == 302
        assert session.session_token in response.getheader("Location")
        conn.close()


class TestStaticSurface:
    @pytest.mark.parametrize("path", ["/app.js", "/styles.css"])
    def test_the_bundled_assets_are_served(self, server, path):
        httpd, _ = server
        conn = connect(httpd)
        conn.request("GET", path)
        assert conn.getresponse().status == 200
        conn.close()

    @pytest.mark.parametrize("path", [
        "/../../etc/passwd",
        "/palette_trace/settings.py",
        "/SPEC.md",
    ])
    def test_nothing_outside_the_bundled_assets_is_served(self, server, path):
        """§9.4.2: no file outside the interface's own assets is reachable."""
        httpd, _ = server
        conn = connect(httpd)
        conn.request("GET", path)
        assert conn.getresponse().status == 404
        conn.close()

    def test_unsupported_methods_are_refused(self, server):
        """§9.1: reject unsupported HTTP methods."""
        httpd, session = server
        conn = connect(httpd)
        conn.request("DELETE", "/api/session", headers={"X-Session-Token": session.session_token})
        assert conn.getresponse().status == 501
        conn.close()
