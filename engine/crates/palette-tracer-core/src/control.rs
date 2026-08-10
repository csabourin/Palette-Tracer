//! Cooperative cancellation and progress (PTE-API-006).
//!
//! The requirement has a detail that is easy to miss: "Cancellation checks
//! MUST occur at bounded work intervals, not merely between full-image
//! stages." A caller who cancels a 24-megapixel watershed must not wait for
//! the watershed to finish. Every stage therefore calls
//! [`TraceControl::is_cancelled`] inside its inner loop on a fixed stride, and
//! the [`crate::limits::WorkBudget`] charges the same loop so a runaway stage
//! also terminates.
//!
//! Callbacks are invoked with no engine lock held, because the engine holds no
//! locks -- stages own their data outright.

use core::fmt;

/// Pipeline stage, as named by the §5.3 table.
///
/// Reported in progress events and in resource errors so a caller can say
/// *where* a long trace is, not merely that it is running.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[non_exhaustive]
pub enum Stage {
    /// A. Validate and normalise.
    Validate,
    /// B. Analyse: colour statistics, edge field, palette model.
    Analyze,
    /// C. Segment into an exclusive label map and region graph.
    Segment,
    /// D. Build the shared topology graph.
    Topology,
    /// E. Refine boundaries to subpixel positions.
    Boundaries,
    /// F. Fit the hybrid representation.
    Fit,
    /// G. Apply destination policy.
    Destination,
    /// H. Serialise.
    Serialize,
    /// I. Validate output.
    Validation,
}

impl Stage {
    /// Stable lowercase identifier, used in reports and error payloads.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Validate => "validate",
            Self::Analyze => "analyze",
            Self::Segment => "segment",
            Self::Topology => "topology",
            Self::Boundaries => "boundaries",
            Self::Fit => "fit",
            Self::Destination => "destination",
            Self::Serialize => "serialize",
            Self::Validation => "validation",
        }
    }
}

impl fmt::Display for Stage {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Something worth telling the caller about, mid-trace.
///
/// Progress is advisory. Nothing in the engine's output depends on whether a
/// caller observes these, which is what keeps a progress callback from
/// becoming a source of nondeterminism (§20.2).
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub enum ProgressEvent {
    /// A stage began.
    StageStarted {
        /// Which stage.
        stage: Stage,
    },
    /// A stage finished.
    StageFinished {
        /// Which stage.
        stage: Stage,
    },
    /// Fractional progress within a stage, in `[0, 1]`.
    StageProgress {
        /// Which stage.
        stage: Stage,
        /// How far through, clamped to `[0, 1]`.
        fraction: f64,
    },
    /// A quantity worth surfacing became known, e.g. the region count after
    /// merging.
    Counted {
        /// Which stage produced it.
        stage: Stage,
        /// Stable snake_case name of the quantity.
        name: &'static str,
        /// Its value.
        value: u64,
    },
}

/// Caller-supplied cancellation and progress sink.
///
/// `Sync` because stages may consult it from more than one thread once §19.6
/// parallelism exists; today every call is from the calling thread.
pub trait TraceControl: Sync {
    /// Whether the caller has asked the trace to stop.
    ///
    /// Called on a bounded stride inside stage inner loops. Implementations
    /// must be cheap -- an atomic load, not a lock or a syscall.
    fn is_cancelled(&self) -> bool;

    /// Receive a progress event. The default drops it.
    fn progress(&self, event: ProgressEvent) {
        let _ = event;
    }
}

/// A control that never cancels and discards progress.
///
/// The default for callers that do not care, and the one used throughout the
/// test suite.
#[derive(Debug, Clone, Copy, Default)]
pub struct NoControl;

impl TraceControl for NoControl {
    fn is_cancelled(&self) -> bool {
        false
    }
}

/// How many inner-loop iterations may pass between cancellation checks.
///
/// Chosen so a check costs well under a percent of a per-pixel loop while
/// keeping worst-case cancellation latency on the order of a millisecond: at
/// roughly 10^8 simple pixel operations per second, 4096 iterations is about
/// 40 microseconds.
pub const CANCEL_CHECK_STRIDE: usize = 4096;

/// Return [`crate::TraceError::Cancelled`] if the caller has cancelled, and
/// only consult the control every [`CANCEL_CHECK_STRIDE`] iterations.
///
/// ```
/// # use palette_tracer_core::control::{check_cancel, NoControl, CANCEL_CHECK_STRIDE};
/// # fn main() -> Result<(), palette_tracer_core::TraceError> {
/// let control = NoControl;
/// for i in 0..10_000usize {
///     check_cancel(&control, i)?;
/// }
/// # Ok(())
/// # }
/// ```
///
/// # Errors
///
/// [`crate::TraceError::Cancelled`] when the control reports cancellation.
#[inline]
pub fn check_cancel(
    control: &dyn TraceControl,
    iteration: usize,
) -> Result<(), crate::error::TraceError> {
    if iteration.is_multiple_of(CANCEL_CHECK_STRIDE) && control.is_cancelled() {
        return Err(crate::error::TraceError::Cancelled);
    }
    Ok(())
}

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::float_cmp,
    clippy::field_reassign_with_default
)]
mod tests {
    use super::*;
    use crate::error::TraceError;
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

    #[derive(Default)]
    struct Recorder {
        cancel_after: AtomicUsize,
        queries: AtomicUsize,
        cancelled: AtomicBool,
        events: std::sync::Mutex<Vec<ProgressEvent>>,
    }

    impl TraceControl for Recorder {
        fn is_cancelled(&self) -> bool {
            let n = self.queries.fetch_add(1, Ordering::Relaxed);
            if n >= self.cancel_after.load(Ordering::Relaxed) {
                self.cancelled.store(true, Ordering::Relaxed);
            }
            self.cancelled.load(Ordering::Relaxed)
        }
        fn progress(&self, event: ProgressEvent) {
            #[allow(clippy::unwrap_used)]
            self.events.lock().unwrap().push(event);
        }
    }

    /// PTE-API-006's real requirement: cancellation lands *inside* a stage.
    /// A loop of a million iterations must stop early, not run to completion.
    #[test]
    fn cancellation_takes_effect_inside_a_long_loop() {
        let control = Recorder::default();
        control.cancel_after.store(2, Ordering::Relaxed);

        let mut completed = 0usize;
        let outcome = (0..1_000_000usize).try_for_each(|i| {
            check_cancel(&control, i)?;
            completed = i;
            Ok::<(), TraceError>(())
        });

        assert_eq!(outcome.unwrap_err(), TraceError::Cancelled);
        assert!(
            completed < 1_000_000,
            "cancellation must interrupt the loop, not wait for it"
        );
        assert!(
            completed <= 3 * CANCEL_CHECK_STRIDE,
            "latency must be bounded by the check stride, got {completed}"
        );
    }

    #[test]
    fn a_control_that_never_cancels_lets_the_loop_finish() {
        let control = NoControl;
        for i in 0..100_000usize {
            assert!(check_cancel(&control, i).is_ok());
        }
    }

    #[test]
    fn progress_events_reach_the_sink() {
        let control = Recorder::default();
        control.cancel_after.store(usize::MAX, Ordering::Relaxed);
        control.progress(ProgressEvent::StageStarted {
            stage: Stage::Segment,
        });
        control.progress(ProgressEvent::Counted {
            stage: Stage::Segment,
            name: "regions",
            value: 42,
        });
        let events = control.events.lock().unwrap();
        assert_eq!(events.len(), 2);
        assert_eq!(
            events[1],
            ProgressEvent::Counted {
                stage: Stage::Segment,
                name: "regions",
                value: 42
            }
        );
    }

    #[test]
    fn stage_names_are_stable() {
        assert_eq!(Stage::Topology.as_str(), "topology");
        assert_eq!(Stage::Boundaries.to_string(), "boundaries");
    }
}
