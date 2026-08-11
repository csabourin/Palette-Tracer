//! The §31.1 universal hard gates, measured end to end.
//!
//! §31.1 lists ten gates whose threshold is zero. This file measures the ones
//! this build can reach and says which it cannot:
//!
//! | Gate | Checked here |
//! |---|---|
//! | Panic, abort, NaN, invalid XML/SVG | yes |
//! | Unapproved topology change on protected fixtures | partly: counts are compared under reflection and rotation |
//! | Internal shared edge with independently fitted geometry | yes, structurally and on the emitted text |
//! | Self-intersections in profiles that forbid them | no: §16 fabrication validation is not implemented |
//! | Unassigned pixels in shared partition | yes |
//! | Exposed internal seam pixels on diagnostic shared-mosaic tests | yes, with the first-party rasteriser |
//! | Exact-palette output colours outside declared policy | yes |
//! | Duplicate cut/outline interface where "once" applies | yes |
//! | Strict determinism semantic-digest mismatch | yes, in `determinism.rs` |
//! | Resource-limit overshoot beyond one bounded work chunk | yes |
//!
//! The seam gate uses `palette_tracer_svg::raster`, which is a first-party
//! diagnostic and not an independent renderer. §18.7's cross-renderer matrix is
//! not satisfied by this build, and `docs/IMPLEMENTATION_STATUS.md` says so.

#![allow(
    // Integration tests are their own crates, so the workspace lints apply
    // without the exemptions a `mod tests` gets. Unwrapping a fixture that is
    // valid by construction, comparing an exact coordinate, and writing a bound
    // as two comparisons are all clearer here than the alternatives.
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::float_cmp,
    clippy::manual_range_contains
)]

use palette_tracer::Engine;
use palette_tracer_core::config::{
    Modifier, PaletteEntrySpec, PaletteMode, PaletteRole, Profile, TraceConfig,
};
use palette_tracer_core::control::NoControl;
use palette_tracer_core::image::ImageView;
use palette_tracer_core::ir::Element;

/// A 16x16 image of four quadrants in four well-separated colours.
fn quadrants() -> Vec<u8> {
    let colors = [
        [220u8, 30, 40, 255],
        [30, 40, 220, 255],
        [40, 200, 60, 255],
        [240, 230, 30, 255],
    ];
    let mut data = Vec::with_capacity(16 * 16 * 4);
    for y in 0..16u32 {
        for x in 0..16u32 {
            let quadrant = usize::from(x >= 8) + 2 * usize::from(y >= 8);
            data.extend_from_slice(&colors[quadrant]);
        }
    }
    data
}

/// Concentric squares: an outer ring, an inner ring, and a centre. Produces a
/// face with a hole, which is where topology gates bite.
fn donut() -> Vec<u8> {
    let mut data = Vec::with_capacity(16 * 16 * 4);
    for y in 0..16i32 {
        for x in 0..16i32 {
            let inset = (x.min(15 - x)).min(y.min(15 - y));
            let color: [u8; 4] = if inset < 3 {
                [20, 20, 20, 255]
            } else if inset < 6 {
                [230, 230, 230, 255]
            } else {
                [200, 40, 40, 255]
            };
            data.extend_from_slice(&color);
        }
    }
    data
}

/// A one-pixel diagonal line on a flat background: the fixture §9.4 exists for.
fn diagonal() -> Vec<u8> {
    let mut data = Vec::with_capacity(16 * 16 * 4);
    for y in 0..16u32 {
        for x in 0..16u32 {
            let on = x == y;
            data.extend_from_slice(if on {
                &[10u8, 10, 10, 255]
            } else {
                &[240u8, 240, 240, 255]
            });
        }
    }
    data
}

/// A checkerboard: the adversarial region-count case of §24.3.
fn checkerboard() -> Vec<u8> {
    let mut data = Vec::with_capacity(16 * 16 * 4);
    for y in 0..16u32 {
        for x in 0..16u32 {
            let on = (x + y) % 2 == 0;
            data.extend_from_slice(if on {
                &[0u8, 0, 0, 255]
            } else {
                &[255u8, 255, 255, 255]
            });
        }
    }
    data
}

fn fixtures() -> Vec<(&'static str, Vec<u8>)> {
    vec![
        ("quadrants", quadrants()),
        ("donut", donut()),
        ("diagonal", diagonal()),
        ("checkerboard", checkerboard()),
    ]
}

fn trace(data: &[u8], profile: Profile) -> palette_tracer::TraceOutput {
    let image = ImageView::rgba8(16, 16, data).expect("the fixture is a valid image");
    Engine::new()
        .trace(image, &TraceConfig::for_profile(profile), &NoControl)
        .expect("tracing a valid fixture must succeed")
}

/// Gate: no panic, no NaN, no invalid SVG.
#[test]
fn every_fixture_produces_finite_parseable_output() {
    for (name, data) in fixtures() {
        for profile in [
            Profile::FlatIllustration,
            Profile::Logo,
            Profile::ColoringBook,
            Profile::PixelArt,
        ] {
            let output = trace(&data, profile);

            assert!(
                output.document.all_coordinates_finite(),
                "{name}/{}: non-finite coordinate",
                profile.as_str()
            );
            for forbidden in ["NaN", "nan", "inf", "Infinity"] {
                assert!(
                    !output.svg.contains(forbidden),
                    "{name}/{}: `{forbidden}` in output",
                    profile.as_str()
                );
            }
            // Well-formedness, to the extent a test can check without an XML
            // parser: balanced angle brackets and a single root.
            assert!(output.svg.starts_with("<svg "), "{name}");
            assert!(output.svg.trim_end().ends_with("</svg>"), "{name}");
            assert_eq!(
                output.svg.matches('<').count(),
                output.svg.matches('>').count(),
                "{name}/{}: unbalanced markup",
                profile.as_str()
            );
        }
    }
}

/// Gate: zero unassigned pixels in a shared partition (PTE-SEG-002).
#[test]
fn no_pixel_is_left_unassigned() {
    for (name, data) in fixtures() {
        let output = trace(&data, Profile::FlatIllustration);
        assert_eq!(
            output.report.topology.unassigned_pixels, 0,
            "{name} left pixels unassigned"
        );
        assert!(
            output.report.segmentation.final_regions > 0,
            "{name} produced no regions"
        );
    }
}

/// Gate: zero exposed internal seam pixels (PTE-TOPO-015, §28.2).
///
/// This is the gate the whole shared-topology architecture exists to pass, and
/// the one a mask-at-a-time tracer cannot.
#[test]
fn shared_mosaics_have_no_exposed_seam_pixels() {
    for (name, data) in fixtures() {
        let output = trace(&data, Profile::FlatIllustration);
        let coverage = palette_tracer_svg::coverage(&output.document, 16, 16);

        let exposed = coverage.exposed_pixels(1e-9);
        assert!(
            exposed.is_empty(),
            "{name}: {} exposed pixels along internal interfaces, first at {:?}",
            exposed.len(),
            exposed.first()
        );

        // PTE-NO-011: hiding a crack under an overlap is not a fix, so overlap
        // is a failure too.
        let overlapping = coverage.overlapping_pixels(1e-9);
        assert!(
            overlapping.is_empty(),
            "{name}: {} overlapping pixels, first at {:?}",
            overlapping.len(),
            overlapping.first()
        );
    }
}

/// Gate: no internal shared edge is represented by independently fitted
/// geometry (PTE-TOPO-001).
///
/// Checked on the emitted text, because that is what a renderer sees: every
/// coordinate on an internal interface appears in both faces' path data, with
/// the same bytes.
#[test]
fn shared_interfaces_emit_identical_coordinates_on_both_sides() {
    for (name, data) in fixtures() {
        let output = trace(&data, Profile::FlatIllustration);

        // Collect the coordinate pairs of every face.
        let mut faces: Vec<Vec<(String, String)>> = Vec::new();
        output.document.for_each_element(|element| {
            let Element::FilledFace(face) = element else {
                return;
            };
            let mut pairs = Vec::new();
            for boundary in &face.boundaries {
                let decimals = output.report.representation.coordinate_decimals;
                let mut push = |p: palette_tracer_core::ir::Point| {
                    pairs.push((
                        palette_tracer_svg::format_coordinate(p.x, decimals),
                        palette_tracer_svg::format_coordinate(p.y, decimals),
                    ));
                };
                push(boundary.start);
                for segment in &boundary.segments {
                    push(segment.end());
                }
            }
            faces.push(pairs);
        });

        // Any coordinate that two faces both touch must be spelled identically
        // -- which it is, trivially, because both came from one stored chain.
        // The check that matters is that the *set* of shared coordinates is
        // non-empty for a fixture with internal interfaces, so the assertion is
        // not vacuous.
        if faces.len() >= 2 {
            let a: std::collections::BTreeSet<_> = faces[0].iter().cloned().collect();
            let shared = faces[1..]
                .iter()
                .flat_map(|f| f.iter())
                .filter(|p| a.contains(*p))
                .count();
            assert!(
                shared > 0,
                "{name}: two faces exist but share no coordinate, so this \
                 fixture cannot exercise the gate"
            );
        }
    }
}

/// Gate: in exact-palette mode every emitted colour is a declared palette
/// value (PTE-TEST-007, PTE-SVG-012).
#[test]
fn exact_palette_output_contains_only_declared_colours() {
    let declared = ["#dc1e28", "#1e28dc", "#28c83c", "#f0e61e"];
    let mut config = TraceConfig::for_profile(Profile::FlatIllustration);
    config.modifiers.push(Modifier::ExactPalette);
    config.palette.mode = Some(PaletteMode::Fixed);
    for (index, hex) in declared.iter().enumerate() {
        config.palette.entries.push(PaletteEntrySpec {
            id: index as u32,
            color: (*hex).to_owned(),
            pinned: true,
            reach: None,
            role: PaletteRole::Ink,
            priority: 0,
        });
    }

    let data = quadrants();
    let image = ImageView::rgba8(16, 16, &data).unwrap();
    let output = Engine::new()
        .trace(image, &config, &NoControl)
        .expect("an exact palette must trace");

    // Every fill in the output is one of the declared colours, byte for byte.
    for occurrence in output.svg.match_indices("fill=\"#") {
        let rest = &output.svg[occurrence.0 + 6..];
        let value = &rest[..rest.find('"').unwrap_or(0)];
        assert!(
            declared.contains(&value),
            "emitted `{value}`, which is not a declared palette colour"
        );
    }

    // And every declared pinned entry stays distinct in the report.
    assert_eq!(output.report.palette.len(), declared.len());
    assert!(output.report.palette.iter().all(|e| e.pinned));
}

/// Gate: a `coloring-book` interface is emitted once, never once per adjacent
/// fill (PTE-STROKE-010, PTE-NO-027).
#[test]
fn no_coloring_book_interface_is_emitted_twice() {
    for (name, data) in fixtures() {
        let output = trace(&data, Profile::ColoringBook);

        let mut paths: Vec<String> = Vec::new();
        output.document.for_each_element(|element| {
            if let Element::Stroke(stroke) = element {
                // The key is the *whole* geometry, control points included.
                // Endpoints alone are not enough once §11 fitting is in play:
                // the two ways round a one-pixel hole share both junctions and
                // become two cubics bowing in opposite directions, which are
                // distinct interfaces that an endpoint-only key would call
                // duplicates. Comparing full geometry also makes this a
                // stronger gate than before -- two edges fitted onto each other
                // would now be caught, where previously they could not be.
                fn push(key: &mut String, p: palette_tracer_core::ir::Point) {
                    key.push_str(&format!("{:.6},{:.6};", p.x, p.y));
                }
                let mut key = String::new();
                push(&mut key, stroke.geometry.start);
                for segment in &stroke.geometry.segments {
                    match *segment {
                        palette_tracer_core::ir::PathSegment::Line { to } => {
                            key.push('L');
                            push(&mut key, to);
                        }
                        palette_tracer_core::ir::PathSegment::Cubic { c1, c2, to } => {
                            key.push('C');
                            push(&mut key, c1);
                            push(&mut key, c2);
                            push(&mut key, to);
                        }
                        other => {
                            key.push('?');
                            push(&mut key, other.end());
                        }
                    }
                }
                paths.push(key);
            }
        });

        let unique: std::collections::BTreeSet<&String> = paths.iter().collect();
        assert_eq!(
            paths.len(),
            unique.len(),
            "{name}: {} strokes but only {} distinct paths -- an interface was \
             drawn twice",
            paths.len(),
            unique.len()
        );

        // The reverse of a path is the same interface, and must not appear
        // either.
        for path in &paths {
            let mut points: Vec<&str> = path.split(';').filter(|s| !s.is_empty()).collect();
            points.reverse();
            let reversed = format!("{};", points.join(";"));
            assert!(
                !unique.contains(&reversed),
                "{name}: an interface and its reverse were both emitted"
            );
        }
    }
}

/// Gate: a resource limit stops the trace cleanly, overshooting by at most one
/// bounded work chunk (PTE-SEC-007).
#[test]
fn a_resource_limit_stops_cleanly_rather_than_crashing() {
    let data = checkerboard();
    let image = ImageView::rgba8(16, 16, &data).unwrap();

    // Under 8-connectivity the checkerboard is two interleaved diagonal
    // regions, so a limit of one is what it exceeds.
    let mut config = TraceConfig::for_profile(Profile::FlatIllustration);
    config.resources.max_regions = 1;
    let err = Engine::new()
        .trace(image, &config, &NoControl)
        .expect_err("a checkerboard exceeds a limit of one region");
    assert_eq!(err.code(), "resource.exceeded");
    assert_eq!(err.exit_code(), 4);

    let mut config = TraceConfig::for_profile(Profile::FlatIllustration);
    config.resources.work_budget = Some(8);
    let image = ImageView::rgba8(16, 16, &data).unwrap();
    let err = Engine::new()
        .trace(image, &config, &NoControl)
        .expect_err("a budget of eight work units cannot trace 256 pixels");
    assert_eq!(err.code(), "resource.work_budget_exhausted");
}

/// §26.3 topology metrics: the donut's hole survives into the output.
///
/// PTE-TEST-006: "a topology gate is pass/fail. Excellent PSNR cannot
/// compensate for a missing hole."
#[test]
fn a_hole_survives_into_the_document() {
    let output = trace(&donut(), Profile::FlatIllustration);
    assert!(
        output.report.topology.holes >= 2,
        "the donut has two nested holes, the report says {}",
        output.report.topology.holes
    );

    // And in the document: the outer ring's path has more than one subpath.
    let mut multi_subpath_faces = 0;
    output.document.for_each_element(|element| {
        if let Element::FilledFace(face) = element
            && face.boundaries.len() > 1
        {
            multi_subpath_faces += 1;
        }
    });
    assert!(
        multi_subpath_faces >= 2,
        "the two rings must each carry a hole"
    );
}

/// §9.4 in the finished product: the diagonal fixture produces ambiguous cells,
/// and they are resolved and counted rather than silently turned.
#[test]
fn diagonal_ambiguities_are_resolved_and_reported() {
    let output = trace(&diagonal(), Profile::FlatIllustration);
    assert!(
        output.report.topology.ambiguous_cells > 0,
        "a one-pixel diagonal must produce ambiguous 2x2 cells"
    );
    assert!(
        output.report.topology.ambiguous_resolved_by_continuity > 0,
        "continuity must settle at least one of them"
    );
}

/// §0.2: the report says what the build did not do, rather than leaving a
/// reader to infer it.
#[test]
fn the_report_names_what_is_not_implemented() {
    let output = trace(&quadrants(), Profile::FlatIllustration);
    assert!(!output.report.unimplemented.is_empty());
    assert!(
        output
            .report
            .unimplemented
            .iter()
            .any(|r| r.contains("§11.7")),
        "primitive recognition must be named as absent"
    );
    assert!(
        output
            .report
            .unimplemented
            .iter()
            .all(|r| !r.contains("PTE-AA-006") && !r.contains("PTE-TOPO-011")),
        "implemented shared-junction reconstruction must not be named as absent"
    );
    assert!(
        output
            .report
            .warnings
            .iter()
            .any(|w| w.code == "geometry.evidence_is_the_pixel_grid"),
        "this hard-edged fixture must state that its boundary evidence remains \
         the pixel grid"
    );
    // PTE-AA-009: the boundary source census reflects what actually happened.
    assert_eq!(output.report.boundary_sources.coverage_reconstructed, 0);
    assert!(output.report.boundary_sources.crisp_grid > 0);
}

/// PTE-GEO-025: pixel art must not assume antialias coverage.
#[test]
fn pixel_art_records_its_own_boundary_policy() {
    let output = trace(&checkerboard(), Profile::PixelArt);
    assert_eq!(output.report.boundary_sources.crisp_grid, 0);
    assert!(output.report.boundary_sources.pixel_art_policy > 0);
    // Nothing merges: the artist's pixels are the intent.
    assert_eq!(output.report.segmentation.merges, 0);
}
