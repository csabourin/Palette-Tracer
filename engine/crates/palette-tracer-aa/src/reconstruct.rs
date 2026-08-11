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
//! ones: the per-chain solve pins endpoints, then a second solve moves each
//! eligible junction once and copies that exact coordinate to every incident
//! chain (PTE-TOPO-011/012). Every move is confined to its source trust region
//! and must preserve cyclic order and avoid non-incident crossings
//! (PTE-TOPO-013). An infinite penalty is a constraint, and a constraint is
//! cheaper and cannot be traded away by a large enough smoothness term.
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

use crate::normal::{Normal, TANGENT_WINDOW, normals};
use crate::square::{Octant, offset_for_coverage};
use palette_tracer_color::alpha::PremulLinearRgba;
use palette_tracer_color::mixture::{MixturePolicy, barycentric, two_color_best};
use palette_tracer_color::sampler::sample_premultiplied;
use palette_tracer_core::control::{TraceControl, check_cancel};
use palette_tracer_core::determinism::QuantKey;
use palette_tracer_core::error::TraceError;
use palette_tracer_core::image::ImageView;
use palette_tracer_core::ir::{
    BoundaryEvidence, CurveChain, FaceId, PathSegment, Point, Topology, VertexId,
};
use std::collections::BTreeSet;

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
    /// Multi-colour junctions moved by one shared solve.
    pub reconstructed_junctions: u32,
    /// Junctions with multi-colour evidence that failed a geometric guard.
    pub low_confidence_junctions: u32,
    /// Junctions that offered no usable multi-colour evidence.
    pub crisp_junctions: u32,
}

/// One sample's coverage constraint: the boundary lies on `n·x = target`.
#[derive(Debug, Clone, Copy)]
struct Constraint {
    target: f64,
    confidence: f64,
}

/// One incident boundary's line constraint near a junction.
#[derive(Debug, Clone, Copy)]
struct JunctionLine {
    nx: f64,
    ny: f64,
    target: f64,
    confidence: f64,
}

#[derive(Debug, Clone, Copy)]
struct JunctionEndpoint {
    line: JunctionLine,
    original_near: Point,
}

/// Coverage lines retained from the two ends of one shared edge.
#[derive(Debug, Clone, Copy, Default)]
struct EndpointLines {
    start: Option<JunctionEndpoint>,
    end: Option<JunctionEndpoint>,
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

/// Keep the nearest usable coverage line to each chain endpoint.
fn endpoint_lines(
    points: &[Point],
    normals: &[Option<Normal>],
    constraints: &[Option<Constraint>],
) -> EndpointLines {
    let line = |i: usize| match (normals[i], constraints[i]) {
        (Some(normal), Some(constraint)) => Some(JunctionLine {
            // `constraint_at` uses the opposite of `Normal` as its line
            // normal; retain that convention exactly.
            nx: -normal.x,
            ny: -normal.y,
            target: constraint.target,
            confidence: constraint.confidence,
        }),
        _ => None,
    };
    let window = (TANGENT_WINDOW + 2).min(normals.len());
    let start = (0..window).find_map(line).map(|line| JunctionEndpoint {
        line,
        original_near: points[1],
    });
    let end = (normals.len().saturating_sub(window)..normals.len())
        .rev()
        .find_map(line)
        .map(|line| JunctionEndpoint {
            line,
            original_near: points[points.len() - 2],
        });
    EndpointLines { start, end }
}

/// Evidence that a raster cell really contains three or four region colours.
///
/// Barycentric weights are deliberately a gate and confidence term, not a
/// direct position. A set of area fractions does not identify the point where
/// the incident boundaries meet; the incident two-colour lines do (§10.4,
/// PTE-AA-006).
fn junction_mixture_confidence(
    image: &ImageView<'_>,
    anchor: Point,
    colors: &[PremulLinearRgba],
    options: &ReconstructOptions,
) -> Option<f64> {
    if !(3..=4).contains(&colors.len()) {
        return None;
    }

    let mut best: Option<(QuantKey, f64)> = None;
    for (dx, dy) in [(-0.5, -0.5), (0.5, -0.5), (-0.5, 0.5), (0.5, 0.5)] {
        let (cx, cy) = (anchor.x + dx, anchor.y + dy);
        if QuantKey::geometry(cx).ok()? < QuantKey::geometry(0.0).ok()?
            || QuantKey::geometry(cy).ok()? < QuantKey::geometry(0.0).ok()?
        {
            continue;
        }
        let (px, py) = (cx.floor(), cy.floor());
        if px >= f64::from(image.width()) || py >= f64::from(image.height()) {
            continue;
        }
        let observed = sample_premultiplied(image, px as u32, py as u32);
        let weights = barycentric(observed, colors, options.mixture.alpha_weight)?;
        let active = weights
            .iter()
            .filter(|&&weight| {
                QuantKey::cost_or_worst(weight) >= QuantKey::cost_or_worst(COVERAGE_BAND)
            })
            .count();
        if active < 3 {
            continue;
        }

        let mut predicted = PremulLinearRgba::TRANSPARENT;
        for (&color, &weight) in colors.iter().zip(&weights) {
            predicted = predicted.plus(color.scale(weight));
        }
        let residual = observed
            .minus(predicted)
            .weighted_norm_squared(options.mixture.alpha_weight)
            .max(0.0)
            .sqrt();
        if QuantKey::cost_or_worst(residual) > QuantKey::cost_or_worst(options.max_residual) {
            continue;
        }
        let confidence = (1.0 - residual / options.max_residual).clamp(0.0, 1.0);
        let key = QuantKey::cost_or_worst(residual);
        if best.is_none_or(|(old, _)| key < old) {
            best = Some((key, confidence));
        }
    }
    best.map(|(_, confidence)| confidence)
}

fn quantized_sign(value: f64) -> i8 {
    QuantKey::geometry(value).map_or(0, |key| key.raw().signum() as i8)
}

fn segments_intersect(a: Point, b: Point, c: Point, d: Point) -> bool {
    let orient = |p: Point, q: Point, r: Point| (q - p).cross(r - p);
    let (ab_c, ab_d) = (
        quantized_sign(orient(a, b, c)),
        quantized_sign(orient(a, b, d)),
    );
    let (cd_a, cd_b) = (
        quantized_sign(orient(c, d, a)),
        quantized_sign(orient(c, d, b)),
    );
    if ab_c != 0 && ab_d != 0 && cd_a != 0 && cd_b != 0 {
        return ab_c != ab_d && cd_a != cd_b;
    }
    let on_segment = |p: Point, q: Point, r: Point| {
        let (Ok(px), Ok(py), Ok(qx), Ok(qy), Ok(rx), Ok(ry)) = (
            QuantKey::geometry(p.x),
            QuantKey::geometry(p.y),
            QuantKey::geometry(q.x),
            QuantKey::geometry(q.y),
            QuantKey::geometry(r.x),
            QuantKey::geometry(r.y),
        ) else {
            return false;
        };
        qx >= px.min(rx) && qx <= px.max(rx) && qy >= py.min(ry) && qy <= py.max(ry)
    };
    (ab_c == 0 && on_segment(a, c, b))
        || (ab_d == 0 && on_segment(a, d, b))
        || (cd_a == 0 && on_segment(c, a, d))
        || (cd_b == 0 && on_segment(c, b, d))
}

/// PTE-TOPO-013's local legality gate for one proposed shared junction.
fn legal_junction_move(
    topology: &Topology,
    vertex: VertexId,
    candidate: Point,
    incident: &[(usize, bool, Point)],
) -> bool {
    let anchor = topology.vertices[vertex.index()].position;
    if !candidate.is_finite()
        || QuantKey::geometry(anchor.distance(candidate)).map_or(true, |distance| {
            distance > QuantKey::geometry(TRUST_RADIUS).unwrap_or(distance)
        })
    {
        return false;
    }

    let mut rays = Vec::with_capacity(incident.len());
    for &(edge_index, at_start, original_near) in incident {
        let Some(points) =
            polyline_points(&topology.chains[topology.edges[edge_index].geometry.index()])
        else {
            return false;
        };
        let near = if at_start {
            points.get(1).copied()
        } else {
            points.get(points.len().saturating_sub(2)).copied()
        };
        let Some(near) = near else {
            return false;
        };
        if QuantKey::geometry(candidate.distance(near)).map_or(true, |distance| {
            distance <= QuantKey::geometry(1.0e-6).unwrap_or(distance)
        }) {
            return false;
        }
        rays.push((near, original_near - anchor, near - candidate));
    }

    // Every ordered ray pair keeps its orientation. Collinear opposite rays
    // (the cap of a T junction) additionally have to remain opposite.
    for i in 0..rays.len() {
        for j in i + 1..rays.len() {
            let old_cross = quantized_sign(rays[i].1.cross(rays[j].1));
            let new_cross = quantized_sign(rays[i].2.cross(rays[j].2));
            if old_cross != 0 && old_cross != new_cross {
                return false;
            }
            if old_cross == 0
                && quantized_sign(rays[i].1.dot(rays[j].1)) < 0
                && quantized_sign(rays[i].2.dot(rays[j].2)) >= 0
            {
                return false;
            }
        }
    }

    // Only the first segment of each incident chain changes. Reject a proper
    // crossing with every segment that does not touch the old shared vertex
    // or that first segment's other endpoint.
    for (near, _, _) in &rays {
        for chain in &topology.chains {
            let Some(points) = polyline_points(chain) else {
                return false;
            };
            for pair in points.windows(2) {
                if pair[0] == anchor || pair[1] == anchor || pair[0] == *near || pair[1] == *near {
                    continue;
                }
                if segments_intersect(candidate, *near, pair[0], pair[1]) {
                    return false;
                }
            }
        }
    }
    true
}

fn set_chain_endpoint(chain: &mut CurveChain, at_start: bool, position: Point) -> bool {
    if at_start {
        chain.start = position;
        return true;
    }
    match chain.segments.last_mut() {
        Some(PathSegment::Line { to }) => {
            *to = position;
            true
        }
        _ => false,
    }
}

/// Solve every multi-colour junction once and propagate the exact coordinate
/// to every incident shared chain (PTE-TOPO-011/012/013, PTE-AA-006).
fn optimize_junctions(
    topology: &mut Topology,
    image: &ImageView<'_>,
    face_colors: &[PremulLinearRgba],
    endpoint_evidence: &[EndpointLines],
    options: &ReconstructOptions,
    control: &dyn TraceControl,
    stats: &mut ReconstructionStats,
) -> Result<(), TraceError> {
    for vertex_index in 0..topology.vertices.len() {
        check_cancel(control, topology.edges.len() + vertex_index)?;
        if topology.vertices[vertex_index].degree < 3 {
            continue;
        }
        let vertex = VertexId(vertex_index as u32);
        let incident: Vec<(usize, bool)> = topology
            .edges
            .iter()
            .enumerate()
            .filter_map(|(edge_index, edge)| {
                if edge.start == vertex {
                    Some((edge_index, true))
                } else if edge.end == vertex {
                    Some((edge_index, false))
                } else {
                    None
                }
            })
            .collect();

        let mut faces = BTreeSet::new();
        for &(edge_index, _) in &incident {
            let edge = &topology.edges[edge_index];
            if !edge.left_face.is_exterior() {
                faces.insert(edge.left_face);
            }
            if !edge.right_face.is_exterior() {
                faces.insert(edge.right_face);
            }
        }
        let colors: Vec<_> = faces
            .into_iter()
            .filter_map(|face| face_color(face_colors, face))
            .collect();
        let anchor = topology.vertices[vertex_index].position;
        let Some(mixture_confidence) = junction_mixture_confidence(image, anchor, &colors, options)
        else {
            stats.crisp_junctions += 1;
            continue;
        };

        let endpoints: Vec<(usize, bool, JunctionEndpoint)> = incident
            .iter()
            .filter_map(|&(edge_index, at_start)| {
                let endpoint = if at_start {
                    endpoint_evidence[edge_index].start
                } else {
                    endpoint_evidence[edge_index].end
                }?;
                Some((edge_index, at_start, endpoint))
            })
            .collect();
        let lines: Vec<JunctionLine> = endpoints
            .iter()
            .map(|&(_, _, endpoint)| endpoint.line)
            .collect();
        if lines.len() < 2 {
            stats.low_confidence_junctions += 1;
            continue;
        }

        let mut xx = 0.0;
        let mut xy = 0.0;
        let mut yy = 0.0;
        let mut xt = 0.0;
        let mut yt = 0.0;
        let mut line_confidence = 0.0;
        for line in &lines {
            let weight = line.confidence * mixture_confidence;
            xx += weight * line.nx * line.nx;
            xy += weight * line.nx * line.ny;
            yy += weight * line.ny * line.ny;
            xt += weight * line.nx * line.target;
            yt += weight * line.ny * line.target;
            line_confidence += weight;
        }
        let determinant = xx.mul_add(yy, -xy * xy);
        if QuantKey::cost_or_worst(determinant.abs()) <= QuantKey::cost_or_worst(1.0e-6) {
            stats.low_confidence_junctions += 1;
            continue;
        }
        let candidate = Point::new(
            (xt.mul_add(yy, -xy * yt)) / determinant,
            (xx.mul_add(yt, -xy * xt)) / determinant,
        );
        let legal_incident: Vec<_> = endpoints
            .iter()
            .map(|&(edge_index, at_start, endpoint)| (edge_index, at_start, endpoint.original_near))
            .collect();
        if legal_incident.len() != incident.len()
            || !legal_junction_move(topology, vertex, candidate, &legal_incident)
        {
            stats.low_confidence_junctions += 1;
            continue;
        }

        topology.vertices[vertex_index].position = candidate;
        let confidence = (line_confidence / lines.len() as f64).clamp(0.0, 1.0);
        for &(edge_index, at_start) in &incident {
            let chain_id = topology.edges[edge_index].geometry;
            if set_chain_endpoint(&mut topology.chains[chain_id.index()], at_start, candidate) {
                topology.edges[edge_index].confidence.evidence =
                    BoundaryEvidence::CoverageReconstructed;
                topology.edges[edge_index].confidence.value =
                    topology.edges[edge_index].confidence.value.max(confidence);
            }
        }
        stats.reconstructed_junctions += 1;
    }
    Ok(())
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
    let mut endpoint_evidence = vec![EndpointLines::default(); topology.edges.len()];

    for (index, endpoint_slot) in endpoint_evidence.iter_mut().enumerate() {
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
        *endpoint_slot = endpoint_lines(&points, &estimated, &constraints);

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
        // This per-chain solve leaves endpoints alone. The joint pass below is
        // the only place they may move, once per shared vertex.
        topology.edges[index].confidence.evidence = BoundaryEvidence::CoverageReconstructed;
        topology.edges[index].confidence.value = confidence.clamp(0.0, 1.0);
        stats.reconstructed_edges += 1;
    }

    optimize_junctions(
        topology,
        image,
        face_colors,
        &endpoint_evidence,
        options,
        control,
        &mut stats,
    )?;

    // A reconstructed junction can upgrade an otherwise crisp incident edge.
    // Recount from the final IR so these public numbers cannot disagree with
    // the report's `boundary_sources` census.
    stats.reconstructed_edges = 0;
    stats.low_confidence_edges = 0;
    stats.crisp_edges = 0;
    for edge in &topology.edges {
        match edge.confidence.evidence {
            BoundaryEvidence::CoverageReconstructed => stats.reconstructed_edges += 1,
            BoundaryEvidence::LowConfidenceFallback => stats.low_confidence_edges += 1,
            BoundaryEvidence::CrispGrid | BoundaryEvidence::PixelArtPolicy => {
                stats.crisp_edges += 1;
            }
            _ => stats.crisp_edges += 1,
        }
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
    use palette_tracer_core::control::NoControl;
    use palette_tracer_core::ir::{
        BoundaryConfidence, ChainId, EdgeFlags, EdgeId, HalfEdgeId, SharedEdge, Vertex,
    };

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

    fn partition_raster(
        regions: &[LinearRgb],
        classify: impl Fn(f64, f64) -> usize,
    ) -> (Vec<u8>, Vec<PremulLinearRgba>) {
        let mut data = Vec::with_capacity((WIDTH * HEIGHT * 4) as usize);
        const SAMPLES: u32 = 16;
        for py in 0..HEIGHT {
            for px in 0..WIDTH {
                let mut sums = [0.0; 3];
                for sy in 0..SAMPLES {
                    for sx in 0..SAMPLES {
                        let x = f64::from(px) + (f64::from(sx) + 0.5) / f64::from(SAMPLES);
                        let y = f64::from(py) + (f64::from(sy) + 0.5) / f64::from(SAMPLES);
                        let color = regions[classify(x, y)];
                        sums[0] += color.r;
                        sums[1] += color.g;
                        sums[2] += color.b;
                    }
                }
                let scale = 1.0 / f64::from(SAMPLES * SAMPLES);
                let [r, g, b] = encode(LinearRgb::new(
                    sums[0] * scale,
                    sums[1] * scale,
                    sums[2] * scale,
                ));
                data.extend_from_slice(&[r, g, b, 255]);
            }
        }
        let mut colors = vec![PremulLinearRgba::TRANSPARENT];
        colors.extend(regions.iter().copied().map(premul));
        (data, colors)
    }

    fn t_junction_raster(junction: Point) -> (Vec<u8>, Vec<PremulLinearRgba>) {
        let regions = [
            LinearRgb::new(0.05, 0.08, 0.14),
            LinearRgb::new(0.78, 0.12, 0.08),
            LinearRgb::new(0.10, 0.62, 0.25),
        ];
        partition_raster(&regions, |x, y| {
            if y < junction.y {
                0
            } else if x < junction.x {
                1
            } else {
                2
            }
        })
    }

    fn x_junction_raster(junction: Point) -> (Vec<u8>, Vec<PremulLinearRgba>) {
        let regions = [
            LinearRgb::new(0.05, 0.08, 0.14),
            LinearRgb::new(0.78, 0.12, 0.08),
            LinearRgb::new(0.10, 0.62, 0.25),
            LinearRgb::new(0.12, 0.22, 0.78),
        ];
        partition_raster(&regions, |x, y| match (x < junction.x, y < junction.y) {
            (true, true) => 0,
            (false, true) => 1,
            (false, false) => 2,
            (true, false) => 3,
        })
    }

    fn t_junction_topology() -> Topology {
        let centre = Point::new(10.0, 11.0);
        let ends = [
            Point::new(2.0, 11.0),
            Point::new(22.0, 11.0),
            Point::new(10.0, 22.0),
        ];
        let left: Vec<_> = (2..=10).map(|x| Point::new(f64::from(x), 11.0)).collect();
        let right: Vec<_> = (10..=22).map(|x| Point::new(f64::from(x), 11.0)).collect();
        let down: Vec<_> = (11..=22).map(|y| Point::new(10.0, f64::from(y))).collect();
        let chains = [&left, &right, &down]
            .into_iter()
            .map(|points| CurveChain::polyline(points).unwrap())
            .collect();
        let pairs = [
            (FaceId(1), FaceId(2), VertexId(1), VertexId(0)),
            (FaceId(1), FaceId(3), VertexId(0), VertexId(2)),
            (FaceId(3), FaceId(2), VertexId(0), VertexId(3)),
        ];
        let edges = pairs
            .into_iter()
            .enumerate()
            .map(|(i, (left_face, right_face, start, end))| SharedEdge {
                id: EdgeId(i as u32),
                left_face,
                right_face,
                geometry: ChainId(i as u32),
                start,
                end,
                confidence: BoundaryConfidence::crisp(),
                flags: EdgeFlags::default(),
            })
            .collect();
        let mut vertices = vec![Vertex {
            position: centre,
            outgoing: HalfEdgeId(0),
            degree: 3,
        }];
        vertices.extend(ends.into_iter().map(|position| Vertex {
            position,
            outgoing: HalfEdgeId(0),
            degree: 1,
        }));
        Topology {
            vertices,
            edges,
            chains,
            ..Topology::default()
        }
    }

    fn x_junction_topology() -> Topology {
        let centre = Point::new(10.0, 11.0);
        let ends = [
            Point::new(2.0, 11.0),
            Point::new(22.0, 11.0),
            Point::new(10.0, 2.0),
            Point::new(10.0, 22.0),
        ];
        let paths: [Vec<Point>; 4] = [
            (2..=10).map(|x| Point::new(f64::from(x), 11.0)).collect(),
            (10..=22).map(|x| Point::new(f64::from(x), 11.0)).collect(),
            (2..=11).map(|y| Point::new(10.0, f64::from(y))).collect(),
            (11..=22).map(|y| Point::new(10.0, f64::from(y))).collect(),
        ];
        let chains = paths
            .iter()
            .map(|points| CurveChain::polyline(points).unwrap())
            .collect();
        let pairs = [
            (FaceId(1), FaceId(4), VertexId(1), VertexId(0)),
            (FaceId(2), FaceId(3), VertexId(0), VertexId(2)),
            (FaceId(2), FaceId(1), VertexId(3), VertexId(0)),
            (FaceId(3), FaceId(4), VertexId(0), VertexId(4)),
        ];
        let edges = pairs
            .into_iter()
            .enumerate()
            .map(|(i, (left_face, right_face, start, end))| SharedEdge {
                id: EdgeId(i as u32),
                left_face,
                right_face,
                geometry: ChainId(i as u32),
                start,
                end,
                confidence: BoundaryConfidence::crisp(),
                flags: EdgeFlags::default(),
            })
            .collect();
        let mut vertices = vec![Vertex {
            position: centre,
            outgoing: HalfEdgeId(0),
            degree: 4,
        }];
        vertices.extend(ends.into_iter().map(|position| Vertex {
            position,
            outgoing: HalfEdgeId(0),
            degree: 1,
        }));
        Topology {
            vertices,
            edges,
            chains,
            ..Topology::default()
        }
    }

    /// PTE-AA-006 and PTE-TOPO-011/012: three-colour evidence gates one
    /// joint solve, and the resulting coordinate is copied exactly to all
    /// three incident chains rather than solved independently per edge.
    #[test]
    fn a_multicolour_junction_is_optimized_once_and_shared_exactly() {
        let truth = Point::new(10.35, 10.62);
        let (data, colors) = t_junction_raster(truth);
        let image = ImageView::rgba8(WIDTH, HEIGHT, &data).unwrap();
        let mut topology = t_junction_topology();

        let stats = reconstruct(
            &mut topology,
            &image,
            &colors,
            &ReconstructOptions::default(),
            &NoControl,
        )
        .unwrap();

        let moved = topology.vertices[0].position;
        assert_eq!(
            stats.reconstructed_junctions, 1,
            "stats={stats:?}, moved={moved:?}"
        );
        assert!(
            moved.distance(truth) < 0.08,
            "junction {moved:?} should recover {truth:?}"
        );
        assert_eq!(topology.chains[0].end(), moved);
        assert_eq!(topology.chains[1].start, moved);
        assert_eq!(topology.chains[2].start, moved);
        assert!(
            topology.edges.iter().all(|edge| {
                edge.confidence.evidence == BoundaryEvidence::CoverageReconstructed
            })
        );
    }

    /// The four-colour branch of §10.4 uses the same shared solve and exact
    /// endpoint propagation as the three-colour T junction.
    #[test]
    fn a_four_colour_junction_is_reconstructed_as_one_shared_vertex() {
        let truth = Point::new(10.35, 10.62);
        let (data, colors) = x_junction_raster(truth);
        let image = ImageView::rgba8(WIDTH, HEIGHT, &data).unwrap();
        let mut topology = x_junction_topology();

        let stats = reconstruct(
            &mut topology,
            &image,
            &colors,
            &ReconstructOptions::default(),
            &NoControl,
        )
        .unwrap();

        let moved = topology.vertices[0].position;
        assert_eq!(stats.reconstructed_junctions, 1, "{stats:?}");
        assert!(moved.distance(truth) < 0.08, "{moved:?} vs {truth:?}");
        assert_eq!(topology.chains[0].end(), moved);
        assert_eq!(topology.chains[1].start, moved);
        assert_eq!(topology.chains[2].end(), moved);
        assert_eq!(topology.chains[3].start, moved);
    }

    /// PTE-TOPO-013: a candidate that crosses an unrelated segment is not a
    /// legal improvement even when it remains inside the trust radius.
    #[test]
    fn a_junction_move_that_crosses_a_nonincident_edge_is_rejected() {
        let mut topology = t_junction_topology();
        topology
            .chains
            .push(CurveChain::polyline(&[Point::new(10.2, 10.7), Point::new(10.2, 11.3)]).unwrap());
        let incident = vec![
            (0, false, Point::new(9.0, 11.0)),
            (1, true, Point::new(11.0, 11.0)),
            (2, true, Point::new(10.0, 12.0)),
        ];
        assert!(!legal_junction_move(
            &topology,
            VertexId(0),
            Point::new(10.4, 10.8),
            &incident
        ));
    }

    /// PTE-TOPO-013: even a subpixel move inside the trust disk is illegal if
    /// one incident ray would pass another in the cyclic order.
    #[test]
    fn a_junction_move_that_reorders_incident_edges_is_rejected() {
        let topology = t_junction_topology();
        let incident = vec![
            (0, false, Point::new(9.0, 11.0)),
            (1, true, Point::new(11.0, 11.0)),
            (2, true, Point::new(10.0, 12.0)),
        ];
        assert!(!legal_junction_move(
            &topology,
            VertexId(0),
            Point::new(10.7, 11.7),
            &incident
        ));
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

    /// PTE-TOPO-012: the independent per-chain solve pins endpoints. Only the
    /// later joint solve may move one, and then only as one shared vertex.
    #[test]
    fn the_per_chain_solve_does_not_move_endpoints() {
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
