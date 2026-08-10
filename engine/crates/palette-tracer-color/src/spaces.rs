//! Colour spaces and the conversions between them (§7.1).
//!
//! PTE-COLOR-003 requires encoded RGB, linear RGB, premultiplied linear RGBA,
//! and perceptual OKLab/OKLCH to be distinguishable, and adds that they
//! "SHOULD be distinct Rust newtypes so accidental mixing is a compile-time
//! error". They are. There is no `From<[f64; 3]>` on any of them and no
//! arithmetic operator that crosses spaces, so mixing an encoded value into a
//! linear computation does not compile.
//!
//! # ΔE_OK convention (PTE-COLOR-004)
//!
//! [`Oklab::delta_e`] returns **native OKLab units**, in which `L` spans
//! `[0, 1]`. It is not multiplied by 100. Every threshold in this workspace is
//! stated in the same units, and [`crate::spaces::DELTA_E_CONVENTION`] names
//! the convention for the report.
//!
//! # Provenance
//!
//! The sRGB transfer function and the OKLab matrices are transcribed from
//! `engine/SPEC.md` §7.1, which reproduces Björn Ottosson's published
//! definition (§42.6 reference 27). The inverse matrices are Ottosson's
//! published inverse. Implementation written independently (PTE-LIC-004);
//! `reference_vectors_match_published_values` checks the constants against
//! values derived from the forward definition rather than from any
//! implementation.

use serde::{Deserialize, Serialize};

/// The `ΔE_OK` convention this build reports, for the report and for
/// threshold documentation (PTE-COLOR-004).
pub const DELTA_E_CONVENTION: &str = "native-oklab-l-in-0-1";

/// A colour with the sRGB transfer function applied: what is in the file.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct EncodedSrgb {
    /// Red in `[0, 1]`, transfer-encoded.
    pub r: f64,
    /// Green in `[0, 1]`, transfer-encoded.
    pub g: f64,
    /// Blue in `[0, 1]`, transfer-encoded.
    pub b: f64,
}

impl EncodedSrgb {
    /// An encoded colour.
    #[must_use]
    pub const fn new(r: f64, g: f64, b: f64) -> Self {
        Self { r, g, b }
    }

    /// From 8-bit bytes.
    #[must_use]
    pub fn from_u8(r: u8, g: u8, b: u8) -> Self {
        use palette_tracer_core::checked::u8_to_unit;
        Self::new(u8_to_unit(r), u8_to_unit(g), u8_to_unit(b))
    }

    /// Apply the sRGB transfer function's inverse, giving linear light.
    ///
    /// PTE-COLOR-001: sampling, alpha reconstruction, resampling, and mixture
    /// fitting all operate here, never on encoded values.
    #[must_use]
    pub fn to_linear(self) -> LinearRgb {
        LinearRgb::new(
            srgb_to_linear(self.r),
            srgb_to_linear(self.g),
            srgb_to_linear(self.b),
        )
    }

    /// Round to 8-bit bytes.
    #[must_use]
    pub fn to_u8(self) -> [u8; 3] {
        use palette_tracer_core::checked::unit_to_u8;
        [unit_to_u8(self.r), unit_to_u8(self.g), unit_to_u8(self.b)]
    }
}

/// Linear-light sRGB primaries. The space every measurement happens in.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct LinearRgb {
    /// Red, linear.
    pub r: f64,
    /// Green, linear.
    pub g: f64,
    /// Blue, linear.
    pub b: f64,
}

impl LinearRgb {
    /// A linear colour.
    #[must_use]
    pub const fn new(r: f64, g: f64, b: f64) -> Self {
        Self { r, g, b }
    }

    /// Black.
    pub const BLACK: Self = Self {
        r: 0.0,
        g: 0.0,
        b: 0.0,
    };

    /// Re-apply the sRGB transfer function.
    ///
    /// §7.1: "The inverse transform MUST only be applied at output/preview
    /// boundaries."
    #[must_use]
    pub fn to_encoded(self) -> EncodedSrgb {
        EncodedSrgb::new(
            linear_to_srgb(self.r),
            linear_to_srgb(self.g),
            linear_to_srgb(self.b),
        )
    }

    /// Convert to OKLab (§7.1).
    #[must_use]
    pub fn to_oklab(self) -> Oklab {
        let l = 0.412_221_470_8 * self.r + 0.536_332_536_3 * self.g + 0.051_445_992_9 * self.b;
        let m = 0.211_903_498_2 * self.r + 0.680_699_545_1 * self.g + 0.107_396_956_6 * self.b;
        let s = 0.088_302_461_9 * self.r + 0.281_718_837_6 * self.g + 0.629_978_700_5 * self.b;

        let l_ = cbrt(l);
        let m_ = cbrt(m);
        let s_ = cbrt(s);

        Oklab {
            l: 0.210_454_255_3 * l_ + 0.793_617_785_0 * m_ - 0.004_072_046_8 * s_,
            a: 1.977_998_495_1 * l_ - 2.428_592_205_0 * m_ + 0.450_593_709_9 * s_,
            b: 0.025_904_037_1 * l_ + 0.782_771_766_2 * m_ - 0.808_675_766_0 * s_,
        }
    }

    /// Component-wise scaling, for mixture arithmetic.
    #[must_use]
    pub fn scale(self, k: f64) -> Self {
        Self::new(self.r * k, self.g * k, self.b * k)
    }

    /// Component-wise addition.
    ///
    /// Named `plus` rather than implementing `std::ops::Add`: PTE-COLOR-003
    /// wants mixing spaces to be a compile error, and the clearest way to keep
    /// that true is for these types to carry no arithmetic operators at all.
    #[must_use]
    pub fn plus(self, other: Self) -> Self {
        Self::new(self.r + other.r, self.g + other.g, self.b + other.b)
    }

    /// Component-wise subtraction. See [`Self::plus`] for why it is not `Sub`.
    #[must_use]
    pub fn minus(self, other: Self) -> Self {
        Self::new(self.r - other.r, self.g - other.g, self.b - other.b)
    }

    /// Clamp every channel into `[0, 1]`.
    ///
    /// Applied only where a value is about to leave the engine. An
    /// out-of-gamut intermediate is legitimate; an out-of-gamut output byte is
    /// not.
    #[must_use]
    pub fn clamped(self) -> Self {
        use palette_tracer_core::checked::clamp_unit;
        Self::new(clamp_unit(self.r), clamp_unit(self.g), clamp_unit(self.b))
    }
}

/// Perceptual OKLab.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Oklab {
    /// Lightness, roughly `[0, 1]`.
    pub l: f64,
    /// Green-red opponent axis.
    pub a: f64,
    /// Blue-yellow opponent axis.
    pub b: f64,
}

impl Oklab {
    /// An OKLab colour.
    #[must_use]
    pub const fn new(l: f64, a: f64, b: f64) -> Self {
        Self { l, a, b }
    }

    /// Origin, which is black.
    pub const ZERO: Self = Self {
        l: 0.0,
        a: 0.0,
        b: 0.0,
    };

    /// Convert back to linear sRGB (Ottosson's published inverse).
    #[must_use]
    pub fn to_linear(self) -> LinearRgb {
        let l_ = self.l + 0.396_337_777_4 * self.a + 0.215_803_757_3 * self.b;
        let m_ = self.l - 0.105_561_345_8 * self.a - 0.063_854_172_8 * self.b;
        let s_ = self.l - 0.089_484_177_5 * self.a - 1.291_485_548_0 * self.b;

        let l = l_ * l_ * l_;
        let m = m_ * m_ * m_;
        let s = s_ * s_ * s_;

        LinearRgb::new(
            4.076_741_662_1 * l - 3.307_711_591_3 * m + 0.230_969_929_2 * s,
            -1.268_438_004_6 * l + 2.609_757_401_1 * m - 0.341_319_396_5 * s,
            -0.004_196_086_3 * l - 0.703_418_614_7 * m + 1.707_614_701_0 * s,
        )
    }

    /// Convert to cylindrical OKLCH.
    ///
    /// PTE-COLOR-022: statistics are computed in Cartesian OKLab, never here.
    /// OKLCH exists for user-facing controls and for the reach score, which
    /// needs chroma and hue separately.
    #[must_use]
    pub fn to_oklch(self) -> Oklch {
        Oklch {
            l: self.l,
            c: self.a.hypot(self.b),
            h: self.b.atan2(self.a),
        }
    }

    /// Perceptual distance in native OKLab units (see the module docs).
    #[must_use]
    pub fn delta_e(self, other: Self) -> f64 {
        self.delta_e_squared(other).sqrt()
    }

    /// Squared perceptual distance, avoiding a square root.
    ///
    /// This is the quantity §8.5's merge identity is expressed in.
    #[must_use]
    pub fn delta_e_squared(self, other: Self) -> f64 {
        let dl = self.l - other.l;
        let da = self.a - other.a;
        let db = self.b - other.b;
        dl * dl + da * da + db * db
    }

    /// Component-wise scaling, for Cartesian centroid arithmetic.
    #[must_use]
    pub fn scale(self, k: f64) -> Self {
        Self::new(self.l * k, self.a * k, self.b * k)
    }

    /// Component-wise addition.
    ///
    /// Named `plus` rather than implementing `std::ops::Add`: PTE-COLOR-003
    /// wants mixing spaces to be a compile error, and the clearest way to keep
    /// that true is for these types to carry no arithmetic operators at all.
    #[must_use]
    pub fn plus(self, other: Self) -> Self {
        Self::new(self.l + other.l, self.a + other.a, self.b + other.b)
    }

    /// Whether every component is finite.
    #[must_use]
    pub fn is_finite(self) -> bool {
        self.l.is_finite() && self.a.is_finite() && self.b.is_finite()
    }
}

/// Cylindrical OKLCH, for user-facing controls and the reach score.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Oklch {
    /// Lightness.
    pub l: f64,
    /// Chroma, non-negative.
    pub c: f64,
    /// Hue in radians, from `atan2`, so in `(-π, π]`.
    pub h: f64,
}

impl Oklch {
    /// An OKLCH colour.
    #[must_use]
    pub const fn new(l: f64, c: f64, h: f64) -> Self {
        Self { l, c, h }
    }

    /// Convert back to Cartesian OKLab.
    #[must_use]
    pub fn to_oklab(self) -> Oklab {
        Oklab::new(self.l, self.c * self.h.cos(), self.c * self.h.sin())
    }
}

/// The smallest signed angle from `b` to `a`, in `(-π, π]`.
///
/// §7.4 defines it as `atan2(sin(h_x - h_k), cos(h_x - h_k))`, which is what
/// makes hue wraparound at ±π a non-event: 179° and -179° are two degrees
/// apart, not 358.
#[must_use]
pub fn hue_difference(a: f64, b: f64) -> f64 {
    let d = a - b;
    d.sin().atan2(d.cos())
}

/// sRGB transfer function inverse, per §7.1.
#[must_use]
pub fn srgb_to_linear(c: f64) -> f64 {
    if c <= 0.040_45 {
        c / 12.92
    } else {
        ((c + 0.055) / 1.055).powf(2.4)
    }
}

/// sRGB transfer function, per §7.1.
#[must_use]
pub fn linear_to_srgb(c: f64) -> f64 {
    if c <= 0.003_130_8 {
        c * 12.92
    } else {
        1.055 * c.powf(1.0 / 2.4) - 0.055
    }
}

/// Real cube root, defined for negative inputs.
///
/// `f64::cbrt` already handles negatives; the wrapper exists so the OKLab
/// transform reads like §7.1 and so a future target without `cbrt` has one
/// place to change.
#[inline]
fn cbrt(x: f64) -> f64 {
    x.cbrt()
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
    use proptest::prelude::*;

    /// §25.4 "Color" row: sRGB transfer round-trip.
    #[test]
    fn the_srgb_transfer_function_round_trips() {
        for i in 0..=255u16 {
            let c = f64::from(i) / 255.0;
            let back = linear_to_srgb(srgb_to_linear(c));
            assert!((back - c).abs() < 1e-12, "{c} round-tripped to {back}");
        }
    }

    /// The two branches must meet closely enough that no gradient the engine
    /// measures acquires a visible step.
    ///
    /// They do not meet *exactly*: the sRGB definition's constants are rounded
    /// decimals, and the published threshold 0.04045 leaves a discontinuity of
    /// about 2.3e-9 in linear light. That is roughly six orders of magnitude
    /// below one 8-bit code point (1/255 ≈ 3.9e-3), so it cannot move a
    /// palette claim, a merge decision, or a coverage estimate. The tolerance
    /// records the real size of the gap rather than pretending it is zero.
    #[test]
    fn the_transfer_function_branches_meet_at_the_join() {
        let below = srgb_to_linear(0.040_45 - 1e-12);
        let above = srgb_to_linear(0.040_45 + 1e-12);
        let gap = (below - above).abs();
        assert!(
            gap < 1e-8,
            "gap of {gap} between branches: {below} vs {above}"
        );
        assert!(
            gap < 1.0 / 255.0 / 1000.0,
            "the gap must stay far below one code point"
        );
    }

    /// §25.4 "Color" row: OKLab reference vectors. PTE-COLOR-004 requires the
    /// constants to be tested against published values.
    ///
    /// White, red, green and blue in linear sRGB have known OKLab values from
    /// Ottosson's definition. White in particular must be exactly
    /// `L = 1, a = 0, b = 0`, which is the property the matrices are
    /// normalised for -- a transcription slip in any of the nine forward
    /// coefficients moves it.
    #[test]
    fn reference_vectors_match_published_values() {
        let cases: [(LinearRgb, [f64; 3]); 4] = [
            (LinearRgb::new(1.0, 1.0, 1.0), [1.0, 0.0, 0.0]),
            (
                LinearRgb::new(1.0, 0.0, 0.0),
                [0.627_955, 0.224_863, 0.125_846],
            ),
            (
                LinearRgb::new(0.0, 1.0, 0.0),
                [0.866_440, -0.233_888, 0.179_498],
            ),
            (
                LinearRgb::new(0.0, 0.0, 1.0),
                [0.452_014, -0.032_457, -0.311_528],
            ),
        ];
        for (rgb, [l, a, b]) in cases {
            let got = rgb.to_oklab();
            assert!((got.l - l).abs() < 1e-5, "L: {got:?} vs {l}");
            assert!((got.a - a).abs() < 1e-5, "a: {got:?} vs {a}");
            assert!((got.b - b).abs() < 1e-5, "b: {got:?} vs {b}");
        }
    }

    #[test]
    fn black_maps_to_the_oklab_origin() {
        let got = LinearRgb::BLACK.to_oklab();
        assert!(got.l.abs() < 1e-12 && got.a.abs() < 1e-12 && got.b.abs() < 1e-12);
    }

    #[test]
    fn oklab_round_trips_to_linear_rgb() {
        for c in [
            LinearRgb::new(1.0, 1.0, 1.0),
            LinearRgb::new(0.2, 0.5, 0.9),
            LinearRgb::new(0.0, 0.0, 0.0),
            LinearRgb::new(0.05, 0.01, 0.7),
        ] {
            // The published forward and inverse matrices are decimal
            // approximations, not exact inverses of one another, so the round
            // trip is accurate to about 1e-7 rather than to machine epsilon.
            // That is four orders below one 8-bit code point, which is why the
            // byte-level round trip below is exact.
            let back = c.to_oklab().to_linear();
            assert!((back.r - c.r).abs() < 1e-6, "{back:?} vs {c:?}");
            assert!((back.g - c.g).abs() < 1e-6, "{back:?} vs {c:?}");
            assert!((back.b - c.b).abs() < 1e-6, "{back:?} vs {c:?}");
        }
    }

    /// §25.4 "Color" row: hue wrap. The fixture family in §25.3 calls for
    /// "hue wraparound near 0/360 degrees" explicitly.
    #[test]
    fn hue_difference_is_shortest_across_the_wrap() {
        use core::f64::consts::PI;
        let deg = |d: f64| d * PI / 180.0;

        assert!((hue_difference(deg(179.0), deg(-179.0)) - deg(-2.0)).abs() < 1e-12);
        assert!((hue_difference(deg(-179.0), deg(179.0)) - deg(2.0)).abs() < 1e-12);
        assert!((hue_difference(deg(10.0), deg(350.0)) - deg(20.0)).abs() < 1e-12);
        assert!(hue_difference(deg(0.0), deg(0.0)).abs() < 1e-12);
        // Antipodal is π in magnitude regardless of which way it is measured.
        assert!((hue_difference(deg(180.0), deg(0.0)).abs() - PI).abs() < 1e-12);
    }

    #[test]
    fn oklch_round_trips_through_oklab() {
        let lab = LinearRgb::new(0.3, 0.6, 0.2).to_oklab();
        let back = lab.to_oklch().to_oklab();
        assert!(lab.delta_e(back) < 1e-12);
    }

    /// PTE-COLOR-004: the convention is documented, not assumed. `L` spans
    /// `[0, 1]`, so black to white is a ΔE of 1, not 100.
    #[test]
    fn delta_e_is_reported_in_native_units() {
        let black = LinearRgb::BLACK.to_oklab();
        let white = LinearRgb::new(1.0, 1.0, 1.0).to_oklab();
        assert!((black.delta_e(white) - 1.0).abs() < 1e-6);
        assert_eq!(DELTA_E_CONVENTION, "native-oklab-l-in-0-1");
    }

    proptest! {
        #[test]
        fn every_encoded_colour_survives_the_full_round_trip(
            r in 0u8..=255, g in 0u8..=255, b in 0u8..=255
        ) {
            let encoded = EncodedSrgb::from_u8(r, g, b);
            let back = encoded.to_linear().to_oklab().to_linear().to_encoded().to_u8();
            prop_assert_eq!(back, [r, g, b]);
        }

        #[test]
        fn oklab_is_always_finite_for_in_gamut_input(
            r in 0.0f64..=1.0, g in 0.0f64..=1.0, b in 0.0f64..=1.0
        ) {
            prop_assert!(LinearRgb::new(r, g, b).to_oklab().is_finite());
        }

        /// Hue difference is antisymmetric and never leaves (-π, π].
        #[test]
        fn hue_difference_is_bounded_and_antisymmetric(
            a in -10.0f64..10.0, b in -10.0f64..10.0
        ) {
            use core::f64::consts::PI;
            let d = hue_difference(a, b);
            prop_assert!(d > -PI - 1e-9 && d <= PI + 1e-9);
            prop_assert!((d + hue_difference(b, a)).abs() < 1e-9);
        }
    }
}
