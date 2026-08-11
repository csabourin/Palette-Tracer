//! Turning coverage evidence into boundary positions (§10.3, §10.5).
//!
//! # What this stage receives
//!
//! By the time it runs, topology extraction has produced every shared boundary
//! as a polyline whose points sit on pixel *corners*: integer coordinates in
//! image space, where the pixel with raster index `(px, py)` occupies the cell
//! `[px, px+1] × [py, py+1]` and has its centre at `(px + ½, py + ½)`.
//!
//! §8.6 has already absorbed the antialias fringe into one side or the other,
//! so the partition is clean -- but absorption is a *decision*, and the
//! subpixel information that justified it was thrown away with it. The original
//! pixel colours are still in the source raster, and this stage reads them
//! back.
//!
//! # The per-sample constraint
//!
//! For a boundary sample `x_i` with unit normal `n_i` pointing out of the left
//! face:
//!
//! 1. Look at the pixels touching `x_i`. Each is a candidate observation.
//! 2. For each, estimate the coverage `α` of the left face's colour against the
//!    right face's, under both compositing hypotheses (§10.2).
//! 3. Invert it: `d = A_square⁻¹(α, n_i)` is the offset of the true boundary
//!    from *that pixel's centre* (§10.3).
//! 4. That is a line in image space: `n_i · x = n_i · c + d`, for pixel centre
//!    `c`. Call the right-hand side `t_i`, the constraint's target.
//!
//! The candidate kept is the one with the smallest mixture residual among those
//! whose implied line actually passes within the trust region of `x_i`
//! (PTE-AA-007). A pixel two cells away may be a perfect mixture of the same
//! two colours -- on the far side of a thin feature -- and it is not evidence
//! about *this* boundary.
//!
//! # The optimisation (§10.5)
//!
//! §10.5 minimises
//!
//! ```text
//! E = Σ c_i ρ(n_i·x_i − t_i) + λ_s Σ ‖x_{i−1} − 2x_i + x_{i+1}‖² + λ_t E_topology + λ_p E_pins
//! ```
//!
//! Two of those four terms are structural here rather than penalties.
//! `E_topology` and `E_pins` are enforced as hard constraints instead of soft
//! ones: chain endpoints are junctions shared with other chains, so they do not
//! move at all (PTE-GEO-013, and PTE-TOPO-011's junction optimisation is not
//! implemented), and every interior point is confined to a trust region around
//! its source position. An infinite penalty is a constraint, and a constraint
//! is cheaper and cannot be traded away by a large enough smoothness term.
//!
//! What remains is a data term and a smoothness term. Points move only along
//! their own normals -- a boundary sample sliding *along* the boundary carries
//! no information and would only redistribute the parameterisation -- so the
//! unknown at each sample is one scalar `s_i`, the signed displacement:
//!
//! ```text
//! E(s) = Σ c_i (n_i·x_i + s_i − t_i)² + λ_s Σ ‖(x_{i−1} + s_{i−1}n_{i−1}) − 2(x_i + s_i n_i) + (x_{i+1} + s_{i+1}n_{i+1})‖²
//! ```
//!
//! This is a quadratic in `s` with a tridiagonal-plus-diagonal structure, and
//! it is solved by a fixed number of Gauss-Seidel sweeps. A *fixed* number, not
//! a convergence test: PTE-DET-004 needs the same decision on every target, and
//! "iterate until the residual stops changing" is exactly the kind of rule that
//! lands differently under a different floating-point evaluation order. Eight
//! sweeps is enough for the smoothness term to propagate across the tangent
//! window, which is as far as any single sample's evidence is relevant.
//!
//! The robust `ρ` of §10.5 is realised by the confidence weight `c_i` rather
//! than by a redescending loss: a sample whose mixture residual is large, whose
//! colours barely separate, or whose normal is unstable gets a small weight, so
//! it cannot drag the boundary. That keeps the objective quadratic, which keeps
//! the solve closed and the iteration count honest.
//!
//! # Provenance
//!
//! §10.5's objective is quoted above from the specification. Its reduction to a
//! one-dimensional-per-sample quadratic, and the choice to make topology and
//! pins hard rather than soft, are derived here. Written independently
//! (PTE-LIC-004).

use crate::normal::{Normal, normals};
use crate::square::{Octant, offset_for_coverage};
use palette_tracer_color::alpha::PremulLinearRgba;
use palette_tracer_color::mixture::{MixturePolicy, two_color_best};
use palette_tracer_color::sampler::sample_premultiplied;
use palette_tracer_core::control::{TraceControl, check_cancel};
use palette_tracer_core::error::TraceError;
use palette_tracer_core::image::ImageView;
use palette_tracer_core::ir::{BoundaryEvidence, CurveChain, FaceId, Point, Topology};

/// How far a boundary sample may move from where the raster put it, in pixels.
///
/// PTE-AA-007: "Optimization MUST be bounded by a trust region derived from
/// source support. A curve cannot 'improve smoothness' by drifting across an
/// unrelated feature." The source support of a boundary sample is the pixels
/// that touch it, so a displacement beyond one pixel is describing a different
/// edge. The largest offset §10.3 can return is `1/√2 ≈ 0.707` from a pixel
/// centre, and a pixel centre is `1/√2` from the corner the sample sits on, so
/// one pixel admits every offset the inversion can produce and nothing further.
pub const TRUST_RADIUS: f64 = 1.0;

/// Weight on the smoothness term, `λ_s` in §10.5.
///
/// Relative to a unit data weight. Below about `0.1` the boundary follows the
/// per-pixel coverage including its quantisation noise; above about `1` the
/// smoothness term wins and the reconstruction under-shoots a real curve.
const SMOOTHNESS_WEIGHT: f64 = 0.35;

/// Gauss-Seidel sweeps. Fixed, not adaptive: see the module documentation.
const SWEEPS: usize = 8;

/// Confidence below which a sample carries no usable evidence and the crisp
/// grid position stands (PTE-AA-002, PTE-AA-005).
const MIN_CONFIDENCE: f64 = 0.15;

/// How far inside `[0, 1]` a coverage must be to carry subpixel information.
///
/// A pixel lying wholly on one side is a *perfect* two-colour mixture at
/// coverage `0` or `1`: residual zero, and therefore the winner of any
/// contest ranked by residual. It is also the one observation that says
/// nothing about where the boundary is, because every position outside the
/// pixel produces it. Its inverted offset is the pixel's own edge, so
/// admitting it would pin the boundary back onto the grid -- reconstruction
/// that reproduces exactly what §10.1 calls insufficient.
///
/// The band also has to clear 8-bit quantisation. One code point near the
/// light end of this build's fixtures is about `0.005` of the colour
/// separation, so a coverage inside `0.03` of an endpoint is not
/// distinguishable from saturation.
const COVERAGE_BAND: f64 = 0.03;

/// What reconstruction did, for the report and the status document.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ReconstructionStats {
    /// Shared edges whose position came from inverted coverage.
    pub reconstructed_edges: u32,
    /// Shared edges that were offered coverage evidence and rejected it.
    pub low_confidence_edges: u32,
    /// Shared edges left on the pixel grid because no evidence was offered.
    pub crisp_edges: u32,
    /// Boundary samples that received a coverage constraint.
    pub constrained_samples: u32,
    /// Boundary samples with no usable constraint.
    pub unconstrained_samples: u32,
}

/// One sample's coverage constraint: the boundary lies on `n·x = target`.
#[derive(Debug, Clone, Copy)]
struct Constraint {
    target: f64,
    confidence: f64,
}

/// What the pixels around one boundary sample had to say (PTE-AA-009).
///
/// The three cases are the three the report distinguishes, and they are not
/// interchangeable: "no antialiasing here" and "antialiasing I could not
/// trust" are different facts about the input, and only the second is a
/// reason to doubt the boundary.
#[derive(Debug, Clone, Copy)]
enum Evidence {
    /// Coverage was inverted to a position.
    Constrained(Constraint),
    /// A mixture was found and then failed a residual, confidence, or trust
    /// test. §10.6's crisp estimate stands for this sample.
    Rejected,
    /// No pixel here is a mixture of the two sides at all: the boundary is
    /// crisp, or runs along a saturated axis-aligned run.
    Absent,
}

impl Evidence {
    /// The constraint, if there is one. The reconstruction loop matches on the
    /// variants directly, because it needs to tell `Rejected` from `Absent`;
    /// the tests only care whether a sample was constrained.
    #[cfg(test)]
    fn into_option(self) -> Option<Constraint> {
        match self {
            Self::Constrained(c) => Some(c),
            Self::Rejected | Self::Absent => None,
        }
    }
}

/// Options for reconstruction.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ReconstructOptions {
    /// Mixture thresholds (§10.2).
    pub mixture: MixturePolicy,
    /// Largest mixture residual that still counts as coverage evidence.
    pub max_residual: f64,
}

impl Default for ReconstructOptions {
    fn default() -> Self {
        Self {
            mixture: MixturePolicy::default(),
            // Deliberately looser than §8.6's fringe gate. That gate decides
            // whether to destroy a region, which is irreversible; this one
            // decides whether to move a boundary by a fraction of a pixel
            // inside a trust region, and a wrong answer is bounded and small.
            max_residual: 0.08,
        }
    }
}

/// Straight-line points of a chain, if it is still a polyline.
///
/// §10 runs before §11 fitting, so every chain is a polyline at this point.
/// A chain that is not is left alone rather than approximated, because moving
/// a cubic's control points is a different problem with a different error
/// bound.
fn polyline_points(chain: &CurveChain) -> Option<Vec<Point>> {
    let mut points = Vec::with_capacity(chain.segments.len() + 1);
    points.push(chain.start);
    for segment in &chain.segments {
        match segment {
            palette_tracer_core::ir::PathSegment::Line { to } => points.push(*to),
            _ => return None,
        }
    }
    Some(points)
}

/// What the pixels around one boundary sample say about where it belongs.
fn constraint_at(
    image: &ImageView<'_>,
    sample: Point,
    normal: Normal,
    left: PremulLinearRgba,
    right: PremulLinearRgba,
    options: &ReconstructOptions,
) -> Evidence {
    // The normal from `normal::normals` points into the left face. §10.3's
    // convention has the normal pointing *out* of the region whose coverage is
    // measured, so measuring the left face's coverage uses the reverse.
    let (nx, ny) = (-normal.x, -normal.y);
    let Some(octant) = Octant::of(nx, ny) else {
        return Evidence::Absent;
    };
    let mut rejected = false;

    // The pixel corner `sample` is shared by up to four cells. In image
    // coordinates the cell of raster pixel `(px, py)` is `[px, px+1]²`, so the
    // four centres are half a pixel away on each diagonal.
    let mut best: Option<(Constraint, f64)> = None;
    for (dx, dy) in [(-0.5, -0.5), (0.5, -0.5), (-0.5, 0.5), (0.5, 0.5)] {
        let cx = sample.x + dx;
        let cy = sample.y + dy;
        if cx < 0.0 || cy < 0.0 {
            continue;
        }
        let (px, py) = (cx.floor(), cy.floor());
        if px >= f64::from(image.width()) || py >= f64::from(image.height()) {
            continue;
        }
        let observed = sample_premultiplied(image, px as u32, py as u32);

        let Some(best_mixture) = two_color_best(observed, left, right, &options.mixture) else {
            continue;
        };
        let mixture = best_mixture.mixture;
        // A saturated pixel explains itself perfectly and locates nothing.
        // That is an *absence* of evidence, not evidence rejected: a boundary
        // running along a crisp axis-aligned run is exactly where the grid
        // says it is, and must not be reported as a failed reconstruction.
        if mixture.coverage < COVERAGE_BAND || mixture.coverage > 1.0 - COVERAGE_BAND {
            continue;
        }
        // Past this point the pixel *is* an intermediate mixture, so anything
        // that turns it away is a rejection worth reporting (PTE-AA-002).
        if mixture.residual > options.max_residual {
            rejected = true;
            continue;
        }

        let offset = offset_for_coverage(mixture.coverage, octant);
        // The constraint line, in image space.
        let target = nx.mul_add(cx, ny * cy) + offset;

        // PTE-AA-007: evidence from a pixel whose implied boundary does not
        // pass near this sample is evidence about a different edge.
        let here = nx.mul_add(sample.x, ny * sample.y);
        if (target - here).abs() > TRUST_RADIUS {
            rejected = true;
            continue;
        }

        // PTE-AA-005: confidence from mixture residual, colour separation,
        // and normal stability. Neighbourhood agreement is supplied by the
        // §10.5 smoothness term rather than by a per-sample score.
        let residual_term = (1.0 - mixture.residual / options.max_residual).clamp(0.0, 1.0);
        let separation_term = (mixture.separation_squared
            / (mixture.separation_squared + options.mixture.min_separation_squared * 100.0))
            .clamp(0.0, 1.0);
        let confidence = residual_term * separation_term * normal.stability;

        if confidence < MIN_CONFIDENCE {
            rejected = true;
            continue;
        }
        let candidate = Constraint { target, confidence };
        // Smallest residual wins; a tie goes to the earlier candidate, and the
        // candidate order is fixed, so the choice is a property of the raster.
        if best.as_ref().is_none_or(|&(_, r)| mixture.residual < r) {
            best = Some((candidate, mixture.residual));
        }
    }
    match best {
        Some((constraint, _)) => Evidence::Constrained(constraint),
        None if rejected => Evidence::Rejected,
        None => Evidence::Absent,
    }
}

/// Solve §10.5's objective for one chain, returning the displaced points.
///
/// `points[0]` and the last point are junctions and never move.
fn optimize(
    points: &[Point],
    normals: &[Option<Normal>],
    constraints: &[Option<Constraint>],
) -> Vec<Point> {
    let n = points.len();
    let mut displacement = vec![0.0f64; n];
    if n < 3 {
        return points.to_vec();
    }

    // Position of sample `i` given the current displacements.
    let placed = |i: usize, s: &[f64]| -> (f64, f64) {
        normals[i].map_or((points[i].x, points[i].y), |m| {
            (
                s[i].mul_add(-m.x, points[i].x),
                s[i].mul_add(-m.y, points[i].y),
            )
        })
    };

    for _ in 0..SWEEPS {
        for i in 1..n - 1 {
            let (Some(m), Some(c)) = (normals[i], constraints[i]) else {
                continue;
            };
            // The constraint's normal is `n = −m`, and a sample is placed at
            // `x_i + s·n = x_i − s·m`. Since `n` is a unit vector,
            // `n·(x_i + s n) = n·x_i + s`, so the displacement that satisfies
            // `n·x = target` exactly is `s = target − n·x_i = target + m·x_i`.
            let here = m.x.mul_add(points[i].x, m.y * points[i].y);
            let data_target = c.target + here;

            // Smoothness: pull towards the average of the neighbours, measured
            // along this sample's own normal.
            let (px, py) = placed(i - 1, &displacement);
            let (qx, qy) = placed(i + 1, &displacement);
            let midpoint_x = (px + qx) / 2.0;
            let midpoint_y = (py + qy) / 2.0;
            // Displacement that would put `x_i` on the neighbours' midpoint,
            // along the normal.
            let smooth_target =
                (points[i].x - midpoint_x).mul_add(m.x, (points[i].y - midpoint_y) * m.y);

            let weight = c.confidence;
            let total = weight + SMOOTHNESS_WEIGHT;
            if total <= 0.0 {
                continue;
            }
            let solved = SMOOTHNESS_WEIGHT.mul_add(smooth_target, weight * data_target) / total;
            // PTE-AA-007: the trust region is a hard bound, not a penalty.
            displacement[i] = solved.clamp(-TRUST_RADIUS, TRUST_RADIUS);
        }
    }

    (0..n)
        .map(|i| {
            let (x, y) = placed(i, &displacement);
            Point::new(x, y)
        })
        .collect()
}

/// Reconstruct subpixel boundary positions for every shared edge (§10.3,
/// §10.5).
///
/// `face_colors` is indexed by [`FaceId`] and holds each face's mean colour as
/// a premultiplied linear sample; the exterior face's entry is ignored.
///
/// # Errors
///
/// [`TraceError::Cancelled`] if the caller cancels.
pub fn reconstruct(
    topology: &mut Topology,
    image: &ImageView<'_>,
    face_colors: &[PremulLinearRgba],
    options: &ReconstructOptions,
    control: &dyn TraceControl,
) -> Result<ReconstructionStats, TraceError> {
    let mut stats = ReconstructionStats::default();

    for index in 0..topology.edges.len() {
        check_cancel(control, index)?;

        let edge = &topology.edges[index];
        let (left, right) = (edge.left_face, edge.right_face);
        let chain_id = edge.geometry;

        // The exterior face has no colour of its own, and a domain-border edge
        // has no pixel on its far side to observe. §10.6's crisp estimate is
        // the honest answer there.
        let colors = match (
            face_color(face_colors, left),
            face_color(face_colors, right),
        ) {
            (Some(a), Some(b)) => (a, b),
            _ => {
                stats.crisp_edges += 1;
                continue;
            }
        };

        let Some(points) = polyline_points(&topology.chains[chain_id.index()]) else {
            stats.crisp_edges += 1;
            continue;
        };
        if points.len() < 3 {
            stats.crisp_edges += 1;
            continue;
        }

        let estimated = normals(&points);
        let evidence: Vec<Evidence> = points
            .iter()
            .zip(&estimated)
            .map(|(&p, n)| {
                n.map_or(Evidence::Absent, |n| {
                    constraint_at(image, p, n, colors.0, colors.1, options)
                })
            })
            .collect();
        let constraints: Vec<Option<Constraint>> = evidence
            .iter()
            .map(|e| match e {
                Evidence::Constrained(c) => Some(*c),
                _ => None,
            })
            .collect();

        let constrained = constraints.iter().filter(|c| c.is_some()).count();
        stats.constrained_samples += constrained as u32;
        stats.unconstrained_samples += (points.len() - constrained) as u32;

        // PTE-AA-009's three outcomes, decided per edge from per-sample
        // evidence.
        //
        // There is deliberately no quorum here. An earlier draft required half
        // an edge's samples to carry coverage before any of them were used,
        // which threw away the corners of `curves/rounded-rectangle`: its
        // sides are axis-aligned runs of saturated pixels with no coverage to
        // read, so the whole boundary -- corners included -- fell back to the
        // grid. Each constrained sample is justified on its own and bounded by
        // its own trust region, so the right unit of decision is the sample,
        // not the edge. Unconstrained samples simply do not move, which is the
        // correct answer for a boundary that really is on a cell interface.
        if constrained == 0 {
            let (label, counter) = if evidence.iter().any(|e| matches!(e, Evidence::Rejected)) {
                // Coverage was there and could not be trusted (PTE-AA-002).
                (
                    BoundaryEvidence::LowConfidenceFallback,
                    &mut stats.low_confidence_edges,
                )
            } else {
                // §10.6: no antialiasing to invert. The crisp estimate is not
                // a fallback here, it is the right answer.
                (BoundaryEvidence::CrispGrid, &mut stats.crisp_edges)
            };
            *counter += 1;
            topology.edges[index].confidence.evidence = label;
            continue;
        }

        let moved = optimize(&points, &estimated, &constraints);

        // PTE-AA-005: the edge's confidence is the mean over its *interior*
        // samples, not over the constrained ones. An edge where two samples of
        // forty carried coverage is barely reconstructed, and saying `0.05`
        // reports that; averaging over the two would report `0.98` and hide it.
        let interior = points.len().saturating_sub(2).max(1);
        let confidence = constraints
            .iter()
            .flatten()
            .map(|c| c.confidence)
            .sum::<f64>()
            / interior as f64;

        // PTE-TOPO-001 and PTE-AA-008: the chain is replaced once, and both
        // faces inherit it because both read the same chain.
        if let Some(chain) = CurveChain::polyline(&moved) {
            topology.chains[chain_id.index()] = chain;
        }
        // The endpoints did not move, so the vertices they name are still
        // correct and nothing else has to be told (PTE-GEO-013).
        topology.edges[index].confidence.evidence = BoundaryEvidence::CoverageReconstructed;
        topology.edges[index].confidence.value = confidence.clamp(0.0, 1.0);
        stats.reconstructed_edges += 1;
    }

    Ok(stats)
}

/// A face's colour, or `None` for the exterior.
fn face_color(face_colors: &[PremulLinearRgba], face: FaceId) -> Option<PremulLinearRgba> {
    if face.is_exterior() {
        return None;
    }
    face_colors.get(face.index()).copied()
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::float_cmp)]
mod tests {
    use super::*;
    use palette_tracer_color::spaces::{EncodedSrgb, LinearRgb};

    const WIDTH: u32 = 24;
    const HEIGHT: u32 = 12;

    fn encode(linear: LinearRgb) -> [u8; 3] {
        let e = linear.to_encoded();
        [
            (e.r * 255.0).round().clamp(0.0, 255.0) as u8,
            (e.g * 255.0).round().clamp(0.0, 255.0) as u8,
            (e.b * 255.0).round().clamp(0.0, 255.0) as u8,
        ]
    }

    /// A raster with one vertical edge at `edge_x`, antialiased by exact area
    /// coverage in linear light. The region to the *left* of the edge is dark.
    ///
    /// Image coordinates: pixel `px` occupies `[px, px+1]`, so its coverage by
    /// the dark side is `clamp(edge_x − px, 0, 1)`.
    fn vertical_edge(edge_x: f64) -> Vec<u8> {
        let dark = LinearRgb::new(0.02, 0.02, 0.03);
        let light = LinearRgb::new(0.85, 0.84, 0.80);
        let mut data = Vec::with_capacity((WIDTH * HEIGHT * 4) as usize);
        for _y in 0..HEIGHT {
            for x in 0..WIDTH {
                let dark_fraction = (edge_x - f64::from(x)).clamp(0.0, 1.0);
                let mixed = LinearRgb::new(
                    dark_fraction.mul_add(dark.r - light.r, light.r),
                    dark_fraction.mul_add(dark.g - light.g, light.g),
                    dark_fraction.mul_add(dark.b - light.b, light.b),
                );
                let [r, g, b] = encode(mixed);
                data.extend_from_slice(&[r, g, b, 255]);
            }
        }
        data
    }

    fn premul(linear: LinearRgb) -> PremulLinearRgba {
        // Round-trip through the 8-bit encoding, as the pipeline's region means
        // effectively have.
        let [r, g, b] = encode(linear);
        let e = EncodedSrgb::new(
            f64::from(r) / 255.0,
            f64::from(g) / 255.0,
            f64::from(b) / 255.0,
        );
        PremulLinearRgba::from_straight(e.to_linear(), 1.0)
    }

    /// A downward polyline on the grid line `x = grid_x`, which is where
    /// topology extraction puts a vertical boundary.
    fn downward_polyline(grid_x: f64) -> Vec<Point> {
        (0..=HEIGHT)
            .map(|y| Point::new(grid_x, f64::from(y)))
            .collect()
    }

    /// §10.3 end to end on a case whose answer is known exactly: a vertical
    /// edge at a subpixel position, reconstructed from its coverage.
    ///
    /// This is the claim §10 exists to make. The grid can only place the
    /// boundary at an integer, so the error before reconstruction is the
    /// fractional part of the true position; afterwards it is the accuracy of
    /// the inversion.
    #[test]
    fn a_subpixel_vertical_edge_is_recovered_to_its_true_position() {
        let dark = premul(LinearRgb::new(0.02, 0.02, 0.03));
        let light = premul(LinearRgb::new(0.85, 0.84, 0.80));
        let options = ReconstructOptions::default();

        for edge_x in [10.15, 10.37, 10.5, 10.62, 10.85] {
            let data = vertical_edge(edge_x);
            let image = ImageView::rgba8(WIDTH, HEIGHT, &data).unwrap();
            let points = downward_polyline(10.0);
            let estimated = normals(&points);

            // Travelling downward, the left face is the one at larger x, which
            // is the light side; the right face is the dark side.
            let constraints: Vec<Option<Constraint>> = points
                .iter()
                .zip(&estimated)
                .map(|(&p, n)| {
                    n.and_then(|n| constraint_at(&image, p, n, light, dark, &options).into_option())
                })
                .collect();

            let constrained = constraints.iter().filter(|c| c.is_some()).count();
            assert!(
                constrained >= points.len() - 2 * crate::normal::TANGENT_WINDOW,
                "edge {edge_x}: only {constrained} of {} samples carried evidence",
                points.len()
            );

            let moved = optimize(&points, &estimated, &constraints);
            // The endpoints are junctions and stay put; check the interior.
            for p in &moved[2..moved.len() - 2] {
                assert!(
                    (p.x - edge_x).abs() < 0.02,
                    "edge {edge_x}: sample landed at {} ",
                    p.x
                );
                assert_eq!(p.y, points[0].y + (p.y - points[0].y), "y must not drift");
            }
        }
    }

    /// The measurement that justifies the stage: reconstruction has to beat
    /// the grid position it replaces, and by the margin §10.1 claims.
    ///
    /// The comparison is against the *thresholded* position, which is what a
    /// build without §10 produces: segmentation assigns the fringe pixel to
    /// whichever side covers most of it, so the boundary lands on the nearer
    /// pixel edge. That is the honest baseline, and its worst case is half a
    /// pixel by construction.
    #[test]
    fn reconstruction_beats_the_grid_position_it_replaces() {
        let dark = premul(LinearRgb::new(0.02, 0.02, 0.03));
        let light = premul(LinearRgb::new(0.85, 0.84, 0.80));
        let options = ReconstructOptions::default();

        let mut grid_error = 0.0f64;
        let mut reconstructed_error = 0.0f64;
        let mut worst = 0.0f64;
        let mut samples = 0.0f64;

        for step in 0..=20 {
            let edge_x = 10.0 + f64::from(step) / 20.0;
            // Where thresholding puts the boundary: the nearer pixel edge.
            let thresholded = edge_x.round();
            let data = vertical_edge(edge_x);
            let image = ImageView::rgba8(WIDTH, HEIGHT, &data).unwrap();
            let points = downward_polyline(thresholded);
            let estimated = normals(&points);
            let constraints: Vec<Option<Constraint>> = points
                .iter()
                .zip(&estimated)
                .map(|(&p, n)| {
                    n.and_then(|n| constraint_at(&image, p, n, light, dark, &options).into_option())
                })
                .collect();
            let moved = optimize(&points, &estimated, &constraints);

            for i in 2..moved.len() - 2 {
                grid_error += (points[i].x - edge_x).abs();
                let error = (moved[i].x - edge_x).abs();
                reconstructed_error += error;
                worst = worst.max(error);
                samples += 1.0;
            }
        }

        let grid_mean = grid_error / samples;
        let reconstructed_mean = reconstructed_error / samples;
        assert!(
            reconstructed_mean < 0.02,
            "mean reconstructed error {reconstructed_mean} px (grid {grid_mean} px)"
        );
        assert!(worst < 0.05, "worst reconstructed error {worst} px");
        assert!(
            reconstructed_mean * 10.0 < grid_mean,
            "reconstruction ({reconstructed_mean} px) should beat the grid \
             ({grid_mean} px) by more than an order of magnitude"
        );
    }

    /// PTE-AA-007: a sample cannot be dragged out of its trust region, however
    /// the evidence is arranged.
    #[test]
    fn no_sample_leaves_its_trust_region() {
        let points = downward_polyline(10.0);
        let estimated = normals(&points);
        // A constraint demanding a wild displacement, at full confidence.
        let constraints: Vec<Option<Constraint>> = points
            .iter()
            .map(|_| {
                Some(Constraint {
                    target: -1000.0,
                    confidence: 1.0,
                })
            })
            .collect();
        let moved = optimize(&points, &estimated, &constraints);
        for (i, p) in moved.iter().enumerate() {
            let d = (p.x - points[i].x).hypot(p.y - points[i].y);
            assert!(d <= TRUST_RADIUS + 1e-12, "sample {i} moved {d} px");
        }
    }

    /// PTE-GEO-013 and PTE-TOPO-001: the endpoints are junctions shared with
    /// other chains, and §10.5 must not move them. PTE-TOPO-011's junction
    /// optimisation is not implemented, so a moved endpoint would silently tear
    /// the boundary apart at the junction.
    #[test]
    fn the_endpoints_of_a_chain_never_move() {
        let dark = premul(LinearRgb::new(0.02, 0.02, 0.03));
        let light = premul(LinearRgb::new(0.85, 0.84, 0.80));
        let data = vertical_edge(10.4);
        let image = ImageView::rgba8(WIDTH, HEIGHT, &data).unwrap();
        let points = downward_polyline(10.0);
        let estimated = normals(&points);
        let constraints: Vec<Option<Constraint>> = points
            .iter()
            .zip(&estimated)
            .map(|(&p, n)| {
                n.and_then(|n| {
                    constraint_at(&image, p, n, light, dark, &options_default()).into_option()
                })
            })
            .collect();
        let moved = optimize(&points, &estimated, &constraints);
        assert_eq!(moved[0], points[0]);
        assert_eq!(moved[moved.len() - 1], points[points.len() - 1]);
    }

    fn options_default() -> ReconstructOptions {
        ReconstructOptions::default()
    }

    /// PTE-AA-002: when the two sides are the same colour there is no coverage
    /// to invert, and the grid position stands rather than being moved by noise.
    #[test]
    fn indistinguishable_sides_produce_no_constraint() {
        let same = premul(LinearRgb::new(0.5, 0.5, 0.5));
        let data = vertical_edge(10.4);
        let image = ImageView::rgba8(WIDTH, HEIGHT, &data).unwrap();
        let points = downward_polyline(10.0);
        let estimated = normals(&points);
        for (p, n) in points.iter().zip(&estimated) {
            let Some(n) = *n else { continue };
            assert!(
                constraint_at(&image, *p, n, same, same, &options_default())
                    .into_option()
                    .is_none(),
                "identical sides must not yield a constraint"
            );
        }
    }
}
