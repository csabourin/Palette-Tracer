"""
Session token and active document state manager.
"""

import secrets
from typing import Optional

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
        self.is_applied = False
        self.is_cancelled = False
        #: True when a recorded source fingerprint no longer matches (§27).
        #: The interface must offer the recovery choices before applying.
        self.source_changed = False

    def validate_token(self, token: str) -> bool:
        return secrets.compare_digest(self.session_token, token)
