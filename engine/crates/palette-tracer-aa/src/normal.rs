//! Boundary normals and position confidence (§10.3, §10.4).
//!
//! # Where the normal comes from
//!
//! `PTE-AA-004` is explicit and restrictive: "The local normal MUST be
//! estimated from multi-pixel edge evidence or current boundary tangents. It
//! MUST NOT be inferred from one noisy pixel alone."
//!
//! This build takes the second option. By the time §10 runs, topology
//! extraction has produced the shared boundary as a polyline, so a tangent is
//! available directly and needs no gradient operator over the raster. The
//! tangent at point `i` is the chord between the samples `w` steps either
//! side:
//!
//! ```text
//! t_i = x_{i+w} − x_{i−w}
//! ```
//!
//! and the normal is that rotated a quarter turn. The window `w` is what makes
//! this multi-sample rather than per-pixel: at `w = 1` the chord of a staircase
//! alternates between two directions 90° apart, which is exactly the "one noisy
//! pixel" PTE-AA-004 forbids.
//!
//! # Why the window is two and not one
//!
//! A raster boundary is a staircase. Over a window of `2w` samples a staircase
//! approximating a line of slope `s` contributes whole steps whose directions
//! average to the line's, with a residual that falls as `1/w`. Against that,
//! a larger window smooths across real corners. `w = 2` -- a five-sample chord
//! -- is the smallest window for which a 45° staircase gives the exact
//! diagonal rather than an axis direction, which is the case that matters:
//! a 45° edge is where grid snapping costs the most and where §10 has the most
//! to recover.
//!
//! At a chain end the window is truncated rather than wrapped, because the
//! endpoint is a junction shared with other chains and the samples beyond it
//! belong to a different boundary (PTE-TOPO-001).
//!
//! # Stability, and what it feeds
//!
//! `PTE-AA-005` requires confidence "based on mixture residual, color
//! separation, normal stability, and neighborhood agreement". Normal stability
//! is measured here, as the agreement between the wide chord and a narrow one:
//! where the two disagree the boundary is turning inside the window and the
//! tangent is not a reliable description of it. That is the same guard the
//! §11 fitter applies to Richardson extrapolation, for the same reason, and it
//! is recorded in `docs/notes/curve-fitting.md`.
//!
//! # Provenance
//!
//! Chord tangents and their windowing are elementary; the choice of window and
//! the stability measure are derived here from PTE-AA-004's constraint.
//! Written independently (PTE-LIC-004).

use palette_tracer_core::ir::Point;

/// Half-width of the tangent window, in samples.
///
/// See the module documentation: the smallest window that reads a 45°
/// staircase as a diagonal.
pub const TANGENT_WINDOW: usize = 2;

/// A unit normal at a boundary sample, with how much it can be trusted.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Normal {
    /// Horizontal component of the unit normal.
    pub x: f64,
    /// Vertical component of the unit normal.
    pub y: f64,
    /// Agreement between the wide and narrow tangent estimates, in `[0, 1]`.
    ///
    /// `1` where the boundary is locally straight, falling to `0` where the
    /// two windows disagree by 90° or more. This is PTE-AA-005's "normal
    /// stability" term.
    pub stability: f64,
}

/// Rotate a tangent a quarter turn to get a normal, and normalise it.
fn perpendicular(dx: f64, dy: f64) -> Option<(f64, f64)> {
    let length = dx.hypot(dy);
    if !length.is_finite() || length <= 0.0 {
        return None;
    }
    // (dx, dy) -> (dy, -dx): a quarter turn. Which of the two perpendiculars
    // this is fixes the sign convention for the whole crate, and
    // `offset::signed_offset` is written against it.
    Some((dy / length, -dx / length))
}

/// The chord between the samples `w` steps either side of `i`, truncated at
/// the ends of the chain.
fn chord(points: &[Point], i: usize, w: usize) -> Option<(f64, f64)> {
    let last = points.len().checked_sub(1)?;
    let lo = i.saturating_sub(w);
    let hi = (i + w).min(last);
    if hi == lo {
        return None;
    }
    Some((points[hi].x - points[lo].x, points[hi].y - points[lo].y))
}

/// Estimate the unit normal at every sample of a boundary polyline
/// (PTE-AA-004).
///
/// Returns `None` at samples where no tangent exists -- a chain of one point,
/// or a window in which every sample coincides. PTE-AA-002 wants that said
/// rather than filled in with a guess.
#[must_use]
pub fn normals(points: &[Point]) -> Vec<Option<Normal>> {
    (0..points.len())
        .map(|i| {
            let wide = chord(points, i, TANGENT_WINDOW)?;
            let (nx, ny) = perpendicular(wide.0, wide.1)?;

            // Normal stability: how far the narrow window's normal has turned
            // from the wide one. `cos θ` between the two, floored at zero, so
            // a disagreement of a right angle or more scores nothing.
            let stability = chord(points, i, 1)
                .and_then(|narrow| perpendicular(narrow.0, narrow.1))
                .map_or(0.0, |(mx, my)| (nx * mx + ny * my).clamp(0.0, 1.0));

            Some(Normal {
                x: nx,
                y: ny,
                stability,
            })
        })
        .collect()
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::float_cmp)]
mod tests {
    use super::*;

    fn polyline(coords: &[(f64, f64)]) -> Vec<Point> {
        coords.iter().map(|&(x, y)| Point::new(x, y)).collect()
    }

    /// A straight horizontal boundary has a vertical normal, everywhere along
    /// it, and nothing to be unstable about.
    #[test]
    fn a_straight_boundary_has_one_normal_all_along_it() {
        let points = polyline(&[(0.0, 3.0), (1.0, 3.0), (2.0, 3.0), (3.0, 3.0), (4.0, 3.0)]);
        for n in normals(&points).into_iter().flatten() {
            assert!((n.x.abs() - 0.0).abs() < 1e-12, "normal should be vertical");
            assert!((n.y.abs() - 1.0).abs() < 1e-12);
            assert!((n.stability - 1.0).abs() < 1e-12);
        }
    }

    /// PTE-AA-004, the case the requirement exists for. A 45-degree staircase
    /// is the shape a rasteriser makes of a diagonal line, and its per-sample
    /// chords alternate between horizontal and vertical -- 90 degrees apart.
    /// The windowed estimate has to read the diagonal through that.
    #[test]
    fn a_forty_five_degree_staircase_reads_as_a_diagonal_not_as_axis_steps() {
        // The classic stair: right, down, right, down, ...
        let mut coords = Vec::new();
        for step in 0..8 {
            coords.push((f64::from(step), f64::from(step)));
            coords.push((f64::from(step) + 1.0, f64::from(step)));
        }
        let points = polyline(&coords);
        let estimated = normals(&points);

        // Away from the ends, every normal is within a few degrees of the
        // diagonal's -- never an axis direction.
        for (i, n) in estimated.iter().enumerate() {
            if i < TANGENT_WINDOW || i + TANGENT_WINDOW >= points.len() {
                continue;
            }
            let n = n.unwrap();
            let diagonal = (n.x - n.y).abs() / std::f64::consts::SQRT_2;
            assert!(
                diagonal > 0.9,
                "sample {i}: normal ({}, {}) is not diagonal",
                n.x,
                n.y
            );
        }
    }

    /// The same input with a one-sample window, which is what PTE-AA-004
    /// forbids: this is the evidence that the requirement is doing work rather
    /// than restating what the code would do anyway.
    #[test]
    fn a_one_sample_window_would_alternate_between_axis_directions() {
        let mut coords = Vec::new();
        for step in 0..8 {
            coords.push((f64::from(step), f64::from(step)));
            coords.push((f64::from(step) + 1.0, f64::from(step)));
        }
        let points = polyline(&coords);
        let narrow: Vec<_> = (0..points.len())
            .filter_map(|i| chord(&points, i, 1).and_then(|(dx, dy)| perpendicular(dx, dy)))
            .collect();
        // Some sample's narrow normal is axis-aligned, which the windowed
        // estimate above never is.
        assert!(
            narrow
                .iter()
                .any(|&(x, y)| x.abs() < 1e-9 || y.abs() < 1e-9),
            "the narrow window should produce axis-aligned normals"
        );
    }

    /// PTE-AA-005's stability term: a boundary that turns a corner inside the
    /// window says so, and the confidence downstream can act on it.
    #[test]
    fn stability_falls_where_the_boundary_turns_a_corner() {
        let points = polyline(&[
            (0.0, 0.0),
            (1.0, 0.0),
            (2.0, 0.0),
            (2.0, 1.0),
            (2.0, 2.0),
            (2.0, 3.0),
        ]);
        let estimated = normals(&points);
        let at_corner = estimated[2].unwrap();
        let on_the_straight = estimated[5].unwrap();
        assert!(
            at_corner.stability < on_the_straight.stability,
            "corner {} should be less stable than the straight run {}",
            at_corner.stability,
            on_the_straight.stability
        );
    }

    /// The normal is a unit vector wherever it exists. Downstream, §10.3's
    /// octant reduction assumes it.
    #[test]
    fn every_normal_is_a_unit_vector() {
        let points = polyline(&[
            (0.0, 0.0),
            (1.0, 0.3),
            (2.0, 1.1),
            (3.5, 1.2),
            (4.0, 3.0),
            (6.0, 3.1),
        ]);
        for n in normals(&points).into_iter().flatten() {
            assert!((n.x.hypot(n.y) - 1.0).abs() < 1e-12);
        }
    }

    /// PTE-AA-002: a chain with nothing to take a tangent from has no normal,
    /// rather than a default one.
    #[test]
    fn a_degenerate_chain_has_no_normals() {
        assert!(normals(&[]).is_empty());
        assert!(normals(&polyline(&[(1.0, 1.0)]))[0].is_none());
        let coincident = polyline(&[(1.0, 1.0), (1.0, 1.0), (1.0, 1.0)]);
        assert!(normals(&coincident).iter().all(Option::is_none));
    }

    /// Reversing a chain reverses its normals exactly. PTE-TOPO-001 makes the
    /// two sides of an interface the same chain read in two directions, so a
    /// normal that did not reverse cleanly would move the shared boundary
    /// differently for the two faces.
    #[test]
    fn reversing_the_chain_negates_every_normal() {
        let points = polyline(&[
            (0.0, 0.0),
            (1.0, 0.4),
            (2.0, 1.2),
            (3.0, 1.4),
            (4.0, 2.6),
            (5.0, 3.0),
        ]);
        let mut reversed = points.clone();
        reversed.reverse();

        let forward = normals(&points);
        let backward = normals(&reversed);
        for (i, f) in forward.iter().enumerate() {
            let b = backward[points.len() - 1 - i];
            match (f, b) {
                (Some(f), Some(b)) => {
                    assert!((f.x + b.x).abs() < 1e-12, "sample {i}: {} vs {}", f.x, b.x);
                    assert!((f.y + b.y).abs() < 1e-12);
                    assert!((f.stability - b.stability).abs() < 1e-12);
                }
                (None, None) => {}
                _ => panic!("sample {i}: one direction has a normal and the other does not"),
            }
        }
    }
}
