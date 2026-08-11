//! The vector intermediate representation (§6.4).
//!
//! Three layers live here, and the distinction between them matters:
//!
//! * [`geom`] and [`paint`] are primitives.
//! * [`topology`] is the *internal* representation, "richer than the output
//!   tree" as §6.4 puts it. Faces reference oriented shared edges; reversing a
//!   traversal reverses the same chain.
//! * [`document`] is what a caller receives and what the serialiser lowers.
//!
//! Geometry stays `f64` from fitting through validation, and rounding happens
//! once, at serialisation, after the error gates pass (PTE-API-011,
//! PTE-SVG-007).

pub mod document;
pub mod geom;
pub mod paint;
pub mod topology;

pub use document::{
    Definition, DocumentProvenance, Element, FillRule, FilledFace, Group, Layer, PhysicalSize,
    Primitive, PrimitiveGeometry, PrimitiveRecognition, StrokePath, VectorDocument,
};
pub use geom::{CurveChain, PathSegment, Point, Rect, Vec2};
pub use paint::{Color, GradientStop, LinearGradient, Paint, RadialGradient, SpreadMethod};
pub use topology::{
    BoundaryConfidence, BoundaryEvidence, ChainId, Cycle, CycleKind, EdgeFlags, EdgeId, Face,
    FaceId, HalfEdge, HalfEdgeId, PaletteId, RegionId, SharedEdge, Topology, Vertex, VertexId,
};
