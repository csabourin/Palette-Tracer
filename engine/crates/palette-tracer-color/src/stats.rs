//! Region colour statistics (§7.7).
//!
//! §7.7 asks regions to maintain the sufficient statistics
//! `(n, ΣL, Σa, Σb, ΣL², Σa², Σb²)`, "permit[ting] constant-time merged means
//! and within-region sum-of-squares".
//!
//! That constant-time merge is what makes §8.5's region merging tractable:
//! the increase in constant-colour approximation error when two regions join
//! has the closed form
//!
//! ```text
//! ΔV_ij = (n_i n_j / (n_i + n_j)) ‖μ_i − μ_j‖²
//! ```
//!
//! which needs only the counts and the means. Without it, every candidate
//! merge would rescan pixels, and PTE-SEG-010 forbids that explicitly.

use crate::spaces::Oklab;
use palette_tracer_core::determinism::QuantKey;

/// Sufficient statistics for one region's colour.
///
/// Accumulated in `f64`. §7.7 asks for "overflow-safe integer or deterministic
/// floating reductions appropriate to bit depth": the reduction here is
/// deterministic because it is a single-threaded left fold in row-major pixel
/// order, and the same order is used on every target. Once §19.6 parallelism
/// exists, this becomes a fixed merge tree rather than a free-form reduction
/// (PTE-PERF-010).
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct ColorStats {
    /// Pixel count.
    pub n: u64,
    /// Sum of `L`, `a`, `b`.
    pub sum: [f64; 3],
    /// Sum of squares of `L`, `a`, `b`.
    pub sum_sq: [f64; 3],
}

impl ColorStats {
    /// Empty statistics.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            n: 0,
            sum: [0.0; 3],
            sum_sq: [0.0; 3],
        }
    }

    /// Statistics for a single sample.
    #[must_use]
    pub fn of(color: Oklab) -> Self {
        let mut s = Self::new();
        s.push(color);
        s
    }

    /// Accumulate one sample.
    pub fn push(&mut self, color: Oklab) {
        let v = [color.l, color.a, color.b];
        self.n += 1;
        for ((sum, sum_sq), value) in self.sum.iter_mut().zip(self.sum_sq.iter_mut()).zip(v) {
            *sum += value;
            *sum_sq += value * value;
        }
    }

    /// Whether any sample has been accumulated.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.n == 0
    }

    /// Mean colour, or [`Oklab::ZERO`] when empty.
    ///
    /// Cartesian, never cylindrical (PTE-COLOR-022, PTE-NO-005).
    #[must_use]
    pub fn mean(&self) -> Oklab {
        if self.n == 0 {
            return Oklab::ZERO;
        }
        let n = self.n as f64;
        Oklab::new(self.sum[0] / n, self.sum[1] / n, self.sum[2] / n)
    }

    /// Total within-region sum of squared deviations from the mean.
    ///
    /// Computed as `Σx² − (Σx)²/n`, which is exact in real arithmetic. In
    /// floating point it can go slightly negative for a region whose colour is
    /// almost constant, so the result is clamped at zero -- a negative
    /// variance would propagate a NaN through the merge cost's square root.
    #[must_use]
    pub fn variance_sum(&self) -> f64 {
        if self.n == 0 {
            return 0.0;
        }
        let n = self.n as f64;
        let total: f64 = self
            .sum
            .iter()
            .zip(self.sum_sq.iter())
            .map(|(sum, sum_sq)| sum_sq - sum * sum / n)
            .sum();
        total.max(0.0)
    }

    /// Combine two regions' statistics in constant time (PTE-SEG-010).
    #[must_use]
    pub fn merged(&self, other: &Self) -> Self {
        let mut out = Self {
            n: self.n + other.n,
            sum: [0.0; 3],
            sum_sq: [0.0; 3],
        };
        for i in 0..3 {
            out.sum[i] = self.sum[i] + other.sum[i];
            out.sum_sq[i] = self.sum_sq[i] + other.sum_sq[i];
        }
        out
    }

    /// The increase in constant-colour approximation error from merging
    /// (§8.5).
    ///
    /// Uses the closed form rather than recomputing variances, which is what
    /// makes a merge candidate `O(1)` to price.
    #[must_use]
    pub fn merge_cost(&self, other: &Self) -> f64 {
        if self.n == 0 || other.n == 0 {
            return 0.0;
        }
        let ni = self.n as f64;
        let nj = other.n as f64;
        (ni * nj / (ni + nj)) * self.mean().delta_e_squared(other.mean())
    }

    /// A quantised key for the merge cost, for the deterministic heap
    /// (PTE-SEG-014).
    #[must_use]
    pub fn merge_cost_key(&self, other: &Self) -> QuantKey {
        QuantKey::cost_or_worst(self.merge_cost(other))
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::float_cmp, clippy::field_reassign_with_default)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    fn stats_of(colors: &[Oklab]) -> ColorStats {
        let mut s = ColorStats::new();
        for c in colors {
            s.push(*c);
        }
        s
    }

    fn direct_variance_sum(colors: &[Oklab]) -> f64 {
        if colors.is_empty() {
            return 0.0;
        }
        let n = colors.len() as f64;
        let mean = Oklab::new(
            colors.iter().map(|c| c.l).sum::<f64>() / n,
            colors.iter().map(|c| c.a).sum::<f64>() / n,
            colors.iter().map(|c| c.b).sum::<f64>() / n,
        );
        colors.iter().map(|c| c.delta_e_squared(mean)).sum()
    }

    /// §25.4 "Region stats" row: the merged mean and variance must equal the
    /// direct calculation.
    #[test]
    fn merged_statistics_equal_the_direct_calculation() {
        let a = [
            Oklab::new(0.1, 0.02, -0.03),
            Oklab::new(0.15, 0.01, -0.02),
        ];
        let b = [
            Oklab::new(0.8, -0.05, 0.06),
            Oklab::new(0.82, -0.04, 0.07),
            Oklab::new(0.79, -0.06, 0.05),
        ];
        let all: Vec<Oklab> = a.iter().chain(b.iter()).copied().collect();

        let merged = stats_of(&a).merged(&stats_of(&b));
        let direct = stats_of(&all);

        assert_eq!(merged.n, direct.n);
        assert!(merged.mean().delta_e(direct.mean()) < 1e-12);
        assert!(
            (merged.variance_sum() - direct_variance_sum(&all)).abs() < 1e-9,
            "{} vs {}",
            merged.variance_sum(),
            direct_variance_sum(&all)
        );
    }

    /// §8.5's identity, checked against the definition it is derived from:
    /// `ΔV = Var(A ∪ B) − Var(A) − Var(B)`.
    #[test]
    fn the_merge_cost_identity_matches_the_variance_definition() {
        let a = [Oklab::new(0.2, 0.0, 0.0), Oklab::new(0.3, 0.01, 0.0)];
        let b = [Oklab::new(0.7, 0.0, 0.02), Oklab::new(0.75, 0.0, 0.03)];

        let sa = stats_of(&a);
        let sb = stats_of(&b);
        let union = sa.merged(&sb);

        let by_definition = union.variance_sum() - sa.variance_sum() - sb.variance_sum();
        let by_identity = sa.merge_cost(&sb);

        assert!(
            (by_definition - by_identity).abs() < 1e-9,
            "definition {by_definition} vs identity {by_identity}"
        );
    }

    #[test]
    fn merging_with_an_empty_region_costs_nothing_and_changes_nothing() {
        let a = stats_of(&[Oklab::new(0.5, 0.0, 0.0)]);
        let empty = ColorStats::new();
        assert_eq!(a.merge_cost(&empty), 0.0);
        assert_eq!(a.merged(&empty), a);
    }

    /// A near-constant region must not produce a negative variance, which
    /// would become a NaN in the merge cost.
    #[test]
    fn a_constant_region_has_non_negative_variance() {
        let same = Oklab::new(0.123_456_789, -0.05, 0.06);
        let s = stats_of(&[same; 1000]);
        assert!(s.variance_sum() >= 0.0);
        assert!(s.variance_sum() < 1e-12);
    }

    #[test]
    fn merging_is_commutative_and_associative() {
        let a = stats_of(&[Oklab::new(0.1, 0.0, 0.0)]);
        let b = stats_of(&[Oklab::new(0.5, 0.1, 0.0)]);
        let c = stats_of(&[Oklab::new(0.9, 0.0, 0.1)]);
        assert_eq!(a.merged(&b), b.merged(&a));
        assert_eq!(a.merged(&b).merged(&c), a.merged(&b.merged(&c)));
    }

    proptest! {
        /// The identity must hold for arbitrary region contents, not just the
        /// hand-picked ones.
        #[test]
        fn the_merge_identity_holds_for_random_regions(
            xs in prop::collection::vec(-1.0f64..1.0, 1..12),
            ys in prop::collection::vec(-1.0f64..1.0, 1..12),
        ) {
            let a: Vec<Oklab> = xs.iter().map(|&v| Oklab::new(v, v * 0.1, v * 0.2)).collect();
            let b: Vec<Oklab> = ys.iter().map(|&v| Oklab::new(v, -v * 0.1, v * 0.3)).collect();
            let sa = stats_of(&a);
            let sb = stats_of(&b);
            let by_definition =
                sa.merged(&sb).variance_sum() - sa.variance_sum() - sb.variance_sum();
            prop_assert!((by_definition - sa.merge_cost(&sb)).abs() < 1e-6);
        }

        #[test]
        fn the_merge_cost_is_never_negative(
            xs in prop::collection::vec(-1.0f64..1.0, 1..8),
            ys in prop::collection::vec(-1.0f64..1.0, 1..8),
        ) {
            let sa = stats_of(&xs.iter().map(|&v| Oklab::new(v, 0.0, 0.0)).collect::<Vec<_>>());
            let sb = stats_of(&ys.iter().map(|&v| Oklab::new(v, 0.0, 0.0)).collect::<Vec<_>>());
            prop_assert!(sa.merge_cost(&sb) >= 0.0);
        }
    }
}
