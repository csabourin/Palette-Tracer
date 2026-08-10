//! Audited numeric conversions for image-boundary arithmetic.
//!
//! PTE-SEC-005 requires dimension and byte-count arithmetic to use checked
//! operations *before* allocation. Rather than scatter `checked_mul` and
//! `try_into` across the pipeline, every conversion that can lose information
//! at an image boundary goes through one of these functions, each of which
//! names the failure it prevents.

use crate::error::DecodeError;

/// Widen a `u32` pixel coordinate or dimension to `usize`.
///
/// # Panics
///
/// Never on any target the engine supports. `usize` is at least 32 bits on
/// every target in the specification's matrix, including
/// `wasm32-unknown-unknown`; the assertion documents the assumption and would
/// fire at compile time on a hypothetical 16-bit target.
#[inline]
#[must_use]
pub const fn u32_to_usize(value: u32) -> usize {
    const { assert!(usize::BITS >= 32, "PTE requires a 32-bit or wider usize") };
    value as usize
}

/// Compute `width * height` as a pixel count, rejecting overflow.
///
/// This is the first thing [`crate::ImageView`] does, because every later
/// allocation is derived from it (PTE-SEC-005).
///
/// # Errors
///
/// [`DecodeError::EmptyImage`] if either dimension is zero, or
/// [`DecodeError::DimensionOverflow`] if the product does not fit `usize`.
pub fn pixel_count(width: u32, height: u32) -> Result<usize, DecodeError> {
    if width == 0 || height == 0 {
        return Err(DecodeError::EmptyImage);
    }
    u32_to_usize(width)
        .checked_mul(u32_to_usize(height))
        .ok_or(DecodeError::DimensionOverflow { width, height })
}

/// Multiply two byte counts, rejecting overflow.
///
/// # Errors
///
/// [`DecodeError::DimensionOverflow`] with the supplied dimensions for
/// context if the product does not fit `usize`.
pub fn mul_bytes(a: usize, b: usize, width: u32, height: u32) -> Result<usize, DecodeError> {
    a.checked_mul(b)
        .ok_or(DecodeError::DimensionOverflow { width, height })
}

/// Convert a `usize` count to `u64` for reporting and limit comparison.
#[inline]
#[must_use]
pub const fn usize_to_u64(value: usize) -> u64 {
    const { assert!(usize::BITS <= 64, "PTE assumes usize fits u64") };
    value as u64
}

/// Clamp a `f64` to the closed unit interval, mapping NaN to `0.0`.
///
/// Coverage fractions, confidences, and normalised scores all live in `[0, 1]`
/// and must never escape it -- a coverage of `1.0000000001` becomes a negative
/// signed distance in the §10.3 inversion. NaN maps to zero rather than
/// propagating, because every caller of this function has already decided that
/// an unusable value means "no evidence".
#[inline]
#[must_use]
pub fn clamp_unit(value: f64) -> f64 {
    if value.is_nan() {
        0.0
    } else {
        value.clamp(0.0, 1.0)
    }
}

/// Convert an 8-bit sample to `f64` in `[0, 1]`.
#[inline]
#[must_use]
pub fn u8_to_unit(value: u8) -> f64 {
    f64::from(value) / 255.0
}

/// Convert a 16-bit sample to `f64` in `[0, 1]`.
#[inline]
#[must_use]
pub fn u16_to_unit(value: u16) -> f64 {
    f64::from(value) / 65535.0
}

/// Round a unit-interval value back to an 8-bit sample.
///
/// Uses round-half-away-from-zero so the mapping is symmetric and independent
/// of the platform's current rounding mode (PTE-DET-002).
#[inline]
#[must_use]
pub fn unit_to_u8(value: f64) -> u8 {
    (clamp_unit(value) * 255.0).round() as u8
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

    #[test]
    fn zero_dimension_is_rejected_before_multiplication() {
        assert_eq!(pixel_count(0, 10), Err(DecodeError::EmptyImage));
        assert_eq!(pixel_count(10, 0), Err(DecodeError::EmptyImage));
    }

    /// The adversarial case PTE-SEC-005 exists for: two dimensions that are
    /// individually legal and whose product is not.
    #[test]
    fn maximal_dimensions_overflow_rather_than_wrap() {
        let r = pixel_count(u32::MAX, u32::MAX);
        if usize::BITS < 64 {
            assert!(matches!(r, Err(DecodeError::DimensionOverflow { .. })));
        } else {
            // On 64-bit the product fits; the limit check, not the arithmetic,
            // is what rejects an image this large (PTE-SEC-006).
            assert_eq!(r.unwrap(), 18_446_744_065_119_617_025);
        }
    }

    #[test]
    fn unit_round_trip_is_exact_for_every_byte() {
        for v in 0u8..=255 {
            assert_eq!(unit_to_u8(u8_to_unit(v)), v);
        }
    }

    #[test]
    fn clamp_unit_maps_nan_to_zero_and_never_escapes() {
        assert_eq!(clamp_unit(f64::NAN), 0.0);
        assert_eq!(clamp_unit(-1.0), 0.0);
        assert_eq!(clamp_unit(2.0), 1.0);
        assert_eq!(clamp_unit(f64::INFINITY), 1.0);
        assert_eq!(clamp_unit(f64::NEG_INFINITY), 0.0);
    }

    proptest! {
        #[test]
        fn clamp_unit_output_is_always_in_range(v in proptest::num::f64::ANY) {
            let c = clamp_unit(v);
            prop_assert!((0.0..=1.0).contains(&c));
            prop_assert!(c.is_finite());
        }

        #[test]
        fn pixel_count_never_wraps(w in 1u32..=u32::MAX, h in 1u32..=u32::MAX) {
            if let Ok(n) = pixel_count(w, h) {
                prop_assert_eq!(n, u32_to_usize(w) * u32_to_usize(h));
            }
        }
    }
}
