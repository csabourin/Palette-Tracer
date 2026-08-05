"""
Domain-specific exceptions for Palette Trace.
"""

class PaletteTraceError(Exception):
    """Base class for Palette Trace errors."""
    pass

class SelectionError(PaletteTraceError):
    """Raised when SVG selection is invalid (e.g. no selection or multiple selection)."""
    pass

class ImageSourceError(PaletteTraceError):
    """Raised when source bitmap cannot be loaded or resolved."""
    pass

class SchemaValidationError(PaletteTraceError):
    """Raised when settings or preset schema validation fails."""
    pass

class BackendError(PaletteTraceError):
    """Raised when backend tracing fails or is unavailable."""
    pass

class BackendNotFoundError(BackendError):
    """Raised when no compatible tracing backend is found."""
    pass
