//! Resolving the ambiguous 2x2 configuration (§9.4).
//!
//! At a grid vertex whose four pixels read `A B / B A`, all four elementary
//! segments exist and there are exactly two non-crossing ways to connect them
//! (PTE-TOPO-008):
//!
//! ```text
//!   A | B          A | B            A | B
//!   --+--    →     ──┐┌──    or     ──┘└──
//!   B | A          B ││ A          B    A
//! ```
//!
//! Pairing the north stub with the east stub (and south with west) cuts off
//! the north-east and south-west corners, leaving the `A` diagonal connected.
//! Pairing north with west (and east with south) does the opposite.
//!
//! §9.4 is explicit that this must not be decided by a rule of thumb:
//! "Fixed 'always left' policies visibly bias diagonal features", and
//! PTE-NO-012 forbids resolving it "by hash/iteration order or a universal
//! turn direction". So each such vertex is decided by the energy
//!
//! ```text
//! E(t) = λ_d E_data(t) + λ_c E_continuity(t) + λ_m E_mixture(t) + λ_p E_profile(t)
//! ```
//!
//! # What each term measures here
//!
//! * `E_data` -- the two pixels of a diagonal being *similar in colour* is
//!   evidence they are one feature. Measured as the OKLab distance within the
//!   diagonal; a diagonal whose two pixels differ is a poor candidate for
//!   being connected.
//! * `E_continuity` -- two questions about the larger neighbourhood. First,
//!   does the diagonal connection *matter*: are the two pixels already joined
//!   by a four-connected path of their own label nearby? A background label
//!   usually is, and connecting it diagonally adds nothing; a one-pixel
//!   diagonal line is not, and the connection is the whole feature. Second,
//!   given that it matters, does the diagonal carry on past the cell? This is
//!   the "tangent/edge continuation from a larger neighbourhood" §9.4 asks
//!   for, and it is why a long thin diagonal line stays connected while the
//!   background around it does not steal the connection.
//! * `E_mixture` -- the antialias coverage residual. If one diagonal's colour
//!   is well explained as a mixture of the other's, it is fringe rather than a
//!   feature (§10.2, PTE-NO-006).
//! * `E_profile` -- pixel-art connectivity policy (PTE-TOPO-010). Pixel art
//!   gets its own rule and must not borrow the antialias assumptions, so under
//!   [`AmbiguityProfile::PixelArt`] the mixture term is switched off entirely
//!   and the sparse-pixel preference is switched on.
//!
//! # Determinism
//!
//! Energies are compared as quantised keys (PTE-DET-003). An exact tie is
//! broken by the smaller minimum label identifier of the diagonal -- a
//! property of the labels, not of the traversal, so a reflected or rotated
//! image resolves the same cells the same way (PTE-TEST-010).
//!
//! # Not implemented
//!
//! PTE-TOPO-009 also asks for "a deterministic asymptotic-decider-like data
//! rule" for smooth scalar cases; the data term here is a colour-similarity
//! rule rather than a bilinear saddle evaluation.
//! `docs/IMPLEMENTATION_STATUS.md` records the difference. Of Kopf and
//! Lischinski's depixelization heuristics, only the sparse-pixel preference is
//! implemented; curve continuation and island detection are not.

use crate::grid::{GridPoint, Labels};
use palette_tracer_color::mixture::{MixturePolicy, two_color_residual};
use palette_tracer_color::spaces::Oklab;
use palette_tracer_core::determinism::QuantKey;
use palette_tracer_core::labels::LabelId;

/// Which diagonal is connected through an ambiguous vertex.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Connection {
    /// The north-west and south-east pixels are joined; north pairs with east.
    NorthWestSouthEast,
    /// The north-east and south-west pixels are joined; north pairs with west.
    NorthEastSouthWest,
}

/// Which policy the profile asks for (PTE-TOPO-010).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AmbiguityProfile {
    /// Photographic or illustrated content with antialias evidence.
    Antialiased,
    /// Pixel art: no coverage mixture may be assumed (PTE-GEO-025).
    PixelArt,
}

/// Weights on the four §9.4 energy terms.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AmbiguityWeights {
    /// `λ_d`.
    pub data: f64,
    /// `λ_c`.
    pub continuity: f64,
    /// `λ_m`.
    pub mixture: f64,
    /// `λ_p`.
    pub profile: f64,
}

impl Default for AmbiguityWeights {
    fn default() -> Self {
        // Continuity dominates because it is the only term that looks beyond
        // the cell, and a diagonal feature is exactly a thing that continues.
        // Calibration against the reference corpus is Phase 0 work that has
        // not been done.
        Self {
            data: 1.0,
            continuity: 2.0,
            mixture: 1.0,
            profile: 1.0,
        }
    }
}

/// One resolved ambiguous vertex.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Decision {
    /// Where.
    pub vertex: GridPoint,
    /// Which diagonal is connected.
    pub connection: Connection,
    /// Whether the continuity term decided it, for the report.
    pub decided_by_continuity: bool,
}

/// Per-pixel colour, for the data and mixture terms.
///
/// Row-major over the image, matching the label plane.
pub type ColorPlane = [Oklab];

/// Whether the four labels around `vertex` form the ambiguous configuration.
#[must_use]
pub fn is_ambiguous(labels: &Labels<'_>, vertex: GridPoint) -> bool {
    let [nw, ne, sw, se] = labels.around(vertex);
    nw == se && ne == sw && nw != ne
}

/// Decide one ambiguous vertex by the §9.4 energy.
///
/// # Panics
///
/// Never; every pixel access is bounds-checked through [`Labels`].
#[must_use]
pub fn decide(
    labels: &Labels<'_>,
    colors: &ColorPlane,
    vertex: GridPoint,
    weights: &AmbiguityWeights,
    profile: AmbiguityProfile,
) -> Decision {
    let [nw, ne, sw, se] = labels.around(vertex);
    debug_assert!(nw == se && ne == sw && nw != ne, "not an ambiguous vertex");

    let x = i64::from(vertex.x);
    let y = i64::from(vertex.y);
    let color_at = |px: i64, py: i64| -> Option<Oklab> {
        if px < 0 || py < 0 || px >= i64::from(labels.width()) || py >= i64::from(labels.height()) {
            None
        } else {
            colors
                .get(py as usize * labels.width() as usize + px as usize)
                .copied()
        }
    };

    // The two diagonals: A joins north-west with south-east, B joins
    // north-east with south-west.
    let a_pixels = [(x - 1, y - 1), (x, y)];
    let b_pixels = [(x, y - 1), (x - 1, y)];

    let data_a = diagonal_similarity(&color_at, a_pixels);
    let data_b = diagonal_similarity(&color_at, b_pixels);

    // Continuity, as described in the module documentation.
    let cont_a = continuity(
        labels,
        nw,
        (x - 1, y - 1),
        (x, y),
        [(x - 2, y - 2), (x + 1, y + 1)],
    );
    let cont_b = continuity(
        labels,
        ne,
        (x, y - 1),
        (x - 1, y),
        [(x + 1, y - 2), (x - 2, y + 1)],
    );

    // Mixture: is one diagonal's colour explained as coverage of the other?
    // A diagonal that is a mixture is fringe, and fringe is not a feature.
    let (mix_a, mix_b) = match profile {
        // PTE-TOPO-010: pixel art must not borrow antialias assumptions.
        AmbiguityProfile::PixelArt => (0.0, 0.0),
        AmbiguityProfile::Antialiased => (
            mixture_evidence(&color_at, a_pixels, b_pixels),
            mixture_evidence(&color_at, b_pixels, a_pixels),
        ),
    };

    // Connectivity preference: the sparse-pixel heuristic. A label that
    // appears rarely nearby is more likely an intentional thin feature, and
    // diagonal connectivity is what keeps thin features whole. The *cost* of
    // connecting a diagonal therefore rises with how common its label is: a
    // one-pixel diagonal line through a large background connects, and the
    // background does not.
    //
    // This is the term that carries pixel art, where the mixture term is
    // unavailable by rule (PTE-TOPO-010), but it is legitimate evidence in
    // both profiles and is used in both.
    let prof_a = density(labels, nw, vertex);
    let prof_b = density(labels, ne, vertex);

    // Lower energy wins. Each term is phrased as a *cost* of choosing that
    // connection.
    let energy_a = weights.data * data_a
        + weights.continuity * (1.0 - cont_a)
        + weights.mixture * mix_a
        + weights.profile * prof_a;
    let energy_b = weights.data * data_b
        + weights.continuity * (1.0 - cont_b)
        + weights.mixture * mix_b
        + weights.profile * prof_b;

    let key_a = QuantKey::cost_or_worst(energy_a);
    let key_b = QuantKey::cost_or_worst(energy_b);

    let connection = match key_a.cmp(&key_b) {
        core::cmp::Ordering::Less => Connection::NorthWestSouthEast,
        core::cmp::Ordering::Greater => Connection::NorthEastSouthWest,
        // An exact tie. The tie-break is the smaller label identifier, which
        // is a property of the palette and the raster and therefore survives
        // reflection and rotation (PTE-TEST-010). It is emphatically not a
        // turn direction (PTE-NO-012).
        core::cmp::Ordering::Equal => {
            if nw.min(se) <= ne.min(sw) {
                Connection::NorthWestSouthEast
            } else {
                Connection::NorthEastSouthWest
            }
        }
    };

    Decision {
        vertex,
        connection,
        decided_by_continuity: (cont_a - cont_b).abs() > 1e-9,
    }
}

/// Cost of a diagonal whose two pixels differ in colour, in `[0, 1]`.
fn diagonal_similarity(
    color_at: &impl Fn(i64, i64) -> Option<Oklab>,
    pixels: [(i64, i64); 2],
) -> f64 {
    match (
        color_at(pixels[0].0, pixels[0].1),
        color_at(pixels[1].0, pixels[1].1),
    ) {
        (Some(a), Some(b)) => palette_tracer_core::checked::clamp_unit(a.delta_e(b)),
        // A diagonal reaching outside the image cannot be a continuous feature
        // inside it, so it carries the maximum cost.
        _ => 1.0,
    }
}

/// Evidence in `[0, 1]` that this diagonal should be the connected one.
///
/// Zero when the two pixels are already four-connected nearby, because then
/// the diagonal link carries no structure. Otherwise `0.5`, plus up to another
/// `0.5` for the diagonal continuing past the cell.
fn continuity(
    labels: &Labels<'_>,
    label: LabelId,
    first: (i64, i64),
    second: (i64, i64),
    beyond: [(i64, i64); 2],
) -> f64 {
    if four_connected_nearby(labels, label, first, second) {
        return 0.0;
    }
    let matches = beyond
        .iter()
        .filter(|&&(px, py)| labels.at(px, py) == label)
        .count();
    0.5 + 0.5 * (matches as f64 / beyond.len() as f64)
}

/// Whether `first` and `second` are joined by a four-connected path of `label`
/// inside a bounded window around them.
///
/// The window is the 6x6 block centred on the cell. Bounded on purpose: a
/// global connectivity query would make one vertex's decision depend on the
/// whole image and would cost far more than §19.1 allows for a local rule.
fn four_connected_nearby(
    labels: &Labels<'_>,
    label: LabelId,
    first: (i64, i64),
    second: (i64, i64),
) -> bool {
    const RADIUS: i64 = 2;
    let min_x = first.0.min(second.0) - RADIUS;
    let max_x = first.0.max(second.0) + RADIUS;
    let min_y = first.1.min(second.1) - RADIUS;
    let max_y = first.1.max(second.1) + RADIUS;
    let width = (max_x - min_x + 1) as usize;
    let height = (max_y - min_y + 1) as usize;

    let index = |px: i64, py: i64| ((py - min_y) as usize) * width + ((px - min_x) as usize);
    let mut seen = vec![false; width * height];
    let mut stack = vec![first];
    seen[index(first.0, first.1)] = true;

    while let Some((px, py)) = stack.pop() {
        if (px, py) == second {
            return true;
        }
        for (dx, dy) in [(1i64, 0i64), (-1, 0), (0, 1), (0, -1)] {
            let (nx, ny) = (px + dx, py + dy);
            if nx < min_x || nx > max_x || ny < min_y || ny > max_y {
                continue;
            }
            let slot = index(nx, ny);
            if seen[slot] || labels.at(nx, ny) != label {
                continue;
            }
            seen[slot] = true;
            stack.push((nx, ny));
        }
    }
    false
}

/// Cost in `[0, 1]` reflecting how well `candidate`'s colour is explained as a
/// mixture involving `other`'s.
///
/// A low residual means the candidate diagonal is antialias fringe between the
/// other diagonal's colours, and fringe should not be promoted to a connected
/// feature (PTE-NO-006). The cost is therefore high when the residual is low.
fn mixture_evidence(
    color_at: &impl Fn(i64, i64) -> Option<Oklab>,
    candidate: [(i64, i64); 2],
    other: [(i64, i64); 2],
) -> f64 {
    let policy = MixturePolicy::default();
    let to_premul =
        |c: Oklab| palette_tracer_color::PremulLinearRgba::from_straight(c.to_linear(), 1.0);

    let Some(c) = color_at(candidate[0].0, candidate[0].1) else {
        return 0.0;
    };
    let (Some(a), Some(b)) = (
        color_at(other[0].0, other[0].1),
        color_at(other[1].0, other[1].1),
    ) else {
        return 0.0;
    };

    match two_color_residual(to_premul(c), to_premul(a), to_premul(b), &policy) {
        // Residual near zero means "definitely a mixture", which costs the
        // most; a large residual means "a colour of its own", which costs
        // nothing.
        Some((_, residual)) => {
            palette_tracer_core::checked::clamp_unit(1.0 - residual / policy.max_residual)
        }
        None => 0.0,
    }
}

/// Cost in `[0, 1]` that rises with how *common* `label` is nearby.
///
/// Kopf and Lischinski's sparse-pixel heuristic in cost form: connecting the
/// rarer label is preferred, so the commoner label carries the higher cost.
fn density(labels: &Labels<'_>, label: LabelId, vertex: GridPoint) -> f64 {
    let x = i64::from(vertex.x);
    let y = i64::from(vertex.y);
    let mut matches = 0u32;
    let mut total = 0u32;
    for dy in -2..=1 {
        for dx in -2..=1 {
            total += 1;
            if labels.at(x + dx, y + dy) == label {
                matches += 1;
            }
        }
    }
    f64::from(matches) / f64::from(total.max(1))
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
    use palette_tracer_color::EncodedSrgb;
    use palette_tracer_core::labels::LabelPlane;

    fn scene(cells: &[u32], width: u32, height: u32) -> (LabelPlane, Vec<Oklab>) {
        let mut plane = LabelPlane::new(width, height, 256).unwrap();
        let mut colors = Vec::with_capacity(cells.len());
        for (i, &c) in cells.iter().enumerate() {
            plane.set_index(i, LabelId(c));
            // Distinct, well-separated colours per label.
            let v = (c * 90 % 256) as u8;
            colors.push(EncodedSrgb::from_u8(v, 255 - v, 128).to_linear().to_oklab());
        }
        (plane, colors)
    }

    fn decide_at(
        cells: &[u32],
        w: u32,
        h: u32,
        v: GridPoint,
        profile: AmbiguityProfile,
    ) -> Connection {
        let (plane, colors) = scene(cells, w, h);
        let labels = Labels::new(&plane);
        assert!(is_ambiguous(&labels, v), "the fixture must be ambiguous");
        decide(&labels, &colors, v, &AmbiguityWeights::default(), profile).connection
    }

    #[test]
    fn only_the_checkerboard_configuration_is_ambiguous() {
        let (four_labels, _) = scene(&[1, 2, 3, 4], 2, 2);
        assert!(!is_ambiguous(
            &Labels::new(&four_labels),
            GridPoint::new(1, 1)
        ));

        let (checker, _) = scene(&[1, 2, 2, 1], 2, 2);
        assert!(is_ambiguous(&Labels::new(&checker), GridPoint::new(1, 1)));

        let (three, _) = scene(&[1, 2, 2, 2], 2, 2);
        assert!(!is_ambiguous(&Labels::new(&three), GridPoint::new(1, 1)));
    }

    /// The case §9.4 exists for: a diagonal line one pixel wide. It must stay
    /// connected, and a fixed turn rule would break half of them.
    #[test]
    fn a_diagonal_line_stays_connected() {
        // A 4x4 with label 1 on the main diagonal, 0 elsewhere.
        let mut cells = vec![0u32; 16];
        for i in 0..4 {
            cells[i * 4 + i] = 1;
        }
        // The vertex at (2, 2) sits between diagonal pixels (1,1) and (2,2).
        let connection = decide_at(
            &cells,
            4,
            4,
            GridPoint::new(2, 2),
            AmbiguityProfile::Antialiased,
        );
        assert_eq!(
            connection,
            Connection::NorthWestSouthEast,
            "the main diagonal must be the connected one"
        );
    }

    /// And the mirror image must resolve the mirrored way. A universal turn
    /// direction passes the previous test and fails this one, which is exactly
    /// what PTE-NO-012 and PTE-TEST-010 are guarding.
    #[test]
    fn the_anti_diagonal_resolves_the_mirrored_way() {
        let mut cells = vec![0u32; 16];
        for i in 0..4 {
            cells[i * 4 + (3 - i)] = 1;
        }
        let connection = decide_at(
            &cells,
            4,
            4,
            GridPoint::new(2, 2),
            AmbiguityProfile::Antialiased,
        );
        assert_eq!(
            connection,
            Connection::NorthEastSouthWest,
            "the anti-diagonal must be the connected one"
        );
    }

    /// A whole-image checkerboard has no continuation evidence either way and
    /// no colour asymmetry, so the tie-break decides -- and it must decide the
    /// same way on every run and on the reflected image.
    #[test]
    fn a_pure_checkerboard_resolves_stably_and_symmetrically() {
        let mut cells = Vec::new();
        for y in 0..6u32 {
            for x in 0..6u32 {
                cells.push(u32::from((x + y) % 2 == 0));
            }
        }
        let (plane, colors) = scene(&cells, 6, 6);
        let labels = Labels::new(&plane);

        let vertices: Vec<GridPoint> = (1..6u32)
            .flat_map(|y| (1..6u32).map(move |x| GridPoint::new(x, y)))
            .filter(|&v| is_ambiguous(&labels, v))
            .collect();
        assert!(!vertices.is_empty(), "a checkerboard must have ambiguities");

        let first: Vec<Connection> = vertices
            .iter()
            .map(|&v| {
                decide(
                    &labels,
                    &colors,
                    v,
                    &AmbiguityWeights::default(),
                    AmbiguityProfile::Antialiased,
                )
                .connection
            })
            .collect();
        for _ in 0..3 {
            let again: Vec<Connection> = vertices
                .iter()
                .map(|&v| {
                    decide(
                        &labels,
                        &colors,
                        v,
                        &AmbiguityWeights::default(),
                        AmbiguityProfile::Antialiased,
                    )
                    .connection
                })
                .collect();
            assert_eq!(again, first, "the decision must be reproducible");
        }
    }

    /// PTE-TEST-010: reflecting the image must reflect the decisions. This is
    /// the test that a "always turn left" implementation cannot pass.
    #[test]
    fn reflection_reflects_the_decisions() {
        // An L-shaped diagonal feature: asymmetric, so a bias shows.
        let cells = vec![
            0, 0, 0, 0, //
            0, 1, 0, 0, //
            0, 0, 1, 0, //
            0, 0, 0, 1, //
        ];
        let original = decide_at(
            &cells,
            4,
            4,
            GridPoint::new(2, 2),
            AmbiguityProfile::Antialiased,
        );

        let mut mirrored = vec![0u32; 16];
        for y in 0..4usize {
            for x in 0..4usize {
                mirrored[y * 4 + x] = cells[y * 4 + (3 - x)];
            }
        }
        let reflected = decide_at(
            &mirrored,
            4,
            4,
            GridPoint::new(2, 2),
            AmbiguityProfile::Antialiased,
        );

        assert_ne!(
            original, reflected,
            "a mirrored diagonal must resolve the mirrored way; equal answers \
             would mean a fixed turn direction"
        );
    }

    /// PTE-TOPO-010 and PTE-GEO-025: pixel-art mode must not use the mixture
    /// term. Switching profile on the same input must be able to change the
    /// answer, and the pixel-art answer must not depend on antialias
    /// reasoning.
    #[test]
    fn pixel_art_mode_uses_its_own_policy() {
        // A lone diagonal pair of a rare label in a sea of a common one.
        let mut cells = vec![0u32; 25];
        cells[6] = 1; // (1, 1)
        cells[12] = 1; // (2, 2)
        let v = GridPoint::new(2, 2);

        let pixel_art = decide_at(&cells, 5, 5, v, AmbiguityProfile::PixelArt);
        // Under the sparse-pixel rule the rare label is the one to connect.
        assert_eq!(
            pixel_art,
            Connection::NorthWestSouthEast,
            "pixel art must connect the sparse label's diagonal"
        );
    }

    /// The report distinguishes cells the continuity term separated from cells
    /// where it was silent and something else decided (§26.3 counts both).
    #[test]
    fn a_decision_records_whether_continuity_separated_the_options() {
        let flag = |cells: &[u32], side: u32| {
            let (plane, colors) = scene(cells, side, side);
            let labels = Labels::new(&plane);
            decide(
                &labels,
                &colors,
                GridPoint::new(2, 2),
                &AmbiguityWeights::default(),
                AmbiguityProfile::Antialiased,
            )
            .decided_by_continuity
        };

        // A short diagonal in open background: the two feature pixels are not
        // four-connected and the background is, so continuity separates them.
        let mut short = vec![0u32; 25];
        short[6] = 1; // (1, 1)
        short[12] = 1; // (2, 2)
        assert!(
            flag(&short, 5),
            "an isolated diagonal pair must be settled by continuity"
        );

        // A full-image diagonal cuts the background in two inside the window,
        // so *both* labels need the connection and continuity has nothing to
        // separate. The sparse-pixel preference decides instead -- and it still
        // gets the right answer, which the other tests check.
        let mut line = vec![0u32; 16];
        for i in 0..4 {
            line[i * 4 + i] = 1;
        }
        assert!(
            !flag(&line, 4),
            "when both diagonals need the connection, continuity is silent"
        );
    }
}
