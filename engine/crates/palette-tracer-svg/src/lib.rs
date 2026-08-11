//! Topology lowering and deterministic SVG serialisation (§18).
//!
//! * [`lower`] turns the shared topology into a `VectorDocument`, assembling
//!   each face from oriented references to the stored chains so the two sides
//!   of an interface come from one set of numbers (PTE-GEO-012, PTE-SVG-009).
//! * [`writer`] serialises that document from typed segments only
//!   (PTE-SVG-004), searching for a safe coordinate precision rather than
//!   fixing one (PTE-SVG-007/008).
//! * [`raster`] measures per-pixel coverage so the shared-seam gate
//!   (PTE-TOPO-015) can be checked. It is a first-party diagnostic, not a
//!   conforming renderer; §18.7's cross-renderer matrix is **not** satisfied by
//!   this build.

pub mod lower;
pub mod raster;
pub mod writer;

pub use lower::{OutlineStyle, count_segments, lower, lower_with_primitives};
pub use raster::{Coverage, coverage};
pub use writer::{
    choose_precision, choose_serialization_precision, escape_xml, format_coordinate, serialize,
};
