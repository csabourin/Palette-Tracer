//! The semantic digest (Appendix F).
//!
//! F.1: "The semantic digest detects nondeterminism without making irrelevant
//! XML formatting part of the algorithm contract."
//!
//! The stream is built from tagged tokens rather than from serialised JSON,
//! because JSON would make a field rename a digest change and would drag map
//! ordering into the contract. Each token is length-prefixed and type-tagged,
//! so two different structures cannot hash to the same bytes by accident.
//!
//! # What is in the stream (F.2)
//!
//! Digest schema version; algorithm-stage versions and output-affecting
//! feature flags; the output-affecting configuration; viewport and physical
//! size; palette and paint definitions in stable identifier order; layers and
//! elements in canonical semantic order; geometry with a canonical cycle start
//! and orientation; output-affecting fallbacks.
//!
//! # What is not
//!
//! Timings, allocation counts, progress events, human messages, filenames,
//! host identity, and non-semantic metadata (F.2). Also **serialisation
//! policy**: indentation, whether `<metadata>` is emitted, and the chosen
//! decimal precision are excluded, because §20.1 defines the digest over the
//! quantised typed IR rather than over SVG bytes, and Appendix E.1 classifies
//! a precision change as invalidating serialisation only. Two builds that
//! agree on geometry and disagree on formatting have the same digest, which is
//! exactly the property PTE-DET-001 needs across native and WASM.
//!
//! # Not a signature
//!
//! F.4: "The digest is a reproducibility tool, not a security signature and
//! not a stable cache key across unannounced algorithm versions."

use crate::config::EffectiveConfig;
use crate::determinism::QuantKey;
use crate::error::NumericalError;
use crate::ir::{CurveChain, Element, PathSegment, Point, VectorDocument};
use crate::report::{AlgorithmVersions, Fallback};

/// Digest schema version, the first token in the stream.
pub const DIGEST_SCHEMA_VERSION: u32 = 1;

/// Prefix identifying the algorithm and canonical schema (F.4).
pub const DIGEST_PREFIX: &str = "pte-semantic-v1-blake3";

/// Quantiser for coordinates in the digest stream (F.3).
///
/// One millionth of a source pixel: far below the tightest §31.2 gate
/// (0.10 px), so a difference that matters is always visible and a difference
/// of one floating-point ulp never is.
const COORD_SCALE: f64 = 1.0e6;

/// Builds the canonical stream.
#[derive(Debug)]
pub struct DigestWriter {
    hasher: blake3::Hasher,
}

impl Default for DigestWriter {
    fn default() -> Self {
        Self::new()
    }
}

impl DigestWriter {
    /// Start a stream and write the schema version.
    #[must_use]
    pub fn new() -> Self {
        let mut w = Self {
            hasher: blake3::Hasher::new(),
        };
        w.tag("pte-digest");
        w.u64(u64::from(DIGEST_SCHEMA_VERSION));
        w
    }

    /// Write a structural tag, so two different shapes cannot collide.
    pub fn tag(&mut self, tag: &str) {
        self.hasher.update(b"\x01");
        self.hasher.update(&(tag.len() as u64).to_le_bytes());
        self.hasher.update(tag.as_bytes());
    }

    /// Write an unsigned integer.
    pub fn u64(&mut self, value: u64) {
        self.hasher.update(b"\x02");
        self.hasher.update(&value.to_le_bytes());
    }

    /// Write a signed integer.
    pub fn i64(&mut self, value: i64) {
        self.hasher.update(b"\x03");
        self.hasher.update(&value.to_le_bytes());
    }

    /// Write a string.
    pub fn str(&mut self, value: &str) {
        self.hasher.update(b"\x04");
        self.hasher.update(&(value.len() as u64).to_le_bytes());
        self.hasher.update(value.as_bytes());
    }

    /// Write a boolean.
    pub fn bool(&mut self, value: bool) {
        self.hasher.update(b"\x05");
        self.hasher.update(&[u8::from(value)]);
    }

    /// Write a quantised coordinate (F.3).
    ///
    /// `-0.0` canonicalises to `0`, because [`QuantKey`] rounds the scaled
    /// product and `(-0.0 * s).round()` is `-0.0`, whose `as i64` is `0`.
    ///
    /// # Errors
    ///
    /// [`NumericalError::NonFinite`] -- F.3 requires NaN and infinity to be
    /// rejected, not encoded.
    pub fn coord(&mut self, value: f64) -> Result<(), NumericalError> {
        let key = QuantKey::new(value, COORD_SCALE, "digest coordinate")?;
        self.i64(key.raw());
        Ok(())
    }

    /// Write a quantised scalar that is not a coordinate.
    ///
    /// # Errors
    ///
    /// [`NumericalError::NonFinite`] as [`DigestWriter::coord`].
    pub fn scalar(&mut self, value: f64) -> Result<(), NumericalError> {
        let key = QuantKey::cost(value)?;
        self.i64(key.raw());
        Ok(())
    }

    /// Finish, returning `pte-semantic-v1-blake3:<hex>`.
    #[must_use]
    pub fn finish(self) -> String {
        format!("{DIGEST_PREFIX}:{}", self.hasher.finalize().to_hex())
    }
}

/// Quantise a point for canonical-rotation comparison.
fn point_key(p: Point) -> Option<(i64, i64)> {
    let x = QuantKey::new(p.x, COORD_SCALE, "digest coordinate").ok()?;
    let y = QuantKey::new(p.y, COORD_SCALE, "digest coordinate").ok()?;
    Some((x.raw(), y.raw()))
}

/// Write a chain, rotating a closed one to a canonical start (F.3).
///
/// F.3: "Cyclic face paths rotate to the lexicographically smallest valid
/// start token while preserving required winding." Two runs that walked the
/// same closed boundary from different vertices therefore agree, while a run
/// that reversed the winding does not -- the rotation preserves direction.
///
/// # Errors
///
/// [`NumericalError::NonFinite`] if any coordinate is not finite.
fn write_chain(w: &mut DigestWriter, chain: &CurveChain) -> Result<(), NumericalError> {
    let mut points = Vec::with_capacity(chain.segments.len() + 1);
    points.push(chain.start);
    for s in &chain.segments {
        points.push(s.end());
    }

    let closed = points
        .last()
        .and_then(|&last| point_key(last).zip(point_key(chain.start)))
        .is_some_and(|(a, b)| a == b);

    // For a closed chain the last point repeats the first, so the rotatable
    // sequence is the segments paired with the vertex each starts from.
    let rotation = if closed && !chain.segments.is_empty() {
        let n = chain.segments.len();
        let mut best = 0usize;
        let mut best_key = point_key(points[0]);
        for (i, p) in points.iter().take(n).enumerate().skip(1) {
            let key = point_key(*p);
            // `None` means a non-finite coordinate, which the writes below
            // will reject with a proper error; treat it as never smallest.
            if let (Some(k), Some(b)) = (key, best_key) {
                if k < b {
                    best = i;
                    best_key = key;
                }
            } else if best_key.is_none() {
                best_key = key;
                best = i;
            }
        }
        best
    } else {
        0
    };

    w.tag("chain");
    w.bool(closed);
    w.u64(chain.segments.len() as u64);
    w.coord(points[rotation].x)?;
    w.coord(points[rotation].y)?;

    let n = chain.segments.len();
    for offset in 0..n {
        let i = if closed {
            (rotation + offset) % n
        } else {
            offset
        };
        match chain.segments[i] {
            PathSegment::Line { to } => {
                w.tag("L");
                w.coord(to.x)?;
                w.coord(to.y)?;
            }
            PathSegment::Cubic { c1, c2, to } => {
                w.tag("C");
                for p in [c1, c2, to] {
                    w.coord(p.x)?;
                    w.coord(p.y)?;
                }
            }
            PathSegment::Arc {
                radii,
                rotation: rot,
                large,
                sweep,
                to,
            } => {
                w.tag("A");
                w.coord(radii.x)?;
                w.coord(radii.y)?;
                w.scalar(rot)?;
                w.bool(large);
                w.bool(sweep);
                w.coord(to.x)?;
                w.coord(to.y)?;
            }
        }
    }
    Ok(())
}

fn write_paint(w: &mut DigestWriter, paint: &crate::ir::Paint) -> Result<(), NumericalError> {
    match paint {
        crate::ir::Paint::Solid(c) => {
            w.tag("solid");
            w.u64(u64::from(u32::from_be_bytes([c.r, c.g, c.b, c.a])));
        }
        crate::ir::Paint::LinearGradient(g) => {
            w.tag("linear");
            for p in [g.from, g.to] {
                w.coord(p.x)?;
                w.coord(p.y)?;
            }
            w.u64(g.stops.len() as u64);
            for s in &g.stops {
                w.scalar(s.offset)?;
                w.u64(u64::from(u32::from_be_bytes([
                    s.color.r, s.color.g, s.color.b, s.color.a,
                ])));
            }
        }
        crate::ir::Paint::RadialGradient(g) => {
            w.tag("radial");
            w.coord(g.center.x)?;
            w.coord(g.center.y)?;
            w.coord(g.radius)?;
            w.u64(g.stops.len() as u64);
            for s in &g.stops {
                w.scalar(s.offset)?;
                w.u64(u64::from(u32::from_be_bytes([
                    s.color.r, s.color.g, s.color.b, s.color.a,
                ])));
            }
        }
    }
    Ok(())
}

fn write_element(w: &mut DigestWriter, element: &Element) -> Result<(), NumericalError> {
    match element {
        Element::FilledFace(f) => {
            w.tag("face");
            w.u64(u64::from(f.face.0));
            match f.palette {
                Some(p) => w.u64(u64::from(p.0)),
                None => w.tag("no-palette"),
            }
            write_paint(w, &f.paint)?;
            w.tag(match f.fill_rule {
                crate::ir::FillRule::NonZero => "nonzero",
                crate::ir::FillRule::EvenOdd => "evenodd",
            });
            w.u64(f.boundaries.len() as u64);
            for chain in &f.boundaries {
                write_chain(w, chain)?;
            }
        }
        Element::Stroke(s) => {
            w.tag("stroke");
            write_paint(w, &s.paint)?;
            w.coord(s.width)?;
            w.bool(s.closed);
            write_chain(w, &s.geometry)?;
        }
        Element::Primitive(p) => {
            w.tag("primitive");
            // Primitives keep their semantics until lowering (PTE-GEO-011), so
            // the digest records the semantic form, not a cubic approximation.
            match p {
                crate::ir::Primitive::Rect {
                    bounds,
                    corner_radius,
                    paint,
                } => {
                    w.tag("rect");
                    for v in [bounds.x, bounds.y, bounds.width, bounds.height] {
                        w.coord(v)?;
                    }
                    match corner_radius {
                        Some(r) => w.coord(*r)?,
                        None => w.tag("sharp"),
                    }
                    write_paint(w, paint)?;
                }
                crate::ir::Primitive::Ellipse {
                    center,
                    radii,
                    paint,
                } => {
                    w.tag("ellipse");
                    for v in [center.x, center.y, radii.x, radii.y] {
                        w.coord(v)?;
                    }
                    write_paint(w, paint)?;
                }
            }
        }
        Element::Group(g) => {
            w.tag("group");
            w.u64(g.children.len() as u64);
            for child in &g.children {
                write_element(w, child)?;
            }
        }
    }
    Ok(())
}

/// Write the output-affecting configuration (F.2 item 3).
///
/// Serialisation policy is excluded; see the module documentation.
fn write_config(w: &mut DigestWriter, config: &EffectiveConfig) -> Result<(), NumericalError> {
    w.tag("config");
    w.u64(u64::from(config.schema_version));
    w.u64(u64::from(config.profile_schema_version));
    w.str(config.profile.as_str());

    w.tag("modifiers");
    w.u64(config.modifiers.len() as u64);
    for m in &config.modifiers {
        w.str(m.as_str());
    }

    w.tag("palette");
    w.u64(u64::from(config.palette.max_colors));
    w.scalar(config.palette.default_reach.lightness.get())?;
    w.scalar(config.palette.default_reach.chroma.get())?;
    w.scalar(config.palette.default_reach.hue.get())?;
    w.u64(config.palette.entries.len() as u64);
    for e in &config.palette.entries {
        w.u64(u64::from(e.id));
        w.u64(u64::from(u32::from_be_bytes(e.rgba)));
        w.bool(e.pinned);
        w.i64(i64::from(e.priority));
        w.scalar(e.reach.lightness.get())?;
        w.scalar(e.reach.chroma.get())?;
        w.scalar(e.reach.hue.get())?;
    }

    w.tag("segmentation");
    for v in [
        config.segmentation.edge_weight_color,
        config.segmentation.edge_weight_gradient,
        config.segmentation.edge_weight_alpha,
        config.segmentation.edge_weight_claim,
        config.segmentation.merge_threshold,
        config.segmentation.fringe_max_residual,
    ] {
        w.scalar(v)?;
    }
    w.u64(u64::from(config.segmentation.small_region_pixels));
    w.bool(config.segmentation.protect_thin_features);

    w.tag("geometry");
    for v in [
        config.geometry.curve_tolerance_px,
        config.geometry.corner_threshold_deg,
        config.geometry.corner_hysteresis_deg,
    ] {
        w.scalar(v)?;
    }
    w.bool(config.geometry.allow_arcs);
    w.u64(u64::from(config.geometry.max_span_candidates));

    w.tag("color");
    w.scalar(config.color.neutral_chroma_threshold)?;
    w.bool(config.color.alpha.preserve);
    w.scalar(config.color.alpha.unpremultiply_epsilon.get())?;

    // Enum discriminants go in as their serialised names so a reordering of an
    // enum's variants cannot silently change the digest.
    for value in [
        serde_json::to_string(&config.palette.mode),
        serde_json::to_string(&config.color.background),
        serde_json::to_string(&config.geometry.mode),
        serde_json::to_string(&config.geometry.boundary_source),
        serde_json::to_string(&config.geometry.pixel_art_mode),
        serde_json::to_string(&config.strokes),
        serde_json::to_string(&config.gradients),
        serde_json::to_string(&config.determinism),
    ] {
        w.str(&value.unwrap_or_else(|_| "?".into()));
    }
    Ok(())
}

/// Compute the semantic digest of a finished document (Appendix F).
///
/// # Errors
///
/// [`NumericalError::NonFinite`] if any coordinate is NaN or infinite. F.3
/// rejects them rather than encoding them, so a document that fails here would
/// also fail the §31.1 finiteness gate.
pub fn digest_document(
    document: &VectorDocument,
    config: &EffectiveConfig,
    algorithms: &AlgorithmVersions,
    features: &[&str],
    fallbacks: &[Fallback],
) -> Result<String, NumericalError> {
    let mut w = DigestWriter::new();

    // 2. Algorithm-stage versions and output-affecting feature flags.
    w.tag("algorithms");
    w.str(&algorithms.segmentation);
    w.str(&algorithms.topology);
    w.str(&algorithms.boundary);
    w.str(&algorithms.curves);
    w.tag("features");
    w.u64(features.len() as u64);
    for f in features {
        w.str(f);
    }

    // 3. Output-affecting configuration.
    write_config(&mut w, config)?;

    // 4. Viewport and physical-unit metadata.
    w.tag("viewport");
    for v in [
        document.viewport.x,
        document.viewport.y,
        document.viewport.width,
        document.viewport.height,
    ] {
        w.coord(v)?;
    }
    match &document.physical_size {
        Some(p) => {
            w.tag("physical");
            w.scalar(p.width_mm)?;
            w.scalar(p.height_mm)?;
        }
        None => w.tag("no-physical"),
    }

    // 5. Palette and paint definitions in stable identifier order.
    w.tag("paints");
    w.u64(document.palette.len() as u64);
    for paint in &document.palette {
        write_paint(&mut w, paint)?;
    }
    w.tag("defs");
    w.u64(document.defs.len() as u64);
    for def in &document.defs {
        w.str(&def.id);
        write_paint(&mut w, &def.paint)?;
    }

    // 6 and 7. Layers and elements in canonical semantic order, with geometry.
    w.tag("layers");
    w.u64(document.layers.len() as u64);
    for layer in &document.layers {
        w.tag("layer");
        w.str(&layer.role);
        w.u64(layer.elements.len() as u64);
        for element in &layer.elements {
            write_element(&mut w, element)?;
        }
    }

    // 9. Output-affecting fallbacks.
    w.tag("fallbacks");
    w.u64(fallbacks.len() as u64);
    for f in fallbacks {
        w.str(&f.code);
        w.str(&f.requested);
        w.str(&f.effective);
    }

    Ok(w.finish())
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
    use crate::config::{Profile, TraceConfig};
    use crate::ir::topology::{FaceId, PaletteId};
    use crate::ir::{Color, DocumentProvenance, FillRule, FilledFace, Layer, Paint, Rect};

    fn closed_square(offset: f64) -> CurveChain {
        CurveChain::polyline(&[
            Point::new(offset, 0.0),
            Point::new(offset + 2.0, 0.0),
            Point::new(offset + 2.0, 2.0),
            Point::new(offset, 2.0),
            Point::new(offset, 0.0),
        ])
        .unwrap()
    }

    fn document(chains: Vec<CurveChain>) -> VectorDocument {
        VectorDocument {
            viewport: Rect::image_domain(8, 8),
            physical_size: None,
            palette: vec![Paint::Solid(Color::rgb(10, 20, 30))],
            defs: Vec::new(),
            layers: vec![Layer {
                id: "layer-0".into(),
                label: Some("ignored prose".into()),
                role: "fills".into(),
                elements: chains
                    .into_iter()
                    .map(|c| {
                        crate::ir::Element::FilledFace(FilledFace {
                            face: FaceId(1),
                            palette: Some(PaletteId(0)),
                            paint: Paint::Solid(Color::rgb(10, 20, 30)),
                            boundaries: vec![c],
                            fill_rule: FillRule::NonZero,
                        })
                    })
                    .collect(),
            }],
            provenance: DocumentProvenance {
                engine: "e".into(),
                version: "v".into(),
                profile: "flat-illustration".into(),
                semantic_digest: String::new(),
            },
        }
    }

    fn digest(doc: &VectorDocument) -> String {
        let config = Profile::expand(&TraceConfig::default()).unwrap();
        digest_document(doc, &config, &AlgorithmVersions::default(), &[], &[]).unwrap()
    }

    #[test]
    fn the_digest_carries_its_algorithm_and_schema_prefix() {
        let d = digest(&document(vec![closed_square(0.0)]));
        assert!(d.starts_with("pte-semantic-v1-blake3:"), "{d}");
        assert_eq!(d.len(), DIGEST_PREFIX.len() + 1 + 64);
    }

    #[test]
    fn identical_documents_digest_identically() {
        let a = document(vec![closed_square(0.0)]);
        let b = document(vec![closed_square(0.0)]);
        assert_eq!(digest(&a), digest(&b));
    }

    #[test]
    fn a_moved_boundary_changes_the_digest() {
        let a = document(vec![closed_square(0.0)]);
        let b = document(vec![closed_square(1.0)]);
        assert_ne!(digest(&a), digest(&b));
    }

    /// F.2 excludes human messages and non-semantic metadata. A label is
    /// prose; changing it must not change the digest.
    #[test]
    fn non_semantic_fields_do_not_reach_the_digest() {
        let mut a = document(vec![closed_square(0.0)]);
        let before = digest(&a);

        a.layers[0].label = Some("completely different prose".into());
        a.layers[0].id = "layer-renamed".into();
        a.provenance.version = "99.99.99".into();
        a.provenance.engine = "something-else".into();
        assert_eq!(digest(&a), before);
    }

    /// F.3: a closed path rotated to a different start vertex is the same
    /// path. Two runs that began the walk at different corners must agree.
    #[test]
    fn a_rotated_closed_path_digests_identically() {
        let square = closed_square(0.0);
        let rotated = CurveChain::polyline(&[
            Point::new(2.0, 2.0),
            Point::new(0.0, 2.0),
            Point::new(0.0, 0.0),
            Point::new(2.0, 0.0),
            Point::new(2.0, 2.0),
        ])
        .unwrap();
        assert_eq!(
            digest(&document(vec![square])),
            digest(&document(vec![rotated]))
        );
    }

    /// Rotation preserves winding, so a reversed traversal is a different
    /// semantic object and must not collide.
    #[test]
    fn a_reversed_closed_path_does_not_collide_with_the_forward_one() {
        let forward = closed_square(0.0);
        let backward = forward.reversed();
        assert_ne!(
            digest(&document(vec![forward])),
            digest(&document(vec![backward]))
        );
    }

    /// PTE-DET-004: changing a stage's algorithm version changes the digest,
    /// so a fixture review is forced rather than optional.
    #[test]
    fn an_algorithm_version_change_changes_the_digest() {
        let doc = document(vec![closed_square(0.0)]);
        let config = Profile::expand(&TraceConfig::default()).unwrap();
        let base = digest_document(&doc, &config, &AlgorithmVersions::default(), &[], &[]).unwrap();
        let bumped = digest_document(
            &doc,
            &config,
            &AlgorithmVersions {
                topology: std::borrow::Cow::Borrowed("pte-dcel/2"),
                ..AlgorithmVersions::default()
            },
            &[],
            &[],
        )
        .unwrap();
        assert_ne!(base, bumped);
    }

    #[test]
    fn a_configuration_change_that_affects_output_changes_the_digest() {
        let doc = document(vec![closed_square(0.0)]);
        let base = Profile::expand(&TraceConfig::default()).unwrap();
        let mut other = TraceConfig::default();
        other.geometry.curve_tolerance_px = Some(crate::config::Finite::new(2.5, "t").unwrap());
        let other = Profile::expand(&other).unwrap();

        let a = digest_document(&doc, &base, &AlgorithmVersions::default(), &[], &[]).unwrap();
        let b = digest_document(&doc, &other, &AlgorithmVersions::default(), &[], &[]).unwrap();
        assert_ne!(a, b);
    }

    /// F.3: `-0` canonicalises to `0`.
    #[test]
    fn negative_zero_canonicalises() {
        let a = document(vec![
            CurveChain::polyline(&[Point::new(0.0, 0.0), Point::new(1.0, 1.0)]).unwrap(),
        ]);
        let b = document(vec![
            CurveChain::polyline(&[Point::new(-0.0, -0.0), Point::new(1.0, 1.0)]).unwrap(),
        ]);
        assert_eq!(digest(&a), digest(&b));
    }

    /// F.3: NaN and infinity are rejected, not encoded.
    #[test]
    fn a_non_finite_coordinate_is_rejected() {
        let doc = document(vec![
            CurveChain::polyline(&[Point::new(f64::NAN, 0.0), Point::new(1.0, 1.0)]).unwrap(),
        ]);
        let config = Profile::expand(&TraceConfig::default()).unwrap();
        assert!(digest_document(&doc, &config, &AlgorithmVersions::default(), &[], &[]).is_err());
    }

    /// Tagged tokens exist so two different structures cannot produce the same
    /// byte stream. One face with two boundaries is not two faces with one.
    #[test]
    fn structure_is_distinguishable_from_content() {
        let two_faces = document(vec![closed_square(0.0), closed_square(4.0)]);
        let mut one_face = document(vec![closed_square(0.0)]);
        if let crate::ir::Element::FilledFace(f) = &mut one_face.layers[0].elements[0] {
            f.boundaries.push(closed_square(4.0));
        }
        assert_ne!(digest(&two_faces), digest(&one_face));
    }
}
