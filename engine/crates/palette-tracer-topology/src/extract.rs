//! Label map to shared half-edges (§9.3, Appendix D.3).
//!
//! # The traversal rule
//!
//! Every face cycle is walked with the face on the left. Arriving at a vertex
//! through some stub, the next dart is the first one clockwise from that stub
//! that still has the same face on its left. On a square grid that is the
//! tightest turn, and it is correct for every configuration except one.
//!
//! The exception is the ambiguous 2x2 cell, where *both* the tight turn and
//! the wide turn keep the same face on the left. Taking the tight turn there
//! unconditionally is precisely the "universal turn direction" PTE-NO-012
//! forbids, so those vertices are decided first, by the §9.4 energy in
//! [`crate::ambiguity`], and the traversal obeys the decision.
//!
//! # Vertex degrees and where chains split
//!
//! A stub exists where the two pixels flanking it differ, so the degree is 0,
//! 2, 3, or 4 -- never 1. Degree 3 is the T-junction where three labels meet,
//! and it occurs both in the interior and on the domain border, where the
//! exterior label is the third. This is checked exhaustively over every 2x2
//! label pattern by `vertex_degrees_are_zero_two_three_or_four`.
//!
//! A chain therefore splits at every vertex of degree other than two, and an
//! ambiguous 2x2 cell is the one exception: its degree is four but the §9.4
//! decision turns it into two pass-throughs, so it is not a split point.
//!
//! Degree two is exactly the case where the face pair cannot change: with only
//! two stubs, the other two flanking pairs are equal, which forces the two
//! segments to separate the same two labels. So "split at degree other than
//! two" is the same rule as §9.3's "aggregate connected pieces into chains
//! between junctions", with the face pair constant along each chain.
//!
//! # Complexity
//!
//! PTE-TOPO-006 requires `O(N)` plus output. One pass over the pixels finds
//! the segments and the ambiguities; one pass over the darts builds the
//! chains; one pass over the chains builds the cycles. No step revisits a
//! dart, and the maps are keyed by grid position rather than by content.

use crate::ambiguity::{self, AmbiguityProfile, AmbiguityWeights, Connection, Decision};
use crate::grid::{Dir, GridPoint, Labels};
use palette_tracer_color::spaces::Oklab;
use palette_tracer_core::control::{TraceControl, check_cancel};
use palette_tracer_core::error::TraceError;
use palette_tracer_core::ir::{
    BoundaryConfidence, BoundaryEvidence, ChainId, CurveChain, Cycle, CycleKind, EdgeFlags, EdgeId,
    Face, FaceId, HalfEdge, HalfEdgeId, PaletteId, Point, RegionId, SharedEdge, Topology, Vertex,
    VertexId,
};
use palette_tracer_core::labels::{LabelId, LabelPlane};
use palette_tracer_core::limits::{ResourceLimits, WorkBudget, cost};
use std::collections::BTreeMap;

/// A directed elementary segment: leave `at` travelling in `dir`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
struct Dart {
    at: GridPoint,
    dir: Dir,
}

impl Dart {
    fn end(self, width: u32, height: u32) -> Option<GridPoint> {
        self.at.step(self.dir, width, height)
    }

    /// The same segment traversed the other way.
    fn reversed(self, width: u32, height: u32) -> Option<Self> {
        self.end(width, height).map(|end| Self {
            at: end,
            dir: self.dir.opposite(),
        })
    }
}

/// What the extractor produced, plus what it decided along the way.
#[derive(Debug, Clone)]
pub struct Extraction {
    /// The shared topology.
    pub topology: Topology,
    /// Ambiguous cells encountered (§26.3).
    pub ambiguous_cells: u32,
    /// Of those, how many the continuity term separated.
    pub ambiguous_resolved_by_continuity: u32,
    /// Junction vertices: degree three or four, where chains split.
    pub junctions: u32,
}

/// Options for extraction.
#[derive(Debug, Clone, Copy)]
pub struct ExtractOptions {
    /// Weights on the §9.4 energy.
    pub weights: AmbiguityWeights,
    /// Connectivity profile (PTE-TOPO-010).
    pub profile: AmbiguityProfile,
    /// Boundary evidence to record on every edge. Refinement replaces it.
    pub evidence: BoundaryEvidence,
}

impl Default for ExtractOptions {
    fn default() -> Self {
        Self {
            weights: AmbiguityWeights::default(),
            profile: AmbiguityProfile::Antialiased,
            evidence: BoundaryEvidence::CrispGrid,
        }
    }
}

/// Build the shared topology from a label plane (Appendix D.3).
///
/// `palette_of` maps a region label to the palette entry that paints it.
///
/// # Errors
///
/// [`TraceError::Cancelled`], [`TraceError::ResourceLimit`] if the boundary
/// sample count exceeds the limit, or [`TraceError::Internal`] if an invariant
/// fails while building -- which the validator would also catch.
///
/// # Panics
///
/// Never; every index is derived from a length in the same scope.
// The arity is the §5.3 stage contract; see the note on `segment`.
#[allow(clippy::too_many_arguments)]
pub fn extract(
    plane: &LabelPlane,
    colors: &[Oklab],
    palette_of: &[Option<PaletteId>],
    pinned_of: &[bool],
    options: &ExtractOptions,
    limits: &ResourceLimits,
    budget: &WorkBudget,
    control: &dyn TraceControl,
) -> Result<Extraction, TraceError> {
    let labels = Labels::new(plane);
    let width = labels.width();
    let height = labels.height();

    // 1. Decide every ambiguous cell before any traversal, so the traversal is
    //    a lookup rather than a judgement (PTE-NO-012).
    let mut decisions: BTreeMap<GridPoint, Decision> = BTreeMap::new();
    let mut resolved_by_continuity = 0u32;
    let mut step = 0usize;
    for y in 1..height {
        for x in 1..width {
            check_cancel(control, step)?;
            budget.charge(cost::PIXEL)?;
            step += 1;

            let v = GridPoint::new(x, y);
            if ambiguity::is_ambiguous(&labels, v) {
                let decision =
                    ambiguity::decide(&labels, colors, v, &options.weights, options.profile);
                if decision.decided_by_continuity {
                    resolved_by_continuity += 1;
                }
                decisions.insert(v, decision);
            }
        }
    }
    let ambiguous_cells = decisions.len() as u32;

    // 2. Find the node vertices: any degree other than zero or two, except an
    //    ambiguous cell, which the §9.4 decision turns into pass-throughs.
    let mut nodes: BTreeMap<GridPoint, VertexId> = BTreeMap::new();
    let mut junctions = 0u32;
    for y in 0..=height {
        for x in 0..=width {
            let v = GridPoint::new(x, y);
            let degree = labels.degree(v);
            if degree != 0 && degree != 2 && !decisions.contains_key(&v) {
                nodes.insert(v, VertexId(0));
                junctions += 1;
            }
        }
    }

    // 3. Trace chains between nodes.
    let mut chains: Vec<TracedChain> = Vec::new();
    let mut first_dart_of: BTreeMap<Dart, usize> = BTreeMap::new();
    let mut visited: BTreeMap<Dart, bool> = BTreeMap::new();

    // Node-anchored chains first, then any remaining closed loops. Both sweeps
    // visit candidate darts in grid order, so the chain numbering is a
    // property of the raster (PTE-API-010).
    for pass in 0..2 {
        for y in 0..=height {
            for x in 0..=width {
                let v = GridPoint::new(x, y);
                if pass == 0 && !nodes.contains_key(&v) {
                    continue;
                }
                for dir in Dir::ALL {
                    let dart = Dart { at: v, dir };
                    if labels.stub(v, dir).is_none() || visited.contains_key(&dart) {
                        continue;
                    }
                    check_cancel(control, chains.len())?;
                    budget.charge(cost::BOUNDARY_SEGMENT)?;

                    let traced = trace_chain(&labels, &decisions, &nodes, dart)?;
                    for d in &traced.darts {
                        visited.insert(*d, true);
                    }
                    first_dart_of.insert(dart, chains.len());
                    chains.push(traced);
                }
            }
        }
    }

    let sample_count: u64 = chains.iter().map(|c| c.points.len() as u64).sum();
    limits.check_boundary_samples(sample_count)?;

    // 4. Pair chains with their twins and build half-edges.
    //    Chain `i`'s twin starts at the reverse of chain `i`'s last dart.
    let mut twin_of = vec![usize::MAX; chains.len()];
    for (i, chain) in chains.iter().enumerate() {
        let Some(last) = chain.darts.last() else {
            continue;
        };
        let Some(reverse_start) = last.reversed(width, height) else {
            continue;
        };
        if let Some(&j) = first_dart_of.get(&reverse_start) {
            twin_of[i] = j;
        }
    }
    for (i, &j) in twin_of.iter().enumerate() {
        if j == usize::MAX {
            return Err(palette_tracer_core::internal_error!(
                "half_edge_without_twin",
                "chain {i} has no reverse chain; every segment is shared \
                 (PTE-TOPO-003)"
            ));
        }
    }

    // 5. Vertices: every chain endpoint.
    let mut vertex_of: BTreeMap<GridPoint, VertexId> = BTreeMap::new();
    for chain in &chains {
        for point in [chain.start, chain.end] {
            let next = VertexId(vertex_of.len() as u32);
            vertex_of.entry(point).or_insert(next);
        }
    }
    let mut vertices = vec![
        Vertex {
            position: Point::ORIGIN,
            outgoing: HalfEdgeId(0),
            degree: 0,
        };
        vertex_of.len()
    ];
    for (point, id) in &vertex_of {
        vertices[id.index()].position = point.to_point();
    }
    for (i, chain) in chains.iter().enumerate() {
        let v = vertex_of[&chain.start];
        vertices[v.index()].degree += 1;
        if vertices[v.index()].degree == 1 {
            vertices[v.index()].outgoing = HalfEdgeId(i as u32);
        }
    }

    // 6. Faces, one per region label plus the exterior at index zero.
    let region_count = palette_of.len();
    let mut faces: Vec<Face> = Vec::with_capacity(region_count + 1);
    faces.push(Face {
        id: FaceId::EXTERIOR,
        palette: None,
        exterior: true,
        // A sentinel, not region zero: region zero is a real region, and a
        // caller looking a face up by region must not find the exterior.
        region: RegionId(u32::MAX),
        cycles: Vec::new(),
        area_pixels: 0,
    });
    let areas = plane.histogram(region_count);
    for region in 0..region_count {
        faces.push(Face {
            id: FaceId(region as u32 + 1),
            palette: palette_of[region],
            exterior: false,
            region: RegionId(region as u32),
            cycles: Vec::new(),
            area_pixels: areas[region],
        });
    }
    let face_of = |label: LabelId| -> FaceId {
        if label == Labels::EXTERIOR {
            FaceId::EXTERIOR
        } else {
            FaceId(label.0 + 1)
        }
    };

    // 7. Shared edges and half-edges. Each undirected edge is created once,
    //    from the chain with the smaller index, so the forward direction is a
    //    property of the raster order.
    let mut edges: Vec<SharedEdge> = Vec::new();
    let mut ir_chains: Vec<CurveChain> = Vec::new();
    let mut edge_of_chain = vec![EdgeId(0); chains.len()];
    let mut forward_of_chain = vec![false; chains.len()];

    for (i, chain) in chains.iter().enumerate() {
        let j = twin_of[i];
        if j < i {
            continue;
        }
        let edge_id = EdgeId(edges.len() as u32);
        let chain_id = ChainId(ir_chains.len() as u32);

        let geometry = CurveChain::polyline(&chain.points).ok_or_else(|| {
            palette_tracer_core::internal_error!(
                "degenerate_chain",
                "chain {i} has fewer than two points"
            )
        })?;
        ir_chains.push(geometry);

        let left = face_of(chain.left);
        let right = face_of(chain.right);
        let pinned_interface = pinned_label(pinned_of, chain.left)
            && pinned_label(pinned_of, chain.right)
            && chain.left != chain.right;

        edges.push(SharedEdge {
            id: edge_id,
            left_face: left,
            right_face: right,
            geometry: chain_id,
            start: vertex_of[&chain.start],
            end: vertex_of[&chain.end],
            confidence: BoundaryConfidence {
                value: 1.0,
                evidence: options.evidence,
            },
            flags: EdgeFlags {
                domain_border: left.is_exterior() || right.is_exterior(),
                pinned_interface,
                visible_interface: !left.is_exterior() && !right.is_exterior(),
            },
        });

        edge_of_chain[i] = edge_id;
        forward_of_chain[i] = true;
        edge_of_chain[j] = edge_id;
        forward_of_chain[j] = false;
    }

    // Half-edge `i` is chain `i`. `next` is the chain the traversal continues
    // into, which is exactly the chain whose first dart the dart rule yields.
    let mut half_edges = Vec::with_capacity(chains.len());
    let mut next_of = vec![HalfEdgeId(0); chains.len()];
    for (i, chain) in chains.iter().enumerate() {
        let Some(last) = chain.darts.last() else {
            continue;
        };
        let successor = next_dart(&labels, &decisions, *last, chain.left, width, height)
            .and_then(|d| first_dart_of.get(&d).copied())
            .ok_or_else(|| {
                palette_tracer_core::internal_error!(
                    "cycle_does_not_close",
                    "chain {i} has no successor keeping face {:?} on the left",
                    chain.left
                )
            })?;
        next_of[i] = HalfEdgeId(successor as u32);
    }
    let mut prev_of = vec![HalfEdgeId(0); chains.len()];
    for (i, next) in next_of.iter().enumerate() {
        prev_of[next.index()] = HalfEdgeId(i as u32);
    }

    for (i, chain) in chains.iter().enumerate() {
        half_edges.push(HalfEdge {
            origin: vertex_of[&chain.start],
            twin: HalfEdgeId(twin_of[i] as u32),
            next: next_of[i],
            prev: prev_of[i],
            face: face_of(chain.left),
            edge: edge_of_chain[i],
            forward: forward_of_chain[i],
        });
    }

    // 8. Cycles. Each half-edge belongs to exactly one; the outer boundary of
    //    an interior face has negative signed area with this orientation, and
    //    holes have positive.
    let mut seen = vec![false; half_edges.len()];
    for start in 0..half_edges.len() {
        if seen[start] {
            continue;
        }
        let mut walk = Vec::new();
        let mut current = start;
        loop {
            seen[current] = true;
            walk.push(current);
            current = half_edges[current].next.index();
            if current == start {
                break;
            }
            if seen[current] {
                return Err(palette_tracer_core::internal_error!(
                    "cycle_does_not_close",
                    "half-edge {current} is reachable from two cycles"
                ));
            }
        }

        let area = signed_area(&walk, &chains);
        let face = half_edges[start].face;
        let kind = if area < 0.0 {
            CycleKind::Outer
        } else {
            CycleKind::Hole
        };
        faces[face.index()].cycles.push(Cycle {
            start: HalfEdgeId(start as u32),
            kind,
        });
    }

    // Outer cycles first within each face, so a serialiser can emit the
    // boundary before its holes without re-sorting (§18.4).
    for face in &mut faces {
        face.cycles.sort_by_key(|c| (c.kind == CycleKind::Hole, c.start.0));
    }

    Ok(Extraction {
        topology: Topology {
            vertices,
            half_edges,
            edges,
            faces,
            chains: ir_chains,
        },
        ambiguous_cells,
        ambiguous_resolved_by_continuity: resolved_by_continuity,
        junctions,
    })
}

fn pinned_label(pinned_of: &[bool], label: LabelId) -> bool {
    if label == Labels::EXTERIOR {
        false
    } else {
        pinned_of.get(label.index()).copied().unwrap_or(false)
    }
}

/// Shoelace area of the polygon a cycle traces, in image units.
fn signed_area(walk: &[usize], chains: &[TracedChain]) -> f64 {
    let mut sum = 0.0;
    for &index in walk {
        let points = &chains[index].points;
        for pair in points.windows(2) {
            sum += pair[0].x * pair[1].y - pair[1].x * pair[0].y;
        }
    }
    sum / 2.0
}

/// A chain of elementary segments between two node vertices.
#[derive(Debug, Clone)]
struct TracedChain {
    darts: Vec<Dart>,
    points: Vec<Point>,
    start: GridPoint,
    end: GridPoint,
    left: LabelId,
    right: LabelId,
}

/// Walk from `first` until a node vertex, or until the chain closes.
fn trace_chain(
    labels: &Labels<'_>,
    decisions: &BTreeMap<GridPoint, Decision>,
    nodes: &BTreeMap<GridPoint, VertexId>,
    first: Dart,
) -> Result<TracedChain, TraceError> {
    let width = labels.width();
    let height = labels.height();
    let (left, right) = labels.stub(first.at, first.dir).ok_or_else(|| {
        palette_tracer_core::internal_error!("missing_stub", "traced a dart with no segment")
    })?;

    let mut darts = vec![first];
    let mut points = vec![first.at.to_point()];
    let mut current = first;

    // The bound is the total dart count, so a corrupt neighbourhood produces
    // an error rather than a hang (PTE-SEC-008).
    let bound = 4 * (usize::try_from(width).unwrap_or(0) + 1) * (usize::try_from(height).unwrap_or(0) + 1);
    for _ in 0..bound {
        let Some(end) = current.end(width, height) else {
            return Err(palette_tracer_core::internal_error!(
                "dart_leaves_grid",
                "a segment left the grid, which the stub test should prevent"
            ));
        };
        points.push(end.to_point());

        if nodes.contains_key(&end) || end == first.at {
            return Ok(TracedChain {
                darts,
                points,
                start: first.at,
                end,
                left,
                right,
            });
        }

        let Some(next) = next_dart(labels, decisions, current, left, width, height) else {
            return Err(palette_tracer_core::internal_error!(
                "chain_dead_end",
                "no continuation keeping the same face on the left"
            ));
        };
        darts.push(next);
        current = next;
    }

    Err(palette_tracer_core::internal_error!(
        "chain_did_not_terminate",
        "a chain exceeded the total dart count"
    ))
}

/// The next dart continuing `current`'s face-on-left traversal.
fn next_dart(
    labels: &Labels<'_>,
    decisions: &BTreeMap<GridPoint, Decision>,
    current: Dart,
    face: LabelId,
    width: u32,
    height: u32,
) -> Option<Dart> {
    let end = current.end(width, height)?;
    // The stub we arrived through, as seen from `end`.
    let arrival = current.dir.opposite();

    if let Some(decision) = decisions.get(&end) {
        // The pairing is a decision, not a turn preference (PTE-NO-012).
        let partner = match decision.connection {
            // North pairs with east, south with west.
            Connection::NorthWestSouthEast => match arrival {
                Dir::North => Dir::East,
                Dir::East => Dir::North,
                Dir::South => Dir::West,
                Dir::West => Dir::South,
            },
            // North pairs with west, east with south.
            Connection::NorthEastSouthWest => match arrival {
                Dir::North => Dir::West,
                Dir::West => Dir::North,
                Dir::East => Dir::South,
                Dir::South => Dir::East,
            },
        };
        return Some(Dart {
            at: end,
            dir: partner,
        });
    }

    // The tight turn: first stub clockwise from the arrival that still has the
    // same face on its left.
    for k in 1..=4u8 {
        let dir = arrival.rotate(k);
        if let Some((left, _)) = labels.stub(end, dir)
            && left == face
        {
            return Some(Dart { at: end, dir });
        }
    }
    None
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::float_cmp, clippy::field_reassign_with_default)]
mod tests {
    use super::*;
    use palette_tracer_core::control::NoControl;

    fn plane(cells: &[u32], width: u32, height: u32) -> LabelPlane {
        let count = cells.iter().max().map_or(1, |m| u64::from(*m) + 1);
        let mut p = LabelPlane::new(width, height, count.max(2)).unwrap();
        for (i, &c) in cells.iter().enumerate() {
            p.set_index(i, LabelId(c));
        }
        p
    }

    fn extract_of(cells: &[u32], width: u32, height: u32) -> Extraction {
        let p = plane(cells, width, height);
        let region_count = cells.iter().max().map_or(0, |m| *m as usize + 1);
        let colors = vec![Oklab::ZERO; cells.len()];
        let palette: Vec<Option<PaletteId>> =
            (0..region_count).map(|i| Some(PaletteId(i as u32))).collect();
        let pinned = vec![false; region_count];
        extract(
            &p,
            &colors,
            &palette,
            &pinned,
            &ExtractOptions::default(),
            &ResourceLimits::default(),
            &WorkBudget::unbounded(),
            &NoControl,
        )
        .unwrap()
    }

    /// The enumeration the module documentation claims, over every 2x2 label
    /// pattern: degree is 0, 2, 3, or 4 -- never 1. Degree 3 appears only where
    /// the exterior label enters, which is the domain border, and that is the
    /// case a rule written for the interior alone would miss.
    #[test]
    fn vertex_degrees_are_zero_two_three_or_four() {
        let mut saw_three = false;
        let mut saw_four = false;
        for pattern in 0..256u32 {
            let cells = [
                (pattern) & 3,
                (pattern >> 2) & 3,
                (pattern >> 4) & 3,
                (pattern >> 6) & 3,
            ];
            let p = plane(&cells, 2, 2);
            let labels = Labels::new(&p);
            for y in 0..=2u32 {
                for x in 0..=2u32 {
                    let degree = labels.degree(GridPoint::new(x, y));
                    assert!(
                        matches!(degree, 0 | 2 | 3 | 4),
                        "pattern {cells:?} gave degree {degree} at ({x}, {y})"
                    );
                    saw_three |= degree == 3;
                    saw_four |= degree == 4;
                }
            }
        }
        assert!(saw_three, "three labels meeting must produce degree three");
        assert!(saw_four, "some pattern must produce a degree-four vertex");
    }

    /// One pixel: one interior face, one exterior face, and one shared edge --
    /// the whole square, because nothing on it is a junction. Splitting the
    /// square at its corners is curve fitting's job (§11.3), not topology's.
    #[test]
    fn a_single_pixel_produces_a_square_with_an_exterior() {
        let e = extract_of(&[0], 1, 1);
        let t = &e.topology;

        assert_eq!(t.faces.len(), 2);
        assert!(t.faces[0].exterior);
        assert_eq!(t.interior_face_count(), 1);
        assert_eq!(t.edges.len(), 1, "one closed interface, not four sides");
        assert_eq!(t.chains[0].segments.len(), 4, "traced through four corners");
        assert_eq!(t.half_edges.len(), 2, "the interface, both ways");

        for (i, he) in t.half_edges.iter().enumerate() {
            assert_eq!(
                t.half_edges[he.twin.index()].twin.index(),
                i,
                "twins must be an involution"
            );
            let this = t.oriented_chain(HalfEdgeId(i as u32));
            let twin = t.oriented_chain(he.twin);
            assert_eq!(this, twin.reversed(), "one chain, two orientations");
        }
    }

    /// PTE-TOPO-004: every face cycle closes, and no half-edge appears twice
    /// in one directed cycle.
    #[test]
    fn every_face_cycle_closes_exactly_once() {
        for (cells, w, h) in [
            (vec![0u32], 1, 1),
            (vec![0, 1, 1, 0], 2, 2),
            (vec![0, 0, 0, 0, 1, 0, 0, 0, 0], 3, 3),
            (vec![0, 1, 2, 3], 2, 2),
        ] {
            let t = extract_of(&cells, w, h).topology;
            let mut visited = vec![0u32; t.half_edges.len()];
            for face in &t.faces {
                for cycle in &face.cycles {
                    let walk = t.walk_cycle(cycle.start);
                    let mut seen = std::collections::BTreeSet::new();
                    for he in &walk {
                        assert!(
                            seen.insert(*he),
                            "half-edge repeats within a cycle for {cells:?}"
                        );
                        assert_eq!(t.half_edges[he.index()].face, face.id);
                        visited[he.index()] += 1;
                    }
                }
            }
            assert!(
                visited.iter().all(|&c| c == 1),
                "every half-edge belongs to exactly one cycle for {cells:?}"
            );
        }
    }

    /// A donut: an interior face with a hole, and the hole's cycle classified
    /// as such (§26.3 counts holes).
    #[test]
    fn a_hole_is_detected_as_a_hole() {
        let cells = vec![
            0, 0, 0, //
            0, 1, 0, //
            0, 0, 0, //
        ];
        let t = extract_of(&cells, 3, 3).topology;

        assert_eq!(t.interior_face_count(), 2, "the ring and the centre");
        assert_eq!(t.hole_count(), 1, "the ring has one hole");

        let ring = t
            .faces
            .iter()
            .find(|f| !f.exterior && f.region == RegionId(0))
            .unwrap();
        assert_eq!(ring.cycles.len(), 2, "an outer boundary and a hole");
        assert_eq!(ring.cycles[0].kind, CycleKind::Outer);
        assert_eq!(ring.cycles[1].kind, CycleKind::Hole);
    }

    /// PTE-TOPO-002: adjacency is integer. Two faces that share an interface
    /// have exactly one shared edge between them, whichever side asks.
    #[test]
    fn an_interface_is_one_shared_edge() {
        let cells = vec![0, 0, 1, 1, 0, 0, 1, 1];
        let t = extract_of(&cells, 4, 2).topology;

        let interior: Vec<&SharedEdge> = t
            .edges
            .iter()
            .filter(|e| !e.left_face.is_exterior() && !e.right_face.is_exterior())
            .collect();
        assert_eq!(interior.len(), 1, "one interface between the two halves");
        assert_eq!(interior[0].face_pair(), (FaceId(1), FaceId(2)));
    }

    /// Four labels meeting at a point is a junction. So is each of the four
    /// places on the border where one region's edge gives way to the next --
    /// those are degree three, with the exterior as the third label.
    #[test]
    fn a_four_label_junction_is_counted() {
        let e = extract_of(&[0, 1, 2, 3], 2, 2);
        assert_eq!(e.ambiguous_cells, 0, "four distinct labels are not ambiguous");
        assert_eq!(
            e.junctions, 5,
            "the centre, plus one on each side of the border"
        );

        let t = &e.topology;
        let degree_four = t.vertices.iter().filter(|v| v.degree == 4).count();
        assert_eq!(degree_four, 1, "only the centre has four incident chains");
        let degree_three = t.vertices.iter().filter(|v| v.degree == 3).count();
        assert_eq!(degree_three, 4, "one T-junction per side");
    }

    /// A checkerboard corner is ambiguous, and the §9.4 decision turns it into
    /// two pass-throughs rather than a junction. The four T-junctions on the
    /// border are unaffected and are still counted.
    #[test]
    fn a_checkerboard_corner_is_resolved_rather_than_made_a_junction() {
        let e = extract_of(&[0, 1, 1, 0], 2, 2);
        assert_eq!(e.ambiguous_cells, 1);

        let t = &e.topology;
        // The centre grid point is (1, 1). If it had become a junction it would
        // be a vertex of degree four; the decision means it is not a vertex at
        // all, because chains pass straight through it.
        let centre = Point::new(1.0, 1.0);
        assert!(
            !t.vertices.iter().any(|v| v.position == centre),
            "the ambiguous cell must be resolved into pass-throughs, not \
             promoted to a junction"
        );
        assert_eq!(e.junctions, 4, "the border T-junctions remain");
    }

    /// PTE-TOPO-006: extraction is linear in pixels plus output. Doubling the
    /// side of a flat image must not more than roughly quadruple the work,
    /// which is the pixel count -- the output stays constant.
    #[test]
    fn work_scales_with_pixels_not_with_something_worse() {
        let charge = |side: u32| {
            let cells = vec![0u32; (side * side) as usize];
            let p = plane(&cells, side, side);
            let budget = WorkBudget::unbounded();
            extract(
                &p,
                &vec![Oklab::ZERO; cells.len()],
                &[Some(PaletteId(0))],
                &[false],
                &ExtractOptions::default(),
                &ResourceLimits::default(),
                &budget,
                &NoControl,
            )
            .unwrap();
            budget.spent()
        };
        let small = charge(8);
        let large = charge(16);
        assert!(
            large <= small * 6,
            "quadrupling the pixels charged {large} against {small}"
        );
    }

    /// PTE-TEST-010: reflection must produce a congruent topology. Counts are
    /// the coarse check; the diagonal fixture is what makes it meaningful.
    #[test]
    fn reflection_preserves_the_topology_counts() {
        let mut cells = vec![0u32; 25];
        for i in 0..5 {
            cells[i * 5 + i] = 1;
        }
        let original = extract_of(&cells, 5, 5);

        let mut mirrored = vec![0u32; 25];
        for y in 0..5usize {
            for x in 0..5usize {
                mirrored[y * 5 + x] = cells[y * 5 + (4 - x)];
            }
        }
        let reflected = extract_of(&mirrored, 5, 5);

        assert_eq!(
            original.topology.edges.len(),
            reflected.topology.edges.len()
        );
        assert_eq!(original.junctions, reflected.junctions);
        assert_eq!(original.ambiguous_cells, reflected.ambiguous_cells);
        assert_eq!(
            original.topology.interior_face_count(),
            reflected.topology.interior_face_count()
        );
    }

    #[test]
    fn the_boundary_sample_limit_is_enforced() {
        let cells: Vec<u32> = (0..64).map(|i| i % 2).collect();
        let p = plane(&cells, 8, 8);
        let mut limits = ResourceLimits::small();
        limits.max_boundary_samples = 4;
        let err = extract(
            &p,
            &vec![Oklab::ZERO; 64],
            &[Some(PaletteId(0)), Some(PaletteId(1))],
            &[false, false],
            &ExtractOptions::default(),
            &limits,
            &WorkBudget::unbounded(),
            &NoControl,
        )
        .unwrap_err();
        assert_eq!(err.code(), "resource.exceeded");
    }
}
