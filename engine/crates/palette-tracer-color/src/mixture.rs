//! Two-colour and multi-colour mixture estimation (§10.2, §10.4).
//!
//! An antialiased boundary pixel is approximately an area-coverage mixture of
//! the two colours on either side. Recovering that coverage is the first half
//! of subpixel boundary reconstruction; turning it into a position is the
//! second half and lives in the geometry crate (§10.3).
//!
//! It lives in the *colour* crate because it is colour mathematics, and
//! because two stages need it: segmentation uses the residual to recognise
//! antialias fringe regions (§8.6), and boundary refinement uses the coverage
//! (§10.2).
//!
//! # The estimate
//!
//! For observed `C` and endpoints `A`, `B`, the least-squares coverage of `A`
//! is the projection onto the segment:
//!
//! ```text
//! α* = clamp( (C − B)·(A − B) / ‖A − B‖² , 0, 1 )
//! r  = ‖C − (α* A + (1 − α*) B)‖
//! ```
//!
//! PTE-AA-001: in linear light, on premultiplied values. PTE-AA-002: when
//! `‖A − B‖²` is ill-conditioned or the residual fails, the answer is a
//! declared low-confidence outcome, never an amplified one.
//!
//! # Provenance
//!
//! The projection is elementary least squares; the formulation and the
//! conditioning rule are §10.2 and Appendix D.4. Written independently
//! (PTE-LIC-004).

use crate::alpha::{ALPHA_CHANNEL_WEIGHT, PremulLinearRgba};
use palette_tracer_core::determinism::QuantKey;

/// Why a mixture estimate could not be trusted (Appendix D.4).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum MixtureRejection {
    /// The two endpoint colours are too close to separate: `‖A − B‖²` is below
    /// the configured minimum, so any coverage would be noise divided by
    /// noise.
    IllConditionedColors,
    /// The observation is not on the segment between the endpoints, so it is
    /// not a two-colour mixture of them.
    NotTwoColorMixture,
}

/// A successful two-colour mixture estimate.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Mixture {
    /// Least-squares coverage of `A`, in `[0, 1]`.
    pub coverage: f64,
    /// Residual after projection, in linear-light units.
    pub residual: f64,
    /// Squared colour separation `‖A − B‖²`, which is the conditioning of the
    /// estimate and feeds the confidence (PTE-AA-005).
    pub separation_squared: f64,
}

/// Thresholds for the mixture estimate.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MixturePolicy {
    /// Smallest `‖A − B‖²` that counts as separable.
    ///
    /// One 8-bit code point in linear light near mid-grey is about 0.003, and
    /// its square is about 1e-5. Below that the two sides are the same colour
    /// as far as the raster can express, and the projection's denominator is
    /// dominated by quantisation.
    pub min_separation_squared: f64,
    /// Largest residual at which the observation still counts as a mixture of
    /// the two endpoints.
    pub max_residual: f64,
    /// Weight on the alpha channel in the four-dimensional fit (§10.2).
    pub alpha_weight: f64,
}

impl Default for MixturePolicy {
    fn default() -> Self {
        Self {
            min_separation_squared: 1.0e-5,
            max_residual: 0.05,
            alpha_weight: ALPHA_CHANNEL_WEIGHT,
        }
    }
}

/// Estimate the coverage of `a` in the observation `c` (§10.2, Appendix D.4).
///
/// # Errors
///
/// [`MixtureRejection`] when the endpoints are inseparable or the observation
/// is not a mixture of them. PTE-AA-002 requires the caller to fall back to a
/// declared geometric estimate rather than use an amplified number.
pub fn two_color(
    c: PremulLinearRgba,
    a: PremulLinearRgba,
    b: PremulLinearRgba,
    policy: &MixturePolicy,
) -> Result<Mixture, MixtureRejection> {
    let direction = a.minus(b);
    let separation_squared = direction.weighted_norm_squared(policy.alpha_weight);
    if separation_squared <= policy.min_separation_squared || !separation_squared.is_finite() {
        return Err(MixtureRejection::IllConditionedColors);
    }

    let coverage = palette_tracer_core::checked::clamp_unit(
        c.minus(b).weighted_dot(direction, policy.alpha_weight) / separation_squared,
    );

    let predicted = a.scale(coverage).plus(b.scale(1.0 - coverage));
    let residual = c
        .minus(predicted)
        .weighted_norm_squared(policy.alpha_weight)
        .max(0.0)
        .sqrt();

    if residual > policy.max_residual {
        return Err(MixtureRejection::NotTwoColorMixture);
    }

    Ok(Mixture {
        coverage,
        residual,
        separation_squared,
    })
}

/// Residual of the best two-colour explanation, ignoring the mixture gates.
///
/// Segmentation needs the number even when it is large -- §8.6 decides whether
/// a thin region *is* fringe by comparing the residual to a threshold, so
/// being told "rejected" instead of the value would lose the comparison.
#[must_use]
pub fn two_color_residual(
    c: PremulLinearRgba,
    a: PremulLinearRgba,
    b: PremulLinearRgba,
    policy: &MixturePolicy,
) -> Option<(f64, f64)> {
    let direction = a.minus(b);
    let separation_squared = direction.weighted_norm_squared(policy.alpha_weight);
    if separation_squared <= policy.min_separation_squared || !separation_squared.is_finite() {
        return None;
    }
    let coverage = palette_tracer_core::checked::clamp_unit(
        c.minus(b).weighted_dot(direction, policy.alpha_weight) / separation_squared,
    );
    let predicted = a.scale(coverage).plus(b.scale(1.0 - coverage));
    let residual = c
        .minus(predicted)
        .weighted_norm_squared(policy.alpha_weight)
        .max(0.0)
        .sqrt();
    Some((coverage, residual))
}

/// Which compositing transfer the source raster's antialiasing used.
///
/// §10.2's model `C = αA + (1 − α)B` is only the right model if the *source*
/// composited in linear light. Most 2D rasterisers do not: they blend the
/// transfer-encoded bytes directly. That is arithmetically a different curve,
/// and fitting a straight line to it produces a systematic error in both the
/// residual and the coverage, not a random one.
///
/// The size of the error is not marginal. For an sRGB-space blend of
/// `#202634` and `#f7f4ec`, a true coverage of `0.5` projects onto the linear
/// segment at `0.717`; for `#c63e36` and `#f7f4ec` it projects at `0.676`. Fed
/// to §10.3's `A_square⁻¹` that is a fifth of a pixel of position error, which
/// is larger than the grid error §10 exists to remove.
///
/// So the transfer is *estimated from the evidence* rather than assumed, by
/// scoring both hypotheses and keeping the one that explains the observation.
/// PTE-AA-001 is satisfied throughout: every value compared, and every residual
/// reported, is linear-light premultiplied. Only the parameter search for
/// [`Self::EncodedSrgb`] happens in the encoded space, because that is what the
/// hypothesis *is*.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum CompositeSpace {
    /// The source composited in linear light: §10.2's literal model.
    LinearLight,
    /// The source composited on transfer-encoded sRGB values.
    EncodedSrgb,
}

impl CompositeSpace {
    /// Stable identifier for the report and the design note.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::LinearLight => "linear-light",
            Self::EncodedSrgb => "encoded-srgb",
        }
    }
}

/// A two-colour estimate together with the transfer that produced it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SpacedMixture {
    /// The estimate, whose `residual` is always in linear-light units.
    pub mixture: Mixture,
    /// Which hypothesis won.
    pub space: CompositeSpace,
}

/// Alpha below which a premultiplied value carries no recoverable straight
/// colour, so the encoded hypothesis cannot be formed (PTE-COLOR-009).
const STRAIGHT_EPSILON: f64 = 1.0e-6;

/// Encode a premultiplied linear sample into the transfer-encoded coordinates
/// the encoded hypothesis is linear in.
///
/// Alpha is *not* transfer-encoded -- no raster format encodes it -- so it
/// passes through and is carried as the fourth coordinate. Returns `None` when
/// the straight colour cannot be recovered.
fn to_encoded_coords(v: PremulLinearRgba) -> Option<[f64; 4]> {
    let straight = v.to_straight(STRAIGHT_EPSILON)?;
    let e = straight.to_encoded();
    Some([e.r, e.g, e.b, v.a])
}

/// Undo [`to_encoded_coords`].
fn from_encoded_coords(v: [f64; 4]) -> PremulLinearRgba {
    let linear = crate::spaces::EncodedSrgb::new(v[0], v[1], v[2]).to_linear();
    PremulLinearRgba::from_straight(linear, v[3])
}

/// The linear-light residual of the encoded-space hypothesis, and its coverage.
///
/// Under this hypothesis the observation is linear in the *encoded*
/// coordinates, so the coverage is the projection there. The residual is then
/// measured after decoding, in linear light, so that it is directly comparable
/// with [`two_color_residual`]'s.
#[must_use]
fn encoded_space_residual(
    c: PremulLinearRgba,
    a: PremulLinearRgba,
    b: PremulLinearRgba,
    policy: &MixturePolicy,
) -> Option<(f64, f64)> {
    let (ce, ae, be) = (
        to_encoded_coords(c)?,
        to_encoded_coords(a)?,
        to_encoded_coords(b)?,
    );

    let w = [1.0, 1.0, 1.0, policy.alpha_weight];
    let mut numerator = 0.0;
    let mut denominator = 0.0;
    for k in 0..4 {
        let d = ae[k] - be[k];
        numerator += w[k] * (ce[k] - be[k]) * d;
        denominator += w[k] * d * d;
    }
    if denominator <= policy.min_separation_squared || !denominator.is_finite() {
        return None;
    }

    let coverage = palette_tracer_core::checked::clamp_unit(numerator / denominator);
    let mut predicted_encoded = [0.0; 4];
    for k in 0..4 {
        predicted_encoded[k] = coverage.mul_add(ae[k] - be[k], be[k]);
    }
    // PTE-AA-001: the comparison is in linear light, so decode before measuring.
    let predicted = from_encoded_coords(predicted_encoded);
    let residual = c
        .minus(predicted)
        .weighted_norm_squared(policy.alpha_weight)
        .max(0.0)
        .sqrt();
    Some((coverage, residual))
}

/// Estimate coverage under both compositing hypotheses and keep the better
/// (§10.2, PTE-AA-002).
///
/// Returns `None` only when *neither* hypothesis can be formed, which means the
/// endpoints are inseparable. The mixture gates are not applied: like
/// [`two_color_residual`], the caller compares the residual to its own
/// threshold, because §8.6 and §10.3 use different ones.
///
/// # Determinism
///
/// The two residuals are compared through [`QuantKey`], never as bare floats
/// (PTE-DET-002/003). A tie keeps [`CompositeSpace::LinearLight`], which is
/// §10.2's stated model, so the spec's default is what a coin-flip resolves to.
#[must_use]
pub fn two_color_best(
    c: PremulLinearRgba,
    a: PremulLinearRgba,
    b: PremulLinearRgba,
    policy: &MixturePolicy,
) -> Option<SpacedMixture> {
    let separation_squared = a.minus(b).weighted_norm_squared(policy.alpha_weight);

    let linear = two_color_residual(c, a, b, policy).map(|(coverage, residual)| SpacedMixture {
        mixture: Mixture {
            coverage,
            residual,
            separation_squared,
        },
        space: CompositeSpace::LinearLight,
    });
    let encoded =
        encoded_space_residual(c, a, b, policy).map(|(coverage, residual)| SpacedMixture {
            mixture: Mixture {
                coverage,
                residual,
                separation_squared,
            },
            space: CompositeSpace::EncodedSrgb,
        });

    match (linear, encoded) {
        (Some(l), Some(e)) => {
            let lk = QuantKey::cost_or_worst(l.mixture.residual);
            let ek = QuantKey::cost_or_worst(e.mixture.residual);
            // `<` and not `<=`: a tie keeps the linear hypothesis.
            Some(if ek < lk { e } else { l })
        }
        (Some(l), None) => Some(l),
        (None, Some(e)) => Some(e),
        (None, None) => None,
    }
}

/// Barycentric coverage weights over up to four colours (§10.4).
///
/// Solves
///
/// ```text
/// min_w ‖C − Σ w_k A_k‖²   subject to   w_k ≥ 0,  Σ w_k = 1
/// ```
///
/// by enumerating active sets. §10.4 authorises exactly this: "The small
/// constrained problem MAY be solved by enumerating active sets because
/// `m ≤ 4` in a local pixel cell." With four colours there are fifteen
/// non-empty subsets, and each is a tiny unconstrained least-squares problem;
/// the best feasible one wins. Enumeration is exhaustive, so the answer is the
/// global optimum and there is no iteration count to make target-dependent.
///
/// PTE-AA-006: the weights are *evidence*. Junction optimisation must still
/// satisfy planar topology and incident-edge order.
///
/// Returns weights parallel to `colors`, or `None` if `colors` is empty or
/// longer than four.
#[must_use]
pub fn barycentric(
    c: PremulLinearRgba,
    colors: &[PremulLinearRgba],
    alpha_weight: f64,
) -> Option<Vec<f64>> {
    let m = colors.len();
    if m == 0 || m > 4 {
        return None;
    }

    let mut best: Option<(f64, Vec<f64>)> = None;

    // Every non-empty subset of the colours is one candidate active set.
    for mask in 1u32..(1u32 << m) {
        let members: Vec<usize> = (0..m).filter(|k| mask & (1 << k) != 0).collect();
        let Some(weights) = solve_simplex(c, colors, &members, alpha_weight) else {
            continue;
        };
        // Feasibility: every weight non-negative. The sum is one by
        // construction.
        if weights.iter().any(|&w| w < -1e-9) {
            continue;
        }
        let mut predicted = PremulLinearRgba::TRANSPARENT;
        for (k, &w) in weights.iter().enumerate() {
            predicted = predicted.plus(colors[k].scale(w));
        }
        let error = c.minus(predicted).weighted_norm_squared(alpha_weight);
        let better = best.as_ref().is_none_or(|(e, _)| error < *e - 1e-15);
        if better {
            best = Some((error, weights));
        }
    }

    best.map(|(_, w)| w)
}

/// Least squares over the affine hull of the selected colours.
///
/// With `members` fixed, the constraint `Σ w = 1` lets one weight be
/// eliminated, leaving an unconstrained problem in the differences from the
/// last member. Sizes are at most 3x3, so the normal equations are solved by
/// Gaussian elimination with partial pivoting -- exact enough at this size and
/// with no iteration to diverge.
fn solve_simplex(
    c: PremulLinearRgba,
    colors: &[PremulLinearRgba],
    members: &[usize],
    alpha_weight: f64,
) -> Option<Vec<f64>> {
    let k = members.len();
    let base = *members.last()?;

    if k == 1 {
        let mut w = vec![0.0; colors.len()];
        w[base] = 1.0;
        return Some(w);
    }

    // Basis of differences from the last member.
    let free = &members[..k - 1];
    let dim = free.len();
    let basis: Vec<PremulLinearRgba> = free
        .iter()
        .map(|&i| colors[i].minus(colors[base]))
        .collect();
    let target = c.minus(colors[base]);

    // Normal equations: (Bᵀ B) x = Bᵀ t.
    let mut matrix = vec![0.0f64; dim * dim];
    let mut rhs = vec![0.0f64; dim];
    for i in 0..dim {
        for j in 0..dim {
            matrix[i * dim + j] = basis[i].weighted_dot(basis[j], alpha_weight);
        }
        rhs[i] = basis[i].weighted_dot(target, alpha_weight);
    }

    let solution = gaussian_solve(&mut matrix, &mut rhs, dim)?;

    let mut w = vec![0.0; colors.len()];
    let mut used = 0.0;
    for (slot, &index) in free.iter().enumerate() {
        w[index] = solution[slot];
        used += solution[slot];
    }
    w[base] = 1.0 - used;
    Some(w)
}

/// Gaussian elimination with partial pivoting, for `dim <= 3`.
fn gaussian_solve(matrix: &mut [f64], rhs: &mut [f64], dim: usize) -> Option<Vec<f64>> {
    for col in 0..dim {
        // Partial pivoting: choose the largest magnitude pivot, ties going to
        // the lower row so the choice is deterministic (PTE-DET-002).
        let mut pivot = col;
        for row in col + 1..dim {
            if matrix[row * dim + col].abs() > matrix[pivot * dim + col].abs() {
                pivot = row;
            }
        }
        if matrix[pivot * dim + col].abs() < 1e-12 {
            return None;
        }
        if pivot != col {
            for j in 0..dim {
                matrix.swap(col * dim + j, pivot * dim + j);
            }
            rhs.swap(col, pivot);
        }
        let diagonal = matrix[col * dim + col];
        for row in col + 1..dim {
            let factor = matrix[row * dim + col] / diagonal;
            if factor == 0.0 {
                continue;
            }
            for j in col..dim {
                matrix[row * dim + j] -= factor * matrix[col * dim + j];
            }
            rhs[row] -= factor * rhs[col];
        }
    }

    let mut x = vec![0.0; dim];
    for row in (0..dim).rev() {
        let mut sum = rhs[row];
        for j in row + 1..dim {
            sum -= matrix[row * dim + j] * x[j];
        }
        x[row] = sum / matrix[row * dim + row];
        if !x[row].is_finite() {
            return None;
        }
    }
    Some(x)
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
    use crate::spaces::LinearRgb;
    use proptest::prelude::*;

    fn opaque(r: f64, g: f64, b: f64) -> PremulLinearRgba {
        PremulLinearRgba::from_straight(LinearRgb::new(r, g, b), 1.0)
    }

    /// §25.4 "AA" row: mixture recovery. A pixel synthesised at a known
    /// coverage must return that coverage.
    #[test]
    fn a_synthesised_mixture_returns_its_coverage() {
        let a = opaque(1.0, 0.0, 0.0);
        let b = opaque(0.0, 0.0, 1.0);
        for target in [0.0, 0.125, 0.25, 0.5, 0.75, 0.875, 1.0] {
            let c = a.scale(target).plus(b.scale(1.0 - target));
            let m = two_color(c, a, b, &MixturePolicy::default()).unwrap();
            assert!(
                (m.coverage - target).abs() < 1e-12,
                "expected {target}, got {}",
                m.coverage
            );
            assert!(m.residual < 1e-12);
        }
    }

    /// §25.4 "AA" row: label reversal symmetry. Swapping which side is `A`
    /// must complement the coverage exactly, or the two faces of a shared
    /// boundary would disagree about where it is.
    #[test]
    fn swapping_the_endpoints_complements_the_coverage() {
        let a = opaque(0.9, 0.1, 0.2);
        let b = opaque(0.1, 0.4, 0.8);
        let c = a.scale(0.3).plus(b.scale(0.7));

        let forward = two_color(c, a, b, &MixturePolicy::default()).unwrap();
        let reverse = two_color(c, b, a, &MixturePolicy::default()).unwrap();
        assert!(
            (forward.coverage + reverse.coverage - 1.0).abs() < 1e-12,
            "{} + {} != 1",
            forward.coverage,
            reverse.coverage
        );
        assert!((forward.residual - reverse.residual).abs() < 1e-12);
    }

    /// PTE-AA-002: an ill-conditioned pair is rejected, not amplified.
    #[test]
    fn inseparable_endpoints_are_rejected() {
        let a = opaque(0.5, 0.5, 0.5);
        let b = opaque(0.500_001, 0.5, 0.5);
        let c = opaque(0.6, 0.5, 0.5);
        assert_eq!(
            two_color(c, a, b, &MixturePolicy::default()).unwrap_err(),
            MixtureRejection::IllConditionedColors
        );
    }

    /// A pixel that is a third colour entirely is not a mixture of these two,
    /// and saying so is the point (PTE-NO-006).
    #[test]
    fn a_third_colour_is_not_reported_as_a_mixture() {
        let a = opaque(1.0, 0.0, 0.0);
        let b = opaque(0.0, 0.0, 1.0);
        let c = opaque(0.0, 1.0, 0.0);
        assert_eq!(
            two_color(c, a, b, &MixturePolicy::default()).unwrap_err(),
            MixtureRejection::NotTwoColorMixture
        );
    }

    /// The coverage is clamped, so an observation beyond an endpoint reports
    /// full coverage rather than a value outside `[0, 1]` that would become a
    /// boundary position outside the pixel (PTE-AA-005).
    #[test]
    fn coverage_is_clamped_to_the_unit_interval() {
        let a = opaque(0.6, 0.0, 0.0);
        let b = opaque(0.2, 0.0, 0.0);
        let beyond = opaque(1.0, 0.0, 0.0);
        let (coverage, _) = two_color_residual(beyond, a, b, &MixturePolicy::default()).unwrap();
        assert_eq!(coverage, 1.0);
    }

    /// Alpha participates in the fit rather than being solved separately
    /// (§10.2).
    #[test]
    fn alpha_is_part_of_the_four_dimensional_fit() {
        let a = PremulLinearRgba::from_straight(LinearRgb::new(1.0, 1.0, 1.0), 1.0);
        let b = PremulLinearRgba::TRANSPARENT;
        for target in [0.25, 0.5, 0.75] {
            let c = a.scale(target).plus(b.scale(1.0 - target));
            let m = two_color(c, a, b, &MixturePolicy::default()).unwrap();
            assert!((m.coverage - target).abs() < 1e-12);
            assert!((c.a - target).abs() < 1e-12, "the alpha channel carries it");
        }
    }

    /// §10.4: three-colour junction weights, recovered exactly when the
    /// observation really is a mixture of three.
    #[test]
    fn three_colour_barycentric_weights_are_recovered() {
        let colors = [
            opaque(1.0, 0.0, 0.0),
            opaque(0.0, 1.0, 0.0),
            opaque(0.0, 0.0, 1.0),
        ];
        let truth = [0.5, 0.3, 0.2];
        let mut c = PremulLinearRgba::TRANSPARENT;
        for (color, w) in colors.iter().zip(truth) {
            c = c.plus(color.scale(w));
        }

        let w = barycentric(c, &colors, ALPHA_CHANNEL_WEIGHT).unwrap();
        for (got, want) in w.iter().zip(truth) {
            assert!((got - want).abs() < 1e-9, "{w:?} vs {truth:?}");
        }
    }

    /// The constraint is what makes the answer usable as coverage: weights
    /// must be non-negative and sum to one even when the observation sits
    /// outside the triangle.
    #[test]
    fn barycentric_weights_stay_feasible_outside_the_simplex() {
        let colors = [
            opaque(1.0, 0.0, 0.0),
            opaque(0.0, 1.0, 0.0),
            opaque(0.0, 0.0, 1.0),
        ];
        // Beyond the red vertex.
        let c = opaque(2.0, -0.5, -0.5);
        let w = barycentric(c, &colors, ALPHA_CHANNEL_WEIGHT).unwrap();
        assert!(w.iter().all(|&v| v >= -1e-9), "{w:?}");
        assert!((w.iter().sum::<f64>() - 1.0).abs() < 1e-9, "{w:?}");
    }

    #[test]
    fn barycentric_refuses_more_than_four_colours() {
        let colors = vec![opaque(0.1, 0.1, 0.1); 5];
        assert!(barycentric(opaque(0.1, 0.1, 0.1), &colors, 1.0).is_none());
        assert!(barycentric(opaque(0.1, 0.1, 0.1), &[], 1.0).is_none());
    }

    /// Degenerate input -- three identical colours -- must produce a feasible
    /// answer rather than a singular-matrix NaN.
    #[test]
    fn degenerate_colours_do_not_produce_nan_weights() {
        let colors = [opaque(0.3, 0.3, 0.3); 3];
        let w = barycentric(opaque(0.3, 0.3, 0.3), &colors, 1.0).unwrap();
        assert!(w.iter().all(|v| v.is_finite()), "{w:?}");
        assert!((w.iter().sum::<f64>() - 1.0).abs() < 1e-9);
    }

    proptest! {
        /// The estimate must be total: any two endpoints and any observation
        /// either produce a finite coverage in range or a declared rejection.
        #[test]
        fn the_estimate_is_total_and_bounded(
            ar in 0.0f64..=1.0, ab in 0.0f64..=1.0,
            br in 0.0f64..=1.0, bb in 0.0f64..=1.0,
            cr in 0.0f64..=1.0, cb in 0.0f64..=1.0,
        ) {
            let policy = MixturePolicy::default();
            let a = opaque(ar, 0.0, ab);
            let b = opaque(br, 0.0, bb);
            let c = opaque(cr, 0.0, cb);
            // A rejection is a legitimate outcome; what must never happen is
            // an accepted estimate that is out of range or non-finite.
            if let Ok(m) = two_color(c, a, b, &policy) {
                prop_assert!((0.0..=1.0).contains(&m.coverage));
                prop_assert!(m.residual.is_finite() && m.residual >= 0.0);
            }
        }

        /// Reversal symmetry, in general rather than on one example.
        #[test]
        fn reversal_symmetry_holds_generally(
            t in 0.0f64..=1.0, ar in 0.0f64..=1.0, br in 0.0f64..=1.0,
        ) {
            prop_assume!((ar - br).abs() > 0.05);
            let a = opaque(ar, 0.2, 0.9);
            let b = opaque(br, 0.7, 0.1);
            let c = a.scale(t).plus(b.scale(1.0 - t));
            let policy = MixturePolicy::default();
            let forward = two_color(c, a, b, &policy).unwrap();
            let reverse = two_color(c, b, a, &policy).unwrap();
            prop_assert!((forward.coverage + reverse.coverage - 1.0).abs() < 1e-9);
        }
    }

    // ---- §10.2 compositing-space estimation -----------------------------

    /// A colour as a rasteriser holds it: transfer-encoded bytes.
    fn from_bytes(r: u8, g: u8, b: u8) -> PremulLinearRgba {
        let e = crate::spaces::EncodedSrgb::new(
            f64::from(r) / 255.0,
            f64::from(g) / 255.0,
            f64::from(b) / 255.0,
        );
        PremulLinearRgba::from_straight(e.to_linear(), 1.0)
    }

    /// Blend two colours the way most 2D rasterisers actually do: on the
    /// transfer-encoded values, without linearising first.
    fn blend_in_encoded_space(a: [u8; 3], b: [u8; 3], t: f64) -> PremulLinearRgba {
        let c: Vec<f64> = (0..3)
            .map(|k| (f64::from(a[k]) * t + f64::from(b[k]) * (1.0 - t)) / 255.0)
            .collect();
        let e = crate::spaces::EncodedSrgb::new(c[0], c[1], c[2]);
        PremulLinearRgba::from_straight(e.to_linear(), 1.0)
    }

    const INK: [u8; 3] = [32, 38, 52];
    const PAPER: [u8; 3] = [247, 244, 236];
    const RED: [u8; 3] = [198, 62, 54];

    /// §10.2 with PTE-AA-002: an antialiased pixel composited on encoded
    /// values is *not* a linear-light mixture, and the estimator has to notice
    /// rather than return a confidently wrong coverage.
    ///
    /// This is the defect the corpus census was measuring. `curves/star-acute-
    /// corners` traced to 47 faces because every one of its fringe pixels
    /// failed §8.6's residual gate by this margin.
    #[test]
    fn an_encoded_space_blend_is_recognised_and_its_coverage_recovered() {
        let a = from_bytes(RED[0], RED[1], RED[2]);
        let b = from_bytes(PAPER[0], PAPER[1], PAPER[2]);
        let policy = MixturePolicy::default();

        for target in [0.125, 0.25, 0.5, 0.75, 0.875] {
            let c = blend_in_encoded_space(RED, PAPER, target);
            let best = two_color_best(c, a, b, &policy).unwrap();

            assert_eq!(
                best.space,
                CompositeSpace::EncodedSrgb,
                "the encoded hypothesis explains an encoded blend"
            );
            assert!(
                (best.mixture.coverage - target).abs() < 2e-3,
                "coverage {} should recover {target}",
                best.mixture.coverage
            );
            assert!(
                best.mixture.residual < 1e-3,
                "an exact encoded blend leaves no residual, got {}",
                best.mixture.residual
            );
        }
    }

    /// The other half of the same claim, stated as the size of the error the
    /// old estimator made. §10.3 turns coverage into a position, so a coverage
    /// bias of this size is a position error of the same size in pixels --
    /// larger than the grid error §10 exists to remove.
    #[test]
    fn the_linear_hypothesis_alone_is_biased_by_a_fifth_of_a_pixel() {
        let policy = MixturePolicy::default();
        for (pair, name) in [(INK, "ink"), (RED, "red")] {
            let a = from_bytes(pair[0], pair[1], pair[2]);
            let b = from_bytes(PAPER[0], PAPER[1], PAPER[2]);
            let c = blend_in_encoded_space(pair, PAPER, 0.5);

            let (linear_coverage, _) = two_color_residual(c, a, b, &policy).unwrap();
            assert!(
                linear_coverage - 0.5 > 0.17,
                "{name}: the linear hypothesis should be biased high by more \
                 than 0.17, got {linear_coverage}"
            );

            let best = two_color_best(c, a, b, &policy).unwrap();
            assert!(
                (best.mixture.coverage - 0.5).abs() < 2e-3,
                "{name}: choosing the transfer removes the bias, got {}",
                best.mixture.coverage
            );
        }
    }

    /// A regression guard in the other direction: correctly antialiased input
    /// must not be re-explained by the encoded hypothesis. §10.2's model is
    /// the default and stays the default when it is right.
    #[test]
    fn a_linear_light_blend_still_chooses_the_linear_hypothesis() {
        let a = from_bytes(RED[0], RED[1], RED[2]);
        let b = from_bytes(PAPER[0], PAPER[1], PAPER[2]);
        let policy = MixturePolicy::default();

        for target in [0.125, 0.25, 0.5, 0.75, 0.875] {
            let c = a.scale(target).plus(b.scale(1.0 - target));
            let best = two_color_best(c, a, b, &policy).unwrap();
            assert_eq!(best.space, CompositeSpace::LinearLight);
            assert!((best.mixture.coverage - target).abs() < 1e-9);
        }
    }

    /// PTE-AA-003's symmetry requirement, at the estimator level: which side is
    /// called `A` is a labelling choice and must not change the transfer that
    /// wins or the boundary it implies.
    #[test]
    fn the_transfer_choice_is_symmetric_under_swapping_the_sides() {
        let a = from_bytes(RED[0], RED[1], RED[2]);
        let b = from_bytes(PAPER[0], PAPER[1], PAPER[2]);
        let policy = MixturePolicy::default();

        for target in [0.2, 0.5, 0.8] {
            let c = blend_in_encoded_space(RED, PAPER, target);
            let forward = two_color_best(c, a, b, &policy).unwrap();
            let reverse = two_color_best(c, b, a, &policy).unwrap();
            assert_eq!(forward.space, reverse.space);
            assert!(
                (forward.mixture.coverage + reverse.mixture.coverage - 1.0).abs() < 1e-9,
                "coverages should sum to one"
            );
        }
    }

    /// PTE-AA-002: inseparable endpoints have no coverage under either
    /// hypothesis, and the answer is `None` rather than an amplified number.
    #[test]
    fn inseparable_endpoints_have_no_estimate_in_either_space() {
        let a = from_bytes(128, 128, 128);
        let policy = MixturePolicy::default();
        assert!(two_color_best(a, a, a, &policy).is_none());
    }

    /// A fully transparent endpoint has no recoverable straight colour, so the
    /// encoded hypothesis cannot be formed. §10.2's linear model still can, and
    /// is the honest answer rather than a refusal (PTE-COLOR-009).
    #[test]
    fn a_transparent_endpoint_falls_back_to_the_linear_hypothesis() {
        let a = from_bytes(200, 40, 40);
        let b = PremulLinearRgba::TRANSPARENT;
        let policy = MixturePolicy::default();
        let c = a.scale(0.5).plus(b.scale(0.5));
        let best = two_color_best(c, a, b, &policy).unwrap();
        assert_eq!(best.space, CompositeSpace::LinearLight);
        assert!((best.mixture.coverage - 0.5).abs() < 1e-9);
    }
}
