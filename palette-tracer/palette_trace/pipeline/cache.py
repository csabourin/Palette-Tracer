"""
Caching manager for decoded source images, color maps, and label maps.
"""

from typing import Dict, Any, Optional

class PipelineCache:
    """Stores intermediate pipeline states to accelerate interactive UI updates."""

    def __init__(self):
        self.source_fingerprint: Optional[str] = None
        self.oklch_array = None
        self.label_map = None
        self.cleaned_masks: dict[str, Any] = {}
        self.vector_results: dict[str, Any] = {}

    def clear(self):
        self.source_fingerprint = None
        self.oklch_array = None
        self.label_map = None
        self.cleaned_masks.clear()
        self.vector_results.clear()
