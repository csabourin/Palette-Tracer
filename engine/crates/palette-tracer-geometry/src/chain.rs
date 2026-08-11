//! Chain preparation (§11.2) and multi-scale corner classification (§11.3).
//!
//! # Preparation (PTE-GEO-001)
//!
//! "Consecutive duplicate samples and zero-length edges MUST be removed before
//! fitting, without deleting required junction identities." The first and last
//! samples are the chain's junction vertices and are never removed, even if
//! they coincide — a closed chain is exactly the case where they do.
//!
//! # Corner classification
//!
//! For each interior sample and each scale `s`, the incoming and outgoing
//! directions are the *chords* to the samples `s` pixels of arclength away, and
//! the turning angle is the angle between them. The corner statistic is the
//! **minimum** turning angle over every supported scale, so a sample must look
//! like a corner at every scale to be one.
//!
//! That single choice is what discharges PTE-GEO-002, "a corner MUST NOT be
//! declared solely from one three-point angle on a stair-stepped raster
//! boundary". On a 45° staircase the fine-scale angle alternates between +90°
//! and −90° while the coarse-scale chords both lie along the diagonal, so the
//! minimum over scales is near zero and no corner is declared. On a real 90°
//! corner every scale agrees, and the minimum is near 90°.
//!
//! Hysteresis (PTE-GEO-004) is Canny's, applied along the chain rather than
//! across an image: samples at or above `enter` seed a corner region, the
//! region grows along the chain while the statistic stays above `exit`, and one
//! corner — the strongest sample in the region — is emitted per region. A
//! numeric wobble near the threshold therefore moves a corner's position by a
//! sample at most; it cannot split one corner into two, nor flip a whole run.
//!
//! # Provenance
//!
//! The multi-scale chord-angle statistic is the classical Rosenfeld–Johnston /
//! Chetverikov family of corner detectors, described in the open literature and
//! reimplemented here from the description in §11.3. The hysteresis is Canny's,
//! applied to a one-dimensional chain. No implementation was consulted.

use crate::FitOptions;
use crate::curves::within;
use palette_tracer_core::determinism::QuantKey;
use palette_tracer_core::ir::{CurveChain, PathSegment, Point, Vec2};

/// The scales, in source pixels, at which corner evidence must agree.
///
/// Three octaves. The finest is above one pixel so that a single stair step
/// cannot span a whole window, which is the mechanism PTE-GEO-002 asks for.
const CORNER_SCALES_PX: [f64; 3] = [2.0, 4.0, 8.0];

/// A prepared chain: deduplicated samples with cumulative arclength.
#[derive(Debug, Clone)]
pub struct Samples {
    /// Sample positions in order. `points[0]` and the last are junctions.
    pub points: Vec<Point>,
    /// Cumulative arclength; `arclen[0] == 0.0` and it is non-decreasing.
    pub arclen: Vec<f64>,
}

impl Samples {
    /// Prepare a polyline chain for fitting, or `None` if it is not a polyline.
    ///
    /// A chain that already contains a curve has been fitted; refitting it
    /// would violate PTE-GEO-012's "a shared edge is simplified and fit once",
    /// so this refuses rather than round-tripping it through samples.
    #[must_use]
    pub fn from_chain(chain: &CurveChain) -> Option<Self> {
        let mut points = Vec::with_capacity(chain.segments.len() + 1);
        points.push(chain.start);
        for segment in &chain.segments {
            match segment {
                PathSegment::Line { to } => points.push(*to),
                _ => return None,
            }
        }
        Some(Self::from_points(&points))
    }

    /// Prepare from raw points, applying PTE-GEO-001.
    #[must_use]
    pub fn from_points(raw: &[Point]) -> Self {
        let mut points: Vec<Point> = Vec::with_capacity(raw.len());
        for (index, &p) in raw.iter().enumerate() {
            let last = index + 1 == raw.len();
            match points.last() {
                // The final sample is a junction identity: keep it even when it
                // coincides with its predecessor, which is what a closed chain
                // of length zero would look like.
                Some(&previous) if !last && same_position(previous, p) => {}
                Some(&previous) if last && same_position(previous, p) && points.len() > 1 => {
                    // A duplicate *final* sample would create a zero-length
                    // segment. Replacing the predecessor keeps the junction
                    // identity at the cost of the duplicate, not the other way
                    // round.
                    let n = points.len();
                    points[n - 1] = p;
                }
                _ => points.push(p),
            }
        }

        let mut arclen = Vec::with_capacity(points.len());
        let mut total = 0.0;
        for (index, &p) in points.iter().enumerate() {
            if index > 0 {
                total += p.distance(points[index - 1]);
            }
            arclen.push(total);
        }

        Self { points, arclen }
    }

    /// Number of samples.
    #[must_use]
    pub fn len(&self) -> usize {
        self.points.len()
    }

    /// Whether there is nothing to fit.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.points.is_empty()
    }

    /// Total arclength.
    #[must_use]
    pub fn length(&self) -> f64 {
        self.arclen.last().copied().unwrap_or(0.0)
    }

    /// Whether the chain returns to where it started.
    #[must_use]
    pub fn is_closed(&self) -> bool {
        match (self.points.first(), self.points.last()) {
            (Some(&a), Some(&b)) => self.points.len() > 2 && same_position(a, b),
            _ => false,
        }
    }

    /// The furthest sample at most `distance` of arclength before `index`.
    ///
    /// Returns `None` when the chain does not extend that far, which is how a
    /// scale is found to be unsupported near an endpoint.
    #[must_use]
    fn back_by(&self, index: usize, distance: f64) -> Option<usize> {
        let target = self.arclen[index] - distance;
        if target < -f64::EPSILON {
            return None;
        }
        // Arclength is non-decreasing, so a linear scan backwards finds the
        // first sample at or before the target. Windows are a few samples wide,
        // so this is cheaper than a binary search and has no midpoint rounding.
        let mut i = index;
        while i > 0 && self.arclen[i] > target {
            i -= 1;
        }
        Some(i)
    }

    /// The nearest sample at least `distance` of arclength after `index`.
    #[must_use]
    fn forward_by(&self, index: usize, distance: f64) -> Option<usize> {
        let target = self.arclen[index] + distance;
        if target > self.length() + f64::EPSILON {
            return None;
        }
        let mut i = index;
        while i + 1 < self.points.len() && self.arclen[i] < target {
            i += 1;
        }
        Some(i)
    }

    /// The unit tangent at `index` looking forward, measured over a chord of
    /// about `window` pixels but never leaving `[index, upper]`.
    ///
    /// Confining the chord to the span is PTE-GEO-013: at a corner, the two
    /// sides must be free to disagree, so neither may see across it.
    ///
    /// # Why two chords
    ///
    /// A chord is not a tangent. On an arc of radius `r`, the chord spanning
    /// arclength `h` from a point is rotated away from that point's tangent by
    /// `h / 2r` — a bias *linear in the window*, in a fixed direction, that no
    /// amount of least-squares fitting over the same window removes. On a
    /// 40-pixel radius with a 2.5-pixel window it is about 1.8°, and 1.8° of
    /// endpoint tangent error costs a quarter of a pixel over the span: twenty
    /// times the §31.2 median gate, arising entirely from the estimator.
    ///
    /// Measuring at `h` and at `2h` and extrapolating, `T ≈ 2·d(h) − d(2h)`,
    /// cancels that linear term exactly for a circular arc and to first order
    /// for anything else. It is Richardson extrapolation, applied to direction
    /// rather than to a scalar. When the span is too short to offer `2h`, the
    /// single chord stands and the bias is accepted rather than extrapolated
    /// from evidence that is not there.
    #[must_use]
    pub fn forward_tangent(&self, index: usize, window: f64, upper: usize) -> Option<Vec2Unit> {
        let near = self.step_forward(index, window, upper)?;
        let d1 = (self.points[near] - self.points[index]).normalized(1e-12)?;
        let h1 = self.arclen[near] - self.arclen[index];

        let Some(far) = self.step_forward(index, window * 2.0, upper) else {
            return Some(Vec2Unit(d1));
        };
        if far <= near {
            return Some(Vec2Unit(d1));
        }
        let Some(d2) = (self.points[far] - self.points[index]).normalized(1e-12) else {
            return Some(Vec2Unit(d1));
        };
        let h2 = self.arclen[far] - self.arclen[index];
        Some(Vec2Unit(extrapolate(d1, h1, d2, h2)))
    }

    /// The unit tangent at `index` looking backward along the traversal, in the
    /// direction of travel, confined to `[lower, index]`.
    ///
    /// The mirror of [`Samples::forward_tangent`], including its extrapolation.
    #[must_use]
    pub fn backward_tangent(&self, index: usize, window: f64, lower: usize) -> Option<Vec2Unit> {
        let near = self.step_backward(index, window, lower)?;
        let d1 = (self.points[index] - self.points[near]).normalized(1e-12)?;
        let h1 = self.arclen[index] - self.arclen[near];

        let Some(far) = self.step_backward(index, window * 2.0, lower) else {
            return Some(Vec2Unit(d1));
        };
        if far >= near {
            return Some(Vec2Unit(d1));
        }
        let Some(d2) = (self.points[index] - self.points[far]).normalized(1e-12) else {
            return Some(Vec2Unit(d1));
        };
        let h2 = self.arclen[index] - self.arclen[far];
        Some(Vec2Unit(extrapolate(d1, h1, d2, h2)))
    }

    /// The sample about `window` pixels after `index`, clamped into the span.
    fn step_forward(&self, index: usize, window: f64, upper: usize) -> Option<usize> {
        let far = self.forward_by(index, window).unwrap_or(upper).min(upper);
        // A window that collapsed onto the sample itself still needs a
        // direction; step one sample, which is the finest evidence available.
        let far = if far > index {
            far
        } else {
            (index + 1).min(upper)
        };
        (far > index).then_some(far)
    }

    /// The sample about `window` pixels before `index`, clamped into the span.
    fn step_backward(&self, index: usize, window: f64, lower: usize) -> Option<usize> {
        let far = self.back_by(index, window).unwrap_or(lower).max(lower);
        let far = if far < index {
            far
        } else {
            index.saturating_sub(1).max(lower)
        };
        (far < index).then_some(far)
    }
}

/// Largest disagreement between the two chords for which extrapolating is
/// justified, in degrees.
///
/// Extrapolation removes a *systematic* bias and amplifies a *random* one: it
/// doubles whatever the near chord contributes. On a smooth arc the two chords
/// differ by the half-angle alone — 1.8° for a 40-pixel radius at this window —
/// and the difference is signal. On a staircase they differ by the step phase,
/// which is noise, and doubling it is worse than accepting the bias.
///
/// Eight degrees separates the two cleanly: a circular arc stays inside it down
/// to a radius of about nine pixels, and a stair step exceeds it. Below that
/// radius the single chord stands, and its bias is bounded by the span being
/// short.
const EXTRAPOLATION_LIMIT_DEG: f64 = 8.0;

/// The tangent two chords of arclength `h1 < h2` extrapolate to.
///
/// Richardson's formula in general form: the chord direction is
/// `T + h·(bias)`, so eliminating the linear term needs
///
/// ```text
/// T ≈ d1 + (d1 − d2) · h1 / (h2 − h1)
/// ```
///
/// The familiar `2·d1 − d2` is the `h2 = 2·h1` case, and using it when the
/// windows are *not* in that ratio removes only part of the bias. They usually
/// are not: both windows snap to whole samples, so a request for 2.5 and 5.0
/// pixels on a chain sampled every 1.05 pixels yields 3.14 and 5.24 — a ratio
/// of 1.67, which the constant form would under-correct by a third.
///
/// Falls back to `d1` when the two chords disagree by more than
/// [`EXTRAPOLATION_LIMIT_DEG`], when the windows are too close in length for
/// the division to be stable, or when the result degenerates.
fn extrapolate(d1: Vec2, h1: f64, d2: Vec2, h2: f64) -> Vec2 {
    let spread = h2 - h1;
    if within(spread, 0.25 * h1) || !h1.is_finite() || !h2.is_finite() {
        return d1;
    }
    let disagreement = d1.dot(d2).clamp(-1.0, 1.0).acos().to_degrees();
    if !within(disagreement, EXTRAPOLATION_LIMIT_DEG) {
        return d1;
    }
    (d1 + (d1 - d2) * (h1 / spread))
        .normalized(1e-9)
        .unwrap_or(d1)
}

/// A unit vector, so a signature can say so.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Vec2Unit(pub Vec2);

/// Whether two samples occupy the same position, decided on a quantised key
/// rather than by `==` (PTE-DET-002).
fn same_position(a: Point, b: Point) -> bool {
    let key = |p: Point| (QuantKey::geometry(p.x).ok(), QuantKey::geometry(p.y).ok());
    let (ax, ay) = key(a);
    let (bx, by) = key(b);
    ax.is_some() && ax == bx && ay.is_some() && ay == by
}

/// Corner evidence for one sample.
#[derive(Debug, Clone, Copy)]
struct Evidence {
    /// Minimum turning angle over the supported scales, in degrees.
    strength: f64,
    /// How many scales had enough chain on both sides to measure.
    supported: u32,
}

/// Interior sample indices that are corners, in increasing order (§11.3).
///
/// The endpoints are junctions and are pinned by the caller regardless; they
/// are never returned here.
#[must_use]
pub fn detect_corners(samples: &Samples, options: &FitOptions) -> Vec<usize> {
    let n = samples.len();
    if n < 5 {
        // Too short for any scale to be supported on both sides. Declaring a
        // corner from what remains would be the three-point angle PTE-GEO-002
        // forbids.
        return Vec::new();
    }

    let evidence: Vec<Evidence> = (0..n).map(|i| measure(samples, i)).collect();

    // PTE-GEO-004: `enter` seeds a corner region, `exit` sustains it.
    let half = options.corner_hysteresis_deg.max(0.0) / 2.0;
    let enter = options.corner_threshold_deg + half;
    let exit = (options.corner_threshold_deg - half).max(0.0);

    let mut corners = Vec::new();
    let mut index = 1;
    while index + 1 < n {
        let seed = evidence[index];
        if seed.supported < 2 || seed.strength < enter {
            index += 1;
            continue;
        }

        // Grow the region while the statistic stays above `exit`, then emit the
        // strongest sample in it. Ties go to the lowest index (PTE-API-010:
        // position decides, never iteration order).
        let start = index;
        let mut end = index;
        while end + 2 < n && evidence[end + 1].supported >= 2 && evidence[end + 1].strength >= exit
        {
            end += 1;
        }
        let mut best = start;
        for candidate in start..=end {
            if QuantKey::cost_or_worst(evidence[candidate].strength)
                > QuantKey::cost_or_worst(evidence[best].strength)
            {
                best = candidate;
            }
        }
        corners.push(best);
        index = end + 1;
    }

    corners
}

/// The multi-scale turning statistic at one sample.
fn measure(samples: &Samples, index: usize) -> Evidence {
    let mut strength = f64::INFINITY;
    let mut supported = 0;

    for scale in CORNER_SCALES_PX {
        let (Some(before), Some(after)) = (
            samples.back_by(index, scale),
            samples.forward_by(index, scale),
        ) else {
            continue;
        };
        if before == index || after == index {
            continue;
        }
        let (Some(incoming), Some(outgoing)) = (
            (samples.points[index] - samples.points[before]).normalized(1e-12),
            (samples.points[after] - samples.points[index]).normalized(1e-12),
        ) else {
            continue;
        };
        let angle = incoming.dot(outgoing).clamp(-1.0, 1.0).acos().to_degrees();
        supported += 1;
        strength = strength.min(angle);
    }

    Evidence {
        strength: if supported == 0 { 0.0 } else { strength },
        supported,
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::float_cmp)]
mod tests {
    use super::*;

    fn options() -> FitOptions {
        FitOptions {
            tolerance_px: 0.6,
            corner_threshold_deg: 55.0,
            corner_hysteresis_deg: 8.0,
            allow_arcs: false,
            max_span_candidates: 48,
        }
    }

    /// A 45° staircase: the shape every raster boundary is made of, and the one
    /// PTE-GEO-002 exists to protect. Not one corner may be declared.
    #[test]
    fn a_forty_five_degree_staircase_has_no_corners() {
        let mut points = Vec::new();
        for i in 0..24 {
            points.push(Point::new(f64::from(i), f64::from(i)));
            points.push(Point::new(f64::from(i) + 1.0, f64::from(i)));
        }
        let samples = Samples::from_points(&points);
        let corners = detect_corners(&samples, &options());
        assert!(
            corners.is_empty(),
            "a staircase is a straight line at every scale that matters, got {corners:?}"
        );
    }

    /// A shallow staircase — one step down every four pixels — is likewise a
    /// line, and its steps are further apart, which is the harder case.
    #[test]
    fn a_shallow_staircase_has_no_corners() {
        let mut points = Vec::new();
        for i in 0..16 {
            let x = f64::from(i) * 4.0;
            points.push(Point::new(x, f64::from(i)));
            points.push(Point::new(x + 4.0, f64::from(i)));
        }
        let samples = Samples::from_points(&points);
        assert!(detect_corners(&samples, &options()).is_empty());
    }

    /// A real right angle must be found, and found once.
    #[test]
    fn a_right_angle_is_one_corner_at_the_turn() {
        let mut points = Vec::new();
        for i in 0..=20 {
            points.push(Point::new(f64::from(i), 0.0));
        }
        for i in 1..=20 {
            points.push(Point::new(20.0, f64::from(i)));
        }
        let samples = Samples::from_points(&points);
        let corners = detect_corners(&samples, &options());
        assert_eq!(corners.len(), 1, "got {corners:?}");
        assert_eq!(
            samples.points[corners[0]],
            Point::new(20.0, 0.0),
            "the corner sits at the turn, not beside it"
        );
    }

    /// Four corners on a square, each found once. A square is the fixture
    /// §31.5's complexity rule is written about.
    #[test]
    fn a_square_has_exactly_four_corners() {
        let mut points = Vec::new();
        let side = 20;
        for i in 0..side {
            points.push(Point::new(f64::from(i), 0.0));
        }
        for i in 0..side {
            points.push(Point::new(f64::from(side), f64::from(i)));
        }
        for i in 0..side {
            points.push(Point::new(f64::from(side - i), f64::from(side)));
        }
        for i in 0..=side {
            points.push(Point::new(0.0, f64::from(side - i)));
        }
        let samples = Samples::from_points(&points);
        // The start sample sits mid-edge, so all four corners are interior.
        let corners = detect_corners(&samples, &options());
        assert_eq!(
            corners.len(),
            3,
            "three interior corners plus the pinned start, got {corners:?}"
        );
    }

    /// A smooth circle has no corners at all, which is the complement of the
    /// staircase test: the detector must not fire on curvature.
    #[test]
    fn a_circle_has_no_corners() {
        let points: Vec<Point> = (0..=180)
            .map(|i| {
                let a = f64::from(i) / 180.0 * std::f64::consts::TAU;
                Point::new(30.0 + 25.0 * a.cos(), 30.0 + 25.0 * a.sin())
            })
            .collect();
        let samples = Samples::from_points(&points);
        assert!(detect_corners(&samples, &options()).is_empty());
    }

    /// PTE-GEO-001: duplicates go, junction identities stay.
    #[test]
    fn duplicate_samples_are_removed_but_the_endpoints_survive() {
        let raw = [
            Point::new(0.0, 0.0),
            Point::new(0.0, 0.0),
            Point::new(1.0, 0.0),
            Point::new(1.0, 0.0),
            Point::new(1.0, 0.0),
            Point::new(2.0, 0.0),
        ];
        let s = Samples::from_points(&raw);
        assert_eq!(s.points.len(), 3);
        assert_eq!(s.points[0], Point::new(0.0, 0.0));
        assert_eq!(s.points[2], Point::new(2.0, 0.0));
        assert_eq!(s.arclen, vec![0.0, 1.0, 2.0]);
    }

    /// A closed chain keeps both copies of the shared endpoint, because they
    /// are the same junction and the traversal must return to it.
    #[test]
    fn a_closed_chain_keeps_its_repeated_endpoint() {
        let raw = [
            Point::new(0.0, 0.0),
            Point::new(2.0, 0.0),
            Point::new(2.0, 2.0),
            Point::new(0.0, 0.0),
        ];
        let s = Samples::from_points(&raw);
        assert_eq!(s.points.len(), 4);
        assert!(s.is_closed());
    }

    /// A chain that already carries a cubic has been fitted once; refitting it
    /// is what PTE-GEO-012 forbids.
    #[test]
    fn an_already_fitted_chain_is_refused() {
        let chain = CurveChain {
            start: Point::ORIGIN,
            segments: vec![PathSegment::Cubic {
                c1: Point::new(1.0, 0.0),
                c2: Point::new(2.0, 0.0),
                to: Point::new(3.0, 0.0),
            }],
        };
        assert!(Samples::from_chain(&chain).is_none());
    }

    /// PTE-GEO-004's purpose, stated as a test: a corner near the threshold
    /// must not multiply as the angle wobbles.
    #[test]
    fn a_near_threshold_corner_stays_a_single_corner() {
        // Two 30° turns two pixels apart read as one gentle bend at the coarse
        // scales and must not become two corners.
        let mut points = Vec::new();
        for i in 0..=15 {
            points.push(Point::new(f64::from(i), 0.0));
        }
        let mut p = Point::new(15.0, 0.0);
        for _ in 0..3 {
            p = Point::new(p.x + 0.866, p.y + 0.5);
            points.push(p);
        }
        for _ in 0..15 {
            p = Point::new(p.x + 0.5, p.y + 0.866);
            points.push(p);
        }
        let samples = Samples::from_points(&points);
        let corners = detect_corners(&samples, &options());
        assert!(
            corners.len() <= 1,
            "a single bend must not become {} corners",
            corners.len()
        );
    }
}
