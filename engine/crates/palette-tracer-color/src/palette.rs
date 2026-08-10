//! Palette entries, colour reaches, and memory-bounded assignment
//! (§7.4, §7.5).
//!
//! # The reach model (§7.4)
//!
//! An entry claims a sample when the anisotropic normalised distance is at
//! most one:
//!
//! ```text
//! s_k(x) = w_L (ΔL/r_L)² + w_C (ΔC/r_C)² + w_H (2√(C_x C_k) sin(Δh/2) / r_H)²
//! ```
//!
//! The hue term is a *chord* in the `a`-`b` plane, not an angle. That is what
//! makes it behave near grey: as either chroma goes to zero the chord goes to
//! zero, so an unstable hue angle contributes nothing. PTE-COLOR-011 asks for
//! the weight to "smoothly lose weight rather than applying an unstable
//! angle", and the chord alone already does most of that; this build adds an
//! explicit smoothstep on top so the transition is controllable and reported
//! (`color.neutralChromaThreshold`).
//!
//! # Assignment (§7.5)
//!
//! `O(N)` label storage and bounded working memory independent of `N x K`
//! (PTE-COLOR-015). Pixels are the outer loop and entries the inner one, with
//! a running `(best_score, best_id)` per pixel that lives in registers and is
//! written once to the label plane. There is no per-pixel-per-entry buffer
//! anywhere, so PTE-NO-004's forbidden shape cannot be written.
//!
//! # Provenance
//!
//! Written from `engine/SPEC.md` §7.4--§7.5. The reach concept originates in
//! the Palette-Tracer product specification; the formula implemented here is
//! the one written out in the engine specification, and no code was consulted
//! (PTE-LIC-004).

use crate::alpha::PremulLinearRgba;
use crate::sampler::{cache_key, sample_premultiplied};
use crate::spaces::{EncodedSrgb, Oklab, Oklch, hue_difference};
use palette_tracer_core::config::{EffectiveColor, EffectivePalette, PaletteRole, ReachSpec};
use palette_tracer_core::control::{TraceControl, check_cancel};
use palette_tracer_core::determinism::QuantKey;
use palette_tracer_core::error::TraceError;
use palette_tracer_core::image::ImageView;
use palette_tracer_core::ir::Color;
use palette_tracer_core::labels::{LabelId, LabelPlane};
use palette_tracer_core::limits::{WorkBudget, cost};

/// Relative weight of the lightness term in the reach score.
///
/// All three weights are 1.0: the reach radii already carry the anisotropy,
/// and a second set of knobs doing the same job would make the objective
/// ambiguous, which is what PTE-SEG-011 warns against for merging and applies
/// equally here.
pub const WEIGHT_LIGHTNESS: f64 = 1.0;
/// Relative weight of the chroma term.
pub const WEIGHT_CHROMA: f64 = 1.0;
/// Relative weight of the hue term.
pub const WEIGHT_HUE: f64 = 1.0;

/// A palette entry, resolved and ready to match against.
#[derive(Debug, Clone, PartialEq)]
pub struct PaletteEntry {
    /// Stable identifier (§7.4).
    pub id: u32,
    /// The colour that is emitted, byte for byte (PTE-SVG-012).
    pub output: Color,
    /// Perceptual anchor used for matching. Never emitted (PTE-SVG-012).
    pub anchor: Oklab,
    /// The anchor in cylindrical form, precomputed because the reach score
    /// needs chroma and hue on every comparison.
    pub anchor_lch: Oklch,
    /// Pinned entries never move and never merge away.
    pub pinned: bool,
    /// Reach radii.
    pub reach: ReachSpec,
    /// Role.
    pub role: PaletteRole,
    /// Tie-break priority; higher wins (PTE-COLOR-012).
    pub priority: i16,
}

impl PaletteEntry {
    /// Build an entry from an output colour.
    #[must_use]
    pub fn new(id: u32, output: Color, reach: ReachSpec) -> Self {
        let anchor = EncodedSrgb::from_u8(output.r, output.g, output.b)
            .to_linear()
            .to_oklab();
        Self {
            id,
            output,
            anchor,
            anchor_lch: anchor.to_oklch(),
            pinned: false,
            reach,
            role: PaletteRole::Fill,
            priority: 0,
        }
    }

    /// The anisotropic normalised distance of §7.4.
    ///
    /// Returns the score, where `<= 1.0` means the entry claims the sample.
    /// Lower is better; the value is unbounded above.
    #[must_use]
    pub fn reach_score(&self, sample: Oklch, neutral_chroma_threshold: f64) -> f64 {
        let d_l = sample.l - self.anchor_lch.l;
        let d_c = sample.c - self.anchor_lch.c;

        let r_l = self.reach.lightness.get();
        let r_c = self.reach.chroma.get();
        let r_h = self.reach.hue.get();

        let term_l = WEIGHT_LIGHTNESS * (d_l / r_l).powi(2);
        let term_c = WEIGHT_CHROMA * (d_c / r_c).powi(2);

        // The hue chord: `2 sqrt(C_x C_k) sin(Δh/2)` is the straight-line
        // distance in the a-b plane between two points at those chromas
        // separated by that angle.
        let d_h = hue_difference(sample.h, self.anchor_lch.h);
        let chord = 2.0 * (sample.c * self.anchor_lch.c).max(0.0).sqrt() * (d_h / 2.0).sin();
        let term_h = WEIGHT_HUE
            * hue_weight(sample.c, self.anchor_lch.c, neutral_chroma_threshold)
            * (chord / r_h).powi(2);

        term_l + term_c + term_h
    }

    /// Whether this entry claims `sample` (§7.4).
    #[must_use]
    pub fn claims(&self, sample: Oklch, neutral_chroma_threshold: f64) -> bool {
        self.reach_score(sample, neutral_chroma_threshold) <= 1.0
    }
}

/// Smooth weight for the hue term, going to zero near neutral
/// (PTE-COLOR-011).
///
/// A hard switch at the threshold would make two nearly identical greys land
/// in different entries depending on which side of the cutoff they fall, which
/// is exactly the instability the requirement exists to prevent. The
/// smoothstep `3t² - 2t³` is `0` at `t = 0`, `1` at `t = 1`, and has zero
/// derivative at both ends, so no threshold crossing produces a kink.
#[must_use]
pub fn hue_weight(sample_chroma: f64, anchor_chroma: f64, threshold: f64) -> f64 {
    if threshold <= 0.0 {
        return 1.0;
    }
    let t = (sample_chroma.min(anchor_chroma) / threshold).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// A resolved palette.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Palette {
    entries: Vec<PaletteEntry>,
}

impl Palette {
    /// Build from resolved configuration entries.
    ///
    /// Entries arrive in identifier order (the expansion sorts them), and that
    /// order is preserved so a label index maps to a stable entry
    /// (PTE-API-010).
    #[must_use]
    pub fn from_effective(palette: &EffectivePalette) -> Self {
        let entries = palette
            .entries
            .iter()
            .map(|e| {
                let output = Color::from_array(e.rgba);
                let anchor = EncodedSrgb::from_u8(output.r, output.g, output.b)
                    .to_linear()
                    .to_oklab();
                PaletteEntry {
                    id: e.id,
                    output,
                    anchor,
                    anchor_lch: anchor.to_oklch(),
                    pinned: e.pinned,
                    reach: e.reach,
                    role: e.role,
                    priority: e.priority,
                }
            })
            .collect();
        Self { entries }
    }

    /// Build from a list of entries, keeping them in the given order.
    #[must_use]
    pub fn from_entries(entries: Vec<PaletteEntry>) -> Self {
        Self { entries }
    }

    /// The entries, in label order.
    #[must_use]
    pub fn entries(&self) -> &[PaletteEntry] {
        &self.entries
    }

    /// Number of entries.
    #[must_use]
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Whether the palette is empty.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// The winning entry for `sample`, by the §7.4 total order.
    ///
    /// PTE-COLOR-012: "If multiple entries claim a sample, the winner MUST be
    /// determined by normalized score, explicit priority, then stable palette
    /// ID. Array iteration order alone MUST NOT be the undocumented
    /// tie-break."
    ///
    /// The score is compared as a quantised key so that two scores differing
    /// by a rounding artefact are treated as the tie they are, and the
    /// declared tie-break decides them (PTE-DET-003).
    ///
    /// Returns the label index and whether the winner actually *claims* the
    /// sample. An unclaimed sample still gets an owner -- PTE-SEG-002 requires
    /// every in-domain pixel to have one -- but the caller counts it for the
    /// §26.4 "pixels outside every reach" metric.
    #[must_use]
    pub fn best_match(&self, sample: Oklch, neutral_chroma_threshold: f64) -> Option<Match> {
        let mut best: Option<(QuantKey, i16, u32, usize, f64)> = None;
        for (index, entry) in self.entries.iter().enumerate() {
            let score = entry.reach_score(sample, neutral_chroma_threshold);
            let key = QuantKey::cost_or_worst(score);
            let candidate = (key, entry.priority, entry.id, index, score);
            best = Some(match best {
                None => candidate,
                Some(current) => {
                    // Lower key wins; then higher priority; then lower id.
                    let better = (candidate.0, core::cmp::Reverse(candidate.1), candidate.2)
                        < (current.0, core::cmp::Reverse(current.1), current.2);
                    if better { candidate } else { current }
                }
            });
        }
        best.map(|(_, _, _, index, score)| Match {
            label: LabelId(index as u32),
            entry_id: self.entries[index].id,
            score,
            claimed: score <= 1.0,
        })
    }

    /// Total pinned entries.
    #[must_use]
    pub fn pinned_count(&self) -> usize {
        self.entries.iter().filter(|e| e.pinned).count()
    }
}

/// The outcome of matching one sample.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Match {
    /// Index into the palette, which is also the label.
    pub label: LabelId,
    /// The winning entry's stable identifier.
    pub entry_id: u32,
    /// Its normalised reach score.
    pub score: f64,
    /// Whether the score was within the reach.
    pub claimed: bool,
}

/// Fixed-size exact cache for repeated colours (PTE-COLOR-017).
///
/// Direct-mapped, so lookup and insert are both one array access and there is
/// no eviction policy to make nondeterministic. Crucially the key comparison
/// is *exact*: a hit returns the result that a miss would have computed, so
/// which entries happen to be resident cannot change a single label. That is
/// the whole of "Hash-table eviction or iteration MUST NOT affect labels".
struct ColorCache {
    keys: Vec<u32>,
    values: Vec<LabelId>,
    /// Whether the cached match was inside the entry's reach. Cached
    /// alongside the label because the claim status is part of the answer:
    /// without it a hit would be miscounted, and the §26.4 "outside every
    /// reach" metric would silently read zero.
    claimed: Vec<bool>,
    occupied: Vec<bool>,
    hits: u64,
}

impl ColorCache {
    /// 65536 slots: 512 KiB, independent of image size and of palette size.
    const SLOTS: usize = 1 << 16;

    fn new() -> Self {
        Self {
            keys: vec![0; Self::SLOTS],
            values: vec![LabelId::ZERO; Self::SLOTS],
            claimed: vec![false; Self::SLOTS],
            occupied: vec![false; Self::SLOTS],
            hits: 0,
        }
    }

    #[inline]
    fn slot(key: u32) -> usize {
        // Multiplicative hash by a 32-bit odd constant, then take the high
        // bits. Fixed and unseeded -- a randomised hash would be a §20.2
        // nondeterminism source even though it could not change a label.
        ((key.wrapping_mul(2_654_435_761) >> 16) as usize) & (Self::SLOTS - 1)
    }

    #[inline]
    fn get(&mut self, key: u32) -> Option<(LabelId, bool)> {
        let s = Self::slot(key);
        if self.occupied[s] && self.keys[s] == key {
            self.hits += 1;
            Some((self.values[s], self.claimed[s]))
        } else {
            None
        }
    }

    #[inline]
    fn put(&mut self, key: u32, value: LabelId, claimed: bool) {
        let s = Self::slot(key);
        self.keys[s] = key;
        self.values[s] = value;
        self.claimed[s] = claimed;
        self.occupied[s] = true;
    }
}

/// The result of assigning every pixel to a palette entry.
#[derive(Debug, Clone, PartialEq)]
pub struct Assignment {
    /// One label per pixel, indexing the palette.
    pub labels: LabelPlane,
    /// Pixels claimed by some entry's reach.
    pub claimed_pixels: u64,
    /// Pixels no entry claimed, assigned to the nearest by score
    /// (§26.4, "pixels/regions outside every reach").
    pub unclaimed_pixels: u64,
    /// Pixels with no colour evidence under the alpha policy
    /// (PTE-COLOR-008). These carry the label of the nearest entry so the
    /// partition stays exclusive, and are counted separately so nothing reads
    /// them as colour evidence.
    pub transparent_pixels: u64,
    /// Pixels per palette entry, in label order.
    pub pixels_per_entry: Vec<u64>,
    /// Cache hits, for the report.
    pub cache_hits: u64,
}

/// Assign every pixel to a palette entry (§7.5).
///
/// Time is `O(N x K)` in the worst case and memory is `O(N)` plus a fixed
/// cache, exactly as the §19.1 table requires.
///
/// # Errors
///
/// [`TraceError::Cancelled`] if the caller cancels,
/// [`TraceError::ResourceLimit`] if the work budget runs out, or
/// [`TraceError::Decode`] if the label plane cannot be allocated.
///
/// # Panics
///
/// Never; the palette is checked non-empty before the loop.
pub fn assign(
    view: &ImageView<'_>,
    palette: &Palette,
    color: &EffectiveColor,
    budget: &WorkBudget,
    control: &dyn TraceControl,
) -> Result<Assignment, TraceError> {
    if palette.is_empty() {
        return Err(TraceError::Config(
            palette_tracer_core::error::ConfigError::Palette {
                reason: "assignment needs at least one palette entry",
            },
        ));
    }

    let mut labels = LabelPlane::new(view.width(), view.height(), palette.len() as u64)?;
    let mut cache = ColorCache::new();
    let mut pixels_per_entry = vec![0u64; palette.len()];
    let mut claimed = 0u64;
    let mut unclaimed = 0u64;
    let mut transparent = 0u64;

    let threshold = color.neutral_chroma_threshold;
    let epsilon = color.alpha.unpremultiply_epsilon.get();

    let mut i = 0usize;
    for y in 0..view.height() {
        for x in 0..view.width() {
            check_cancel(control, i)?;
            budget.charge(cost::PIXEL)?;

            let premul = sample_premultiplied(view, x, y);
            let label = if premul.a <= epsilon {
                // PTE-COLOR-008: no colour influence. The pixel still needs an
                // owner, and entry zero is the deterministic choice; the count
                // below is what keeps it out of every colour statistic.
                transparent += 1;
                LabelId::ZERO
            } else {
                let key = cache_key(view, x, y);
                match key.and_then(|k| cache.get(k)) {
                    Some((cached, was_claimed)) => {
                        if was_claimed {
                            claimed += 1;
                        } else {
                            unclaimed += 1;
                        }
                        cached
                    }
                    None => {
                        let straight = premul
                            .to_straight(epsilon)
                            .unwrap_or(crate::spaces::LinearRgb::BLACK);
                        let lch = straight.to_oklab().to_oklch();
                        let m = palette.best_match(lch, threshold).unwrap_or(Match {
                            label: LabelId::ZERO,
                            entry_id: 0,
                            score: f64::MAX,
                            claimed: false,
                        });
                        if m.claimed {
                            claimed += 1;
                        } else {
                            unclaimed += 1;
                        }
                        if let Some(k) = key {
                            cache.put(k, m.label, m.claimed);
                        }
                        m.label
                    }
                }
            };

            labels.set_index(i, label);
            pixels_per_entry[label.index()] += 1;
            i += 1;
        }
    }

    Ok(Assignment {
        labels,
        claimed_pixels: claimed,
        unclaimed_pixels: unclaimed,
        transparent_pixels: transparent,
        pixels_per_entry,
        cache_hits: cache.hits,
    })
}

/// Assign one sample without a cache, for oracles and tests.
///
/// The scalar oracle §25.4's "Assignment" row asks the fast path to agree
/// with.
#[must_use]
pub fn assign_one(palette: &Palette, sample: PremulLinearRgba, color: &EffectiveColor) -> Match {
    let epsilon = color.alpha.unpremultiply_epsilon.get();
    let straight = sample
        .to_straight(epsilon)
        .unwrap_or(crate::spaces::LinearRgb::BLACK);
    palette
        .best_match(
            straight.to_oklab().to_oklch(),
            color.neutral_chroma_threshold,
        )
        .unwrap_or(Match {
            label: LabelId::ZERO,
            entry_id: 0,
            score: f64::MAX,
            claimed: false,
        })
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
    use palette_tracer_core::limits::WorkBudget;

    fn color_policy() -> EffectiveColor {
        Profile::expand(&TraceConfig::default()).unwrap().color
    }

    fn entry(id: u32, rgb: [u8; 3]) -> PaletteEntry {
        PaletteEntry::new(id, Color::rgb(rgb[0], rgb[1], rgb[2]), ReachSpec::default())
    }

    fn lch(rgb: [u8; 3]) -> Oklch {
        EncodedSrgb::from_u8(rgb[0], rgb[1], rgb[2])
            .to_linear()
            .to_oklab()
            .to_oklch()
    }

    /// §25.4 "Color" row: reach inclusivity. An entry claims its own colour
    /// with a score of zero, and something far away with a score above one.
    #[test]
    fn an_entry_claims_itself_exactly_and_rejects_a_distant_colour() {
        let red = entry(0, [220, 30, 40]);
        assert!(red.reach_score(lch([220, 30, 40]), 0.02) < 1e-12);
        assert!(red.claims(lch([220, 30, 40]), 0.02));
        assert!(!red.claims(lch([30, 40, 220]), 0.02));
    }

    /// PTE-COLOR-011: near grey, hue is powerless. Two near-neutral colours
    /// with wildly different hue angles must not be separated by hue.
    #[test]
    fn hue_is_powerless_near_neutral() {
        // Two nearly identical greys whose hue angles are almost opposite.
        let warm = lch([128, 127, 127]);
        let cool = lch([127, 127, 128]);
        assert!(
            (warm.c - cool.c).abs() < 0.01,
            "both must be near-neutral for the test to mean anything"
        );

        let grey = entry(0, [127, 127, 127]);
        let threshold = 0.02;
        assert!(grey.claims(warm, threshold), "warm grey must be claimed");
        assert!(grey.claims(cool, threshold), "cool grey must be claimed");

        // The weight itself decays to zero and is continuous.
        assert_eq!(hue_weight(0.0, 0.1, 0.02), 0.0);
        assert_eq!(hue_weight(0.1, 0.1, 0.02), 1.0);
        let mid = hue_weight(0.01, 0.1, 0.02);
        assert!(
            mid > 0.0 && mid < 1.0,
            "the transition must be gradual: {mid}"
        );
    }

    /// The smoothstep has zero derivative at both ends, so no small change in
    /// chroma produces a jump in the score.
    #[test]
    fn the_hue_weight_transition_has_no_kink() {
        let threshold = 0.02;
        let mut previous = hue_weight(0.0, 1.0, threshold);
        let mut max_step: f64 = 0.0;
        for i in 1..=200 {
            let c = f64::from(i) / 200.0 * threshold * 1.5;
            let w = hue_weight(c, 1.0, threshold);
            max_step = max_step.max((w - previous).abs());
            previous = w;
        }
        assert!(max_step < 0.02, "largest single step was {max_step}");
    }

    /// PTE-COLOR-012: the declared total order, not array position.
    #[test]
    fn ties_are_broken_by_priority_then_id_not_array_order() {
        let mut low = entry(9, [100, 100, 100]);
        low.priority = 1;
        let mut high = entry(2, [100, 100, 100]);
        high.priority = 5;

        // Identical anchors, so the scores tie exactly. Priority decides.
        let palette = Palette::from_entries(vec![low.clone(), high.clone()]);
        let m = palette.best_match(lch([100, 100, 100]), 0.02).unwrap();
        assert_eq!(m.entry_id, 2, "higher priority must win the tie");

        // Reversing the array must not change the answer.
        let reversed = Palette::from_entries(vec![high, low]);
        assert_eq!(
            reversed
                .best_match(lch([100, 100, 100]), 0.02)
                .unwrap()
                .entry_id,
            2
        );

        // With equal priority, the lower id wins.
        let a = entry(7, [100, 100, 100]);
        let b = entry(3, [100, 100, 100]);
        let palette = Palette::from_entries(vec![a, b]);
        assert_eq!(
            palette
                .best_match(lch([100, 100, 100]), 0.02)
                .unwrap()
                .entry_id,
            3
        );
    }

    /// PTE-COLOR-013: two inks with the same visible colour stay distinct.
    #[test]
    fn entries_with_equal_colours_remain_separately_addressable() {
        let mut a = entry(1, [10, 10, 10]);
        a.pinned = true;
        let mut b = entry(2, [10, 10, 10]);
        b.pinned = true;
        let palette = Palette::from_entries(vec![a, b]);
        assert_eq!(palette.len(), 2);
        assert_eq!(palette.pinned_count(), 2);
        assert_eq!(palette.entries()[0].id, 1);
        assert_eq!(palette.entries()[1].id, 2);
    }

    fn checkerboard_view(data: &[u8]) -> ImageView<'_> {
        ImageView::new(
            4,
            4,
            0,
            PixelFormat::Rgba8,
            data,
            ColorEncoding::Srgb,
            AlphaMode::Straight,
            Orientation::Identity,
        )
        .unwrap()
    }

    fn two_colour_image() -> Vec<u8> {
        let mut data = Vec::with_capacity(4 * 4 * 4);
        for y in 0..4 {
            for x in 0..4 {
                if (x + y) % 2 == 0 {
                    data.extend_from_slice(&[220, 30, 40, 255]);
                } else {
                    data.extend_from_slice(&[30, 40, 220, 255]);
                }
            }
        }
        data
    }

    /// §25.4 "Assignment" row: the cached fast path must agree with the
    /// scalar oracle on every pixel.
    #[test]
    fn the_cached_path_agrees_with_the_scalar_oracle() {
        let data = two_colour_image();
        let view = checkerboard_view(&data);
        let palette = Palette::from_entries(vec![entry(0, [220, 30, 40]), entry(1, [30, 40, 220])]);
        let color = color_policy();
        let budget = WorkBudget::unbounded();

        let result = assign(&view, &palette, &color, &budget, &NoControl).unwrap();

        for y in 0..4 {
            for x in 0..4 {
                let oracle = assign_one(&palette, sample_premultiplied(&view, x, y), &color);
                assert_eq!(
                    result.labels.get(x, y),
                    oracle.label,
                    "pixel ({x}, {y}) disagreed with the oracle"
                );
            }
        }
        assert_eq!(result.claimed_pixels, 16);
        assert_eq!(result.unclaimed_pixels, 0);
        assert_eq!(result.pixels_per_entry, vec![8, 8]);
        assert!(result.cache_hits > 0, "repeated colours must hit the cache");
    }

    /// The cache is exact, so its state cannot change a label. Running the
    /// same image twice, and running it with the pixels visited in a different
    /// spatial arrangement, must give the same per-colour answer.
    #[test]
    fn cache_residency_cannot_change_a_label() {
        let palette = Palette::from_entries(vec![entry(0, [220, 30, 40]), entry(1, [30, 40, 220])]);
        let color = color_policy();

        let data = two_colour_image();
        let view = checkerboard_view(&data);
        let a = assign(
            &view,
            &palette,
            &color,
            &WorkBudget::unbounded(),
            &NoControl,
        )
        .unwrap();
        let b = assign(
            &view,
            &palette,
            &color,
            &WorkBudget::unbounded(),
            &NoControl,
        )
        .unwrap();
        assert_eq!(a.labels, b.labels);

        // Same two colours, different spatial layout: every red pixel must
        // still be entry 0 regardless of what the cache saw before it.
        let mut striped = Vec::new();
        for y in 0..4 {
            for _ in 0..4 {
                if y < 2 {
                    striped.extend_from_slice(&[220, 30, 40, 255]);
                } else {
                    striped.extend_from_slice(&[30, 40, 220, 255]);
                }
            }
        }
        let sv = checkerboard_view(&striped);
        let s = assign(&sv, &palette, &color, &WorkBudget::unbounded(), &NoControl).unwrap();
        assert_eq!(s.pixels_per_entry, vec![8, 8]);
    }

    /// PTE-COLOR-008 at the assignment level: a transparent pixel's stored
    /// colour must not reach any statistic.
    #[test]
    fn transparent_pixels_are_counted_separately_and_never_claimed() {
        let mut data = Vec::new();
        for i in 0..16 {
            if i < 8 {
                // Saturated colours hidden under zero alpha.
                data.extend_from_slice(&[255, 0, 255, 0]);
            } else {
                data.extend_from_slice(&[220, 30, 40, 255]);
            }
        }
        let view = checkerboard_view(&data);
        let palette = Palette::from_entries(vec![entry(0, [220, 30, 40]), entry(1, [30, 40, 220])]);
        let result = assign(
            &view,
            &palette,
            &color_policy(),
            &WorkBudget::unbounded(),
            &NoControl,
        )
        .unwrap();

        assert_eq!(result.transparent_pixels, 8);
        assert_eq!(result.claimed_pixels, 8);
        assert_eq!(result.unclaimed_pixels, 0);
    }

    /// PTE-SEG-002: every in-domain pixel gets an owner, even one no entry
    /// claims -- and the count says so rather than hiding it.
    #[test]
    fn a_colour_outside_every_reach_is_still_owned_and_counted() {
        let mut data = Vec::new();
        for _ in 0..16 {
            data.extend_from_slice(&[0, 200, 0, 255]);
        }
        let view = checkerboard_view(&data);
        let palette = Palette::from_entries(vec![entry(0, [220, 30, 40]), entry(1, [30, 40, 220])]);
        let result = assign(
            &view,
            &palette,
            &color_policy(),
            &WorkBudget::unbounded(),
            &NoControl,
        )
        .unwrap();

        assert_eq!(result.unclaimed_pixels, 16);
        assert_eq!(result.claimed_pixels, 0);
        assert_eq!(
            result.pixels_per_entry.iter().sum::<u64>(),
            16,
            "every pixel must still have exactly one owner"
        );
    }

    /// §19.4: memory must not scale with `N x K`. The label plane is the only
    /// per-pixel allocation, and the cache is a fixed 512 KiB.
    #[test]
    fn working_memory_does_not_scale_with_palette_size() {
        let data = two_colour_image();
        let view = checkerboard_view(&data);
        let color = color_policy();

        let small = Palette::from_entries(vec![entry(0, [220, 30, 40])]);
        let big = Palette::from_entries(
            (0..200u32)
                .map(|i| {
                    entry(
                        i,
                        [
                            (i * 7 % 256) as u8,
                            (i * 13 % 256) as u8,
                            (i * 3 % 256) as u8,
                        ],
                    )
                })
                .collect(),
        );

        let a = assign(&view, &small, &color, &WorkBudget::unbounded(), &NoControl).unwrap();
        let b = assign(&view, &big, &color, &WorkBudget::unbounded(), &NoControl).unwrap();
        // Both label planes are exactly one byte per pixel: the palette size
        // changed the tier bound, not the per-pixel cost.
        assert_eq!(a.labels.bytes(), 16);
        assert_eq!(b.labels.bytes(), 16);
    }

    #[test]
    fn assignment_is_cancellable_and_budgeted() {
        let data = two_colour_image();
        let view = checkerboard_view(&data);
        let palette = Palette::from_entries(vec![entry(0, [220, 30, 40])]);

        let mut limits = palette_tracer_core::limits::ResourceLimits::small();
        limits.work_budget = Some(4);
        let budget = WorkBudget::new(&limits);
        let err = assign(&view, &palette, &color_policy(), &budget, &NoControl).unwrap_err();
        assert_eq!(err.code(), "resource.work_budget_exhausted");
    }

    #[test]
    fn an_empty_palette_is_refused_rather_than_producing_an_empty_plane() {
        let data = two_colour_image();
        let view = checkerboard_view(&data);
        let err = assign(
            &view,
            &Palette::default(),
            &color_policy(),
            &WorkBudget::unbounded(),
            &NoControl,
        )
        .unwrap_err();
        assert_eq!(err.code(), "config.palette");
    }
}
