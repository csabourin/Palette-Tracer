//! Geometric primitives.
//!
//! Everything is `f64` (PTE-API-011). Coordinates are in the image domain
//! `[0, width] x [0, height]` with pixel centres at `(x + 0.5, y + 0.5)`
//! (PTE-API-002), and `y` increases downward, matching both the raster and the
//! SVG user space.

use serde::{Deserialize, Serialize};

/// A point in image coordinates.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Point {
    /// Horizontal coordinate.
    pub x: f64,
    /// Vertical coordinate, increasing downward.
    pub y: f64,
}

impl Point {
    /// A point.
    #[must_use]
    pub const fn new(x: f64, y: f64) -> Self {
        Self { x, y }
    }

    /// The origin.
    pub const ORIGIN: Self = Self { x: 0.0, y: 0.0 };

    /// Whether both coordinates are finite (PTE-SVG-003).
    #[must_use]
    pub fn is_finite(self) -> bool {
        self.x.is_finite() && self.y.is_finite()
    }

    /// Euclidean distance to `other`.
    #[must_use]
    pub fn distance(self, other: Self) -> f64 {
        (self - other).length()
    }

    /// Squared distance to `other`, avoiding a square root.
    #[must_use]
    pub fn distance_squared(self, other: Self) -> f64 {
        let d = self - other;
        d.x * d.x + d.y * d.y
    }

    /// Linear interpolation toward `other`.
    #[must_use]
    pub fn lerp(self, other: Self, t: f64) -> Self {
        Self::new(self.x + (other.x - self.x) * t, self.y + (other.y - self.y) * t)
    }
}

impl core::ops::Sub for Point {
    type Output = Vec2;
    fn sub(self, rhs: Self) -> Vec2 {
        Vec2::new(self.x - rhs.x, self.y - rhs.y)
    }
}

impl core::ops::Add<Vec2> for Point {
    type Output = Self;
    fn add(self, rhs: Vec2) -> Self {
        Self::new(self.x + rhs.x, self.y + rhs.y)
    }
}

impl core::ops::Sub<Vec2> for Point {
    type Output = Self;
    fn sub(self, rhs: Vec2) -> Self {
        Self::new(self.x - rhs.x, self.y - rhs.y)
    }
}

/// A displacement in image coordinates.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Vec2 {
    /// Horizontal component.
    pub x: f64,
    /// Vertical component.
    pub y: f64,
}

impl Vec2 {
    /// A vector.
    #[must_use]
    pub const fn new(x: f64, y: f64) -> Self {
        Self { x, y }
    }

    /// The zero vector.
    pub const ZERO: Self = Self { x: 0.0, y: 0.0 };

    /// Euclidean length.
    #[must_use]
    pub fn length(self) -> f64 {
        self.x.hypot(self.y)
    }

    /// Squared length.
    #[must_use]
    pub fn length_squared(self) -> f64 {
        self.x * self.x + self.y * self.y
    }

    /// Unit vector, or `None` if the length is below `epsilon`.
    ///
    /// Returning `None` rather than a NaN-filled vector is what keeps
    /// PTE-GEO-006 ("degenerate solves MUST NOT emit NaN control points")
    /// mechanical.
    #[must_use]
    pub fn normalized(self, epsilon: f64) -> Option<Self> {
        let len = self.length();
        if len <= epsilon || !len.is_finite() {
            None
        } else {
            Some(Self::new(self.x / len, self.y / len))
        }
    }

    /// Dot product.
    #[must_use]
    pub fn dot(self, other: Self) -> f64 {
        self.x * other.x + self.y * other.y
    }

    /// Two-dimensional cross product, positive when `other` is
    /// counter-clockwise from `self` in a y-down space.
    #[must_use]
    pub fn cross(self, other: Self) -> f64 {
        self.x * other.y - self.y * other.x
    }

    /// Rotate a quarter turn, giving a normal.
    #[must_use]
    pub fn perp(self) -> Self {
        Self::new(-self.y, self.x)
    }

    /// Whether both components are finite.
    #[must_use]
    pub fn is_finite(self) -> bool {
        self.x.is_finite() && self.y.is_finite()
    }
}

impl core::ops::Mul<f64> for Vec2 {
    type Output = Self;
    fn mul(self, rhs: f64) -> Self {
        Self::new(self.x * rhs, self.y * rhs)
    }
}

impl core::ops::Add for Vec2 {
    type Output = Self;
    fn add(self, rhs: Self) -> Self {
        Self::new(self.x + rhs.x, self.y + rhs.y)
    }
}

impl core::ops::Sub for Vec2 {
    type Output = Self;
    fn sub(self, rhs: Self) -> Self {
        Self::new(self.x - rhs.x, self.y - rhs.y)
    }
}

/// An axis-aligned rectangle.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Rect {
    /// Left edge.
    pub x: f64,
    /// Top edge.
    pub y: f64,
    /// Width, non-negative.
    pub width: f64,
    /// Height, non-negative.
    pub height: f64,
}

impl Rect {
    /// A rectangle.
    #[must_use]
    pub const fn new(x: f64, y: f64, width: f64, height: f64) -> Self {
        Self {
            x,
            y,
            width,
            height,
        }
    }

    /// The image domain for a `width x height` raster (PTE-API-002).
    #[must_use]
    pub fn image_domain(width: u32, height: u32) -> Self {
        Self::new(0.0, 0.0, f64::from(width), f64::from(height))
    }

    /// Whether every field is finite and the extents are positive
    /// (PTE-SVG-002).
    #[must_use]
    pub fn is_valid_viewport(self) -> bool {
        self.x.is_finite()
            && self.y.is_finite()
            && self.width.is_finite()
            && self.height.is_finite()
            && self.width > 0.0
            && self.height > 0.0
    }
}

/// One segment of a path (§6.4).
///
/// The start point is the previous segment's end, or the subpath's move-to.
/// PTE-SVG-004 requires SVG path construction to originate from these typed
/// segments; the serialiser never concatenates caller-provided strings.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
#[non_exhaustive]
pub enum PathSegment {
    /// Straight line to `to`.
    Line {
        /// End point.
        to: Point,
    },
    /// Cubic Bézier with control points `c1` and `c2`.
    Cubic {
        /// First control point.
        c1: Point,
        /// Second control point.
        c2: Point,
        /// End point.
        to: Point,
    },
    /// Elliptical arc, in SVG's parameterisation.
    Arc {
        /// Semi-axes.
        radii: Vec2,
        /// X-axis rotation, in radians.
        rotation: f64,
        /// Large-arc flag.
        large: bool,
        /// Sweep flag.
        sweep: bool,
        /// End point.
        to: Point,
    },
}

impl PathSegment {
    /// The segment's end point.
    #[must_use]
    pub const fn end(&self) -> Point {
        match self {
            Self::Line { to } | Self::Cubic { to, .. } | Self::Arc { to, .. } => *to,
        }
    }

    /// Whether every coordinate is finite and every parameter is legal.
    ///
    /// PTE-SVG-003 forbids NaN, infinity, a negative radius, and an invalid
    /// arc flag from reaching the serialiser. Booleans cannot be invalid in
    /// Rust, so the flags need no check; the radii do.
    #[must_use]
    pub fn is_finite(&self) -> bool {
        match self {
            Self::Line { to } => to.is_finite(),
            Self::Cubic { c1, c2, to } => c1.is_finite() && c2.is_finite() && to.is_finite(),
            Self::Arc {
                radii,
                rotation,
                to,
                ..
            } => {
                radii.is_finite()
                    && radii.x >= 0.0
                    && radii.y >= 0.0
                    && rotation.is_finite()
                    && to.is_finite()
            }
        }
    }

    /// Number of control points this segment contributes to the node count
    /// reported under §26.7.
    #[must_use]
    pub const fn control_point_count(&self) -> u32 {
        match self {
            Self::Line { .. } | Self::Arc { .. } => 1,
            Self::Cubic { .. } => 3,
        }
    }
}

/// A polyline or curve chain: a start point followed by segments.
///
/// The one geometry a shared edge owns. Both incident faces reference it;
/// neither owns a copy (PTE-TOPO-001, PTE-GEO-012).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CurveChain {
    /// Where the chain begins.
    pub start: Point,
    /// Segments in order.
    pub segments: Vec<PathSegment>,
}

impl CurveChain {
    /// An empty chain starting at `start`.
    #[must_use]
    pub fn new(start: Point) -> Self {
        Self {
            start,
            segments: Vec::new(),
        }
    }

    /// A chain of straight lines through `points`.
    ///
    /// Returns `None` if fewer than two points are supplied, because a chain
    /// with no segments has no direction to reverse.
    #[must_use]
    pub fn polyline(points: &[Point]) -> Option<Self> {
        let (&start, rest) = points.split_first()?;
        if rest.is_empty() {
            return None;
        }
        Some(Self {
            start,
            segments: rest.iter().map(|&to| PathSegment::Line { to }).collect(),
        })
    }

    /// The chain's end point.
    #[must_use]
    pub fn end(&self) -> Point {
        self.segments.last().map_or(self.start, PathSegment::end)
    }

    /// Whether every coordinate is finite.
    #[must_use]
    pub fn is_finite(&self) -> bool {
        self.start.is_finite() && self.segments.iter().all(PathSegment::is_finite)
    }

    /// The same geometry traversed backwards.
    ///
    /// This is the operation PTE-TOPO-001 requires: "Reversing a face
    /// traversal reverses the same curve chain; it does not clone and
    /// independently refit it." Reversal is exact -- control points swap,
    /// coordinates are copied, nothing is recomputed -- so reversing twice is
    /// the identity down to the bit, and the two faces of a shared edge
    /// serialise from identical numbers (PTE-SVG-009).
    #[must_use]
    pub fn reversed(&self) -> Self {
        let mut points = Vec::with_capacity(self.segments.len() + 1);
        points.push(self.start);
        for s in &self.segments {
            points.push(s.end());
        }

        let mut segments = Vec::with_capacity(self.segments.len());
        for (i, s) in self.segments.iter().enumerate().rev() {
            let from = points[i];
            segments.push(match *s {
                PathSegment::Line { .. } => PathSegment::Line { to: from },
                PathSegment::Cubic { c1, c2, .. } => PathSegment::Cubic {
                    c1: c2,
                    c2: c1,
                    to: from,
                },
                PathSegment::Arc {
                    radii,
                    rotation,
                    large,
                    sweep,
                    ..
                } => PathSegment::Arc {
                    radii,
                    rotation,
                    large,
                    // Traversing the same arc backwards keeps the large-arc
                    // flag and flips the sweep direction.
                    sweep: !sweep,
                    to: from,
                },
            });
        }

        Self {
            start: self.end(),
            segments,
        }
    }

    /// Total control points, for the §26.7 complexity report.
    #[must_use]
    pub fn control_point_count(&self) -> u32 {
        1 + self
            .segments
            .iter()
            .map(PathSegment::control_point_count)
            .sum::<u32>()
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::float_cmp, clippy::field_reassign_with_default)]
mod tests {
    use super::*;

    fn sample_chain() -> CurveChain {
        CurveChain {
            start: Point::new(1.0, 2.0),
            segments: vec![
                PathSegment::Line {
                    to: Point::new(3.0, 4.0),
                },
                PathSegment::Cubic {
                    c1: Point::new(4.0, 5.0),
                    c2: Point::new(6.0, 7.0),
                    to: Point::new(8.0, 9.0),
                },
                PathSegment::Arc {
                    radii: Vec2::new(2.0, 3.0),
                    rotation: 0.5,
                    large: true,
                    sweep: false,
                    to: Point::new(10.0, 11.0),
                },
            ],
        }
    }

    /// PTE-TOPO-001's operational content, and the §27.2 property "reversing
    /// twice returns original geometry". Exact equality is the assertion, not
    /// approximate: reversal copies coordinates and must not perturb them.
    #[test]
    fn reversing_twice_is_the_identity_exactly() {
        let chain = sample_chain();
        assert_eq!(chain.reversed().reversed(), chain);
    }

    #[test]
    fn reversal_swaps_endpoints_and_cubic_controls() {
        let chain = sample_chain();
        let r = chain.reversed();
        assert_eq!(r.start, chain.end());
        assert_eq!(r.end(), chain.start);
        assert_eq!(r.segments.len(), chain.segments.len());

        // The middle cubic keeps its geometry with controls exchanged.
        match (r.segments[1], chain.segments[1]) {
            (
                PathSegment::Cubic {
                    c1: rc1, c2: rc2, ..
                },
                PathSegment::Cubic {
                    c1: fc1, c2: fc2, ..
                },
            ) => {
                assert_eq!(rc1, fc2);
                assert_eq!(rc2, fc1);
            }
            _ => panic!("expected cubics"),
        }
    }

    #[test]
    fn reversal_flips_arc_sweep_but_not_large_arc() {
        let r = sample_chain().reversed();
        match r.segments[0] {
            PathSegment::Arc { large, sweep, .. } => {
                assert!(large, "the large-arc flag describes the arc, not the direction");
                assert!(sweep, "the sweep flag describes the direction and must flip");
            }
            _ => panic!("expected the arc first after reversal"),
        }
    }

    #[test]
    fn a_polyline_needs_at_least_two_points() {
        assert!(CurveChain::polyline(&[]).is_none());
        assert!(CurveChain::polyline(&[Point::ORIGIN]).is_none());
        assert!(CurveChain::polyline(&[Point::ORIGIN, Point::new(1.0, 0.0)]).is_some());
    }

    /// PTE-SVG-003: a negative radius must never reach the serialiser, and the
    /// check that stops it lives on the segment itself.
    #[test]
    fn non_finite_and_illegal_segments_are_detected() {
        assert!(!PathSegment::Line {
            to: Point::new(f64::NAN, 0.0)
        }
        .is_finite());
        assert!(!PathSegment::Arc {
            radii: Vec2::new(-1.0, 1.0),
            rotation: 0.0,
            large: false,
            sweep: false,
            to: Point::ORIGIN,
        }
        .is_finite());
        assert!(sample_chain().is_finite());
    }

    #[test]
    fn a_zero_length_vector_normalises_to_none_not_nan() {
        assert!(Vec2::ZERO.normalized(1e-12).is_none());
        let n = Vec2::new(3.0, 4.0).normalized(1e-12).unwrap();
        assert!((n.length() - 1.0).abs() < 1e-15);
    }

    #[test]
    fn the_image_domain_is_a_valid_viewport() {
        assert!(Rect::image_domain(16, 9).is_valid_viewport());
        assert!(!Rect::new(0.0, 0.0, 0.0, 1.0).is_valid_viewport());
        assert!(!Rect::new(0.0, 0.0, f64::NAN, 1.0).is_valid_viewport());
    }
}
