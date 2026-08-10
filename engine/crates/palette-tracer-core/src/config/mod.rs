//! The configuration model (§6.3) and its expansion into an effective
//! configuration (§3, Appendix G).
//!
//! Two rules shape every type here.
//!
//! **PTE-API-007/008: reject, do not coerce.** Unknown enum values, unknown
//! object keys, out-of-range numbers, NaN and infinity are all configuration
//! errors. `serde(deny_unknown_fields)` handles the keys; [`Validate`] handles
//! the ranges; [`Finite`] handles NaN and infinity at the type level, so a
//! non-finite float cannot exist in a deserialised config at all.
//!
//! **PTE-API-009 and §G.1: one source of defaults.** Every tunable is
//! `Option<T>` in [`TraceConfig`]. A profile fills the `None`s during
//! expansion, and the resolver records where each value came from. The
//! documentation and `pte profiles --json` both read that record, so there is
//! no second hand-written defaults list that can drift.

mod effective;
mod policy;
mod profile;

pub use effective::{
    EffectiveColor, EffectiveConfig, EffectiveGeometry, EffectiveOutput, EffectivePalette,
    EffectivePaletteEntry, EffectiveSegmentation, FieldProvenance, Resolver, SettingSource,
};
pub use policy::{
    AlphaPolicy, BackgroundPolicy, BoundarySource, ColorPolicy, DeltaEScale, DeterminismMode,
    DeterminismPolicy, FabricationPolicy, GeometryMode, GeometryPolicy, GradientMode,
    GradientPolicy, OutputPolicy, PaletteEntrySpec, PaletteMode, PalettePolicy, PaletteRole,
    PixelArtMode, Precision, ReachSpec, SegmentationPolicy, StrokeMode, StrokePolicy,
};
pub use profile::{Modifier, Profile};

use crate::error::ConfigError;
use crate::limits::ResourceLimits;
use serde::{Deserialize, Deserializer, Serialize};

/// Configuration schema version this build implements.
///
/// PTE-API-007: a config declaring a different version is refused rather than
/// interpreted optimistically.
pub const CONFIG_SCHEMA_VERSION: u32 = 1;

/// A `f64` that is guaranteed finite.
///
/// PTE-API-008 requires NaN and infinity to be rejected *at deserialisation*.
/// Making that a property of the type rather than of a validation pass means
/// no later stage has to re-check, and no future field can forget to.
#[derive(Debug, Clone, Copy, PartialEq, PartialOrd, Serialize)]
#[serde(transparent)]
pub struct Finite(f64);

impl Finite {
    /// Wrap `value` if it is finite.
    ///
    /// # Errors
    ///
    /// [`ConfigError::NotFinite`] if `value` is NaN or infinite.
    pub fn new(value: f64, field: &'static str) -> Result<Self, ConfigError> {
        if value.is_finite() {
            Ok(Self(value))
        } else {
            Err(ConfigError::NotFinite { field })
        }
    }

    /// The wrapped value, always finite.
    #[must_use]
    pub const fn get(self) -> f64 {
        self.0
    }
}

impl<'de> Deserialize<'de> for Finite {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let value = f64::deserialize(deserializer)?;
        if value.is_finite() {
            Ok(Self(value))
        } else {
            Err(serde::de::Error::custom(
                "value must be finite: NaN and infinity are rejected (PTE-API-008)",
            ))
        }
    }
}

impl From<Finite> for f64 {
    fn from(value: Finite) -> Self {
        value.0
    }
}

/// Range validation for a policy block.
pub trait Validate {
    /// Check every field against its documented range.
    ///
    /// # Errors
    ///
    /// [`ConfigError::OutOfRange`] naming the first field that fails.
    fn validate(&self) -> Result<(), ConfigError>;
}

/// Check that an optional number lies in an inclusive range.
///
/// # Errors
///
/// [`ConfigError::OutOfRange`] if the value is present and outside.
pub fn check_range(
    field: &'static str,
    value: Option<Finite>,
    min: f64,
    max: f64,
    expected: &'static str,
) -> Result<(), ConfigError> {
    if let Some(v) = value {
        let v = v.get();
        if v < min || v > max {
            return Err(ConfigError::OutOfRange {
                field,
                got: format!("{v}"),
                expected,
            });
        }
    }
    Ok(())
}

/// Check that an optional integer lies in an inclusive range.
///
/// # Errors
///
/// [`ConfigError::OutOfRange`] if the value is present and outside.
pub fn check_u32_range(
    field: &'static str,
    value: Option<u32>,
    min: u32,
    max: u32,
    expected: &'static str,
) -> Result<(), ConfigError> {
    if let Some(v) = value
        && (v < min || v > max)
    {
        return Err(ConfigError::OutOfRange {
            field,
            got: format!("{v}"),
            expected,
        });
    }
    Ok(())
}

/// What the caller asked for.
///
/// Every tunable is optional. `None` means "let the profile decide", and the
/// profile's choice is recorded in [`EffectiveConfig`] with
/// [`SettingSource::Profile`] so the report can distinguish it from a value
/// the caller set (PTE-API-009).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase", default)]
pub struct TraceConfig {
    /// Schema version. Must equal [`CONFIG_SCHEMA_VERSION`].
    pub schema_version: u32,
    /// Primary output profile (§3).
    pub profile: Profile,
    /// Compatible modifiers (§3.1).
    pub modifiers: Vec<Modifier>,
    /// Palette policy (§7.4, §7.6).
    pub palette: PalettePolicy,
    /// Colour management policy (§7.2, §7.3).
    pub color: ColorPolicy,
    /// Segmentation policy (§8).
    pub segmentation: SegmentationPolicy,
    /// Geometry policy (§10, §11).
    pub geometry: GeometryPolicy,
    /// Stroke policy (§13).
    pub strokes: StrokePolicy,
    /// Gradient policy (§14).
    pub gradients: GradientPolicy,
    /// Fabrication policy (§16).
    pub fabrication: FabricationPolicy,
    /// Output and serialisation policy (§18).
    pub output: OutputPolicy,
    /// Resource limits (§24.1).
    pub resources: ResourceLimits,
    /// Determinism policy (§20).
    pub determinism: DeterminismPolicy,
}

impl Default for TraceConfig {
    fn default() -> Self {
        Self {
            schema_version: CONFIG_SCHEMA_VERSION,
            profile: Profile::FlatIllustration,
            modifiers: Vec::new(),
            palette: PalettePolicy::default(),
            color: ColorPolicy::default(),
            segmentation: SegmentationPolicy::default(),
            geometry: GeometryPolicy::default(),
            strokes: StrokePolicy::default(),
            gradients: GradientPolicy::default(),
            fabrication: FabricationPolicy::default(),
            output: OutputPolicy::default(),
            resources: ResourceLimits::default(),
            determinism: DeterminismPolicy::default(),
        }
    }
}

impl TraceConfig {
    /// A configuration for `profile` with every tunable left to the profile.
    #[must_use]
    pub fn for_profile(profile: Profile) -> Self {
        Self {
            profile,
            ..Self::default()
        }
    }

    /// Parse from JSON, rejecting unknown keys, unknown enum values, and
    /// non-finite numbers.
    ///
    /// # Errors
    ///
    /// [`ConfigError::UnknownValue`] with the parser's message. The distinction
    /// between an unknown key and a bad value is preserved in the message
    /// rather than the variant, because `serde_json` reports both the same way
    /// and inventing a classification would be guessing.
    pub fn from_json(text: &str) -> Result<Self, ConfigError> {
        serde_json::from_str(text).map_err(|e| ConfigError::UnknownValue {
            field: "config",
            got: e.to_string(),
        })
    }

    /// Serialise to pretty JSON.
    ///
    /// # Errors
    ///
    /// [`ConfigError::UnknownValue`] if serialisation fails, which can only
    /// happen through a map key that is not a string -- there are none.
    pub fn to_json(&self) -> Result<String, ConfigError> {
        serde_json::to_string_pretty(self).map_err(|e| ConfigError::UnknownValue {
            field: "config",
            got: e.to_string(),
        })
    }

    /// Check ranges and cross-field compatibility, without expanding.
    ///
    /// # Errors
    ///
    /// Any [`ConfigError`]; see [`Validate`] and
    /// [`Profile::check_modifiers`].
    pub fn validate(&self) -> Result<(), ConfigError> {
        if self.schema_version != CONFIG_SCHEMA_VERSION {
            return Err(ConfigError::UnsupportedSchemaVersion {
                got: self.schema_version,
                supported: CONFIG_SCHEMA_VERSION,
            });
        }
        self.palette.validate()?;
        self.color.validate()?;
        self.segmentation.validate()?;
        self.geometry.validate()?;
        self.strokes.validate()?;
        self.gradients.validate()?;
        self.fabrication.validate()?;
        self.output.validate()?;
        self.determinism.validate()?;
        // Cross-field conflicts are checked first: telling a caller that
        // `physical-scale-known` has no scale is more useful than telling them
        // the modifier is unimplemented, and the conflict is true either way.
        self.check_cross_field()?;
        self.profile.check_modifiers(&self.modifiers)?;
        Ok(())
    }

    /// Conflicts that span two policy blocks (PTE-PROFILE-001).
    fn check_cross_field(&self) -> Result<(), ConfigError> {
        if self.modifiers.contains(&Modifier::TransparentBackground)
            && matches!(self.color.background, Some(BackgroundPolicy::Keep))
        {
            return Err(ConfigError::IncompatibleCombination {
                first: "modifier `transparent-background`".into(),
                second: "color.background = keep".into(),
                reason: "one removes the background face and the other retains it",
            });
        }
        if self.modifiers.contains(&Modifier::SharedMosaic)
            && matches!(self.geometry.mode, Some(GeometryMode::Stacked))
        {
            return Err(ConfigError::IncompatibleCombination {
                first: "modifier `shared-mosaic`".into(),
                second: "geometry.mode = stacked".into(),
                reason: "a stacked trace is not a partition (PTE-TOPO-016)",
            });
        }
        if self.modifiers.contains(&Modifier::PhysicalScaleKnown)
            && self.fabrication.physical_width_mm.is_none()
            && self.fabrication.physical_height_mm.is_none()
        {
            return Err(ConfigError::IncompatibleCombination {
                first: "modifier `physical-scale-known`".into(),
                second: "no fabrication.physicalWidthMm or physicalHeightMm".into(),
                reason: "physical operations are meaningless without a scale (PTE-FAB-001)",
            });
        }
        if self.modifiers.contains(&Modifier::ExactPalette)
            && matches!(self.palette.mode, Some(PaletteMode::Automatic))
        {
            return Err(ConfigError::IncompatibleCombination {
                first: "modifier `exact-palette`".into(),
                second: "palette.mode = automatic".into(),
                reason: "an exact palette must be supplied, not derived",
            });
        }
        Ok(())
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

    #[test]
    fn a_default_config_is_valid() {
        TraceConfig::default().validate().unwrap();
    }

    #[test]
    fn round_trips_through_json() {
        let config = TraceConfig::for_profile(Profile::ColoringBook);
        let text = config.to_json().unwrap();
        assert_eq!(TraceConfig::from_json(&text).unwrap(), config);
    }

    /// PTE-API-007: an unknown key is refused, not ignored. Silently
    /// discarding it is exactly PTE-NO-042.
    #[test]
    fn an_unknown_key_is_refused() {
        let err = TraceConfig::from_json(r#"{"schemaVersion":1,"nonsense":true}"#).unwrap_err();
        assert_eq!(err.code(), "config.unknown_value");
        assert!(err.to_string().contains("nonsense"), "{err}");
    }

    #[test]
    fn an_unknown_enum_value_is_refused() {
        let err = TraceConfig::from_json(r#"{"profile":"watercolour"}"#).unwrap_err();
        assert_eq!(err.code(), "config.unknown_value");
    }

    /// PTE-API-008, enforced by the type rather than by a validation pass.
    #[test]
    fn non_finite_numbers_are_refused_at_deserialisation() {
        // An overflowing literal is refused however the JSON parser chooses to
        // describe it -- what matters is that no config carrying it exists.
        for literal in ["1e999", "-1e999"] {
            let json = format!(r#"{{"geometry":{{"curveTolerancePx":{literal}}}}}"#);
            let err = TraceConfig::from_json(&json).unwrap_err();
            assert_eq!(err.code(), "config.unknown_value", "for {literal}");
        }

        // JSON has no literal for infinity, so `serde_json` alone cannot
        // produce one. The `Finite` guard covers every other serde format --
        // and a future one that does -- so it is exercised through a
        // format-independent deserialiser rather than through JSON text.
        use serde::de::IntoDeserializer;
        let d: serde::de::value::F64Deserializer<serde::de::value::Error> =
            f64::INFINITY.into_deserializer();
        let err = Finite::deserialize(d).unwrap_err();
        assert!(err.to_string().contains("finite"), "{err}");

        assert!(Finite::new(f64::NAN, "x").is_err());
        assert!(Finite::new(f64::INFINITY, "x").is_err());
    }

    #[test]
    fn a_future_schema_version_is_refused() {
        let mut config = TraceConfig::default();
        config.schema_version = 99;
        assert_eq!(
            config.validate().unwrap_err(),
            ConfigError::UnsupportedSchemaVersion {
                got: 99,
                supported: 1
            }
        );
    }

    /// PTE-PROFILE-001: conflicting settings produce a typed error and never
    /// an arbitrary winner.
    #[test]
    fn conflicting_settings_name_both_sides() {
        let mut config = TraceConfig::default();
        config.modifiers.push(Modifier::TransparentBackground);
        config.color.background = Some(BackgroundPolicy::Keep);
        let err = config.validate().unwrap_err();
        assert_eq!(err.code(), "config.incompatible_combination");
        let text = err.to_string();
        assert!(text.contains("transparent-background"), "{text}");
        assert!(text.contains("keep"), "{text}");
    }

    #[test]
    fn shared_mosaic_and_stacked_geometry_conflict() {
        let mut config = TraceConfig::default();
        config.modifiers.push(Modifier::SharedMosaic);
        config.geometry.mode = Some(GeometryMode::Stacked);
        assert_eq!(
            config.validate().unwrap_err().code(),
            "config.incompatible_combination"
        );
    }

    #[test]
    fn physical_scale_without_a_size_conflicts() {
        let mut config = TraceConfig::default();
        config.modifiers.push(Modifier::PhysicalScaleKnown);
        assert_eq!(
            config.validate().unwrap_err().code(),
            "config.incompatible_combination"
        );
    }

    #[test]
    fn an_out_of_range_number_names_its_field_and_range() {
        let mut config = TraceConfig::default();
        config.geometry.curve_tolerance_px = Some(Finite::new(-1.0, "t").unwrap());
        let err = config.validate().unwrap_err();
        match err {
            ConfigError::OutOfRange { field, .. } => {
                assert_eq!(field, "geometry.curveTolerancePx");
            }
            other => panic!("expected OutOfRange, got {other}"),
        }
    }
}
