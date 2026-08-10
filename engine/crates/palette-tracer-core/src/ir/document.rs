//! The output vector document (§6.4).
//!
//! PTE-ARCH-006: "The core MUST return structured `VectorDocument` data. It
//! MUST NOT use unvalidated SVG path strings as its internal geometry
//! protocol." Nothing in this module is a string that a renderer would parse.

use super::geom::{CurveChain, Rect};
use super::paint::Paint;
use super::topology::{FaceId, PaletteId};
use serde::{Deserialize, Serialize};

/// Physical output size, when the caller declared one (PTE-FAB-001).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct PhysicalSize {
    /// Width in millimetres.
    pub width_mm: f64,
    /// Height in millimetres.
    pub height_mm: f64,
}

/// A referenced definition, emitted into `<defs>`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Definition {
    /// Deterministic identifier, derived from traversal order (§18.4).
    pub id: String,
    /// The paint this definition provides.
    pub paint: Paint,
}

/// How a face's outline is filled (§18.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum FillRule {
    /// Non-zero winding.
    NonZero,
    /// Even-odd.
    EvenOdd,
}

/// A filled face: one outer boundary and its holes.
///
/// The `boundaries` are assembled from oriented references to shared chains,
/// so a coordinate that appears in two faces came from one stored number
/// (PTE-GEO-012, PTE-SVG-009).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FilledFace {
    /// Topological face this came from, for provenance and for the report.
    pub face: FaceId,
    /// Palette entry that paints it.
    pub palette: Option<PaletteId>,
    /// Paint, resolved.
    pub paint: Paint,
    /// Outer boundary first, then holes. Each is a closed chain.
    pub boundaries: Vec<CurveChain>,
    /// Fill rule.
    pub fill_rule: FillRule,
}

/// A stroked path.
///
/// Present in the IR because §6.4 defines it and because the `coloring-book`
/// profile emits interfaces as strokes. Centreline *inference* (§13) is not
/// implemented; these strokes are shared interfaces drawn once, not medial
/// reconstructions (PTE-STROKE-010).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StrokePath {
    /// The path.
    pub geometry: CurveChain,
    /// Stroke paint.
    pub paint: Paint,
    /// Width in user units.
    pub width: f64,
    /// Whether the path closes.
    pub closed: bool,
}

/// A recognised primitive (§11.7).
///
/// Defined so that recognition, when implemented, can keep semantics until
/// lowering (PTE-GEO-011). Nothing produces these yet.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
#[non_exhaustive]
pub enum Primitive {
    /// An axis-aligned rectangle.
    Rect {
        /// Its bounds.
        bounds: Rect,
        /// Corner radius, if rounded.
        corner_radius: Option<f64>,
        /// Paint.
        paint: Paint,
    },
    /// A circle or ellipse.
    Ellipse {
        /// Centre.
        center: super::geom::Point,
        /// Semi-axes.
        radii: super::geom::Vec2,
        /// Paint.
        paint: Paint,
    },
}

/// A document element (§6.4).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
#[non_exhaustive]
pub enum Element {
    /// A filled face.
    FilledFace(FilledFace),
    /// A stroked path.
    Stroke(StrokePath),
    /// A recognised primitive.
    Primitive(Primitive),
    /// A nested group.
    Group(Group),
}

/// A group of elements, with a semantic name.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Group {
    /// Deterministic identifier.
    pub id: String,
    /// Human-meaningful label, escaped at serialisation (PTE-SEC-001).
    pub label: Option<String>,
    /// Children in canonical order.
    pub children: Vec<Element>,
}

/// A semantic layer.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Layer {
    /// Deterministic identifier.
    pub id: String,
    /// Human-meaningful label.
    pub label: Option<String>,
    /// Role, e.g. `fills` or `outlines`, used by profiles that separate style
    /// roles (PTE-STROKE-011).
    pub role: String,
    /// Elements in canonical order.
    pub elements: Vec<Element>,
}

/// How this document was produced.
///
/// §18.6: metadata "MUST NOT contain the input raster, absolute local paths,
/// usernames, timestamps, or host details unless explicitly requested". There
/// is no field here for any of those, so the serialiser cannot leak one.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DocumentProvenance {
    /// Engine name.
    pub engine: String,
    /// Engine version.
    pub version: String,
    /// Profile identifier.
    pub profile: String,
    /// Semantic digest, `pte-semantic-v1-blake3:<hex>` (Appendix F.4).
    pub semantic_digest: String,
}

/// The complete output document (§6.4).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct VectorDocument {
    /// Viewport in user units, initially the image domain (PTE-API-002).
    pub viewport: Rect,
    /// Physical size, when declared.
    pub physical_size: Option<PhysicalSize>,
    /// Palette, in stable identifier order.
    pub palette: Vec<Paint>,
    /// Referenced definitions, in canonical dependency order (§18.4).
    pub defs: Vec<Definition>,
    /// Layers in declared semantic order.
    pub layers: Vec<Layer>,
    /// Provenance.
    pub provenance: DocumentProvenance,
}

impl VectorDocument {
    /// Total elements across every layer, counting group children.
    #[must_use]
    pub fn element_count(&self) -> u64 {
        fn count(elements: &[Element]) -> u64 {
            elements
                .iter()
                .map(|e| match e {
                    Element::Group(g) => 1 + count(&g.children),
                    _ => 1,
                })
                .sum()
        }
        self.layers.iter().map(|l| count(&l.elements)).sum()
    }

    /// Visit every element in canonical order.
    pub fn for_each_element(&self, mut f: impl FnMut(&Element)) {
        fn walk(elements: &[Element], f: &mut impl FnMut(&Element)) {
            for e in elements {
                f(e);
                if let Element::Group(g) = e {
                    walk(&g.children, f);
                }
            }
        }
        for layer in &self.layers {
            walk(&layer.elements, &mut f);
        }
    }

    /// Whether every coordinate in the document is finite (PTE-SVG-003).
    ///
    /// The last gate before serialisation. A `false` here is a
    /// [`crate::ValidationError`], never a silently clamped number.
    #[must_use]
    pub fn all_coordinates_finite(&self) -> bool {
        if !self.viewport.is_valid_viewport() {
            return false;
        }
        let mut ok = true;
        self.for_each_element(|e| {
            let finite = match e {
                Element::FilledFace(f) => f.boundaries.iter().all(CurveChain::is_finite),
                Element::Stroke(s) => s.geometry.is_finite() && s.width.is_finite(),
                Element::Primitive(_) | Element::Group(_) => true,
            };
            ok &= finite;
        });
        ok
    }
}

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::float_cmp,
    clippy::field_reassign_with_default
)]
mod tests {
    use super::super::geom::Point;
    use super::super::paint::Color;
    use super::*;

    fn face(x: f64) -> Element {
        Element::FilledFace(FilledFace {
            face: FaceId(1),
            palette: Some(PaletteId(0)),
            paint: Paint::Solid(Color::BLACK),
            boundaries: vec![
                CurveChain::polyline(&[
                    Point::new(x, 0.0),
                    Point::new(x + 1.0, 0.0),
                    Point::new(x, 1.0),
                    Point::new(x, 0.0),
                ])
                .unwrap(),
            ],
            fill_rule: FillRule::NonZero,
        })
    }

    fn document(elements: Vec<Element>) -> VectorDocument {
        VectorDocument {
            viewport: Rect::image_domain(4, 4),
            physical_size: None,
            palette: vec![Paint::Solid(Color::BLACK)],
            defs: Vec::new(),
            layers: vec![Layer {
                id: "layer-0".into(),
                label: None,
                role: "fills".into(),
                elements,
            }],
            provenance: DocumentProvenance {
                engine: crate::ENGINE_NAME.into(),
                version: crate::ENGINE_VERSION.into(),
                profile: "flat-illustration".into(),
                semantic_digest: String::new(),
            },
        }
    }

    #[test]
    fn elements_are_counted_through_groups() {
        let doc = document(vec![
            face(0.0),
            Element::Group(Group {
                id: "g".into(),
                label: None,
                children: vec![face(1.0), face(2.0)],
            }),
        ]);
        assert_eq!(doc.element_count(), 4);
    }

    #[test]
    fn traversal_visits_group_children() {
        let doc = document(vec![Element::Group(Group {
            id: "g".into(),
            label: None,
            children: vec![face(1.0)],
        })]);
        let mut seen = 0;
        doc.for_each_element(|_| seen += 1);
        assert_eq!(seen, 2);
    }

    /// PTE-SVG-003 in document form. A NaN must be detectable before the
    /// serialiser ever sees the document.
    #[test]
    fn a_nan_coordinate_fails_the_finiteness_gate() {
        let mut doc = document(vec![face(0.0)]);
        assert!(doc.all_coordinates_finite());

        if let Element::FilledFace(f) = &mut doc.layers[0].elements[0] {
            f.boundaries[0].start.x = f64::NAN;
        }
        assert!(!doc.all_coordinates_finite());
    }

    #[test]
    fn a_degenerate_viewport_fails_the_gate() {
        let mut doc = document(vec![face(0.0)]);
        doc.viewport = Rect::new(0.0, 0.0, 0.0, 4.0);
        assert!(!doc.all_coordinates_finite());
    }
}
