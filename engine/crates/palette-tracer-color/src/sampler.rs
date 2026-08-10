//! Turning a validated [`ImageView`] into linear-light premultiplied samples.
//!
//! PTE-COLOR-001 requires every measurement to happen in linear light, and
//! §7.3 requires alpha to be handled explicitly rather than inherited from
//! whatever the decoder happened to produce. This is the one place the
//! declared [`ColorEncoding`] and [`AlphaMode`] are consumed; after it,
//! nothing in the engine needs to remember what the input format was.

use crate::alpha::PremulLinearRgba;
use crate::spaces::{EncodedSrgb, LinearRgb};
use palette_tracer_core::image::{AlphaMode, ColorEncoding, ImageView};

/// Read pixel `(x, y)` as a premultiplied linear-light sample.
///
/// Outside the image the result is [`PremulLinearRgba::TRANSPARENT`], which
/// makes halo reads at the domain border total without a branch at every call
/// site.
#[must_use]
pub fn sample_premultiplied(view: &ImageView<'_>, x: u32, y: u32) -> PremulLinearRgba {
    if !view.contains(x, y) {
        return PremulLinearRgba::TRANSPARENT;
    }
    let [r, g, b, a] = view.sample(x, y);

    // `AlphaMode` and `ColorEncoding` are `#[non_exhaustive]`, so each match
    // needs a wildcard. The wildcard treats an unknown mode as the most
    // conservative interpretation rather than guessing, and a variant added
    // later without updating this file produces visibly wrong alpha in tests
    // rather than silently plausible output.
    let alpha = match view.alpha_mode() {
        AlphaMode::Opaque => 1.0,
        AlphaMode::Straight | AlphaMode::Premultiplied => a,
        _ => a,
    };

    // Linearisation must happen on *straight* colour: the transfer function is
    // nonlinear, so linearising a premultiplied value and linearising a
    // straight one then multiplying give different answers, and only the
    // latter is what the encoder meant.
    let straight_encoded = match view.alpha_mode() {
        AlphaMode::Premultiplied if alpha > 0.0 => {
            EncodedSrgb::new(r / alpha, g / alpha, b / alpha)
        }
        AlphaMode::Premultiplied => return PremulLinearRgba::TRANSPARENT,
        AlphaMode::Straight | AlphaMode::Opaque => EncodedSrgb::new(r, g, b),
        _ => EncodedSrgb::new(r, g, b),
    };

    let linear = match view.color_encoding() {
        ColorEncoding::Srgb => straight_encoded.to_linear(),
        ColorEncoding::LinearSrgb => {
            LinearRgb::new(straight_encoded.r, straight_encoded.g, straight_encoded.b)
        }
        _ => straight_encoded.to_linear(),
    };

    PremulLinearRgba::from_straight(linear, alpha)
}

/// Pack the raw stored bytes of pixel `(x, y)` into a cache key.
///
/// Exactness matters: the assignment cache (PTE-COLOR-017) may only reuse a
/// result for a pixel that is *identical*, so the key is the stored sample
/// quantised to 8 bits per channel and the cache is consulted only for formats
/// whose samples that represents exactly.
#[must_use]
pub fn cache_key(view: &ImageView<'_>, x: u32, y: u32) -> Option<u32> {
    use palette_tracer_core::image::PixelFormat;
    if !matches!(
        view.format(),
        PixelFormat::Gray8 | PixelFormat::GrayAlpha8 | PixelFormat::Rgb8 | PixelFormat::Rgba8
    ) {
        return None;
    }
    let [r, g, b, a] = view.sample(x, y);
    use palette_tracer_core::checked::unit_to_u8;
    Some(u32::from_be_bytes([
        unit_to_u8(r),
        unit_to_u8(g),
        unit_to_u8(b),
        unit_to_u8(a),
    ]))
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::float_cmp, clippy::field_reassign_with_default)]
mod tests {
    use super::*;
    use palette_tracer_core::image::{Orientation, PixelFormat};

    fn view<'a>(
        data: &'a [u8],
        format: PixelFormat,
        encoding: ColorEncoding,
        alpha: AlphaMode,
    ) -> ImageView<'a> {
        ImageView::new(1, 1, 0, format, data, encoding, alpha, Orientation::Identity).unwrap()
    }

    #[test]
    fn straight_srgb_is_linearised_then_premultiplied() {
        let data = [255u8, 0, 0, 128];
        let v = view(&data, PixelFormat::Rgba8, ColorEncoding::Srgb, AlphaMode::Straight);
        let p = sample_premultiplied(&v, 0, 0);
        let alpha = 128.0 / 255.0;
        assert!((p.a - alpha).abs() < 1e-12);
        // Red is 1.0 in linear light too, so the premultiplied red is alpha.
        assert!((p.r - alpha).abs() < 1e-12);
        assert!(p.g.abs() < 1e-12);
    }

    /// Linearising a premultiplied value directly would be wrong, because the
    /// transfer function is nonlinear. Both alpha modes must agree on the same
    /// underlying colour.
    #[test]
    fn premultiplied_input_agrees_with_straight_input() {
        let alpha = 128u8;
        let a = f64::from(alpha) / 255.0;

        // 50% grey, straight.
        let straight = [128u8, 128, 128, alpha];
        let sv = view(&straight, PixelFormat::Rgba8, ColorEncoding::Srgb, AlphaMode::Straight);
        let expected = sample_premultiplied(&sv, 0, 0);

        // The same colour, stored premultiplied in the encoded domain.
        let encoded_premul = palette_tracer_core::checked::unit_to_u8(128.0 / 255.0 * a);
        let premul = [encoded_premul, encoded_premul, encoded_premul, alpha];
        let pv = view(&premul, PixelFormat::Rgba8, ColorEncoding::Srgb, AlphaMode::Premultiplied);
        let got = sample_premultiplied(&pv, 0, 0);

        // Within one 8-bit quantisation step of the stored premultiplied value.
        assert!((got.r - expected.r).abs() < 0.01, "{got:?} vs {expected:?}");
        assert!((got.a - expected.a).abs() < 1e-12);
    }

    #[test]
    fn a_linear_encoded_image_is_not_linearised_twice() {
        let data = [128u8, 128, 128, 255];
        let v = view(&data, PixelFormat::Rgba8, ColorEncoding::LinearSrgb, AlphaMode::Straight);
        let p = sample_premultiplied(&v, 0, 0);
        assert!((p.r - 128.0 / 255.0).abs() < 1e-12);
    }

    #[test]
    fn opaque_mode_ignores_a_stored_alpha_channel() {
        let data = [10u8, 20, 30, 0];
        let v = view(&data, PixelFormat::Rgba8, ColorEncoding::Srgb, AlphaMode::Opaque);
        let p = sample_premultiplied(&v, 0, 0);
        assert!((p.a - 1.0).abs() < 1e-12);
        assert!(p.r > 0.0);
    }

    #[test]
    fn outside_the_domain_is_transparent() {
        let data = [255u8; 4];
        let v = view(&data, PixelFormat::Rgba8, ColorEncoding::Srgb, AlphaMode::Straight);
        assert_eq!(sample_premultiplied(&v, 5, 5), PremulLinearRgba::TRANSPARENT);
    }

    /// The cache key must be exact, so it is offered only for formats where
    /// eight bits per channel is the whole sample.
    #[test]
    fn the_cache_key_is_offered_only_for_eight_bit_formats() {
        let data8 = [1u8, 2, 3, 4];
        let v8 = view(&data8, PixelFormat::Rgba8, ColorEncoding::Srgb, AlphaMode::Straight);
        assert_eq!(cache_key(&v8, 0, 0), Some(0x0102_0304));

        let data16 = [0u8; 8];
        let v16 = view(&data16, PixelFormat::Rgba16, ColorEncoding::Srgb, AlphaMode::Straight);
        assert_eq!(cache_key(&v16, 0, 0), None);
    }
}
