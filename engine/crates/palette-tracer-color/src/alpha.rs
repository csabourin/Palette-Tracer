//! Alpha handling (§7.3).
//!
//! Two requirements shape this module, and they pull in opposite directions.
//!
//! **PTE-COLOR-008**: "Fully transparent pixels MUST have no color influence.
//! Changing RGB values where alpha is zero MUST leave semantic output
//! unchanged." A PNG encoder is free to store whatever it likes under a
//! transparent pixel, and many store garbage.
//!
//! **PTE-COLOR-009**: "Unpremultiplication MUST use a documented epsilon and
//! MUST NOT amplify arbitrary transparent RGB into palette evidence."
//! Dividing a stored colour by an alpha of `1/255` multiplies it by 255, which
//! turns a rounding artefact into a saturated colour.
//!
//! Both are satisfied the same way: below the epsilon, the colour is not
//! recovered at all -- it is reported as having no evidence, and the caller
//! (assignment, mixture fitting, statistics) skips it.

use crate::spaces::LinearRgb;
use palette_tracer_core::config::AlphaPolicy;

/// A pixel in premultiplied linear light: the form every mixture computation
/// uses (§7.3, §10.2).
///
/// Premultiplied because source-over compositing is linear in this form, which
/// is what makes the §10.2 coverage estimate a least-squares problem with a
/// closed-form answer.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PremulLinearRgba {
    /// Red, already multiplied by alpha.
    pub r: f64,
    /// Green, already multiplied by alpha.
    pub g: f64,
    /// Blue, already multiplied by alpha.
    pub b: f64,
    /// Alpha in `[0, 1]`.
    pub a: f64,
}

impl PremulLinearRgba {
    /// A premultiplied pixel.
    #[must_use]
    pub const fn new(r: f64, g: f64, b: f64, a: f64) -> Self {
        Self { r, g, b, a }
    }

    /// Fully transparent, with no colour.
    pub const TRANSPARENT: Self = Self {
        r: 0.0,
        g: 0.0,
        b: 0.0,
        a: 0.0,
    };

    /// From straight linear colour and alpha.
    #[must_use]
    pub fn from_straight(color: LinearRgb, alpha: f64) -> Self {
        Self::new(
            color.r * alpha,
            color.g * alpha,
            color.b * alpha,
            alpha,
        )
    }

    /// Recover straight colour, or `None` when alpha is below `epsilon`.
    ///
    /// `None` is the honest answer, not a fallback: below the epsilon the
    /// stored colour carries less than one code point of information, and
    /// producing a number would be inventing evidence (PTE-COLOR-009).
    #[must_use]
    pub fn to_straight(self, epsilon: f64) -> Option<LinearRgb> {
        if self.a <= epsilon {
            None
        } else {
            Some(LinearRgb::new(
                self.r / self.a,
                self.g / self.a,
                self.b / self.a,
                ))
        }
    }

    /// Component-wise difference, for mixture arithmetic. See [`Self::plus`].
    #[must_use]
    pub fn minus(self, other: Self) -> Self {
        Self::new(
            self.r - other.r,
            self.g - other.g,
            self.b - other.b,
            self.a - other.a,
        )
    }

    /// Component-wise sum. Named `plus` for the reason given on `LinearRgb`.
    #[must_use]
    pub fn plus(self, other: Self) -> Self {
        Self::new(
            self.r + other.r,
            self.g + other.g,
            self.b + other.b,
            self.a + other.a,
        )
    }

    /// Component-wise scaling.
    #[must_use]
    pub fn scale(self, k: f64) -> Self {
        Self::new(self.r * k, self.g * k, self.b * k, self.a * k)
    }

    /// Dot product over all four channels, with the alpha channel weighted.
    ///
    /// §10.2: "When alpha differs across the two regions, use a
    /// four-dimensional premultiplied RGBA observation with channel weights;
    /// do not fit straight RGB and alpha independently."
    #[must_use]
    pub fn weighted_dot(self, other: Self, alpha_weight: f64) -> f64 {
        self.r * other.r
            + self.g * other.g
            + self.b * other.b
            + alpha_weight * self.a * other.a
    }

    /// Squared weighted norm.
    #[must_use]
    pub fn weighted_norm_squared(self, alpha_weight: f64) -> f64 {
        self.weighted_dot(self, alpha_weight)
    }

    /// Source-over composite of `self` over `backdrop` (§7.3).
    #[must_use]
    pub fn over(self, backdrop: Self) -> Self {
        let k = 1.0 - self.a;
        Self::new(
            self.r + backdrop.r * k,
            self.g + backdrop.g * k,
            self.b + backdrop.b * k,
            self.a + backdrop.a * k,
        )
    }

    /// Whether this pixel carries usable colour evidence under `policy`.
    ///
    /// The single predicate that enforces PTE-COLOR-008: a pixel that fails it
    /// never reaches a histogram, a centroid, or a palette claim, so whatever
    /// colour is stored underneath cannot influence anything.
    #[must_use]
    pub fn has_color_evidence(self, policy: &AlphaPolicy) -> bool {
        self.a > policy.unpremultiply_epsilon.get()
    }
}

/// The default alpha-channel weight in four-dimensional mixture fitting.
///
/// Alpha and each colour channel are both in `[0, 1]`, so equal weight is the
/// neutral choice and is what this build uses. It is named rather than
/// inlined because §10.2 calls for "channel weights" and a future calibration
/// may change it -- at which point it becomes a reported setting rather than a
/// constant.
pub const ALPHA_CHANNEL_WEIGHT: f64 = 1.0;

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::float_cmp, clippy::field_reassign_with_default)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    fn policy() -> AlphaPolicy {
        AlphaPolicy::default()
    }

    /// PTE-COLOR-008, stated as the property §25.4 calls "alpha-zero
    /// invariance": what is stored under a transparent pixel cannot matter.
    #[test]
    fn a_transparent_pixel_carries_no_colour_evidence_whatever_is_stored() {
        for color in [
            LinearRgb::new(1.0, 0.0, 0.0),
            LinearRgb::new(0.0, 1.0, 1.0),
            LinearRgb::new(0.37, 0.91, 0.02),
        ] {
            let p = PremulLinearRgba::from_straight(color, 0.0);
            assert!(!p.has_color_evidence(&policy()));
            assert_eq!(p.to_straight(policy().unpremultiply_epsilon.get()), None);
            // Premultiplication itself already erased it.
            assert_eq!(p, PremulLinearRgba::TRANSPARENT);
        }
    }

    /// PTE-COLOR-009: the epsilon exists so a near-zero divisor cannot turn a
    /// rounding artefact into a saturated colour.
    #[test]
    fn a_near_transparent_pixel_is_not_amplified() {
        let epsilon = policy().unpremultiply_epsilon.get();
        // One 8-bit step of alpha is 1/255, which is above the epsilon of
        // 1/510; half a step is below it.
        let just_above = PremulLinearRgba::new(0.004, 0.0, 0.0, 1.0 / 255.0);
        let just_below = PremulLinearRgba::new(0.004, 0.0, 0.0, 1.0 / 1020.0);

        assert!(just_above.has_color_evidence(&policy()));
        assert!(!just_below.has_color_evidence(&policy()));
        assert_eq!(just_below.to_straight(epsilon), None);

        // And when it is recovered, the amplification is bounded by 1/epsilon
        // rather than unbounded.
        let recovered = just_above.to_straight(epsilon).unwrap();
        assert!(recovered.r <= 0.004 * 255.0 + 1e-9);
    }

    #[test]
    fn premultiplication_round_trips_above_the_epsilon() {
        let epsilon = policy().unpremultiply_epsilon.get();
        let color = LinearRgb::new(0.25, 0.5, 0.75);
        for alpha in [1.0, 0.5, 0.1, 0.01] {
            let p = PremulLinearRgba::from_straight(color, alpha);
            let back = p.to_straight(epsilon).unwrap();
            assert!((back.r - color.r).abs() < 1e-12, "alpha {alpha}");
            assert!((back.g - color.g).abs() < 1e-12, "alpha {alpha}");
            assert!((back.b - color.b).abs() < 1e-12, "alpha {alpha}");
        }
    }

    /// §7.3's compositing definition, checked against its own algebra.
    #[test]
    fn source_over_matches_the_specified_formula() {
        let src = PremulLinearRgba::from_straight(LinearRgb::new(1.0, 0.0, 0.0), 0.5);
        let dst = PremulLinearRgba::from_straight(LinearRgb::new(0.0, 0.0, 1.0), 1.0);
        let out = src.over(dst);
        assert!((out.a - 1.0).abs() < 1e-12);
        assert!((out.r - 0.5).abs() < 1e-12);
        assert!((out.b - 0.5).abs() < 1e-12);
    }

    #[test]
    fn compositing_over_an_opaque_backdrop_is_opaque() {
        let opaque = PremulLinearRgba::from_straight(LinearRgb::new(0.1, 0.2, 0.3), 1.0);
        let over = PremulLinearRgba::TRANSPARENT.over(opaque);
        assert_eq!(over, opaque);
    }

    proptest! {
        /// The invariance property in its general form: randomising the colour
        /// stored beneath a transparent pixel changes nothing observable.
        #[test]
        fn randomised_transparent_colour_is_always_erased(
            r in 0.0f64..=1.0, g in 0.0f64..=1.0, b in 0.0f64..=1.0
        ) {
            let p = PremulLinearRgba::from_straight(LinearRgb::new(r, g, b), 0.0);
            prop_assert_eq!(p, PremulLinearRgba::TRANSPARENT);
            prop_assert!(!p.has_color_evidence(&policy()));
        }

        #[test]
        fn unpremultiplication_never_produces_a_non_finite_value(
            r in 0.0f64..=1.0, a in 0.0f64..=1.0
        ) {
            let epsilon = policy().unpremultiply_epsilon.get();
            let p = PremulLinearRgba::new(r * a, 0.0, 0.0, a);
            if let Some(s) = p.to_straight(epsilon) {
                prop_assert!(s.r.is_finite() && s.g.is_finite() && s.b.is_finite());
            }
        }
    }
}
