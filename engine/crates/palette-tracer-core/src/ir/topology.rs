//! The shared-topology intermediate representation (§9, §6.4).
//!
//! This is the structure the whole engine exists to have. §9.1 states the
//! invariant:
//!
//! ```text
//! ∂A ∩ ∂B = e(A,B),   e(B,A) = reverse(e(A,B))
//! ```
//!
//! An interface between two faces is *one* [`SharedEdge`] with *one*
//! [`CurveChain`]. The two incident faces reference it through half-edges;
//! one traverses the chain forwards and the other traverses
//! [`CurveChain::reversed`]. Neither owns a copy, so "independent control
//! points for the two sides" is not representable rather than merely
//! discouraged (PTE-TOPO-001).
//!
//! The data structure lives here, in the contracts crate, because both
//! `palette-tracer-topology` (which builds it) and `palette-tracer-geometry`
//! (which fits its chains) need it, and stage crates do not depend on each
//! other (ADR-0002).
//!
//! # Provenance
//!
//! A standard doubly-connected edge list, specialised to planar image
//! partitions. Written from §9.2 and Appendix D.3; no implementation was
//! consulted.

use super::geom::{CurveChain, Point};
use crate::determinism::canonical_pair;
use serde::{Deserialize, Serialize};

macro_rules! id_type {
    ($name:ident, $doc:literal) => {
        #[doc = $doc]
        ///
        /// Assigned by a deterministic order derived from raster or
        /// topological position, never allocation order (PTE-API-010).
        #[derive(
            Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize,
        )]
        #[serde(transparent)]
        pub struct $name(pub u32);

        impl $name {
            /// The raw index.
            #[must_use]
            pub const fn index(self) -> usize {
                self.0 as usize
            }
        }
    };
}

id_type!(VertexId, "A junction, corner pin, or image-domain corner.");
id_type!(HalfEdgeId, "One oriented side of a shared edge.");
id_type!(EdgeId, "An undirected interface between two faces.");
id_type!(FaceId, "A filled region, or the exterior.");
id_type!(ChainId, "One shared curve chain.");
id_type!(RegionId, "A region of the raster partition.");
id_type!(PaletteId, "A palette entry.");

impl FaceId {
    /// The exterior face.
    ///
    /// PTE-TOPO-007: "Exterior/background is an explicit face, not an absence
    /// of topology." It is always index 0, so a domain-border half-edge always
    /// has a twin to point at (PTE-TOPO-003).
    pub const EXTERIOR: Self = Self(0);

    /// Whether this is the exterior face.
    #[must_use]
    pub const fn is_exterior(self) -> bool {
        self.0 == Self::EXTERIOR.0
    }
}

/// Where a boundary's position came from (PTE-AA-009).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
#[non_exhaustive]
pub enum BoundaryEvidence {
    /// Antialias coverage was inverted to a subpixel offset (§10.3).
    CoverageReconstructed,
    /// The boundary sits on the pixel-cell interface (§10.6).
    CrispGrid,
    /// Coverage was attempted and rejected; the geometric estimate stands.
    LowConfidenceFallback,
    /// Pixel-art connectivity policy decided it (PTE-TOPO-010).
    PixelArtPolicy,
}

impl BoundaryEvidence {
    /// Stable identifier for the report.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::CoverageReconstructed => "coverage_reconstructed",
            Self::CrispGrid => "crisp_grid",
            Self::LowConfidenceFallback => "low_confidence_fallback",
            Self::PixelArtPolicy => "pixel_art_policy",
        }
    }
}

/// How much a boundary's position is trusted (PTE-AA-005).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct BoundaryConfidence {
    /// Confidence in `[0, 1]`, combining mixture residual, colour separation,
    /// normal stability, and neighbourhood agreement.
    pub value: f64,
    /// What produced the position.
    pub evidence: BoundaryEvidence,
}

impl BoundaryConfidence {
    /// A crisp grid boundary, which is certain about the *interval* it lies in
    /// even though it carries no subpixel evidence (§10.6).
    #[must_use]
    pub const fn crisp() -> Self {
        Self {
            value: 1.0,
            evidence: BoundaryEvidence::CrispGrid,
        }
    }
}

/// Properties of a shared edge that the fitter and serialiser must respect.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct EdgeFlags {
    /// The edge lies on the image domain border and twins the exterior face.
    pub domain_border: bool,
    /// The edge separates two distinct pinned palette identities and must not
    /// be simplified across (PTE-SEG-012).
    pub pinned_interface: bool,
    /// A profile marked this interface for once-only emission, as
    /// `coloring-book` does (PTE-STROKE-010).
    pub visible_interface: bool,
}

/// An undirected interface between exactly two faces (§6.4).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SharedEdge {
    /// Stable identifier.
    pub id: EdgeId,
    /// Face on the left of the forward traversal.
    pub left_face: FaceId,
    /// Face on the right of the forward traversal.
    pub right_face: FaceId,
    /// The one curve chain, traversed forwards from `start` to `end`.
    pub geometry: ChainId,
    /// Where the chain starts.
    pub start: VertexId,
    /// Where the chain ends.
    pub end: VertexId,
    /// Position confidence.
    pub confidence: BoundaryConfidence,
    /// Fitting and emission flags.
    pub flags: EdgeFlags,
}

impl SharedEdge {
    /// The two faces in canonical order, so `(A, B)` and `(B, A)` name the
    /// same interface.
    #[must_use]
    pub fn face_pair(&self) -> (FaceId, FaceId) {
        canonical_pair(self.left_face, self.right_face)
    }
}

/// One oriented side of a shared edge (§9.2).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct HalfEdge {
    /// Where this traversal begins.
    pub origin: VertexId,
    /// The opposite traversal. Always exists (PTE-TOPO-003).
    pub twin: HalfEdgeId,
    /// Next half-edge around the incident face.
    pub next: HalfEdgeId,
    /// Previous half-edge around the incident face.
    pub prev: HalfEdgeId,
    /// The face this traversal borders.
    pub face: FaceId,
    /// The undirected edge.
    pub edge: EdgeId,
    /// Whether this traversal follows the chain as stored, or reversed.
    pub forward: bool,
}

/// Whether a face cycle is an outer boundary or a hole.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum CycleKind {
    /// The outer boundary of the face.
    Outer,
    /// A hole inside the face.
    Hole,
}

/// One closed traversal around a face.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Cycle {
    /// Any half-edge on the cycle; traversal starts here.
    pub start: HalfEdgeId,
    /// Outer boundary or hole.
    pub kind: CycleKind,
}

/// A face: one filled region, or the exterior (§9.2).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Face {
    /// Stable identifier.
    pub id: FaceId,
    /// Palette entry that paints this face, or `None` for the exterior.
    pub palette: Option<PaletteId>,
    /// Whether this is the exterior face.
    pub exterior: bool,
    /// The raster region this face came from.
    pub region: RegionId,
    /// Outer boundary first, then holes.
    pub cycles: Vec<Cycle>,
    /// Raster area in pixels, carried for the report and for area gates.
    pub area_pixels: u64,
}

/// A junction, corner pin, or image-domain corner (§9.5).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Vertex {
    /// Position, shared by every incident curve. PTE-TOPO-011 requires this to
    /// be optimised once, not once per incident edge.
    pub position: Point,
    /// One outgoing half-edge, for starting a traversal.
    pub outgoing: HalfEdgeId,
    /// Degree: how many half-edges leave this vertex.
    pub degree: u32,
}

impl Vertex {
    /// Whether this vertex is a junction rather than a point along a chain.
    ///
    /// §9.5: "Junctions have degree other than two."
    #[must_use]
    pub const fn is_junction(&self) -> bool {
        self.degree != 2
    }
}

/// The shared topology of one raster partition.
///
/// Arena-allocated: every reference is an index, so the whole structure is one
/// contiguous set of vectors with no pointer chasing and no interior
/// mutability (§35.4).
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct Topology {
    /// Vertices, indexed by [`VertexId`].
    pub vertices: Vec<Vertex>,
    /// Half-edges, indexed by [`HalfEdgeId`].
    pub half_edges: Vec<HalfEdge>,
    /// Shared edges, indexed by [`EdgeId`].
    pub edges: Vec<SharedEdge>,
    /// Faces, indexed by [`FaceId`]. Index 0 is always the exterior.
    pub faces: Vec<Face>,
    /// Curve chains, indexed by [`ChainId`]. One per shared edge.
    pub chains: Vec<CurveChain>,
}

impl Topology {
    /// The chain for `half_edge`, oriented for that traversal.
    ///
    /// This is the only way a face gets its geometry, and it is why the two
    /// sides of an interface can never drift apart: the reverse traversal is
    /// computed from the same stored numbers every time (PTE-TOPO-001,
    /// PTE-AA-008).
    ///
    /// # Panics
    ///
    /// If `half_edge` or its chain is out of range, which would be an engine
    /// defect -- the validator checks both.
    #[must_use]
    pub fn oriented_chain(&self, half_edge: HalfEdgeId) -> CurveChain {
        let he = &self.half_edges[half_edge.index()];
        let chain = &self.chains[self.edges[he.edge.index()].geometry.index()];
        if he.forward {
            chain.clone()
        } else {
            chain.reversed()
        }
    }

    /// Walk a cycle, yielding its half-edges in order.
    ///
    /// Stops after `half_edges.len()` steps so a corrupt `next` chain produces
    /// a finite result the validator can report, rather than a hang
    /// (PTE-SEC-008).
    #[must_use]
    pub fn walk_cycle(&self, start: HalfEdgeId) -> Vec<HalfEdgeId> {
        let mut out = Vec::new();
        let mut current = start;
        for _ in 0..=self.half_edges.len() {
            out.push(current);
            let next = self.half_edges[current.index()].next;
            if next == start {
                return out;
            }
            current = next;
        }
        out
    }

    /// Number of faces excluding the exterior.
    #[must_use]
    pub fn interior_face_count(&self) -> usize {
        self.faces.iter().filter(|f| !f.exterior).count()
    }

    /// Number of junction vertices (§26.3).
    #[must_use]
    pub fn junction_count(&self) -> usize {
        self.vertices.iter().filter(|v| v.is_junction()).count()
    }

    /// Total holes across the interior faces (§26.3).
    ///
    /// The exterior face's cycles are excluded. Every one of them is a hole by
    /// the signed-area test -- the image sits *inside* the exterior -- and
    /// counting them would report a hole for every component of the artwork.
    #[must_use]
    pub fn hole_count(&self) -> usize {
        self.faces
            .iter()
            .filter(|f| !f.exterior)
            .flat_map(|f| &f.cycles)
            .filter(|c| c.kind == CycleKind::Hole)
            .count()
    }

    /// Total control points across all chains (§26.7).
    #[must_use]
    pub fn control_point_count(&self) -> u32 {
        self.chains.iter().map(CurveChain::control_point_count).sum()
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::float_cmp, clippy::field_reassign_with_default)]
mod tests {
    use super::*;
    use crate::ir::geom::Point;

    /// A two-face topology: one square face inside the exterior, four edges.
    /// Small enough to write by hand and check every field.
    fn square_topology() -> Topology {
        let corners = [
            Point::new(1.0, 1.0),
            Point::new(3.0, 1.0),
            Point::new(3.0, 3.0),
            Point::new(1.0, 3.0),
        ];
        let mut t = Topology::default();

        for (i, &p) in corners.iter().enumerate() {
            t.vertices.push(Vertex {
                position: p,
                outgoing: HalfEdgeId(2 * i as u32),
                degree: 2,
            });
        }

        for i in 0..4u32 {
            let next_corner = (i + 1) % 4;
            t.chains.push(
                CurveChain::polyline(&[
                    corners[i as usize],
                    corners[next_corner as usize],
                ])
                .unwrap(),
            );
            t.edges.push(SharedEdge {
                id: EdgeId(i),
                left_face: FaceId(1),
                right_face: FaceId::EXTERIOR,
                geometry: ChainId(i),
                start: VertexId(i),
                end: VertexId(next_corner),
                confidence: BoundaryConfidence::crisp(),
                flags: EdgeFlags::default(),
            });
            // Even half-edges bound the square, odd ones bound the exterior.
            t.half_edges.push(HalfEdge {
                origin: VertexId(i),
                twin: HalfEdgeId(2 * i + 1),
                next: HalfEdgeId(2 * next_corner),
                prev: HalfEdgeId(2 * ((i + 3) % 4)),
                face: FaceId(1),
                edge: EdgeId(i),
                forward: true,
            });
            t.half_edges.push(HalfEdge {
                origin: VertexId(next_corner),
                twin: HalfEdgeId(2 * i),
                next: HalfEdgeId(2 * ((i + 3) % 4) + 1),
                prev: HalfEdgeId(2 * next_corner + 1),
                face: FaceId::EXTERIOR,
                edge: EdgeId(i),
                forward: false,
            });
        }

        t.faces.push(Face {
            id: FaceId::EXTERIOR,
            palette: None,
            exterior: true,
            region: RegionId(0),
            cycles: vec![Cycle {
                start: HalfEdgeId(1),
                kind: CycleKind::Outer,
            }],
            area_pixels: 0,
        });
        t.faces.push(Face {
            id: FaceId(1),
            palette: Some(PaletteId(0)),
            exterior: false,
            region: RegionId(1),
            cycles: vec![Cycle {
                start: HalfEdgeId(0),
                kind: CycleKind::Outer,
            }],
            area_pixels: 4,
        });
        t
    }

    /// The §9.1 invariant, checked on real data: the two sides of every
    /// interface are the same numbers, one of them reversed.
    #[test]
    fn twin_traversals_are_exact_reverses_of_one_chain() {
        let t = square_topology();
        for (i, he) in t.half_edges.iter().enumerate() {
            let this = t.oriented_chain(HalfEdgeId(i as u32));
            let twin = t.oriented_chain(he.twin);
            assert_eq!(
                this,
                twin.reversed(),
                "half-edge {i} and its twin must describe one chain"
            );
        }
    }

    #[test]
    fn twins_are_an_involution() {
        let t = square_topology();
        for (i, he) in t.half_edges.iter().enumerate() {
            assert_eq!(t.half_edges[he.twin.index()].twin.index(), i);
        }
    }

    #[test]
    fn face_cycles_close() {
        let t = square_topology();
        for face in &t.faces {
            for cycle in &face.cycles {
                let walk = t.walk_cycle(cycle.start);
                assert_eq!(walk.len(), 4, "the square has four sides per cycle");
                let mut seen = std::collections::BTreeSet::new();
                for he in &walk {
                    assert!(seen.insert(*he), "a half-edge appears twice in one cycle");
                    assert_eq!(t.half_edges[he.index()].face, face.id);
                }
            }
        }
    }

    /// A corrupt `next` chain must terminate, not hang (PTE-SEC-008).
    #[test]
    fn walking_a_broken_cycle_terminates() {
        let mut t = square_topology();
        t.half_edges[0].next = HalfEdgeId(0);
        let walk = t.walk_cycle(HalfEdgeId(2));
        assert!(walk.len() <= t.half_edges.len() + 1);
    }

    #[test]
    fn the_exterior_face_is_explicit_and_first() {
        let t = square_topology();
        assert!(t.faces[0].exterior);
        assert!(FaceId::EXTERIOR.is_exterior());
        assert_eq!(t.interior_face_count(), 1);
    }

    #[test]
    fn a_face_pair_canonicalises_regardless_of_side() {
        let t = square_topology();
        let e = &t.edges[0];
        assert_eq!(e.face_pair(), (FaceId::EXTERIOR, FaceId(1)));
    }

    #[test]
    fn degree_two_vertices_are_not_junctions() {
        let t = square_topology();
        assert_eq!(t.junction_count(), 0);
        assert_eq!(t.hole_count(), 0);
    }
}
