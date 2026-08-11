//! Corner classification, curve fitting, and chain segmentation (§11).
//!
//! This crate turns the grid-aligned polylines the topology extractor produces
//! into the simplest editable geometry that still satisfies the profile's
//! fidelity budget — §11.1's design objective, and the largest single gap in
//! the build before it existed.
//!
//! # Where it sits
//!
//! [`fit_topology`] rewrites [`Topology::chains`] in place. It does not touch
//! vertices, half-edges, faces or cycles: a chain's endpoints are its junction
//! vertices, every model is built with the span's own endpoints, and so the
//! graph is unchanged by construction. That is PTE-GEO-012 and PTE-AA-008 made
//! structural rather than checked — there is one chain per shared edge, it is
//! fitted once, and both faces inherit it through
//! [`Topology::oriented_chain`].
//!
//! The caller must still rerun the topology validator afterwards
//! (PTE-GEO-014); [`fit_topology`] is a geometry operation and does not
//! validate the graph it was handed.
//!
//! # Reverse equivalence (§25.4, §27.2)
//!
//! §25.4 lists "reverse-chain equivalence" as a mandatory property for this
//! module: fitting a chain and reversing it must give the same answer as
//! reversing it and fitting. Least squares is not symmetric under reversal —
//! the parameterisation, the accumulation order and the tie-breaks all differ —
//! so the property is obtained rather than hoped for. Each chain is rotated
//! into a canonical direction before fitting and rotated back afterwards, so
//! the two traversals of one interface are literally the same computation
//! (see [`canonical_orientation`]).
//!
//! # What is not here
//!
//! * **Arcs.** §11.4 lists a circular arc as an optional model between the line
//!   and the cubic. This build fits lines and cubics only. `allow_arcs` is
//!   reported as unimplemented rather than accepted and ignored (PTE-NO-042),
//!   and the profile that would ask for it is told so.
//! * **Primitive recognition (§11.7).** Complete circles are recognised when
//!   support, displacement and complexity gates pass. Rectangles, ellipses,
//!   rounded rectangles, polygons and repeated radii remain absent.
//! * **Subpixel evidence (§10).** Sample positions come from the pixel grid.
//!   Fitting makes the geometry *compact and smooth*; it does not make it
//!   *subpixel-accurate*, and the report keeps saying `crisp_grid` because that
//!   is still where the evidence comes from.
//!
//! # Design note
//!
//! `docs/notes/curve-fitting.md` carries the §34.2 contents: mathematics,
//! invariants, complexity, tie rules, cancellation, provenance, alternatives
//! and limitations.

pub mod chain;
pub mod curves;
pub mod models;
pub mod primitives;
pub mod segmentation;

use palette_tracer_core::control::{TraceControl, check_cancel};
use palette_tracer_core::error::TraceError;
use palette_tracer_core::ir::{ChainId, CurveChain, Point, Topology};

pub use chain::{Samples, detect_corners};
pub use primitives::{PrimitiveOptions, PrimitiveStats, recognize_primitives};
pub use segmentation::{Fitted, fit_chain};

/// Settings the fitter reads (§11, Appendix G.1).
///
/// A projection of `EffectiveGeometry`, so this crate does not depend on the
/// configuration model and can be exercised from a test with three lines of
/// setup.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FitOptions {
    /// Boundary error budget in source pixels (PTE-GEO-008).
    pub tolerance_px: f64,
    /// Turning angle above which a corner may be declared.
    pub corner_threshold_deg: f64,
    /// Hysteresis band around that threshold (PTE-GEO-004).
    pub corner_hysteresis_deg: f64,
    /// Whether arcs are permitted. **Not implemented**; see the module
    /// documentation.
    pub allow_arcs: bool,
    /// Cap on predecessor proposals per position (PTE-GEO-009).
    pub max_span_candidates: u32,
}

impl FitOptions {
    /// Whether fitting is enabled at all.
    ///
    /// A zero or negative tolerance is not a very tight fit: it is the
    /// `pixel-art` profile saying the pixel grid *is* the intended geometry
    /// (§15). Fitting is skipped and the polylines stand.
    #[must_use]
    pub fn enabled(&self) -> bool {
        self.tolerance_px.is_finite() && self.tolerance_px > 0.0
    }
}

/// What a whole-topology fit produced, for the report (§26.7).
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct FitStats {
    /// Chains replaced by fitted geometry.
    pub chains_fitted: u32,
    /// Chains where the search ran out of budget: PTE-GEO-005's declared
    /// fallback, and the only one of the three that is a *failure*.
    pub polyline_fallbacks: u32,
    /// Chains where the search completed and found nothing shorter than the
    /// polyline it started from.
    ///
    /// Kept separate from [`FitStats::polyline_fallbacks`] because the two read
    /// completely differently. A short jagged fringe boundary genuinely has no
    /// simpler faithful representation, and counting that as a fallback would
    /// report a search failure where the search in fact succeeded and returned
    /// the right answer.
    pub chains_already_minimal: u32,
    /// Chains with nothing to fit: fewer than three samples, or already
    /// carrying fitted geometry.
    pub chains_trivial: u32,
    /// Corners promoted to fitting pins (PTE-GEO-003).
    pub corners_pinned: u32,
    /// Candidate spans evaluated, the instrumentation PTE-GEO-005 requires.
    pub candidates_evaluated: u64,
    /// Path segments before fitting.
    pub segments_before: u64,
    /// Path segments after fitting.
    pub segments_after: u64,
    /// Largest bidirectional error accepted, over every chain.
    pub max_error_px: f64,
    /// Largest 95th-percentile source-to-curve distance, over every chain.
    pub p95_error_px: f64,
}

impl FitStats {
    /// Segments removed as a fraction of those there were, in `[0, 1]`.
    ///
    /// §31.5 requires simplification to "reduce nodes materially versus raw
    /// topology"; this is the number that claim is made with.
    #[must_use]
    pub fn reduction(&self) -> f64 {
        if self.segments_before == 0 {
            return 0.0;
        }
        let before = self.segments_before as f64;
        let after = self.segments_after as f64;
        ((before - after) / before).clamp(0.0, 1.0)
    }
}

/// Fit every chain in `topology`, in place.
///
/// # Errors
///
/// [`TraceError::Cancelled`] if the control reports cancellation. Fitting
/// itself cannot fail: a chain that cannot be fitted keeps its polyline.
pub fn fit_topology(
    topology: &mut Topology,
    options: &FitOptions,
    control: &dyn TraceControl,
) -> Result<FitStats, TraceError> {
    let mut stats = FitStats::default();
    if !options.enabled() {
        stats.segments_before = topology
            .chains
            .iter()
            .map(|c| c.segments.len() as u64)
            .sum();
        stats.segments_after = stats.segments_before;
        return Ok(stats);
    }

    for index in 0..topology.chains.len() {
        check_cancel(control, index)?;
        let before = topology.chains[index].segments.len() as u64;
        stats.segments_before += before;

        match fit_one(&topology.chains[index], options, &mut stats) {
            Ok(fitted) => {
                stats.chains_fitted += 1;
                stats.segments_after += fitted.chain.segments.len() as u64;
                stats.max_error_px = stats.max_error_px.max(fitted.max_error);
                stats.p95_error_px = stats.p95_error_px.max(fitted.p95_error);
                topology.chains[index] = fitted.chain;
            }
            Err(Skipped::NothingToFit) => {
                stats.chains_trivial += 1;
                stats.segments_after += before;
            }
            Err(Skipped::BudgetExhausted) => {
                stats.polyline_fallbacks += 1;
                stats.segments_after += before;
            }
            Err(Skipped::AlreadyMinimal) => {
                stats.chains_already_minimal += 1;
                stats.segments_after += before;
            }
        }
    }

    Ok(stats)
}

/// A fitted chain and its measured error.
#[derive(Debug, Clone)]
struct FittedChain {
    chain: CurveChain,
    max_error: f64,
    p95_error: f64,
}

/// Why a chain was left as it arrived.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Skipped {
    /// Fewer than three samples, or already fitted: no simplification exists.
    NothingToFit,
    /// The search ran out of candidate budget (PTE-GEO-005).
    BudgetExhausted,
    /// The search completed and the polyline was already the shortest faithful
    /// representation.
    AlreadyMinimal,
}

/// Fit one chain, or say why it was left alone.
fn fit_one(
    chain: &CurveChain,
    options: &FitOptions,
    stats: &mut FitStats,
) -> Result<FittedChain, Skipped> {
    let forward = canonical_orientation(chain);
    let working = if forward {
        chain.clone()
    } else {
        chain.reversed()
    };

    let samples = Samples::from_chain(&working).ok_or(Skipped::NothingToFit)?;
    if samples.len() < 3 {
        // One segment already; there is nothing to simplify and a "fit" would
        // only risk moving it.
        return Err(Skipped::NothingToFit);
    }

    let pins = detect_corners(&samples, options);
    stats.corners_pinned += pins.len() as u32;

    let fitted = fit_chain(&samples, &pins, options, stats);
    if fitted.exhausted || fitted.segments.is_empty() {
        return Err(Skipped::BudgetExhausted);
    }
    // Refuse an outcome that is not an improvement, so a chain is never made
    // more complex by being fitted.
    if fitted.segments.len() >= working.segments.len() {
        return Err(Skipped::AlreadyMinimal);
    }

    let out = CurveChain {
        start: working.start,
        segments: fitted.segments,
    };
    debug_assert!(out.is_finite(), "PTE-GEO-006: no non-finite control point");

    Ok(FittedChain {
        chain: if forward { out } else { out.reversed() },
        max_error: fitted.max_error,
        p95_error: fitted.p95_error,
    })
}

/// Whether `chain` is already in the canonical fitting direction.
///
/// The rule is a total order on the endpoints: fit from the lexicographically
/// smaller end. A chain and its reverse therefore canonicalise to the *same*
/// direction, are fitted by the same arithmetic in the same order, and differ
/// in the result only by [`CurveChain::reversed`], which copies coordinates
/// rather than recomputing them.
///
/// A closed chain has equal endpoints and no orientation to choose from; it
/// keeps the one it arrived with, which is deterministic because the extractor
/// assigns it from raster position (PTE-API-010).
#[must_use]
pub fn canonical_orientation(chain: &CurveChain) -> bool {
    let start = chain.start;
    let end = chain.end();
    match order(start, end) {
        std::cmp::Ordering::Less | std::cmp::Ordering::Equal => true,
        std::cmp::Ordering::Greater => false,
    }
}

/// Total order on points: `y` then `x`, matching raster order.
fn order(a: Point, b: Point) -> std::cmp::Ordering {
    a.y.total_cmp(&b.y).then(a.x.total_cmp(&b.x))
}

/// Every chain identifier in `topology`, for a caller that wants to inspect the
/// result. Present so a test can name a chain without reaching into the field.
#[must_use]
pub fn chain_ids(topology: &Topology) -> Vec<ChainId> {
    (0..topology.chains.len())
        .map(|i| ChainId(i as u32))
        .collect()
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::float_cmp)]
mod tests {
    use super::*;
    use palette_tracer_core::control::NoControl;
    use palette_tracer_core::ir::PathSegment;

    fn options() -> FitOptions {
        FitOptions {
            tolerance_px: 0.6,
            corner_threshold_deg: 55.0,
            corner_hysteresis_deg: 8.0,
            allow_arcs: false,
            max_span_candidates: 48,
        }
    }

    fn staircase(steps: u32) -> CurveChain {
        let mut points = Vec::new();
        for i in 0..steps {
            points.push(Point::new(f64::from(i), f64::from(i)));
            points.push(Point::new(f64::from(i) + 1.0, f64::from(i)));
        }
        points.push(Point::new(f64::from(steps), f64::from(steps)));
        CurveChain::polyline(&points).unwrap()
    }

    /// §25.4's mandatory property for this module, and the reason
    /// [`canonical_orientation`] exists. Exact equality is the assertion: an
    /// approximate one would let the two sides of a shared edge drift apart by
    /// a rounding step and reappear as a seam (PTE-SVG-009).
    #[test]
    fn fitting_a_reversed_chain_gives_the_reversed_fit_exactly() {
        let cases = [
            staircase(20),
            CurveChain::polyline(
                &(0..=120)
                    .map(|i| {
                        let a = f64::from(i) / 120.0 * std::f64::consts::PI;
                        Point::new(30.0 + 25.0 * a.cos(), 30.0 + 25.0 * a.sin())
                    })
                    .collect::<Vec<_>>(),
            )
            .unwrap(),
            CurveChain::polyline(&[
                Point::new(4.0, 0.0),
                Point::new(4.0, 9.0),
                Point::new(4.0, 18.0),
                Point::new(13.0, 18.0),
                Point::new(22.0, 18.0),
            ])
            .unwrap(),
        ];

        for chain in cases {
            let mut stats = FitStats::default();
            let forward = fit_one(&chain, &options(), &mut stats);
            let mut stats = FitStats::default();
            let backward = fit_one(&chain.reversed(), &options(), &mut stats);

            match (forward, backward) {
                (Ok(f), Ok(b)) => assert_eq!(
                    f.chain,
                    b.chain.reversed(),
                    "the two traversals of one interface disagree"
                ),
                (Err(a), Err(b)) => assert_eq!(a, b, "the two directions gave up differently"),
                (f, b) => panic!(
                    "one direction fitted and the other did not: {:?} vs {:?}",
                    f.is_ok(),
                    b.is_ok()
                ),
            }
        }
    }

    /// Canonicalisation is what makes that property hold, so it is asserted
    /// directly too: a chain and its reverse must agree on which end to start
    /// from.
    #[test]
    fn a_chain_and_its_reverse_canonicalise_to_the_same_direction() {
        let chain = staircase(6);
        assert_ne!(
            canonical_orientation(&chain),
            canonical_orientation(&chain.reversed()),
            "exactly one of the two traversals is the canonical one"
        );
    }

    /// PTE-GEO-013: the junctions do not move, so the graph the fitter was
    /// handed still describes the geometry it returns.
    #[test]
    fn fitting_never_moves_a_chain_endpoint() {
        let chain = staircase(24);
        let mut stats = FitStats::default();
        let fitted = fit_one(&chain, &options(), &mut stats).expect("a staircase fits");
        assert_eq!(fitted.chain.start, chain.start);
        assert_eq!(fitted.chain.end(), chain.end());
    }

    /// The headline claim, on the shape the whole gap was about: forty
    /// grid-aligned segments become a handful.
    ///
    /// Not *one*, at this tolerance, and the reason is arithmetic rather than
    /// search quality: a span's endpoints are source samples, and on a 45°
    /// staircase the samples off the ideal diagonal are `1/√2 ≈ 0.707` px from
    /// the chord joining the ones on it. `segmentation.rs` carries the full
    /// note, and the companion test below shows the same chain becoming one
    /// line as soon as the tolerance admits it.
    #[test]
    fn a_staircase_of_forty_segments_becomes_a_handful() {
        let chain = staircase(20);
        assert_eq!(chain.segments.len(), 40);

        let mut stats = FitStats::default();
        let fitted = fit_one(&chain, &options(), &mut stats).unwrap();
        assert!(
            fitted.chain.segments.len() <= 5,
            "got {}",
            fitted.chain.segments.len()
        );
        assert!(fitted.max_error <= 0.6);
    }

    #[test]
    fn the_same_staircase_is_one_line_at_a_tolerance_that_admits_it() {
        let chain = staircase(20);
        let mut stats = FitStats::default();
        let fitted = fit_one(
            &chain,
            &FitOptions {
                tolerance_px: 0.75,
                ..options()
            },
            &mut stats,
        )
        .unwrap();
        assert_eq!(fitted.chain.segments.len(), 1);
    }

    /// Fitting must never make a chain worse. A chain that cannot be simplified
    /// keeps exactly the geometry it had.
    #[test]
    fn a_chain_that_cannot_be_simplified_is_left_alone() {
        // Alternating spikes far larger than the tolerance: no model spans two
        // of them, so the fit can only reproduce the polyline.
        let mut points = Vec::new();
        for i in 0..20 {
            points.push(Point::new(f64::from(i) * 3.0, 0.0));
            points.push(Point::new(f64::from(i) * 3.0 + 1.5, 9.0));
        }
        let chain = CurveChain::polyline(&points).unwrap();
        let mut stats = FitStats::default();
        if let Ok(f) = fit_one(&chain, &options(), &mut stats) {
            assert!(f.chain.segments.len() < chain.segments.len());
        }
    }

    /// A zero tolerance means the pixel grid is the intended geometry (§15), so
    /// nothing is touched and the statistics say so rather than reporting a
    /// reduction that did not happen.
    #[test]
    fn a_zero_tolerance_leaves_every_chain_untouched() {
        let mut topology = Topology::default();
        topology.chains.push(staircase(10));
        let before = topology.clone();

        let stats = fit_topology(
            &mut topology,
            &FitOptions {
                tolerance_px: 0.0,
                ..options()
            },
            &NoControl,
        )
        .unwrap();

        assert_eq!(topology, before);
        assert_eq!(stats.chains_fitted, 0);
        assert_eq!(stats.segments_before, stats.segments_after);
        assert_eq!(stats.reduction(), 0.0);
    }

    #[test]
    fn fitting_a_topology_reports_what_it_did() {
        let mut topology = Topology::default();
        topology.chains.push(staircase(20));
        topology.chains.push(staircase(12));

        let stats = fit_topology(&mut topology, &options(), &NoControl).unwrap();

        assert_eq!(stats.chains_fitted, 2);
        assert_eq!(stats.polyline_fallbacks, 0);
        assert_eq!(stats.segments_before, 40 + 24);
        assert!(stats.segments_after <= 10, "got {}", stats.segments_after);
        assert!(stats.reduction() > 0.8, "got {}", stats.reduction());
        assert!(
            stats.candidates_evaluated > 0,
            "PTE-GEO-005 instrumentation"
        );
    }

    /// An already-fitted chain is not refitted (PTE-GEO-012).
    #[test]
    fn a_chain_containing_a_cubic_is_left_alone() {
        let mut topology = Topology::default();
        topology.chains.push(CurveChain {
            start: Point::ORIGIN,
            segments: vec![
                PathSegment::Cubic {
                    c1: Point::new(1.0, 2.0),
                    c2: Point::new(3.0, 2.0),
                    to: Point::new(4.0, 0.0),
                },
                PathSegment::Line {
                    to: Point::new(9.0, 0.0),
                },
            ],
        });
        let before = topology.clone();

        let stats = fit_topology(&mut topology, &options(), &NoControl).unwrap();
        assert_eq!(topology, before);
        assert_eq!(stats.chains_trivial, 1, "already fitted is not a fallback");
        assert_eq!(stats.polyline_fallbacks, 0);
        assert_eq!(stats.chains_already_minimal, 0);
    }

    /// Cancellation is honoured on a bounded stride, like every other stage
    /// (PTE-API-006).
    #[test]
    fn fitting_can_be_cancelled() {
        struct Cancelled;
        impl TraceControl for Cancelled {
            fn is_cancelled(&self) -> bool {
                true
            }
        }

        let mut topology = Topology::default();
        for _ in 0..64 {
            topology.chains.push(staircase(4));
        }
        assert!(matches!(
            fit_topology(&mut topology, &options(), &Cancelled),
            Err(TraceError::Cancelled)
        ));
    }
}
