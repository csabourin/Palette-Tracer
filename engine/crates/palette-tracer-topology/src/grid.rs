//! The raster grid: labels, stubs, and the direction convention.
//!
//! The whole of §9 rests on one geometric convention, so it is stated once
//! here and never restated.
//!
//! * Pixel `(x, y)` occupies the unit square `[x, x+1] x [y, y+1]`, and its
//!   centre is `(x + 0.5, y + 0.5)` (PTE-API-002).
//! * Grid vertices are the integer points `(x, y)` for `0 ≤ x ≤ width` and
//!   `0 ≤ y ≤ height`.
//! * `y` increases downward, matching the raster and SVG user space.
//!
//! # Which face is on the left
//!
//! A *dart* is a directed elementary segment. Its left side is the quarter
//! turn anticlockwise on paper, which with `y` downward is the direction
//! `(dy, -dx)`:
//!
//! | direction | left is |
//! |---|---|
//! | east `(1, 0)` | north |
//! | south `(0, 1)` | east |
//! | west `(-1, 0)` | south |
//! | north `(0, -1)` | west |
//!
//! Every face cycle is traversed with the face on the left, which is why the
//! twin of a half-edge is exactly the reverse traversal (PTE-TOPO-001).

use palette_tracer_core::labels::{LabelId, LabelPlane};

/// The four compass directions, in clockwise order as seen on screen.
///
/// Clockwise because `y` points down: north, east, south, west is the order a
/// clock hand sweeps. The order is what the tight-turn rule in [`crate::extract`]
/// scans, so it is fixed here rather than re-derived.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(u8)]
pub enum Dir {
    /// Toward decreasing `y`.
    North = 0,
    /// Toward increasing `x`.
    East = 1,
    /// Toward increasing `y`.
    South = 2,
    /// Toward decreasing `x`.
    West = 3,
}

impl Dir {
    /// All four, clockwise from north.
    pub const ALL: [Self; 4] = [Self::North, Self::East, Self::South, Self::West];

    /// From a clockwise index.
    #[must_use]
    pub const fn from_index(index: u8) -> Self {
        match index & 3 {
            0 => Self::North,
            1 => Self::East,
            2 => Self::South,
            _ => Self::West,
        }
    }

    /// Clockwise index.
    #[must_use]
    pub const fn index(self) -> u8 {
        self as u8
    }

    /// The opposite direction.
    #[must_use]
    pub const fn opposite(self) -> Self {
        Self::from_index(self.index() + 2)
    }

    /// Rotate `steps` positions clockwise.
    #[must_use]
    pub const fn rotate(self, steps: u8) -> Self {
        Self::from_index(self.index() + steps)
    }

    /// Unit step in grid coordinates.
    #[must_use]
    pub const fn step(self) -> (i64, i64) {
        match self {
            Self::North => (0, -1),
            Self::East => (1, 0),
            Self::South => (0, 1),
            Self::West => (-1, 0),
        }
    }
}

/// A grid vertex.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct GridPoint {
    /// Column, `0 ..= width`.
    pub x: u32,
    /// Row, `0 ..= height`.
    pub y: u32,
}

impl GridPoint {
    /// A grid vertex.
    #[must_use]
    pub const fn new(x: u32, y: u32) -> Self {
        Self { x, y }
    }

    /// The vertex one step in `dir`, or `None` if that leaves the grid.
    #[must_use]
    pub fn step(self, dir: Dir, width: u32, height: u32) -> Option<Self> {
        let (dx, dy) = dir.step();
        let x = i64::from(self.x) + dx;
        let y = i64::from(self.y) + dy;
        if x < 0 || y < 0 || x > i64::from(width) || y > i64::from(height) {
            None
        } else {
            Some(Self::new(x as u32, y as u32))
        }
    }

    /// As an image-space point.
    #[must_use]
    pub fn to_point(self) -> palette_tracer_core::ir::Point {
        palette_tracer_core::ir::Point::new(f64::from(self.x), f64::from(self.y))
    }
}

/// A label plane plus the exterior convention (PTE-TOPO-007).
///
/// Reading outside the domain returns [`Labels::EXTERIOR`], which is a real
/// face, not an absence. That is what lets a domain-border segment have a twin
/// and a face on both sides like any other (PTE-TOPO-003).
#[derive(Debug, Clone, Copy)]
pub struct Labels<'a> {
    plane: &'a LabelPlane,
}

impl<'a> Labels<'a> {
    /// The exterior label.
    ///
    /// `u32::MAX` rather than a value stored in the plane: the plane holds
    /// in-domain labels only, so no image can collide with it, and the `u8`
    /// storage tier does not lose a code point to it.
    pub const EXTERIOR: LabelId = LabelId(u32::MAX);

    /// Wrap a plane.
    #[must_use]
    pub const fn new(plane: &'a LabelPlane) -> Self {
        Self { plane }
    }

    /// Image width in pixels.
    #[must_use]
    pub const fn width(&self) -> u32 {
        self.plane.width()
    }

    /// Image height in pixels.
    #[must_use]
    pub const fn height(&self) -> u32 {
        self.plane.height()
    }

    /// Label of pixel `(x, y)` in *signed* coordinates, or the exterior.
    #[must_use]
    pub fn at(&self, x: i64, y: i64) -> LabelId {
        if x < 0
            || y < 0
            || x >= i64::from(self.plane.width())
            || y >= i64::from(self.plane.height())
        {
            Self::EXTERIOR
        } else {
            self.plane.get(x as u32, y as u32)
        }
    }

    /// The four pixels around grid vertex `v`, as
    /// `(north-west, north-east, south-west, south-east)`.
    #[must_use]
    pub fn around(&self, v: GridPoint) -> [LabelId; 4] {
        let x = i64::from(v.x);
        let y = i64::from(v.y);
        [
            self.at(x - 1, y - 1),
            self.at(x, y - 1),
            self.at(x - 1, y),
            self.at(x, y),
        ]
    }

    /// Whether the elementary segment leaving `v` in `dir` exists, and the
    /// labels on either side of it.
    ///
    /// Returns `(left, right)` relative to travelling in `dir`.
    #[must_use]
    pub fn stub(&self, v: GridPoint, dir: Dir) -> Option<(LabelId, LabelId)> {
        let [nw, ne, sw, se] = self.around(v);
        // The segment leaving `v` in `dir` separates the two pixels flanking
        // that direction. Left and right follow the convention in the module
        // documentation.
        let (left, right) = match dir {
            // Going north from `v`: the pixels north-west and north-east of
            // `v` flank it, and west is on the left.
            Dir::North => (nw, ne),
            // Going east: north is on the left.
            Dir::East => (ne, se),
            // Going south: east is on the left.
            Dir::South => (se, sw),
            // Going west: south is on the left.
            Dir::West => (sw, nw),
        };
        (left != right).then_some((left, right))
    }

    /// Number of elementary segments incident to `v`.
    #[must_use]
    pub fn degree(&self, v: GridPoint) -> u8 {
        Dir::ALL
            .iter()
            .filter(|&&d| self.stub(v, d).is_some())
            .count() as u8
    }
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

    fn plane(labels: &[u32], width: u32, height: u32) -> LabelPlane {
        let mut p = LabelPlane::new(width, height, 256).unwrap();
        for (i, &l) in labels.iter().enumerate() {
            p.set_index(i, LabelId(l));
        }
        p
    }

    #[test]
    fn directions_are_clockwise_on_screen() {
        assert_eq!(Dir::North.rotate(1), Dir::East);
        assert_eq!(Dir::East.rotate(1), Dir::South);
        assert_eq!(Dir::South.rotate(1), Dir::West);
        assert_eq!(Dir::West.rotate(1), Dir::North);
        assert_eq!(Dir::North.opposite(), Dir::South);
    }

    /// The convention that everything else depends on: travelling east, north
    /// is on the left.
    #[test]
    fn the_left_side_follows_the_stated_convention() {
        // A 2x2 with all four labels distinct, so every stub exists and every
        // side is identifiable.
        let p = plane(&[10, 11, 12, 13], 2, 2);
        let labels = Labels::new(&p);
        let centre = GridPoint::new(1, 1);
        assert_eq!(
            labels.around(centre),
            [LabelId(10), LabelId(11), LabelId(12), LabelId(13)]
        );

        // Going east from the centre: north-east is on the left.
        assert_eq!(
            labels.stub(centre, Dir::East),
            Some((LabelId(11), LabelId(13)))
        );
        // Going south: south-east is on the left.
        assert_eq!(
            labels.stub(centre, Dir::South),
            Some((LabelId(13), LabelId(12)))
        );
        // Going west: south-west is on the left.
        assert_eq!(
            labels.stub(centre, Dir::West),
            Some((LabelId(12), LabelId(10)))
        );
        // Going north: north-west is on the left.
        assert_eq!(
            labels.stub(centre, Dir::North),
            Some((LabelId(10), LabelId(11)))
        );
    }

    /// PTE-TOPO-007: outside the domain is a face, so a border segment has a
    /// neighbour and can have a twin.
    #[test]
    fn the_exterior_is_a_real_label_on_every_border() {
        let p = plane(&[7], 1, 1);
        let labels = Labels::new(&p);
        assert_eq!(labels.at(-1, 0), Labels::EXTERIOR);
        assert_eq!(labels.at(0, 0), LabelId(7));
        assert_eq!(labels.at(1, 0), Labels::EXTERIOR);

        // Every corner of a one-pixel image is a degree-2 vertex, and every
        // side has a segment.
        for v in [
            GridPoint::new(0, 0),
            GridPoint::new(1, 0),
            GridPoint::new(0, 1),
            GridPoint::new(1, 1),
        ] {
            assert_eq!(labels.degree(v), 2, "at {v:?}");
        }
    }

    #[test]
    fn a_uniform_image_has_no_interior_segments() {
        let p = plane(&[3; 9], 3, 3);
        let labels = Labels::new(&p);
        assert_eq!(labels.degree(GridPoint::new(1, 1)), 0);
        assert_eq!(labels.degree(GridPoint::new(0, 0)), 2, "the corner");
        assert_eq!(labels.degree(GridPoint::new(1, 0)), 2, "the top edge");
    }

    /// A four-label junction is the genuine degree-four case.
    #[test]
    fn four_distinct_labels_meet_at_a_degree_four_vertex() {
        let p = plane(&[1, 2, 3, 4], 2, 2);
        let labels = Labels::new(&p);
        assert_eq!(labels.degree(GridPoint::new(1, 1)), 4);
    }

    /// A checkerboard corner is also degree four, and is the ambiguous case.
    #[test]
    fn a_checkerboard_corner_is_degree_four() {
        let p = plane(&[1, 2, 2, 1], 2, 2);
        let labels = Labels::new(&p);
        assert_eq!(labels.degree(GridPoint::new(1, 1)), 4);
    }

    #[test]
    fn stepping_off_the_grid_is_refused() {
        let v = GridPoint::new(0, 0);
        assert_eq!(v.step(Dir::North, 4, 4), None);
        assert_eq!(v.step(Dir::West, 4, 4), None);
        assert_eq!(v.step(Dir::East, 4, 4), Some(GridPoint::new(1, 0)));
        assert_eq!(GridPoint::new(4, 4).step(Dir::East, 4, 4), None);
    }
}
