"""
Mandatory Tracing Backend protocol and dataclasses as specified in Section 23.1.
"""

from dataclasses import dataclass
from typing import Protocol, Optional, Tuple, FrozenSet

@dataclass(frozen=True)
class BackendCapabilities:
    backend_id: str
    version: str

    supports_binary_masks: bool
    supports_holes: bool
    supports_cancellation: bool
    deterministic: bool

    supported_canonical_settings: FrozenSet[str]

@dataclass(frozen=True)
class TraceRequest:
    width: int
    height: int
    packed_binary_mask: bytes  # 1 byte per pixel or packed bitmask
    profile: dict
    transform_hint: Optional[Tuple[float, float, float, float, float, float]] = None

@dataclass(frozen=True)
class TraceResult:
    svg_path_data: Tuple[str, ...]
    fill_rule: str
    warnings: Tuple[str, ...]
    statistics: dict

class TraceBackend(Protocol):
    def capabilities(self) -> BackendCapabilities:
        ...

    def trace_mask(
        self,
        request: TraceRequest,
        cancellation_token: object = None,
    ) -> TraceResult:
        ...
