//! Paint servers (§6.4, §18.5).
//!
//! PTE-SVG-012: "Exact palette mode MUST emit the requested output color
//! values or a documented lossless equivalent. Internal OKLab anchors are not
//! substituted for user colors." [`Color`] therefore stores the caller's
//! 8-bit sRGB bytes verbatim. The OKLab anchor used for matching lives beside
//! it in the colour crate and never comes back out.

use serde::{Deserialize, Serialize};

/// An output colour: straight (non-premultiplied) 8-bit sRGB with alpha.
///
/// Deliberately integral. A float colour would invite a round-trip through
/// linear light and back, and the whole point of exact-palette mode is that
/// the bytes the caller supplied are the bytes that come out.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct Color {
    /// Red, 0-255.
    pub r: u8,
    /// Green, 0-255.
    pub g: u8,
    /// Blue, 0-255.
    pub b: u8,
    /// Alpha, 0-255, straight.
    pub a: u8,
}

impl Color {
    /// An opaque colour.
    #[must_use]
    pub const fn rgb(r: u8, g: u8, b: u8) -> Self {
        Self { r, g, b, a: 255 }
    }

    /// A colour with alpha.
    #[must_use]
    pub const fn rgba(r: u8, g: u8, b: u8, a: u8) -> Self {
        Self { r, g, b, a }
    }

    /// From the `[r, g, b, a]` array a parsed palette entry carries.
    #[must_use]
    pub const fn from_array(v: [u8; 4]) -> Self {
        Self {
            r: v[0],
            g: v[1],
            b: v[2],
            a: v[3],
        }
    }

    /// Fully transparent black.
    pub const TRANSPARENT: Self = Self {
        r: 0,
        g: 0,
        b: 0,
        a: 0,
    };

    /// Opaque black.
    pub const BLACK: Self = Self::rgb(0, 0, 0);

    /// Opaque white.
    pub const WHITE: Self = Self::rgb(255, 255, 255);

    /// Whether the colour is fully transparent.
    #[must_use]
    pub const fn is_transparent(self) -> bool {
        self.a == 0
    }

    /// The `#rrggbb` form, for the `fill` attribute.
    ///
    /// Alpha is *not* folded in here. §18.5 and PTE-SVG-013 require alpha to
    /// be applied exactly once; the serialiser emits it as a separate
    /// `fill-opacity`, so encoding it here as well would double-apply it.
    #[must_use]
    pub fn to_hex_rgb(self) -> String {
        format!("#{:02x}{:02x}{:02x}", self.r, self.g, self.b)
    }

    /// Alpha as an opacity in `[0, 1]`.
    #[must_use]
    pub fn opacity(self) -> f64 {
        f64::from(self.a) / 255.0
    }
}

/// How a gradient extends beyond its stops.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
#[non_exhaustive]
pub enum SpreadMethod {
    /// Clamp the end stops.
    Pad,
    /// Mirror.
    Reflect,
    /// Tile.
    Repeat,
}

/// One gradient stop.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct GradientStop {
    /// Position along the gradient, in `[0, 1]`.
    pub offset: f64,
    /// Colour at this position.
    pub color: Color,
}

/// A linear gradient (§14.4).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LinearGradient {
    /// Start point in user space.
    pub from: super::geom::Point,
    /// End point in user space.
    pub to: super::geom::Point,
    /// Stops, ordered by offset.
    pub stops: Vec<GradientStop>,
    /// Spread method.
    pub spread: SpreadMethod,
}

/// A radial gradient (§14.5).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RadialGradient {
    /// Centre in user space.
    pub center: super::geom::Point,
    /// Radius in user space.
    pub radius: f64,
    /// Focal point, if not the centre.
    pub focus: Option<super::geom::Point>,
    /// Stops, ordered by offset.
    pub stops: Vec<GradientStop>,
    /// Spread method.
    pub spread: SpreadMethod,
    /// Elliptical geometry is expressed as a transform on a circular gradient
    /// rather than approximated with bands (PTE-GRAD-006). Row-major
    /// `[a, b, c, d, e, f]`.
    pub transform: Option<[f64; 6]>,
}

/// A paint server (§6.4).
///
/// Only [`Paint::Solid`] is produced by this build. The gradient variants are
/// part of the contract and are refused at configuration time
/// (PTE-GRAD-001), so an unimplemented paint can never reach the serialiser.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
#[non_exhaustive]
pub enum Paint {
    /// A solid colour.
    Solid(Color),
    /// A linear gradient.
    LinearGradient(LinearGradient),
    /// A radial or elliptical gradient.
    RadialGradient(RadialGradient),
}

impl Paint {
    /// Whether this paint is portable SVG 2 (§14.1).
    #[must_use]
    pub const fn is_portable(&self) -> bool {
        matches!(
            self,
            Self::Solid(_) | Self::LinearGradient(_) | Self::RadialGradient(_)
        )
    }

    /// The solid colour, if this is one.
    #[must_use]
    pub const fn as_solid(&self) -> Option<Color> {
        match self {
            Self::Solid(c) => Some(*c),
            _ => None,
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::float_cmp, clippy::field_reassign_with_default)]
mod tests {
    use super::*;

    /// PTE-SVG-013: alpha is applied once. If `to_hex_rgb` folded alpha in and
    /// the serialiser also emitted `fill-opacity`, a 50% colour would render
    /// at 25%.
    #[test]
    fn hex_output_carries_no_alpha() {
        let half = Color::rgba(255, 128, 0, 128);
        assert_eq!(half.to_hex_rgb(), "#ff8000");
        assert!((half.opacity() - 128.0 / 255.0).abs() < 1e-12);
    }

    /// PTE-SVG-012: the caller's bytes survive unchanged.
    #[test]
    fn a_palette_colour_survives_verbatim() {
        let c = Color::from_array([0x12, 0x34, 0x56, 0xff]);
        assert_eq!(c.to_hex_rgb(), "#123456");
        assert_eq!(c, Color::rgb(0x12, 0x34, 0x56));
    }

    #[test]
    fn transparency_is_detected_regardless_of_colour() {
        assert!(Color::rgba(255, 255, 255, 0).is_transparent());
        assert!(!Color::rgba(0, 0, 0, 1).is_transparent());
    }
}
