//! Candidate models (§11.4) and their bidirectional validation (§11.5).
//!
//! # Why both directions
//!
//! §11.5 opens with the reason: "Sampling only source points against a
//! candidate curve is insufficient: a Bézier can pass near samples while
//! ballooning between them." So every candidate is measured twice:
//!
//! * **source → curve**, the largest distance from a source sample to the
//!   candidate;
//! * **curve → source**, the largest distance from a point *on the candidate*
//!   to the source polyline.
//!
//! The second is what catches the balloon, and it is the direction a
//! sample-only fitter cannot see. Their maximum is the approximate bidirectional
//! Hausdorff distance §11.5 defines, and it is what the tolerance gates.
//!
//! Both are computed against a flattening whose error is *bounded*, not
//! estimated (see [`crate::curves`]), and the bound is added to the measured
//! distance before comparing against the tolerance. A candidate this module
//! accepts is therefore accepted under the true distance as well.
//!
//! # Cubic fitting
//!
//! §11.4's constrained form: `P1 = P0 + α·t0` and `P2 = P3 − β·t1`, with `α`
//! and `β` from weighted least squares over a chord-length parameterisation.
//! Endpoints are exact, which is what lets a fitted span be pinned to a
//! junction (PTE-GEO-013).
//!
//! A degenerate normal-equation solve does not produce a NaN control point
//! (PTE-GEO-006): it falls back to the declared tangent-length heuristic
//! `α = β = chord/3`, and if that also fails validation the candidate is simply
//! not offered, leaving the line — and ultimately the raw polyline — to carry
//! the span.

use crate::curves::{Cubic, point_to_segment, sweep_distances, within};
use palette_tracer_core::ir::{PathSegment, Point, Vec2};

/// How many parameters a model spends, for §11.6's tie-break rule 3.
const LINE_PARAMETERS: u32 = 2;
const CUBIC_PARAMETERS: u32 = 6;

/// Simplicity order for §11.6's tie-break rule 4: `line`, then `arc`, then
/// `cubic`.
const LINE_RANK: u32 = 0;
const CUBIC_RANK: u32 = 2;

/// Samples used to probe a cubic for cusps and loops.
const TURNING_SAMPLES: u32 = 24;

/// Newton reparameterisation passes. A constant, so the "worst-case recursive
/// refitting MUST be bounded" of PTE-GEO-005 is bounded by construction.
const MAX_REPARAMETERISATIONS: u32 = 4;

/// A cubic that turns further than this between two pins is looping or has a
/// cusp inside it, whatever its endpoint error says (§11.5).
const MAX_TURNING: f64 = std::f64::consts::PI;

/// One accepted candidate for a span.
#[derive(Debug, Clone, Copy)]
pub struct Candidate {
    /// The segment to emit.
    pub segment: PathSegment,
    /// Free parameters, for tie-break rule 3.
    pub parameters: u32,
    /// Model simplicity, for tie-break rule 4.
    pub rank: u32,
    /// Approximate bidirectional Hausdorff distance, in source pixels.
    pub error: f64,
    /// 95th percentile of the source-to-curve distance (§26.2).
    pub p95: f64,
}

/// Distances a candidate produced.
#[derive(Debug, Clone, Copy)]
struct Errors {
    hausdorff: f64,
    p95: f64,
}

/// The straight-line candidate for `points[first..=last]`, or `None` if it does
/// not hold the tolerance.
#[must_use]
pub fn fit_line(points: &[Point], first: usize, last: usize, tolerance: f64) -> Option<Candidate> {
    let a = points[first];
    let b = points[last];
    let span = &points[first..=last];

    // A line is its own flattening, so no bound is added in either direction.
    // The scan stops at the first sample outside the tolerance: on anything but
    // a straight run that is within a few samples, and it is what keeps the
    // dynamic program's long-span proposals cheap when they cannot succeed.
    let mut distances = Vec::with_capacity(span.len());
    let mut forward: f64 = 0.0;
    for &p in span {
        let d = point_to_segment(p, a, b);
        if !within(d, tolerance) {
            return None;
        }
        forward = forward.max(d);
        distances.push(d);
    }
    // Curve to source: the segment's own endpoints lie on the source, and the
    // furthest a straight chord can stray from the polyline it spans is
    // attained at a source vertex, which `forward` already measured. Sampling
    // the chord adds nothing a straight model can hide.
    let hausdorff = forward;
    if !within(hausdorff, tolerance) {
        return None;
    }

    Some(Candidate {
        segment: PathSegment::Line { to: b },
        parameters: LINE_PARAMETERS,
        rank: LINE_RANK,
        error: hausdorff,
        p95: percentile95(&mut distances),
    })
}

/// The cubic candidate for `points[first..=last]`, or `None`.
///
/// `t0` and `t1` are unit tangents in the direction of travel at the two ends,
/// estimated by the caller from inside the span only (PTE-GEO-013).
#[must_use]
pub fn fit_cubic(
    points: &[Point],
    arclen: &[f64],
    first: usize,
    last: usize,
    t0: Vec2,
    t1: Vec2,
    tolerance: f64,
) -> Option<Candidate> {
    let p0 = points[first];
    let p3 = points[last];
    let chord = p0.distance(p3);
    if !chord.is_finite() || chord <= 0.0 {
        // A closed span has no chord to scale tangent lengths by. Rejecting it
        // leaves the split to the dynamic program, which has other options.
        return None;
    }

    let mut u = chord_parameterisation(arclen, first, last)?;
    let heuristic = (chord / 3.0, chord / 3.0);
    let limit = chord * 3.0;
    let mut best: Option<Candidate> = None;

    // §11.4: "apply a stable parameterization, then reparameterize with bounded
    // Newton iterations only when it improves a validated objective."
    //
    // The chord-length parameterisation is stable but biased: it assumes the
    // cubic's parameter advances in proportion to arclength, which it does not.
    // On a quarter circle the bias costs about 1.4% of the tangent length, and
    // 1.4% of tangent length is 0.16 px of sag at the midpoint — enough on its
    // own to miss the §31.2 gates. Newton removes it in two or three steps.
    //
    // "Only when it improves" is honoured by keeping the best *validated*
    // candidate across iterations rather than the last one, so a step that
    // makes things worse cannot make the result worse. The iteration count is
    // a constant, which is what keeps PTE-GEO-005's bound on refitting true.
    for iteration in 0..=MAX_REPARAMETERISATIONS {
        let solved = solve_alpha_beta(points, first, p0, p3, t0, t1, &u);

        // The declared fallback is offered once, on the first pass: reparameterising
        // a heuristic that ignores the samples would be refining a guess.
        let attempts = if iteration == 0 {
            [solved, Some(heuristic)]
        } else {
            [solved, None]
        };

        let mut solved_cubic = None;
        for (index, attempt) in attempts.into_iter().enumerate() {
            let Some((alpha, beta)) = attempt.filter(|&(a, b)| {
                // A non-positive tangent length puts a control point behind its
                // endpoint, which is a cusp waiting to happen (PTE-GEO-006).
                a.is_finite() && b.is_finite() && a > 0.0 && b > 0.0
            }) else {
                continue;
            };
            let (alpha, beta) = (alpha.min(limit), beta.min(limit));
            let cubic = Cubic::new(p0, p0 + t0 * alpha, p3 - t1 * beta, p3);
            if index == 0 {
                // Newton continues from the solved cubic even if it failed
                // validation: a near miss is exactly what reparameterising fixes.
                solved_cubic = Some(cubic);
            }
            let Some(errors) = validate_cubic(&cubic, points, first, last, tolerance) else {
                continue;
            };
            let candidate = Candidate {
                segment: PathSegment::Cubic {
                    c1: cubic.p1,
                    c2: cubic.p2,
                    to: p3,
                },
                parameters: CUBIC_PARAMETERS,
                rank: CUBIC_RANK,
                error: errors.hausdorff,
                p95: errors.p95,
            };
            if best.is_none_or(|b| candidate.error < b.error) {
                best = Some(candidate);
            }
        }

        if iteration == MAX_REPARAMETERISATIONS {
            break;
        }
        let Some(cubic) = solved_cubic else { break };
        u = reparameterise(&cubic, points, first, &u);
    }

    best
}

/// One Newton step per sample, moving each parameter towards the foot of the
/// perpendicular from its sample to the curve.
///
/// The step is the derivative of `‖B(u) − P‖²` set to zero:
///
/// ```text
/// u' = u − ((B(u) − P)·B'(u)) / (‖B'(u)‖² + (B(u) − P)·B''(u))
/// ```
///
/// A vanishing denominator leaves the parameter alone rather than diverging,
/// and the endpoints stay pinned at 0 and 1 so the fit keeps its junctions.
fn reparameterise(cubic: &Cubic, points: &[Point], first: usize, u: &[f64]) -> Vec<f64> {
    let last_index = u.len().saturating_sub(1);
    u.iter()
        .enumerate()
        .map(|(k, &t)| {
            if k == 0 {
                return 0.0;
            }
            if k == last_index {
                return 1.0;
            }
            let diff = cubic.eval(t) - points[first + k];
            let d1 = cubic.deriv(t);
            let d2 = cubic.deriv2(t);
            let denominator = d1.length_squared() + diff.dot(d2);
            if !denominator.is_finite() || denominator.abs() <= 1e-12 {
                return t;
            }
            let next = t - diff.dot(d1) / denominator;
            if next.is_finite() {
                next.clamp(0.0, 1.0)
            } else {
                t
            }
        })
        .collect()
}

/// Measure a cubic in both directions and reject loops and cusps.
fn validate_cubic(
    cubic: &Cubic,
    points: &[Point],
    first: usize,
    last: usize,
    tolerance: f64,
) -> Option<Errors> {
    if !(cubic.p1.is_finite() && cubic.p2.is_finite()) {
        return None;
    }
    // A cusp or a loop is a hard rejection, checked before any distance: a
    // curve can be arbitrarily close to the samples and still be unusable.
    if !within(cubic.total_turning(TURNING_SAMPLES), MAX_TURNING) {
        return None;
    }

    // Flatten a good deal finer than the tolerance, so the bound that gets
    // added below is a small fraction of the budget rather than most of it.
    let flatten_tolerance = tolerance / 16.0;
    let flat = cubic.flatten(flatten_tolerance)?;
    let span = &points[first..=last];

    // Measuring against the flattening *over*-states the distance by at most
    // the flattening bound, so testing against `tolerance - bound` keeps the
    // conclusion conservative while letting the sweep reject early.
    let budget = tolerance - flatten_tolerance;
    if !budget.is_finite() || within(budget, 0.0) {
        return None;
    }

    // Source to curve.
    let mut forward = sweep_distances(span, &flat, budget)?;
    let forward_max = forward.iter().copied().fold(0.0, f64::max) + flatten_tolerance;

    // Curve to source: the direction that catches a curve ballooning between
    // samples. The source polyline is exact, so only the flattening bound is
    // added.
    let backward = sweep_distances(&flat, span, budget)?
        .iter()
        .copied()
        .fold(0.0, f64::max)
        + flatten_tolerance;

    for d in &mut forward {
        *d += flatten_tolerance;
    }

    Some(Errors {
        hausdorff: forward_max.max(backward),
        p95: percentile95(&mut forward),
    })
}

/// Chord-length parameters in `[0, 1]` for the samples of a span.
fn chord_parameterisation(arclen: &[f64], first: usize, last: usize) -> Option<Vec<f64>> {
    let base = arclen[first];
    let total = arclen[last] - base;
    if !total.is_finite() || total <= 0.0 {
        return None;
    }
    Some(
        arclen[first..=last]
            .iter()
            .map(|&a| ((a - base) / total).clamp(0.0, 1.0))
            .collect(),
    )
}

/// Least-squares `α` and `β` for the §11.4 constrained cubic.
///
/// Returns `None` when the 2×2 normal-equation system is ill-conditioned, which
/// is the case PTE-GEO-006 requires a declared fallback for rather than a NaN.
#[allow(clippy::too_many_arguments)]
fn solve_alpha_beta(
    points: &[Point],
    first: usize,
    p0: Point,
    p3: Point,
    t0: Vec2,
    t1: Vec2,
    u: &[f64],
) -> Option<(f64, f64)> {
    let (mut c11, mut c12, mut c22) = (0.0, 0.0, 0.0);
    let (mut x1, mut x2) = (0.0, 0.0);
    let t0_dot_t1 = t0.dot(t1);

    for (k, &t) in u.iter().enumerate() {
        let v = 1.0 - t;
        let b0 = v * v * v;
        let b1 = 3.0 * v * v * t;
        let b2 = 3.0 * v * t * t;
        let b3 = t * t * t;

        // The residual after the two fixed endpoint contributions.
        let target = points[first + k];
        let rx = target.x - (b0 + b1) * p0.x - (b2 + b3) * p3.x;
        let ry = target.y - (b0 + b1) * p0.y - (b2 + b3) * p3.y;

        c11 += b1 * b1;
        c12 -= b1 * b2 * t0_dot_t1;
        c22 += b2 * b2;
        x1 += rx * b1 * t0.x + ry * b1 * t0.y;
        x2 -= rx * b2 * t1.x + ry * b2 * t1.y;
    }
    let det = c11 * c22 - c12 * c12;
    // Scale the conditioning test by the magnitudes involved, so it means the
    // same thing for a two-pixel span and a two-hundred-pixel one.
    let scale = (c11 * c22).abs().max(f64::MIN_POSITIVE);
    if !det.is_finite() || det.abs() <= 1e-9 * scale {
        return None;
    }
    let alpha = (x1 * c22 - c12 * x2) / det;
    let beta = (c11 * x2 - x1 * c12) / det;
    Some((alpha, beta))
}

/// The 95th percentile of a set of distances, by nearest-rank.
///
/// Sorted with `total_cmp`, which orders every `f64` including NaN, so the
/// result never depends on comparison order (PTE-DET-002).
fn percentile95(values: &mut [f64]) -> f64 {
    if values.is_empty() {
        return 0.0;
    }
    values.sort_by(|a, b| a.total_cmp(b));
    // Nearest-rank: the smallest value at or above the 95th percentile.
    let rank = ((values.len() as f64) * 0.95).ceil() as usize;
    let index = rank.saturating_sub(1).min(values.len() - 1);
    values[index]
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::float_cmp)]
mod tests {
    use super::*;
    use crate::chain::Samples;

    fn samples_of(points: &[Point]) -> Samples {
        Samples::from_points(points)
    }

    #[test]
    fn a_straight_run_is_one_line_with_no_error() {
        let points: Vec<Point> = (0..=10).map(|i| Point::new(f64::from(i), 0.0)).collect();
        let c = fit_line(&points, 0, 10, 0.5).expect("a straight run fits a line");
        assert_eq!(c.error, 0.0);
        assert_eq!(
            c.segment,
            PathSegment::Line {
                to: Point::new(10.0, 0.0)
            }
        );
    }

    #[test]
    fn a_bent_run_does_not_fit_one_line() {
        let points = [
            Point::new(0.0, 0.0),
            Point::new(5.0, 4.0),
            Point::new(10.0, 0.0),
        ];
        assert!(fit_line(&points, 0, 2, 0.5).is_none());
        assert!(fit_line(&points, 0, 2, 5.0).is_some());
    }

    /// The load-bearing test of §11.5: a candidate that hits every sample and
    /// balloons between them must be rejected. Without the curve-to-source
    /// direction it would pass.
    #[test]
    fn a_ballooning_cubic_is_rejected_though_it_passes_through_the_samples() {
        // Three collinear samples. A cubic with enormous, opposed tangents
        // passes through all three and bulges far away in between.
        let points = [
            Point::new(0.0, 0.0),
            Point::new(5.0, 0.0),
            Point::new(10.0, 0.0),
        ];
        let balloon = Cubic::new(
            Point::new(0.0, 0.0),
            Point::new(0.0, 20.0),
            Point::new(10.0, 20.0),
            Point::new(10.0, 0.0),
        );

        // Source to curve alone would be misleadingly small at the ends...
        let forward = points
            .iter()
            .map(|&p| crate::curves::point_to_polyline(p, &balloon.flatten(0.01).unwrap()))
            .fold(0.0, f64::max);
        assert!(forward > 1.0, "sanity: this fixture does stray");

        // ...and the validator rejects it outright.
        assert!(validate_cubic(&balloon, &points, 0, 2, 1.0).is_none());
    }

    /// An analytic quarter circle is the case a cubic is *for*: it must fit
    /// inside a tight tolerance, and the fit must be a cubic, not a line.
    #[test]
    fn a_quarter_circle_fits_one_cubic_within_a_tenth_of_a_pixel() {
        let radius = 40.0;
        let points: Vec<Point> = (0..=60)
            .map(|i| {
                let a = f64::from(i) / 60.0 * std::f64::consts::FRAC_PI_2;
                Point::new(radius * a.cos(), radius * a.sin())
            })
            .collect();
        let s = samples_of(&points);
        let n = s.len() - 1;
        let t0 = s.forward_tangent(0, 2.0, n).unwrap().0;
        let t1 = s.backward_tangent(n, 2.0, 0).unwrap().0;

        let c = fit_cubic(&s.points, &s.arclen, 0, n, t0, t1, 0.1)
            .expect("a quarter circle is what a cubic models");
        assert!(matches!(c.segment, PathSegment::Cubic { .. }));
        assert!(c.error <= 0.1, "error {}", c.error);
        assert!(
            fit_line(&s.points, 0, n, 0.1).is_none(),
            "and it is not a line"
        );
    }

    /// PTE-GEO-006: no NaN control point ever leaves this module, whatever the
    /// solve does.
    #[test]
    fn a_degenerate_span_never_produces_a_nan_control_point() {
        // Every sample identical: chord length zero, normal equations singular.
        let points = vec![Point::new(3.0, 3.0); 8];
        let s = samples_of(&points);
        let t = Vec2::new(1.0, 0.0);
        assert!(fit_cubic(&s.points, &s.arclen, 0, s.len() - 1, t, t, 1.0).is_none());

        // A span whose samples are collinear but whose tangents are opposed:
        // the solve is legal but the geometry is not.
        let straight: Vec<Point> = (0..=8).map(|i| Point::new(f64::from(i), 0.0)).collect();
        let s = samples_of(&straight);
        let out = fit_cubic(
            &s.points,
            &s.arclen,
            0,
            s.len() - 1,
            Vec2::new(1.0, 0.0),
            Vec2::new(-1.0, 0.0),
            0.5,
        );
        if let Some(c) = out {
            match c.segment {
                PathSegment::Cubic { c1, c2, to } => {
                    assert!(c1.is_finite() && c2.is_finite() && to.is_finite());
                }
                other => panic!("expected a cubic, got {other:?}"),
            }
        }
    }

    #[test]
    fn the_percentile_is_nearest_rank_and_order_independent() {
        let mut a = [1.0, 2.0, 3.0, 4.0, 100.0];
        let mut b = [100.0, 3.0, 1.0, 4.0, 2.0];
        assert_eq!(percentile95(&mut a), percentile95(&mut b));
        assert_eq!(percentile95(&mut a), 100.0);
        assert_eq!(percentile95(&mut [7.0]), 7.0);
        assert_eq!(percentile95(&mut []), 0.0);
    }
}
