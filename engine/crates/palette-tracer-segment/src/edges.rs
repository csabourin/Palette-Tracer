//! The edge field (§8.2).
//!
//! A cost on the 4-connected pixel graph, built from linear-light and
//! perceptual evidence:
//!
//! ```text
//! w_pq = λ_c ρ(‖o_p − o_q‖_OK) + λ_g g_pq + λ_α |α_p − α_q| + λ_r r_pq
//! ```
//!
//! * `ρ` is a robust loss -- here a saturating one, so a single extreme
//!   difference cannot dominate a whole region's statistics.
//! * `g` is a multi-scale directional gradient response (§8.2, PTE-SEG-005):
//!   the same difference measured over two pixels as well as one, which is
//!   what separates a real low-contrast edge from compression noise. A blocky
//!   JPEG artefact is high-frequency and disappears at the coarser scale; a
//!   genuine soft edge survives both.
//! * `r` is palette-claim disagreement, which is how PTE-COLOR-014 gets its
//!   way: "Palette colors MUST participate during segmentation", so a boundary
//!   between two different palette claims is evidence of a boundary even when
//!   the colours are close.
//!
//! PTE-SEG-004: none of this operates on gamma-encoded RGB differences. Every
//! term comes from a linear-light or OKLab quantity.
//!
//! # Quantisation
//!
//! # Connectivity
//!
//! §8.2 permits "the 4- or 8-connected pixel graph", and this build uses the
//! 8-connected one. That is not a quality tweak; it is what makes §9.4 mean
//! anything. Under 4-connectivity the pixels of a one-pixel diagonal line are
//! never adjacent, so each becomes its own region, the label map never contains
//! the `A B / B A` pattern, and the ambiguity machinery is dead code that
//! happens to compile. Under 8-connectivity the line is one region, every cell
//! along it is ambiguous, and §9.4 decides -- which is the behaviour the
//! specification describes.
//!
//! The *region adjacency graph* stays 4-connected: a shared boundary is a
//! shared raster interface, and two regions touching only at a corner share no
//! interface and no length. So connectivity and adjacency deliberately differ,
//! and each is used for the thing it means.
//!
//! # Quantisation
//!
//! Weights are stored as `u16`. That is not a memory optimisation -- it is
//! what makes the partition deterministic. A counting sort over 65536 buckets
//! is exact and order-preserving, so two targets whose floating-point
//! arithmetic differs in the last bit still sort the edges identically
//! (PTE-DET-003). The resolution, one part in 65535 of the normalised cost
//! range, is far finer than any merge threshold.

use palette_tracer_color::spaces::Oklab;
use palette_tracer_core::checked::u32_to_usize;
use palette_tracer_core::config::EffectiveSegmentation;
use palette_tracer_core::control::{TraceControl, check_cancel};
use palette_tracer_core::error::TraceError;
use palette_tracer_core::labels::{LabelId, LabelPlane};
use palette_tracer_core::limits::{WorkBudget, cost};

/// Largest quantised edge weight.
pub const MAX_WEIGHT: u16 = u16::MAX;

/// Normalising divisor for the colour term.
///
/// The largest OKLab distance between two in-gamut sRGB colours is a little
/// under 1.2 (black to white is 1.0; the most saturated pairs exceed it). 1.5
/// leaves headroom so the robust loss saturates on genuinely extreme pairs
/// rather than on ordinary ones.
const COLOR_NORMALISER: f64 = 1.5;

/// Robust loss `ρ`.
///
/// `x / (x + c)` maps `[0, ∞)` onto `[0, 1)`, is monotone, and has a bounded
/// derivative, so no single pair of pixels can dominate. `c` sets where the
/// response is half: at an OKLab distance of 0.25, which is roughly where a
/// difference stops being "a shade of the same colour".
fn robust(x: f64) -> f64 {
    const HALF_RESPONSE: f64 = 0.25;
    x / (x + HALF_RESPONSE)
}

/// Per-pixel evidence the edge field is built from.
///
/// Assembled once by the caller so the field does not re-derive colours it
/// already has, and so the same samples feed the region statistics.
#[derive(Debug, Clone)]
pub struct PixelEvidence {
    /// Perceptual colour per pixel, row-major.
    pub color: Vec<Oklab>,
    /// Alpha per pixel, row-major.
    pub alpha: Vec<f64>,
    /// Palette claim per pixel, row-major.
    pub claim: Vec<LabelId>,
    /// Image width.
    pub width: u32,
    /// Image height.
    pub height: u32,
}

impl PixelEvidence {
    /// Row-major index.
    #[must_use]
    pub fn index(&self, x: u32, y: u32) -> usize {
        u32_to_usize(y) * u32_to_usize(self.width) + u32_to_usize(x)
    }
}

/// Quantised costs on the 4-connected pixel graph.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EdgeField {
    width: u32,
    height: u32,
    /// Cost between `(x, y)` and `(x + 1, y)`, indexed by `(x, y)`.
    horizontal: Vec<u16>,
    /// Cost between `(x, y)` and `(x, y + 1)`, indexed by `(x, y)`.
    vertical: Vec<u16>,
    /// Cost between `(x, y)` and `(x + 1, y + 1)`, indexed by `(x, y)`.
    down_right: Vec<u16>,
    /// Cost between `(x, y)` and `(x - 1, y + 1)`, indexed by `(x, y)`.
    down_left: Vec<u16>,
}

impl EdgeField {
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

    /// Cost between `(x, y)` and `(x + 1, y)`.
    ///
    /// # Panics
    ///
    /// If `(x, y)` is outside the image.
    #[must_use]
    pub fn horizontal(&self, x: u32, y: u32) -> u16 {
        self.horizontal[u32_to_usize(y) * u32_to_usize(self.width) + u32_to_usize(x)]
    }

    /// Cost between `(x, y)` and `(x, y + 1)`.
    ///
    /// # Panics
    ///
    /// If `(x, y)` is outside the image.
    #[must_use]
    pub fn vertical(&self, x: u32, y: u32) -> u16 {
        self.vertical[u32_to_usize(y) * u32_to_usize(self.width) + u32_to_usize(x)]
    }

    /// Bytes this field occupies, for the §19.2 budget.
    #[must_use]
    pub fn bytes(&self) -> u64 {
        palette_tracer_core::checked::usize_to_u64(
            (self.horizontal.len()
                + self.vertical.len()
                + self.down_right.len()
                + self.down_left.len())
                * 2,
        )
    }

    /// Every graph edge, as `(weight, a, b)` with `a < b` in row-major index
    /// order.
    ///
    /// The iteration order is fixed and derived from raster position, so the
    /// sort that follows is stable in a way that does not depend on the
    /// container (PTE-SEG-007).
    pub fn edges(&self) -> impl Iterator<Item = (u16, usize, usize)> + '_ {
        let w = u32_to_usize(self.width);
        let h = u32_to_usize(self.height);
        (0..h).flat_map(move |y| {
            (0..w).flat_map(move |x| {
                let i = y * w + x;
                let right = (x + 1 < w).then(|| (self.horizontal[i], i, i + 1));
                let down = (y + 1 < h).then(|| (self.vertical[i], i, i + w));
                let down_right =
                    (x + 1 < w && y + 1 < h).then(|| (self.down_right[i], i, i + w + 1));
                let down_left = (x > 0 && y + 1 < h).then(|| (self.down_left[i], i, i + w - 1));
                right
                    .into_iter()
                    .chain(down)
                    .chain(down_right)
                    .chain(down_left)
            })
        })
    }
}

/// Build the edge field (§8.2).
///
/// # Errors
///
/// [`TraceError::Cancelled`] or [`TraceError::ResourceLimit`].
pub fn build(
    evidence: &PixelEvidence,
    policy: &EffectiveSegmentation,
    budget: &WorkBudget,
    control: &dyn TraceControl,
) -> Result<EdgeField, TraceError> {
    let w = u32_to_usize(evidence.width);
    let h = u32_to_usize(evidence.height);
    let mut horizontal = vec![0u16; w * h];
    let mut vertical = vec![0u16; w * h];
    let mut down_right = vec![0u16; w * h];
    let mut down_left = vec![0u16; w * h];

    let total_weight = policy.edge_weight_color
        + policy.edge_weight_gradient
        + policy.edge_weight_alpha
        + policy.edge_weight_claim;
    // A caller who zeroes every weight gets a uniform field rather than a
    // division by zero; the partition then comes from connectivity alone.
    let normaliser = if total_weight > 0.0 {
        total_weight
    } else {
        1.0
    };

    let mut counter = 0usize;
    for y in 0..h {
        for x in 0..w {
            check_cancel(control, counter)?;
            budget.charge(cost::EDGE)?;
            counter += 1;

            let i = y * w + x;
            if x + 1 < w {
                horizontal[i] = weight(evidence, policy, normaliser, i, i + 1, (2, 0), (w, h));
            }
            if y + 1 < h {
                vertical[i] = weight(evidence, policy, normaliser, i, i + w, (0, 2), (w, h));
            }
            // Diagonal costs carry a penalty: a corner touch is weaker evidence
            // of continuity than a shared edge, so a diagonal join must be a
            // clearer colour match than an orthogonal one to win. The factor is
            // the ratio of the distances, which is the same normalisation the
            // edge cost's units already imply.
            if x + 1 < w && y + 1 < h {
                let base = weight(evidence, policy, normaliser, i, i + w + 1, (2, 2), (w, h));
                down_right[i] = diagonal_penalty(base);
            }
            if x > 0 && y + 1 < h {
                let base = weight(evidence, policy, normaliser, i, i + w - 1, (0, 0), (w, h));
                down_left[i] = diagonal_penalty(base);
            }
        }
    }

    Ok(EdgeField {
        width: evidence.width,
        height: evidence.height,
        horizontal,
        vertical,
        down_right,
        down_left,
    })
}

/// The §8.2 cost for one pair, quantised.
///
/// `coarse_step` is the offset used for the multi-scale term: the same
/// direction, two pixels apart instead of one.
fn weight(
    evidence: &PixelEvidence,
    policy: &EffectiveSegmentation,
    normaliser: f64,
    p: usize,
    q: usize,
    coarse_step: (usize, usize),
    dims: (usize, usize),
) -> u16 {
    let (w, h) = dims;

    let color_term = robust(evidence.color[p].delta_e(evidence.color[q]) / COLOR_NORMALISER);

    // Multi-scale: measure the same direction across two pixels. Where the
    // fine and coarse responses agree, the edge is real; where the fine
    // response is large and the coarse one is not, it is noise at the pixel
    // scale (PTE-SEG-005).
    let coarse = {
        let px = p % w;
        let py = p / w;
        let cx = px + coarse_step.0;
        let cy = py + coarse_step.1;
        if cx < w && cy < h {
            let c = cy * w + cx;
            robust(evidence.color[p].delta_e(evidence.color[c]) / COLOR_NORMALISER)
        } else {
            color_term
        }
    };
    // The coarse response confirms rather than replaces: a fine edge with no
    // coarse support is attenuated, one with support is not.
    let gradient_term = color_term.min(coarse);

    let alpha_term = (evidence.alpha[p] - evidence.alpha[q]).abs();

    let claim_term = if evidence.claim[p] == evidence.claim[q] {
        0.0
    } else {
        1.0
    };

    let combined = (policy.edge_weight_color * color_term
        + policy.edge_weight_gradient * gradient_term
        + policy.edge_weight_alpha * alpha_term
        + policy.edge_weight_claim * claim_term)
        / normaliser;

    quantise(combined)
}

/// Scale a diagonal cost by the distance ratio.
///
/// Two pixels a corner apart are `sqrt(2)` times as far as two sharing an edge,
/// so the same colour difference is a steeper gradient and a weaker case for
/// joining. Saturating rather than wrapping keeps the ordering total.
#[must_use]
fn diagonal_penalty(weight: u16) -> u16 {
    const SQRT_2: f64 = core::f64::consts::SQRT_2;
    let scaled = f64::from(weight) * SQRT_2;
    if scaled >= f64::from(MAX_WEIGHT) {
        MAX_WEIGHT
    } else {
        scaled.round() as u16
    }
}

/// Map a normalised cost in `[0, 1]` to the `u16` grid.
#[must_use]
pub fn quantise(value: f64) -> u16 {
    let clamped = palette_tracer_core::checked::clamp_unit(value);
    (clamped * f64::from(MAX_WEIGHT)).round() as u16
}

/// Build per-pixel evidence from an assignment and the image.
///
/// # Errors
///
/// [`TraceError::Cancelled`] or [`TraceError::ResourceLimit`].
pub fn evidence_from(
    view: &palette_tracer_core::image::ImageView<'_>,
    labels: &LabelPlane,
    color: &palette_tracer_core::config::EffectiveColor,
    budget: &WorkBudget,
    control: &dyn TraceControl,
) -> Result<PixelEvidence, TraceError> {
    let n = labels.len();
    let mut colors = Vec::with_capacity(n);
    let mut alphas = Vec::with_capacity(n);
    let mut claims = Vec::with_capacity(n);
    let epsilon = color.alpha.unpremultiply_epsilon.get();

    let mut i = 0usize;
    for y in 0..view.height() {
        for x in 0..view.width() {
            check_cancel(control, i)?;
            budget.charge(cost::PIXEL)?;

            let premul = palette_tracer_color::sample_premultiplied(view, x, y);
            // A pixel with no colour evidence contributes its *position* to the
            // partition but not a colour; using black would make transparent
            // areas attract dark palette entries (PTE-COLOR-008).
            let lab = premul
                .to_straight(epsilon)
                .map_or(Oklab::ZERO, |s| s.to_oklab());
            colors.push(lab);
            alphas.push(premul.a);
            claims.push(labels.get_index(i));
            i += 1;
        }
    }

    Ok(PixelEvidence {
        color: colors,
        alpha: alphas,
        claim: claims,
        width: view.width(),
        height: view.height(),
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

    fn policy() -> EffectiveSegmentation {
        Profile::expand(&TraceConfig::default())
            .unwrap()
            .segmentation
    }

    fn evidence(colors: &[Oklab], width: u32, height: u32) -> PixelEvidence {
        PixelEvidence {
            color: colors.to_vec(),
            alpha: vec![1.0; colors.len()],
            claim: vec![LabelId::ZERO; colors.len()],
            width,
            height,
        }
    }

    fn lab(rgb: [u8; 3]) -> Oklab {
        palette_tracer_color::EncodedSrgb::from_u8(rgb[0], rgb[1], rgb[2])
            .to_linear()
            .to_oklab()
    }

    fn build_field(e: &PixelEvidence) -> EdgeField {
        build(e, &policy(), &WorkBudget::unbounded(), &NoControl).unwrap()
    }

    #[test]
    fn a_flat_image_has_zero_cost_everywhere() {
        let c = lab([100, 120, 140]);
        let e = evidence(&[c; 9], 3, 3);
        let field = build_field(&e);
        for y in 0..3 {
            for x in 0..3 {
                if x < 2 {
                    assert_eq!(field.horizontal(x, y), 0);
                }
                if y < 2 {
                    assert_eq!(field.vertical(x, y), 0);
                }
            }
        }
    }

    /// The whole point of the field: a real boundary must cost more than the
    /// interior of a region.
    #[test]
    fn a_colour_boundary_costs_more_than_a_flat_interior() {
        // Left half dark, right half light, 4x1.
        let e = evidence(
            &[
                lab([20, 20, 20]),
                lab([20, 20, 20]),
                lab([230, 230, 230]),
                lab([230, 230, 230]),
            ],
            4,
            1,
        );
        let field = build_field(&e);
        assert_eq!(field.horizontal(0, 0), 0, "inside the dark region");
        assert!(field.horizontal(1, 0) > 10_000, "across the boundary");
        assert_eq!(field.horizontal(2, 0), 0, "inside the light region");
    }

    /// PTE-SEG-004: the cost must not come from gamma-encoded differences. Two
    /// pairs an equal number of *code points* apart, one in the shadows and
    /// one in the highlights, are perceptually very different -- and the field
    /// must say so.
    #[test]
    fn equal_code_point_steps_are_not_equal_costs() {
        let shadow = evidence(&[lab([8, 8, 8]), lab([28, 28, 28])], 2, 1);
        let highlight = evidence(&[lab([225, 225, 225]), lab([245, 245, 245])], 2, 1);

        let shadow_cost = build_field(&shadow).horizontal(0, 0);
        let highlight_cost = build_field(&highlight).horizontal(0, 0);

        // In gamma-encoded RGB these two pairs are *identical*: 20 code
        // points apart in every channel. A cost built on encoded differences
        // would score them the same. OKLab does not, and that difference is
        // the requirement.
        assert!(
            shadow_cost > highlight_cost,
            "a 20-code-point step in shadow ({shadow_cost}) must cost more than \
             the same step in highlight ({highlight_cost}); an encoded-RGB cost \
             would rate them equal"
        );
        let ratio = f64::from(shadow_cost) / f64::from(highlight_cost);
        assert!(
            ratio > 1.25,
            "the perceptual gap must be substantial, not incidental: ratio {ratio}"
        );
    }

    /// PTE-COLOR-014: the palette participates in segmentation. Two colours
    /// close enough that colour alone would not separate them must still be
    /// separated when they carry different palette claims.
    #[test]
    fn a_palette_claim_disagreement_raises_the_cost() {
        let colors = [lab([120, 120, 120]), lab([124, 124, 124])];
        let same = evidence(&colors, 2, 1);
        let mut different = same.clone();
        different.claim = vec![LabelId(0), LabelId(1)];

        let same_cost = build_field(&same).horizontal(0, 0);
        let different_cost = build_field(&different).horizontal(0, 0);
        assert!(
            different_cost > same_cost,
            "claim disagreement must be evidence: {different_cost} vs {same_cost}"
        );
    }

    #[test]
    fn an_alpha_discontinuity_raises_the_cost() {
        let c = lab([100, 100, 100]);
        let mut e = evidence(&[c, c], 2, 1);
        let opaque = build_field(&e).horizontal(0, 0);
        e.alpha = vec![1.0, 0.0];
        let transparent = build_field(&e).horizontal(0, 0);
        assert_eq!(opaque, 0);
        assert!(transparent > 0);
    }

    /// PTE-SEG-005: a one-pixel spike that the coarser scale does not confirm
    /// must be attenuated relative to a step the coarser scale does confirm.
    #[test]
    fn a_single_pixel_spike_is_attenuated_relative_to_a_real_step() {
        let dark = lab([40, 40, 40]);
        let light = lab([200, 200, 200]);

        // A genuine step: dark dark | light light.
        let step = evidence(&[dark, dark, light, light], 4, 1);
        // A one-pixel spike: dark dark | light dark.
        let spike = evidence(&[dark, dark, light, dark], 4, 1);

        let step_cost = build_field(&step).horizontal(1, 0);
        let spike_cost = build_field(&spike).horizontal(1, 0);
        assert!(
            spike_cost < step_cost,
            "an unconfirmed spike ({spike_cost}) must cost less than a \
             confirmed step ({step_cost})"
        );
    }

    #[test]
    fn every_graph_edge_is_enumerated_exactly_once() {
        let e = evidence(&[lab([10, 10, 10]); 6], 3, 2);
        let field = build_field(&e);
        let edges: Vec<_> = field.edges().collect();
        // A 3x2 grid on the 8-connected graph has 2*2 horizontal, 3*1
        // vertical, 2 down-right and 2 down-left edges.
        assert_eq!(edges.len(), 4 + 3 + 2 + 2);
        let mut pairs: Vec<(usize, usize)> = edges.iter().map(|&(_, a, b)| (a, b)).collect();
        pairs.sort_unstable();
        pairs.dedup();
        assert_eq!(pairs.len(), 11, "no edge may be enumerated twice");
        assert!(
            pairs.iter().all(|&(a, b)| a < b),
            "edges must be canonical: {pairs:?}"
        );
    }

    #[test]
    fn quantisation_is_monotone_and_saturating() {
        assert_eq!(quantise(0.0), 0);
        assert_eq!(quantise(1.0), MAX_WEIGHT);
        assert_eq!(quantise(2.0), MAX_WEIGHT);
        assert_eq!(quantise(-1.0), 0);
        assert!(quantise(0.4) < quantise(0.6));
    }
}
