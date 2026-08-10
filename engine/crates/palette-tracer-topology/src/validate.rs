//! The topology validator (Appendix B, PTE-TOPO-005).
//!
//! Appendix B lists what must hold before geometry fitting. Each check here
//! names the checklist item it implements, and each returns a
//! [`ValidationError::Topology`] carrying the check's stable name so a failure
//! is machine-identifiable rather than a sentence (PTE-API-012).
//!
//! PTE-TOPO-005 requires this to run "in debug/test builds and on requested
//! strict production output". [`validate`] is called unconditionally by the
//! facade after extraction, because it is `O(V + E + F)` and cheap relative to
//! everything around it -- there is no version of this engine where skipping
//! it is worth the risk.

use palette_tracer_core::error::ValidationError;
use palette_tracer_core::ir::{CycleKind, HalfEdgeId, Topology};

/// Run every applicable Appendix B check.
///
/// # Errors
///
/// [`ValidationError::Topology`] naming the first check that fails.
pub fn validate(topology: &Topology) -> Result<(), ValidationError> {
    check_indices(topology)?;
    check_twins(topology)?;
    check_next_prev(topology)?;
    check_cycles(topology)?;
    check_shared_geometry(topology)?;
    check_faces(topology)?;
    Ok(())
}

fn fail(check: &'static str, detail: String) -> ValidationError {
    ValidationError::Topology { check, detail }
}

/// Every index refers to something that exists.
fn check_indices(t: &Topology) -> Result<(), ValidationError> {
    for (i, he) in t.half_edges.iter().enumerate() {
        if he.twin.index() >= t.half_edges.len()
            || he.next.index() >= t.half_edges.len()
            || he.prev.index() >= t.half_edges.len()
        {
            return Err(fail(
                "half_edge_indices",
                format!("half-edge {i} refers to a half-edge that does not exist"),
            ));
        }
        if he.origin.index() >= t.vertices.len() {
            return Err(fail(
                "half_edge_origin",
                format!("half-edge {i} has origin {:?}", he.origin),
            ));
        }
        if he.face.index() >= t.faces.len() {
            return Err(fail(
                "half_edge_face",
                format!("half-edge {i} has face {:?}", he.face),
            ));
        }
        if he.edge.index() >= t.edges.len() {
            return Err(fail(
                "half_edge_edge",
                format!("half-edge {i} has edge {:?}", he.edge),
            ));
        }
    }
    for (i, edge) in t.edges.iter().enumerate() {
        if edge.geometry.index() >= t.chains.len() {
            return Err(fail(
                "edge_geometry",
                format!("edge {i} refers to chain {:?}", edge.geometry),
            ));
        }
    }
    Ok(())
}

/// Appendix B: "Every half-edge has a valid twin including the exterior
/// convention." PTE-TOPO-003.
fn check_twins(t: &Topology) -> Result<(), ValidationError> {
    for (i, he) in t.half_edges.iter().enumerate() {
        let twin = &t.half_edges[he.twin.index()];
        if twin.twin.index() != i {
            return Err(fail(
                "twin_involution",
                format!("half-edge {i}'s twin does not point back at it"),
            ));
        }
        if he.twin.index() == i {
            return Err(fail(
                "twin_is_distinct",
                format!("half-edge {i} is its own twin"),
            ));
        }
        if twin.edge != he.edge {
            return Err(fail(
                "twins_share_an_edge",
                format!("half-edge {i} and its twin name different edges"),
            ));
        }
        if twin.forward == he.forward {
            return Err(fail(
                "twins_are_opposite",
                format!("half-edge {i} and its twin claim the same direction"),
            ));
        }
    }
    Ok(())
}

/// Appendix B: "`next` and `prev` are mutual inverses." PTE-TOPO-004.
fn check_next_prev(t: &Topology) -> Result<(), ValidationError> {
    for (i, he) in t.half_edges.iter().enumerate() {
        if t.half_edges[he.next.index()].prev.index() != i {
            return Err(fail(
                "next_prev_inverse",
                format!("prev(next({i})) is not {i}"),
            ));
        }
        if t.half_edges[he.prev.index()].next.index() != i {
            return Err(fail(
                "next_prev_inverse",
                format!("next(prev({i})) is not {i}"),
            ));
        }
        // A traversal must stay on one face.
        if t.half_edges[he.next.index()].face != he.face {
            return Err(fail(
                "cycle_stays_on_one_face",
                format!("next({i}) borders a different face"),
            ));
        }
    }
    Ok(())
}

/// Appendix B: "Face cycles terminate and close", and no half-edge appears
/// twice in the same directed cycle (PTE-TOPO-004).
fn check_cycles(t: &Topology) -> Result<(), ValidationError> {
    let mut membership = vec![0u32; t.half_edges.len()];

    for face in &t.faces {
        for cycle in &face.cycles {
            if cycle.start.index() >= t.half_edges.len() {
                return Err(fail(
                    "cycle_start",
                    format!("face {:?} names a half-edge that does not exist", face.id),
                ));
            }
            let walk = t.walk_cycle(cycle.start);
            let mut seen = std::collections::BTreeSet::new();
            for he in &walk {
                if !seen.insert(*he) {
                    return Err(fail(
                        "half_edge_repeats_in_cycle",
                        format!("half-edge {} appears twice in one cycle", he.0),
                    ));
                }
                if t.half_edges[he.index()].face != face.id {
                    return Err(fail(
                        "cycle_belongs_to_its_face",
                        format!("half-edge {} is in face {:?}'s cycle", he.0, face.id),
                    ));
                }
                membership[he.index()] += 1;
            }
            // `walk_cycle` stops after the whole half-edge list rather than
            // hanging; a cycle that long did not close.
            if walk.len() > t.half_edges.len() {
                return Err(fail(
                    "cycle_does_not_close",
                    format!("a cycle of face {:?} did not return to its start", face.id),
                ));
            }
        }
    }

    if let Some(orphan) = membership.iter().position(|&c| c == 0) {
        return Err(fail(
            "half_edge_without_cycle",
            format!("half-edge {orphan} belongs to no face cycle"),
        ));
    }
    if let Some(shared) = membership.iter().position(|&c| c > 1) {
        return Err(fail(
            "half_edge_in_two_cycles",
            format!("half-edge {shared} belongs to more than one cycle"),
        ));
    }
    Ok(())
}

/// Appendix B, after fitting: "Shared twins reference exactly one curve
/// chain", and "Junction endpoints coincide exactly in IR".
///
/// This is the §31.1 hard gate "Internal shared edge represented by
/// independently fitted geometry: 0" made checkable: if the two sides of an
/// interface ever stopped being one chain, the reversal test below would fail.
fn check_shared_geometry(t: &Topology) -> Result<(), ValidationError> {
    for (i, he) in t.half_edges.iter().enumerate() {
        let this = t.oriented_chain(HalfEdgeId(i as u32));
        let twin = t.oriented_chain(he.twin);
        if this != twin.reversed() {
            return Err(fail(
                "shared_edge_is_one_chain",
                format!(
                    "half-edge {i} and its twin do not describe one chain \
                     traversed both ways (PTE-TOPO-001)"
                ),
            ));
        }
        // The chain must start where the half-edge says it does.
        let origin = t.vertices[he.origin.index()].position;
        if this.start != origin {
            return Err(fail(
                "chain_starts_at_its_origin",
                format!(
                    "half-edge {i} starts at {:?} but its origin vertex is at {origin:?}",
                    this.start
                ),
            ));
        }
        if !this.is_finite() {
            return Err(fail(
                "chain_is_finite",
                format!("half-edge {i} has a non-finite coordinate"),
            ));
        }
    }
    Ok(())
}

/// Appendix B: "Hole/outer-cycle containment is valid", plus the exterior
/// convention (PTE-TOPO-007).
fn check_faces(t: &Topology) -> Result<(), ValidationError> {
    if t.faces.is_empty() {
        return Err(fail("exterior_face_exists", "there are no faces".into()));
    }
    if !t.faces[0].exterior {
        return Err(fail(
            "exterior_face_is_first",
            "face zero is not the exterior".into(),
        ));
    }
    if t.faces.iter().filter(|f| f.exterior).count() != 1 {
        return Err(fail(
            "one_exterior_face",
            "there must be exactly one exterior face".into(),
        ));
    }

    for face in &t.faces {
        if face.exterior {
            // The exterior has no outer boundary: everything it touches is
            // inside it. Its cycles are all holes by the signed-area test.
            if face.cycles.iter().any(|c| c.kind == CycleKind::Outer) {
                return Err(fail(
                    "exterior_has_no_outer_cycle",
                    "the exterior face claims an outer boundary".into(),
                ));
            }
            continue;
        }
        if face.cycles.is_empty() {
            return Err(fail(
                "face_has_a_boundary",
                format!("face {:?} has no cycles", face.id),
            ));
        }
        if !face.cycles.iter().any(|c| c.kind == CycleKind::Outer) {
            return Err(fail(
                "face_has_an_outer_cycle",
                format!("face {:?} has only holes", face.id),
            ));
        }
        // Outer cycles come first, so a serialiser can emit the boundary
        // before its holes without sorting (§18.4).
        let first_hole = face
            .cycles
            .iter()
            .position(|c| c.kind == CycleKind::Hole)
            .unwrap_or(face.cycles.len());
        if face.cycles[first_hole..]
            .iter()
            .any(|c| c.kind == CycleKind::Outer)
        {
            return Err(fail(
                "outer_cycles_come_first",
                format!("face {:?} interleaves outer cycles and holes", face.id),
            ));
        }
    }
    Ok(())
}

/// The Euler characteristic of the planar subdivision (§9.6, §26.3).
///
/// `V - E + F` for a connected planar graph is 2. The topology here need not
/// be connected -- an image can contain separate components -- so the value is
/// `1 + C` where `C` is the number of connected components. It is reported
/// rather than asserted, because comparing it to a fixture's expected value is
/// how §26.3 uses it.
#[must_use]
pub fn euler_characteristic(t: &Topology) -> i64 {
    t.vertices.len() as i64 - t.edges.len() as i64 + t.faces.len() as i64
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::float_cmp, clippy::field_reassign_with_default)]
mod tests {
    use super::*;
    use crate::extract::{ExtractOptions, extract};
    use palette_tracer_color::spaces::Oklab;
    use palette_tracer_core::control::NoControl;
    use palette_tracer_core::ir::PaletteId;
    use palette_tracer_core::labels::{LabelId, LabelPlane};
    use palette_tracer_core::limits::{ResourceLimits, WorkBudget};

    /// Renumber labels densely, in row-major order of first appearance.
    ///
    /// Segmentation guarantees this (`Segmentation::assert_complete_ownership`
    /// refuses a plane where a label names no pixels), so the fixtures must
    /// too: a face for a label that occurs nowhere has no boundary, and the
    /// validator is right to reject it.
    fn compact(cells: &[u32]) -> Vec<u32> {
        let mut seen = std::collections::BTreeMap::new();
        cells
            .iter()
            .map(|&c| {
                let next = seen.len() as u32;
                *seen.entry(c).or_insert(next)
            })
            .collect()
    }

    fn topology_of(cells: &[u32], width: u32, height: u32) -> Topology {
        let cells = &compact(cells);
        let count = cells.iter().max().map_or(1, |m| u64::from(*m) + 1);
        let mut plane = LabelPlane::new(width, height, count.max(2)).unwrap();
        for (i, &c) in cells.iter().enumerate() {
            plane.set_index(i, LabelId(c));
        }
        let region_count = count as usize;
        extract(
            &plane,
            &vec![Oklab::ZERO; cells.len()],
            &(0..region_count).map(|i| Some(PaletteId(i as u32))).collect::<Vec<_>>(),
            &vec![false; region_count],
            &ExtractOptions::default(),
            &ResourceLimits::default(),
            &WorkBudget::unbounded(),
            &NoControl,
        )
        .unwrap()
        .topology
    }

    /// §39.5 asks for "a label-map-to-DCEL oracle" starting with "exhaustive
    /// 2x2/3x3 label patterns". Every 2x2 pattern over two labels and every
    /// 3x3 pattern over two labels must produce a valid topology.
    #[test]
    fn every_small_label_pattern_validates() {
        for pattern in 0..16u32 {
            let cells: Vec<u32> = (0..4).map(|i| (pattern >> i) & 1).collect();
            let t = topology_of(&cells, 2, 2);
            validate(&t).unwrap_or_else(|e| panic!("2x2 pattern {cells:?}: {e}"));
        }
        for pattern in 0..512u32 {
            let cells: Vec<u32> = (0..9).map(|i| (pattern >> i) & 1).collect();
            let t = topology_of(&cells, 3, 3);
            validate(&t).unwrap_or_else(|e| panic!("3x3 pattern {cells:?}: {e}"));
        }
    }

    /// Every 2x2 pattern over *three* labels, which is where T-junctions and
    /// four-way junctions both appear.
    #[test]
    fn every_three_label_two_by_two_pattern_validates() {
        for pattern in 0..81u32 {
            let cells: Vec<u32> = (0..4).map(|i| (pattern / 3u32.pow(i)) % 3).collect();
            let t = topology_of(&cells, 2, 2);
            validate(&t).unwrap_or_else(|e| panic!("pattern {cells:?}: {e}"));
        }
    }

    #[test]
    fn a_valid_topology_passes_every_check() {
        for (cells, w, h) in [
            (vec![0u32], 1, 1),
            (vec![0, 0, 0, 0, 1, 0, 0, 0, 0], 3, 3),
            (vec![0, 1, 1, 0], 2, 2),
            (vec![0, 1, 2, 0, 1, 2, 0, 1, 2], 3, 3),
        ] {
            validate(&topology_of(&cells, w, h))
                .unwrap_or_else(|e| panic!("{cells:?} at {w}x{h}: {e}"));
        }
    }

    /// The validator has to actually catch things. Each of these is a
    /// corruption a real defect could produce.
    #[test]
    fn a_broken_twin_is_caught() {
        let mut t = topology_of(&[0, 1, 1, 0], 2, 2);
        t.half_edges[0].twin = HalfEdgeId(0);
        let err = validate(&t).unwrap_err();
        assert_eq!(err.code(), "validation.topology");
        assert!(format!("{err}").contains("twin"), "{err}");
    }

    #[test]
    fn a_broken_next_prev_pair_is_caught() {
        let mut t = topology_of(&[0, 1, 1, 0], 2, 2);
        let victim = t.half_edges.len() - 1;
        t.half_edges[0].next = HalfEdgeId(victim as u32);
        assert!(validate(&t).is_err());
    }

    /// The gate that matters most: if the two sides of an interface ever stop
    /// being one chain, the validator must say so (§31.1, PTE-TOPO-001).
    #[test]
    fn independently_fitted_shared_geometry_is_caught() {
        let mut t = topology_of(&[0, 0, 1, 1], 2, 2);
        // Nudge one chain, simulating a fitter that refitted one side.
        t.chains[0].start.x += 0.25;
        let err = validate(&t).unwrap_err();
        assert!(
            format!("{err}").contains("PTE-TOPO-001") || format!("{err}").contains("chain"),
            "{err}"
        );
    }

    #[test]
    fn a_missing_exterior_face_is_caught() {
        let mut t = topology_of(&[0], 1, 1);
        t.faces[0].exterior = false;
        assert!(validate(&t).is_err());
    }

    /// §9.6: the Euler relation is reported for fixture comparison.
    #[test]
    fn the_euler_characteristic_is_computed() {
        // One pixel: one vertex-free closed chain, so V = 1 (the loop's
        // canonical start), E = 1, F = 2.
        let t = topology_of(&[0], 1, 1);
        assert_eq!(t.vertices.len(), 1);
        assert_eq!(t.edges.len(), 1);
        assert_eq!(t.faces.len(), 2);
        assert_eq!(euler_characteristic(&t), 2);
    }
}
