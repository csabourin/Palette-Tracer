//! Global segmentation of a chain by dynamic programming (§11.6).
//!
//! # The recurrence
//!
//! §11.6's `D[j] = min_{i<j, M} (D[i] + cost(M, i, j))`, run independently over
//! each block between consecutive pins so that pins are honoured by
//! construction rather than by a penalty (PTE-GEO-003, PTE-GEO-013).
//!
//! Candidates that fail the §11.5 hard tests never enter the graph, exactly as
//! §11.6 requires, so any path the recurrence finds is admissible and the
//! minimisation is over quality alone.
//!
//! # Cost, and why it is a tuple
//!
//! §11.1 gives a soft objective with weights `λ_n`, `λ_p`, `λ_r`; §11.6 gives a
//! strict tie-break order. Choosing weights would make the tie-break order an
//! emergent property of three arbitrary constants, and PTE-DET-003 forbids a
//! bare float from deciding anything. So the cost *is* the tie-break order,
//! expressed as an additive lexicographic tuple:
//!
//! 1. segment count (`λ_n`, rule 2);
//! 2. free parameters (`λ_p`, rule 3);
//! 3. model simplicity — line before arc before cubic (rule 4);
//! 4. quantised total error (`E_data`, at [`GEOMETRY_SCALE`]);
//! 5. earliest split, realised by preferring the smaller predecessor on a tie
//!    (rule 5).
//!
//! Every component is additive, so the recurrence keeps optimal substructure.
//! Rule 1, "lower hard-error class", is not a component because the graph
//! contains no failing candidate to be worse than a passing one.
//!
//! Fewer segments therefore always beats a smaller error, which is what §31.5
//! asks for: a straight shared boundary is one line, not four that fit slightly
//! better.
//!
//! # Bounded work (PTE-GEO-005, PTE-GEO-009)
//!
//! Predecessors are proposed on a geometric ladder — 1, 2, 3, 4, 6, 8, 12, 16,
//! … samples back — plus the block's first sample, always. The ladder makes
//! candidate generation `O(n log n)` instead of `O(n²)`; including the block
//! start unconditionally is what keeps "a straight boundary is *one* line"
//! representable no matter how long the run is. `max_span_candidates` caps the
//! ladder, and a whole-chain candidate budget caps the rest: exceeding it
//! abandons the chain and keeps the polyline, which is the declared fallback
//! rather than an unbounded search.

use crate::chain::Samples;
use crate::models::{Candidate, fit_cubic, fit_line};
use crate::{FitOptions, FitStats};
use palette_tracer_core::determinism::QuantKey;
use palette_tracer_core::ir::{PathSegment, Point};

/// Tangent estimation window, in source pixels.
///
/// Long enough that a stair step does not dominate the direction, short enough
/// that a tangent still belongs to its end of the span.
const TANGENT_WINDOW_PX: f64 = 2.5;

/// Slack above the search's own worst case, and a floor so a short chain is
/// never starved.
///
/// The budget is derived from the search rather than picked: each position
/// offers at most `ladder + 1` predecessors and each span costs at most two
/// candidate fits (a line, then a cubic if the line failed), so
/// `2·(ladder + 1)·n` is exactly the intended worst case. The budget's job is
/// to stop a *super*-linear search, not to ration the linear one, so anything
/// beyond that is pathological and the polyline fallback is the right answer.
const CANDIDATE_BUDGET_FLOOR: u64 = 256;

/// The largest predecessor ladder the search will use, whatever the
/// configuration asks for. Caps the constant in the `O(n·K)` bound.
const MAX_LADDER: usize = 64;

/// The lexicographic cost of a partial solution.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
struct Cost {
    segments: u32,
    parameters: u32,
    rank: u32,
    error: i64,
}

impl Cost {
    const ZERO: Self = Self {
        segments: 0,
        parameters: 0,
        rank: 0,
        error: 0,
    };

    fn plus(self, candidate: &Candidate) -> Self {
        Self {
            segments: self.segments.saturating_add(1),
            parameters: self.parameters.saturating_add(candidate.parameters),
            rank: self.rank.saturating_add(candidate.rank),
            // Quantised at the geometry scale, so two errors that differ below
            // a millionth of a pixel are tied and rule 5 decides (PTE-DET-003).
            error: self
                .error
                .saturating_add(QuantKey::cost_or_worst(candidate.error).raw().max(0)),
        }
    }
}

/// What one chain's fit produced.
#[derive(Debug, Clone)]
pub struct Fitted {
    /// Segments from the chain's start to its end.
    pub segments: Vec<PathSegment>,
    /// Largest bidirectional error over the accepted spans.
    pub max_error: f64,
    /// Largest 95th-percentile source-to-curve distance over the spans.
    pub p95_error: f64,
    /// Whether the fitter gave up and the caller should keep the polyline.
    pub exhausted: bool,
}

/// Fit one prepared chain, given the sample indices that must be pins.
///
/// `pins` must be sorted, must not contain the endpoints, and must lie strictly
/// inside the chain.
#[must_use]
pub fn fit_chain(
    samples: &Samples,
    pins: &[usize],
    options: &FitOptions,
    stats: &mut FitStats,
) -> Fitted {
    let n = samples.len();
    let mut out = Fitted {
        segments: Vec::new(),
        max_error: 0.0,
        p95_error: 0.0,
        exhausted: false,
    };
    if n < 2 {
        return out;
    }

    let ladder = ladder_size(options.max_span_candidates) as u64;
    let mut budget = (n as u64)
        .saturating_mul(ladder.saturating_add(1))
        .saturating_mul(2)
        .max(CANDIDATE_BUDGET_FLOOR);

    // Blocks run pin to pin, with the two endpoints always pins.
    let mut boundaries = Vec::with_capacity(pins.len() + 2);
    boundaries.push(0);
    boundaries.extend(pins.iter().copied().filter(|&p| p > 0 && p < n - 1));
    boundaries.push(n - 1);

    for window in boundaries.windows(2) {
        let (start, end) = (window[0], window[1]);
        if end <= start {
            continue;
        }
        match fit_block(samples, start, end, options, &mut budget, stats) {
            Some(block) => {
                out.max_error = out.max_error.max(block.max_error);
                out.p95_error = out.p95_error.max(block.p95_error);
                out.segments.extend(block.segments);
            }
            None => {
                // The declared fallback: this chain keeps its polyline. Partial
                // results are discarded rather than mixed, so a chain is either
                // fitted or not (§0.2 — no half-claim).
                out.exhausted = true;
                return out;
            }
        }
    }

    out
}

/// One pin-to-pin block.
fn fit_block(
    samples: &Samples,
    start: usize,
    end: usize,
    options: &FitOptions,
    budget: &mut u64,
    stats: &mut FitStats,
) -> Option<Fitted> {
    let count = end - start + 1;
    // `best[k]` is the optimum for `samples[start + k]`.
    let mut best: Vec<Option<(Cost, usize, Candidate)>> = vec![None; count];
    let mut reached: Vec<Option<Cost>> = vec![None; count];
    reached[0] = Some(Cost::ZERO);

    for k in 1..count {
        let j = start + k;
        for i in predecessors(start, j, options.max_span_candidates) {
            let Some(previous) = reached[i - start] else {
                continue;
            };
            for candidate in candidates(samples, i, j, options, budget, stats) {
                let cost = previous.plus(&candidate);
                let better = match reached[k] {
                    None => true,
                    // Strictly better only, and predecessors are visited in
                    // increasing `i`, so a tie keeps the earliest split
                    // (§11.6 rule 5).
                    Some(existing) => cost < existing,
                };
                if better {
                    reached[k] = Some(cost);
                    best[k] = Some((cost, i, candidate));
                }
            }
            if *budget == 0 {
                return None;
            }
        }
        // Every position is reachable from its immediate predecessor by a
        // zero-error line, so an unreachable one means the budget ran out
        // mid-block and the polyline fallback is the answer.
        reached[k]?;
    }

    // Walk the parents back, then reverse: the path is built end to start.
    let mut segments = Vec::new();
    let mut max_error: f64 = 0.0;
    let mut p95_error: f64 = 0.0;
    let mut k = count - 1;
    while k > 0 {
        let (_, i, candidate) = best[k]?;
        segments.push(candidate.segment);
        max_error = max_error.max(candidate.error);
        p95_error = p95_error.max(candidate.p95);
        k = i - start;
    }
    segments.reverse();

    Some(Fitted {
        segments,
        max_error,
        p95_error,
        exhausted: false,
    })
}

/// How many rungs the ladder has, given the configured cap.
fn ladder_size(cap: u32) -> usize {
    (cap.max(2) as usize).min(MAX_LADDER)
}

/// Predecessor proposals for `j`, in increasing order, always including
/// `start`.
fn predecessors(start: usize, j: usize, cap: u32) -> Vec<usize> {
    let mut out = Vec::new();
    let mut step = 1usize;
    let limit = ladder_size(cap);
    // The geometric ladder, gathered descending then reversed so the result is
    // sorted and rule 5's "earliest split" reading is the natural one.
    while out.len() < limit {
        let Some(i) = j.checked_sub(step) else { break };
        if i < start {
            break;
        }
        out.push(i);
        if i == start {
            break;
        }
        // 1, 2, 3, 4, 6, 8, 12, 16, 24, 32, ... — a factor of about 1.5 with
        // integer steps, so short spans keep full resolution.
        step = if step < 4 { step + 1 } else { step + step / 2 };
    }
    if !out.contains(&start) {
        out.push(start);
    }
    out.sort_unstable();
    out
}

/// Admissible candidates for the span `i..=j`.
fn candidates(
    samples: &Samples,
    i: usize,
    j: usize,
    options: &FitOptions,
    budget: &mut u64,
    stats: &mut FitStats,
) -> Vec<Candidate> {
    let mut out = Vec::new();
    if *budget == 0 {
        return out;
    }
    *budget -= 1;
    stats.candidates_evaluated += 1;

    if let Some(line) = fit_line(&samples.points, i, j, options.tolerance_px) {
        out.push(line);
        // A line that holds the tolerance is already the simplest model
        // available and would beat any cubic on every tie-break, so fitting one
        // would be work whose result cannot win.
        return out;
    }

    // Two samples with no line between them cannot do better with a curve.
    if j - i < 2 {
        return out;
    }

    if *budget == 0 {
        return out;
    }
    *budget -= 1;
    stats.candidates_evaluated += 1;

    let (Some(t0), Some(t1)) = (
        samples.forward_tangent(i, TANGENT_WINDOW_PX, j),
        samples.backward_tangent(j, TANGENT_WINDOW_PX, i),
    ) else {
        return out;
    };

    if let Some(cubic) = fit_cubic(
        &samples.points,
        &samples.arclen,
        i,
        j,
        t0.0,
        t1.0,
        options.tolerance_px,
    ) {
        out.push(cubic);
    }
    out
}

/// Whether a fitted chain still ends where the polyline did.
///
/// PTE-GEO-013's pinning, checked rather than assumed: every model is built
/// with the span's endpoints as its own, so this is an invariant, and a failure
/// would mean a junction had moved.
#[must_use]
pub fn ends_at(segments: &[PathSegment], expected: Point) -> bool {
    segments.last().is_some_and(|s| s.end() == expected)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::float_cmp)]
mod tests {
    use super::*;

    fn options() -> FitOptions {
        FitOptions {
            tolerance_px: 0.6,
            corner_threshold_deg: 55.0,
            corner_hysteresis_deg: 8.0,
            allow_arcs: false,
            max_span_candidates: 48,
        }
    }

    /// §31.5: "straight shared boundaries SHOULD be one line". A 200-sample run
    /// must collapse to one segment, which is what including the block start in
    /// every predecessor ladder is for.
    #[test]
    fn a_long_straight_run_becomes_exactly_one_line() {
        let points: Vec<Point> = (0..=200).map(|i| Point::new(f64::from(i), 5.0)).collect();
        let samples = Samples::from_points(&points);
        let mut stats = FitStats::default();
        let fitted = fit_chain(&samples, &[], &options(), &mut stats);

        assert!(!fitted.exhausted);
        assert_eq!(fitted.segments.len(), 1, "got {:?}", fitted.segments);
        assert!(
            fitted.max_error < 1e-9,
            "a collinear run has no error beyond rounding, got {}",
            fitted.max_error
        );
    }

    /// A staircase is a straight line the raster could not draw straight, and
    /// it is the shape most of a traced boundary is made of.
    ///
    /// **Why not one segment at 0.6 px.** The split points of a fitted chain
    /// are source samples, so a span's model runs corner to corner of the pixel
    /// grid. On a 45° staircase the samples alternate on and off the ideal
    /// diagonal, and the ones off it sit `1/√2 ≈ 0.707` px from the chord
    /// joining any two on it. No model pinned to those endpoints can do better,
    /// so a tolerance below 0.707 px necessarily splits — this is arithmetic,
    /// not a weakness of the search. The reduction is still 40 segments to a
    /// handful.
    #[test]
    fn a_diagonal_staircase_collapses_to_a_handful_of_segments() {
        let mut points = Vec::new();
        for i in 0..40 {
            points.push(Point::new(f64::from(i), f64::from(i)));
            points.push(Point::new(f64::from(i) + 1.0, f64::from(i)));
        }
        let samples = Samples::from_points(&points);
        let mut stats = FitStats::default();
        let fitted = fit_chain(&samples, &[], &options(), &mut stats);

        assert!(
            fitted.segments.len() * 8 <= points.len(),
            "expected at least an eightfold reduction from {} samples, got {}",
            points.len(),
            fitted.segments.len()
        );
        assert!(fitted.max_error <= 0.6);
    }

    /// Given a tolerance that admits it — anything above `1/√2` — the same
    /// staircase becomes the single line it always was.
    #[test]
    fn a_diagonal_staircase_becomes_one_line_once_the_tolerance_admits_it() {
        let mut points = Vec::new();
        for i in 0..40 {
            points.push(Point::new(f64::from(i), f64::from(i)));
            points.push(Point::new(f64::from(i) + 1.0, f64::from(i)));
        }
        let samples = Samples::from_points(&points);
        let mut stats = FitStats::default();
        let fitted = fit_chain(
            &samples,
            &[],
            &FitOptions {
                tolerance_px: 0.75,
                ..options()
            },
            &mut stats,
        );

        assert_eq!(fitted.segments.len(), 1, "got {:?}", fitted.segments);
        assert!(matches!(fitted.segments[0], PathSegment::Line { .. }));
    }

    /// The complexity claim §31.5 makes about a rectangle, on the geometry that
    /// actually reaches the fitter: pinned corners, straight sides.
    #[test]
    fn a_rectangle_becomes_four_lines() {
        let mut points = Vec::new();
        for i in 0..=30 {
            points.push(Point::new(f64::from(i), 0.0));
        }
        for i in 1..=20 {
            points.push(Point::new(30.0, f64::from(i)));
        }
        for i in 1..=30 {
            points.push(Point::new(30.0 - f64::from(i), 20.0));
        }
        for i in 1..=20 {
            points.push(Point::new(0.0, 20.0 - f64::from(i)));
        }
        let samples = Samples::from_points(&points);
        let pins = crate::chain::detect_corners(&samples, &options());
        let mut stats = FitStats::default();
        let fitted = fit_chain(&samples, &pins, &options(), &mut stats);

        assert_eq!(
            fitted.segments.len(),
            4,
            "four sides, got {:?}",
            fitted.segments
        );
        assert!(
            fitted
                .segments
                .iter()
                .all(|s| matches!(s, PathSegment::Line { .. })),
            "and every side is a line"
        );
    }

    /// Every span ends where the samples do, so a fitted chain reaches its
    /// junction exactly (PTE-GEO-013).
    #[test]
    fn a_fitted_chain_still_ends_at_its_junction() {
        let points: Vec<Point> = (0..=90)
            .map(|i| {
                let a = f64::from(i) / 90.0 * std::f64::consts::PI;
                Point::new(20.0 * a.cos(), 20.0 * a.sin())
            })
            .collect();
        let samples = Samples::from_points(&points);
        let mut stats = FitStats::default();
        let fitted = fit_chain(&samples, &[], &options(), &mut stats);
        assert!(ends_at(&fitted.segments, *samples.points.last().unwrap()));
    }

    /// PTE-GEO-009: the ladder is bounded and always offers the block start.
    #[test]
    fn the_predecessor_ladder_is_bounded_and_reaches_the_block_start() {
        let p = predecessors(0, 500, 48);
        assert!(p.len() <= 48 + 1, "got {}", p.len());
        assert_eq!(p[0], 0, "the block start is always a candidate");
        assert_eq!(*p.last().unwrap(), 499, "and so is the nearest sample");
        assert!(p.windows(2).all(|w| w[0] < w[1]), "sorted and unique");

        // A short block degrades to every predecessor, with no duplicates.
        let short = predecessors(10, 13, 48);
        assert_eq!(short, vec![10, 11, 12]);
    }

    /// Increasing the tolerance must not increase the segment count (§31.5's
    /// monotonicity rule).
    #[test]
    fn a_looser_tolerance_never_needs_more_segments() {
        let points: Vec<Point> = (0..=120)
            .map(|i| {
                let x = f64::from(i) / 4.0;
                Point::new(x, (x / 3.0).sin() * 6.0)
            })
            .collect();
        let samples = Samples::from_points(&points);

        let mut previous = usize::MAX;
        for tolerance in [0.1, 0.25, 0.5, 1.0, 2.0] {
            let mut stats = FitStats::default();
            let fitted = fit_chain(
                &samples,
                &[],
                &FitOptions {
                    tolerance_px: tolerance,
                    ..options()
                },
                &mut stats,
            );
            assert!(
                fitted.segments.len() <= previous,
                "tolerance {tolerance} used {} segments after {previous}",
                fitted.segments.len()
            );
            assert!(fitted.max_error <= tolerance);
            previous = fitted.segments.len();
        }
    }

    /// The budget is a real limit, and running out keeps the polyline rather
    /// than emitting a partial fit (PTE-GEO-005).
    #[test]
    fn exhausting_the_candidate_budget_falls_back_to_the_polyline() {
        let points: Vec<Point> = (0..=400)
            .map(|i| {
                let a = f64::from(i) / 400.0 * std::f64::consts::TAU;
                Point::new(100.0 * a.cos(), 100.0 * a.sin())
            })
            .collect();
        let samples = Samples::from_points(&points);
        let mut stats = FitStats::default();
        // A block starved to almost nothing.
        let mut budget = 3u64;
        let block = fit_block(
            &samples,
            0,
            samples.len() - 1,
            &options(),
            &mut budget,
            &mut stats,
        );
        assert!(
            block.is_none(),
            "a starved block declines rather than guesses"
        );
    }
}
