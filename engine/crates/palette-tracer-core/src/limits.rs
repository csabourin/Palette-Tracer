//! Resource limits and the deterministic work budget (§24.1).
//!
//! PTE-SEC-006 is the requirement that shapes this module: "Limits MUST be
//! enforced incrementally. Checking only estimated input size is insufficient
//! because adversarial topology can produce extreme region/edge counts." A
//! 512x512 checkerboard is a quarter of a megapixel and has a quarter of a
//! million regions. So the limits here are checked where the quantity grows,
//! not once at the door.
//!
//! The work budget is separate from wall-clock time on purpose: time is not
//! deterministic, and §20.2 lists wall-clock seeds as a source of
//! nondeterminism. A budget denominated in abstract work units terminates a
//! runaway trace identically on every machine.

use crate::error::{ResourceLimitError, TraceError};
use crate::report::ResourceUsage;
use serde::{Deserialize, Serialize};
use std::cell::Cell;

/// Bounds on what one trace may consume.
///
/// [`ResourceLimits::default`] is sized for an interactive desktop or browser
/// session. Batch callers raise them explicitly; nothing raises them
/// automatically.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct ResourceLimits {
    /// Largest accepted width in pixels.
    pub max_width: u32,
    /// Largest accepted height in pixels.
    pub max_height: u32,
    /// Largest accepted pixel count. Bounds area independently of the side
    /// limits, so a 100000x1 strip is refused.
    pub max_pixels: u64,
    /// Largest accepted encoded input, for adapters that decode.
    pub max_input_bytes: u64,
    /// Soft ceiling on working memory, in bytes, used to size reservations.
    pub max_working_bytes: u64,
    /// Largest region count after segmentation. The adversarial bound.
    pub max_regions: u32,
    /// Largest number of boundary samples across all shared edges.
    pub max_boundary_samples: u64,
    /// Largest number of elements in the output document.
    pub max_output_elements: u32,
    /// Largest serialised output, in bytes.
    pub max_output_bytes: u64,
    /// Largest number of gradient stops across the document.
    pub max_gradient_stops: u32,
    /// Iteration cap for any bounded numerical refinement.
    pub max_iterations: u32,
    /// Total abstract work units, or `None` for unbounded.
    pub work_budget: Option<u64>,
}

impl Default for ResourceLimits {
    fn default() -> Self {
        Self {
            max_width: 32_768,
            max_height: 32_768,
            // 64 megapixels. Above this a browser tab is in trouble regardless
            // of what the engine does.
            max_pixels: 64 << 20,
            max_input_bytes: 512 << 20,
            max_working_bytes: 2 << 30,
            // §19.3 budgets assume regions are far fewer than pixels. Four
            // million admits pathological input while still bounding the RAG.
            max_regions: 4 << 20,
            max_boundary_samples: 256 << 20,
            max_output_elements: 1 << 20,
            max_output_bytes: 512 << 20,
            max_gradient_stops: 4096,
            max_iterations: 64,
            work_budget: None,
        }
    }
}

impl ResourceLimits {
    /// Limits small enough to keep a fuzz or property test fast.
    #[must_use]
    pub fn small() -> Self {
        Self {
            max_width: 4096,
            max_height: 4096,
            max_pixels: 1 << 20,
            max_input_bytes: 1 << 20,
            max_working_bytes: 64 << 20,
            max_regions: 1 << 16,
            max_boundary_samples: 1 << 22,
            max_output_elements: 1 << 16,
            max_output_bytes: 16 << 20,
            max_gradient_stops: 256,
            max_iterations: 16,
            work_budget: Some(1 << 28),
        }
    }

    /// Check image dimensions before any plane is allocated.
    ///
    /// # Errors
    ///
    /// [`ResourceLimitError::Exceeded`] naming the first limit that fails.
    pub fn check_dimensions(&self, width: u32, height: u32) -> Result<(), ResourceLimitError> {
        if width > self.max_width {
            return Err(ResourceLimitError::Exceeded {
                limit: "max_width",
                allowed: u64::from(self.max_width),
                required: u64::from(width),
            });
        }
        if height > self.max_height {
            return Err(ResourceLimitError::Exceeded {
                limit: "max_height",
                allowed: u64::from(self.max_height),
                required: u64::from(height),
            });
        }
        let pixels = u64::from(width) * u64::from(height);
        if pixels > self.max_pixels {
            return Err(ResourceLimitError::Exceeded {
                limit: "max_pixels",
                allowed: self.max_pixels,
                required: pixels,
            });
        }
        Ok(())
    }

    /// Check a count against a named limit.
    ///
    /// # Errors
    ///
    /// [`ResourceLimitError::Exceeded`] if `count` is above `allowed`.
    pub fn check_count(
        limit: &'static str,
        count: u64,
        allowed: u64,
    ) -> Result<(), ResourceLimitError> {
        if count > allowed {
            return Err(ResourceLimitError::Exceeded {
                limit,
                allowed,
                required: count,
            });
        }
        Ok(())
    }

    /// Check the region count, which grows during segmentation
    /// (PTE-SEC-006).
    ///
    /// # Errors
    ///
    /// [`ResourceLimitError::Exceeded`] for `max_regions`.
    pub fn check_regions(&self, count: u64) -> Result<(), ResourceLimitError> {
        Self::check_count("max_regions", count, u64::from(self.max_regions))
    }

    /// Check the boundary-sample count, which grows during topology
    /// extraction.
    ///
    /// # Errors
    ///
    /// [`ResourceLimitError::Exceeded`] for `max_boundary_samples`.
    pub fn check_boundary_samples(&self, count: u64) -> Result<(), ResourceLimitError> {
        Self::check_count("max_boundary_samples", count, self.max_boundary_samples)
    }

    /// Check the output element count.
    ///
    /// # Errors
    ///
    /// [`ResourceLimitError::Exceeded`] for `max_output_elements`.
    pub fn check_output_elements(&self, count: u64) -> Result<(), ResourceLimitError> {
        Self::check_count("max_output_elements", count, u64::from(self.max_output_elements))
    }
}

/// Abstract cost of one unit of work, charged by stages as they run.
///
/// The numbers are relative and deliberately coarse: they exist so that a
/// budget bounds total work in a target-independent way, not so that they
/// predict nanoseconds.
pub mod cost {
    /// One pixel visited in a linear scan.
    pub const PIXEL: u64 = 1;
    /// One pixel-graph edge relaxed in the watershed.
    pub const EDGE: u64 = 2;
    /// One priority-queue pop in region merging (Appendix D.2).
    pub const MERGE_POP: u64 = 8;
    /// One elementary boundary segment emitted.
    pub const BOUNDARY_SEGMENT: u64 = 4;
    /// One candidate curve fitted and validated (Appendix D.5).
    pub const CURVE_CANDIDATE: u64 = 32;
    /// One coverage observation inverted (§10.3).
    pub const COVERAGE_SAMPLE: u64 = 8;
}

/// Deterministic work accounting.
///
/// Interior mutability keeps `&self` in the stage signatures, so a budget can
/// be shared without threading a `&mut` through every helper. Single-threaded
/// by construction, matching §19.6's requirement that threads be an
/// acceleration and never a correctness dependency.
#[derive(Debug)]
pub struct WorkBudget {
    limit: Option<u64>,
    spent: Cell<u64>,
    stage: Cell<&'static str>,
    peak_bytes: Cell<u64>,
}

impl WorkBudget {
    /// A budget enforcing `limits.work_budget`.
    #[must_use]
    pub fn new(limits: &ResourceLimits) -> Self {
        Self {
            limit: limits.work_budget,
            spent: Cell::new(0),
            stage: Cell::new("validate"),
            peak_bytes: Cell::new(0),
        }
    }

    /// An unbounded budget that still counts, for measurement.
    #[must_use]
    pub fn unbounded() -> Self {
        Self {
            limit: None,
            spent: Cell::new(0),
            stage: Cell::new("validate"),
            peak_bytes: Cell::new(0),
        }
    }

    /// Record which stage subsequent charges belong to.
    pub fn enter(&self, stage: crate::control::Stage) {
        self.stage.set(stage.as_str());
    }

    /// Charge `units` of work.
    ///
    /// # Errors
    ///
    /// [`TraceError::ResourceLimit`] with
    /// [`ResourceLimitError::WorkBudgetExhausted`] once the budget is spent.
    /// PTE-SEC-007: the overshoot is bounded by one charge, never unbounded.
    #[inline]
    pub fn charge(&self, units: u64) -> Result<(), TraceError> {
        let spent = self.spent.get().saturating_add(units);
        self.spent.set(spent);
        if let Some(limit) = self.limit
            && spent > limit
        {
            return Err(TraceError::ResourceLimit(
                ResourceLimitError::WorkBudgetExhausted {
                    budget: limit,
                    stage: self.stage.get(),
                },
            ));
        }
        Ok(())
    }

    /// Note that `bytes` of working memory are live, tracking the high-water
    /// mark for the report (§26.8).
    ///
    /// This records what the engine believes it allocated. It is not an
    /// allocator trace, and PTE-PERF-002's lifetime proof is not satisfied by
    /// it; `docs/IMPLEMENTATION_STATUS.md` says so.
    pub fn note_bytes(&self, bytes: u64) {
        if bytes > self.peak_bytes.get() {
            self.peak_bytes.set(bytes);
        }
    }

    /// Work units charged so far.
    #[must_use]
    pub fn spent(&self) -> u64 {
        self.spent.get()
    }

    /// Statistics for the report.
    #[must_use]
    pub fn statistics(&self) -> ResourceUsage {
        ResourceUsage {
            work_units: self.spent.get(),
            work_budget: self.limit,
            peak_working_bytes: self.peak_bytes.get(),
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::float_cmp, clippy::field_reassign_with_default)]
mod tests {
    use super::*;
    use crate::control::Stage;

    #[test]
    fn dimension_limits_are_checked_before_area() {
        let limits = ResourceLimits::small();
        let err = limits.check_dimensions(9999, 10).unwrap_err();
        assert_eq!(
            err,
            ResourceLimitError::Exceeded {
                limit: "max_width",
                allowed: 4096,
                required: 9999
            }
        );
    }

    /// A long thin strip is within both side limits and still far too large.
    /// PTE-SEC-006 is why `max_pixels` exists separately.
    #[test]
    fn area_is_bounded_independently_of_the_sides() {
        let limits = ResourceLimits::small();
        assert!(limits.check_dimensions(4096, 4096).is_err());
        assert!(limits.check_dimensions(1024, 1024).is_ok());
    }

    /// The adversarial case PTE-SEC-006 names: a 512x512 checkerboard is a
    /// quarter of a megapixel -- comfortably inside every dimension limit --
    /// and has 262144 single-pixel regions. Checking only the input size would
    /// wave it through, so the region count is checked where it grows.
    #[test]
    fn region_count_is_bounded_after_the_image_was_accepted() {
        let limits = ResourceLimits::small();
        assert!(
            limits.check_dimensions(512, 512).is_ok(),
            "the image itself is small and must be accepted"
        );
        assert_eq!(
            limits.check_regions(262_144).unwrap_err(),
            ResourceLimitError::Exceeded {
                limit: "max_regions",
                allowed: 65_536,
                required: 262_144,
            },
            "the checkerboard's region count must be what stops it"
        );
        // The default limits are wide enough for that image; they still bound.
        let default = ResourceLimits::default();
        assert!(default.check_regions(262_144).is_ok());
        assert!(default.check_regions(70_000_000).is_err());
    }

    #[test]
    fn the_work_budget_stops_a_runaway_stage() {
        let mut limits = ResourceLimits::small();
        limits.work_budget = Some(100);
        let budget = WorkBudget::new(&limits);
        budget.enter(Stage::Segment);

        let mut charged = 0u64;
        let outcome = (0..1000).try_for_each(|_| {
            budget.charge(cost::MERGE_POP)?;
            charged += cost::MERGE_POP;
            Ok::<(), TraceError>(())
        });

        let err = outcome.unwrap_err();
        assert_eq!(
            err,
            TraceError::ResourceLimit(ResourceLimitError::WorkBudgetExhausted {
                budget: 100,
                stage: "segment",
            })
        );
        // PTE-SEC-007: overshoot is bounded by one charge.
        assert!(budget.spent() <= 100 + cost::MERGE_POP);
    }

    #[test]
    fn an_unbounded_budget_still_counts() {
        let budget = WorkBudget::unbounded();
        budget.charge(cost::PIXEL * 1000).unwrap();
        assert_eq!(budget.spent(), 1000);
        assert_eq!(budget.statistics().work_budget, None);
    }

    #[test]
    fn peak_bytes_is_a_high_water_mark_not_a_last_value() {
        let budget = WorkBudget::unbounded();
        budget.note_bytes(1000);
        budget.note_bytes(10);
        assert_eq!(budget.statistics().peak_working_bytes, 1000);
    }
}
