//! The expanded configuration and its provenance record.
//!
//! §G.1 requires `pte profiles --json` to return "every effective field, its
//! value, source (`profile`, `user`, `auto-analysis`, or `fallback`), profile
//! schema version, and valid range", and adds that documentation "MUST use
//! this output rather than maintain a second hand-written default list that
//! can drift".
//!
//! [`Resolver`] is how that is guaranteed rather than promised: expansion is
//! the *only* place a default is written down, and every default passes
//! through [`Resolver::pick`], which records it. A field that skipped the
//! resolver would be missing from `pte profiles --json`, and
//! `profiles_json_lists_every_effective_field` fails.

use super::policy::{
    AlphaPolicy, BackgroundPolicy, BoundarySource, DeltaEScale, DeterminismMode, GeometryMode,
    GradientMode, PaletteMode, PaletteRole, PixelArtMode, Precision, ReachSpec, StrokeMode,
};
use super::profile::{Modifier, Profile};
use crate::limits::ResourceLimits;
use serde::{Deserialize, Serialize};
use std::borrow::Cow;
use serde_json::Value;
use std::collections::BTreeMap;

/// Where an effective value came from (§G.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
#[non_exhaustive]
pub enum SettingSource {
    /// The profile's declared default.
    Profile,
    /// The caller set it explicitly.
    User,
    /// Chosen from image evidence, and therefore also reported as an automatic
    /// decision (PTE-API-009).
    AutoAnalysis,
    /// A documented fallback taken after something else failed or was
    /// unavailable.
    Fallback,
}

/// One row of the effective-settings table.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FieldProvenance {
    /// The value, as it appears in the report.
    pub value: Value,
    /// Where it came from.
    pub source: SettingSource,
    /// The documented valid range, in prose.
    ///
    /// `Cow` rather than `&'static str` so the report can be deserialised as
    /// well as produced; construction from a literal still allocates nothing.
    pub range: Cow<'static, str>,
}

/// Records every default as it is chosen.
#[derive(Debug, Default)]
pub struct Resolver {
    fields: BTreeMap<String, FieldProvenance>,
}

impl Resolver {
    /// Resolve one field: take the caller's value if present, otherwise the
    /// profile's, and record which happened.
    ///
    /// # Panics
    ///
    /// If `value` cannot be serialised to JSON. Every type passed here is a
    /// plain enum, number, or bool, none of which can fail.
    pub fn pick<T: Serialize + Clone>(
        &mut self,
        path: &'static str,
        user: Option<T>,
        profile_default: T,
        range: &'static str,
    ) -> T {
        let (value, source) = match user {
            Some(v) => (v, SettingSource::User),
            None => (profile_default, SettingSource::Profile),
        };
        self.fields.insert(
            path.to_owned(),
            FieldProvenance {
                value: serde_json::to_value(value.clone())
                    .unwrap_or(Value::String("<unserialisable>".into())),
                source,
                range: Cow::Borrowed(range),
            },
        );
        value
    }

    /// Record a value the engine chose from image evidence (PTE-API-009).
    ///
    /// # Panics
    ///
    /// Never; see [`Resolver::pick`].
    pub fn record_auto<T: Serialize>(&mut self, path: &str, value: &T, range: &'static str) {
        self.fields.insert(
            path.to_owned(),
            FieldProvenance {
                value: serde_json::to_value(value).unwrap_or(Value::Null),
                source: SettingSource::AutoAnalysis,
                range: Cow::Borrowed(range),
            },
        );
    }

    /// Record a documented fallback.
    ///
    /// # Panics
    ///
    /// Never; see [`Resolver::pick`].
    pub fn record_fallback<T: Serialize>(&mut self, path: &str, value: &T, range: &'static str) {
        self.fields.insert(
            path.to_owned(),
            FieldProvenance {
                value: serde_json::to_value(value).unwrap_or(Value::Null),
                source: SettingSource::Fallback,
                range: Cow::Borrowed(range),
            },
        );
    }

    /// Consume the resolver, yielding the recorded table.
    #[must_use]
    pub fn into_fields(self) -> BTreeMap<String, FieldProvenance> {
        self.fields
    }
}

/// A palette entry with every optional field resolved.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EffectivePaletteEntry {
    /// Stable identifier.
    pub id: u32,
    /// Straight 8-bit RGBA, parsed from the declared hex string. This is what
    /// exact mode emits, unchanged (PTE-SVG-012).
    pub rgba: [u8; 4],
    /// Whether the entry is pinned.
    pub pinned: bool,
    /// Resolved reach.
    pub reach: ReachSpec,
    /// Declared role.
    pub role: PaletteRole,
    /// Tie-break priority.
    pub priority: i16,
}

/// Resolved palette settings.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EffectivePalette {
    /// Fixed or automatic.
    pub mode: PaletteMode,
    /// Entries, empty when the palette is derived.
    pub entries: Vec<EffectivePaletteEntry>,
    /// Target entry count for automatic palettes.
    pub max_colors: u32,
}

/// Resolved colour settings.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EffectiveColor {
    /// Assume sRGB when untagged.
    pub assume_srgb_when_untagged: bool,
    /// Chroma below which hue loses weight.
    pub neutral_chroma_threshold: f64,
    /// Reporting convention for `ΔE_OK`.
    pub delta_e_scale: DeltaEScale,
    /// Background handling.
    pub background: BackgroundPolicy,
    /// Alpha handling.
    pub alpha: AlphaPolicy,
}

/// Resolved segmentation settings.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EffectiveSegmentation {
    /// `λ_c` in the §8.2 edge cost.
    pub edge_weight_color: f64,
    /// `λ_g` in the §8.2 edge cost.
    pub edge_weight_gradient: f64,
    /// `λ_α` in the §8.2 edge cost.
    pub edge_weight_alpha: f64,
    /// `λ_r` in the §8.2 edge cost.
    pub edge_weight_claim: f64,
    /// Merge cost ceiling (§8.5).
    pub merge_threshold: f64,
    /// Reassignment candidate size (PTE-SEG-003).
    pub small_region_pixels: u32,
    /// Fringe mixture residual ceiling (§8.6).
    pub fringe_max_residual: f64,
    /// Whether the §8.7 protection score applies.
    pub protect_thin_features: bool,
}

/// Resolved geometry settings.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EffectiveGeometry {
    /// Partition semantics.
    pub mode: GeometryMode,
    /// Boundary error budget, in source pixels.
    pub curve_tolerance_px: f64,
    /// Corner turning-angle threshold, in degrees.
    pub corner_threshold_deg: f64,
    /// Hysteresis band, in degrees.
    pub corner_hysteresis_deg: f64,
    /// Whether arcs may be emitted.
    pub allow_arcs: bool,
    /// Boundary evidence policy.
    pub boundary_source: BoundarySource,
    /// Pixel-art mode.
    pub pixel_art_mode: PixelArtMode,
    /// Candidate-span cap per chain position.
    pub max_span_candidates: u32,
}

/// Resolved output settings.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EffectiveOutput {
    /// Coordinate precision policy.
    pub precision: Precision,
    /// Whether `<metadata>` is emitted.
    pub metadata: bool,
    /// Whether output is indented.
    pub pretty: bool,
    /// Whether the §31.1 output gates run.
    pub validate_output: bool,
}

/// A fully expanded configuration.
///
/// PTE-PROFILE-002: this, including every default the profile supplied, is
/// what the report carries.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EffectiveConfig {
    /// Config schema version.
    pub schema_version: u32,
    /// Version of the profile's frozen defaults (§G, "A profile version
    /// freezes its expanded defaults").
    pub profile_schema_version: u32,
    /// Primary profile.
    pub profile: Profile,
    /// Active modifiers, in canonical order.
    pub modifiers: Vec<Modifier>,
    /// Resolved palette settings.
    pub palette: EffectivePalette,
    /// Resolved colour settings.
    pub color: EffectiveColor,
    /// Resolved segmentation settings.
    pub segmentation: EffectiveSegmentation,
    /// Resolved geometry settings.
    pub geometry: EffectiveGeometry,
    /// Resolved stroke mode. Only [`StrokeMode::Off`] is implemented in this
    /// build; anything else was refused during validation.
    pub strokes: StrokeMode,
    /// Resolved gradient mode. Only [`GradientMode::Off`] is implemented.
    pub gradients: GradientMode,
    /// Resolved output settings.
    pub output: EffectiveOutput,
    /// Resource limits.
    pub resources: ResourceLimits,
    /// Determinism level.
    pub determinism: DeterminismMode,
    /// Every effective field with its value, source, and range (§G.1).
    pub provenance: BTreeMap<String, FieldProvenance>,
}

impl EffectiveConfig {
    /// Serialise to pretty JSON, for `--print-effective-config`.
    ///
    /// # Panics
    ///
    /// Never; every field is a plain data type.
    #[must_use]
    pub fn to_json(&self) -> String {
        serde_json::to_string_pretty(self).unwrap_or_else(|_| "{}".into())
    }

    /// Whether `modifier` is active.
    #[must_use]
    pub fn has(&self, modifier: Modifier) -> bool {
        self.modifiers.contains(&modifier)
    }
}
