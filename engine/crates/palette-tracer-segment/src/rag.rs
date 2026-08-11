//! Region adjacency graph, deterministic merging, fringe reassignment, and
//! thin-feature protection (§8.4--§8.7).
//!
//! # Adjacency comes from the raster (PTE-SEG-009, PTE-NO-013)
//!
//! Two regions are neighbours because pixels of one touch pixels of the other,
//! full stop. No floating-point curve is consulted, because no curve exists
//! yet -- and once one does, the raster partition stays authoritative.
//!
//! # Merging (§8.5, Appendix D.2)
//!
//! ```text
//! C_ij = (ΔV_ij + P_claim + P_role + P_edge) / G_ij
//! ```
//!
//! `ΔV` is the §8.5 identity, priced in constant time from the sufficient
//! statistics (PTE-SEG-010). The gain `G` combines the shared boundary length
//! `G_MS` with the area gain `G_area = max(n_i, n_j)/(n_i + n_j)`, which is
//! what makes a thin fringe fragment cheap to absorb into a large neighbour
//! while two large neighbours stay separate.
//!
//! §8.5 is explicit that the area gain "is not sufficient by itself: edge
//! confidence and palette roles must prevent absorption across a real
//! boundary". `P_edge` is that edge confidence and `P_claim` is that palette
//! role.
//!
//! The queue holds stale entries and detects them by generation on pop
//! (PTE-SEG-013), and equal costs resolve by
//! `(cost_bin, min_region_key, max_region_key)` (PTE-SEG-014).
//!
//! # Dimensions (PTE-SEG-011)
//!
//! `ΔV` is in squared OKLab units multiplied by pixels. The gain is in pixels
//! multiplied by a dimensionless area ratio. The quotient is therefore in
//! squared OKLab units, and `segmentation.mergeThreshold` is documented in
//! exactly those. The penalties are added *before* the division and are
//! expressed in the same units as `ΔV`, so nothing dimensionally incompatible
//! shares a slider.

use crate::edges::{EdgeField, PixelEvidence};
use palette_tracer_color::alpha::PremulLinearRgba;
use palette_tracer_color::mixture::{CompositeSpace, MixturePolicy, two_color_best};
use palette_tracer_color::spaces::LinearRgb;
use palette_tracer_color::stats::ColorStats;
use palette_tracer_core::checked::u32_to_usize;
use palette_tracer_core::config::EffectiveSegmentation;
use palette_tracer_core::control::{TraceControl, check_cancel};
use palette_tracer_core::determinism::{Generation, MinHeap, QuantKey, canonical_pair};
use palette_tracer_core::error::TraceError;
use palette_tracer_core::labels::{LabelId, LabelPlane};
use palette_tracer_core::limits::{WorkBudget, cost};
use std::collections::BTreeMap;

/// Penalty added to a merge that would cross a palette-claim boundary
/// (PTE-COLOR-014).
///
/// In the same units as `ΔV`: squared OKLab distance times pixels. `0.02`
/// corresponds to two regions of one pixel each whose colours differ by an
/// OKLab distance of about 0.14 -- a clearly visible difference. Below that,
/// the claim disagreement is not by itself enough to keep them apart.
const CLAIM_PENALTY: f64 = 0.02;

/// Scale of the edge-evidence penalty.
///
/// Multiplied by the mean normalised edge weight along the shared boundary and
/// by the boundary length, so a long high-contrast interface is expensive to
/// cross and a short faint one is not.
const EDGE_PENALTY_SCALE: f64 = 0.04;

/// Scale of the protection penalty for a thin feature (§8.7).
const PROTECTION_PENALTY_SCALE: f64 = 0.25;

/// Everything the merge objective needs to know about one region.
#[derive(Debug, Clone, PartialEq)]
pub struct RegionSummary {
    /// Pixel count.
    pub area: u64,
    /// Bounding box as `(min_x, min_y, max_x, max_y)`, inclusive.
    pub bbox: (u32, u32, u32, u32),
    /// Colour sufficient statistics (§7.7).
    pub stats: ColorStats,
    /// Mean alpha.
    pub mean_alpha: f64,
    /// The palette entry most of this region claims.
    pub claim: LabelId,
    /// Whether that claim is pinned (PTE-SEG-012).
    pub pinned: bool,
    /// Length of the region's boundary, counting raster cell interfaces
    /// including the image border.
    pub perimeter: u64,
    /// Thin-feature protection score in `[0, 1]` (§8.7).
    pub protection: f64,
}

impl RegionSummary {
    /// Mean colour.
    #[must_use]
    pub fn mean(&self) -> palette_tracer_color::Oklab {
        self.stats.mean()
    }

    /// The region's mean colour as a premultiplied linear sample, for mixture
    /// tests.
    #[must_use]
    pub fn premultiplied_mean(&self) -> PremulLinearRgba {
        PremulLinearRgba::from_straight(self.mean().to_linear(), self.mean_alpha)
    }
}

/// One raster interface between two regions.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Adjacency {
    /// Number of raster cell interfaces shared.
    pub boundary_length: u64,
    /// Sum of the edge-field weights across those interfaces.
    pub weight_sum: u64,
}

impl Adjacency {
    /// Mean normalised edge weight across the interface, in `[0, 1]`.
    #[must_use]
    pub fn mean_weight(&self) -> f64 {
        if self.boundary_length == 0 {
            0.0
        } else {
            self.weight_sum as f64
                / (self.boundary_length as f64 * f64::from(crate::edges::MAX_WEIGHT))
        }
    }
}

/// A region adjacency graph (§8.4).
#[derive(Debug, Clone)]
pub struct RegionGraph {
    summaries: Vec<RegionSummary>,
    adjacency: Vec<BTreeMap<u32, Adjacency>>,
    parent: Vec<u32>,
    generation: Vec<Generation>,
    alive: Vec<bool>,
}

impl RegionGraph {
    /// Number of region slots, including ones merged away.
    #[must_use]
    pub fn len(&self) -> usize {
        self.summaries.len()
    }

    /// Whether the graph has no regions.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.summaries.is_empty()
    }

    /// Regions still alive.
    #[must_use]
    pub fn live_count(&self) -> u32 {
        self.alive.iter().filter(|a| **a).count() as u32
    }

    /// The current root of the region that started as `id`.
    ///
    /// Path halving, as in the partition's union-find, for the same reason:
    /// §19.4 forbids recursion proportional to structure size.
    #[must_use]
    pub fn root(&self, mut id: u32) -> u32 {
        while self.parent[id as usize] != id {
            id = self.parent[self.parent[id as usize] as usize];
        }
        id
    }

    fn root_mut(&mut self, mut id: u32) -> u32 {
        while self.parent[id as usize] != id {
            let grandparent = self.parent[self.parent[id as usize] as usize];
            self.parent[id as usize] = grandparent;
            id = grandparent;
        }
        id
    }

    /// Summary of a live region.
    #[must_use]
    pub fn summary(&self, id: u32) -> &RegionSummary {
        &self.summaries[id as usize]
    }

    /// Neighbours of a live region, in ascending identifier order.
    ///
    /// `BTreeMap`, not `HashMap`: PTE-NO-043 forbids unordered iteration in
    /// anything that reaches output, and the merge order does.
    #[must_use]
    pub fn neighbors(&self, id: u32) -> &BTreeMap<u32, Adjacency> {
        &self.adjacency[id as usize]
    }
}

/// Build the region adjacency graph from a partition (§8.4).
///
/// # Errors
///
/// [`TraceError::Cancelled`] or [`TraceError::ResourceLimit`].
///
/// # Panics
///
/// Never; every index derives from `region_count`, which the caller obtained
/// from the same label plane.
pub fn build(
    labels: &LabelPlane,
    region_count: u32,
    evidence: &PixelEvidence,
    field: &EdgeField,
    pinned_claims: &[bool],
    budget: &WorkBudget,
    control: &dyn TraceControl,
) -> Result<RegionGraph, TraceError> {
    let n = region_count as usize;
    let w = labels.width();
    let h = labels.height();

    let mut summaries = vec![
        RegionSummary {
            area: 0,
            bbox: (u32::MAX, u32::MAX, 0, 0),
            stats: ColorStats::new(),
            mean_alpha: 0.0,
            claim: LabelId::ZERO,
            pinned: false,
            perimeter: 0,
            protection: 0.0,
        };
        n
    ];
    let mut adjacency = vec![BTreeMap::<u32, Adjacency>::new(); n];
    let mut alpha_sums = vec![0.0f64; n];
    // Claim histogram per region, as a sparse map: a region rarely spans more
    // than a handful of palette entries, so this stays far below `R x K`.
    let mut claim_counts = vec![BTreeMap::<u32, u64>::new(); n];

    let mut i = 0usize;
    for y in 0..h {
        for x in 0..w {
            check_cancel(control, i)?;
            budget.charge(cost::PIXEL)?;

            let region = labels.get_index(i).0 as usize;
            let s = &mut summaries[region];
            s.area += 1;
            s.bbox.0 = s.bbox.0.min(x);
            s.bbox.1 = s.bbox.1.min(y);
            s.bbox.2 = s.bbox.2.max(x);
            s.bbox.3 = s.bbox.3.max(y);
            s.stats.push(evidence.color[i]);
            alpha_sums[region] += evidence.alpha[i];
            *claim_counts[region].entry(evidence.claim[i].0).or_insert(0) += 1;

            // Interfaces to the right and below, plus the image border. The
            // border counts toward the perimeter because a region touching the
            // edge of the image has a real boundary there -- with the exterior
            // face (PTE-TOPO-007).
            if x + 1 < w {
                let other = labels.get(x + 1, y).0 as usize;
                if other != region {
                    record(
                        &mut adjacency,
                        &mut summaries,
                        region,
                        other,
                        field.horizontal(x, y),
                    );
                }
            } else {
                summaries[region].perimeter += 1;
            }
            if x == 0 {
                summaries[region].perimeter += 1;
            }
            if y + 1 < h {
                let other = labels.get(x, y + 1).0 as usize;
                if other != region {
                    record(
                        &mut adjacency,
                        &mut summaries,
                        region,
                        other,
                        field.vertical(x, y),
                    );
                }
            } else {
                summaries[region].perimeter += 1;
            }
            if y == 0 {
                summaries[region].perimeter += 1;
            }

            i += 1;
        }
    }

    for (region, summary) in summaries.iter_mut().enumerate() {
        if summary.area > 0 {
            summary.mean_alpha = alpha_sums[region] / summary.area as f64;
        }
        // The dominant claim, ties going to the lower palette id so the answer
        // is a property of the palette rather than of the scan.
        summary.claim = LabelId(
            claim_counts[region]
                .iter()
                .max_by_key(|(id, count)| (**count, std::cmp::Reverse(**id)))
                .map_or(0, |(id, _)| *id),
        );
        summary.pinned = pinned_claims
            .get(summary.claim.index())
            .copied()
            .unwrap_or(false);
        summary.protection = protection_score(summary);
    }

    Ok(RegionGraph {
        summaries,
        adjacency,
        parent: (0..region_count).collect(),
        generation: vec![Generation::default(); n],
        alive: vec![true; n],
    })
}

fn record(
    adjacency: &mut [BTreeMap<u32, Adjacency>],
    summaries: &mut [RegionSummary],
    a: usize,
    b: usize,
    weight: u16,
) {
    for (from, to) in [(a, b), (b, a)] {
        let entry = adjacency[from].entry(to as u32).or_default();
        entry.boundary_length += 1;
        entry.weight_sum += u64::from(weight);
        summaries[from].perimeter += 1;
    }
}

/// Thin-feature protection score in `[0, 1]` (§8.7).
///
/// PTE-SEG-017: "Size alone MUST NOT classify a component as noise. A long
/// one-pixel feature, punctuation mark, letter counter, eye highlight, or
/// registration mark may be semantically important."
///
/// Two of the §8.7 signals are computable from the summary alone:
///
/// * **Elongation** `perimeter² / (16 · area)`. For a compact blob this is
///   near `1`; for a one-pixel-wide line of length `L` it grows like `L/4`. A
///   long thin feature scores high, a speck scores low, and *both* may be
///   small in area -- which is precisely the distinction area cannot make.
/// * **Distance-transform width** approximated by `2 · area / perimeter`. A
///   feature one pixel wide has a width near `1` regardless of how long it is.
///
/// The remaining §8.7 signals -- geodesic length, repeated-pattern evidence,
/// endpoint and junction evidence, and profile role -- are **not** implemented.
/// `docs/IMPLEMENTATION_STATUS.md` records that, because a partial score that
/// claimed to be the whole thing would be exactly the kind of quiet
/// approximation §0.2 exists to prevent.
#[must_use]
pub fn protection_score(summary: &RegionSummary) -> f64 {
    if summary.area == 0 || summary.perimeter == 0 {
        return 0.0;
    }
    let area = summary.area as f64;
    let perimeter = summary.perimeter as f64;

    let elongation = perimeter * perimeter / (16.0 * area);
    let width = 2.0 * area / perimeter;

    // Thin *and* elongated is the protected shape. A wide blob is not
    // protected however large, and a single stray pixel is not protected
    // however thin.
    let thinness = (2.0 - width).clamp(0.0, 1.0);
    let length = ((elongation - 1.0) / 3.0).clamp(0.0, 1.0);
    (thinness * length).clamp(0.0, 1.0)
}

/// One merge candidate, ordered by the §8.5 cost and the PTE-SEG-014 tie rule.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
struct Candidate {
    /// Quantised cost. First in the tuple, so it dominates the ordering.
    cost: QuantKey,
    /// Lower region identifier.
    low: u32,
    /// Higher region identifier.
    high: u32,
    /// Generations recorded when the candidate was queued (PTE-SEG-013).
    generations: (Generation, Generation),
}

/// What merging did, for the report and for the audit trail.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct MergeOutcome {
    /// Merges performed.
    pub merges: u32,
    /// Candidates popped and discarded as stale.
    pub stale_entries: u32,
    /// Regions whose protection score kept them (§8.7).
    pub protected: u32,
    /// Fringe reassignments (§8.6).
    pub fringe: Vec<FringeAudit>,
}

/// A reversible record of one fringe reassignment (PTE-SEG-016).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FringeAudit {
    /// The region that was absorbed.
    pub source: u32,
    /// The region that absorbed it.
    pub target: u32,
    /// Its pixel count.
    pub pixels: u64,
    /// Mixture residual that justified the decision, in linear-light units.
    pub residual: f64,
    /// Inferred coverage of the target.
    pub coverage: f64,
    /// Which compositing transfer explained the observation (§10.2).
    ///
    /// PTE-SEG-016 wants the decision reversible from its record, and the
    /// residual alone does not say which model produced it.
    pub space: CompositeSpace,
}

/// The §8.5 merge cost for two live regions, or `None` if the merge is
/// forbidden outright.
fn merge_cost(graph: &RegionGraph, a: u32, b: u32, policy: &EffectiveSegmentation) -> Option<f64> {
    let sa = &graph.summaries[a as usize];
    let sb = &graph.summaries[b as usize];

    // PTE-SEG-012: distinct pinned palette identities never merge. This is a
    // hard rule, not a penalty -- no cost is low enough to buy it.
    if sa.pinned && sb.pinned && sa.claim != sb.claim {
        return None;
    }

    let adjacency = graph.adjacency[a as usize].get(&b).copied()?;
    if adjacency.boundary_length == 0 {
        return None;
    }

    let delta_v = sa.stats.merge_cost(&sb.stats);

    let claim_penalty = if sa.claim == sb.claim {
        0.0
    } else {
        CLAIM_PENALTY * policy.edge_weight_claim
    };
    let edge_penalty =
        EDGE_PENALTY_SCALE * adjacency.mean_weight() * adjacency.boundary_length as f64;
    // §8.7: a protected feature resists absorption. The penalty scales with
    // the *smaller* region's protection, because absorbing a protected thin
    // feature into a big neighbour is what destroys it.
    let role_penalty = PROTECTION_PENALTY_SCALE
        * if sa.area <= sb.area {
            sa.protection
        } else {
            sb.protection
        };

    let area_gain = sa.area.max(sb.area) as f64 / (sa.area + sb.area) as f64;
    let gain = adjacency.boundary_length as f64 * area_gain;
    if gain <= 0.0 {
        return None;
    }

    Some((delta_v + claim_penalty + edge_penalty + role_penalty) / gain)
}

/// Merge regions by the §8.5 objective, following Appendix D.2.
///
/// # Errors
///
/// [`TraceError::Cancelled`] or [`TraceError::ResourceLimit`].
pub fn merge(
    graph: &mut RegionGraph,
    policy: &EffectiveSegmentation,
    budget: &WorkBudget,
    control: &dyn TraceControl,
) -> Result<MergeOutcome, TraceError> {
    let mut outcome = MergeOutcome::default();
    if policy.merge_threshold <= 0.0 {
        // A threshold of zero means "do not merge", which the `pixel-art`
        // profile relies on: its regions are the artist's intent, not an
        // over-segmentation to be cleaned up (PTE-GEO-025).
        return Ok(outcome);
    }

    let mut heap: MinHeap<Candidate> = MinHeap::with_capacity(graph.len() * 2);
    for a in 0..graph.len() as u32 {
        for &b in graph.adjacency[a as usize].keys() {
            if a >= b {
                continue;
            }
            if let Some(cost) = merge_cost(graph, a, b, policy)
                && cost <= policy.merge_threshold
            {
                heap.push(Candidate {
                    cost: QuantKey::cost_or_worst(cost),
                    low: a,
                    high: b,
                    generations: (graph.generation[a as usize], graph.generation[b as usize]),
                });
            }
        }
    }

    let mut popped = 0usize;
    while let Some(entry) = heap.pop() {
        check_cancel(control, popped)?;
        budget.charge(cost::MERGE_POP)?;
        popped += 1;

        let a = graph.root_mut(entry.low);
        let b = graph.root_mut(entry.high);
        if a == b {
            outcome.stale_entries += 1;
            continue;
        }
        // PTE-SEG-013: a queued candidate naming a generation that has moved
        // on describes regions that no longer exist in that form.
        if entry.generations != (graph.generation[a as usize], graph.generation[b as usize])
            || entry.low != a
            || entry.high != b
        {
            outcome.stale_entries += 1;
            // Re-price rather than drop: the two roots may still be a good
            // merge, and Appendix D.2 pushes the recomputed entry.
            if let Some(cost) = merge_cost(graph, a.min(b), a.max(b), policy)
                && cost <= policy.merge_threshold
            {
                heap.push(Candidate {
                    cost: QuantKey::cost_or_worst(cost),
                    low: a.min(b),
                    high: a.max(b),
                    generations: (graph.generation[a as usize], graph.generation[b as usize]),
                });
            }
            continue;
        }

        let Some(cost) = merge_cost(graph, a, b, policy) else {
            continue;
        };
        if cost > policy.merge_threshold {
            if graph.summaries[a as usize].protection > 0.5
                || graph.summaries[b as usize].protection > 0.5
            {
                outcome.protected += 1;
            }
            continue;
        }

        commit(graph, a, b);
        outcome.merges += 1;

        // Re-price the survivor's neighbourhood.
        let keep = graph.root(a);
        let neighbors: Vec<u32> = graph.adjacency[keep as usize].keys().copied().collect();
        for neighbor in neighbors {
            let other = graph.root_mut(neighbor);
            if other == keep {
                continue;
            }
            let (low, high) = canonical_pair(keep, other);
            if let Some(cost) = merge_cost(graph, low, high, policy)
                && cost <= policy.merge_threshold
            {
                heap.push(Candidate {
                    cost: QuantKey::cost_or_worst(cost),
                    low,
                    high,
                    generations: (
                        graph.generation[low as usize],
                        graph.generation[high as usize],
                    ),
                });
            }
        }
    }

    Ok(outcome)
}

/// Join `b` into `a`, keeping the larger region as the survivor.
fn commit(graph: &mut RegionGraph, a: u32, b: u32) {
    let (keep, drop) = if graph.summaries[a as usize].area > graph.summaries[b as usize].area
        || (graph.summaries[a as usize].area == graph.summaries[b as usize].area && a < b)
    {
        (a, b)
    } else {
        (b, a)
    };

    let dropped = graph.summaries[drop as usize].clone();
    let shared = graph.adjacency[keep as usize]
        .get(&drop)
        .copied()
        .unwrap_or_default();

    {
        let s = &mut graph.summaries[keep as usize];
        s.stats = s.stats.merged(&dropped.stats);
        let total = s.area + dropped.area;
        if total > 0 {
            s.mean_alpha = (s.mean_alpha * s.area as f64
                + dropped.mean_alpha * dropped.area as f64)
                / total as f64;
        }
        s.area = total;
        s.bbox = (
            s.bbox.0.min(dropped.bbox.0),
            s.bbox.1.min(dropped.bbox.1),
            s.bbox.2.max(dropped.bbox.2),
            s.bbox.3.max(dropped.bbox.3),
        );
        // The shared interface stops being a boundary of either region.
        s.perimeter = s.perimeter + dropped.perimeter - 2 * shared.boundary_length;
        // A pinned identity survives a merge with an unpinned neighbour; the
        // hard rule above already prevented two different pinned identities
        // from meeting here.
        if dropped.pinned && !s.pinned {
            s.pinned = true;
            s.claim = dropped.claim;
        }
        s.protection = protection_score(s);
    }

    // Re-target adjacency.
    let dropped_adjacency = std::mem::take(&mut graph.adjacency[drop as usize]);
    for (other, adjacency) in dropped_adjacency {
        graph.adjacency[other as usize].remove(&drop);
        if other == keep {
            continue;
        }
        let into = graph.adjacency[keep as usize].entry(other).or_default();
        into.boundary_length += adjacency.boundary_length;
        into.weight_sum += adjacency.weight_sum;
        let back = graph.adjacency[other as usize].entry(keep).or_default();
        back.boundary_length += adjacency.boundary_length;
        back.weight_sum += adjacency.weight_sum;
    }
    graph.adjacency[keep as usize].remove(&drop);

    graph.parent[drop as usize] = keep;
    graph.alive[drop as usize] = false;
    graph.generation[keep as usize].bump();
    graph.generation[drop as usize].bump();
}

/// Reassign antialias fringe regions (§8.6).
///
/// A thin region whose colour is well explained as a mixture of two larger
/// adjacent regions is a rasterisation artefact, not an intended third fill
/// (PTE-NO-006). It is reassigned to the side its inferred coverage favours.
///
/// PTE-SEG-015: this happens *before* topology construction, so the fringe
/// never becomes a face with its own boundary. PTE-SEG-016: every decision
/// leaves an audit record.
///
/// # Errors
///
/// [`TraceError::Cancelled`] or [`TraceError::ResourceLimit`].
pub fn reassign_fringe(
    graph: &mut RegionGraph,
    policy: &EffectiveSegmentation,
    budget: &WorkBudget,
    control: &dyn TraceControl,
) -> Result<Vec<FringeAudit>, TraceError> {
    let mut audits = Vec::new();
    if policy.fringe_max_residual <= 0.0 {
        return Ok(audits);
    }

    let mixture_policy = MixturePolicy {
        max_residual: policy.fringe_max_residual,
        ..MixturePolicy::default()
    };

    // Candidates in ascending identifier order, which is raster order of first
    // appearance -- so the sweep is deterministic (PTE-API-010).
    let candidates: Vec<u32> = (0..graph.len() as u32)
        .filter(|&id| graph.alive[id as usize])
        .collect();

    for (step, id) in candidates.into_iter().enumerate() {
        check_cancel(control, step)?;
        budget.charge(cost::MERGE_POP)?;

        let root = graph.root_mut(id);
        if root != id || !graph.alive[id as usize] {
            continue;
        }
        let summary = graph.summaries[id as usize].clone();

        // A fringe is thin and small relative to its neighbours, and never a
        // pinned identity (§8.6).
        //
        // §8.7's protection score is deliberately *not* consulted here. It used
        // to veto any region scoring above 0.5, which inverted §8.6's own
        // logic: §8.6 lists thinness as evidence *for* a region being fringe,
        // and its four conditions do not include a shape veto. The veto also
        // misfired on precisely the shape antialiasing produces. Under the
        // 8-connected partition a three-pixel chain touching only at corners
        // has perimeter 12 against area 3, so `perimeter²/(16·area)` reads 3.0
        // where the same length of straight one-pixel strip reads 1.33 -- the
        // proxy is calibrated for 4-connected blobs and over-scores diagonal
        // chains by more than twice. That is what kept an antialiased circle
        // at eight faces after every other fringe fragment had been absorbed.
        //
        // What separates a hairline from a fringe is not shape but colour: a
        // dark line between two regions is not a mixture of them, so the
        // residual gate below rejects it. `an_antialiased_hairline_is_not_
        // absorbed_because_it_is_not_a_mixture` is that claim as a test, and
        // `topology/one-pixel-bridge` is it end to end in the corpus.
        if summary.pinned {
            continue;
        }
        if summary.area > u64::from(policy.small_region_pixels).max(1) {
            continue;
        }

        // The two heaviest neighbours are the mixture candidates.
        let mut neighbors: Vec<(u32, u64)> = graph.adjacency[id as usize]
            .iter()
            .map(|(&other, adjacency)| (other, adjacency.boundary_length))
            .collect();
        if neighbors.len() < 2 {
            continue;
        }
        // Sort by shared boundary length descending, ties by identifier so the
        // pair is a property of the partition.
        neighbors.sort_by_key(|&(other, length)| (std::cmp::Reverse(length), other));

        let (a_id, b_id) = (neighbors[0].0, neighbors[1].0);
        let a = graph.summaries[a_id as usize].premultiplied_mean();
        let b = graph.summaries[b_id as usize].premultiplied_mean();
        let c = summary.premultiplied_mean();

        // §10.2: score both compositing hypotheses and keep the one that
        // explains the observation. Fitting only the linear-light model to a
        // raster that was composited on encoded values costs a systematic
        // residual of up to 0.057 -- above this gate for ordinary artwork
        // colours -- and rejects genuine fringe as if it were a third fill.
        let Some(best) = two_color_best(c, a, b, &mixture_policy) else {
            continue;
        };
        let (coverage, residual, space) =
            (best.mixture.coverage, best.mixture.residual, best.space);
        if residual > policy.fringe_max_residual {
            continue;
        }

        // Absorb into whichever side the coverage favours; a tie goes to the
        // lower identifier so the outcome does not depend on rounding.
        let target = if coverage > 0.5 {
            a_id
        } else if coverage < 0.5 {
            b_id
        } else {
            a_id.min(b_id)
        };
        let target_root = graph.root_mut(target);
        if target_root == id {
            continue;
        }

        audits.push(FringeAudit {
            source: id,
            target: target_root,
            pixels: summary.area,
            residual,
            coverage,
            space,
        });
        commit(graph, target_root, id);
    }

    Ok(audits)
}

/// Reassign fringe regions until a complete deterministic sweep makes no
/// further change.
///
/// Absorbing one band can expose the next band to its two true neighbours. A
/// single raster-order sweep can therefore under-trigger on layered
/// antialiasing even though every individual decision is valid. Each
/// successful pass removes at least one live region, so this terminates after
/// at most the initial region count and retains every reversible audit record
/// (PTE-SEG-015/016).
///
/// # Errors
///
/// [`TraceError::Cancelled`] or [`TraceError::ResourceLimit`].
pub fn reassign_fringe_to_fixed_point(
    graph: &mut RegionGraph,
    policy: &EffectiveSegmentation,
    budget: &WorkBudget,
    control: &dyn TraceControl,
) -> Result<Vec<FringeAudit>, TraceError> {
    let mut audits = Vec::new();
    loop {
        let pass = reassign_fringe(graph, policy, budget, control)?;
        if pass.is_empty() {
            return Ok(audits);
        }
        audits.extend(pass);
    }
}

/// Rewrite a label plane so it names the surviving regions, densely numbered
/// in row-major order of first appearance (PTE-API-010).
///
/// PTE-SEG-003 in mechanical form: every pixel of a region that merged away is
/// carried to its new owner. There is no path here that drops a pixel, which
/// is what PTE-NO-007 forbids.
///
/// # Errors
///
/// [`TraceError::Decode`] if the new label plane cannot be allocated.
pub fn apply(graph: &RegionGraph, labels: &LabelPlane) -> Result<(LabelPlane, u32), TraceError> {
    let mut remap = vec![u32::MAX; graph.len()];
    let mut next = 0u32;
    for i in 0..labels.len() {
        let root = graph.root(labels.get_index(i).0) as usize;
        if remap[root] == u32::MAX {
            remap[root] = next;
            next += 1;
        }
    }

    let mut out = LabelPlane::new(labels.width(), labels.height(), u64::from(next))?;
    for i in 0..labels.len() {
        let root = graph.root(labels.get_index(i).0) as usize;
        out.set_index(i, LabelId(remap[root]));
    }
    Ok((out, next))
}

/// Summaries of the surviving regions, in the order [`apply`] numbers them.
#[must_use]
pub fn live_summaries(graph: &RegionGraph, labels: &LabelPlane) -> Vec<RegionSummary> {
    let mut seen = vec![u32::MAX; graph.len()];
    let mut out = Vec::new();
    for i in 0..labels.len() {
        let root = graph.root(labels.get_index(i).0) as usize;
        if seen[root] == u32::MAX {
            seen[root] = out.len() as u32;
            out.push(graph.summaries[root].clone());
        }
    }
    out
}

/// Convert a linear colour to a premultiplied opaque sample, for tests and
/// callers building summaries by hand.
#[must_use]
pub fn opaque(color: LinearRgb) -> PremulLinearRgba {
    PremulLinearRgba::from_straight(color, 1.0)
}

/// Row-major pixel index, re-exported so callers do not re-derive it.
#[must_use]
pub fn index_of(width: u32, x: u32, y: u32) -> usize {
    u32_to_usize(y) * u32_to_usize(width) + u32_to_usize(x)
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
    use crate::edges;
    use crate::partition;
    use palette_tracer_color::EncodedSrgb;
    use palette_tracer_color::spaces::Oklab;
    use palette_tracer_core::config::{Profile, TraceConfig};
    use palette_tracer_core::control::NoControl;
    use palette_tracer_core::limits::ResourceLimits;

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

    struct Scene {
        graph: RegionGraph,
        labels: LabelPlane,
    }

    fn scene(colors: &[Oklab], claims: &[u32], width: u32, height: u32, pinned: &[bool]) -> Scene {
        let evidence = PixelEvidence {
            color: colors.to_vec(),
            alpha: vec![1.0; colors.len()],
            claim: claims.iter().map(|&c| LabelId(c)).collect(),
            width,
            height,
        };
        let field =
            edges::build(&evidence, &policy(), &WorkBudget::unbounded(), &NoControl).unwrap();
        let part = partition::build(
            &field,
            &policy(),
            &ResourceLimits::default(),
            &WorkBudget::unbounded(),
            &NoControl,
        )
        .unwrap();
        let graph = build(
            &part.labels,
            part.region_count,
            &evidence,
            &field,
            pinned,
            &WorkBudget::unbounded(),
            &NoControl,
        )
        .unwrap();
        Scene {
            graph,
            labels: part.labels,
        }
    }

    /// §25.4 "Segmentation" row: RAG symmetry. If `a` says it borders `b`,
    /// `b` must say the same, with the same boundary length.
    #[test]
    fn adjacency_is_symmetric() {
        let mut colors = Vec::new();
        for y in 0..4u32 {
            for x in 0..4u32 {
                colors.push(if x < 2 && y < 2 {
                    lab([10, 10, 10])
                } else if x >= 2 && y < 2 {
                    lab([250, 10, 10])
                } else {
                    lab([10, 10, 250])
                });
            }
        }
        let s = scene(&colors, &[0; 16], 4, 4, &[false]);

        for a in 0..s.graph.len() as u32 {
            for (&b, adjacency) in s.graph.neighbors(a) {
                let back = s.graph.neighbors(b).get(&a).copied().unwrap_or_default();
                assert_eq!(
                    adjacency.boundary_length, back.boundary_length,
                    "asymmetric adjacency between {a} and {b}"
                );
                assert_eq!(adjacency.weight_sum, back.weight_sum);
            }
        }
    }

    /// PTE-SEG-003 and PTE-NO-007: merging reassigns pixels, it never drops
    /// them.
    #[test]
    fn merging_preserves_every_pixel() {
        let mut colors = Vec::new();
        for y in 0..8u32 {
            for x in 0..8u32 {
                // Three shades close enough that some will merge.
                colors.push(match (x / 3, y / 3) {
                    (0, _) => lab([100, 100, 100]),
                    (1, _) => lab([104, 104, 104]),
                    _ => lab([230, 40, 40]),
                });
            }
        }
        let mut s = scene(&colors, &[0; 64], 8, 8, &[false]);
        merge(
            &mut s.graph,
            &policy(),
            &WorkBudget::unbounded(),
            &NoControl,
        )
        .unwrap();

        let (labels, count) = apply(&s.graph, &s.labels).unwrap();
        let counts = labels.histogram(count as usize);
        assert_eq!(counts.iter().sum::<u64>(), 64);
        assert!(
            counts.iter().all(|&c| c > 0),
            "dense numbering after merging"
        );
    }

    /// PTE-SEG-012: two different pinned identities never merge, however
    /// similar their colours.
    #[test]
    fn distinct_pinned_identities_never_merge() {
        // Two near-identical greys, but different pinned palette claims: the
        // classic two-inks-that-print-the-same case (PTE-COLOR-013).
        let colors = vec![
            lab([128, 128, 128]),
            lab([128, 128, 128]),
            lab([129, 129, 129]),
            lab([129, 129, 129]),
        ];
        let claims = [0u32, 0, 1, 1];
        let mut s = scene(&colors, &claims, 4, 1, &[true, true]);

        // The partition may already have merged them on colour alone; what
        // matters is that where the graph *has* two pinned regions, no merge
        // joins them.
        let before: Vec<LabelId> = (0..s.graph.len() as u32)
            .filter(|&i| s.graph.alive[i as usize])
            .map(|i| s.graph.summary(i).claim)
            .collect();
        merge(
            &mut s.graph,
            &policy(),
            &WorkBudget::unbounded(),
            &NoControl,
        )
        .unwrap();
        let after: Vec<LabelId> = (0..s.graph.len() as u32)
            .filter(|&i| s.graph.alive[i as usize])
            .map(|i| s.graph.summary(i).claim)
            .collect();

        if before.len() == 2 && before[0] != before[1] {
            assert_eq!(
                after.len(),
                2,
                "pinned identities merged: {before:?} -> {after:?}"
            );
        }
    }

    /// PTE-SEG-014: merging must not depend on the order candidates were
    /// generated. Running the same scene repeatedly must give the same result.
    #[test]
    fn merging_is_deterministic() {
        let mut colors = Vec::new();
        for y in 0..6u32 {
            for x in 0..6u32 {
                colors.push(lab([(x * 30) as u8, (y * 30) as u8, 128]));
            }
        }
        let claims = [0u32; 36];

        let run = || {
            let mut s = scene(&colors, &claims, 6, 6, &[false]);
            let outcome = merge(
                &mut s.graph,
                &policy(),
                &WorkBudget::unbounded(),
                &NoControl,
            )
            .unwrap();
            let (labels, count) = apply(&s.graph, &s.labels).unwrap();
            (outcome.merges, count, labels)
        };
        let first = run();
        for _ in 0..4 {
            let again = run();
            assert_eq!(again.0, first.0);
            assert_eq!(again.1, first.1);
            assert_eq!(again.2, first.2);
        }
    }

    /// §8.7: a long one-pixel feature and a single stray pixel have the same
    /// tiny area, and only one of them is noise.
    #[test]
    fn protection_distinguishes_a_hairline_from_a_speck() {
        let speck = RegionSummary {
            area: 1,
            bbox: (0, 0, 0, 0),
            stats: ColorStats::new(),
            mean_alpha: 1.0,
            claim: LabelId::ZERO,
            pinned: false,
            perimeter: 4,
            protection: 0.0,
        };
        let hairline = RegionSummary {
            area: 20,
            bbox: (0, 0, 19, 0),
            stats: ColorStats::new(),
            mean_alpha: 1.0,
            claim: LabelId::ZERO,
            pinned: false,
            perimeter: 42,
            protection: 0.0,
        };

        let speck_score = protection_score(&speck);
        let hairline_score = protection_score(&hairline);
        assert!(
            hairline_score > speck_score,
            "a 20x1 hairline ({hairline_score}) must be protected more than a \
             single pixel ({speck_score})"
        );
        assert!(
            hairline_score > 0.5,
            "the hairline must actually be protected"
        );

        // And a compact blob of the same area as the hairline is not.
        let blob = RegionSummary {
            area: 20,
            bbox: (0, 0, 4, 3),
            perimeter: 18,
            ..hairline.clone()
        };
        assert!(protection_score(&blob) < hairline_score);
    }

    /// §8.6 and PTE-SEG-017: what protects a thin feature is colour evidence,
    /// not shape.
    ///
    /// A one-pixel dark line between two *different* light regions is exactly
    /// the shape §8.6 absorbs -- thin, small, two neighbours -- and must
    /// survive anyway, because its colour is nowhere near the segment joining
    /// its neighbours. This is the guard on removing §8.7's shape veto from
    /// the fringe gate; `topology/one-pixel-bridge` is the same claim end to
    /// end in `make engine-corpus`.
    #[test]
    fn an_antialiased_hairline_is_not_absorbed_because_it_is_not_a_mixture() {
        let left = lab([230, 230, 240]);
        let right = lab([240, 232, 228]);
        let line = lab([16, 16, 20]);

        // left | line | right, the line one pixel wide and four tall.
        let mut colors = Vec::new();
        for _ in 0..4u32 {
            for x in 0..5u32 {
                colors.push(match x {
                    0 | 1 => left,
                    2 => line,
                    _ => right,
                });
            }
        }
        let mut s = scene(&colors, &[0; 20], 5, 4, &[false]);

        let mut policy = policy();
        policy.small_region_pixels = 8;
        policy.fringe_max_residual = 0.2;

        let audits =
            reassign_fringe(&mut s.graph, &policy, &WorkBudget::unbounded(), &NoControl).unwrap();

        assert!(
            audits.iter().all(|a| a.pixels != 4),
            "the four-pixel dark line must not be absorbed as fringe: {audits:?}"
        );

        // And nothing was lost either way (PTE-SEG-003, PTE-NO-007).
        let (labels, count) = apply(&s.graph, &s.labels).unwrap();
        assert_eq!(labels.histogram(count as usize).iter().sum::<u64>(), 20);
    }

    /// §8.6 and PTE-NO-006: a fringe region whose colour is a mixture of its
    /// two neighbours is absorbed, and the decision is recorded.
    #[test]
    fn an_antialias_fringe_is_absorbed_with_an_audit_record() {
        // Dark | fringe | light, with the fringe exactly halfway.
        let dark = lab([20, 20, 20]);
        let light = lab([240, 240, 240]);
        let dark_linear = dark.to_linear();
        let light_linear = light.to_linear();
        let mid = LinearRgb::new(
            (dark_linear.r + light_linear.r) / 2.0,
            (dark_linear.g + light_linear.g) / 2.0,
            (dark_linear.b + light_linear.b) / 2.0,
        )
        .to_oklab();

        let mut colors = Vec::new();
        for _ in 0..4u32 {
            for x in 0..5u32 {
                colors.push(match x {
                    0 | 1 => dark,
                    2 => mid,
                    _ => light,
                });
            }
        }
        let mut s = scene(&colors, &[0; 20], 5, 4, &[false]);

        let mut policy = policy();
        policy.small_region_pixels = 8;
        policy.fringe_max_residual = 0.2;

        let audits = reassign_fringe_to_fixed_point(
            &mut s.graph,
            &policy,
            &WorkBudget::unbounded(),
            &NoControl,
        )
        .unwrap();

        let audit = audits.first().expect("the fringe must be reassigned");
        assert_eq!(audit.pixels, 4, "the fringe column is four pixels");
        assert!(audit.residual <= 0.2, "residual {}", audit.residual);
        assert!(
            (0.0..=1.0).contains(&audit.coverage),
            "coverage {}",
            audit.coverage
        );
        // PTE-SEG-016: the record identifies both sides, so the decision can
        // be reviewed and reversed.
        assert_ne!(audit.source, audit.target);

        let after =
            reassign_fringe(&mut s.graph, &policy, &WorkBudget::unbounded(), &NoControl).unwrap();
        assert!(after.is_empty(), "cleanup must stop at a fixed point");

        // Whatever happened, no pixel was lost.
        let (labels, count) = apply(&s.graph, &s.labels).unwrap();
        assert_eq!(labels.histogram(count as usize).iter().sum::<u64>(), 20);
    }

    /// A merge threshold of zero means no merging, which `pixel-art` needs.
    #[test]
    fn a_zero_threshold_merges_nothing() {
        let colors = vec![lab([10, 10, 10]), lab([11, 11, 11])];
        let mut s = scene(&colors, &[0, 0], 2, 1, &[false]);
        let mut policy = policy();
        policy.merge_threshold = 0.0;
        let outcome = merge(&mut s.graph, &policy, &WorkBudget::unbounded(), &NoControl).unwrap();
        assert_eq!(outcome.merges, 0);
    }

    /// PTE-SEG-013: the queue holds stale entries by design -- a merge
    /// invalidates every queued candidate naming either side -- and popping
    /// one must be detected rather than acted on.
    #[test]
    fn stale_queue_entries_are_detected_not_acted_on() {
        // Widely separated colours so the initial partition keeps them apart
        // and merging has real work to do.
        let mut colors = Vec::new();
        for y in 0..4u32 {
            for x in 0..4u32 {
                colors.push(lab([(x * 60) as u8, (y * 60) as u8, 128]));
            }
        }
        let mut s = scene(&colors, &[0; 16], 4, 4, &[false]);
        let before = s.graph.live_count();

        let mut policy = policy();
        policy.merge_threshold = 8.0; // merge aggressively, forcing stale entries
        let outcome = merge(&mut s.graph, &policy, &WorkBudget::unbounded(), &NoControl).unwrap();
        let after = s.graph.live_count();

        // Whatever the queue did, the partition stayed sound.
        let (labels, count) = apply(&s.graph, &s.labels).unwrap();
        assert_eq!(labels.histogram(count as usize).iter().sum::<u64>(), 16);
        assert_eq!(count, after, "the label plane must name every live region");

        if before > 1 {
            assert!(
                outcome.merges > 0,
                "{before} regions at a threshold of 8 must produce merges"
            );
            assert_eq!(
                after,
                before - outcome.merges,
                "each merge must retire exactly one region"
            );
        }
    }

    #[test]
    fn merging_respects_the_work_budget() {
        let mut colors = Vec::new();
        for i in 0..64u32 {
            colors.push(lab([(i * 3) as u8, 50, 200]));
        }
        let mut s = scene(&colors, &[0; 64], 8, 8, &[false]);
        let mut limits = ResourceLimits::small();
        limits.work_budget = Some(1);
        let budget = WorkBudget::new(&limits);
        let mut policy = policy();
        policy.merge_threshold = 4.0;
        let err = merge(&mut s.graph, &policy, &budget, &NoControl).unwrap_err();
        assert_eq!(err.code(), "resource.work_budget_exhausted");
    }
}
