//! Deterministic ordering primitives.
//!
//! §20.2 lists the sources of nondeterminism the engine must eliminate. Three
//! of them -- unstable sorting of equal keys, hash-map iteration order, and
//! target-specific floating point crossing a decision threshold -- are
//! prevented here rather than by convention:
//!
//! * [`QuantKey`] turns a `f64` into an integer decision key, so a difference
//!   of one ulp between native and WASM cannot flip a branch (PTE-DET-003).
//! * [`MinHeap`] pops in a total order defined entirely by the entry type, so
//!   ties are broken by the caller's declared tuple and never by insertion
//!   order (PTE-SEG-014).
//! * [`Generation`] tags let a priority queue hold stale entries and detect
//!   them on pop instead of requiring decrease-key (PTE-SEG-013).
//!
//! Nothing in the workspace iterates a `HashMap` in a path that affects
//! output; `BTreeMap` and sorted `Vec` are the only ordered containers
//! (PTE-NO-043).

use crate::error::NumericalError;
use core::cmp::{Ordering, Reverse};
use std::collections::BinaryHeap;

/// Quantiser scale for merge costs and other unbounded energies.
///
/// Chosen so that a cost difference below `1e-9` cannot decide a merge. §8.5
/// costs are variance-like quantities in OKLab units; OKLab `L` spans `[0, 1]`,
/// so `1e-9` is nine orders of magnitude below any perceptually meaningful
/// difference and far below the noise in any real raster.
pub const COST_SCALE: f64 = 1.0e9;

/// Quantiser scale for geometry expressed in source pixels.
///
/// `1e6` gives a decision resolution of one millionth of a pixel, six orders
/// below the tightest gate in §31.2 (0.10 px median normal error).
pub const GEOMETRY_SCALE: f64 = 1.0e6;

/// An integer key derived from a `f64`, safe to compare and to hash.
///
/// Two values that quantise to the same key are *by definition* tied, and the
/// caller's declared tie-break applies. This is the mechanism PTE-DET-003 asks
/// for: "decision thresholds SHOULD use fixed-point/quantized keys".
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct QuantKey(i64);

impl QuantKey {
    /// Quantise `value` at `scale`, rejecting NaN and infinity.
    ///
    /// Saturates rather than wrapping if the scaled magnitude exceeds `i64`,
    /// which keeps the ordering total for extreme but finite inputs.
    ///
    /// # Errors
    ///
    /// [`NumericalError::NonFinite`] if `value` is NaN or infinite
    /// (PTE-DET-002 requires NaN rejection to be defined, not incidental).
    pub fn new(value: f64, scale: f64, context: &'static str) -> Result<Self, NumericalError> {
        if !value.is_finite() {
            return Err(NumericalError::NonFinite { context });
        }
        let scaled = (value * scale).round();
        // `i64::MAX as f64` rounds up, so compare against the exact power of
        // two below it to keep the cast in range on every rounding mode.
        const LIMIT: f64 = 9.223_372_036_854_775e18;
        let clamped = scaled.clamp(-LIMIT, LIMIT);
        Ok(Self(clamped as i64))
    }

    /// Quantise a merge cost or energy at [`COST_SCALE`].
    ///
    /// # Errors
    ///
    /// As [`QuantKey::new`].
    pub fn cost(value: f64) -> Result<Self, NumericalError> {
        Self::new(value, COST_SCALE, "cost quantisation")
    }

    /// Quantise a length in source pixels at [`GEOMETRY_SCALE`].
    ///
    /// # Errors
    ///
    /// As [`QuantKey::new`].
    pub fn geometry(value: f64) -> Result<Self, NumericalError> {
        Self::new(value, GEOMETRY_SCALE, "geometry quantisation")
    }

    /// Quantise, mapping a non-finite input to the largest key.
    ///
    /// For "worst possible candidate" positions in a minimisation, where an
    /// unusable value should lose rather than abort.
    #[must_use]
    pub fn cost_or_worst(value: f64) -> Self {
        Self::cost(value).unwrap_or(Self(i64::MAX))
    }

    /// The raw quantised integer, for digest canonicalisation (Appendix F.3).
    #[must_use]
    pub const fn raw(self) -> i64 {
        self.0
    }
}

/// Total order over `f64` that rejects nothing and orders NaN last.
///
/// Uses [`f64::total_cmp`], which is the IEEE-754 total order: `-0.0` sorts
/// below `+0.0` and NaN sorts outside the finite range. Use this for *sorting*
/// where every input must appear in the output; use [`QuantKey`] where the
/// comparison *decides* something (PTE-DET-002).
#[inline]
#[must_use]
pub fn total_cmp(a: f64, b: f64) -> Ordering {
    a.total_cmp(&b)
}

/// A monotonically increasing version stamp for a mutable graph node.
///
/// Appendix D.2 pushes merge candidates into a heap and never removes them.
/// When a region is merged, its generation advances, so any queued entry
/// naming the old generation is recognised as stale on pop. This replaces
/// decrease-key, which would need an index whose iteration order could leak
/// into the result.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct Generation(pub u32);

impl Generation {
    /// Advance this generation, invalidating every entry that recorded the old
    /// value.
    ///
    /// Saturates at `u32::MAX`. A region that has been touched four billion
    /// times has other problems, and saturating keeps the operation total.
    #[inline]
    pub fn bump(&mut self) {
        self.0 = self.0.saturating_add(1);
    }
}

/// A minimum-first priority queue whose pop order is fully determined by `T`.
///
/// `BinaryHeap` is a max-heap and its order among equal elements is
/// unspecified, which is exactly the nondeterminism §20.2 forbids. Wrapping
/// entries in [`Reverse`] makes it minimum-first; requiring `T: Ord` and
/// documenting that `T` must include its own tie-break makes the pop order a
/// property of the caller's key, not of the heap's internals.
///
/// The declared tie-break for region merging is
/// `(cost_key, min_region_key, max_region_key)` (PTE-SEG-014).
#[derive(Debug, Clone)]
pub struct MinHeap<T: Ord> {
    inner: BinaryHeap<Reverse<T>>,
}

impl<T: Ord> Default for MinHeap<T> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T: Ord> MinHeap<T> {
    /// An empty heap.
    #[must_use]
    pub fn new() -> Self {
        Self {
            inner: BinaryHeap::new(),
        }
    }

    /// An empty heap with room for `capacity` entries.
    ///
    /// Callers reserve from a checked estimate, never from an input-derived
    /// number they have not bounded (§35.4).
    #[must_use]
    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            inner: BinaryHeap::with_capacity(capacity),
        }
    }

    /// Insert an entry.
    pub fn push(&mut self, entry: T) {
        self.inner.push(Reverse(entry));
    }

    /// Remove and return the minimum entry.
    pub fn pop(&mut self) -> Option<T> {
        self.inner.pop().map(|Reverse(entry)| entry)
    }

    /// Number of queued entries, including ones that are stale.
    #[must_use]
    pub fn len(&self) -> usize {
        self.inner.len()
    }

    /// Whether the heap holds no entries.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.inner.is_empty()
    }
}

/// Order an unordered pair so `(a, b)` and `(b, a)` canonicalise identically.
///
/// PTE-TOPO-* and PTE-SEG-014 both need the interface between two regions to
/// have one name regardless of which side asked.
#[inline]
#[must_use]
pub fn canonical_pair<T: Ord>(a: T, b: T) -> (T, T) {
    if a <= b { (a, b) } else { (b, a) }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::float_cmp, clippy::field_reassign_with_default)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    #[test]
    fn nan_and_infinity_are_rejected_not_ordered() {
        assert!(QuantKey::cost(f64::NAN).is_err());
        assert!(QuantKey::cost(f64::INFINITY).is_err());
        assert!(QuantKey::cost(f64::NEG_INFINITY).is_err());
    }

    /// The point of quantisation: a difference far below any gate cannot
    /// decide anything, so native and WASM cannot disagree about it.
    #[test]
    fn differences_below_the_scale_resolution_tie() {
        let a = QuantKey::cost(0.5).unwrap();
        let b = QuantKey::cost(0.5 + 1e-12).unwrap();
        assert_eq!(a, b, "1e-12 is below COST_SCALE resolution and must tie");

        let c = QuantKey::cost(0.5 + 1e-8).unwrap();
        assert_ne!(a, c, "1e-8 is above the resolution and must separate");
    }

    #[test]
    fn negative_zero_and_zero_quantise_identically() {
        assert_eq!(QuantKey::cost(-0.0).unwrap(), QuantKey::cost(0.0).unwrap());
    }

    #[test]
    fn extreme_finite_values_saturate_rather_than_wrap() {
        let big = QuantKey::cost(f64::MAX).unwrap();
        let bigger_still = QuantKey::cost(f64::MAX / 2.0).unwrap();
        assert!(big >= bigger_still, "saturation must preserve order");
        assert!(big.raw() > 0, "saturation must not wrap to negative");
    }

    /// The heap contract: pop order is a function of the entry type alone, so
    /// two runs that push in different orders pop identically.
    #[test]
    fn pop_order_is_independent_of_push_order() {
        let entries = [(3u32, 'a'), (1, 'b'), (2, 'c'), (1, 'a')];

        let mut forward = MinHeap::new();
        for e in entries {
            forward.push(e);
        }
        let mut backward = MinHeap::new();
        for e in entries.iter().rev() {
            backward.push(*e);
        }

        let drain = |mut h: MinHeap<(u32, char)>| {
            let mut out = Vec::new();
            while let Some(e) = h.pop() {
                out.push(e);
            }
            out
        };
        let a = drain(forward);
        assert_eq!(a, drain(backward));
        assert_eq!(a, vec![(1, 'a'), (1, 'b'), (2, 'c'), (3, 'a')]);
    }

    #[test]
    fn generations_invalidate_stale_entries() {
        let mut g = Generation::default();
        let recorded = g;
        g.bump();
        assert_ne!(recorded, g, "a bumped generation must not match a stale one");
    }

    #[test]
    fn canonical_pair_is_symmetric() {
        assert_eq!(canonical_pair(7u32, 3), canonical_pair(3u32, 7));
    }

    proptest! {
        /// Quantisation must be monotone: it may merge distinct inputs into one
        /// key, but it must never reverse their order.
        #[test]
        fn quantisation_is_monotone(a in -1e6f64..1e6, b in -1e6f64..1e6) {
            let ka = QuantKey::cost(a).unwrap();
            let kb = QuantKey::cost(b).unwrap();
            if a < b {
                prop_assert!(ka <= kb);
            } else if a > b {
                prop_assert!(ka >= kb);
            }
        }

        #[test]
        fn heap_drains_in_sorted_order(mut values in prop::collection::vec(any::<i32>(), 0..64)) {
            let mut heap = MinHeap::new();
            for v in &values {
                heap.push(*v);
            }
            let mut drained = Vec::new();
            while let Some(v) = heap.pop() {
                drained.push(v);
            }
            values.sort_unstable();
            prop_assert_eq!(drained, values);
        }
    }
}
