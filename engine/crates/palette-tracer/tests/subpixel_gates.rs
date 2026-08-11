//! §31.2's synthetic subpixel geometry gates, measured (PTE-AA-001..009).
//!
//! These gates were unmeasured for as long as §10 did not exist: there was no
//! subpixel evidence to grade, so grading it would have been theatre. Now
//! there is, and this file is the measurement.
//!
//! # The fixture, and why it is regenerated here
//!
//! `curves/circle-subpixel-0` and `-1` in the §25.2 corpus are antialiased
//! circles at exactly known centres and radii. PTE-TEST-004 commits the
//! *generator* and not the rasters, so this file reproduces the same two
//! images from the same analytic description rather than reading files that
//! are not in the repository. The reproduction is deliberately faithful to
//! `tools/make_fixtures.py`, including its 16x16 box supersampling and its
//! compositing on transfer-encoded bytes -- the latter being what most 2D
//! rasterisers do, and what §10.2's transfer estimation exists to handle.
//!
//! # Coordinates
//!
//! The generator's pixel `(px, py)` is sampled over `[px − ½, px + ½]`, so its
//! centre is at `(px, py)`. The engine's image space puts the same pixel's cell
//! at `[px, px+1]`, so its centre is at `(px + ½, py + ½)`. Every truth value
//! below is therefore shifted by half a pixel on both axes. Getting this wrong
//! would move the measured centre by `0.71 px` and fail every gate, so it is
//! stated once here and applied once, in [`truth_centre`].

// This is a measurement harness, not library code. `unwrap` on a pivot search
// over a fixed three-row system cannot fail, and indexing a 3x4 array by
// column is clearer than iterator gymnastics.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::needless_range_loop)]

use palette_tracer::{Engine, TraceConfig};
use palette_tracer_core::config::Profile;
use palette_tracer_core::control::NoControl;
use palette_tracer_core::image::ImageView;
use palette_tracer_core::ir::{Element, PathSegment, Point};

const WIDTH: u32 = 100;
const HEIGHT: u32 = 80;
const RADIUS: f64 = 28.0;
const SUPERSAMPLE: u32 = 16;

/// `tools/make_fixtures.py`'s `INK` and `PAPER`.
const INK: [f64; 3] = [32.0, 38.0, 52.0];
const PAPER: [f64; 3] = [247.0, 244.0, 236.0];

/// The generator's analytic centre, in the engine's image coordinates.
fn truth_centre(cx: f64, cy: f64) -> (f64, f64) {
    (cx + 0.5, cy + 0.5)
}

/// The fixture raster: an antialiased disc, composited exactly as
/// `tools/make_fixtures.py` composites it.
fn circle_raster(cx: f64, cy: f64) -> Vec<u8> {
    let mut data = Vec::with_capacity((WIDTH * HEIGHT * 4) as usize);
    let step = 1.0 / f64::from(SUPERSAMPLE);
    for py in 0..HEIGHT {
        for px in 0..WIDTH {
            let mut hits = 0u32;
            for j in 0..SUPERSAMPLE {
                for i in 0..SUPERSAMPLE {
                    let sx = f64::from(px) - 0.5 + (f64::from(i) + 0.5) * step;
                    let sy = f64::from(py) - 0.5 + (f64::from(j) + 0.5) * step;
                    if (sx - cx).hypot(sy - cy) < RADIUS {
                        hits += 1;
                    }
                }
            }
            let t = f64::from(hits) / f64::from(SUPERSAMPLE * SUPERSAMPLE);
            // The generator blends the encoded bytes, not linear light.
            for k in 0..3 {
                let v = t.mul_add(INK[k] - PAPER[k], PAPER[k]);
                data.push(v.round().clamp(0.0, 255.0) as u8);
            }
            data.push(255);
        }
    }
    data
}

/// Every boundary sample of the traced document, flattened.
///
/// Cubic segments are sampled rather than reduced to their endpoints: §31.2
/// measures the *boundary*, and a cubic that bulges away from the circle
/// between its endpoints is exactly the error the gate is looking for.
fn boundary_samples(document: &palette_tracer_core::ir::VectorDocument) -> Vec<Point> {
    fn cubic(p0: Point, c1: Point, c2: Point, p3: Point, out: &mut Vec<Point>) {
        for i in 1..=8 {
            let t = f64::from(i) / 8.0;
            let u = 1.0 - t;
            let (a, b, c, d) = (u * u * u, 3.0 * u * u * t, 3.0 * u * t * t, t * t * t);
            out.push(Point::new(
                a * p0.x + b * c1.x + c * c2.x + d * p3.x,
                a * p0.y + b * c1.y + c * c2.y + d * p3.y,
            ));
        }
    }

    let mut out = Vec::new();
    for layer in &document.layers {
        for element in &layer.elements {
            let Element::FilledFace(face) = element else {
                continue;
            };
            for chain in &face.boundaries {
                let mut cursor = chain.start;
                out.push(cursor);
                for segment in &chain.segments {
                    match segment {
                        PathSegment::Line { to } => {
                            out.push(*to);
                            cursor = *to;
                        }
                        PathSegment::Cubic { c1, c2, to } => {
                            cubic(cursor, *c1, *c2, *to, &mut out);
                            cursor = *to;
                        }
                        _ => {}
                    }
                }
            }
        }
    }
    out
}

/// Samples on the disc's own boundary, discarding the image-border rectangle
/// that the background face contributes.
fn disc_samples(
    document: &palette_tracer_core::ir::VectorDocument,
    cx: f64,
    cy: f64,
) -> Vec<Point> {
    boundary_samples(document)
        .into_iter()
        .filter(|p| {
            // The domain border is the only other geometry present, and it is
            // more than a radius away from the circle everywhere.
            (p.x - cx).hypot(p.y - cy) < RADIUS * 1.5
        })
        .collect()
}

/// Signed distance of each sample from the true circle, in pixels.
fn normal_errors(samples: &[Point], cx: f64, cy: f64) -> Vec<f64> {
    samples
        .iter()
        .map(|p| ((p.x - cx).hypot(p.y - cy) - RADIUS).abs())
        .collect()
}

fn quantile(sorted: &[f64], q: f64) -> f64 {
    if sorted.is_empty() {
        return f64::NAN;
    }
    let index = ((sorted.len() - 1) as f64 * q).round() as usize;
    sorted[index]
}

/// Algebraic circle fit: minimise `Σ (x² + y² + Dx + Ey + F)²`, which is linear
/// in `(D, E, F)` and therefore has a closed-form answer with no iteration.
///
/// Kasa's formulation. It is biased towards small radii when the samples cover
/// a short arc; these cover the whole circle, where the bias is negligible and
/// the closed form is worth more than the accuracy a Levenberg-Marquardt fit
/// would add, because it makes the measurement itself deterministic.
fn fit_circle(samples: &[Point]) -> (f64, f64, f64) {
    let n = samples.len() as f64;
    let (mut sx, mut sy, mut sxx, mut syy, mut sxy, mut sz, mut sxz, mut syz) =
        (0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0);
    for p in samples {
        let z = p.x * p.x + p.y * p.y;
        sx += p.x;
        sy += p.y;
        sxx += p.x * p.x;
        syy += p.y * p.y;
        sxy += p.x * p.y;
        sz += z;
        sxz += p.x * z;
        syz += p.y * z;
    }
    // Normal equations for (D, E, F).
    let a = [[sxx, sxy, sx], [sxy, syy, sy], [sx, sy, n]];
    let b = [-sxz, -syz, -sz];

    // Gaussian elimination on a 3x3; the system is well conditioned for a full
    // circle's worth of samples.
    let mut m = [
        [a[0][0], a[0][1], a[0][2], b[0]],
        [a[1][0], a[1][1], a[1][2], b[1]],
        [a[2][0], a[2][1], a[2][2], b[2]],
    ];
    for col in 0..3 {
        let pivot = (col..3)
            .max_by(|&i, &j| m[i][col].abs().total_cmp(&m[j][col].abs()))
            .unwrap();
        m.swap(col, pivot);
        let d = m[col][col];
        for k in col..4 {
            m[col][k] /= d;
        }
        for row in 0..3 {
            if row != col {
                let f = m[row][col];
                for k in col..4 {
                    m[row][k] -= f * m[col][k];
                }
            }
        }
    }
    let (d, e, f) = (m[0][3], m[1][3], m[2][3]);
    let cx = -d / 2.0;
    let cy = -e / 2.0;
    let radius = (cx * cx + cy * cy - f).max(0.0).sqrt();
    (cx, cy, radius)
}

struct Measurement {
    median: f64,
    p95: f64,
    max: f64,
    centre_error: f64,
    radius_error_px: f64,
    radius_error_relative: f64,
    samples: usize,
}

/// The tolerance the gates are measured at.
///
/// §31.2's boundary-normal gates and `geometry.curveTolerancePx` are budgets
/// for the same quantity, and they are not independent: the fitter is
/// *permitted* to deviate from its samples by up to the tolerance, so a
/// profile configured with a tolerance above a gate cannot meet that gate no
/// matter how exact the reconstruction underneath it is.
///
/// `flat-illustration` defaults to `0.6 px`, six times §31.2's median target.
/// Measuring the gate there would measure the fitter's licence, not the
/// engine's accuracy. `0.10 px` is §31.2's own median figure, so the fitter is
/// allowed exactly what the gate allows and the measurement is of the
/// reconstruction. `the_gates_are_a_budget_shared_with_the_fitters_tolerance`
/// records the interaction rather than leaving it implied.
const GATE_TOLERANCE_PX: f64 = 0.10;

fn measure(cx: f64, cy: f64) -> Measurement {
    measure_at(cx, cy, Some(GATE_TOLERANCE_PX))
}

fn measure_at(cx: f64, cy: f64, tolerance: Option<f64>) -> Measurement {
    let (tx, ty) = truth_centre(cx, cy);
    let data = circle_raster(cx, cy);
    let image = ImageView::rgba8(WIDTH, HEIGHT, &data).expect("the fixture is a valid image");

    let mut config = TraceConfig {
        profile: Profile::FlatIllustration,
        ..TraceConfig::default()
    };
    if let Some(t) = tolerance {
        config.geometry.curve_tolerance_px =
            Some(palette_tracer_core::config::Finite::new(t, "curve_tolerance_px").unwrap());
    }
    let output = Engine::new()
        .trace(image, &config, &NoControl)
        .expect("the fixture traces");

    let samples = disc_samples(&output.document, tx, ty);
    assert!(
        samples.len() > 16,
        "expected a boundary to measure, got {} samples",
        samples.len()
    );

    let mut errors = normal_errors(&samples, tx, ty);
    errors.sort_by(f64::total_cmp);

    let (fx, fy, radius) = fit_circle(&samples);
    Measurement {
        median: quantile(&errors, 0.5),
        p95: quantile(&errors, 0.95),
        max: *errors.last().expect("errors is non-empty"),
        centre_error: (fx - tx).hypot(fy - ty),
        radius_error_px: (radius - RADIUS).abs(),
        radius_error_relative: (radius - RADIUS).abs() / RADIUS,
        samples: samples.len(),
    }
}

/// §31.2's boundary normal error gates: median `≤ 0.10 px`, p95 `≤ 0.35 px`,
/// max `≤ 0.75 px` outside declared ambiguous junction disks.
///
/// This circle has no junctions at all -- one closed boundary, degree-two
/// everywhere -- so the max gate applies with nothing excluded, which is the
/// strictest reading available.
#[test]
fn the_boundary_normal_error_gates_are_met_on_the_analytic_circles() {
    for (cx, cy) in [(50.0, 40.0), (50.37, 40.61)] {
        let m = measure(cx, cy);
        eprintln!(
            "circle ({cx}, {cy}): {} samples, median {:.4} px, p95 {:.4} px, max {:.4} px",
            m.samples, m.median, m.p95, m.max
        );
        assert!(
            m.median <= 0.10,
            "circle ({cx}, {cy}): median boundary normal error {:.4} px exceeds 0.10",
            m.median
        );
        assert!(
            m.p95 <= 0.35,
            "circle ({cx}, {cy}): p95 boundary normal error {:.4} px exceeds 0.35",
            m.p95
        );
        assert!(
            m.max <= 0.75,
            "circle ({cx}, {cy}): max boundary normal error {:.4} px exceeds 0.75",
            m.max
        );
    }
}

/// §31.2's circle centre gate: `≤ 0.20 px`.
///
/// The subpixel-offset fixture is the one that matters. Its centre is at
/// `(50.37, 40.61)`, so a build that thresholds coverage at 50% cannot place
/// it better than the grid allows, and the gate is a real test rather than a
/// restatement of the fixture's symmetry.
#[test]
fn the_circle_centre_gate_is_met_including_at_a_subpixel_offset() {
    for (cx, cy) in [(50.0, 40.0), (50.37, 40.61)] {
        let m = measure(cx, cy);
        eprintln!("circle ({cx}, {cy}): centre error {:.4} px", m.centre_error);
        assert!(
            m.centre_error <= 0.20,
            "circle ({cx}, {cy}): centre error {:.4} px exceeds 0.20",
            m.centre_error
        );
    }
}

/// §31.2's radius gate: `≤ 1%` or `0.20 px`, whichever is larger. At radius 28
/// the larger of the two is `0.28 px`.
#[test]
fn the_circle_radius_gate_is_met_on_the_analytic_circles() {
    for (cx, cy) in [(50.0, 40.0), (50.37, 40.61)] {
        let m = measure(cx, cy);
        let allowed = (0.01 * RADIUS).max(0.20);
        eprintln!(
            "circle ({cx}, {cy}): radius error {:.4} px ({:.3}%)",
            m.radius_error_px,
            m.radius_error_relative * 100.0
        );
        assert!(
            m.radius_error_px <= allowed,
            "circle ({cx}, {cy}): radius error {:.4} px exceeds {allowed:.4}",
            m.radius_error_px
        );
    }
}

/// The comparison that says whether §10 earned its place: the same measurement
/// against what the pixel grid alone can do.
///
/// A boundary constrained to pixel-cell interfaces cannot be closer to a circle
/// of radius 28 than the grid's own quantisation, and the median of that is
/// about a quarter pixel. If reconstruction did not beat it, §10 would be
/// complexity without benefit, and this test would say so.
#[test]
fn reconstruction_beats_what_the_grid_alone_can_express() {
    let (cx, cy) = (50.37, 40.61);
    let (tx, ty) = truth_centre(cx, cy);
    let m = measure(cx, cy);

    // The grid baseline: the best a boundary on integer coordinates can do is
    // the nearest lattice point to each true boundary position.
    let mut grid_errors = Vec::new();
    for step in 0..720 {
        let angle = f64::from(step) * std::f64::consts::TAU / 720.0;
        let x = tx + RADIUS * angle.cos();
        let y = ty + RADIUS * angle.sin();
        // A cell interface runs along an integer in one axis and spans the
        // other, so the reachable set is the pair of nearest half-integer
        // crossings; the nearer of the two axes bounds the error.
        let dx = (x - x.round()).abs();
        let dy = (y - y.round()).abs();
        grid_errors.push(dx.min(dy));
    }
    grid_errors.sort_by(f64::total_cmp);
    let grid_median = quantile(&grid_errors, 0.5);

    eprintln!(
        "reconstructed median {:.4} px vs grid-reachable median {:.4} px",
        m.median, grid_median
    );
    assert!(
        m.median < grid_median,
        "reconstruction ({:.4} px) must beat the grid ({grid_median:.4} px)",
        m.median
    );
}

/// The relationship between §31.2's gates and `geometry.curveTolerancePx`,
/// measured rather than asserted.
///
/// This is not a gate. It exists so that the tolerance the other tests use is
/// a recorded decision with a number behind it, and so that a later session
/// changing a profile default can see what it costs. §31 forbids loosening a
/// gate to make a test pass; measuring the gate where it is meaningful, and
/// saying so, is a different thing.
#[test]
fn the_gates_are_a_budget_shared_with_the_fitters_tolerance() {
    let (cx, cy) = (50.37, 40.61);
    let tight = measure_at(cx, cy, Some(GATE_TOLERANCE_PX));
    let default_profile = measure_at(cx, cy, None);

    eprintln!(
        "curve tolerance {GATE_TOLERANCE_PX} px: median {:.4}, p95 {:.4}\n\
         flat-illustration default (0.6 px): median {:.4}, p95 {:.4}",
        tight.median, tight.p95, default_profile.median, default_profile.p95
    );

    // The centre and radius gates survive the looser tolerance, because
    // fitting error is signed and averages out over a closed curve. The
    // boundary-normal gates do not, because they are pointwise.
    assert!(
        default_profile.centre_error <= 0.20,
        "the centre gate holds at the default tolerance: {:.4} px",
        default_profile.centre_error
    );
    assert!(
        default_profile.radius_error_px <= (0.01 * RADIUS).max(0.20),
        "the radius gate holds at the default tolerance: {:.4} px",
        default_profile.radius_error_px
    );
    assert!(
        tight.median < default_profile.median,
        "a tighter tolerance must not make the pointwise error worse: \
         {:.4} vs {:.4}",
        tight.median,
        default_profile.median
    );
}
