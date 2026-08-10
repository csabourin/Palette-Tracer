//! Host-free types and contracts for the Palette Tracer Engine (PTE).
//!
//! This crate owns everything the pipeline stages need to name their inputs and
//! outputs, and depends on no other crate in the workspace. Orchestration lives
//! in the `palette-tracer` facade; see `docs/decisions/ADR-0002-crate-split.md`
//! for why the crate called "core" does not contain `trace`.
//!
//! # Host isolation (PTE-ARCH-001)
//!
//! Nothing here touches the filesystem, the network, the wall clock, a process,
//! a UI, or an image codec. The whole crate builds for
//! `wasm32-unknown-unknown` without OS stubs (PTE-ARCH-003).
//!
//! # Conventions
//!
//! * Pixel centres are at `(x + 0.5, y + 0.5)`; the image domain is
//!   `[0, width] x [0, height]` (PTE-API-002).
//! * Geometry is `f64` from fitting through validation; rounding happens once,
//!   at serialisation, and only after the error gates pass (PTE-API-011).
//! * Any comparison that decides something goes through
//!   [`determinism::TotalOrdKey`] rather than `<` on a float (PTE-DET-002).
//!
//! # Provenance
//!
//! Written from `engine/SPEC.md` and the public references it cites. No code
//! was ported from the GPL Python application in this repository
//! (PTE-LIC-002/004, `ADR-0003`).

pub mod checked;
pub mod config;
pub mod control;
pub mod determinism;
pub mod digest;
pub mod error;
pub mod image;
pub mod ir;
pub mod labels;
pub mod limits;
pub mod report;

pub use control::{ProgressEvent, Stage, TraceControl};
pub use error::{ConfigError, TraceError, ValidationError};
pub use image::{AlphaMode, ColorEncoding, ImageView, PixelFormat};
pub use ir::VectorDocument;
pub use labels::{LabelId, LabelPlane};
pub use limits::{ResourceLimits, WorkBudget};
pub use report::TraceReport;

/// Version of this engine build, as reported in [`TraceReport`].
pub const ENGINE_VERSION: &str = env!("CARGO_PKG_VERSION");

/// Engine name as it appears in the report and in SVG metadata.
pub const ENGINE_NAME: &str = "palette-tracer-engine";
