//! §10's junction pass against topology it does not get to choose.
//!
//! The unit gates in `palette-tracer-aa` build their junctions by hand, which
//! is the right way to measure the solve but cannot catch a case the extractor
//! produces and the hand-built fixtures do not. This file starts from a label
//! plane, runs the real extractor, reconstructs, and then reruns the Appendix B
//! validator -- the same sequence the facade runs, and the sequence that turns
//! a shared-geometry mistake into a refused trace rather than a bad SVG.

#![allow(
    // An integration test is its own crate, so the workspace lints apply
    // without the exemptions a `mod tests` gets.
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::float_cmp
)]

use palette_tracer_color::alpha::PremulLinearRgba;
use palette_tracer_color::spaces::{EncodedSrgb, LinearRgb, Oklab};
use palette_tracer_core::control::NoControl;
use palette_tracer_core::image::ImageView;
use palette_tracer_core::ir::PaletteId;
use palette_tracer_core::labels::{LabelId, LabelPlane};
use palette_tracer_core::limits::{ResourceLimits, WorkBudget};
use palette_tracer_topology::extract::{ExtractOptions, extract};

const N: u32 = 24;
const SUPERSAMPLE: u32 = 16;
/// The corner where the two squares meet, chosen off the grid in both axes.
const JUNCTION: (f64, f64) = (11.35, 11.42);

/// Two squares that touch at exactly one corner, on a background.
///
/// `0` is the background, `1` the upper-left square and `2` the lower-right
/// one. The shared corner is a vertex of degree four with three labels around
/// it, and each square's whole boundary passes through it -- so each square is
/// one *loop* edge, whose two ends are the same junction vertex.
fn region(x: f64, y: f64) -> usize {
    let (jx, jy) = JUNCTION;
    if x > 4.0 && y > 4.0 && x < jx && y < jy {
        1
    } else if x > jx && y > jy && x < 19.0 && y < 19.0 {
        2
    } else {
        0
    }
}

fn colors() -> [LinearRgb; 3] {
    [
        LinearRgb::new(0.85, 0.84, 0.80),
        LinearRgb::new(0.05, 0.08, 0.42),
        LinearRgb::new(0.70, 0.12, 0.05),
    ]
}

fn encode(linear: LinearRgb) -> [u8; 3] {
    let e = linear.to_encoded();
    [
        (e.r * 255.0).round().clamp(0.0, 255.0) as u8,
        (e.g * 255.0).round().clamp(0.0, 255.0) as u8,
        (e.b * 255.0).round().clamp(0.0, 255.0) as u8,
    ]
}

/// A region's mean colour as the pipeline's own summaries would supply it.
fn premul(linear: LinearRgb) -> PremulLinearRgba {
    let [r, g, b] = encode(linear);
    let e = EncodedSrgb::new(
        f64::from(r) / 255.0,
        f64::from(g) / 255.0,
        f64::from(b) / 255.0,
    );
    PremulLinearRgba::from_straight(e.to_linear(), 1.0)
}

/// The partition, antialiased by box supersampling in linear light, so the
/// cells around the corner really are three-colour mixtures.
fn raster() -> Vec<u8> {
    let regions = colors();
    let mut data = Vec::with_capacity((N * N * 4) as usize);
    for py in 0..N {
        for px in 0..N {
            let mut sums = [0.0f64; 3];
            for sy in 0..SUPERSAMPLE {
                for sx in 0..SUPERSAMPLE {
                    let x = f64::from(px) + (f64::from(sx) + 0.5) / f64::from(SUPERSAMPLE);
                    let y = f64::from(py) + (f64::from(sy) + 0.5) / f64::from(SUPERSAMPLE);
                    let c = regions[region(x, y)];
                    sums[0] += c.r;
                    sums[1] += c.g;
                    sums[2] += c.b;
                }
            }
            let scale = 1.0 / f64::from(SUPERSAMPLE * SUPERSAMPLE);
            let [r, g, b] = encode(LinearRgb::new(
                sums[0] * scale,
                sums[1] * scale,
                sums[2] * scale,
            ));
            data.extend_from_slice(&[r, g, b, 255]);
        }
    }
    data
}

/// PTE-TOPO-011 and PTE-AA-008: a shared edge whose two ends are the same
/// junction gets *both* of them moved, so the chain stays closed.
///
/// Recording such an edge once, as its start, left the other end on the grid
/// corner while the vertex moved. The chain came apart, and the Appendix B
/// check `chain_starts_at_its_origin` failed the whole trace on an image that
/// is nothing more exotic than two blocks meeting corner to corner.
#[test]
fn a_loop_edge_at_a_junction_stays_closed() {
    let mut plane = LabelPlane::new(N, N, 3).unwrap();
    for py in 0..N {
        for px in 0..N {
            // Crisp labels: the pixel centre decides, as extraction expects.
            let label = region(f64::from(px) + 0.5, f64::from(py) + 0.5);
            plane.set_index((py * N + px) as usize, LabelId(label as u32));
        }
    }
    let mut extraction = extract(
        &plane,
        &vec![Oklab::ZERO; (N * N) as usize],
        &(0..3).map(|i| Some(PaletteId(i))).collect::<Vec<_>>(),
        &[false; 3],
        &ExtractOptions::default(),
        &ResourceLimits::default(),
        &WorkBudget::unbounded(),
        &NoControl,
    )
    .unwrap();

    // The fixture only means anything if it really contains the shape.
    let loops: Vec<usize> = extraction
        .topology
        .edges
        .iter()
        .enumerate()
        .filter(|(_, edge)| {
            edge.start == edge.end && extraction.topology.vertices[edge.start.index()].degree >= 3
        })
        .map(|(i, _)| i)
        .collect();
    assert_eq!(
        loops.len(),
        2,
        "two squares meeting at one corner are two loop edges at one junction"
    );
    palette_tracer_topology::validate(&extraction.topology).expect("extraction is valid");

    let data = raster();
    let image = ImageView::rgba8(N, N, &data).unwrap();
    let regions = colors();
    let face_colors = vec![
        PremulLinearRgba::TRANSPARENT,
        premul(regions[0]),
        premul(regions[1]),
        premul(regions[2]),
    ];
    let stats = palette_tracer_aa::reconstruct(
        &mut extraction.topology,
        &image,
        &face_colors,
        &palette_tracer_aa::ReconstructOptions::default(),
        &WorkBudget::unbounded(),
        &NoControl,
    )
    .unwrap();

    assert_eq!(
        stats.reconstructed_junctions, 1,
        "the corner must be solved, or this test proves nothing: {stats:?}"
    );
    for index in loops {
        let edge = &extraction.topology.edges[index];
        let chain = &extraction.topology.chains[edge.geometry.index()];
        let vertex = extraction.topology.vertices[edge.start.index()].position;
        assert_eq!(
            chain.start,
            chain.end(),
            "loop edge {index} must still close on itself"
        );
        assert_eq!(
            chain.start, vertex,
            "loop edge {index} must close on the shared vertex"
        );
    }
    palette_tracer_topology::validate(&extraction.topology)
        .expect("the topology must still be valid after §10");
}
