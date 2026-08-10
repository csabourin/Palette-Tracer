"""
Domain-specific exceptions for Palette Trace.
"""


class PaletteTraceError(Exception):
    """Base class for Palette Trace errors."""


class SelectionError(PaletteTraceError):
    """Raised when SVG selection is invalid (e.g. no selection or multiple selection)."""


class ImageSourceError(PaletteTraceError):
    """Raised when source bitmap cannot be loaded or resolved."""


class BackendError(PaletteTraceError):
    """Raised when backend tracing fails or is unavailable."""


class BackendNotFoundError(BackendError):
    """Raised when no compatible tracing backend is found."""
