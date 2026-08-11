//! End-to-end §11.7 evidence through the real extractor and SVG lowering.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use palette_tracer::{Engine, TraceConfig};
use palette_tracer_core::config::{Finite, Profile};
use palette_tracer_core::control::NoControl;
use palette_tracer_core::image::ImageView;
use palette_tracer_core::ir::{Element, PathSegment, Primitive};

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

    let coverage = palette_tracer_svg::raster::coverage(&output.document, WIDTH, HEIGHT);
    assert!(
        coverage.exposed_pixels(1.0e-9).is_empty(),
        "semantic lowering introduced an exposed seam"
    );
    assert!(
        coverage.overlapping_pixels(1.0e-9).is_empty(),
        "semantic lowering introduced an overlap"
    );
}
