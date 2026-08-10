//! The initial partition: a minimum-spanning-forest over the pixel graph
//! (§8.3).
//!
//! §8.3 recommends "a hierarchical watershed or minimum-spanning-forest
//! construction on the pixel graph, producing a saliency hierarchy rather than
//! committing to one threshold early".
//!
//! # Algorithm
//!
//! Kruskal over the 4-connected pixel graph, with the adaptive merge predicate
//! of Felzenszwalb and Huttenlocher's graph-based segmentation: two components
//! join when the edge between them is no larger than the internal contrast of
//! either, allowing a tolerance that shrinks as components grow.
//!
//! ```text
//! join(C1, C2)  iff  w ≤ min( Int(C1) + k/|C1| , Int(C2) + k/|C2| )
//! ```
//!
//! `Int(C)` is the largest edge already inside `C` -- the component's own
//! contrast -- so the predicate is scale-adaptive: a high-contrast region
//! tolerates high-contrast internal edges, and a flat one does not. `k`
//! controls how much a small component is allowed to grow before its own
//! statistics constrain it.
//!
//! PTE-SEG-008 requires an algorithm that is not the reference watershed to be
//! "a named algorithm with independent conformance results". It is named here,
//! in the report as `pte-watershed-rag/1`, and in
//! `docs/IMPLEMENTATION_STATUS.md`, which records that this is an MSF
//! construction and not Cousty's watershed cut.
//!
//! # Why every pixel ends up owned
//!
//! PTE-SEG-006 forbids "a one-pixel 'watershed line' of unassigned pixels".
//! Kruskal on the *pixel* graph cannot produce one: components are sets of
//! pixels, every pixel starts in its own singleton component, and unions only
//! ever combine them. There is no state a pixel can be in other than "in
//! exactly one component".
//!
//! # Determinism
//!
//! Edges are sorted by a counting sort over the 65536 `u16` buckets, which is
//! stable, and enumerated in row-major order within a bucket. Two edges of
//! equal weight are therefore always considered in raster order, on every
//! target (PTE-SEG-007).
//!
//! # Provenance
//!
//! The predicate is Felzenszwalb and Huttenlocher, *Efficient Graph-Based
//! Image Segmentation* (IJCV 2004) -- a published idea, implemented
//! independently from its description (PTE-LIC-004). Union-find with union by
//! size and path halving is textbook.

use crate::edges::EdgeField;
use palette_tracer_core::config::EffectiveSegmentation;
use palette_tracer_core::control::{TraceControl, check_cancel};
use palette_tracer_core::error::TraceError;
use palette_tracer_core::labels::{LabelId, LabelPlane};
use palette_tracer_core::limits::{ResourceLimits, WorkBudget, cost};

/// Scale parameter `k` in the merge predicate.
///
/// Expressed in quantised weight units multiplied by pixels, so `k/|C|` is a
/// weight. 12000 is roughly a fifth of the full weight range at a component of
/// one pixel and becomes negligible by a hundred pixels: small components are
/// allowed to find their neighbours, large ones are governed by their own
/// contrast. Calibration against the reference corpus is Phase 0 work that has
/// not been done; `docs/IMPLEMENTATION_STATUS.md` says so.
const SCALE_K: f64 = 12_000.0;

/// Disjoint-set forest with union by size and path halving.
///
/// Path *halving* rather than full compression: it gives the same near-linear
/// bound with one pass and no recursion, and §19.4 forbids "recursive call
/// depth proportional to boundary length".
#[derive(Debug)]
struct UnionFind {
    parent: Vec<u32>,
    size: Vec<u32>,
    /// Largest edge weight inside each component: `Int(C)`.
    internal: Vec<u16>,
}

impl UnionFind {
    fn new(n: usize) -> Self {
        Self {
            parent: (0..n as u32).collect(),
            size: vec![1; n],
            internal: vec![0; n],
        }
    }

    #[inline]
    fn find(&mut self, mut x: u32) -> u32 {
        while self.parent[x as usize] != x {
            let grandparent = self.parent[self.parent[x as usize] as usize];
            self.parent[x as usize] = grandparent;
            x = grandparent;
        }
        x
    }

    /// Join two roots, keeping the larger as the survivor.
    ///
    /// Ties go to the lower index, which is a raster position -- so the
    /// survivor is a property of the image, not of the traversal
    /// (PTE-SEG-007).
    fn union(&mut self, a: u32, b: u32, weight: u16) {
        let (keep, drop) = if self.size[a as usize] > self.size[b as usize]
            || (self.size[a as usize] == self.size[b as usize] && a < b)
        {
            (a, b)
        } else {
            (b, a)
        };
        self.parent[drop as usize] = keep;
        self.size[keep as usize] += self.size[drop as usize];
        self.internal[keep as usize] = self.internal[keep as usize]
            .max(self.internal[drop as usize])
            .max(weight);
    }
}

/// The outcome of the initial partition.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Partition {
    /// One region label per pixel, densely numbered from zero in row-major
    /// order of first appearance (PTE-API-010).
    pub labels: LabelPlane,
    /// Number of distinct regions.
    pub region_count: u32,
}

/// Build the initial partition (§8.3).
///
/// # Errors
///
/// [`TraceError::Cancelled`], [`TraceError::ResourceLimit`] if the region
/// count exceeds `limits.max_regions` (PTE-SEC-006), or
/// [`TraceError::Decode`] if the label plane cannot be allocated.
///
/// # Panics
///
/// Never; every index comes from a length computed in this function.
pub fn build(
    field: &EdgeField,
    _policy: &EffectiveSegmentation,
    limits: &ResourceLimits,
    budget: &WorkBudget,
    control: &dyn TraceControl,
) -> Result<Partition, TraceError> {
    let w = field.width();
    let h = field.height();
    let n = palette_tracer_core::checked::pixel_count(w, h)?;

    // Counting sort over the 65536 weight buckets: exact, stable, and O(E).
    // A comparison sort would be O(E log E) and would need a tie rule that
    // does not depend on the sort's internals; this needs neither.
    let mut bucket_counts = vec![0u32; usize::from(u16::MAX) + 1];
    let mut edge_count = 0usize;
    for (weight, _, _) in field.edges() {
        bucket_counts[usize::from(weight)] += 1;
        edge_count += 1;
    }
    let mut offsets = Vec::with_capacity(bucket_counts.len());
    let mut running = 0u32;
    for count in &bucket_counts {
        offsets.push(running);
        running += count;
    }
    let mut sorted: Vec<(u32, u32)> = vec![(0, 0); edge_count];
    let mut cursor = offsets.clone();
    for (weight, a, b) in field.edges() {
        let slot = &mut cursor[usize::from(weight)];
        sorted[*slot as usize] = (a as u32, b as u32);
        *slot += 1;
    }

    let mut uf = UnionFind::new(n);
    let mut position = 0usize;
    for (weight_index, count) in bucket_counts.iter().enumerate() {
        if *count == 0 {
            continue;
        }
        let weight = weight_index as u16;
        for _ in 0..*count {
            check_cancel(control, position)?;
            budget.charge(cost::EDGE)?;

            let (a, b) = sorted[position];
            position += 1;

            let ra = uf.find(a);
            let rb = uf.find(b);
            if ra == rb {
                continue;
            }
            let threshold_a =
                f64::from(uf.internal[ra as usize]) + SCALE_K / f64::from(uf.size[ra as usize]);
            let threshold_b =
                f64::from(uf.internal[rb as usize]) + SCALE_K / f64::from(uf.size[rb as usize]);
            if f64::from(weight) <= threshold_a.min(threshold_b) {
                uf.union(ra, rb, weight);
            }
        }
    }

    // Dense relabelling in row-major order of first appearance, so region 0 is
    // the region containing the top-left pixel on every run (PTE-API-010).
    let mut remap = vec![u32::MAX; n];
    let mut next = 0u32;
    for i in 0..n {
        let root = uf.find(i as u32) as usize;
        if remap[root] == u32::MAX {
            remap[root] = next;
            next += 1;
        }
    }

    limits.check_regions(u64::from(next))?;

    let mut labels = LabelPlane::new(w, h, u64::from(next))?;
    for i in 0..n {
        let root = uf.find(i as u32) as usize;
        labels.set_index(i, LabelId(remap[root]));
    }
    budget.note_bytes(labels.bytes() + field.bytes());

    Ok(Partition {
        labels,
        region_count: next,
    })
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
    use crate::edges::{self, PixelEvidence};
    use palette_tracer_color::EncodedSrgb;
    use palette_tracer_color::spaces::Oklab;
    use palette_tracer_core::config::{Profile, TraceConfig};
    use palette_tracer_core::control::NoControl;

    fn policy() -> EffectiveSegmentation {
        Profile::expand(&TraceConfig::default())
            .unwrap()
            .segmentation
    }

    fn lab(rgb: [u8; 3]) -> Oklab {
        EncodedSrgb::from_u8(rgb[0], rgb[1], rgb[2])
            .to_linear()
            .to_oklab()
    }

    fn partition_of(colors: &[Oklab], width: u32, height: u32) -> Partition {
        let evidence = PixelEvidence {
            color: colors.to_vec(),
            alpha: vec![1.0; colors.len()],
            claim: vec![LabelId::ZERO; colors.len()],
            width,
            height,
        };
        let field =
            edges::build(&evidence, &policy(), &WorkBudget::unbounded(), &NoControl).unwrap();
        build(
            &field,
            &policy(),
            &ResourceLimits::default(),
            &WorkBudget::unbounded(),
            &NoControl,
        )
        .unwrap()
    }

    /// PTE-SEG-002 and PTE-SEG-006: every pixel is owned, and there is no
    /// unassigned watershed line. This is the §31.1 hard gate
    /// "Unassigned pixels in shared partition: 0".
    #[test]
    fn every_pixel_is_owned_by_exactly_one_region() {
        let mut colors = Vec::new();
        for y in 0..8 {
            for x in 0..8 {
                colors.push(if x < 4 {
                    lab([20, 20, 20])
                } else {
                    lab([230, 230, 230])
                });
                let _ = y;
            }
        }
        let p = partition_of(&colors, 8, 8);

        let counts = p.labels.histogram(p.region_count as usize);
        assert_eq!(counts.iter().sum::<u64>(), 64, "every pixel accounted for");
        assert!(
            counts.iter().all(|&c| c > 0),
            "every region label must be used: dense numbering"
        );
    }

    #[test]
    fn a_flat_image_becomes_one_region() {
        let p = partition_of(&[lab([90, 110, 130]); 16], 4, 4);
        assert_eq!(p.region_count, 1);
    }

    #[test]
    fn two_contrasting_halves_become_two_regions() {
        let mut colors = Vec::new();
        for _ in 0..4 {
            for x in 0..4 {
                colors.push(if x < 2 {
                    lab([10, 10, 10])
                } else {
                    lab([245, 245, 245])
                });
            }
        }
        let p = partition_of(&colors, 4, 4);
        assert_eq!(p.region_count, 2);
        assert_ne!(p.labels.get(0, 0), p.labels.get(3, 0));
        assert_eq!(p.labels.get(0, 0), p.labels.get(1, 3));
    }

    /// Labels are assigned in row-major order of first appearance, so the
    /// region containing the top-left pixel is always region 0.
    #[test]
    fn labels_are_numbered_by_raster_position_not_allocation_order() {
        let mut colors = Vec::new();
        for _ in 0..2 {
            for x in 0..4 {
                colors.push(if x < 2 {
                    lab([245, 245, 245])
                } else {
                    lab([10, 10, 10])
                });
            }
        }
        let p = partition_of(&colors, 4, 2);
        assert_eq!(p.labels.get(0, 0), LabelId(0));
        assert_eq!(p.labels.get(3, 0), LabelId(1));
    }

    /// PTE-SEG-007: plateaus must resolve the same way every time. A uniform
    /// image is one enormous plateau, and a checkerboard is the worst case for
    /// equal weights.
    #[test]
    fn plateau_decisions_are_stable_across_runs() {
        let mut colors = Vec::new();
        for y in 0..6u32 {
            for x in 0..6u32 {
                colors.push(if (x + y) % 2 == 0 {
                    lab([0, 0, 0])
                } else {
                    lab([255, 255, 255])
                });
            }
        }
        let first = partition_of(&colors, 6, 6);
        for _ in 0..4 {
            assert_eq!(partition_of(&colors, 6, 6), first);
        }
    }

    /// PTE-TEST-010: a reflected image must produce a reflected partition. An
    /// axis bias in the tie rule shows up here and nowhere else.
    #[test]
    fn a_horizontally_reflected_image_partitions_congruently() {
        let mut colors = Vec::new();
        for y in 0..5u32 {
            for x in 0..5u32 {
                // A diagonal feature, which is what exposes directional bias.
                colors.push(if x == y {
                    lab([20, 20, 20])
                } else {
                    lab([240, 240, 240])
                });
            }
        }
        let original = partition_of(&colors, 5, 5);

        let mut mirrored = Vec::new();
        for y in 0..5usize {
            for x in 0..5usize {
                mirrored.push(colors[y * 5 + (4 - x)]);
            }
        }
        let reflected = partition_of(&mirrored, 5, 5);

        assert_eq!(
            original.region_count, reflected.region_count,
            "reflection must not change how many regions there are"
        );
        // Every pixel's region membership must mirror too.
        for y in 0..5u32 {
            for x in 0..5u32 {
                let a = original.labels.get(x, y);
                let b = reflected.labels.get(4 - x, y);
                let a_partner = original.labels.get(0, 0) == a;
                let b_partner = reflected.labels.get(4, 0) == b;
                assert_eq!(a_partner, b_partner, "at ({x}, {y})");
            }
        }
    }

    /// Under 8-connectivity a checkerboard is *two* interleaved diagonal
    /// regions, not 256 single pixels. That is the correct reading, and it is
    /// what gives §9.4 something to decide.
    #[test]
    fn a_checkerboard_is_two_diagonal_regions_not_one_per_pixel() {
        let mut colors = Vec::new();
        for y in 0..8u32 {
            for x in 0..8u32 {
                colors.push(if (x + y) % 2 == 0 {
                    lab([0, 0, 0])
                } else {
                    lab([255, 255, 255])
                });
            }
        }
        let p = partition_of(&colors, 8, 8);
        assert_eq!(p.region_count, 2);
    }

    /// PTE-SEC-006: the adversarial case that really does explode. Colour noise
    /// gives one region per pixel under any connectivity, and the limit must
    /// catch it after the image itself was accepted.
    #[test]
    fn an_adversarial_region_count_hits_the_limit_cleanly() {
        // A fixed, reproducible spread of well-separated colours: no two
        // neighbours are close enough to join.
        let mut colors = Vec::new();
        for i in 0..256u32 {
            let v = (i * 61 % 256) as u8;
            colors.push(lab([v, 255 - v, (i * 37 % 256) as u8]));
        }
        let evidence = PixelEvidence {
            color: colors,
            alpha: vec![1.0; 256],
            claim: vec![LabelId::ZERO; 256],
            width: 16,
            height: 16,
        };
        let field =
            edges::build(&evidence, &policy(), &WorkBudget::unbounded(), &NoControl).unwrap();

        let mut limits = ResourceLimits::small();
        limits.max_regions = 4;
        let err = build(
            &field,
            &policy(),
            &limits,
            &WorkBudget::unbounded(),
            &NoControl,
        )
        .unwrap_err();
        assert_eq!(err.code(), "resource.exceeded");
    }

    #[test]
    fn the_partition_respects_the_work_budget() {
        let colors = vec![lab([10, 10, 10]); 64];
        let evidence = PixelEvidence {
            color: colors,
            alpha: vec![1.0; 64],
            claim: vec![LabelId::ZERO; 64],
            width: 8,
            height: 8,
        };
        let field =
            edges::build(&evidence, &policy(), &WorkBudget::unbounded(), &NoControl).unwrap();

        let mut limits = ResourceLimits::small();
        limits.work_budget = Some(4);
        let budget = WorkBudget::new(&limits);
        let err = build(&field, &policy(), &limits, &budget, &NoControl).unwrap_err();
        assert_eq!(err.code(), "resource.work_budget_exhausted");
    }
}
