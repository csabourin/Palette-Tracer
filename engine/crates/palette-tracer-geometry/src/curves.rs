//! Cubic Bézier evaluation, flattening with a *proven* bound, and the distance
//! primitives every §11.5 metric is built from.
//!
//! # Why this is first-party
//!
//! `kurbo` was declared in the workspace manifest for exactly this job. It is
//! not used, and the reason is PTE-DET-004: nearest-point queries on a cubic
//! are a quintic root solve, and a library's iteration count and convergence
//! test are not part of its API contract. A distance that is "accurate to
//! `1e-9`" is not the same thing as a distance that produces the same decision
//! on every target, and §20.1's semantic digest is a decision.
//!
//! Everything here is closed-form or a fixed, bounded loop, so the number of
//! floating-point operations for a given input is identical on every target.
//!
//! # The flattening bound (PTE-GEO-007)
//!
//! PTE-GEO-007 requires "a proven geometric flatness/error bound or a
//! conservative cap plus rejection". This module has both.
//!
//! The Bernstein basis has *linear precision*: `Σ B_i(t)·(i/3) = t`. So the
//! straight chord from `P0` to `P3` is itself a Bézier over the same basis,
//! with control points `L(i/3)` on the chord. Subtracting,
//!
//! ```text
//! B(t) - L(t) = Σ B_i(t) · (P_i - L(i/3))
//! ```
//!
//! and because the `B_i(t)` are non-negative and sum to one, the difference is
//! a convex combination:
//!
//! ```text
//! ‖B(t) - L(t)‖ ≤ max_i ‖P_i - L(i/3)‖
//! ```
//!
//! The `i = 0` and `i = 3` terms vanish, leaving [`Cubic::chord_error_bound`].
//! This is an upper bound, not an estimate, so a candidate accepted under it is
//! accepted under the true distance too.
//!
//! Splitting a cubic at its midpoint scales that bound by `1/4` (the bound is a
//! second difference of the control points, and de Casteljau halves the control
//! polygon's spacing). After `d` uniform subdivisions every piece is therefore
//! within `bound / 4^d` of its chord. [`Cubic::flatten`] chooses the smallest
//! `d` meeting the caller's tolerance, and *rejects* rather than silently
//! degrading when the cap is reached.
//!
//! # Provenance
//!
//! Written from §11.4's Bézier definition and the convex-hull/linear-precision
//! properties of the Bernstein basis, both textbook results (Farin, *Curves and
//! Surfaces for CAGD*, §4.3). No implementation was consulted.

use palette_tracer_core::ir::{Point, Vec2};

/// Whether `value` is a real number no greater than `limit`.
///
/// Every tolerance test in this crate goes through here, and the reason is
/// NaN. `value <= limit` is false for NaN, so a naive `if error > tolerance
/// { reject }` *accepts* a NaN error — the one value that must never pass.
/// Writing the test as "is it inside?" rather than "is it outside?" makes the
/// safe answer the default one, and `partial_cmp` says out loud that the two
/// values may not be comparable at all (PTE-DET-002).
#[must_use]
pub fn within(value: f64, limit: f64) -> bool {
    matches!(
        value.partial_cmp(&limit),
        Some(core::cmp::Ordering::Less | core::cmp::Ordering::Equal)
    )
}

/// The largest number of uniform subdivisions [`Cubic::flatten`] will perform.
///
/// `2^10 = 1024` chords is far beyond any span this fitter produces; reaching
/// the cap means the candidate is pathological, and PTE-GEO-007's "conservative
/// cap plus rejection" applies.
const MAX_FLATTEN_DEPTH: u32 = 10;

/// A cubic Bézier in image coordinates (§11.4).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Cubic {
    /// Start point.
    pub p0: Point,
    /// First control point.
    pub p1: Point,
    /// Second control point.
    pub p2: Point,
    /// End point.
    pub p3: Point,
}

impl Cubic {
    /// A cubic from its four control points.
    #[must_use]
    pub const fn new(p0: Point, p1: Point, p2: Point, p3: Point) -> Self {
        Self { p0, p1, p2, p3 }
    }

    /// `B(t)`, by the §11.4 definition.
    #[must_use]
    pub fn eval(&self, t: f64) -> Point {
        let u = 1.0 - t;
        let (b0, b1, b2, b3) = (u * u * u, 3.0 * u * u * t, 3.0 * u * t * t, t * t * t);
        Point::new(
            b0 * self.p0.x + b1 * self.p1.x + b2 * self.p2.x + b3 * self.p3.x,
            b0 * self.p0.y + b1 * self.p1.y + b2 * self.p2.y + b3 * self.p3.y,
        )
    }

    /// `B'(t)`.
    #[must_use]
    pub fn deriv(&self, t: f64) -> Vec2 {
        let u = 1.0 - t;
        let (a, b, c) = (3.0 * u * u, 6.0 * u * t, 3.0 * t * t);
        let d1 = self.p1 - self.p0;
        let d2 = self.p2 - self.p1;
        let d3 = self.p3 - self.p2;
        Vec2::new(
            a * d1.x + b * d2.x + c * d3.x,
            a * d1.y + b * d2.y + c * d3.y,
        )
    }

    /// `B''(t)`, needed by the Newton step that reparameterises a fit.
    #[must_use]
    pub fn deriv2(&self, t: f64) -> Vec2 {
        let u = 1.0 - t;
        let a = Vec2::new(
            self.p2.x - 2.0 * self.p1.x + self.p0.x,
            self.p2.y - 2.0 * self.p1.y + self.p0.y,
        );
        let b = Vec2::new(
            self.p3.x - 2.0 * self.p2.x + self.p1.x,
            self.p3.y - 2.0 * self.p2.y + self.p1.y,
        );
        a * (6.0 * u) + b * (6.0 * t)
    }

    /// A rigorous upper bound on `max_t ‖B(t) - L(t)‖`, where `L` is the chord.
    ///
    /// See the module documentation for the derivation. This is what makes
    /// flattening provable rather than heuristic.
    #[must_use]
    pub fn chord_error_bound(&self) -> f64 {
        let one_third = self.p0.lerp(self.p3, 1.0 / 3.0);
        let two_thirds = self.p0.lerp(self.p3, 2.0 / 3.0);
        (self.p1 - one_third)
            .length()
            .max((self.p2 - two_thirds).length())
    }

    /// Points of a polyline within `tolerance` of the curve, or `None` if the
    /// depth cap was reached first (PTE-GEO-007's rejection branch).
    ///
    /// The returned polyline includes both endpoints and is monotone in `t`, so
    /// a caller may treat consecutive points as chords of curve pieces.
    #[must_use]
    pub fn flatten(&self, tolerance: f64) -> Option<Vec<Point>> {
        if !tolerance.is_finite() || tolerance <= 0.0 {
            return None;
        }
        let bound = self.chord_error_bound();
        if !bound.is_finite() {
            return None;
        }

        // Smallest `d` with `bound / 4^d <= tolerance`, found by a bounded loop
        // rather than a logarithm, so the result is exact at the boundary and
        // identical on every target.
        let mut depth = 0u32;
        let mut remaining = bound;
        while remaining > tolerance {
            if depth >= MAX_FLATTEN_DEPTH {
                return None;
            }
            remaining /= 4.0;
            depth += 1;
        }

        let steps = 1usize << depth;
        let mut points = Vec::with_capacity(steps + 1);
        points.push(self.p0);
        for i in 1..steps {
            points.push(self.eval(i as f64 / steps as f64));
        }
        // The last point is the control point itself, not `eval(1.0)`, so a
        // pinned endpoint stays bit-identical through flattening.
        points.push(self.p3);
        Some(points)
    }

    /// Total unsigned turning of the tangent along the curve, in radians.
    ///
    /// Used to reject loops and cusps (§11.5): a span fitted between two
    /// corners has no business turning through more than a half circle, and a
    /// cubic that does is either looping or has a cusp inside it.
    #[must_use]
    pub fn total_turning(&self, samples: u32) -> f64 {
        let mut total = 0.0;
        let mut previous: Option<Vec2> = None;
        for i in 0..=samples {
            let t = f64::from(i) / f64::from(samples);
            let Some(d) = self.deriv(t).normalized(1e-12) else {
                // A vanishing derivative is a cusp. Reporting an impossible
                // turning makes the caller reject it without a second code
                // path.
                return f64::INFINITY;
            };
            if let Some(p) = previous {
                total += p.dot(d).clamp(-1.0, 1.0).acos();
            }
            previous = Some(d);
        }
        total
    }
}

/// Distance from `p` to the segment `a`–`b`.
#[must_use]
pub fn point_to_segment(p: Point, a: Point, b: Point) -> f64 {
    let ab = b - a;
    let len_sq = ab.length_squared();
    if len_sq <= f64::MIN_POSITIVE {
        return p.distance(a);
    }
    let t = ((p - a).dot(ab) / len_sq).clamp(0.0, 1.0);
    p.distance(a + ab * t)
}

/// Distance from `p` to the polyline through `points`.
///
/// Returns infinity for a polyline with no points, which loses any comparison
/// against a tolerance rather than needing a separate empty case.
#[must_use]
pub fn point_to_polyline(p: Point, points: &[Point]) -> f64 {
    match points {
        [] => f64::INFINITY,
        [only] => p.distance(*only),
        _ => points
            .windows(2)
            .map(|w| point_to_segment(p, w[0], w[1]))
            .fold(f64::INFINITY, f64::min),
    }
}

/// How far past the last improvement the monotone sweep keeps looking before
/// concluding it has passed the nearest segment.
const SWEEP_LOOKAHEAD: usize = 8;

/// Distance from every point of `from` to the polyline `to`, swept in order.
///
/// Returns `None` as soon as a distance exceeds `limit`, which is what makes
/// rejecting a hopeless candidate cheap.
///
/// # Why a sweep is sound here
///
/// The exhaustive form costs `O(|from| · |to|)`, and on a 360-sample arc
/// against a 1024-segment flattening that is the dominant cost of the whole
/// fitter. But both sequences traverse *the same boundary in the same
/// direction*, so the nearest segment to each successive point advances
/// monotonically. The sweep starts each search where the previous one ended,
/// keeps looking [`SWEEP_LOOKAHEAD`] segments past the last improvement, and
/// never moves the cursor backwards, giving `O(|from| + |to|)`.
///
/// Where the monotone assumption fails — a curve that doubles back on itself —
/// the sweep returns an *over*-estimate, because it can only miss a nearer
/// segment behind the cursor. An over-estimate can reject a good candidate; it
/// cannot accept a bad one. The error bound the caller relies on is therefore
/// still an upper bound, which is the property that matters.
#[must_use]
pub fn sweep_distances(from: &[Point], to: &[Point], limit: f64) -> Option<Vec<f64>> {
    if to.len() < 2 {
        return None;
    }
    let mut out = Vec::with_capacity(from.len());
    let mut cursor = 0usize;

    for &p in from {
        let mut best = f64::INFINITY;
        let mut best_at = cursor;
        let mut stale = 0usize;
        let mut i = cursor;
        while i + 1 < to.len() {
            let d = point_to_segment(p, to[i], to[i + 1]);
            if d < best {
                best = d;
                best_at = i;
                stale = 0;
            } else {
                stale += 1;
                if stale >= SWEEP_LOOKAHEAD {
                    break;
                }
            }
            i += 1;
        }
        if !within(best, limit) {
            return None;
        }
        out.push(best);
        cursor = best_at;
    }
    Some(out)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::float_cmp)]
mod tests {
    use super::*;

    /// A cubic whose control points are collinear and evenly spaced *is* a
    /// straight line, and the bound must say so exactly.
    #[test]
    fn a_degenerate_cubic_has_a_zero_chord_bound() {
        let c = Cubic::new(
            Point::new(0.0, 0.0),
            Point::new(1.0, 0.0),
            Point::new(2.0, 0.0),
            Point::new(3.0, 0.0),
        );
        assert_eq!(c.chord_error_bound(), 0.0);
        assert_eq!(c.flatten(0.01).unwrap().len(), 2, "no subdivision needed");
    }

    /// The claim the whole module rests on: the bound is an *upper* bound. If
    /// this ever fails, every accepted candidate in the fitter is suspect.
    #[test]
    fn the_chord_bound_is_never_exceeded_by_the_real_curve() {
        // A deliberately awkward spread of shapes, including asymmetric and
        // near-degenerate control polygons.
        let cases = [
            Cubic::new(
                Point::new(0.0, 0.0),
                Point::new(0.0, 4.0),
                Point::new(6.0, 4.0),
                Point::new(6.0, 0.0),
            ),
            Cubic::new(
                Point::new(-3.0, 1.0),
                Point::new(12.0, -8.0),
                Point::new(-7.0, -8.0),
                Point::new(2.0, 1.0),
            ),
            Cubic::new(
                Point::new(0.0, 0.0),
                Point::new(0.01, 5.0),
                Point::new(0.02, -5.0),
                Point::new(0.03, 0.0),
            ),
        ];

        for c in cases {
            let bound = c.chord_error_bound();
            for i in 0..=512 {
                let t = f64::from(i) / 512.0;
                let on_curve = c.eval(t);
                let on_chord = c.p0.lerp(c.p3, t);
                let actual = on_curve.distance(on_chord);
                assert!(
                    actual <= bound + 1e-12,
                    "the bound {bound} was exceeded by {actual} at t = {t}"
                );
            }
        }
    }

    /// Flattening must honour the tolerance it was given, measured against the
    /// real curve rather than against the bound that chose the depth.
    #[test]
    fn a_flattened_polyline_stays_within_its_tolerance() {
        let c = Cubic::new(
            Point::new(0.0, 0.0),
            Point::new(0.0, 10.0),
            Point::new(20.0, 10.0),
            Point::new(20.0, 0.0),
        );
        for tolerance in [1.0, 0.1, 0.01, 0.001] {
            let flat = c.flatten(tolerance).expect("a benign curve flattens");
            for i in 0..=1024 {
                let t = f64::from(i) / 1024.0;
                let d = point_to_polyline(c.eval(t), &flat);
                assert!(
                    d <= tolerance + 1e-9,
                    "tolerance {tolerance} missed by {d} at t = {t}"
                );
            }
        }
    }

    /// Endpoints survive flattening bit-for-bit. A pinned junction that moved
    /// by one ulp during a *measurement* would be a defect that only showed up
    /// as a seam much later (PTE-GEO-013).
    #[test]
    fn flattening_preserves_the_endpoints_exactly() {
        let c = Cubic::new(
            Point::new(1.25, -3.5),
            Point::new(4.0, 9.0),
            Point::new(-2.0, 7.0),
            Point::new(8.125, 0.75),
        );
        let flat = c.flatten(0.01).unwrap();
        assert_eq!(flat[0], c.p0);
        assert_eq!(flat[flat.len() - 1], c.p3);
    }

    /// PTE-GEO-006: a pathological curve is rejected, not approximated. The
    /// cap is what makes "bounded" true rather than aspirational.
    #[test]
    fn an_impossible_tolerance_is_rejected_rather_than_subdivided_forever() {
        let c = Cubic::new(
            Point::new(0.0, 0.0),
            Point::new(0.0, 1.0e6),
            Point::new(1.0, 1.0e6),
            Point::new(1.0, 0.0),
        );
        assert!(c.flatten(1e-9).is_none());
        assert!(c.flatten(0.0).is_none());
        assert!(c.flatten(f64::NAN).is_none());
    }

    /// Turning is the statistic that rejects cusps and loops. A curve that
    /// doubles back reverses its tangent twice, so its turning far exceeds the
    /// half circle a span between two corners is allowed.
    #[test]
    fn a_doubling_back_curve_reports_more_turning_than_a_half_circle() {
        let cusp = Cubic::new(
            Point::new(0.0, 0.0),
            Point::new(4.0, 0.0),
            Point::new(-4.0, 0.0),
            Point::new(0.0, 0.0),
        );
        assert!(
            cusp.total_turning(32) > std::f64::consts::PI,
            "got {}",
            cusp.total_turning(32)
        );

        // A derivative that actually vanishes is reported as infinite turning,
        // so the caller needs no separate degeneracy branch.
        let stationary = Cubic::new(
            Point::new(0.0, 0.0),
            Point::new(0.0, 0.0),
            Point::new(4.0, 0.0),
            Point::new(4.0, 0.0),
        );
        assert!(!stationary.total_turning(32).is_finite());

        let gentle = Cubic::new(
            Point::new(0.0, 0.0),
            Point::new(2.0, 1.0),
            Point::new(4.0, 1.0),
            Point::new(6.0, 0.0),
        );
        assert!(gentle.total_turning(32) < std::f64::consts::PI);
    }

    #[test]
    fn point_to_segment_handles_both_ends_and_the_interior() {
        let a = Point::new(0.0, 0.0);
        let b = Point::new(4.0, 0.0);
        assert_eq!(point_to_segment(Point::new(2.0, 3.0), a, b), 3.0);
        assert_eq!(point_to_segment(Point::new(-3.0, 0.0), a, b), 3.0);
        assert_eq!(point_to_segment(Point::new(7.0, 0.0), a, b), 3.0);
        // A zero-length segment is a point, not a division by zero.
        assert_eq!(point_to_segment(Point::new(3.0, 4.0), a, a), 5.0);
    }
}
