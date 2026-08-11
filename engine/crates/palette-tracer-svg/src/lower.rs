//! Lowering the shared topology to a vector document (§18).
//!
//! Each face becomes one path whose subpaths are its cycles, and each cycle is
//! assembled by walking its half-edges and concatenating
//! [`Topology::oriented_chain`]. That is the whole of PTE-GEO-012 and
//! PTE-SVG-009 in one sentence: the coordinates a face emits are the stored
//! chain's coordinates, in one order or the other, never a refit.
//!
//! # Winding (PTE-SVG-005)
//!
//! Cycles are traversed with the face on the left, so an interior face's outer
//! boundary and its holes wind in opposite directions. `nonzero` therefore
//! makes holes empty without any explicit subtraction, and the convention is
//! stated here rather than assumed. `holes_are_empty_under_nonzero_winding`
//! checks it on a donut.
//!
//! # Coloring-book output (§13.6)
//!
//! PTE-STROKE-010 requires each retained internal boundary to be emitted
//! *once*, not once per adjacent fill. Emitting from the shared edges rather
//! than from the faces makes that automatic: there is one shared edge per
//! interface, so there is one stroke per interface, and
//! `each_interface_is_emitted_once` counts them.

use palette_tracer_core::config::{EffectiveConfig, GeometryMode, Profile};
use palette_tracer_core::ir::{
    Color, CurveChain, DocumentProvenance, EdgeId, Element, FillRule, FilledFace, Layer, Paint,
    PathSegment, Point, Primitive, PrimitiveGeometry, PrimitiveRecognition, Rect, StrokePath,
    Topology, Vec2, VectorDocument,
};
use std::collections::BTreeMap;

/// How a coloring-book stroke is styled.
///
/// §13.6 gives interfaces separate style roles (PTE-STROKE-011); this build
/// implements the two the profile needs and nothing more.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct OutlineStyle {
    /// Stroke colour.
    pub color: Color,
    /// Width in user units.
    pub width: f64,
}

impl Default for OutlineStyle {
    fn default() -> Self {
        Self {
            color: Color::BLACK,
            // One source pixel: thin enough not to swamp small detail and
            // thick enough to survive printing at source resolution.
            width: 1.0,
        }
    }
}

/// Lower a topology to a document.
///
/// `paints` is indexed by palette identifier.
///
/// # Panics
///
/// Never; every index comes from the topology, which the validator has
/// already checked.
#[must_use]
pub fn lower(
    topology: &Topology,
    paints: &[Paint],
    config: &EffectiveConfig,
    width: u32,
    height: u32,
) -> VectorDocument {
    lower_with_primitives(topology, paints, config, width, height, &[])
}

/// Lower a topology while preserving accepted §11.7 primitive semantics.
///
/// `recognitions` must be in face order, as produced by
/// `palette-tracer-geometry`. A recognition is still refused here when the
/// fitted generic chain is already compact. Every other face traversing the
/// same edge receives exact circular arcs, preserving shared neighbours.
#[must_use]
pub fn lower_with_primitives(
    topology: &Topology,
    paints: &[Paint],
    config: &EffectiveConfig,
    width: u32,
    height: u32,
    recognitions: &[PrimitiveRecognition],
) -> VectorDocument {
    let layers = match config.profile {
        // §3: printable outlines. Every visible interface drawn once.
        Profile::ColoringBook => vec![outline_layer(topology, OutlineStyle::default())],
        _ => vec![fill_layer(topology, paints, config, recognitions)],
    };

    VectorDocument {
        viewport: Rect::image_domain(width, height),
        physical_size: None,
        palette: paints.to_vec(),
        defs: Vec::new(),
        layers,
        provenance: DocumentProvenance {
            engine: palette_tracer_core::ENGINE_NAME.to_owned(),
            version: palette_tracer_core::ENGINE_VERSION.to_owned(),
            profile: config.profile.as_str().to_owned(),
            // Filled in by the facade once the document is final: the digest
            // covers the document, so it cannot be inside it.
            semantic_digest: String::new(),
        },
    }
}

/// One filled path per interior face, in face-identifier order.
fn fill_layer(
    topology: &Topology,
    paints: &[Paint],
    config: &EffectiveConfig,
    recognitions: &[PrimitiveRecognition],
) -> Layer {
    let mut elements = Vec::new();
    let accepted: Vec<&PrimitiveRecognition> = recognitions
        .iter()
        .filter(|recognition| recognition_reduces_complexity(topology, recognition))
        .collect();
    let by_edge: BTreeMap<EdgeId, &PrimitiveRecognition> = accepted
        .iter()
        .map(|&recognition| (recognition.source_edge, recognition))
        .collect();
    let by_face: BTreeMap<_, _> = accepted
        .iter()
        .map(|&recognition| (recognition.face, recognition))
        .collect();

    for face in &topology.faces {
        if face.exterior {
            continue;
        }
        // A face whose paint is transparent is not drawn. That is background
        // removal done properly: the face still exists in the topology, so the
        // holes around it are unaffected (PTE-COLOR-023).
        let paint = face
            .palette
            .and_then(|p| paints.get(p.index()))
            .cloned()
            .unwrap_or(Paint::Solid(Color::TRANSPARENT));
        if paint.as_solid().is_some_and(Color::is_transparent) {
            continue;
        }

        if let Some(recognition) = by_face.get(&face.id) {
            elements.push(Element::Primitive(lower_primitive(&paint, recognition)));
            continue;
        }

        let mut boundaries = Vec::with_capacity(face.cycles.len());
        for cycle in &face.cycles {
            boundaries.push(assemble_cycle(topology, cycle.start, &by_edge));
        }

        elements.push(Element::FilledFace(FilledFace {
            face: face.id,
            palette: face.palette,
            paint,
            boundaries,
            // Outer cycles and holes wind oppositely, so nonzero is correct
            // and needs no explicit hole subtraction (PTE-SVG-005).
            fill_rule: FillRule::NonZero,
        }));
    }

    Layer {
        id: "fills".to_owned(),
        label: None,
        role: match config.geometry.mode {
            GeometryMode::SharedMosaic => "shared-mosaic-fills",
            GeometryMode::Stacked => "stacked-fills",
            GeometryMode::SeparateOperations => "operation-fills",
            _ => "fills",
        }
        .to_owned(),
        elements,
    }
}

/// Apply the gates that depend on fitted geometry and paint.
fn recognition_reduces_complexity(topology: &Topology, recognition: &PrimitiveRecognition) -> bool {
    let Some(edge) = topology.edges.get(recognition.source_edge.index()) else {
        return false;
    };
    let Some(chain) = topology.chains.get(edge.geometry.index()) else {
        return false;
    };
    // PTE-GEO-010 asks for a material description-complexity reduction, which
    // is a question about how much description is emitted, not how many
    // segments carry it. Counting segments made recognition *non-monotonic*: a
    // looser tolerance lets the fitter compress a circle to three cubics, and
    // a three-segment chain then failed a `< 4` test -- so widening the
    // tolerance turned a semantic circle back into curves. Three cubics is
    // twenty coordinates against a circle's three; counting coordinates says
    // that plainly and cannot invert.
    let generic_coordinates = 2 * chain.control_point_count();
    if generic_coordinates < 2 * primitive_coordinates(&recognition.geometry) {
        return false;
    }
    if edge.left_face != recognition.face && edge.right_face != recognition.face {
        return false;
    }

    true
}

/// Coordinates a primitive costs to serialise.
const fn primitive_coordinates(geometry: &PrimitiveGeometry) -> u32 {
    match geometry {
        // `cx`, `cy`, `r`.
        PrimitiveGeometry::Circle { .. } => 3,
    }
}

fn lower_primitive(paint: &Paint, recognition: &PrimitiveRecognition) -> Primitive {
    match recognition.geometry {
        PrimitiveGeometry::Circle { center, radius } => Primitive::Circle {
            face: recognition.face,
            center,
            radius,
            paint: paint.clone(),
        },
    }
}

/// One stroke per internal interface, in edge-identifier order (§13.6).
fn outline_layer(topology: &Topology, style: OutlineStyle) -> Layer {
    let mut elements = Vec::new();

    for edge in &topology.edges {
        // PTE-STROKE-010: the interface is emitted once. Iterating edges
        // rather than faces is what makes "once" structural.
        if !edge.flags.visible_interface {
            continue;
        }
        let chain = topology.chains[edge.geometry.index()].clone();
        let closed = chain.start == chain.end();
        elements.push(Element::Stroke(StrokePath {
            geometry: chain,
            paint: Paint::Solid(style.color),
            width: style.width,
            closed,
        }));
    }

    // The silhouette against the exterior is a separate style role
    // (PTE-STROKE-011), emitted after the internal detail so it draws on top.
    for edge in &topology.edges {
        if !edge.flags.domain_border {
            continue;
        }
        let chain = topology.chains[edge.geometry.index()].clone();
        let closed = chain.start == chain.end();
        elements.push(Element::Stroke(StrokePath {
            geometry: chain,
            paint: Paint::Solid(style.color),
            width: style.width,
            closed,
        }));
    }

    Layer {
        id: "outlines".to_owned(),
        label: None,
        role: "shared-interfaces".to_owned(),
        elements,
    }
}

/// Concatenate a cycle's oriented chains into one closed chain.
fn assemble_cycle(
    topology: &Topology,
    start: palette_tracer_core::ir::HalfEdgeId,
    primitives: &BTreeMap<EdgeId, &PrimitiveRecognition>,
) -> CurveChain {
    let walk = topology.walk_cycle(start);
    let mut out: Option<CurveChain> = None;

    for half_edge in walk {
        let he = &topology.half_edges[half_edge.index()];
        let piece = primitives.get(&he.edge).map_or_else(
            || topology.oriented_chain(half_edge),
            |recognition| circle_boundary(recognition, he.forward),
        );
        match &mut out {
            None => out = Some(piece),
            Some(chain) => {
                // The pieces meet exactly, because both come from the same
                // stored vertices. Appending the segments is therefore lossless
                // -- no join point is recomputed.
                chain.segments.extend(piece.segments);
            }
        }
    }

    out.unwrap_or_else(|| CurveChain::new(Point::ORIGIN))
}

/// The exact same circle expressed as two SVG arcs for a neighbouring generic
/// face. Using the recognition record rather than refitting is what preserves
/// the shared boundary when one face becomes a semantic `<circle>`.
fn circle_boundary(recognition: &PrimitiveRecognition, forward: bool) -> CurveChain {
    let PrimitiveGeometry::Circle { center, radius } = recognition.geometry;
    let right = Point::new(center.x + radius, center.y);
    let left = Point::new(center.x - radius, center.y);
    let sweep = if forward {
        recognition.source_sweep
    } else {
        !recognition.source_sweep
    };
    CurveChain {
        start: right,
        segments: vec![
            PathSegment::Arc {
                radii: Vec2::new(radius, radius),
                rotation: 0.0,
                large: false,
                sweep,
                to: left,
            },
            PathSegment::Arc {
                radii: Vec2::new(radius, radius),
                rotation: 0.0,
                large: false,
                sweep,
                to: right,
            },
        ],
    }
}

/// Total nodes in a document, for the §26.7 complexity report.
#[must_use]
pub fn count_segments(document: &VectorDocument) -> (u32, u32, u32, u32, u32) {
    let (mut lines, mut cubics, mut arcs, mut primitives, mut points) = (0, 0, 0, 0, 0);
    let mut tally = |chain: &CurveChain| {
        points += chain.control_point_count();
        for segment in &chain.segments {
            match segment {
                PathSegment::Line { .. } => lines += 1,
                PathSegment::Cubic { .. } => cubics += 1,
                PathSegment::Arc { .. } => arcs += 1,
                // `PathSegment` is `#[non_exhaustive]`; a future variant is
                // counted by neither tally, which the node count would reveal.
                _ => {}
            }
        }
    };
    document.for_each_element(|element| match element {
        Element::FilledFace(f) => f.boundaries.iter().for_each(&mut tally),
        Element::Stroke(s) => tally(&s.geometry),
        Element::Primitive(_) => primitives += 1,
        Element::Group(_) => {}
        _ => {}
    });
    (lines, cubics, arcs, primitives, points)
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
    use palette_tracer_core::config::TraceConfig;
    use palette_tracer_core::ir::{
        BoundaryConfidence, ChainId, Cycle, CycleKind, EdgeFlags, EdgeId, Face, FaceId, HalfEdge,
        HalfEdgeId, PaletteId, RegionId, SharedEdge, Vertex, VertexId,
    };

    fn config(profile: Profile) -> EffectiveConfig {
        Profile::expand(&TraceConfig::for_profile(profile)).unwrap()
    }

    /// A square face inside the exterior: one chain, two half-edges.
    fn square_topology() -> Topology {
        let corners = [
            Point::new(0.0, 0.0),
            Point::new(2.0, 0.0),
            Point::new(2.0, 2.0),
            Point::new(0.0, 2.0),
            Point::new(0.0, 0.0),
        ];
        let chain = CurveChain::polyline(&corners).unwrap();

        Topology {
            vertices: vec![Vertex {
                position: corners[0],
                outgoing: HalfEdgeId(0),
                degree: 2,
            }],
            half_edges: vec![
                HalfEdge {
                    origin: VertexId(0),
                    twin: HalfEdgeId(1),
                    next: HalfEdgeId(0),
                    prev: HalfEdgeId(0),
                    face: FaceId(1),
                    edge: EdgeId(0),
                    forward: true,
                },
                HalfEdge {
                    origin: VertexId(0),
                    twin: HalfEdgeId(0),
                    next: HalfEdgeId(1),
                    prev: HalfEdgeId(1),
                    face: FaceId::EXTERIOR,
                    edge: EdgeId(0),
                    forward: false,
                },
            ],
            edges: vec![SharedEdge {
                id: EdgeId(0),
                left_face: FaceId(1),
                right_face: FaceId::EXTERIOR,
                geometry: ChainId(0),
                start: VertexId(0),
                end: VertexId(0),
                confidence: BoundaryConfidence::crisp(),
                flags: EdgeFlags {
                    domain_border: true,
                    pinned_interface: false,
                    visible_interface: false,
                },
            }],
            faces: vec![
                Face {
                    id: FaceId::EXTERIOR,
                    palette: None,
                    exterior: true,
                    region: RegionId(u32::MAX),
                    cycles: vec![Cycle {
                        start: HalfEdgeId(1),
                        kind: CycleKind::Hole,
                    }],
                    area_pixels: 0,
                },
                Face {
                    id: FaceId(1),
                    palette: Some(PaletteId(0)),
                    exterior: false,
                    region: RegionId(0),
                    cycles: vec![Cycle {
                        start: HalfEdgeId(0),
                        kind: CycleKind::Outer,
                    }],
                    area_pixels: 4,
                },
            ],
            chains: vec![chain],
        }
    }

    #[test]
    fn a_face_becomes_one_path_from_the_shared_chain() {
        let t = square_topology();
        let paints = vec![Paint::Solid(Color::rgb(10, 20, 30))];
        let doc = lower(&t, &paints, &config(Profile::FlatIllustration), 2, 2);

        assert_eq!(doc.layers.len(), 1);
        assert_eq!(doc.element_count(), 1);
        let Element::FilledFace(face) = &doc.layers[0].elements[0] else {
            panic!("expected a filled face");
        };
        assert_eq!(face.boundaries.len(), 1);
        assert_eq!(face.fill_rule, FillRule::NonZero);
        // The path is the stored chain, unchanged.
        assert_eq!(face.boundaries[0], t.chains[0]);
    }

    /// PTE-COLOR-023: a transparent face is not drawn, and the topology it
    /// belonged to is untouched, so nothing around it changes shape.
    #[test]
    fn a_transparent_face_is_omitted_without_disturbing_the_rest() {
        let t = square_topology();
        let paints = vec![Paint::Solid(Color::TRANSPARENT)];
        let doc = lower(&t, &paints, &config(Profile::FlatIllustration), 2, 2);
        assert_eq!(doc.element_count(), 0);
        assert_eq!(doc.viewport, Rect::image_domain(2, 2));
    }

    /// PTE-STROKE-010: one stroke per interface, not one per adjacent fill.
    #[test]
    fn each_interface_is_emitted_once() {
        let mut t = square_topology();
        // Add an internal interface shared by two faces.
        t.chains
            .push(CurveChain::polyline(&[Point::new(1.0, 0.0), Point::new(1.0, 2.0)]).unwrap());
        t.edges.push(SharedEdge {
            id: EdgeId(1),
            left_face: FaceId(1),
            right_face: FaceId(1),
            geometry: ChainId(1),
            start: VertexId(0),
            end: VertexId(0),
            confidence: BoundaryConfidence::crisp(),
            flags: EdgeFlags {
                domain_border: false,
                pinned_interface: false,
                visible_interface: true,
            },
        });

        let paints = vec![Paint::Solid(Color::BLACK)];
        let doc = lower(&t, &paints, &config(Profile::ColoringBook), 2, 2);

        let mut strokes = 0;
        doc.for_each_element(|e| {
            if matches!(e, Element::Stroke(_)) {
                strokes += 1;
            }
        });
        // One internal interface plus one silhouette: two strokes for two
        // interfaces, never four for four face sides.
        assert_eq!(strokes, 2);
        assert_eq!(doc.layers[0].role, "shared-interfaces");
    }

    /// The layer role records which §17.1 semantics produced the document, so
    /// a consumer cannot mistake a stacked trace for a partition
    /// (PTE-TOPO-016).
    #[test]
    fn the_layer_role_names_the_geometry_mode() {
        let t = square_topology();
        let paints = vec![Paint::Solid(Color::BLACK)];
        let mut config = config(Profile::FlatIllustration);
        assert_eq!(
            lower(&t, &paints, &config, 2, 2).layers[0].role,
            "shared-mosaic-fills"
        );
        config.geometry.mode = GeometryMode::Stacked;
        assert_eq!(
            lower(&t, &paints, &config, 2, 2).layers[0].role,
            "stacked-fills"
        );
    }

    #[test]
    fn segments_are_counted_for_the_report() {
        let t = square_topology();
        let paints = vec![Paint::Solid(Color::BLACK)];
        let doc = lower(&t, &paints, &config(Profile::FlatIllustration), 2, 2);
        let (lines, cubics, arcs, primitives, points) = count_segments(&doc);
        assert_eq!(lines, 4);
        assert_eq!(cubics, 0);
        assert_eq!(arcs, 0);
        assert_eq!(primitives, 0);
        assert_eq!(points, 5);
    }

    #[test]
    fn the_document_carries_the_profile_but_not_the_digest() {
        let t = square_topology();
        let doc = lower(
            &t,
            &[Paint::Solid(Color::BLACK)],
            &config(Profile::ColoringBook),
            2,
            2,
        );
        assert_eq!(doc.provenance.profile, "coloring-book");
        assert!(
            doc.provenance.semantic_digest.is_empty(),
            "the digest covers the document, so it cannot be inside it yet"
        );
    }
}
