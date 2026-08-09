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
import time

import pytest
from http.server import ThreadingHTTPServer

from palette_trace.server import api
from palette_trace.server.app_server import (
    MAX_REQUEST_BYTES,
    PaletteTraceRequestHandler,
    launch_palette_trace_app,
)
from palette_trace.server.session import AppSession
from palette_trace.settings import create_default_settings


@pytest.fixture
def server():
    session = AppSession()
    session.settings = create_default_settings("test-uuid")

    PaletteTraceRequestHandler.session = session
    httpd = ThreadingHTTPServer(("127.0.0.1", 0), PaletteTraceRequestHandler)
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


def tiny_source():
    """A real decoded source, for the endpoints that refuse to run without one."""
    import numpy as np
    from PIL import Image

    from palette_trace.image_source import DecodedImageSource

    pixels = np.zeros((6, 8, 4), dtype=np.uint8)
    pixels[..., :3] = (200, 40, 60)
    pixels[..., 3] = 255
    return DecodedImageSource(Image.fromarray(pixels, mode="RGBA"), "image/png")


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
        assert "No picture arrived" in json.loads(response.read())["error"]
        conn.close()


class TestRawBodies:
    """A bitmap may be posted as its own bytes rather than as JSON (§9.4.2)."""

    def png(self):
        import io

        import numpy as np
        from PIL import Image

        pixels = np.zeros((12, 20, 4), dtype=np.uint8)
        pixels[..., :3] = (10, 120, 200)
        pixels[..., 3] = 255
        buf = io.BytesIO()
        Image.fromarray(pixels, mode="RGBA").save(buf, format="PNG")
        return buf.getvalue()

    def test_an_image_body_reaches_the_endpoint_undecoded(self, server):
        httpd, session = server
        conn = connect(httpd)
        conn.request(
            "POST", "/api/load_image", body=self.png(),
            headers={
                "Content-Type": "image/png",
                "X-Session-Token": session.session_token,
                "X-File-Name": "photo.png",
                "X-Defer-Trace": "1",
            },
        )
        response = conn.getresponse()
        body = json.loads(response.read())

        assert response.status == 200
        assert (body["imageWidth"], body["imageHeight"]) == (20, 12)
        assert body["imageName"] == "photo.png"
        conn.close()

    def test_an_image_body_with_no_content_type_still_arrives(self, server):
        """
        The header is the one part of this request something in the middle can
        rewrite. A body that will not parse as JSON was never JSON.
        """
        httpd, session = server
        raw = self.png()
        conn = connect(httpd)
        conn.putrequest("POST", "/api/load_image")
        conn.putheader("X-Session-Token", session.session_token)
        conn.putheader("X-Defer-Trace", "1")
        conn.putheader("Content-Length", str(len(raw)))
        conn.endheaders()
        conn.send(raw)

        response = conn.getresponse()
        body = json.loads(response.read())

        assert response.status == 200
        assert (body["imageWidth"], body["imageHeight"]) == (20, 12)
        conn.close()

    def test_the_interface_is_never_served_from_cache(self, server):
        """
        A cached app.js against a freshly started server is two versions of one
        program negotiating an API neither of them agrees on.
        """
        httpd, session = server
        for path in (f"/index.html?token={session.session_token}", "/app.js", "/styles.css"):
            conn = connect(httpd)
            conn.request("GET", path)
            response = conn.getresponse()
            assert response.getheader("Cache-Control") == "no-store", path
            response.read()
            conn.close()

    def test_a_json_body_is_still_parsed_as_json(self, server):
        httpd, session = server
        conn = connect(httpd)
        conn.request(
            "POST", "/api/update_settings",
            body=json.dumps({"settings": session.settings}),
            headers={"Content-Type": "application/json", "X-Session-Token": session.session_token},
        )
        # No image yet, so the endpoint says so — which it can only do after
        # having understood the request as JSON at all.
        assert conn.getresponse().status == 400
        conn.close()


class TestConcurrency:
    """
    §9.1 does not ask for a concurrent server, but a single-request one is what
    put the 404s in front of users: a trace holds the loop for seconds, nothing
    else is accepted, and the reverse proxy in front of the server answers the
    browser with its own error page instead. These pin the two halves of the
    fix — that slow work no longer blocks the socket, and that it is still
    serialised against itself.
    """

    @pytest.fixture
    def slow_pipeline(self, server, monkeypatch):
        """Replaces the pipeline with something slow and observable."""
        httpd, session = server
        session.image_source = tiny_source()
        spans = []
        lock = threading.Lock()

        def _slow(_session, preview=False):
            started = time.perf_counter()
            time.sleep(0.5)
            with lock:
                spans.append((started, time.perf_counter()))
            return {"palette_entries": [], "claims_stats": {}, "scan_results": [],
                    "warnings": [], "previewScale": 1.0}

        monkeypatch.setattr(api, "_run_pipeline", _slow)
        return httpd, session, spans

    def trace(self, httpd, session, results=None, key=None):
        conn = connect(httpd)
        conn.request(
            "POST", "/api/update_settings",
            body=json.dumps({"settings": session.settings}),
            headers={"Content-Type": "application/json",
                     "X-Session-Token": session.session_token},
        )
        status = conn.getresponse()
        status.read()
        if results is not None:
            results[key] = status.status
        conn.close()

    @pytest.mark.parametrize("path", ["/app.js", "/styles.css"])
    def test_a_running_trace_no_longer_blocks_the_server(self, slow_pipeline, path):
        """
        The interface's own assets, which is what the proxy in front of the
        server was failing to get. They never touch session state, so they do
        not even reach the lock — the only thing that used to stop them was the
        server accepting one request at a time.
        """
        httpd, session, _ = slow_pipeline
        tracing = threading.Thread(target=self.trace, args=(httpd, session))
        tracing.start()
        time.sleep(0.1)                          # let the trace take the lock

        started = time.perf_counter()
        conn = connect(httpd)
        conn.request("GET", path)
        response = conn.getresponse()
        elapsed = time.perf_counter() - started
        assert response.status == 200
        response.read()
        conn.close()
        tracing.join(timeout=10)

        # The trace still has ~0.4s to run. Answering inside that window is the
        # whole point; the old server could not answer at all until it finished.
        assert elapsed < 0.3, f"{path} waited {elapsed:.2f}s behind a trace"

    def test_a_settings_read_waits_rather_than_tearing(self, slow_pipeline):
        """
        The other half of the trade. `/api/session` hands back the live settings
        dictionary and the caller serialises it later, so it is answered under
        the lock even though that means waiting out a trace. It is fetched once
        at page load; a torn reply would cost far more than the wait.
        """
        httpd, session, _ = slow_pipeline
        tracing = threading.Thread(target=self.trace, args=(httpd, session))
        tracing.start()
        time.sleep(0.1)

        started = time.perf_counter()
        conn = connect(httpd)
        conn.request("GET", "/api/session",
                     headers={"X-Session-Token": session.session_token})
        response = conn.getresponse()
        elapsed = time.perf_counter() - started
        assert response.status == 200
        json.loads(response.read())
        conn.close()
        tracing.join(timeout=10)

        assert elapsed >= 0.3, "/api/session answered while a trace held the lock"

    def test_cancel_does_not_queue_behind_the_trace_it_cancels(self, slow_pipeline):
        """The one request a user sends *because* the trace is slow."""
        httpd, session, _ = slow_pipeline
        tracing = threading.Thread(target=self.trace, args=(httpd, session))
        tracing.start()
        time.sleep(0.1)

        started = time.perf_counter()
        conn = connect(httpd)
        conn.request("POST", "/api/cancel", body="{}",
                     headers={"Content-Type": "application/json",
                              "X-Session-Token": session.session_token})
        assert conn.getresponse().status == 200
        elapsed = time.perf_counter() - started
        conn.close()
        tracing.join(timeout=10)

        assert session.is_cancelled
        assert elapsed < 0.3, f"/api/cancel waited {elapsed:.2f}s"

    def test_two_traces_never_run_at_once(self, slow_pipeline):
        """
        Concurrency must not reach the session document. Two pipeline runs
        interleaving their writes to one settings dict would make the result
        depend on which finished first, which §34.30 does not survive.
        """
        httpd, session, spans = slow_pipeline
        results = {}
        threads = [
            threading.Thread(target=self.trace, args=(httpd, session, results, key))
            for key in ("first", "second")
        ]
        for thread in threads:
            thread.start()
        for thread in threads:
            thread.join(timeout=10)

        assert results == {"first": 200, "second": 200}
        assert len(spans) == 2
        first, second = sorted(spans)
        assert first[1] <= second[0], "two traces overlapped"


class TestShutdown:
    """
    The serving loop ends when the session does. The loop it replaces called
    `handle_request` in a `while not applied` check, so it noticed Apply only
    when some *later* request happened to arrive — on a quiet page, never.
    """

    def free_port(self) -> int:
        import socket

        with socket.socket() as probe:
            probe.bind(("127.0.0.1", 0))
            return probe.getsockname()[1]

    def test_applying_stops_the_server_and_still_answers(self):
        session = AppSession()
        session.settings = create_default_settings("test-uuid")
        session.image_source = tiny_source()
        port = self.free_port()
        applied = []

        hosting = threading.Thread(
            target=lambda: applied.append(
                launch_palette_trace_app(session, open_browser=False, port=port)
            ),
            daemon=True,
        )
        hosting.start()

        deadline = time.time() + 5
        while time.time() < deadline:
            try:
                conn = http.client.HTTPConnection("127.0.0.1", port, timeout=5)
                conn.request("POST", "/api/apply", body="{}",
                             headers={"Content-Type": "application/json",
                                      "X-Session-Token": session.session_token})
                response = conn.getresponse()
                break
            except OSError:
                time.sleep(0.05)
        else:
            pytest.fail("the server never came up")

        # The request that ends the session must still receive its reply: the
        # interface reads this response to know the trace was committed.
        assert response.status == 200
        assert json.loads(response.read())["status"] == "applied"
        conn.close()

        hosting.join(timeout=5)
        assert not hosting.is_alive(), "the server outlived the session"
        assert applied == [True]

    def test_cancelling_stops_the_server_and_reports_it(self):
        session = AppSession()
        session.settings = create_default_settings("test-uuid")
        port = self.free_port()
        outcome = []

        hosting = threading.Thread(
            target=lambda: outcome.append(
                launch_palette_trace_app(session, open_browser=False, port=port)
            ),
            daemon=True,
        )
        hosting.start()
        time.sleep(0.2)

        session.is_cancelled = True
        hosting.join(timeout=5)
        assert not hosting.is_alive()
        assert outcome == [False]


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
