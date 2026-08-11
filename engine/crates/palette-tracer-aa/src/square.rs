//! Coverage of a unit pixel square by a half-plane, and its exact inverse
//! (§10.3).
//!
//! # The problem
//!
//! §10.3: "Approximate the boundary locally by a line with unit normal `n` and
//! signed offset `d` from the pixel center. Let `A_square(d,n)` be the area
//! fraction of the unit pixel square on the `A` side. Recover
//! `d* = A_square⁻¹(α*, n)`."
//!
//! # Conventions
//!
//! The pixel is the unit square `[−½, ½]²` centred on the origin. `n` is the
//! unit normal pointing **out of `A`**, so the `A` side is the half-plane
//! `{x : n·x ≤ d}` and coverage increases with `d`. `d = 0` is a line through
//! the pixel centre, which covers exactly half.
//!
//! # Why this is closed form
//!
//! §10.3 offers three implementations: exact piecewise analytic intersection,
//! a monotone precomputed table with interpolation, or a bounded root solve.
//! This is the first, and it is worth taking some care over why it is the
//! cheapest of the three rather than the dearest.
//!
//! The unit square is invariant under the dihedral group of order eight: the
//! reflections `x ↔ −x`, `y ↔ −y` and `x ↔ y` all map it to itself. The area
//! cut off by a half-plane is therefore a function of the *sorted absolute
//! values* of the normal's components alone. Writing
//!
//! ```text
//! a = max(|n_x|, |n_y|),   b = min(|n_x|, |n_y|),   a² + b² = 1
//! ```
//!
//! reduces every direction to the first octant `a ≥ b ≥ 0`, where the coverage
//! is piecewise quadratic in `d` with three pieces:
//!
//! ```text
//!                    0                                d ≤ −(a+b)/2
//!   A(d) =   (d + (a+b)/2)² / (2ab)         −(a+b)/2 ≤ d ≤ −(a−b)/2
//!            ½ + d/a                        −(a−b)/2 ≤ d ≤  (a−b)/2
//!            1 − ((a+b)/2 − d)² / (2ab)      (a−b)/2 ≤ d ≤  (a+b)/2
//!                    1                                d ≥  (a+b)/2
//! ```
//!
//! The outer pieces are the two corner triangles and the middle piece is the
//! trapezoid. Each is invertible in closed form, so the inverse needs one
//! square root and no iteration:
//!
//! ```text
//!   d(α) =  −(a+b)/2 + √(2ab·α)                    α ≤ b/(2a)
//!            a(α − ½)                     b/(2a) ≤ α ≤ 1 − b/(2a)
//!            (a+b)/2 − √(2ab(1−α))        1 − b/(2a) ≤ α
//! ```
//!
//! That matters for more than speed. PTE-DET-004 requires the same *decision*
//! on every target, and neither a table's interpolation nor a root solve's
//! termination test is guaranteed to land identically on two platforms. A
//! square root is: IEEE-754 requires `sqrt` to be correctly rounded, so this
//! inverse is bit-identical wherever it runs. It is also exactly monotone and
//! exactly symmetric by construction rather than to within a tolerance, which
//! is what PTE-AA-003 asks to be tested.
//!
//! # Degenerate direction
//!
//! When `b = 0` the normal is axis-aligned, the corner triangles have zero
//! width, and `2ab = 0`. Both outer branches are then unreachable because
//! `b/(2a) = 0`, so the middle branch covers the whole unit interval and the
//! division never happens. [`offset_for_coverage`] tests the branch bounds
//! rather than the denominator, so this falls out instead of being special
//! cased.
//!
//! # Provenance
//!
//! The square/half-plane intersection is elementary plane geometry; the
//! octant reduction and the piecewise inverse are derived here from §10.3's
//! statement of the problem. Written independently (PTE-LIC-004).

use palette_tracer_core::checked::clamp_unit;

/// A unit normal reduced to the first octant, where the coverage law is one
/// pair of formulae for every direction.
///
/// `a ≥ b ≥ 0` and `a² + b² = 1`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Octant {
    /// The larger absolute component.
    pub a: f64,
    /// The smaller absolute component.
    pub b: f64,
}

impl Octant {
    /// Reduce a normal to the first octant.
    ///
    /// Returns `None` if the vector is not usable as a direction: zero length,
    /// or non-finite. PTE-AA-002 wants that stated rather than papered over
    /// with a default direction.
    #[must_use]
    pub fn of(nx: f64, ny: f64) -> Option<Self> {
        if !nx.is_finite() || !ny.is_finite() {
            return None;
        }
        let length = nx.hypot(ny);
        if length <= 0.0 || !length.is_finite() {
            return None;
        }
        let (x, y) = ((nx / length).abs(), (ny / length).abs());
        Some(Self {
            a: x.max(y),
            b: x.min(y),
        })
    }

    /// Half the width of the pixel's shadow on the normal: the offset at which
    /// coverage reaches `0` from below and `1` from above.
    ///
    /// This is the support half-width `(a + b)/2`, and it is exactly the bound
    /// PTE-AA-005 means by "bounded to the local pixel support".
    #[must_use]
    pub fn support(self) -> f64 {
        (self.a + self.b) / 2.0
    }

    /// Coverage at which the corner triangle gives way to the trapezoid.
    #[must_use]
    fn corner_coverage(self) -> f64 {
        // `a > 0` always: `a ≥ b ≥ 0` and `a² + b² = 1` force `a ≥ 1/√2`.
        self.b / (2.0 * self.a)
    }
}

/// Area fraction of the unit pixel square on the `A` side of the line
/// `n·x = d`, where `n` points out of `A` (§10.3).
///
/// This is `A_square(d, n)`. It is the forward direction, kept because the
/// inverse's tests are written against it: a claimed inverse that is never
/// composed with the forward map is a claim, not a check.
#[must_use]
pub fn coverage_for_offset(offset: f64, octant: Octant) -> f64 {
    let Octant { a, b } = octant;
    let support = octant.support();
    let inner = (a - b) / 2.0;

    if offset <= -support {
        return 0.0;
    }
    if offset >= support {
        return 1.0;
    }
    if offset <= -inner {
        // Lower corner triangle. `2ab > 0` here: `b = 0` would make
        // `inner = support` and this branch unreachable.
        let reach = offset + support;
        return clamp_unit(reach * reach / (2.0 * a * b));
    }
    if offset >= inner {
        let reach = support - offset;
        return clamp_unit(1.0 - reach * reach / (2.0 * a * b));
    }
    clamp_unit(0.5 + offset / a)
}

/// `A_square⁻¹(α, n)`: the signed offset from the pixel centre at which a line
/// with normal `n` cuts off coverage `α` (§10.3).
///
/// The result is always within `±octant.support()`, which is the pixel's own
/// extent along the normal. PTE-AA-005's "recovered offsets MUST be bounded to
/// the local pixel support" is therefore a property of the formula rather than
/// a clamp applied afterwards.
///
/// # Monotonicity and symmetry
///
/// PTE-AA-003 requires both, and both hold exactly:
///
/// * each branch is increasing in `α`, and the branches agree at their shared
///   bounds, so the whole map is increasing;
/// * `d(1 − α) = −d(α)`, because the two corner branches are reflections of
///   one another and the middle branch is odd about `α = ½`. Reversing the
///   labels swaps `A` and `B`, which complements the coverage and negates the
///   normal -- and negating the normal leaves the octant unchanged, so the
///   offset simply changes sign, as it must.
#[must_use]
pub fn offset_for_coverage(coverage: f64, octant: Octant) -> f64 {
    let Octant { a, b } = octant;
    let alpha = clamp_unit(coverage);
    let support = octant.support();
    let corner = octant.corner_coverage();

    if alpha <= corner {
        // `2ab·α` with `α ≤ b/(2a)` never exceeds `b²`, so the root is real
        // and at most `b`, keeping the result inside the support.
        return (2.0 * a * b * alpha).max(0.0).sqrt() - support;
    }
    if alpha >= 1.0 - corner {
        return support - (2.0 * a * b * (1.0 - alpha)).max(0.0).sqrt();
    }
    a * (alpha - 0.5)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::float_cmp)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    /// Directions covering every octant boundary and interior, including the
    /// two degenerate axis-aligned ones and the 45-degree diagonal.
    fn directions() -> Vec<(f64, f64)> {
        let mut out = Vec::new();
        for step in 0..32 {
            let angle = f64::from(step) * std::f64::consts::TAU / 32.0;
            out.push((angle.cos(), angle.sin()));
        }
        out.extend([(1.0, 0.0), (0.0, 1.0), (-1.0, 0.0), (0.0, -1.0)]);
        out
    }

    /// PTE-AA-003, the composition check: the inverse really is the inverse of
    /// `A_square`, over all octants.
    #[test]
    fn the_inverse_recovers_the_offset_that_produced_the_coverage() {
        for (nx, ny) in directions() {
            let octant = Octant::of(nx, ny).unwrap();
            let support = octant.support();
            for step in 0..=40 {
                let offset = -support + 2.0 * support * f64::from(step) / 40.0;
                let alpha = coverage_for_offset(offset, octant);
                let back = offset_for_coverage(alpha, octant);
                assert!(
                    (back - offset).abs() < 1e-12,
                    "n=({nx}, {ny}) offset {offset} -> alpha {alpha} -> {back}"
                );
            }
        }
    }

    /// The same claim from the other side, which catches a branch that is
    /// self-consistent but wrong.
    #[test]
    fn the_forward_map_recovers_the_coverage_that_produced_the_offset() {
        for (nx, ny) in directions() {
            let octant = Octant::of(nx, ny).unwrap();
            for step in 0..=40 {
                let alpha = f64::from(step) / 40.0;
                let offset = offset_for_coverage(alpha, octant);
                let back = coverage_for_offset(offset, octant);
                assert!(
                    (back - alpha).abs() < 1e-12,
                    "n=({nx}, {ny}) alpha {alpha} -> offset {offset} -> {back}"
                );
            }
        }
    }

    /// PTE-AA-003: "The inverse must be monotone".
    #[test]
    fn the_inverse_is_monotone_in_every_octant() {
        for (nx, ny) in directions() {
            let octant = Octant::of(nx, ny).unwrap();
            let mut previous = f64::NEG_INFINITY;
            for step in 0..=200 {
                let alpha = f64::from(step) / 200.0;
                let offset = offset_for_coverage(alpha, octant);
                assert!(
                    offset >= previous,
                    "n=({nx}, {ny}) not monotone at alpha {alpha}: {offset} < {previous}"
                );
                previous = offset;
            }
        }
    }

    /// PTE-AA-003: "symmetric under label/normal reversal". Calling the other
    /// region `A` complements the coverage and must mirror the offset.
    #[test]
    fn reversing_the_labels_mirrors_the_offset() {
        for (nx, ny) in directions() {
            let octant = Octant::of(nx, ny).unwrap();
            // Reversing the normal must not change the octant at all, because
            // the pixel square is centrally symmetric.
            assert_eq!(Octant::of(-nx, -ny).unwrap(), octant);
            for step in 0..=40 {
                let alpha = f64::from(step) / 40.0;
                let forward = offset_for_coverage(alpha, octant);
                let reverse = offset_for_coverage(1.0 - alpha, octant);
                assert!(
                    (forward + reverse).abs() < 1e-12,
                    "n=({nx}, {ny}) alpha {alpha}: {forward} vs {reverse}"
                );
            }
        }
    }

    /// PTE-AA-005: "Recovered offsets MUST be bounded to the local pixel
    /// support." The support is at most `1/√2 ≈ 0.7071`, at 45 degrees.
    #[test]
    fn every_offset_lies_within_the_pixel_support() {
        for (nx, ny) in directions() {
            let octant = Octant::of(nx, ny).unwrap();
            let support = octant.support();
            assert!(support <= std::f64::consts::FRAC_1_SQRT_2 + 1e-15);
            for step in 0..=40 {
                let alpha = f64::from(step) / 40.0;
                let offset = offset_for_coverage(alpha, octant);
                assert!(offset.abs() <= support + 1e-12, "offset {offset} escapes");
            }
        }
    }

    /// Half coverage is a line through the pixel centre, whatever the normal.
    /// This is the one value the whole scheme has to get right, because it is
    /// where thresholding and reconstruction agree.
    #[test]
    fn half_coverage_is_the_pixel_centre_for_every_normal() {
        for (nx, ny) in directions() {
            let octant = Octant::of(nx, ny).unwrap();
            assert!(offset_for_coverage(0.5, octant).abs() < 1e-15);
        }
    }

    /// The axis-aligned case, where the answer is elementary and can be
    /// written down: a vertical line at `x = d` covers `d + ½` of the square.
    #[test]
    fn an_axis_aligned_normal_gives_the_elementary_answer() {
        let octant = Octant::of(1.0, 0.0).unwrap();
        assert_eq!(octant.support(), 0.5);
        for step in 0..=10 {
            let alpha = f64::from(step) / 10.0;
            assert!((offset_for_coverage(alpha, octant) - (alpha - 0.5)).abs() < 1e-15);
        }
    }

    /// The 45-degree case, where the corner triangles meet in the middle and
    /// the trapezoid branch vanishes: coverage is `√α`-shaped throughout.
    #[test]
    fn a_diagonal_normal_is_two_corner_triangles_with_no_trapezoid() {
        let octant = Octant::of(1.0, 1.0).unwrap();
        assert!((octant.corner_coverage() - 0.5).abs() < 1e-15);
        // A quarter of the square is cut off by a diagonal line at
        // `d = −1/√2 + √(1/2)·... `; check against the triangle's own area.
        let offset = offset_for_coverage(0.25, octant);
        let side = (2.0f64 * 0.25).sqrt() * octant.a; // triangle leg
        assert!((offset - (side - octant.support())).abs() < 1e-12);
    }

    /// PTE-AA-002: a direction that is not a direction has no answer.
    #[test]
    fn a_degenerate_normal_has_no_octant() {
        assert!(Octant::of(0.0, 0.0).is_none());
        assert!(Octant::of(f64::NAN, 1.0).is_none());
        assert!(Octant::of(f64::INFINITY, 1.0).is_none());
    }

    proptest! {
        /// Monotonicity and the support bound over arbitrary directions and
        /// coverages, rather than on a sampled grid.
        #[test]
        fn the_inverse_is_total_monotone_and_bounded(
            angle in 0.0f64..std::f64::consts::TAU,
            lo in 0.0f64..=1.0,
            hi in 0.0f64..=1.0,
        ) {
            let octant = Octant::of(angle.cos(), angle.sin()).unwrap();
            let (lo, hi) = if lo <= hi { (lo, hi) } else { (hi, lo) };
            let a = offset_for_coverage(lo, octant);
            let b = offset_for_coverage(hi, octant);
            prop_assert!(a.is_finite() && b.is_finite());
            prop_assert!(a <= b);
            prop_assert!(a.abs() <= octant.support() + 1e-12);
            prop_assert!(b.abs() <= octant.support() + 1e-12);
        }

        /// Round-tripping through the forward map, for arbitrary directions.
        #[test]
        fn round_trip_holds_generally(
            angle in 0.0f64..std::f64::consts::TAU,
            alpha in 0.0f64..=1.0,
        ) {
            let octant = Octant::of(angle.cos(), angle.sin()).unwrap();
            let offset = offset_for_coverage(alpha, octant);
            let back = coverage_for_offset(offset, octant);
            prop_assert!((back - alpha).abs() < 1e-9, "{alpha} -> {offset} -> {back}");
        }
    }
}
