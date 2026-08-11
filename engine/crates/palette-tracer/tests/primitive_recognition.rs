//! End-to-end §11.7 evidence through the real extractor and SVG lowering.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::float_cmp)]

use palette_tracer::{Engine, TraceConfig};
use palette_tracer_core::config::{Finite, Profile};
use palette_tracer_core::control::NoControl;
use palette_tracer_core::image::ImageView;
use palette_tracer_core::ir::{Element, PathSegment, Primitive};
use palette_tracer_svg::raster;

const WIDTH: u32 = 100;
const HEIGHT: u32 = 80;
const CX: f64 = 50.37;
const CY: f64 = 40.61;
const RADIUS: f64 = 28.0;
const SUPERSAMPLE: u32 = 16;

fn circle_raster() -> Vec<u8> {
    let mut data = Vec::with_capacity((WIDTH * HEIGHT * 4) as usize);
    let step = 1.0 / f64::from(SUPERSAMPLE);
    for py in 0..HEIGHT {
        for px in 0..WIDTH {
            let mut hits = 0u32;
            for sy in 0..SUPERSAMPLE {
                for sx in 0..SUPERSAMPLE {
                    let x = f64::from(px) - 0.5 + (f64::from(sx) + 0.5) * step;
                    let y = f64::from(py) - 0.5 + (f64::from(sy) + 0.5) * step;
                    if (x - CX).hypot(y - CY) < RADIUS {
                        hits += 1;
                    }
                }
            }
            let coverage = f64::from(hits) / f64::from(SUPERSAMPLE * SUPERSAMPLE);
            let paper = [247.0, 244.0, 236.0];
            let ink = [32.0, 38.0, 52.0];
            for channel in 0..3 {
                data.push(
                    coverage
                        .mul_add(ink[channel] - paper[channel], paper[channel])
                        .round() as u8,
                );
            }
            data.push(255);
        }
    }
    data
}

fn trace_circle() -> palette_tracer::TraceOutput {
    let data = circle_raster();
    let image = ImageView::rgba8(WIDTH, HEIGHT, &data).expect("valid analytic raster");
    let mut config = TraceConfig::for_profile(Profile::Logo);
    config.geometry.curve_tolerance_px = Some(Finite::new(0.6, "curve_tolerance_px").unwrap());
    Engine::new()
        .trace(image, &config, &NoControl)
        .expect("the analytic circle traces")
}

#[test]
fn the_real_extractor_emits_a_semantic_circle() {
    let output = trace_circle();
    let circle = output
        .document
        .layers
        .iter()
        .flat_map(|layer| &layer.elements)
        .find_map(|element| match element {
            Element::Primitive(Primitive::Circle { center, radius, .. }) => {
                Some((*center, *radius))
            }
            _ => None,
        })
        .unwrap_or_else(|| {
            panic!(
                "logo mode must retain the passing circle semantically: report={:?} document={:?}",
                output.report.representation, output.document.layers
            )
        });

    // Generator pixels are centred at integer coordinates; engine pixels at
    // half-integers, so analytic truth is shifted by 0.5 on each axis.
    assert!((circle.0.x - (CX + 0.5)).abs() <= 0.20);
    assert!((circle.0.y - (CY + 0.5)).abs() <= 0.20);
    assert!((circle.1 - RADIUS).abs() <= 0.20);
    assert_eq!(output.report.representation.primitives, 1);
    assert!(output.svg.contains("<circle id=\"face-"), "{}", output.svg);
    assert!(
        output
            .report
            .unimplemented
            .iter()
            .all(|item| !item.contains("PTE-GEO-010/011")),
        "implemented circle recognition must not remain globally unimplemented"
    );
}

#[test]
fn the_opaque_neighbour_reuses_the_exact_circle_as_arcs() {
    let output = trace_circle();
    let arc_count = output
        .document
        .layers
        .iter()
        .flat_map(|layer| &layer.elements)
        .filter_map(|element| match element {
            Element::FilledFace(face) => Some(&face.boundaries),
            _ => None,
        })
        .flatten()
        .flat_map(|chain| &chain.segments)
        .filter(|segment| matches!(segment, PathSegment::Arc { .. }))
        .count();
    assert_eq!(
        arc_count, 2,
        "the neighbouring face must traverse the same circle"
    );

    let coverage = raster::coverage(&output.document, WIDTH, HEIGHT);
    assert!(
        coverage.exposed_pixels(1.0e-9).is_empty(),
        "semantic lowering introduced an exposed seam"
    );
    assert!(
        coverage.overlapping_pixels(1.0e-9).is_empty(),
        "semantic lowering introduced an overlap"
    );
}

/// PTE-TOPO-001 and PTE-SVG-009, measured on the bytes rather than on the IR.
///
/// The recognised face is a `<circle>` and its neighbour a path of two arcs.
/// Those are different element types describing one boundary, so they are only
/// the same boundary if they are written from the same numbers. Rounding them
/// independently pulled them apart two ways: the circle's `cx` landed on one
/// value while the arc endpoints `cx ± r` rounded to a pair with another
/// midpoint, and -- the arcs being exact semicircles, so their chord *is* the
/// diameter -- the rounded chord could exceed twice the rounded radius, at
/// which point SVG 1.1 F.6.6.2 obliges the renderer to scale the radii up and
/// draw a circle that is not the one beside it.
///
/// The document-level seam diagnostic cannot see either: it runs before
/// serialisation, on exact `f64`. This reads the emitted text.
#[test]
fn the_written_arcs_and_the_written_circle_are_the_same_numbers() {
    // Centre and radius chosen so every coordinate has a fractional part the
    // serialiser must round; this exact circle produced a chord of 53.5
    // against a diameter of 53.4 before the snap.
    for (cx, cy, r) in [
        (50.95, 40.85, 26.7),
        (50.45, 40.35, 26.7),
        (50.37, 40.61, 28.0),
        (49.5, 40.5, 27.35),
    ] {
        let output = trace_offset_circle(cx, cy, r);
        if output.report.representation.primitives == 0 {
            continue;
        }
        let svg = &output.svg;
        let circle = written_circle(svg).expect("a recognised circle is written");
        let (start, arcs) = written_arcs(svg).expect("its neighbour retraces it");
        assert_eq!(
            arcs.len(),
            2,
            "a full circle is two semicircular arcs: {svg}"
        );

        for &(radius, _) in &arcs {
            assert_eq!(
                radius, circle.2,
                "the arc radius must be the circle's own radius: {svg}"
            );
        }
        // The written values are exact decimals; re-parsing them into binary
        // floats adds noise around 1e-15. The defect these assertions exist
        // for was 0.1 px, so an epsilon here excludes the noise and nothing
        // else. A renderer parses the same decimals and sees the same noise.
        const PARSE_NOISE: f64 = 1.0e-9;
        let far = arcs[0].1;
        let chord = (far - start).abs();
        assert!(
            chord <= 2.0 * circle.2 + PARSE_NOISE,
            "chord {chord} exceeds the diameter {}, so a renderer must resize \
             the arc away from the circle: {svg}",
            2.0 * circle.2
        );
        assert!(
            ((start + far) / 2.0 - circle.0).abs() <= PARSE_NOISE,
            "the arcs' midpoint {} must be the circle's centre {}: {svg}",
            (start + far) / 2.0,
            circle.0
        );
    }
}

fn trace_offset_circle(cx: f64, cy: f64, radius: f64) -> palette_tracer::TraceOutput {
    let mut data = Vec::with_capacity((WIDTH * HEIGHT * 4) as usize);
    let step = 1.0 / f64::from(SUPERSAMPLE);
    for py in 0..HEIGHT {
        for px in 0..WIDTH {
            let mut hits = 0u32;
            for sy in 0..SUPERSAMPLE {
                for sx in 0..SUPERSAMPLE {
                    let x = f64::from(px) - 0.5 + (f64::from(sx) + 0.5) * step;
                    let y = f64::from(py) - 0.5 + (f64::from(sy) + 0.5) * step;
                    if (x - cx).hypot(y - cy) < radius {
                        hits += 1;
                    }
                }
            }
            let coverage = f64::from(hits) / f64::from(SUPERSAMPLE * SUPERSAMPLE);
            let paper = [247.0, 244.0, 236.0];
            let ink = [32.0, 38.0, 52.0];
            for channel in 0..3 {
                data.push(
                    coverage
                        .mul_add(ink[channel] - paper[channel], paper[channel])
                        .round() as u8,
                );
            }
            data.push(255);
        }
    }
    let image = ImageView::rgba8(WIDTH, HEIGHT, &data).expect("valid analytic raster");
    let mut config = TraceConfig::for_profile(Profile::Logo);
    config.geometry.curve_tolerance_px = Some(Finite::new(0.6, "curve_tolerance_px").unwrap());
    Engine::new()
        .trace(image, &config, &NoControl)
        .expect("the analytic circle traces")
}

/// `(cx, cy, r)` as written, from the `<circle>` element.
fn written_circle(svg: &str) -> Option<(f64, f64, f64)> {
    let element = &svg[svg.find("<circle")?..];
    let attribute = |name: &str| -> Option<f64> {
        let at = element.find(&format!("{name}=\""))? + name.len() + 2;
        let rest = &element[at..];
        rest[..rest.find('"')?].parse().ok()
    };
    Some((attribute("cx")?, attribute("cy")?, attribute("r")?))
}

/// The sub-path start x and each arc's `(radius, end x)`, as written.
fn written_arcs(svg: &str) -> Option<(f64, Vec<(f64, f64)>)> {
    let data = &svg[svg.find(" d=\"")? + 4..];
    let data = &data[..data.find('"')?];
    let start_of_arcs = data.rfind('M')?;
    let sub_path = &data[start_of_arcs + 1..];
    let start: f64 = sub_path[..sub_path.find(' ')?].parse().ok()?;

    let mut arcs = Vec::new();
    for piece in sub_path.split('A').skip(1) {
        let piece = piece.trim_end_matches('Z');
        let fields: Vec<&str> = piece.split_whitespace().collect();
        if fields.len() < 7 {
            return None;
        }
        arcs.push((fields[0].parse().ok()?, fields[5].parse().ok()?));
    }
    Some((start, arcs))
}

/// The negative control the seam gate above needs to mean anything.
///
/// `flatten_primitive` tessellates a recognised circle at exactly the step
/// count its neighbour's two arcs use, so a *matching* pair reduces to one
/// polygon and the positive assertion cannot fail for flattening reasons.
/// That is the right choice for a diagnostic -- it measures geometry, not
/// approximation -- but on its own it proves only that the code ran. Moving
/// the circle off its neighbour by a fifth of a pixel must be reported.
#[test]
fn a_circle_that_disagrees_with_its_neighbour_is_detected() {
    let mut output = trace_circle();
    let mut moved = false;
    for element in output
        .document
        .layers
        .iter_mut()
        .flat_map(|layer| &mut layer.elements)
    {
        if let Element::Primitive(Primitive::Circle { radius, .. }) = element {
            *radius -= 0.2;
            moved = true;
        }
    }
    assert!(moved, "the fixture must contain a recognised circle");

    let coverage = raster::coverage(&output.document, WIDTH, HEIGHT);
    assert!(
        !coverage.exposed_pixels(1.0e-9).is_empty(),
        "a circle 0.2 px inside its neighbour's hole leaves a ring of bare \
         pixels, and the seam diagnostic has to see it"
    );
}

/// PTE-GEO-010 as a description-complexity question, not a segment count.
///
/// Counting segments made recognition non-monotonic in tolerance: a looser
/// bound let the fitter compress the circle to three cubics, and a
/// three-segment chain then failed a `< 4` test, so widening the tolerance
/// turned a semantic circle back into curves. Once a circle is recognised,
/// giving the fitter more room must never take it away.
#[test]
fn a_looser_tolerance_never_withdraws_a_recognized_circle() {
    let mut recognized_at = None;
    for tolerance in [0.3, 0.45, 0.6, 0.8, 1.0, 1.5, 2.0] {
        let data = circle_raster();
        let image = ImageView::rgba8(WIDTH, HEIGHT, &data).unwrap();
        let mut config = TraceConfig::for_profile(Profile::Logo);
        config.geometry.curve_tolerance_px =
            Some(Finite::new(tolerance, "curve_tolerance_px").unwrap());
        let output = Engine::new().trace(image, &config, &NoControl).unwrap();
        let primitives = output.report.representation.primitives;

        match recognized_at {
            None if primitives > 0 => recognized_at = Some(tolerance),
            Some(first) => assert!(
                primitives > 0,
                "recognised at {first} px and lost again at {tolerance} px"
            ),
            None => {}
        }
    }
    assert!(
        recognized_at.is_some(),
        "the analytic circle must be recognised somewhere in the sweep"
    );
}
