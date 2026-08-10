//! Deterministic automatic palette generation (§7.6).
//!
//! Four requirements shape the algorithm:
//!
//! * **PTE-COLOR-018**: deterministic, weighted by pixel frequency after the
//!   alpha policy.
//! * **PTE-COLOR-019**: pinned colours never move; automatic centres come from
//!   deterministic weighted farthest-point selection, refined by weighted
//!   Lloyd iterations in OKLab.
//! * **PTE-COLOR-020**: empty clusters are repaired by the deterministically
//!   worst-represented eligible sample, with stable ties.
//! * **PTE-COLOR-022 / PTE-NO-005**: no circular averaging of hue. Centroids
//!   are Cartesian OKLab throughout; OKLCH appears only in the reach score.
//!
//! **PTE-COLOR-021** is a boundary, not an algorithm: what comes out of here
//! is an appearance model, not a segmentation. Nothing downstream may treat a
//! nearest-centroid label map as a set of regions.
//!
//! # Determinism
//!
//! There is no random seed anywhere. The first centre is the heaviest bucket,
//! ties broken by the bucket's canonical key; each subsequent centre is the
//! bucket maximising `weight x distance²` to the nearest existing centre, with
//! the same tie rule. Two runs on the same histogram produce the same palette
//! in the same order, on any target.
//!
//! # Provenance
//!
//! Weighted farthest-point initialisation and Lloyd refinement are textbook.
//! Written from §7.6; no implementation consulted (PTE-LIC-004).

use crate::alpha::PremulLinearRgba;
use crate::palette::{Palette, PaletteEntry};
use crate::sampler::sample_premultiplied;
use crate::spaces::{EncodedSrgb, LinearRgb, Oklab};
use palette_tracer_core::config::{EffectiveColor, ReachSpec};
use palette_tracer_core::control::{TraceControl, check_cancel};
use palette_tracer_core::determinism::QuantKey;
use palette_tracer_core::error::TraceError;
use palette_tracer_core::image::ImageView;
use palette_tracer_core::ir::Color;
use palette_tracer_core::limits::{WorkBudget, cost};

/// Bits kept per channel when building the colour histogram.
///
/// Five bits gives 32768 buckets: fine enough that two colours a human would
/// call different land in different buckets, coarse enough that the histogram
/// is a fixed 32768 entries regardless of image size. Bounded by construction,
/// which is what keeps §19.4's "unbounded memoization keyed by input colors"
/// off the table.
const HISTOGRAM_BITS: u32 = 5;
const HISTOGRAM_SIDE: usize = 1 << HISTOGRAM_BITS;
const HISTOGRAM_SLOTS: usize = HISTOGRAM_SIDE * HISTOGRAM_SIDE * HISTOGRAM_SIDE;

/// Lloyd iterations. Bounded because PTE-SEC-010 requires bounded scaling on
/// adversarial input and §19.1 gives no allowance for an unbounded loop.
const MAX_LLOYD_ITERATIONS: u32 = 24;

/// Convergence threshold: stop when no centre moved further than this in
/// OKLab. One thousandth of the `L` range is well below any perceptible
/// difference, so further iterations cannot change the visible result.
const CONVERGENCE_DELTA: f64 = 1e-3;

/// A weighted colour observation: one histogram bucket.
#[derive(Debug, Clone, Copy)]
struct Bucket {
    /// Bucket index, used as the canonical tie-break key.
    key: u32,
    /// Mean colour of the pixels in the bucket, in Cartesian OKLab.
    color: Oklab,
    /// How many pixels landed here.
    weight: f64,
}

/// A bounded, deterministic histogram of the image's colours.
///
/// `O(1)` memory in the image size, accumulated in one linear pass
/// (PTE-COLOR-018's "weighted by pixel/region frequency after alpha policy").
#[derive(Debug)]
pub struct ColorHistogram {
    sums: Vec<[f64; 3]>,
    counts: Vec<u64>,
    total: u64,
}

impl Default for ColorHistogram {
    fn default() -> Self {
        Self::new()
    }
}

impl ColorHistogram {
    /// An empty histogram.
    #[must_use]
    pub fn new() -> Self {
        Self {
            sums: vec![[0.0; 3]; HISTOGRAM_SLOTS],
            counts: vec![0; HISTOGRAM_SLOTS],
            total: 0,
        }
    }

    fn slot(color: LinearRgb) -> usize {
        let encoded = color.to_encoded();
        let q = |v: f64| -> usize {
            let scaled = (v.clamp(0.0, 1.0) * f64::from((1u32 << HISTOGRAM_BITS) - 1)).round();
            scaled as usize
        };
        // Quantise in the encoded domain: perceptual spacing there is far more
        // even than in linear light, so the buckets are more evenly loaded.
        q(encoded.r) * HISTOGRAM_SIDE * HISTOGRAM_SIDE
            + q(encoded.g) * HISTOGRAM_SIDE
            + q(encoded.b)
    }

    /// Add one observation.
    pub fn add(&mut self, color: LinearRgb) {
        let slot = Self::slot(color);
        let lab = color.to_oklab();
        self.sums[slot][0] += lab.l;
        self.sums[slot][1] += lab.a;
        self.sums[slot][2] += lab.b;
        self.counts[slot] += 1;
        self.total += 1;
    }

    /// Total observations.
    #[must_use]
    pub fn total(&self) -> u64 {
        self.total
    }

    /// Non-empty buckets, in canonical index order.
    ///
    /// Index order, not weight order: two buckets with equal weight must have
    /// a stable relative position, and the index is the only key that is a
    /// property of the colour rather than of the image.
    fn buckets(&self) -> Vec<Bucket> {
        let mut out = Vec::new();
        for (slot, &count) in self.counts.iter().enumerate() {
            if count == 0 {
                continue;
            }
            let n = count as f64;
            let s = self.sums[slot];
            out.push(Bucket {
                key: slot as u32,
                color: Oklab::new(s[0] / n, s[1] / n, s[2] / n),
                weight: n,
            });
        }
        out
    }
}

/// Build a histogram from an image, honouring the alpha policy.
///
/// # Errors
///
/// [`TraceError::Cancelled`] or [`TraceError::ResourceLimit`].
pub fn histogram(
    view: &ImageView<'_>,
    color: &EffectiveColor,
    budget: &WorkBudget,
    control: &dyn TraceControl,
) -> Result<ColorHistogram, TraceError> {
    let mut h = ColorHistogram::new();
    let epsilon = color.alpha.unpremultiply_epsilon.get();
    let mut i = 0usize;
    for y in 0..view.height() {
        for x in 0..view.width() {
            check_cancel(control, i)?;
            budget.charge(cost::PIXEL)?;
            i += 1;

            let premul: PremulLinearRgba = sample_premultiplied(view, x, y);
            // PTE-COLOR-008: a pixel with no colour evidence contributes
            // nothing, so whatever is stored under it cannot move a centroid.
            if let Some(straight) = premul.to_straight(epsilon) {
                h.add(straight);
            }
        }
    }
    Ok(h)
}

/// Generate an automatic palette of at most `max_colors` entries (§7.6).
///
/// `pinned` entries are carried through untouched and are *not* moved by
/// refinement (PTE-COLOR-019); the automatic centres fill the remaining slots.
///
/// # Errors
///
/// [`TraceError::Cancelled`] or [`TraceError::ResourceLimit`].
///
/// # Panics
///
/// Never; every index is derived from a length checked in the same scope.
pub fn generate(
    histogram: &ColorHistogram,
    max_colors: u32,
    pinned: &[PaletteEntry],
    default_reach: ReachSpec,
    budget: &WorkBudget,
    control: &dyn TraceControl,
) -> Result<Palette, TraceError> {
    let buckets = histogram.buckets();
    let pinned_count = pinned.len();
    let wanted = (max_colors as usize).saturating_sub(pinned_count);

    if buckets.is_empty() || wanted == 0 {
        return Ok(Palette::from_entries(pinned.to_vec()));
    }

    let mut centers = farthest_point_init(&buckets, wanted.min(buckets.len()), pinned, control)?;
    lloyd_refine(&buckets, &mut centers, budget, control)?;

    // Centres are emitted in a canonical order -- by quantised OKLab, then by
    // the key of the bucket nearest to them -- so a permutation of the input
    // pixels cannot permute the output palette (PTE-COLOR-018).
    centers.sort_by_key(|c| {
        (
            QuantKey::cost_or_worst(c.l),
            QuantKey::cost_or_worst(c.a),
            QuantKey::cost_or_worst(c.b),
        )
    });

    let mut entries = pinned.to_vec();
    let mut next_id = pinned.iter().map(|e| e.id).max().map_or(0, |m| m + 1);
    for center in centers {
        let rgb = center.to_linear().clamped().to_encoded().to_u8();
        let mut entry =
            PaletteEntry::new(next_id, Color::rgb(rgb[0], rgb[1], rgb[2]), default_reach);
        // The anchor is the *unquantised* centroid, not the rounded output
        // colour: rounding to 8 bits is an output concern, and matching
        // against the rounded value would throw away accuracy the centroid
        // has (PTE-SVG-012 keeps the two separate in the other direction).
        entry.anchor = center;
        entry.anchor_lch = center.to_oklch();
        entries.push(entry);
        next_id += 1;
    }

    Ok(Palette::from_entries(entries))
}

/// Deterministic weighted farthest-point selection (PTE-COLOR-019).
fn farthest_point_init(
    buckets: &[Bucket],
    wanted: usize,
    pinned: &[PaletteEntry],
    control: &dyn TraceControl,
) -> Result<Vec<Oklab>, TraceError> {
    let mut centers: Vec<Oklab> = Vec::with_capacity(wanted);

    // Distance to the nearest centre, including the pinned entries -- a
    // pinned ink already covers its neighbourhood, so an automatic centre
    // should not be spent there.
    let mut nearest: Vec<f64> = buckets
        .iter()
        .map(|b| {
            pinned
                .iter()
                .map(|p| b.color.delta_e_squared(p.anchor))
                .fold(f64::INFINITY, f64::min)
        })
        .collect();

    for step in 0..wanted {
        check_cancel(control, step)?;

        let pick = if centers.is_empty() && pinned.is_empty() {
            // First centre with nothing pinned: the heaviest bucket. Ties go
            // to the lower bucket key, which is a property of the colour.
            best_by(buckets, |b, _| b.weight)
        } else {
            // Every later centre: maximise weight x distance², so a large area
            // of an unrepresented colour beats a single stray pixel far away.
            best_by(buckets, |b, i| b.weight * nearest[i])
        };

        let Some(index) = pick else { break };
        let chosen = buckets[index].color;
        centers.push(chosen);

        for (i, b) in buckets.iter().enumerate() {
            let d = b.color.delta_e_squared(chosen);
            if d < nearest[i] {
                nearest[i] = d;
            }
        }
    }

    Ok(centers)
}

/// Index of the bucket maximising `score`, ties going to the lower key.
fn best_by(buckets: &[Bucket], score: impl Fn(&Bucket, usize) -> f64) -> Option<usize> {
    let mut best: Option<(QuantKey, u32, usize)> = None;
    for (i, b) in buckets.iter().enumerate() {
        let s = score(b, i);
        if !s.is_finite() || s <= 0.0 {
            continue;
        }
        let key = QuantKey::cost_or_worst(-s); // negate so "smallest key" is "largest score"
        let candidate = (key, b.key, i);
        best = Some(match best {
            None => candidate,
            Some(current) => {
                if (candidate.0, candidate.1) < (current.0, current.1) {
                    candidate
                } else {
                    current
                }
            }
        });
    }
    best.map(|(_, _, i)| i)
}

/// Weighted Lloyd refinement in Cartesian OKLab (PTE-COLOR-019, PTE-NO-005).
fn lloyd_refine(
    buckets: &[Bucket],
    centers: &mut [Oklab],
    budget: &WorkBudget,
    control: &dyn TraceControl,
) -> Result<(), TraceError> {
    if centers.is_empty() {
        return Ok(());
    }
    let k = centers.len();

    for iteration in 0..MAX_LLOYD_ITERATIONS {
        check_cancel(control, iteration as usize)?;

        let mut sums = vec![Oklab::ZERO; k];
        let mut weights = vec![0.0f64; k];
        // Worst-represented bucket per empty-cluster repair (PTE-COLOR-020).
        let mut assigned_distance = vec![0.0f64; buckets.len()];
        let mut assigned_center = vec![0usize; buckets.len()];

        for (i, b) in buckets.iter().enumerate() {
            budget.charge(cost::PIXEL)?;
            let mut best = (QuantKey::cost_or_worst(f64::MAX), 0usize, f64::MAX);
            for (c, center) in centers.iter().enumerate() {
                let d = b.color.delta_e_squared(*center);
                let key = QuantKey::cost_or_worst(d);
                // Ties go to the lower centre index, which is stable because
                // centres are only ever appended in a deterministic order.
                if (key, c) < (best.0, best.1) {
                    best = (key, c, d);
                }
            }
            sums[best.1] = sums[best.1].plus(b.color.scale(b.weight));
            weights[best.1] += b.weight;
            assigned_distance[i] = best.2;
            assigned_center[i] = best.1;
        }

        // PTE-COLOR-020: repair an empty cluster with the deterministically
        // worst-represented eligible bucket -- the one furthest from the
        // centre it was assigned to, ties by bucket key.
        let mut moved: f64 = 0.0;
        for c in 0..k {
            if weights[c] > 0.0 {
                let updated = sums[c].scale(1.0 / weights[c]);
                moved = moved.max(updated.delta_e(centers[c]));
                centers[c] = updated;
                continue;
            }
            let worst = best_by(buckets, |b, i| {
                // Do not steal a bucket that is already somebody's only member.
                if weights[assigned_center[i]] <= b.weight {
                    0.0
                } else {
                    b.weight * assigned_distance[i]
                }
            });
            if let Some(i) = worst {
                moved = moved.max(buckets[i].color.delta_e(centers[c]));
                centers[c] = buckets[i].color;
            }
        }

        if moved <= CONVERGENCE_DELTA {
            break;
        }
    }
    Ok(())
}

/// The colour of a palette entry as an sRGB byte triple, for the report.
#[must_use]
pub fn entry_hex(entry: &PaletteEntry) -> String {
    entry.output.to_hex_rgb()
}

/// Convert bytes to an OKLab anchor, for tests and callers building palettes
/// by hand.
#[must_use]
pub fn anchor_of(rgb: [u8; 3]) -> Oklab {
    EncodedSrgb::from_u8(rgb[0], rgb[1], rgb[2])
        .to_linear()
        .to_oklab()
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
    use palette_tracer_core::config::{Profile, TraceConfig};
    use palette_tracer_core::control::NoControl;
    use palette_tracer_core::image::{AlphaMode, ColorEncoding, Orientation, PixelFormat};

    fn color_policy() -> EffectiveColor {
        Profile::expand(&TraceConfig::default()).unwrap().color
    }

    fn image_of(colors: &[[u8; 4]]) -> Vec<u8> {
        colors
            .iter()
            .flat_map(|c| c.iter().copied())
            .collect::<Vec<u8>>()
    }

    fn view<'a>(data: &'a [u8], w: u32, h: u32) -> ImageView<'a> {
        ImageView::new(
            w,
            h,
            0,
            PixelFormat::Rgba8,
            data,
            ColorEncoding::Srgb,
            AlphaMode::Straight,
            Orientation::Identity,
        )
        .unwrap()
    }

    fn hist_of(pixels: &[[u8; 4]]) -> ColorHistogram {
        let data = image_of(pixels);
        let v = view(&data, pixels.len() as u32, 1);
        histogram(&v, &color_policy(), &WorkBudget::unbounded(), &NoControl).unwrap()
    }

    fn generate_from(h: &ColorHistogram, k: u32) -> Palette {
        generate(
            h,
            k,
            &[],
            ReachSpec::default(),
            &WorkBudget::unbounded(),
            &NoControl,
        )
        .unwrap()
    }

    /// PTE-COLOR-018: deterministic. Two runs, and two runs with the pixels
    /// in a different order, must agree.
    #[test]
    fn generation_is_deterministic_and_order_independent() {
        let pixels = [
            [220, 30, 40, 255],
            [30, 40, 220, 255],
            [30, 40, 220, 255],
            [240, 240, 240, 255],
        ];
        let a = generate_from(&hist_of(&pixels), 3);
        let b = generate_from(&hist_of(&pixels), 3);
        assert_eq!(a, b);

        let mut shuffled = pixels;
        shuffled.reverse();
        let c = generate_from(&hist_of(&shuffled), 3);
        assert_eq!(
            a.entries().iter().map(|e| e.output).collect::<Vec<_>>(),
            c.entries().iter().map(|e| e.output).collect::<Vec<_>>()
        );
    }

    /// Three well-separated colours must come back as three entries near them.
    #[test]
    fn three_separated_colours_are_recovered() {
        let pixels = [[220, 30, 40, 255], [30, 40, 220, 255], [40, 200, 60, 255]];
        let palette = generate_from(&hist_of(&pixels), 3);
        assert_eq!(palette.len(), 3);

        for target in pixels {
            let anchor = anchor_of([target[0], target[1], target[2]]);
            let nearest = palette
                .entries()
                .iter()
                .map(|e| e.anchor.delta_e(anchor))
                .fold(f64::INFINITY, f64::min);
            assert!(
                nearest < 0.05,
                "no entry near {target:?}: nearest ΔE {nearest}"
            );
        }
    }

    /// PTE-COLOR-019: a pinned entry is carried through untouched.
    #[test]
    fn pinned_entries_never_move() {
        let mut pin = PaletteEntry::new(0, Color::rgb(255, 0, 0), ReachSpec::default());
        pin.pinned = true;
        let before = pin.clone();

        let pixels = [[30, 40, 220, 255], [40, 200, 60, 255]];
        let palette = generate(
            &hist_of(&pixels),
            3,
            core::slice::from_ref(&pin),
            ReachSpec::default(),
            &WorkBudget::unbounded(),
            &NoControl,
        )
        .unwrap();

        assert_eq!(palette.entries()[0], before);
        assert!(palette.entries()[0].pinned);
        assert_eq!(palette.len(), 3, "pinned entries count toward maxColors");
    }

    /// PTE-COLOR-008 again, at the histogram: transparent pixels contribute
    /// nothing, so a palette derived from them is derived from nothing.
    #[test]
    fn transparent_pixels_do_not_reach_the_histogram() {
        let hidden = hist_of(&[[255, 0, 255, 0], [0, 255, 255, 0]]);
        assert_eq!(hidden.total(), 0);
        assert!(generate_from(&hidden, 4).is_empty());

        // And a mix contributes only the visible pixels.
        let mixed = hist_of(&[[255, 0, 255, 0], [220, 30, 40, 255]]);
        assert_eq!(mixed.total(), 1);
    }

    /// PTE-COLOR-022 / PTE-NO-005: no circular hue averaging. Two colours
    /// whose hues straddle the ±π wrap must average to something between them
    /// in the a-b plane, not to the opposite hue.
    #[test]
    fn centroids_are_cartesian_and_survive_the_hue_wrap() {
        // Two reddish colours on either side of hue = π (the wrap).
        let a = anchor_of([200, 60, 70]);
        let b = anchor_of([200, 70, 60]);
        let mean = a.plus(b).scale(0.5);

        // The Cartesian mean lies between them.
        assert!(mean.delta_e(a) < a.delta_e(b));
        assert!(mean.delta_e(b) < a.delta_e(b));

        // An arithmetic mean of the hue *angles* would be far away whenever
        // the two straddle the wrap; check the property that protects us.
        let mean_lch = mean.to_oklch();
        let wrapped = crate::spaces::hue_difference(mean_lch.h, a.to_oklch().h);
        assert!(wrapped.abs() < 0.5, "mean hue drifted by {wrapped} radians");
    }

    /// Asking for more colours than the image contains must not invent them
    /// or loop forever.
    #[test]
    fn asking_for_more_colours_than_exist_is_bounded() {
        let palette = generate_from(&hist_of(&[[10, 10, 10, 255], [200, 200, 200, 255]]), 32);
        assert!(palette.len() <= 32);
        assert!(!palette.is_empty());
    }

    #[test]
    fn an_empty_image_yields_an_empty_palette_not_a_panic() {
        let empty = ColorHistogram::new();
        assert!(generate_from(&empty, 8).is_empty());
    }

    /// A single flat colour must produce one entry at that colour, not `k`
    /// copies of it.
    #[test]
    fn a_flat_image_produces_one_entry() {
        let pixels: Vec<[u8; 4]> = (0..16).map(|_| [77, 88, 99, 255]).collect();
        let palette = generate_from(&hist_of(&pixels), 8);
        assert_eq!(palette.len(), 1);
        let got = palette.entries()[0].output;
        assert!(
            (i32::from(got.r) - 77).abs() <= 5 && (i32::from(got.b) - 99).abs() <= 5,
            "recovered {got:?}"
        );
    }

    #[test]
    fn generation_respects_the_work_budget() {
        let pixels: Vec<[u8; 4]> = (0..64).map(|i| [i as u8 * 4, 10, 200, 255]).collect();
        let h = hist_of(&pixels);
        let mut limits = palette_tracer_core::limits::ResourceLimits::small();
        limits.work_budget = Some(2);
        let budget = WorkBudget::new(&limits);
        let err = generate(&h, 8, &[], ReachSpec::default(), &budget, &NoControl).unwrap_err();
        assert_eq!(err.code(), "resource.work_budget_exhausted");
    }
}
