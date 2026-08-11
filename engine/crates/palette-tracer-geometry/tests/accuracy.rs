//! Measured fitting accuracy against analytic truth (§26.2, §31.2, §31.5).
//!
//! # What these numbers do and do not claim
//!
//! §31.2's subpixel gates are written for "clean analytic antialiased
//! fixtures": they measure the whole path from antialiased pixels to geometry,
//! and §10's coverage reconstruction is the first half of it. §10 is **not
//! implemented**, so these tests do not claim those gates.
//!
//! What they measure is the second half in isolation — given exact samples on
//! an analytic shape, how far does the fitted geometry stray from that shape?
//! That is a real, falsifiable bound on the fitter's own contribution to the
//! error budget, and it is a prerequisite for the §31.2 gates ever passing:
//! error the fitter adds here cannot be recovered later.
//!
//! The distinction is stated rather than left for a reader to infer, because
//! §0.2 makes "an unlabelled number" a defect.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::float_cmp)]

use palette_tracer_core::control::NoControl;
use palette_tracer_core::ir::{CurveChain, PathSegment, Point, Topology};
use palette_tracer_geometry::curves::{Cubic, point_to_polyline};
use palette_tracer_geometry::{FitOptions, fit_topology};

fn options(tolerance: f64) -> FitOptions {
    FitOptions {
        tolerance_px: tolerance,
        corner_threshold_deg: 55.0,
        corner_hysteresis_deg: 8.0,
        allow_arcs: false,
        max_span_candidates: 48,
    }
}

/// Densely sample a fitted chain, so a distance to it can be measured.
fn densify(chain: &CurveChain) -> Vec<Point> {
    let mut out = vec![chain.start];
    let mut from = chain.start;
    for segment in &chain.segments {
        match *segment {
            PathSegment::Line { to } => {
                out.push(to);
                from = to;
            }
            PathSegment::Cubic { c1, c2, to } => {
                let cubic = Cubic::new(from, c1, c2, to);
                // Three orders finer than the tightest tolerance under test, so
                // the measurement is of the curve rather than of this sampling.
                // Not finer still: the flattening depth is capped, and asking
                // past the cap is rejected rather than approximated.
                out.extend(cubic.flatten(1e-4).expect("a fitted cubic flattens"));
                from = to;
            }
            _ => panic!("this build emits only lines and cubics"),
        }
    }
    out
}

/// One chain sampled from an analytic curve, fitted, then measured against the
/// analytic curve at points the fitter never saw.
fn measure(truth: impl Fn(f64) -> Point, samples: usize, tolerance: f64) -> (f64, usize, usize) {
    let points: Vec<Point> = (0..=samples)
        .map(|i| truth(i as f64 / samples as f64))
        .collect();
    let before = points.len() - 1;

    let mut topology = Topology::default();
    topology
        .chains
        .push(CurveChain::polyline(&points).expect("two points at least"));
    let stats = fit_topology(&mut topology, &options(tolerance), &NoControl).unwrap();
    let _ = stats;

    let fitted = densify(&topology.chains[0]);
    // Probe at parameters between the samples: a fitter that merely
    // interpolated its inputs would score perfectly on the inputs alone.
    let mut worst: f64 = 0.0;
    for i in 0..=(samples * 7) {
        let p = truth(i as f64 / (samples * 7) as f64);
        worst = worst.max(point_to_polyline(p, &fitted));
    }
    (worst, before, topology.chains[0].segments.len())
}

/// A circle is the shape a cubic exists to approximate, and the one §31.2 names
/// a gate for. Sampled exactly, it must be reproduced to a small fraction of a
/// pixel with a handful of segments.
#[test]
fn a_circle_is_reproduced_to_well_under_a_tenth_of_a_pixel() {
    let radius = 40.0;
    let centre = Point::new(50.0, 50.0);
    let (error, before, after) = measure(
        |t| {
            let a = t * std::f64::consts::TAU;
            Point::new(centre.x + radius * a.cos(), centre.y + radius * a.sin())
        },
        360,
        0.1,
    );

    assert!(
        error <= 0.05,
        "a circle should fit far inside the tolerance, got {error}"
    );
    assert!(
        after <= 8,
        "§31.5: a clean circle is a small bounded cubic representation, got {after} from {before}"
    );
}

/// An ellipse has varying curvature, so a single cubic cannot serve and the
/// dynamic program has to choose splits. The accuracy claim is unchanged.
#[test]
fn an_ellipse_is_reproduced_within_its_tolerance() {
    let (error, before, after) = measure(
        |t| {
            let a = t * std::f64::consts::TAU;
            Point::new(60.0 + 50.0 * a.cos(), 40.0 + 18.0 * a.sin())
        },
        400,
        0.15,
    );

    assert!(error <= 0.15, "got {error}");
    assert!(after <= 16, "got {after} segments from {before}");
}

/// An S-curve with an inflection: the case where a fitter that never splits
/// produces a loop or a cusp.
#[test]
fn an_s_curve_keeps_its_inflection_without_looping() {
    let (error, _, after) = measure(
        |t| {
            let x = t * 80.0;
            Point::new(x, 40.0 + 20.0 * (x / 13.0).sin())
        },
        300,
        0.2,
    );
    assert!(error <= 0.2, "got {error}");
    assert!(after >= 2, "an S-curve needs more than one cubic");
    assert!(after <= 20, "but not {after}");
}

/// §31.5's monotonicity rule, measured end to end rather than on one chain:
/// raising the tolerance must not raise the segment count, and the error must
/// stay inside whatever tolerance was asked for.
#[test]
fn loosening_the_tolerance_never_costs_more_segments_or_breaks_the_budget() {
    let mut previous = usize::MAX;
    for tolerance in [0.05, 0.1, 0.2, 0.4, 0.8] {
        let (error, _, after) = measure(
            |t| {
                let a = t * std::f64::consts::TAU;
                Point::new(70.0 + 55.0 * a.cos(), 70.0 + 55.0 * a.sin())
            },
            360,
            tolerance,
        );
        assert!(
            error <= tolerance,
            "tolerance {tolerance} was exceeded by {error}"
        );
        assert!(
            after <= previous,
            "tolerance {tolerance} used {after} segments after {previous}"
        );
        previous = after;
    }
}

/// The complexity claim of §31.5, on a rectangle whose sides are exact.
#[test]
fn a_rectangle_is_four_lines() {
    let mut points = Vec::new();
    let (w, h) = (60.0, 35.0);
    let steps = 60;
    for i in 0..steps {
        points.push(Point::new(w * f64::from(i) / f64::from(steps), 0.0));
    }
    for i in 0..steps {
        points.push(Point::new(w, h * f64::from(i) / f64::from(steps)));
    }
    for i in 0..steps {
        points.push(Point::new(w - w * f64::from(i) / f64::from(steps), h));
    }
    for i in 0..=steps {
        points.push(Point::new(0.0, h - h * f64::from(i) / f64::from(steps)));
    }

    let mut topology = Topology::default();
    topology.chains.push(CurveChain::polyline(&points).unwrap());
    let stats = fit_topology(&mut topology, &options(0.2), &NoControl).unwrap();

    assert_eq!(
        topology.chains[0].segments.len(),
        4,
        "got {:?}",
        topology.chains[0].segments
    );
    assert!(
        topology.chains[0]
            .segments
            .iter()
            .all(|s| matches!(s, PathSegment::Line { .. })),
        "a rectangle's sides are lines, not cubics"
    );
    assert!(stats.reduction() > 0.98, "got {}", stats.reduction());
}

/// PTE-GEO-009 and PTE-GEO-005 as a runtime claim: an adversarial chain — pure
/// noise, where almost no span is admissible — must still terminate quickly and
/// bounded, and must not silently emit something outside the tolerance.
#[test]
fn an_adversarial_noisy_chain_stays_bounded() {
    // A deterministic pseudo-random walk; no `rand` dependency, and the same
    // chain on every run and every target.
    let mut state = 0x2545_f491_4f6c_dd1du64;
    let mut next = || {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        (state >> 11) as f64 / (1u64 << 53) as f64
    };

    let points: Vec<Point> = (0..2000)
        .map(|i| Point::new(f64::from(i) * 0.5, next() * 20.0))
        .collect();

    let mut topology = Topology::default();
    topology.chains.push(CurveChain::polyline(&points).unwrap());

    let start = std::time::Instant::now();
    let stats = fit_topology(&mut topology, &options(0.3), &NoControl).unwrap();
    let elapsed = start.elapsed();

    assert!(
        elapsed < std::time::Duration::from_secs(10),
        "an adversarial chain took {elapsed:?}"
    );
    // Whatever it produced, it is within the tolerance or it kept the polyline.
    assert!(
        stats.max_error_px <= 0.3,
        "the tolerance was exceeded: {}",
        stats.max_error_px
    );
    assert!(
        stats.segments_after <= stats.segments_before,
        "fitting made an adversarial chain worse"
    );
}

/// Fitting must leave the graph alone. The chains change; nothing else may.
#[test]
fn fitting_changes_only_the_chains() {
    use palette_tracer_core::ir::{
        BoundaryConfidence, ChainId, Cycle, CycleKind, EdgeFlags, EdgeId, Face, FaceId, HalfEdge,
        HalfEdgeId, RegionId, SharedEdge, Vertex, VertexId,
    };

    let points: Vec<Point> = (0..=40).map(|i| Point::new(f64::from(i), 0.0)).collect();
    let mut topology = Topology::default();
    topology.chains.push(CurveChain::polyline(&points).unwrap());
    topology.vertices.push(Vertex {
        position: points[0],
        outgoing: HalfEdgeId(0),
        degree: 2,
    });
    topology.vertices.push(Vertex {
        position: points[40],
        outgoing: HalfEdgeId(1),
        degree: 2,
    });
    topology.edges.push(SharedEdge {
        id: EdgeId(0),
        left_face: FaceId(1),
        right_face: FaceId::EXTERIOR,
        geometry: ChainId(0),
        start: VertexId(0),
        end: VertexId(1),
        confidence: BoundaryConfidence::crisp(),
        flags: EdgeFlags::default(),
    });
    for (face, forward) in [(FaceId(1), true), (FaceId::EXTERIOR, false)] {
        topology.half_edges.push(HalfEdge {
            origin: VertexId(0),
            twin: HalfEdgeId(u32::from(forward)),
            next: HalfEdgeId(0),
            prev: HalfEdgeId(0),
            face,
            edge: EdgeId(0),
            forward,
        });
    }
    topology.faces.push(Face {
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

    let before = topology.clone();
    fit_topology(&mut topology, &options(0.3), &NoControl).unwrap();

    assert_eq!(topology.vertices, before.vertices);
    assert_eq!(topology.half_edges, before.half_edges);
    assert_eq!(topology.edges, before.edges);
    assert_eq!(topology.faces, before.faces);
    assert_ne!(topology.chains, before.chains, "the chain was simplified");

    // And the two traversals still describe one chain, reversed (PTE-TOPO-001).
    let a = topology.oriented_chain(HalfEdgeId(0));
    let b = topology.oriented_chain(HalfEdgeId(1));
    assert_eq!(a, b.reversed());
}

/// The statistics a report is built from must describe what happened, not what
/// was hoped for.
#[test]
fn the_statistics_add_up() {
    let mut topology = Topology::default();
    for scale in 1..=6 {
        let points: Vec<Point> = (0..=(20 * scale))
            .map(|i| {
                let a = f64::from(i) / f64::from(20 * scale) * std::f64::consts::PI;
                Point::new(30.0 * a.cos(), 30.0 * a.sin())
            })
            .collect();
        topology.chains.push(CurveChain::polyline(&points).unwrap());
    }

    let stats = fit_topology(&mut topology, &options(0.25), &NoControl).unwrap();

    assert_eq!(
        stats.chains_fitted
            + stats.polyline_fallbacks
            + stats.chains_already_minimal
            + stats.chains_trivial,
        6
    );
    assert_eq!(
        stats.segments_after,
        topology
            .chains
            .iter()
            .map(|c| c.segments.len() as u64)
            .sum::<u64>(),
        "the reported count must be the count in the topology"
    );
    assert!(stats.max_error_px <= 0.25);
    assert!(stats.p95_error_px <= stats.max_error_px);
    assert!(stats.reduction() > 0.5);
}
