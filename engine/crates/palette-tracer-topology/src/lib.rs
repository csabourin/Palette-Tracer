//! Shared topology: one boundary between two faces, fitted once (§9).
//!
//! This crate is why the engine exists. §9.1 states the invariant:
//!
//! ```text
//! ∂A ∩ ∂B = e(A,B),   e(B,A) = reverse(e(A,B))
//! ```
//!
//! The interface between two faces is one [`SharedEdge`](palette_tracer_core::ir::SharedEdge)
//! carrying one [`CurveChain`](palette_tracer_core::ir::CurveChain). Both
//! faces reference it; one traverses it forwards and the other traverses the
//! reverse. There is no code path that gives a face its own copy, which is
//! what makes PTE-NO-009 ("do not fit one contour per colour for
//! seam-sensitive output") structurally impossible rather than merely
//! discouraged.
//!
//! The modules:
//!
//! * [`grid`] fixes the coordinate and left-side conventions.
//! * [`ambiguity`] resolves the 2x2 checkerboard by the §9.4 energy, so no
//!   fixed turn direction biases diagonal features (PTE-NO-012).
//! * [`extract`] builds the half-edge structure from a label plane
//!   (Appendix D.3).
//! * [`validate`] runs the Appendix B checklist (PTE-TOPO-005).
//!
//! # Not implemented
//!
//! Junction position optimisation (§9.5, PTE-TOPO-011/012/013) is not
//! implemented: junction vertices sit on the grid where extraction put them.
//! Nothing moves them, so "optimised once and shared" holds vacuously and no
//! independent per-edge smoothing can break it -- but the optimisation itself
//! is absent. Tiled extraction with seam stitching is also absent.
//! `docs/IMPLEMENTATION_STATUS.md` records both.

pub mod ambiguity;
pub mod extract;
pub mod grid;
pub mod validate;

pub use ambiguity::{AmbiguityProfile, AmbiguityWeights, Connection, Decision};
pub use extract::{ExtractOptions, Extraction, extract};
pub use grid::{Dir, GridPoint, Labels};
pub use validate::{euler_characteristic, validate};

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::float_cmp,
    clippy::field_reassign_with_default
)]
mod property_tests {
    use super::*;
    use palette_tracer_color::spaces::Oklab;
    use palette_tracer_core::control::NoControl;
    use palette_tracer_core::ir::{FaceId, PaletteId};
    use palette_tracer_core::labels::{LabelId, LabelPlane};
    use palette_tracer_core::limits::{ResourceLimits, WorkBudget};
    use proptest::prelude::*;

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

    fn build(cells: &[u32], width: u32, height: u32) -> Extraction {
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
            &(0..region_count)
                .map(|i| Some(PaletteId(i as u32)))
                .collect::<Vec<_>>(),
            &vec![false; region_count],
            &ExtractOptions::default(),
            &ResourceLimits::default(),
            &WorkBudget::unbounded(),
            &NoControl,
        )
        .unwrap()
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(200))]

        /// §27.2's property list, on random bounded partitions: every boundary
        /// has two incident faces including the exterior, twins are an
        /// involution, face walks close, and reversing twice returns the
        /// original geometry.
        #[test]
        fn random_partitions_satisfy_every_topology_invariant(
            side in 1u32..7,
            cells in prop::collection::vec(0u32..4, 1..49),
        ) {
            let needed = (side * side) as usize;
            prop_assume!(cells.len() >= needed);
            let cells = &cells[..needed];

            let extraction = build(cells, side, side);
            let t = &extraction.topology;

            validate(t).map_err(|e| TestCaseError::fail(e.to_string()))?;

            // Every edge separates two faces, and the exterior is one of the
            // faces available to separate.
            for edge in &t.edges {
                prop_assert_ne!(edge.left_face, edge.right_face);
            }
            prop_assert!(t.faces[0].exterior);
            prop_assert_eq!(t.faces[0].id, FaceId::EXTERIOR);

            // Reversing twice returns the original geometry, exactly.
            for chain in &t.chains {
                prop_assert_eq!(&chain.reversed().reversed(), chain);
            }
        }

        /// PTE-TEST-010: reflection and 90-degree rotation must produce a
        /// congruent topology. Random partitions include the diagonals and
        /// junctions the requirement says ordinary rectangles cannot expose.
        #[test]
        fn reflection_and_rotation_preserve_the_topology_counts(
            side in 2u32..6,
            cells in prop::collection::vec(0u32..3, 4..36),
        ) {
            let n = (side * side) as usize;
            prop_assume!(cells.len() >= n);
            let cells = &cells[..n];
            let s = side as usize;

            let original = build(cells, side, side);

            let mirrored: Vec<u32> = (0..n)
                .map(|i| cells[(i / s) * s + (s - 1 - i % s)])
                .collect();
            let rotated: Vec<u32> = (0..n)
                .map(|i| {
                    let (x, y) = (i % s, i / s);
                    cells[(s - 1 - x) * s + y]
                })
                .collect();

            for (name, variant) in [("mirrored", mirrored), ("rotated", rotated)] {
                let other = build(&variant, side, side);
                prop_assert_eq!(
                    original.topology.edges.len(),
                    other.topology.edges.len(),
                    "{} changed the edge count", name
                );
                prop_assert_eq!(
                    original.topology.interior_face_count(),
                    other.topology.interior_face_count(),
                    "{} changed the face count", name
                );
                prop_assert_eq!(
                    original.topology.hole_count(),
                    other.topology.hole_count(),
                    "{} changed the hole count", name
                );
                prop_assert_eq!(
                    original.junctions,
                    other.junctions,
                    "{} changed the junction count", name
                );
                prop_assert_eq!(
                    original.ambiguous_cells,
                    other.ambiguous_cells,
                    "{} changed the ambiguity count", name
                );
            }
        }

        /// Extraction is deterministic: the same labels give the same topology
        /// down to the identifier assignment (PTE-API-010).
        #[test]
        fn extraction_is_reproducible(
            side in 1u32..6,
            cells in prop::collection::vec(0u32..3, 1..36),
        ) {
            let n = (side * side) as usize;
            prop_assume!(cells.len() >= n);
            let cells = &cells[..n];
            let first = build(cells, side, side);
            let again = build(cells, side, side);
            prop_assert_eq!(first.topology, again.topology);
        }
    }
}
