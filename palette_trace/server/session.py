"""
Session token and active document state manager.
"""

import secrets
import threading
from typing import Optional

#: How the session's current bitmap arrived. A host that was given a path on
#: the command line (or an Inkscape `<image>`) can commit back to that
#: location; one the user loaded in the browser has no such path, so its
#: result is delivered as a download instead (§9.4.2).
ORIGIN_HOST = "host"
ORIGIN_UPLOAD = "upload"

#: Where a committed result goes. The interface names its primary action after
#: this — "Add to drawing", "Save SVG file" or "Download SVG" — because a
#: button that does not say where the result lands is a button users hesitate
#: over.
COMMIT_TO_DOCUMENT = "document"
COMMIT_TO_FILE = "file"
COMMIT_TO_DOWNLOAD = "download"


class AppSession:
    """Stores session token and runtime pipeline state."""

    def __init__(self, image_elem=None, doc_path: Optional[str] = None):
        # 128-bit entropy session token
        self.session_token = secrets.token_hex(16)
        self.image_elem = image_elem
        self.doc_path = doc_path
        self.image_source = None
        self.settings = None
        self.controller = None
        self.pipeline_output = None
        self._is_applied = False
        self._is_cancelled = False
        #: Set the moment the session reaches either terminal state. The server
        #: serves requests on worker threads, so it needs something to wait on
        #: rather than a flag to re-read between requests; the two flags stay
        #: plain assignments for every caller that only writes them.
        self.finished = threading.Event()
        #: Held for the duration of any request that runs the pipeline or
        #: replaces the bitmap. Concurrency exists so that a slow trace stops
        #: blocking the whole server, not so that two traces can interleave
        #: their writes to this session (§34.30 would not survive that).
        self.pipeline_lock = threading.RLock()
        #: True when a recorded source fingerprint no longer matches (§27).
        #: The interface must offer the recovery choices before applying.
        self.source_changed = False
        #: Display name for the current bitmap, shown in the interface. Never a
        #: filesystem path — §9.1 keeps local paths out of browser-visible data
        #: unless essential.
        self.image_name = ""
        self.origin = ORIGIN_HOST
        #: Where a committed result is written, when the host has somewhere to
        #: write it. None means the result can only leave as a download.
        self.output_path = None
        #: Set when an oversized upload was resized to stay traceable (§9.4.2).
        self.resize_notice = None

    @property
    def is_applied(self) -> bool:
        return self._is_applied

    @is_applied.setter
    def is_applied(self, value: bool) -> None:
        self._is_applied = bool(value)
        if self._is_applied:
            self.finished.set()

    @property
    def is_cancelled(self) -> bool:
        return self._is_cancelled

    @is_cancelled.setter
    def is_cancelled(self, value: bool) -> None:
        self._is_cancelled = bool(value)
        if self._is_cancelled:
            self.finished.set()

    @property
    def has_image(self) -> bool:
        return self.image_source is not None

    @property
    def commit_target(self) -> str:
        """Where applying this session's result will put it."""
        if self.image_elem is not None:
            return COMMIT_TO_DOCUMENT
        if self.origin == ORIGIN_HOST and self.output_path is not None:
            return COMMIT_TO_FILE
        return COMMIT_TO_DOWNLOAD

    @property
    def can_write_to_disk(self) -> bool:
        """True when Apply has somewhere of its own to commit to."""
        return self.commit_target != COMMIT_TO_DOWNLOAD

    @property
    def can_load_image(self) -> bool:
        """
        True when the user may choose a different bitmap in the interface.

        The Inkscape host is bound to a selected `<image>` in an open document
        — its settings live on that element (§10.2) and its result is committed
        beside it — so swapping the bitmap underneath it is not offered.
        """
        return self.image_elem is None

    def validate_token(self, token: str) -> bool:
        return secrets.compare_digest(self.session_token, token)
