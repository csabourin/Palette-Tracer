//! Subpixel antialias reconstruction (§10).
//!
//! §10.1 states the reason this crate exists: "Thresholding [an antialiased
//! boundary pixel] at 50% discards useful subpixel position information;
//! treating every intermediate color as a new region produces halos and color
//! bands." §8.6 already prevents the second failure by absorbing fringe
//! regions. This crate addresses the first, by turning the coverage that
//! absorption inferred into a boundary *position*.
//!
//! The pieces, in the order the pipeline uses them:
//!
//! 1. [`normal`] estimates the boundary normal from the extracted polyline's
//!    tangents over a multi-sample window (PTE-AA-004).
//! 2. [`square`] inverts area coverage to a signed offset from a pixel centre
//!    (§10.3), exactly and in closed form.
//! 3. [`reconstruct`] reads the source raster back, turns each boundary
//!    sample's coverage into a line constraint, and solves §10.5's objective
//!    for the displaced boundary.
//!
//! # What is not here
//!
//! The mixture estimate itself (§10.2, §10.4) lives in
//! `palette_tracer_color::mixture`, because it is colour mathematics and
//! segmentation needs it too.

pub mod normal;
pub mod reconstruct;
pub mod square;

pub use normal::{Normal, normals};
pub use reconstruct::{ReconstructOptions, ReconstructionStats, reconstruct};
pub use square::{Octant, coverage_for_offset, offset_for_coverage};
