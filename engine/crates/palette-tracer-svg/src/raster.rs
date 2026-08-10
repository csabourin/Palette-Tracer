//! A diagnostic rasteriser for the shared-seam gate (§28.2).
//!
//! # What this is, and what it is not
//!
//! PTE-TOPO-015 is a hard gate: "Shared-mosaic rerendering over a contrasting
//! background MUST contain zero exposed background pixels along internal
//! interfaces." §28.2 adds that the diagnostic-background pass "is mandatory
//! for shared mosaics because tiny cracks can be hidden against the normal
//! background color".
//!
//! This module renders the *document* -- the typed IR -- with a scanline
//! coverage rasteriser, so that gate can be measured. It is **not** a
//! conforming SVG renderer and it is **not** an independent implementation:
//!
//! * it reads the IR, not the serialised text, so it cannot catch a serialiser
//!   defect that the writer's own tests miss;
//! * it implements filled paths with non-zero winding and nothing else -- no
//!   strokes, no gradients, no transforms, no clipping;
//! * being first-party, agreement between it and the engine is weaker evidence
//!   than agreement with `resvg` or a browser would be.
//!
//! §18.7's cross-renderer compatibility matrix is therefore **not** satisfied
//! by this build. `docs/IMPLEMENTATION_STATUS.md` says so in those words.
//!
//! What the module *is* good for is the one thing it is used for: a crack
//! between two faces that should share a boundary shows up as a pixel whose
//! total coverage is below one, and that is a property of the geometry, which
//! is exactly what the gate is about.
//!
//! # Resolution
//!
//! Supersampling resolves a crack of about one eighth of a pixel, and
//! sometimes finer when the crack's edge happens to fall on a sample. A crack
//! narrower than the sample spacing *may* go undetected, depending where it
//! falls -- so this diagnostic can produce false negatives.
//!
//! It cannot produce false positives, and that is what makes a *passing* gate
//! sound: when two faces share a boundary exactly, every sample inside the
//! union is claimed by exactly one of them, so the coverage sums to exactly one
//! at every sampling rate and at every subpixel offset.
//! `exact_sharing_is_exactly_full_coverage_at_any_offset` asserts that
//! property directly, and `a_quarter_pixel_crack_is_detected` fixes the
//! resolution the tests rely on.

use palette_tracer_core::ir::{CurveChain, Element, PathSegment, Point, VectorDocument};

/// Subsamples per axis inside each pixel.
///
/// Eight gives 64 samples per pixel and resolves a crack of about one eighth of
/// a pixel. See the module documentation for why a finer crack is a tolerable
/// blind spot.
pub const SUBSAMPLES: u32 = 8;

/// Per-pixel coverage measured over a document.
#[derive(Debug, Clone)]
pub struct Coverage {
    width: u32,
    height: u32,
    /// Total coverage per pixel, summed over faces. `1.0` means fully painted
    /// exactly once.
    total: Vec<f64>,
    /// How many distinct faces painted any part of each pixel.
    faces: Vec<u32>,
}

impl Coverage {
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

    /// Total coverage at `(x, y)`.
    ///
    /// # Panics
    ///
    /// If `(x, y)` is outside the raster.
    #[must_use]
    pub fn total(&self, x: u32, y: u32) -> f64 {
        self.total[(y * self.width + x) as usize]
    }

    /// How many faces touched `(x, y)`.
    ///
    /// # Panics
    ///
    /// If `(x, y)` is outside the raster.
    #[must_use]
    pub fn face_count(&self, x: u32, y: u32) -> u32 {
        self.faces[(y * self.width + x) as usize]
    }

    /// Pixels whose coverage falls short of `1.0` by more than `tolerance`.
    ///
    /// This is the seam count PTE-TOPO-015 requires to be zero. A pixel on the
    /// image border legitimately has partial coverage when the artwork does not
    /// reach the edge, so only pixels that *some* face claims are counted.
    #[must_use]
    pub fn exposed_pixels(&self, tolerance: f64) -> Vec<(u32, u32, f64)> {
        let mut out = Vec::new();
        for y in 0..self.height {
            for x in 0..self.width {
                let total = self.total(x, y);
                if self.face_count(x, y) > 0 && total < 1.0 - tolerance {
                    out.push((x, y, total));
                }
            }
        }
        out
    }

    /// Pixels where two or more faces overlap by more than `tolerance`.
    ///
    /// A shared mosaic is a partition, so overlap is as much a defect as a gap
    /// -- and hiding a crack under an overlap is what PTE-NO-011 forbids.
    #[must_use]
    pub fn overlapping_pixels(&self, tolerance: f64) -> Vec<(u32, u32, f64)> {
        let mut out = Vec::new();
        for y in 0..self.height {
            for x in 0..self.width {
                let total = self.total(x, y);
                if total > 1.0 + tolerance {
                    out.push((x, y, total));
                }
            }
        }
        out
    }
}

/// Measure per-pixel coverage of every filled face in `document`.
///
/// # Panics
///
/// If the viewport is degenerate. Callers hold a validated document, whose
/// `all_coordinates_finite` check already refuses that.
#[must_use]
pub fn coverage(document: &VectorDocument, width: u32, height: u32) -> Coverage {
    let mut total = vec![0.0f64; (width * height) as usize];
    let mut faces = vec![0u32; (width * height) as usize];

    document.for_each_element(|element| {
        let Element::FilledFace(face) = element else {
            return;
        };
        let polygons: Vec<Vec<Point>> = face.boundaries.iter().map(flatten).collect();
        accumulate(&polygons, width, height, &mut total, &mut faces);
    });

    Coverage {
        width,
        height,
        total,
        faces,
    }
}

/// Flatten a chain to a polygon.
///
/// Cubics are subdivided uniformly. Uniform rather than adaptive because this
/// is a diagnostic: a coarser flattening would *under*-report coverage and so
/// could only invent seams, never hide one, and 16 steps is far finer than the
/// quarter-pixel the sampling can resolve.
fn flatten(chain: &CurveChain) -> Vec<Point> {
    const STEPS: usize = 16;
    let mut points = vec![chain.start];
    let mut from = chain.start;

    for segment in &chain.segments {
        match *segment {
            PathSegment::Line { to } => {
                points.push(to);
                from = to;
            }
            PathSegment::Cubic { c1, c2, to } => {
                for i in 1..=STEPS {
                    let t = i as f64 / STEPS as f64;
                    let u = 1.0 - t;
                    let x = u * u * u * from.x
                        + 3.0 * u * u * t * c1.x
                        + 3.0 * u * t * t * c2.x
                        + t * t * t * to.x;
                    let y = u * u * u * from.y
                        + 3.0 * u * u * t * c1.y
                        + 3.0 * u * t * t * c2.y
                        + t * t * t * to.y;
                    points.push(Point::new(x, y));
                }
                from = to;
            }
            PathSegment::Arc { to, .. } => {
                // Arcs are not produced by this build. Treating one as a line
                // would silently distort the diagnostic, so the coarse
                // approximation is noted here rather than hidden.
                points.push(to);
                from = to;
            }
            // `PathSegment` is `#[non_exhaustive]`; an unknown segment is
            // approximated by its endpoint, which can only under-report
            // coverage and so cannot hide a seam.
            _ => {}
        }
    }
    points
}

/// Accumulate one face's coverage by supersampling with the non-zero rule.
fn accumulate(
    polygons: &[Vec<Point>],
    width: u32,
    height: u32,
    total: &mut [f64],
    faces: &mut [u32],
) {
    let step = 1.0 / f64::from(SUBSAMPLES);
    let weight = step * step;

    // Bounding box, so a face only pays for the pixels it can reach. Without
    // it the diagnostic is quadratic in faces and unusable on real artwork.
    let (mut min_x, mut min_y) = (f64::MAX, f64::MAX);
    let (mut max_x, mut max_y) = (f64::MIN, f64::MIN);
    for polygon in polygons {
        for p in polygon {
            min_x = min_x.min(p.x);
            min_y = min_y.min(p.y);
            max_x = max_x.max(p.x);
            max_y = max_y.max(p.y);
        }
    }
    if min_x > max_x {
        return;
    }
    let x0 = min_x.floor().max(0.0) as u32;
    let y0 = min_y.floor().max(0.0) as u32;
    let x1 = (max_x.ceil().max(0.0) as u32).min(width);
    let y1 = (max_y.ceil().max(0.0) as u32).min(height);

    for y in y0..y1 {
        for x in x0..x1 {
            let mut covered = 0.0;
            for sy in 0..SUBSAMPLES {
                for sx in 0..SUBSAMPLES {
                    let point = Point::new(
                        f64::from(x) + (f64::from(sx) + 0.5) * step,
                        f64::from(y) + (f64::from(sy) + 0.5) * step,
                    );
                    if winding_number(polygons, point) != 0 {
                        covered += weight;
                    }
                }
            }
            if covered > 0.0 {
                let index = (y * width + x) as usize;
                total[index] += covered;
                faces[index] += 1;
            }
        }
    }
}

/// Winding number of `point` with respect to every subpath.
///
/// The standard crossing count, which is exact for a point not exactly on an
/// edge. Sample points sit at odd sixteenths of a pixel and grid-aligned
/// geometry sits at integers and halves, so a sample never lands on an edge in
/// practice -- and if one did, the verdict would move by one subsample, which
/// cannot flip a gate that asks about complete coverage.
fn winding_number(polygons: &[Vec<Point>], point: Point) -> i32 {
    let mut winding = 0;
    for polygon in polygons {
        for pair in polygon.windows(2) {
            let (a, b) = (pair[0], pair[1]);
            if a.y <= point.y {
                if b.y > point.y && is_left(a, b, point) > 0.0 {
                    winding += 1;
                }
            } else if b.y <= point.y && is_left(a, b, point) < 0.0 {
                winding -= 1;
            }
        }
    }
    winding
}

/// Positive when `p` is left of the directed line `a`->`b`.
fn is_left(a: Point, b: Point, p: Point) -> f64 {
    (b.x - a.x) * (p.y - a.y) - (p.x - a.x) * (b.y - a.y)
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
    use palette_tracer_core::ir::topology::FaceId;
    use palette_tracer_core::ir::{
        Color, DocumentProvenance, FillRule, FilledFace, Layer, Paint, PaletteId, Rect,
    };

    fn document(faces: Vec<(u32, Vec<Vec<Point>>)>, width: u32, height: u32) -> VectorDocument {
        VectorDocument {
            viewport: Rect::image_domain(width, height),
            physical_size: None,
            palette: vec![Paint::Solid(Color::BLACK)],
            defs: Vec::new(),
            layers: vec![Layer {
                id: "fills".into(),
                label: None,
                role: "shared-mosaic-fills".into(),
                elements: faces
                    .into_iter()
                    .map(|(id, boundaries)| {
                        Element::FilledFace(FilledFace {
                            face: FaceId(id),
                            palette: Some(PaletteId(0)),
                            paint: Paint::Solid(Color::BLACK),
                            boundaries: boundaries
                                .into_iter()
                                .map(|points| CurveChain::polyline(&points).unwrap())
                                .collect(),
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

    fn rect(x0: f64, y0: f64, x1: f64, y1: f64) -> Vec<Point> {
        vec![
            Point::new(x0, y0),
            Point::new(x1, y0),
            Point::new(x1, y1),
            Point::new(x0, y1),
            Point::new(x0, y0),
        ]
    }

    #[test]
    fn a_full_square_covers_every_pixel_exactly_once() {
        let doc = document(vec![(1, vec![rect(0.0, 0.0, 4.0, 4.0)])], 4, 4);
        let c = coverage(&doc, 4, 4);
        for y in 0..4 {
            for x in 0..4 {
                assert!((c.total(x, y) - 1.0).abs() < 1e-12, "at ({x}, {y})");
                assert_eq!(c.face_count(x, y), 1);
            }
        }
        assert!(c.exposed_pixels(1e-9).is_empty());
        assert!(c.overlapping_pixels(1e-9).is_empty());
    }

    /// The gate itself: two faces sharing an exact boundary leave no exposed
    /// pixel (PTE-TOPO-015).
    #[test]
    fn two_faces_sharing_an_exact_boundary_leave_no_seam() {
        let doc = document(
            vec![
                (1, vec![rect(0.0, 0.0, 2.0, 4.0)]),
                (2, vec![rect(2.0, 0.0, 4.0, 4.0)]),
            ],
            4,
            4,
        );
        let c = coverage(&doc, 4, 4);
        assert!(
            c.exposed_pixels(1e-9).is_empty(),
            "exposed: {:?}",
            c.exposed_pixels(1e-9)
        );
        assert!(c.overlapping_pixels(1e-9).is_empty());
    }

    /// And the test has to be able to *fail*: a crack a quarter of a pixel wide
    /// is invisible against a matching background and must still be caught.
    #[test]
    fn a_quarter_pixel_crack_is_detected() {
        let doc = document(
            vec![
                (1, vec![rect(0.0, 0.0, 1.875, 4.0)]),
                (2, vec![rect(2.125, 0.0, 4.0, 4.0)]),
            ],
            4,
            4,
        );
        let c = coverage(&doc, 4, 4);
        let exposed = c.exposed_pixels(1e-6);
        assert!(
            !exposed.is_empty(),
            "a 0.25 px crack down the middle must be detected"
        );
        assert!(
            exposed.iter().all(|&(x, _, _)| x == 1 || x == 2),
            "the crack is at x = 2: {exposed:?}"
        );
    }

    /// The property the gate's soundness rests on: an exactly shared boundary
    /// gives exactly full coverage, wherever in the pixel it falls. If this
    /// held only at integer offsets, a passing gate would mean nothing for
    /// subpixel-reconstructed boundaries.
    #[test]
    fn exact_sharing_is_exactly_full_coverage_at_any_offset() {
        for offset in [0.0, 0.03, 0.125, 0.5, 0.5 / f64::from(SUBSAMPLES), 0.97] {
            let split = 2.0 + offset;
            let doc = document(
                vec![
                    (1, vec![rect(0.0, 0.0, split, 4.0)]),
                    (2, vec![rect(split, 0.0, 4.0, 4.0)]),
                ],
                4,
                4,
            );
            let c = coverage(&doc, 4, 4);
            assert!(
                c.exposed_pixels(1e-12).is_empty(),
                "offset {offset} produced a seam: {:?}",
                c.exposed_pixels(1e-12)
            );
            assert!(
                c.overlapping_pixels(1e-12).is_empty(),
                "offset {offset} produced an overlap"
            );
        }
    }

    /// PTE-NO-011: hiding a crack under an overlap is not a fix, so overlap is
    /// reported too.
    #[test]
    fn an_overlap_is_reported_rather_than_treated_as_a_fix() {
        let doc = document(
            vec![
                (1, vec![rect(0.0, 0.0, 2.5, 4.0)]),
                (2, vec![rect(1.5, 0.0, 4.0, 4.0)]),
            ],
            4,
            4,
        );
        let c = coverage(&doc, 4, 4);
        assert!(c.exposed_pixels(1e-6).is_empty(), "there is no gap");
        assert!(
            !c.overlapping_pixels(1e-6).is_empty(),
            "the overlap must be reported"
        );
    }

    /// A hole must actually be empty under non-zero winding, which is the
    /// convention the lowering relies on (PTE-SVG-005).
    #[test]
    fn a_hole_is_empty_under_nonzero_winding() {
        // Outer clockwise, hole counter-clockwise: opposite windings.
        let outer = rect(0.0, 0.0, 4.0, 4.0);
        let mut hole = rect(1.0, 1.0, 3.0, 3.0);
        hole.reverse();

        let doc = document(vec![(1, vec![outer, hole])], 4, 4);
        let c = coverage(&doc, 4, 4);

        assert!((c.total(0, 0) - 1.0).abs() < 1e-12, "the ring is painted");
        assert_eq!(c.total(1, 1), 0.0, "the hole is empty");
        assert_eq!(c.total(2, 2), 0.0, "the hole is empty");
        assert!((c.total(3, 3) - 1.0).abs() < 1e-12, "the ring is painted");
    }

    /// A pixel no face claims is not a seam. Artwork that does not reach the
    /// image edge must not be reported as cracked.
    #[test]
    fn an_unclaimed_pixel_is_not_a_seam() {
        let doc = document(vec![(1, vec![rect(0.0, 0.0, 2.0, 2.0)])], 4, 4);
        let c = coverage(&doc, 4, 4);
        assert_eq!(c.face_count(3, 3), 0);
        assert!(c.exposed_pixels(1e-9).is_empty());
    }
}
