//! Validated pixel input (§6.1).
//!
//! # Deviation from the specification's sketch
//!
//! §6.1 shows `ImageView` with public fields. PTE-API-001 requires that
//! dimensions, stride, multiplication overflow, format alignment, and byte
//! length are all validated *before algorithms read any pixel*. Public fields
//! would let a caller construct an unvalidated view and skip that, so the
//! fields are private and [`ImageView::new`] is the only constructor. The
//! accessors are named exactly as the specification names the fields.
//!
//! # Coordinates (PTE-API-002)
//!
//! Pixel `(x, y)` has its centre at `(x + 0.5, y + 0.5)`. The image domain is
//! `[0, width] x [0, height]`, and SVG user coordinates start out identical to
//! it. A boundary that lies exactly on the interface between two pixel columns
//! has an integer `x`.

use crate::checked::{self, mul_bytes, pixel_count, u32_to_usize};
use crate::error::DecodeError;
use serde::{Deserialize, Serialize};

/// Layout of one pixel in the input buffer.
///
/// Multi-byte samples are **little-endian**. The specification does not fix an
/// endianness, so this build declares one rather than inheriting the host's
/// (PTE-DET-001 requires native and WASM to agree).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
#[non_exhaustive]
pub enum PixelFormat {
    /// One 8-bit luminance sample, fully opaque.
    Gray8,
    /// 8-bit luminance followed by 8-bit alpha.
    GrayAlpha8,
    /// Three 8-bit samples, red then green then blue, fully opaque.
    Rgb8,
    /// Four 8-bit samples, red green blue alpha.
    Rgba8,
    /// Four 16-bit little-endian samples, red green blue alpha.
    Rgba16,
    /// Four 32-bit little-endian IEEE floats, red green blue alpha.
    RgbaF32,
}

impl PixelFormat {
    /// Bytes one pixel occupies.
    #[must_use]
    pub const fn bytes_per_pixel(self) -> usize {
        match self {
            Self::Gray8 => 1,
            Self::GrayAlpha8 => 2,
            Self::Rgb8 => 3,
            Self::Rgba8 => 4,
            Self::Rgba16 => 8,
            Self::RgbaF32 => 16,
        }
    }

    /// Whether the format carries an alpha channel.
    #[must_use]
    pub const fn has_alpha(self) -> bool {
        matches!(
            self,
            Self::GrayAlpha8 | Self::Rgba8 | Self::Rgba16 | Self::RgbaF32
        )
    }
}

/// How the numeric channel values are encoded (§7.1, PTE-COLOR-003).
///
/// The engine linearises on the way in and applies the inverse only at output
/// or preview boundaries. Declaring the input encoding is the caller's job;
/// the core never guesses from the pixel statistics.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
#[non_exhaustive]
pub enum ColorEncoding {
    /// sRGB transfer function, sRGB primaries.
    Srgb,
    /// Already linear-light, sRGB primaries.
    LinearSrgb,
}

/// Whether colour channels are premultiplied by alpha.
///
/// PTE-COLOR-009: straight-alpha input is converted with a documented epsilon
/// and must not amplify arbitrary transparent RGB into palette evidence.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
#[non_exhaustive]
pub enum AlphaMode {
    /// Colour channels are independent of alpha.
    Straight,
    /// Colour channels are already multiplied by alpha.
    Premultiplied,
    /// The image has no meaningful alpha; every pixel is opaque.
    Opaque,
}

/// How orientation metadata was handled (PTE-API-003).
///
/// The core never guesses. A codec or host adapter either applies the
/// transform and says so, or declares that none was present.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
#[non_exhaustive]
pub enum Orientation {
    /// No orientation metadata existed, or it was the identity.
    Identity,
    /// Orientation metadata existed and the adapter has already applied it.
    AlreadyApplied,
}

/// A validated, borrowed view of decoded pixels.
///
/// Construction performs every check PTE-API-001 lists. After construction,
/// [`ImageView::sample`] is total for any `(x, y)` inside the declared
/// dimensions, which is why the sampling path has no bounds error to return.
///
/// PTE-ARCH-002: the view borrows. Nothing here clones the buffer.
#[derive(Debug, Clone, Copy)]
pub struct ImageView<'a> {
    width: u32,
    height: u32,
    stride_bytes: usize,
    format: PixelFormat,
    data: &'a [u8],
    color_encoding: ColorEncoding,
    alpha_mode: AlphaMode,
    orientation: Orientation,
}

impl<'a> ImageView<'a> {
    /// Validate and wrap a decoded pixel buffer.
    ///
    /// Pass `stride_bytes = 0` for tightly packed rows; it is replaced by
    /// `width * bytes_per_pixel`.
    ///
    /// # Errors
    ///
    /// * [`DecodeError::EmptyImage`] -- a dimension is zero.
    /// * [`DecodeError::DimensionOverflow`] -- the geometry overflows `usize`.
    /// * [`DecodeError::StrideTooSmall`] -- the stride cannot hold one row.
    /// * [`DecodeError::StrideMisaligned`] -- rows would not start on a pixel
    ///   boundary.
    /// * [`DecodeError::BufferTooShort`] -- fewer bytes than the geometry
    ///   needs.
    // The parameter list is §6.1's field list. Grouping some of them into a
    // sub-struct to satisfy an arity lint would put a shape in the public API
    // that the specification does not describe.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        width: u32,
        height: u32,
        stride_bytes: usize,
        format: PixelFormat,
        data: &'a [u8],
        color_encoding: ColorEncoding,
        alpha_mode: AlphaMode,
        orientation: Orientation,
    ) -> Result<Self, DecodeError> {
        // Order matters: every later check divides or multiplies by a quantity
        // this one proved finite (PTE-SEC-005).
        let _ = pixel_count(width, height)?;

        let bpp = format.bytes_per_pixel();
        let row_bytes = mul_bytes(u32_to_usize(width), bpp, width, height)?;

        let stride = if stride_bytes == 0 {
            row_bytes
        } else {
            stride_bytes
        };
        if stride < row_bytes {
            return Err(DecodeError::StrideTooSmall {
                stride_bytes: stride,
                minimum: row_bytes,
            });
        }
        if stride % bpp != 0 {
            return Err(DecodeError::StrideMisaligned {
                stride_bytes: stride,
                bytes_per_pixel: bpp,
            });
        }

        // The last row needs `row_bytes`, not a full stride. Requiring a full
        // trailing stride would reject buffers that are exactly large enough,
        // which is a common way to hand over a sub-rectangle of a larger image.
        let full_rows = mul_bytes(stride, u32_to_usize(height) - 1, width, height)?;
        let needed = full_rows
            .checked_add(row_bytes)
            .ok_or(DecodeError::DimensionOverflow { width, height })?;
        if data.len() < needed {
            return Err(DecodeError::BufferTooShort {
                got: data.len(),
                needed,
            });
        }

        Ok(Self {
            width,
            height,
            stride_bytes: stride,
            format,
            data,
            color_encoding,
            alpha_mode,
            orientation,
        })
    }

    /// Convenience constructor for tightly packed 8-bit sRGB RGBA.
    ///
    /// # Errors
    ///
    /// As [`ImageView::new`].
    pub fn rgba8(width: u32, height: u32, data: &'a [u8]) -> Result<Self, DecodeError> {
        Self::new(
            width,
            height,
            0,
            PixelFormat::Rgba8,
            data,
            ColorEncoding::Srgb,
            AlphaMode::Straight,
            Orientation::Identity,
        )
    }

    /// Image width in pixels.
    #[must_use]
    pub const fn width(&self) -> u32 {
        self.width
    }

    /// Image height in pixels.
    #[must_use]
    pub const fn height(&self) -> u32 {
        self.height
    }

    /// Distance between the starts of consecutive rows, in bytes.
    #[must_use]
    pub const fn stride_bytes(&self) -> usize {
        self.stride_bytes
    }

    /// Layout of one pixel.
    #[must_use]
    pub const fn format(&self) -> PixelFormat {
        self.format
    }

    /// Declared transfer function of the channel values.
    #[must_use]
    pub const fn color_encoding(&self) -> ColorEncoding {
        self.color_encoding
    }

    /// Declared alpha association.
    #[must_use]
    pub const fn alpha_mode(&self) -> AlphaMode {
        self.alpha_mode
    }

    /// How orientation metadata was handled.
    #[must_use]
    pub const fn orientation(&self) -> Orientation {
        self.orientation
    }

    /// Number of pixels, validated non-overflowing at construction.
    #[must_use]
    pub fn pixel_count(&self) -> usize {
        u32_to_usize(self.width) * u32_to_usize(self.height)
    }

    /// Whether `(x, y)` is inside the image.
    #[must_use]
    pub const fn contains(&self, x: u32, y: u32) -> bool {
        x < self.width && y < self.height
    }

    /// Row-major index of `(x, y)`, for indexing parallel planes.
    ///
    /// # Panics
    ///
    /// Debug builds panic if `(x, y)` is outside the image. Release builds
    /// return a value that will index out of the plane, which the caller's own
    /// bounds check catches -- callers are internal and always iterate in
    /// range.
    #[inline]
    #[must_use]
    pub fn index(&self, x: u32, y: u32) -> usize {
        debug_assert!(self.contains(x, y), "index out of image domain");
        u32_to_usize(y) * u32_to_usize(self.width) + u32_to_usize(x)
    }

    /// Read pixel `(x, y)` as four channels in `[0, 1]`, exactly as stored.
    ///
    /// No transfer function is applied and alpha association is not changed;
    /// the returned values still mean whatever [`ImageView::color_encoding`]
    /// and [`ImageView::alpha_mode`] say they mean. Interpretation is the
    /// colour crate's job (PTE-COLOR-001).
    ///
    /// Returns `[0, 0, 0, 0]` outside the image so that halo reads at the
    /// domain border are total. Callers that must distinguish the exterior use
    /// [`ImageView::contains`].
    #[must_use]
    pub fn sample(&self, x: u32, y: u32) -> [f64; 4] {
        if !self.contains(x, y) {
            return [0.0; 4];
        }
        let base = u32_to_usize(y) * self.stride_bytes
            + u32_to_usize(x) * self.format.bytes_per_pixel();
        let b = &self.data[base..];
        match self.format {
            PixelFormat::Gray8 => {
                let g = checked::u8_to_unit(b[0]);
                [g, g, g, 1.0]
            }
            PixelFormat::GrayAlpha8 => {
                let g = checked::u8_to_unit(b[0]);
                [g, g, g, checked::u8_to_unit(b[1])]
            }
            PixelFormat::Rgb8 => [
                checked::u8_to_unit(b[0]),
                checked::u8_to_unit(b[1]),
                checked::u8_to_unit(b[2]),
                1.0,
            ],
            PixelFormat::Rgba8 => [
                checked::u8_to_unit(b[0]),
                checked::u8_to_unit(b[1]),
                checked::u8_to_unit(b[2]),
                checked::u8_to_unit(b[3]),
            ],
            PixelFormat::Rgba16 => {
                let s = |i: usize| checked::u16_to_unit(u16::from_le_bytes([b[i], b[i + 1]]));
                [s(0), s(2), s(4), s(6)]
            }
            PixelFormat::RgbaF32 => {
                let s = |i: usize| {
                    let v = f32::from_le_bytes([b[i], b[i + 1], b[i + 2], b[i + 3]]);
                    checked::clamp_unit(f64::from(v))
                };
                [s(0), s(4), s(8), s(12)]
            }
        }
    }

    /// Alpha of pixel `(x, y)` in `[0, 1]`, or `0.0` outside the image.
    #[inline]
    #[must_use]
    pub fn alpha(&self, x: u32, y: u32) -> f64 {
        match self.alpha_mode {
            AlphaMode::Opaque if self.contains(x, y) => 1.0,
            AlphaMode::Opaque => 0.0,
            _ => self.sample(x, y)[3],
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::float_cmp, clippy::field_reassign_with_default)]
mod tests {
    use super::*;

    fn view(w: u32, h: u32, stride: usize, fmt: PixelFormat, data: &[u8]) -> Result<ImageView<'_>, DecodeError> {
        ImageView::new(
            w,
            h,
            stride,
            fmt,
            data,
            ColorEncoding::Srgb,
            AlphaMode::Straight,
            Orientation::Identity,
        )
    }

    #[test]
    fn zero_dimensions_are_refused() {
        let data = [0u8; 16];
        assert_eq!(
            view(0, 4, 0, PixelFormat::Rgba8, &data).unwrap_err(),
            DecodeError::EmptyImage
        );
        assert_eq!(
            view(4, 0, 0, PixelFormat::Rgba8, &data).unwrap_err(),
            DecodeError::EmptyImage
        );
    }

    #[test]
    fn a_short_buffer_is_refused_before_any_read() {
        let data = [0u8; 15];
        assert_eq!(
            view(2, 2, 0, PixelFormat::Rgba8, &data).unwrap_err(),
            DecodeError::BufferTooShort {
                got: 15,
                needed: 16
            }
        );
    }

    #[test]
    fn a_stride_below_one_row_is_refused() {
        let data = [0u8; 64];
        assert_eq!(
            view(4, 2, 8, PixelFormat::Rgba8, &data).unwrap_err(),
            DecodeError::StrideTooSmall {
                stride_bytes: 8,
                minimum: 16
            }
        );
    }

    #[test]
    fn a_stride_that_splits_a_pixel_is_refused() {
        let data = [0u8; 256];
        assert_eq!(
            view(4, 2, 17, PixelFormat::Rgba8, &data).unwrap_err(),
            DecodeError::StrideMisaligned {
                stride_bytes: 17,
                bytes_per_pixel: 4
            }
        );
    }

    /// A sub-rectangle handed over as the tail of a larger buffer is exactly
    /// `stride * (h - 1) + row_bytes` long. Demanding a full trailing stride
    /// would reject it for no reason.
    #[test]
    fn the_last_row_does_not_need_a_full_trailing_stride() {
        let stride = 32;
        let data = vec![0u8; stride * 2 + 16];
        let v = view(4, 3, stride, PixelFormat::Rgba8, &data).unwrap();
        assert_eq!(v.pixel_count(), 12);
    }

    #[test]
    fn padded_rows_are_sampled_from_the_right_offset() {
        // 2x2 RGBA8 with 8 bytes of padding after each row.
        let mut data = vec![0u8; 2 * (8 + 8)];
        data[0..4].copy_from_slice(&[255, 0, 0, 255]); // (0,0) red
        data[4..8].copy_from_slice(&[0, 255, 0, 255]); // (1,0) green
        data[16..20].copy_from_slice(&[0, 0, 255, 255]); // (0,1) blue
        let v = view(2, 2, 16, PixelFormat::Rgba8, &data).unwrap();
        assert_eq!(v.sample(0, 0), [1.0, 0.0, 0.0, 1.0]);
        assert_eq!(v.sample(1, 0), [0.0, 1.0, 0.0, 1.0]);
        assert_eq!(v.sample(0, 1), [0.0, 0.0, 1.0, 1.0]);
    }

    #[test]
    fn sampling_outside_the_domain_is_total_and_transparent() {
        let data = [255u8; 16];
        let v = view(2, 2, 0, PixelFormat::Rgba8, &data).unwrap();
        assert_eq!(v.sample(2, 0), [0.0; 4]);
        assert_eq!(v.sample(0, 99), [0.0; 4]);
        assert_eq!(v.alpha(9, 9), 0.0);
    }

    #[test]
    fn sixteen_bit_samples_are_little_endian() {
        // One RGBA16 pixel: R = 0xFFFF, G = 0x0000, B = 0x8000, A = 0xFFFF.
        let data = [0xFF, 0xFF, 0x00, 0x00, 0x00, 0x80, 0xFF, 0xFF];
        let v = view(1, 1, 0, PixelFormat::Rgba16, &data).unwrap();
        let s = v.sample(0, 0);
        assert_eq!(s[0], 1.0);
        assert_eq!(s[1], 0.0);
        assert!((s[2] - 32768.0 / 65535.0).abs() < 1e-12);
        assert_eq!(s[3], 1.0);
    }

    #[test]
    fn opaque_alpha_mode_reports_one_inside_and_zero_outside() {
        let data = [0u8; 4];
        let v = ImageView::new(
            2,
            2,
            0,
            PixelFormat::Gray8,
            &data,
            ColorEncoding::Srgb,
            AlphaMode::Opaque,
            Orientation::Identity,
        )
        .unwrap();
        assert_eq!(v.alpha(1, 1), 1.0);
        assert_eq!(v.alpha(2, 1), 0.0);
    }
}
