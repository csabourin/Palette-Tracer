//! Deterministic SVG serialisation (§18).
//!
//! # Typed segments only (PTE-SVG-004)
//!
//! Path data is built from [`PathSegment`] values. There is no function here
//! that accepts a path string, so "concatenating backend-provided path
//! strings" is not something this serialiser can be asked to do.
//!
//! # Byte-identical shared coordinates (PTE-SVG-009)
//!
//! Two faces sharing an interface get their coordinates from one stored chain,
//! one of them reversed. Reversal copies coordinates rather than recomputing
//! them, and [`format_coordinate`] is a pure function of the value and the
//! precision, so the two faces' serialised coordinates are byte-identical.
//! `shared_coordinates_are_byte_identical` asserts it on the emitted text
//! rather than on the geometry, because the text is what a renderer sees.
//!
//! # Precision (§18.3, PTE-SVG-007/008)
//!
//! "A fixed 'three decimals for everything' rule is forbidden." The precision
//! is searched: the fewest decimals whose worst-case coordinate displacement
//! stays inside the profile's boundary budget and which collapses no two
//! distinct points onto each other. Rounding is the last geometry-changing
//! step (PTE-NO-020).
//!
//! # Escaping (PTE-SEC-001)
//!
//! Every string that reaches the output goes through [`escape_xml`]. Labels
//! come from callers and callers can be hostile.

use palette_tracer_core::config::{BoundarySource, EffectiveConfig, Precision, Profile};
use palette_tracer_core::error::ValidationError;
use palette_tracer_core::ir::{
    CurveChain, Element, FillRule, Layer, Paint, PathSegment, Point, Primitive, Vec2,
    VectorDocument,
};

/// Largest precision the search will consider.
///
/// Eight decimals on a coordinate that is at most a few tens of thousands of
/// pixels is far below `f64` precision, so nothing above this can improve
/// anything.
const MAX_DECIMALS: u8 = 8;

/// Format one coordinate at `decimals`, canonically.
///
/// Trailing zeros and a trailing point are removed, and `-0` becomes `0`, so
/// two values that round to the same number always produce the same bytes --
/// which is what PTE-SVG-009 needs.
#[must_use]
// Exact comparison against zero is the question being asked: "does this value
// format as zero", so that `-0` and `0` produce the same bytes. A tolerance
// would be the wrong tool.
#[allow(clippy::float_cmp)]
pub fn format_coordinate(value: f64, decimals: u8) -> String {
    let rounded = round_to(value, decimals);
    if rounded == 0.0 {
        // Catches both zeros, and avoids emitting "-0".
        return "0".to_owned();
    }
    let mut text = format!("{rounded:.*}", usize::from(decimals));
    if text.contains('.') {
        while text.ends_with('0') {
            text.pop();
        }
        if text.ends_with('.') {
            text.pop();
        }
    }
    text
}

/// Round `value` to `decimals` decimal places.
#[must_use]
pub fn round_to(value: f64, decimals: u8) -> f64 {
    let scale = 10f64.powi(i32::from(decimals));
    (value * scale).round() / scale
}

/// Choose the fewest decimals that keep the document inside `tolerance_px`
/// and preserve distinctness (§18.3).
///
/// # Errors
///
/// [`ValidationError::Svg`] if even [`MAX_DECIMALS`] cannot meet the budget,
/// which means the geometry carries detail finer than `f64` formatting can
/// express -- a defect, not a rounding choice.
pub fn choose_precision(
    document: &VectorDocument,
    tolerance_px: f64,
) -> Result<u8, ValidationError> {
    // Half the boundary budget is spent on rounding at most, leaving the rest
    // for the fitting error the budget is really for.
    let budget = (tolerance_px * 0.5).max(1e-9);

    for decimals in 0..=MAX_DECIMALS {
        if precision_is_safe(document, decimals, budget) {
            return Ok(decimals);
        }
    }
    Err(ValidationError::Svg {
        check: "coordinate_precision",
        detail: format!(
            "no precision up to {MAX_DECIMALS} decimals keeps coordinates \
             within {budget} px"
        ),
    })
}

/// Choose the precision implied by the resolved output and boundary policies.
///
/// Coverage reconstruction carries evidence at §31.2's tenth-pixel scale, so
/// automatic serialisation reserves at most `0.05 px` for rounding it. Crisp
/// and pixel-art geometry keep the broader fitting budget; integer coordinates
/// still naturally choose zero decimals.
///
/// # Errors
///
/// As [`choose_precision`].
pub fn choose_serialization_precision(
    document: &VectorDocument,
    config: &EffectiveConfig,
) -> Result<u8, ValidationError> {
    match config.output.precision {
        Precision::Fixed(decimals) => Ok(decimals.min(MAX_DECIMALS)),
        _ => {
            let tolerance = if config.geometry.boundary_source != BoundarySource::CrispGrid
                && config.profile != Profile::PixelArt
            {
                config.geometry.curve_tolerance_px.clamp(1.0e-6, 0.10)
            } else {
                config.geometry.curve_tolerance_px.max(0.5)
            };
            choose_precision(document, tolerance)
        }
    }
}

// The comparisons here are exact on purpose: "did rounding turn two different
// numbers into the same number" is a bit-equality question, and answering it
// with a tolerance would report a collapse that did not happen.
#[allow(clippy::float_cmp)]
fn precision_is_safe(document: &VectorDocument, decimals: u8, budget: f64) -> bool {
    let safe = std::cell::Cell::new(true);
    let mut check_chain = |chain: &CurveChain| {
        let mut previous: Option<Point> = None;
        let mut visit = |p: Point| {
            let rounded = Point::new(round_to(p.x, decimals), round_to(p.y, decimals));
            if (rounded.x - p.x).abs() > budget || (rounded.y - p.y).abs() > budget {
                safe.set(false);
            }
            // Two distinct consecutive points must not collapse: that would
            // delete a segment, which is a topology change (PTE-TOPO-014).
            if let Some(prev) = previous {
                let prev_rounded =
                    Point::new(round_to(prev.x, decimals), round_to(prev.y, decimals));
                let was_distinct = prev.x != p.x || prev.y != p.y;
                let still_distinct = prev_rounded.x != rounded.x || prev_rounded.y != rounded.y;
                if was_distinct && !still_distinct {
                    safe.set(false);
                }
            }
            previous = Some(p);
        };
        visit(chain.start);
        for segment in &chain.segments {
            // An arc's radii are geometry too: rounding them changes the drawn
            // curve exactly as moving an endpoint does, and a recognised
            // circle's neighbour is described entirely by them.
            if let PathSegment::Arc { radii, .. } = segment {
                for value in [radii.x, radii.y] {
                    if (round_to(value, decimals) - value).abs() > budget {
                        safe.set(false);
                    }
                }
            }
            visit(segment.end());
        }
    };

    document.for_each_element(|element| match element {
        Element::FilledFace(f) => f.boundaries.iter().for_each(&mut check_chain),
        Element::Stroke(s) => check_chain(&s.geometry),
        Element::Primitive(primitive) => {
            let check = |value: f64| {
                if (round_to(value, decimals) - value).abs() > budget {
                    safe.set(false);
                }
            };
            match primitive {
                Primitive::Rect {
                    bounds,
                    corner_radius,
                    ..
                } => {
                    for value in [bounds.x, bounds.y, bounds.width, bounds.height] {
                        check(value);
                    }
                    if let Some(radius) = corner_radius {
                        check(*radius);
                    }
                }
                Primitive::Ellipse { center, radii, .. } => {
                    for value in [center.x, center.y, radii.x, radii.y] {
                        check(value);
                    }
                }
                Primitive::Circle { center, radius, .. } => {
                    for value in [center.x, center.y, *radius] {
                        check(value);
                    }
                }
                _ => safe.set(false),
            }
        }
        Element::Group(_) => {}
        _ => {}
    });
    safe.get()
}

/// Whether any element is a recognised circle, so the snap below can be
/// skipped -- and the document left uncloned -- on the ordinary path.
fn document_has_circle(document: &VectorDocument) -> bool {
    let mut found = false;
    document.for_each_element(|element| {
        if matches!(element, Element::Primitive(Primitive::Circle { .. })) {
            found = true;
        }
    });
    found
}

/// Put every recognised circle and the arcs that retrace it on one grid.
///
/// §11.7 gives the two faces of a shared circular boundary different SVG
/// element types: the recognised face is a `<circle>`, its opaque neighbour a
/// path of two arcs through the same analytic circle. Rounding them
/// independently at write time pulls them apart. Two ways, both measured:
/// `cx` rounds to one value while the arc endpoints `cx ± r` round to a pair
/// whose midpoint is another, and -- because the two arcs are exact
/// semicircles, so their chord *is* the diameter -- the rounded chord exceeds
/// twice the rounded radius about a quarter of the time, at which point SVG
/// 1.1 F.6.6.2 obliges the renderer to scale the radii up and draw a circle
/// that is not the one beside it.
///
/// Snapping the circle first and deriving the arcs from the snapped numbers
/// removes both: `cx' ± r'` are exact multiples of the grid, so writing them
/// is the identity and the chord is exactly `2r'` (PTE-TOPO-001, PTE-SVG-009).
// The equality tests below are identity, not measurement: the arc's numbers
// are a bit-for-bit copy of the circle's, made by `lower`'s `circle_boundary`
// from one `PrimitiveRecognition`. A margin would match a *different* circle of
// nearly the same radius and snap it to the wrong grid point.
#[allow(clippy::float_cmp)]
fn snap_circles_to_grid(document: &mut VectorDocument, decimals: u8) {
    let mut snapped: Vec<(Point, f64, Point, f64)> = Vec::new();
    visit_elements_mut(&mut document.layers, &mut |element| {
        if let Element::Primitive(Primitive::Circle { center, radius, .. }) = element {
            let to = Point::new(round_to(center.x, decimals), round_to(center.y, decimals));
            let to_radius = round_to(*radius, decimals);
            snapped.push((*center, *radius, to, to_radius));
            *center = to;
            *radius = to_radius;
        }
    });
    if snapped.is_empty() {
        return;
    }

    // The neighbour's chain was built by `lower`'s `circle_boundary` from the
    // very same `PrimitiveRecognition`, so its numbers are bit-identical to the
    // circle's and matching on equality identifies them exactly.
    visit_elements_mut(&mut document.layers, &mut |element| {
        let Element::FilledFace(face) = element else {
            return;
        };
        for chain in &mut face.boundaries {
            for &(center, radius, center_to, radius_to) in &snapped {
                let (right, left) = (
                    Point::new(center.x + radius, center.y),
                    Point::new(center.x - radius, center.y),
                );
                let traces_this_circle = chain.segments.iter().any(|segment| {
                    matches!(
                        segment,
                        PathSegment::Arc { radii, to, .. }
                            if radii.x == radius && radii.y == radius
                                && (*to == right || *to == left)
                    )
                });
                if !traces_this_circle {
                    continue;
                }
                let snap_point = |p: Point| {
                    if p == right {
                        Point::new(center_to.x + radius_to, center_to.y)
                    } else if p == left {
                        Point::new(center_to.x - radius_to, center_to.y)
                    } else {
                        p
                    }
                };
                chain.start = snap_point(chain.start);
                for segment in &mut chain.segments {
                    if let PathSegment::Arc { radii, to, .. } = segment
                        && radii.x == radius
                        && radii.y == radius
                    {
                        *radii = Vec2::new(radius_to, radius_to);
                        *to = snap_point(*to);
                    }
                }
            }
        }
    });
}

/// Apply `visit` to every element, descending into groups.
fn visit_elements_mut(layers: &mut [Layer], visit: &mut impl FnMut(&mut Element)) {
    fn walk(elements: &mut [Element], visit: &mut impl FnMut(&mut Element)) {
        for element in elements {
            if let Element::Group(group) = element {
                walk(&mut group.children, visit);
            }
            visit(element);
        }
    }
    for layer in layers {
        walk(&mut layer.elements, visit);
    }
}

/// Escape text for XML content and attribute values (PTE-SEC-001).
#[must_use]
pub fn escape_xml(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            // Control characters are not legal XML 1.0 content at all, so they
            // are dropped rather than escaped into something a parser would
            // still reject.
            c if (c as u32) < 0x20 && c != '\t' && c != '\n' && c != '\r' => {}
            c => out.push(c),
        }
    }
    out
}

/// Serialise a chain's path data.
///
/// Absolute commands only. §18.4 permits relative commands and shorthand as an
/// optimisation *after* semantic validation (PTE-SVG-010); this build does not
/// apply them, so what a reader sees is exactly the typed segments.
fn write_path_data(out: &mut String, chain: &CurveChain, decimals: u8, close: bool) {
    out.push('M');
    out.push_str(&format_coordinate(chain.start.x, decimals));
    out.push(' ');
    out.push_str(&format_coordinate(chain.start.y, decimals));

    for segment in &chain.segments {
        match segment {
            PathSegment::Line { to } => {
                out.push('L');
                out.push_str(&format_coordinate(to.x, decimals));
                out.push(' ');
                out.push_str(&format_coordinate(to.y, decimals));
            }
            PathSegment::Cubic { c1, c2, to } => {
                out.push('C');
                for (i, p) in [c1, c2, to].into_iter().enumerate() {
                    if i > 0 {
                        out.push(' ');
                    }
                    out.push_str(&format_coordinate(p.x, decimals));
                    out.push(' ');
                    out.push_str(&format_coordinate(p.y, decimals));
                }
            }
            PathSegment::Arc {
                radii,
                rotation,
                large,
                sweep,
                to,
            } => {
                out.push('A');
                out.push_str(&format_coordinate(radii.x, decimals));
                out.push(' ');
                out.push_str(&format_coordinate(radii.y, decimals));
                out.push(' ');
                out.push_str(&format_coordinate(rotation.to_degrees(), decimals));
                out.push(' ');
                out.push(if *large { '1' } else { '0' });
                out.push(' ');
                out.push(if *sweep { '1' } else { '0' });
                out.push(' ');
                out.push_str(&format_coordinate(to.x, decimals));
                out.push(' ');
                out.push_str(&format_coordinate(to.y, decimals));
            }
            // `PathSegment` is `#[non_exhaustive]`; skipping an unknown segment
            // breaks the path visibly rather than emitting wrong geometry.
            _ => {}
        }
    }

    if close {
        out.push('Z');
    }
}

fn write_paint_attributes(out: &mut String, paint: &Paint, stroke: bool) {
    let attribute = if stroke { "stroke" } else { "fill" };
    match paint {
        Paint::Solid(color) => {
            out.push_str(&format!(" {attribute}=\"{}\"", color.to_hex_rgb()));
            // PTE-SVG-013: alpha appears exactly once, as an opacity, never
            // also folded into the colour.
            if color.a != 255 {
                out.push_str(&format!(
                    " {attribute}-opacity=\"{}\"",
                    format_coordinate(color.opacity(), 4)
                ));
            }
        }
        // Gradients are refused at configuration time, so a document carrying
        // one is an engine defect. Emitting a visible marker beats emitting
        // silently wrong paint.
        Paint::LinearGradient(_) | Paint::RadialGradient(_) => {
            out.push_str(&format!(" {attribute}=\"none\""));
        }
        // `Paint` is `#[non_exhaustive]`. An unknown paint paints nothing
        // rather than guessing a colour.
        _ => out.push_str(&format!(" {attribute}=\"none\"")),
    }
}

/// Serialise a document to standalone SVG (§18.1).
///
/// # Errors
///
/// [`ValidationError::Svg`] if a coordinate is not finite (PTE-SVG-003), the
/// viewport is degenerate (PTE-SVG-002), or no safe precision exists.
pub fn serialize(
    document: &VectorDocument,
    config: &EffectiveConfig,
) -> Result<String, ValidationError> {
    // PTE-SVG-003: nothing non-finite may reach the serialiser. This is the
    // gate, not a hope.
    if !document.all_coordinates_finite() {
        return Err(ValidationError::Svg {
            check: "finite_coordinates",
            detail: "the document contains a non-finite coordinate or a \
                     degenerate viewport"
                .to_owned(),
        });
    }

    let decimals = choose_serialization_precision(document, config)?;

    // The snap has to happen at the precision actually used, so it is done on
    // a local copy here rather than on the caller's document: choosing a
    // precision from already-snapped geometry could pick a coarser one and
    // reintroduce exactly the disagreement this removes.
    let snapped;
    let document = if document_has_circle(document) {
        let mut copy = document.clone();
        snap_circles_to_grid(&mut copy, decimals);
        snapped = copy;
        &snapped
    } else {
        document
    };

    let newline = if config.output.pretty { "\n" } else { "" };
    let indent = |depth: usize| {
        if config.output.pretty {
            "  ".repeat(depth)
        } else {
            String::new()
        }
    };

    let mut out = String::new();
    out.push_str(&format!(
        "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"{}\" height=\"{}\" \
         viewBox=\"{} {} {} {}\">{newline}",
        format_coordinate(document.viewport.width, decimals),
        format_coordinate(document.viewport.height, decimals),
        format_coordinate(document.viewport.x, decimals),
        format_coordinate(document.viewport.y, decimals),
        format_coordinate(document.viewport.width, decimals),
        format_coordinate(document.viewport.height, decimals),
    ));

    if config.output.metadata {
        // §18.6: engine, version, profile and digest only. There is no field
        // here for a path, a username, or a timestamp, so none can leak.
        out.push_str(&format!(
            "{}<metadata><!--{} {} profile={} digest={}--></metadata>{newline}",
            indent(1),
            escape_xml(&document.provenance.engine),
            escape_xml(&document.provenance.version),
            escape_xml(&document.provenance.profile),
            escape_xml(&document.provenance.semantic_digest),
        ));
    }

    for layer in &document.layers {
        out.push_str(&format!(
            "{}<g id=\"{}\" data-role=\"{}\"",
            indent(1),
            escape_xml(&layer.id),
            escape_xml(&layer.role)
        ));
        if let Some(label) = &layer.label {
            out.push_str(&format!(" aria-label=\"{}\"", escape_xml(label)));
        }
        out.push_str(&format!(">{newline}"));

        for (index, element) in layer.elements.iter().enumerate() {
            write_element(&mut out, element, decimals, &indent(2), newline, index);
        }
        out.push_str(&format!("{}</g>{newline}", indent(1)));
    }

    out.push_str("</svg>");
    if config.output.pretty {
        out.push('\n');
    }
    Ok(out)
}

fn write_element(
    out: &mut String,
    element: &Element,
    decimals: u8,
    indent: &str,
    newline: &str,
    index: usize,
) {
    match element {
        Element::FilledFace(face) => {
            out.push_str(indent);
            // Identifiers come from the topological face, which is a stable
            // raster-derived number, not from a counter (PTE-API-010).
            out.push_str(&format!("<path id=\"face-{}\"", face.face.0));
            write_paint_attributes(out, &face.paint, false);
            if face.fill_rule == FillRule::EvenOdd {
                out.push_str(" fill-rule=\"evenodd\"");
            }
            out.push_str(" d=\"");
            for (i, boundary) in face.boundaries.iter().enumerate() {
                if i > 0 {
                    out.push(' ');
                }
                write_path_data(out, boundary, decimals, true);
            }
            out.push_str(&format!("\"/>{newline}"));
        }
        Element::Stroke(stroke) => {
            out.push_str(indent);
            out.push_str(&format!("<path id=\"edge-{index}\" fill=\"none\""));
            write_paint_attributes(out, &stroke.paint, true);
            out.push_str(&format!(
                " stroke-width=\"{}\" stroke-linecap=\"round\" \
                 stroke-linejoin=\"round\" d=\"",
                format_coordinate(stroke.width, decimals)
            ));
            write_path_data(out, &stroke.geometry, decimals, stroke.closed);
            out.push_str(&format!("\"/>{newline}"));
        }
        Element::Primitive(primitive) => match primitive {
            Primitive::Rect {
                face,
                bounds,
                corner_radius,
                paint,
            } => {
                out.push_str(indent);
                out.push_str(&format!(
                    "<rect id=\"face-{}\" x=\"{}\" y=\"{}\" width=\"{}\" height=\"{}\"",
                    face.0,
                    format_coordinate(bounds.x, decimals),
                    format_coordinate(bounds.y, decimals),
                    format_coordinate(bounds.width, decimals),
                    format_coordinate(bounds.height, decimals)
                ));
                if let Some(radius) = corner_radius {
                    out.push_str(&format!(" rx=\"{}\"", format_coordinate(*radius, decimals)));
                }
                write_paint_attributes(out, paint, false);
                out.push_str(&format!("/>{newline}"));
            }
            Primitive::Ellipse {
                face,
                center,
                radii,
                paint,
            } => {
                out.push_str(indent);
                out.push_str(&format!(
                    "<ellipse id=\"face-{}\" cx=\"{}\" cy=\"{}\" rx=\"{}\" ry=\"{}\"",
                    face.0,
                    format_coordinate(center.x, decimals),
                    format_coordinate(center.y, decimals),
                    format_coordinate(radii.x, decimals),
                    format_coordinate(radii.y, decimals)
                ));
                write_paint_attributes(out, paint, false);
                out.push_str(&format!("/>{newline}"));
            }
            Primitive::Circle {
                face,
                center,
                radius,
                paint,
            } => {
                out.push_str(indent);
                out.push_str(&format!(
                    "<circle id=\"face-{}\" cx=\"{}\" cy=\"{}\" r=\"{}\"",
                    face.0,
                    format_coordinate(center.x, decimals),
                    format_coordinate(center.y, decimals),
                    format_coordinate(*radius, decimals)
                ));
                write_paint_attributes(out, paint, false);
                out.push_str(&format!("/>{newline}"));
            }
            _ => {}
        },
        Element::Group(group) => {
            out.push_str(indent);
            out.push_str(&format!("<g id=\"{}\"", escape_xml(&group.id)));
            if let Some(label) = &group.label {
                out.push_str(&format!(" aria-label=\"{}\"", escape_xml(label)));
            }
            out.push_str(&format!(">{newline}"));
            for (i, child) in group.children.iter().enumerate() {
                write_element(out, child, decimals, indent, newline, i);
            }
            out.push_str(indent);
            out.push_str(&format!("</g>{newline}"));
        }
        // `Element` is `#[non_exhaustive]`; an unknown element is omitted
        // rather than approximated.
        _ => {}
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
    use super::*;
    use palette_tracer_core::config::{Profile, TraceConfig};
    use palette_tracer_core::ir::topology::FaceId;
    use palette_tracer_core::ir::{
        Color, DocumentProvenance, FilledFace, Layer, PaletteId, Rect, StrokePath,
    };

    fn config() -> EffectiveConfig {
        Profile::expand(&TraceConfig::default()).unwrap()
    }

    fn document(elements: Vec<Element>) -> VectorDocument {
        VectorDocument {
            viewport: Rect::image_domain(4, 4),
            physical_size: None,
            palette: vec![Paint::Solid(Color::BLACK)],
            defs: Vec::new(),
            layers: vec![Layer {
                id: "fills".into(),
                label: None,
                role: "shared-mosaic-fills".into(),
                elements,
            }],
            provenance: DocumentProvenance {
                engine: "palette-tracer-engine".into(),
                version: "0.1.0".into(),
                profile: "flat-illustration".into(),
                semantic_digest: "pte-semantic-v1-blake3:abc".into(),
            },
        }
    }

    fn face_of(chain: CurveChain, id: u32) -> Element {
        Element::FilledFace(FilledFace {
            face: FaceId(id),
            palette: Some(PaletteId(0)),
            paint: Paint::Solid(Color::rgb(0x12, 0x34, 0x56)),
            boundaries: vec![chain],
            fill_rule: FillRule::NonZero,
        })
    }

    #[test]
    fn output_is_a_standalone_svg_with_a_finite_viewport() {
        let chain = CurveChain::polyline(&[
            Point::new(0.0, 0.0),
            Point::new(1.0, 0.0),
            Point::new(0.0, 0.0),
        ])
        .unwrap();
        let svg = serialize(&document(vec![face_of(chain, 1)]), &config()).unwrap();

        assert!(svg.starts_with("<svg xmlns=\"http://www.w3.org/2000/svg\""));
        assert!(svg.contains("viewBox=\"0 0 4 4\""));
        assert!(svg.ends_with("</svg>"));
        // §18.1: no scripts, no external references.
        for forbidden in ["<script", "xlink:href", "http://", "https://"] {
            let occurrences = svg.matches(forbidden).count();
            let allowed = usize::from(forbidden == "http://"); // the namespace
            assert_eq!(occurrences, allowed, "`{forbidden}` in output");
        }
    }

    /// PTE-SVG-009, checked on the emitted text. The two faces of a shared
    /// interface must contain the same coordinate bytes.
    #[test]
    fn shared_coordinates_are_byte_identical_after_reversal() {
        // One chain, and two faces built from it -- one forwards, one
        // reversed, exactly as the topology hands them over.
        let chain = CurveChain::polyline(&[
            Point::new(1.0, 0.5),
            Point::new(2.25, 1.75),
            Point::new(3.5, 0.5),
        ])
        .unwrap();
        let doc = document(vec![
            face_of(chain.clone(), 1),
            face_of(chain.reversed(), 2),
        ]);
        let svg = serialize(&doc, &config()).unwrap();

        // Pull the two path-data strings out and compare their coordinate
        // tokens. Asserting on the *text* is the point: byte-identical
        // coordinates are what stops a renderer seeing a crack, and no
        // property of the f64 geometry guarantees it.
        let paths: Vec<&str> = svg
            .match_indices(" d=\"")
            .map(|(i, m)| {
                let rest = &svg[i + m.len()..];
                &rest[..rest.find('"').unwrap_or(rest.len())]
            })
            .collect();
        assert_eq!(paths.len(), 2, "two faces, two paths: {svg}");

        let tokens = |path: &str| -> Vec<String> {
            path.split(['M', 'L', 'C', 'A', 'Z'])
                .flat_map(|part| part.split_whitespace())
                .map(str::to_owned)
                .collect()
        };
        let forward = tokens(paths[0]);
        let backward = tokens(paths[1]);
        assert!(!forward.is_empty());
        assert_eq!(
            forward.len(),
            backward.len(),
            "the same coordinates, in the other order"
        );

        // Coordinates come in (x, y) pairs, so reversing the traversal
        // reverses the pairs, not the individual numbers.
        let pairs = |flat: &[String]| -> Vec<(String, String)> {
            flat.chunks(2)
                .filter(|c| c.len() == 2)
                .map(|c| (c[0].clone(), c[1].clone()))
                .collect()
        };
        let mut reversed = pairs(&backward);
        reversed.reverse();
        assert_eq!(
            pairs(&forward),
            reversed,
            "the two faces must emit byte-identical coordinates"
        );
    }

    /// PTE-SVG-008: "A fixed 'three decimals for everything' rule is
    /// forbidden." Integer geometry needs none, and fine geometry gets more.
    #[test]
    fn precision_is_searched_not_fixed() {
        let integers = CurveChain::polyline(&[Point::new(0.0, 0.0), Point::new(2.0, 3.0)]).unwrap();
        assert_eq!(
            choose_precision(&document(vec![face_of(integers, 1)]), 1.0).unwrap(),
            0
        );

        let fine =
            CurveChain::polyline(&[Point::new(0.0, 0.0), Point::new(0.001_25, 0.002_5)]).unwrap();
        let decimals = choose_precision(&document(vec![face_of(fine, 1)]), 0.001).unwrap();
        assert!(
            decimals >= 3,
            "fine geometry needs decimals, got {decimals}"
        );
    }

    /// §10 evidence must survive §18.3's final geometry-changing step. The
    /// general 0.6 px profile budget would permit this quarter-pixel position
    /// to round back onto the grid; the reconstruction budget must not.
    #[test]
    fn automatic_precision_preserves_subpixel_reconstruction() {
        let subpixel = CurveChain::polyline(&[
            Point::new(0.24, 0.0),
            Point::new(0.24, 1.0),
            Point::new(1.0, 1.0),
        ])
        .unwrap();
        let doc = document(vec![face_of(subpixel, 1)]);
        let config = config();

        let decimals = choose_serialization_precision(&doc, &config).unwrap();
        assert!(
            decimals >= 1,
            "subpixel evidence needs at least one decimal"
        );
        let svg = serialize(&doc, &config).unwrap();
        assert!(svg.contains("0.2"), "subpixel coordinate was erased: {svg}");
    }

    /// Rounding must not collapse two distinct points into one: that deletes a
    /// segment, which is a topology change (PTE-TOPO-014).
    #[test]
    fn precision_never_collapses_two_distinct_points() {
        let close = CurveChain::polyline(&[
            Point::new(0.0, 0.0),
            Point::new(0.4, 0.0),
            Point::new(1.0, 1.0),
        ])
        .unwrap();
        let decimals = choose_precision(&document(vec![face_of(close, 1)]), 4.0).unwrap();
        assert!(
            decimals >= 1,
            "zero decimals would merge 0 and 0.4; chose {decimals}"
        );
    }

    /// PTE-SVG-003: a non-finite coordinate is refused, never emitted.
    #[test]
    fn a_non_finite_coordinate_is_refused() {
        let bad =
            CurveChain::polyline(&[Point::new(0.0, 0.0), Point::new(f64::INFINITY, 1.0)]).unwrap();
        let err = serialize(&document(vec![face_of(bad, 1)]), &config()).unwrap_err();
        assert_eq!(err.code(), "validation.svg");
    }

    /// PTE-SEC-001: a hostile label cannot inject markup.
    #[test]
    fn a_hostile_label_is_escaped() {
        let chain = CurveChain::polyline(&[Point::new(0.0, 0.0), Point::new(1.0, 1.0)]).unwrap();
        let mut doc = document(vec![face_of(chain, 1)]);
        doc.layers[0].label = Some("\"><script>alert('x')</script>".into());

        let svg = serialize(&doc, &config()).unwrap();
        assert!(!svg.contains("<script"), "{svg}");
        assert!(svg.contains("&lt;script&gt;"), "{svg}");
        assert!(svg.contains("&quot;"), "{svg}");
    }

    #[test]
    fn escaping_covers_every_xml_metacharacter() {
        assert_eq!(
            escape_xml("a&b<c>d\"e'f"),
            "a&amp;b&lt;c&gt;d&quot;e&apos;f"
        );
        assert_eq!(
            escape_xml("keep\ttabs\nand\rnewlines"),
            "keep\ttabs\nand\rnewlines"
        );
        assert_eq!(escape_xml("drop\u{0}control"), "dropcontrol");
    }

    /// PTE-SVG-013: alpha appears once, as an opacity, and never folded into
    /// the hex colour.
    #[test]
    fn alpha_is_applied_exactly_once() {
        let chain = CurveChain::polyline(&[Point::new(0.0, 0.0), Point::new(1.0, 1.0)]).unwrap();
        let mut doc = document(vec![face_of(chain, 1)]);
        if let Element::FilledFace(f) = &mut doc.layers[0].elements[0] {
            f.paint = Paint::Solid(Color::rgba(0x12, 0x34, 0x56, 128));
        }
        let svg = serialize(&doc, &config()).unwrap();
        assert!(svg.contains("fill=\"#123456\""), "{svg}");
        assert!(svg.contains("fill-opacity="), "{svg}");
    }

    /// §18.6: no paths, usernames or timestamps. The metadata carries four
    /// declared fields and nothing else.
    #[test]
    fn metadata_carries_only_the_declared_fields() {
        let chain = CurveChain::polyline(&[Point::new(0.0, 0.0), Point::new(1.0, 1.0)]).unwrap();
        let svg = serialize(&document(vec![face_of(chain, 1)]), &config()).unwrap();
        assert!(svg.contains("palette-tracer-engine"));
        assert!(svg.contains("digest=pte-semantic-v1-blake3:abc"));
        assert!(!svg.contains("/home/"), "{svg}");
    }

    #[test]
    fn coordinate_formatting_is_canonical() {
        assert_eq!(format_coordinate(0.0, 3), "0");
        assert_eq!(format_coordinate(-0.0, 3), "0");
        assert_eq!(
            format_coordinate(-0.000_01, 3),
            "0",
            "rounds to zero, not -0"
        );
        assert_eq!(format_coordinate(1.500, 3), "1.5");
        assert_eq!(format_coordinate(2.0, 3), "2");
        assert_eq!(format_coordinate(-3.25, 3), "-3.25");
    }

    /// Serialisation must be a pure function of the document and the config.
    #[test]
    fn serialisation_is_reproducible() {
        let chain = CurveChain::polyline(&[
            Point::new(0.0, 0.0),
            Point::new(1.5, 2.5),
            Point::new(0.0, 0.0),
        ])
        .unwrap();
        let doc = document(vec![face_of(chain, 1)]);
        let first = serialize(&doc, &config()).unwrap();
        for _ in 0..4 {
            assert_eq!(serialize(&doc, &config()).unwrap(), first);
        }
    }

    #[test]
    fn a_stroke_is_emitted_with_no_fill() {
        let chain = CurveChain::polyline(&[Point::new(0.0, 0.0), Point::new(4.0, 4.0)]).unwrap();
        let doc = document(vec![Element::Stroke(StrokePath {
            geometry: chain,
            paint: Paint::Solid(Color::BLACK),
            width: 1.0,
            closed: false,
        })]);
        let svg = serialize(&doc, &config()).unwrap();
        assert!(svg.contains("fill=\"none\""), "{svg}");
        assert!(svg.contains("stroke=\"#000000\""), "{svg}");
        assert!(!svg.contains('Z'), "an open stroke must not be closed");
    }
}
