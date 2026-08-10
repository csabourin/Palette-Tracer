//! The label plane: one owner per pixel.
//!
//! This is the single most memory-sensitive structure in the engine, and the
//! one that makes §19.4's first two forbidden shapes unrepresentable. There is
//! no per-palette-entry mask type anywhere in the workspace, so "one
//! full-resolution binary mask per palette entry" cannot be written; and there
//! is no per-pixel-per-entry buffer, so an `N x K` distance matrix cannot be
//! written either. Assignment keeps a running winner in *this* plane
//! (PTE-COLOR-015, PTE-NO-004).
//!
//! # Storage width
//!
//! §7.5 recommends `u8` when the internal label set fits in 255 entries and
//! `u16` otherwise. That covers palettes but not regions: a 512x512
//! checkerboard has 262144 single-pixel regions, so a `u32` tier is required
//! for the adversarial corpus of §24.3. The width is chosen once from a
//! declared upper bound and recorded in the report; it never changes under a
//! plane's feet, so the choice cannot depend on the order pixels arrive in.
//!
//! # Exterior
//!
//! The plane holds in-domain labels only. Everything outside `[0, width) x
//! [0, height)` is the exterior face, which PTE-TOPO-007 requires to be
//! explicit -- it is explicit in the topology graph, not as a sentinel here,
//! because a sentinel would waste a code point in the `u8` tier for every
//! image that does not need one.

use crate::checked::{pixel_count, u32_to_usize};
use crate::error::DecodeError;
use serde::{Deserialize, Serialize};

/// An internal label: a palette entry during assignment, a region afterwards.
///
/// Deliberately not `PaletteId` or `RegionId` -- the plane is reused across
/// stages and the meaning of its contents changes with the stage. Stages name
/// the meaning in their own signatures.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct LabelId(pub u32);

impl LabelId {
    /// Label zero, the conventional first entry.
    pub const ZERO: Self = Self(0);

    /// The raw index.
    #[must_use]
    pub const fn index(self) -> usize {
        self.0 as usize
    }
}

/// How wide each label is stored.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum LabelWidth {
    /// One byte per pixel; up to 256 distinct labels.
    U8,
    /// Two bytes per pixel; up to 65536 distinct labels.
    U16,
    /// Four bytes per pixel; the adversarial tier.
    U32,
}

impl LabelWidth {
    /// The narrowest width that can hold `label_count` distinct labels.
    #[must_use]
    pub const fn for_count(label_count: u64) -> Self {
        if label_count <= 256 {
            Self::U8
        } else if label_count <= 65_536 {
            Self::U16
        } else {
            Self::U32
        }
    }

    /// Bytes one label occupies.
    #[must_use]
    pub const fn bytes(self) -> usize {
        match self {
            Self::U8 => 1,
            Self::U16 => 2,
            Self::U32 => 4,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Storage {
    U8(Vec<u8>),
    U16(Vec<u16>),
    U32(Vec<u32>),
}

/// A dense, exclusive owner map over the image domain.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LabelPlane {
    width: u32,
    height: u32,
    storage: Storage,
}

impl LabelPlane {
    /// A plane of `width x height` filled with [`LabelId::ZERO`], sized for
    /// `label_count` distinct labels.
    ///
    /// # Errors
    ///
    /// [`DecodeError::EmptyImage`] or [`DecodeError::DimensionOverflow`]. A
    /// caller holding a validated [`crate::ImageView`] cannot hit either, but
    /// the allocation is checked rather than asserted so that a 32-bit target
    /// gets an error instead of a panic (PTE-SEC-005).
    pub fn new(width: u32, height: u32, label_count: u64) -> Result<Self, DecodeError> {
        let n = pixel_count(width, height)?;
        let storage = match LabelWidth::for_count(label_count) {
            LabelWidth::U8 => Storage::U8(vec![0u8; n]),
            LabelWidth::U16 => Storage::U16(vec![0u16; n]),
            LabelWidth::U32 => Storage::U32(vec![0u32; n]),
        };
        Ok(Self {
            width,
            height,
            storage,
        })
    }

    /// Width in pixels.
    #[must_use]
    pub const fn width(&self) -> u32 {
        self.width
    }

    /// Height in pixels.
    #[must_use]
    pub const fn height(&self) -> u32 {
        self.height
    }

    /// Number of pixels.
    #[must_use]
    pub fn len(&self) -> usize {
        match &self.storage {
            Storage::U8(v) => v.len(),
            Storage::U16(v) => v.len(),
            Storage::U32(v) => v.len(),
        }
    }

    /// Whether the plane is empty. Never true for a plane built from a
    /// validated image, which cannot have a zero dimension.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Storage width, for the report.
    #[must_use]
    pub const fn label_width(&self) -> LabelWidth {
        match &self.storage {
            Storage::U8(_) => LabelWidth::U8,
            Storage::U16(_) => LabelWidth::U16,
            Storage::U32(_) => LabelWidth::U32,
        }
    }

    /// Bytes this plane occupies, for the §19.2 budget.
    #[must_use]
    pub fn bytes(&self) -> u64 {
        crate::checked::usize_to_u64(self.len() * self.label_width().bytes())
    }

    /// Row-major index of `(x, y)`.
    #[inline]
    #[must_use]
    pub fn index(&self, x: u32, y: u32) -> usize {
        u32_to_usize(y) * u32_to_usize(self.width) + u32_to_usize(x)
    }

    /// Whether `(x, y)` is inside the domain.
    #[inline]
    #[must_use]
    pub const fn contains(&self, x: u32, y: u32) -> bool {
        x < self.width && y < self.height
    }

    /// Label at flat index `i`.
    ///
    /// # Panics
    ///
    /// If `i` is out of range.
    #[inline]
    #[must_use]
    pub fn get_index(&self, i: usize) -> LabelId {
        match &self.storage {
            Storage::U8(v) => LabelId(u32::from(v[i])),
            Storage::U16(v) => LabelId(u32::from(v[i])),
            Storage::U32(v) => LabelId(v[i]),
        }
    }

    /// Label at `(x, y)`.
    ///
    /// # Panics
    ///
    /// If `(x, y)` is outside the domain.
    #[inline]
    #[must_use]
    pub fn get(&self, x: u32, y: u32) -> LabelId {
        self.get_index(self.index(x, y))
    }

    /// Set the label at flat index `i`.
    ///
    /// Values wider than the storage tier are truncated, which would be a
    /// silent corruption. Debug builds assert instead; release builds saturate
    /// so the plane stays internally consistent. The tier is chosen from a
    /// declared bound at construction, so exceeding it is an engine defect.
    ///
    /// # Panics
    ///
    /// In debug builds, if `label` does not fit the storage tier.
    #[inline]
    pub fn set_index(&mut self, i: usize, label: LabelId) {
        match &mut self.storage {
            Storage::U8(v) => {
                debug_assert!(
                    label.0 <= u32::from(u8::MAX),
                    "label {label:?} exceeds u8 tier"
                );
                v[i] = label.0.min(u32::from(u8::MAX)) as u8;
            }
            Storage::U16(v) => {
                debug_assert!(
                    label.0 <= u32::from(u16::MAX),
                    "label {label:?} exceeds u16 tier"
                );
                v[i] = label.0.min(u32::from(u16::MAX)) as u16;
            }
            Storage::U32(v) => v[i] = label.0,
        }
    }

    /// Set the label at `(x, y)`.
    ///
    /// # Panics
    ///
    /// If `(x, y)` is outside the domain, or in debug builds if `label` does
    /// not fit the storage tier.
    #[inline]
    pub fn set(&mut self, x: u32, y: u32, label: LabelId) {
        let i = self.index(x, y);
        self.set_index(i, label);
    }

    /// Iterate labels in row-major order.
    ///
    /// Row-major is the canonical traversal for every stage that derives IDs
    /// from raster position (PTE-API-010).
    pub fn iter(&self) -> impl Iterator<Item = LabelId> + '_ {
        (0..self.len()).map(move |i| self.get_index(i))
    }

    /// Rewrite every label through `map`, in place.
    ///
    /// Used to apply a canonical relabelling after merging, where `map` is
    /// indexed by the old label.
    ///
    /// # Panics
    ///
    /// If any stored label is outside `map`.
    pub fn remap(&mut self, map: &[LabelId]) {
        for i in 0..self.len() {
            let old = self.get_index(i);
            self.set_index(i, map[old.index()]);
        }
    }

    /// Count how many pixels carry each label, given `label_count` labels.
    ///
    /// `O(N)` time and `O(label_count)` space -- never `O(N x K)`
    /// (PTE-NO-004).
    ///
    /// # Panics
    ///
    /// If any stored label is at or above `label_count`.
    #[must_use]
    pub fn histogram(&self, label_count: usize) -> Vec<u64> {
        let mut counts = vec![0u64; label_count];
        for i in 0..self.len() {
            counts[self.get_index(i).index()] += 1;
        }
        counts
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

    /// §7.5's storage rule, plus the `u32` tier the adversarial corpus needs.
    #[test]
    fn storage_width_follows_the_label_count() {
        assert_eq!(LabelWidth::for_count(2), LabelWidth::U8);
        assert_eq!(LabelWidth::for_count(256), LabelWidth::U8);
        assert_eq!(LabelWidth::for_count(257), LabelWidth::U16);
        assert_eq!(LabelWidth::for_count(65_536), LabelWidth::U16);
        assert_eq!(LabelWidth::for_count(65_537), LabelWidth::U32);
        assert_eq!(LabelWidth::for_count(262_144), LabelWidth::U32);
    }

    /// The transition §25.4 asks for: the same labels must survive a change of
    /// storage tier.
    #[test]
    fn the_u8_to_u16_transition_preserves_values() {
        for count in [200u64, 300] {
            let mut plane = LabelPlane::new(4, 2, count).unwrap();
            for i in 0..8 {
                plane.set_index(i, LabelId(i as u32 * 25));
            }
            let read: Vec<u32> = (0..8).map(|i| plane.get_index(i).0).collect();
            assert_eq!(read, vec![0, 25, 50, 75, 100, 125, 150, 175]);
        }
    }

    #[test]
    fn a_wide_label_survives_the_u32_tier() {
        let mut plane = LabelPlane::new(2, 1, 300_000).unwrap();
        assert_eq!(plane.label_width(), LabelWidth::U32);
        plane.set(1, 0, LabelId(262_143));
        assert_eq!(plane.get(1, 0), LabelId(262_143));
    }

    #[test]
    fn indexing_is_row_major() {
        let mut plane = LabelPlane::new(3, 2, 8).unwrap();
        plane.set(2, 1, LabelId(7));
        assert_eq!(plane.index(2, 1), 5);
        assert_eq!(plane.get_index(5), LabelId(7));
    }

    #[test]
    fn bytes_reflect_the_chosen_tier() {
        assert_eq!(LabelPlane::new(100, 100, 10).unwrap().bytes(), 10_000);
        assert_eq!(LabelPlane::new(100, 100, 1000).unwrap().bytes(), 20_000);
        assert_eq!(LabelPlane::new(100, 100, 100_000).unwrap().bytes(), 40_000);
    }

    #[test]
    fn remap_applies_a_canonical_relabelling() {
        let mut plane = LabelPlane::new(2, 2, 4).unwrap();
        plane.set_index(0, LabelId(3));
        plane.set_index(1, LabelId(1));
        plane.set_index(2, LabelId(3));
        plane.set_index(3, LabelId(0));
        // 3 -> 0, 1 -> 1, 0 -> 2
        plane.remap(&[LabelId(2), LabelId(1), LabelId(9), LabelId(0)]);
        let read: Vec<u32> = plane.iter().map(|l| l.0).collect();
        assert_eq!(read, vec![0, 1, 0, 2]);
    }

    #[test]
    fn histogram_is_linear_in_pixels_and_counts_every_one() {
        let mut plane = LabelPlane::new(4, 4, 3).unwrap();
        for i in 0..16 {
            plane.set_index(i, LabelId((i % 3) as u32));
        }
        let h = plane.histogram(3);
        assert_eq!(h.iter().sum::<u64>(), 16);
        assert_eq!(h, vec![6, 5, 5]);
    }
}
