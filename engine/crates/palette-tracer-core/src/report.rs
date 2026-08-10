//! The machine-readable trace report (§6.5, Appendix A).
//!
//! PTE-GOAL-008 requires "structured diagnostics and a machine-readable report
//! explaining important decisions, confidence, warnings, effective settings,
//! and measured output properties", and PTE-NO-050 forbids optimising away
//! the diagnostics that explain a fallback.
//!
//! Two rules keep this honest:
//!
//! * Every warning and fallback carries a stable code (PTE-API-012). Prose is
//!   for humans; the code is the contract.
//! * Nothing here is a claim. Counts are measured from the produced document,
//!   and the fields for metrics this build does not compute are absent rather
//!   than zero -- a zero would read as "no error".

use crate::config::EffectiveConfig;
use crate::image::{AlphaMode, ColorEncoding, PixelFormat};
use crate::labels::LabelWidth;
use serde::{Deserialize, Serialize};
use std::borrow::Cow;

/// Report schema version.
pub const REPORT_SCHEMA_VERSION: u32 = 1;

/// Severity of a diagnostic.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Severity {
    /// Worth knowing; output is as requested.
    Info,
    /// Output differs from the ideal in a way the caller should see.
    Warning,
}

/// One diagnostic.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Warning {
    /// Stable machine-readable code (PTE-API-012).
    pub code: Cow<'static, str>,
    /// Severity.
    pub severity: Severity,
    /// Human-readable detail. May change between versions.
    pub message: String,
    /// Requirement this relates to, when one governs it.
    pub requirement: Option<Cow<'static, str>>,
}

impl Warning {
    /// A warning.
    #[must_use]
    pub fn warn(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            code: Cow::Borrowed(code),
            severity: Severity::Warning,
            message: message.into(),
            requirement: None,
        }
    }

    /// An informational note.
    #[must_use]
    pub fn info(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            code: Cow::Borrowed(code),
            severity: Severity::Info,
            message: message.into(),
            requirement: None,
        }
    }

    /// Attach the governing requirement identifier.
    #[must_use]
    pub fn about(mut self, requirement: &'static str) -> Self {
        self.requirement = Some(Cow::Borrowed(requirement));
        self
    }
}

/// A documented downgrade (§E.3).
///
/// Every downgrade records what was requested, what was done, and why. §E.3:
/// "Only caller-authorized downgrades are legal."
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Fallback {
    /// Stable machine-readable code.
    pub code: Cow<'static, str>,
    /// What the caller asked for.
    pub requested: String,
    /// What was actually done.
    pub effective: String,
    /// Why.
    pub reason: String,
}

/// Which algorithm version produced each stage's output (PTE-DET-004).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AlgorithmVersions {
    /// Segmentation algorithm.
    pub segmentation: Cow<'static, str>,
    /// Topology construction.
    pub topology: Cow<'static, str>,
    /// Boundary reconstruction.
    pub boundary: Cow<'static, str>,
    /// Curve fitting.
    pub curves: Cow<'static, str>,
}

impl Default for AlgorithmVersions {
    fn default() -> Self {
        Self {
            segmentation: Cow::Borrowed("pte-watershed-rag/1"),
            topology: Cow::Borrowed("pte-dcel/1"),
            boundary: Cow::Borrowed("pte-coverage/1"),
            curves: Cow::Borrowed("pte-hybrid-fit/1"),
        }
    }
}

/// Engine identity (Appendix A).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EngineInfo {
    /// Engine name.
    pub name: Cow<'static, str>,
    /// Engine version.
    pub version: Cow<'static, str>,
    /// Compilation target triple.
    pub target: Cow<'static, str>,
    /// Enabled output-affecting features.
    pub features: Vec<Cow<'static, str>>,
    /// Per-stage algorithm versions.
    pub algorithms: AlgorithmVersions,
}

impl Default for EngineInfo {
    fn default() -> Self {
        Self {
            name: Cow::Borrowed(crate::ENGINE_NAME),
            version: Cow::Borrowed(crate::ENGINE_VERSION),
            target: Cow::Borrowed(TARGET),
            features: Vec::new(),
            algorithms: AlgorithmVersions::default(),
        }
    }
}

/// The compilation target, as far as the engine can tell without a build
/// script.
///
/// `std::env::consts` gives the OS and architecture; that is enough to
/// distinguish native from WASM, which is what PTE-DET-001's cross-target
/// contract cares about.
const TARGET: &str = {
    if cfg!(target_arch = "wasm32") {
        "wasm32-unknown-unknown"
    } else if cfg!(target_arch = "x86_64") {
        "x86_64"
    } else if cfg!(target_arch = "aarch64") {
        "aarch64"
    } else {
        "unknown"
    }
};

/// What the input was and how it was interpreted.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InputInfo {
    /// Width in pixels.
    pub width: u32,
    /// Height in pixels.
    pub height: u32,
    /// Pixel layout.
    pub format: PixelFormat,
    /// Declared transfer function.
    pub encoding: ColorEncoding,
    /// Declared alpha association.
    pub alpha_mode: AlphaMode,
    /// What was done about a colour profile (PTE-COLOR-005).
    pub profile_action: Cow<'static, str>,
}

/// One palette entry as it was actually used.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PaletteReportEntry {
    /// Stable identifier.
    pub id: u32,
    /// Output colour, `#rrggbb`.
    pub color: String,
    /// Whether the entry was pinned.
    pub pinned: bool,
    /// Pixels claimed by this entry.
    pub pixels: u64,
    /// Regions that ended up owned by this entry.
    pub regions: u32,
}

/// Segmentation outcome.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SegmentationReport {
    /// Regions in the initial partition, before merging.
    pub initial_regions: u32,
    /// Regions after merging.
    pub final_regions: u32,
    /// Merges performed.
    pub merges: u32,
    /// Regions reassigned as antialias fringe (§8.6).
    pub fringe_reassignments: u32,
    /// Pixels moved by fringe reassignment.
    pub fringe_pixels: u64,
    /// Regions kept because the §8.7 protection score said so.
    pub protected_regions: u32,
    /// Storage width of the label plane (§7.5).
    pub label_width: Option<LabelWidth>,
}

/// Topology outcome (§26.3).
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TopologyReport {
    /// Interior faces.
    pub faces: u32,
    /// Shared edges.
    pub shared_edges: u32,
    /// Junction vertices.
    pub junctions: u32,
    /// Holes across all faces.
    pub holes: u32,
    /// Ambiguous 2x2 cells encountered (§9.4).
    pub ambiguous_cells: u32,
    /// Of those, how many the continuity term decided.
    pub ambiguous_resolved_by_continuity: u32,
    /// Pixels with no owner. Must be zero for a shared partition (§31.1).
    pub unassigned_pixels: u64,
}

/// Representation outcome (§26.7).
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RepresentationReport {
    /// Visible paths.
    pub paths: u32,
    /// Line segments emitted.
    pub lines: u32,
    /// Cubic segments emitted.
    pub cubics: u32,
    /// Arc segments emitted.
    pub arcs: u32,
    /// Recognised primitives.
    pub primitives: u32,
    /// Stroked paths.
    pub strokes: u32,
    /// Gradients.
    pub gradients: u32,
    /// Gradient stops.
    pub gradient_stops: u32,
    /// Total control points.
    pub control_points: u32,
    /// Chains that fell back to a validated polyline (§11.6).
    pub polyline_fallbacks: u32,
    /// Serialised bytes.
    pub svg_bytes: u64,
    /// Decimal places used (§18.3).
    pub coordinate_decimals: u8,
}

/// Boundary-source census (PTE-AA-009).
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BoundarySourceReport {
    /// Edges positioned by inverting antialias coverage.
    pub coverage_reconstructed: u32,
    /// Edges left on the pixel-cell interface.
    pub crisp_grid: u32,
    /// Edges where coverage was attempted and rejected.
    pub low_confidence_fallback: u32,
    /// Edges decided by pixel-art connectivity policy.
    pub pixel_art_policy: u32,
}

/// Resource consumption (§26.8).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ResourceUsage {
    /// Abstract work units charged.
    pub work_units: u64,
    /// The configured budget, if any.
    pub work_budget: Option<u64>,
    /// Highest working-memory figure the engine recorded.
    ///
    /// This is the engine's own accounting of the planes it allocated, not an
    /// allocator trace. PTE-PERF-002's lifetime proof is not satisfied by it.
    pub peak_working_bytes: u64,
}

/// The trace report (Appendix A).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TraceReport {
    /// Report schema version.
    pub schema_version: u32,
    /// Engine identity and algorithm versions.
    pub engine: EngineInfo,
    /// Input description.
    pub input: InputInfo,
    /// Fully expanded configuration (PTE-PROFILE-002).
    pub effective_config: EffectiveConfig,
    /// Palette as used.
    pub palette: Vec<PaletteReportEntry>,
    /// Segmentation outcome.
    pub segmentation: SegmentationReport,
    /// Topology outcome.
    pub topology: TopologyReport,
    /// Representation outcome.
    pub representation: RepresentationReport,
    /// Boundary-source census.
    pub boundary_sources: BoundarySourceReport,
    /// Resource consumption.
    pub resources: ResourceUsage,
    /// Diagnostics.
    pub warnings: Vec<Warning>,
    /// Downgrades taken.
    pub fallbacks: Vec<Fallback>,
    /// Requirements this build does not implement and that the request
    /// touched.
    ///
    /// §0.2's evidence rule in report form: the caller is told what the engine
    /// did *not* do, rather than being left to infer it from silence.
    pub unimplemented: Vec<Cow<'static, str>>,
    /// Semantic digest, `pte-semantic-v1-blake3:<hex>` (Appendix F.4).
    pub semantic_digest: String,
}

impl TraceReport {
    /// Serialise to pretty JSON.
    ///
    /// # Panics
    ///
    /// Never; every field is plain data with string keys.
    #[must_use]
    pub fn to_json(&self) -> String {
        serde_json::to_string_pretty(self).unwrap_or_else(|_| "{}".into())
    }

    /// Add a diagnostic.
    pub fn warn(&mut self, warning: Warning) {
        self.warnings.push(warning);
    }

    /// Whether any diagnostic has [`Severity::Warning`].
    #[must_use]
    pub fn has_warnings(&self) -> bool {
        self.warnings
            .iter()
            .any(|w| w.severity == Severity::Warning)
    }
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
    use crate::config::{Profile, TraceConfig};

    fn report() -> TraceReport {
        TraceReport {
            schema_version: REPORT_SCHEMA_VERSION,
            engine: EngineInfo::default(),
            input: InputInfo {
                width: 4,
                height: 4,
                format: PixelFormat::Rgba8,
                encoding: ColorEncoding::Srgb,
                alpha_mode: AlphaMode::Straight,
                profile_action: Cow::Borrowed("assumed-srgb"),
            },
            effective_config: Profile::expand(&TraceConfig::default()).unwrap(),
            palette: Vec::new(),
            segmentation: SegmentationReport::default(),
            topology: TopologyReport::default(),
            representation: RepresentationReport::default(),
            boundary_sources: BoundarySourceReport::default(),
            resources: ResourceUsage::default(),
            warnings: Vec::new(),
            fallbacks: Vec::new(),
            unimplemented: Vec::new(),
            semantic_digest: "pte-semantic-v1-blake3:00".into(),
        }
    }

    /// Appendix A fixes the top-level shape. A consumer keying on these names
    /// must not break because a field was renamed.
    #[test]
    fn the_report_has_the_appendix_a_top_level_keys() {
        let json: serde_json::Value = serde_json::from_str(&report().to_json()).unwrap();
        for key in [
            "schemaVersion",
            "engine",
            "input",
            "effectiveConfig",
            "palette",
            "segmentation",
            "topology",
            "representation",
            "resources",
            "warnings",
            "fallbacks",
            "semanticDigest",
        ] {
            assert!(json.get(key).is_some(), "missing `{key}`");
        }
    }

    #[test]
    fn round_trips_through_json() {
        let r = report();
        let parsed: TraceReport = serde_json::from_str(&r.to_json()).unwrap();
        assert_eq!(parsed, r);
    }

    /// PTE-API-012: the code is the contract, the message is not.
    #[test]
    fn warnings_carry_codes_and_optional_requirements() {
        let mut r = report();
        assert!(!r.has_warnings());
        r.warn(Warning::info(
            "palette.assumed_srgb",
            "no profile was present",
        ));
        assert!(!r.has_warnings(), "info is not a warning");
        r.warn(
            Warning::warn("geometry.polyline_fallback", "chain 4 fell back").about("PTE-GEO-005"),
        );
        assert!(r.has_warnings());
        assert_eq!(r.warnings[1].requirement.as_deref(), Some("PTE-GEO-005"));
    }

    /// §E.3: a downgrade names what was requested and what happened.
    #[test]
    fn a_fallback_records_both_sides() {
        let mut r = report();
        r.fallbacks.push(Fallback {
            code: Cow::Borrowed("output.precision_increased"),
            requested: "4 decimals".into(),
            effective: "6 decimals".into(),
            reason: "4 decimals moved a boundary outside the error budget".into(),
        });
        let json = r.to_json();
        assert!(json.contains("precision_increased"));
        assert!(json.contains("4 decimals"));
    }

    #[test]
    fn algorithm_versions_are_reported_for_every_stage() {
        let a = AlgorithmVersions::default();
        for v in [&a.segmentation, &a.topology, &a.boundary, &a.curves] {
            assert!(v.contains('/'), "`{v}` must carry a version suffix");
        }
    }
}
