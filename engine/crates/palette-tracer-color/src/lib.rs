//! Colour management, palettes, and assignment for the Palette Tracer Engine
//! (§7).
//!
//! The pieces, in the order the pipeline uses them:
//!
//! 1. [`sampler`] turns a validated [`ImageView`](palette_tracer_core::ImageView)
//!    into linear-light premultiplied samples, consuming the declared encoding
//!    and alpha mode once and for all (PTE-COLOR-001).
//! 2. [`spaces`] holds the colour spaces and the conversions between them, as
//!    distinct newtypes so mixing them does not compile (PTE-COLOR-003).
//! 3. [`alpha`] enforces that a transparent pixel has no colour influence
//!    (PTE-COLOR-008/009).
//! 4. [`palette`] holds the reach model and the memory-bounded assignment
//!    (§7.4, §7.5).
//! 5. [`auto`] derives a palette deterministically when the caller supplies
//!    none (§7.6).
//! 6. [`stats`] maintains the sufficient statistics region merging needs
//!    (§7.7).
//!
//! # What this crate is not
//!
//! PTE-COLOR-021: "Automatic palette generation is an initialization/appearance
//! model. It MUST NOT replace spatial segmentation with a global
//! nearest-centroid label map." The label plane this crate produces is colour
//! evidence for segmentation, not a partition. `palette-tracer-segment` is
//! what turns it into regions, and PTE-NO-002 is the requirement that keeps
//! anyone from shipping the shortcut.
//!
//! # Provenance
//!
//! Written from `engine/SPEC.md` §7 and the public references it cites,
//! principally Ottosson's OKLab definition (§42.6). No code was ported from
//! the GPL Python application in this repository (PTE-LIC-002/004, ADR-0003).

pub mod alpha;
pub mod auto;
pub mod palette;
pub mod sampler;
pub mod spaces;
pub mod stats;

pub use alpha::{ALPHA_CHANNEL_WEIGHT, PremulLinearRgba};
pub use auto::{ColorHistogram, histogram};
pub use palette::{Assignment, Match, Palette, PaletteEntry, assign, assign_one};
pub use sampler::sample_premultiplied;
pub use spaces::{DELTA_E_CONVENTION, EncodedSrgb, LinearRgb, Oklab, Oklch};
pub use stats::ColorStats;
