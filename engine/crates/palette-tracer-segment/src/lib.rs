//! Segmentation: from pixels to intentional regions (§8).
//!
//! §8.1 states the principle this crate exists to enforce: "A quantized pixel
//! label map is useful evidence, but it is not the final partition." The
//! colour crate produces that evidence; this crate turns it into regions by
//! combining colour likelihood, local edge evidence, connectivity, scale, and
//! region role (PTE-SEG-001).
//!
//! The stages, in order:
//!
//! 1. [`edges`] builds the edge field (§8.2) from linear-light and OKLab
//!    evidence, never from gamma-encoded differences (PTE-SEG-004).
//! 2. [`partition`] builds the initial partition as a minimum-spanning forest
//!    over the pixel graph (§8.3), assigning every eligible pixel
//!    (PTE-SEG-006).
//! 3. [`rag`] builds the region adjacency graph (§8.4), merges by the §8.5
//!    objective, absorbs antialias fringe (§8.6), and protects thin features
//!    (§8.7).
//!
//! # The output contract
//!
//! PTE-SEG-002: an exclusive owner for every in-domain pixel. PTE-SEG-003:
//! removing a small region reassigns its raster support before topology
//! extraction -- omitting a completed vector path and leaving a hole is
//! forbidden (PTE-NO-007). Both are checked by
//! [`Segmentation::assert_complete_ownership`].
//!
//! # Not implemented
//!
//! Tiled segmentation with halos (§8.8, PTE-SEG-018/019) is not implemented.
//! Everything here processes the whole image in one pass, which is correct but
//! does not meet §19.5's streaming goal.
//! `docs/IMPLEMENTATION_STATUS.md` records it.

pub mod edges;
pub mod partition;
pub mod rag;

pub use edges::{EdgeField, PixelEvidence};
pub use partition::Partition;
pub use rag::{FringeAudit, MergeOutcome, RegionGraph, RegionSummary};

use palette_tracer_core::config::EffectiveSegmentation;
use palette_tracer_core::control::{ProgressEvent, Stage, TraceControl};
use palette_tracer_core::error::{TraceError, ValidationError};
use palette_tracer_core::image::ImageView;
use palette_tracer_core::labels::LabelPlane;
use palette_tracer_core::limits::{ResourceLimits, WorkBudget};

/// The finished partition and everything measured about it.
#[derive(Debug, Clone)]
pub struct Segmentation {
    /// One region label per pixel, densely numbered in row-major order of
    /// first appearance.
    pub labels: LabelPlane,
    /// Number of regions.
    pub region_count: u32,
    /// Summaries in the same order the labels number them.
    pub summaries: Vec<RegionSummary>,
    /// Regions before merging, for the report.
    pub initial_region_count: u32,
    /// What merging did.
    pub merge_outcome: MergeOutcome,
    /// Fringe reassignments, with their evidence (PTE-SEG-016).
    pub fringe: Vec<FringeAudit>,
}

impl Segmentation {
    /// Verify that every pixel has exactly one owner (PTE-SEG-002).
    ///
    /// This is the §31.1 gate "Unassigned pixels in shared partition: 0"
    /// evaluated where the partition is produced, rather than trusted.
    ///
    /// # Errors
    ///
    /// [`ValidationError::UnassignedPixels`] if any label is outside the
    /// region range, or [`ValidationError::Topology`] if a region has no
    /// pixels -- which would mean the dense numbering is wrong and a later
    /// stage would index a summary that describes nothing.
    pub fn assert_complete_ownership(&self) -> Result<(), ValidationError> {
        let counts = self.labels.histogram(self.region_count as usize);
        let total: u64 = counts.iter().sum();
        let expected = palette_tracer_core::checked::usize_to_u64(self.labels.len());
        if total != expected {
            return Err(ValidationError::UnassignedPixels {
                count: expected.saturating_sub(total),
            });
        }
        if let Some(empty) = counts.iter().position(|&c| c == 0) {
            return Err(ValidationError::Topology {
                check: "dense_region_numbering",
                detail: format!("region {empty} has no pixels"),
            });
        }
        if self.summaries.len() != self.region_count as usize {
            return Err(ValidationError::Topology {
                check: "summary_count",
                detail: format!(
                    "{} summaries for {} regions",
                    self.summaries.len(),
                    self.region_count
                ),
            });
        }
        Ok(())
    }
}

/// Run the whole of §8: edge field, partition, graph, merge, fringe.
///
/// `claims` is the palette assignment from the colour crate, and
/// `pinned_claims` says which palette entries are pinned, indexed by label.
///
/// # Errors
///
/// [`TraceError::Cancelled`], [`TraceError::ResourceLimit`],
/// [`TraceError::Decode`] for an allocation, or [`TraceError::Validation`] if
/// the finished partition does not own every pixel.
// The arity is the §5.3 stage contract: pixels, the analysis products, the
// policy, the limits, the budget, and the control. Bundling them into a
// context struct would hide which stage reads what, and PTE-ARCH-008 wants
// each stage's inputs to be an explicit typed contract.
#[allow(clippy::too_many_arguments)]
pub fn segment(
    view: &ImageView<'_>,
    claims: &LabelPlane,
    pinned_claims: &[bool],
    color: &palette_tracer_core::config::EffectiveColor,
    policy: &EffectiveSegmentation,
    limits: &ResourceLimits,
    budget: &WorkBudget,
    control: &dyn TraceControl,
) -> Result<Segmentation, TraceError> {
    budget.enter(Stage::Segment);
    control.progress(ProgressEvent::StageStarted {
        stage: Stage::Segment,
    });

    let evidence = edges::evidence_from(view, claims, color, budget, control)?;
    let field = edges::build(&evidence, policy, budget, control)?;
    let initial = partition::build(&field, policy, limits, budget, control)?;
    let initial_region_count = initial.region_count;

    control.progress(ProgressEvent::Counted {
        stage: Stage::Segment,
        name: "initial_regions",
        value: u64::from(initial_region_count),
    });

    let mut graph = rag::build(
        &initial.labels,
        initial.region_count,
        &evidence,
        &field,
        pinned_claims,
        budget,
        control,
    )?;

    let mut merge_outcome = rag::merge(&mut graph, policy, budget, control)?;
    // PTE-SEG-015: fringe cleanup happens before topology construction, so a
    // fringe fragment never becomes a face with its own shared boundary. One
    // reassignment can expose a candidate to its two true neighbours, so sweep
    // to a fixed point instead of making the result depend on a single pass.
    let fringe = rag::reassign_fringe_to_fixed_point(&mut graph, policy, budget, control)?;
    merge_outcome.fringe = fringe.clone();

    let (labels, region_count) = rag::apply(&graph, &initial.labels)?;
    limits.check_regions(u64::from(region_count))?;
    let summaries = rag::live_summaries(&graph, &initial.labels);

    let segmentation = Segmentation {
        labels,
        region_count,
        summaries,
        initial_region_count,
        merge_outcome,
        fringe,
    };
    segmentation.assert_complete_ownership()?;

    control.progress(ProgressEvent::Counted {
        stage: Stage::Segment,
        name: "final_regions",
        value: u64::from(region_count),
    });
    control.progress(ProgressEvent::StageFinished {
        stage: Stage::Segment,
    });

    Ok(segmentation)
}
