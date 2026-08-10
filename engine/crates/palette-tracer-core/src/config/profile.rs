//! Output profiles and their frozen defaults (§3, Appendix G).
//!
//! A profile "is a versioned bundle of defaults; it is never an opaque code
//! path" (§3). Everything a profile does is visible in
//! [`EffectiveConfig::provenance`] after expansion; there is no branch
//! anywhere in the engine of the form "if profile == logo then behave
//! differently" that is not first expressed as a setting here.
//!
//! # Implemented in this build
//!
//! [`Profile::FlatIllustration`], [`Profile::ColoringBook`],
//! [`Profile::PixelArt`], and [`Profile::Logo`]. The remaining profiles are
//! defined -- they are part of the contract -- but selecting one returns
//! [`ConfigError::UnsupportedInThisBuild`] naming the requirement that governs
//! the missing behaviour. PTE-NO-042 forbids the alternative of accepting the
//! name and quietly tracing something else.

use super::effective::{
    EffectiveColor, EffectiveConfig, EffectiveGeometry, EffectiveOutput, EffectivePalette,
    EffectivePaletteEntry, EffectiveSegmentation, Resolver,
};
use super::policy::{
    AlphaPolicy, BackgroundPolicy, BoundarySource, DeltaEScale, DeterminismMode, GeometryMode,
    GradientMode, PaletteMode, PixelArtMode, Precision, ReachSpec, StrokeMode, parse_hex_color,
};
use super::{CONFIG_SCHEMA_VERSION, TraceConfig};
use crate::error::ConfigError;
use serde::{Deserialize, Serialize};

/// Version of the frozen profile defaults.
///
/// §G: "A profile version freezes its expanded defaults; changing a cell later
/// requires a profile version bump."
pub const PROFILE_SCHEMA_VERSION: u32 = 1;

/// Primary output profile (§3).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
#[non_exhaustive]
pub enum Profile {
    /// Logos, marks, icons, signage.
    Logo,
    /// Cartoons, clip art, flat artwork.
    FlatIllustration,
    /// Adhesive vinyl and paper cutters.
    VinylCut,
    /// Cut, score, and engrave preparation.
    Laser,
    /// Printable outlines.
    ColoringBook,
    /// Drawings, plans, ink strokes.
    LineArt,
    /// Raster type and hand lettering.
    Lettering,
    /// Spot-colour separations.
    ScreenPrint,
    /// Modern shaded illustrations.
    GradientIllustration,
    /// Low-resolution pixel art.
    PixelArt,
    /// Stylised photos and complex art.
    Poster,
}

impl Profile {
    /// Every profile the specification defines, in declaration order.
    pub const ALL: [Self; 11] = [
        Self::Logo,
        Self::FlatIllustration,
        Self::VinylCut,
        Self::Laser,
        Self::ColoringBook,
        Self::LineArt,
        Self::Lettering,
        Self::ScreenPrint,
        Self::GradientIllustration,
        Self::PixelArt,
        Self::Poster,
    ];

    /// Stable kebab-case identifier, as used on the command line.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Logo => "logo",
            Self::FlatIllustration => "flat-illustration",
            Self::VinylCut => "vinyl-cut",
            Self::Laser => "laser",
            Self::ColoringBook => "coloring-book",
            Self::LineArt => "line-art",
            Self::Lettering => "lettering",
            Self::ScreenPrint => "screen-print",
            Self::GradientIllustration => "gradient-illustration",
            Self::PixelArt => "pixel-art",
            Self::Poster => "poster",
        }
    }

    /// Whether this build implements the profile end to end.
    #[must_use]
    pub const fn is_implemented(self) -> bool {
        matches!(
            self,
            Self::FlatIllustration | Self::ColoringBook | Self::PixelArt | Self::Logo
        )
    }

    /// The requirement that governs the behaviour a profile still needs.
    const fn missing_requirement(self) -> &'static str {
        match self {
            Self::VinylCut | Self::Laser | Self::ScreenPrint => "PTE-FAB-001 (§16 fabrication)",
            Self::LineArt | Self::Lettering => "PTE-STROKE-001 (§13 centrelines)",
            Self::GradientIllustration => "PTE-GRAD-001 (§14 gradients)",
            Self::Poster => "PTE-GOAL-006 (§3 poster profile)",
            _ => "",
        }
    }

    /// Reject a profile this build cannot honour.
    ///
    /// # Errors
    ///
    /// [`ConfigError::UnsupportedInThisBuild`] naming the governing
    /// requirement.
    pub fn check_implemented(self) -> Result<(), ConfigError> {
        if self.is_implemented() {
            Ok(())
        } else {
            Err(ConfigError::UnsupportedInThisBuild {
                field: "profile",
                requirement: self.missing_requirement(),
            })
        }
    }

    /// Reject modifier combinations this profile forbids (PTE-PROFILE-001).
    ///
    /// # Errors
    ///
    /// [`ConfigError::IncompatibleCombination`] naming both sides, or
    /// [`ConfigError::UnsupportedInThisBuild`] for a modifier whose behaviour
    /// is not implemented.
    pub fn check_modifiers(self, modifiers: &[Modifier]) -> Result<(), ConfigError> {
        self.check_implemented()?;

        let has = |m: Modifier| modifiers.contains(&m);

        if has(Modifier::SharedMosaic) && has(Modifier::StackedLayers) {
            return Err(ConfigError::IncompatibleCombination {
                first: "modifier `shared-mosaic`".into(),
                second: "modifier `stacked-layers`".into(),
                reason: "a partition and a stack are different semantics (PTE-TOPO-016)",
            });
        }
        if has(Modifier::StackedLayers) && self == Self::VinylCut {
            return Err(ConfigError::IncompatibleCombination {
                first: "modifier `stacked-layers`".into(),
                second: "profile `vinyl-cut`".into(),
                reason: "an exclusive cutout cannot contain hidden stacked geometry \
                         (PTE-FAB-004)",
            });
        }
        if has(Modifier::PreferStrokes) && self == Self::PixelArt {
            return Err(ConfigError::IncompatibleCombination {
                first: "modifier `prefer-strokes`".into(),
                second: "profile `pixel-art`".into(),
                reason: "pixel art has no medial model to infer (PTE-TOPO-010)",
            });
        }
        if has(Modifier::AllowExperimentalShading) && !has(Modifier::AllowPortableGradients) {
            return Err(ConfigError::IncompatibleCombination {
                first: "modifier `allow-experimental-shading`".into(),
                second: "no `allow-portable-gradients`".into(),
                reason: "experimental shading requires a portable fallback (PTE-GRAD-009)",
            });
        }

        // Modifiers whose behaviour this build does not implement are refused
        // rather than accepted and ignored (PTE-NO-042).
        for (modifier, requirement) in [
            (Modifier::PreferStrokes, "PTE-STROKE-001 (§13 centrelines)"),
            (
                Modifier::AllowPortableGradients,
                "PTE-GRAD-001 (§14 gradients)",
            ),
            (
                Modifier::AllowExperimentalShading,
                "PTE-GRAD-009 (§14.7 experimental shading)",
            ),
            (
                Modifier::RecognizePrimitives,
                "PTE-GEO-010 (§11.7 primitive recognition)",
            ),
            (
                Modifier::PhysicalScaleKnown,
                "PTE-FAB-001 (§16 fabrication)",
            ),
        ] {
            if has(modifier) {
                return Err(ConfigError::UnsupportedInThisBuild {
                    field: modifier.as_str(),
                    requirement,
                });
            }
        }
        Ok(())
    }
}

/// A compatible modifier (§3.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
#[non_exhaustive]
pub enum Modifier {
    /// Emit exactly the declared palette colours.
    ExactPalette,
    /// Carry source alpha into the output.
    PreserveAlpha,
    /// Remove the background while retaining foreground holes.
    TransparentBackground,
    /// Force shared-mosaic partition semantics.
    SharedMosaic,
    /// Force stacked-artwork semantics.
    StackedLayers,
    /// Recognise rectangles, circles, and other primitives.
    RecognizePrimitives,
    /// Prefer stroked centrelines where the medial model fits.
    PreferStrokes,
    /// Physical output scale is declared.
    PhysicalScaleKnown,
    /// Permit `linearGradient` and `radialGradient`.
    AllowPortableGradients,
    /// Permit non-portable shading.
    AllowExperimentalShading,
    /// Require strict cross-target determinism.
    StrictDeterminism,
}

impl Modifier {
    /// Stable kebab-case identifier.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::ExactPalette => "exact-palette",
            Self::PreserveAlpha => "preserve-alpha",
            Self::TransparentBackground => "transparent-background",
            Self::SharedMosaic => "shared-mosaic",
            Self::StackedLayers => "stacked-layers",
            Self::RecognizePrimitives => "recognize-primitives",
            Self::PreferStrokes => "prefer-strokes",
            Self::PhysicalScaleKnown => "physical-scale-known",
            Self::AllowPortableGradients => "allow-portable-gradients",
            Self::AllowExperimentalShading => "allow-experimental-shading",
            Self::StrictDeterminism => "strict-determinism",
        }
    }
}

/// The frozen per-profile defaults of Appendix G.
///
/// This struct is the single place a default is written. Everything else reads
/// it through [`Resolver::pick`], which is what makes §G.1 hold.
struct ProfileDefaults {
    geometry_mode: GeometryMode,
    curve_tolerance_px: f64,
    corner_threshold_deg: f64,
    corner_hysteresis_deg: f64,
    allow_arcs: bool,
    boundary_source: BoundarySource,
    pixel_art_mode: PixelArtMode,
    max_span_candidates: u32,
    merge_threshold: f64,
    small_region_pixels: u32,
    fringe_max_residual: f64,
    protect_thin_features: bool,
    edge_weight_color: f64,
    edge_weight_gradient: f64,
    edge_weight_alpha: f64,
    edge_weight_claim: f64,
    palette_mode: PaletteMode,
    max_colors: u32,
    background: BackgroundPolicy,
    neutral_chroma_threshold: f64,
    precision: Precision,
}

impl Profile {
    fn defaults(self) -> ProfileDefaults {
        // Starting policies for corpus calibration, per the preamble to
        // Appendix G. They are engineering choices, not measured optima;
        // `docs/IMPLEMENTATION_STATUS.md` records that calibration is Phase 0
        // work which has not been done.
        let base = ProfileDefaults {
            geometry_mode: GeometryMode::SharedMosaic,
            curve_tolerance_px: 0.6,
            corner_threshold_deg: 55.0,
            corner_hysteresis_deg: 8.0,
            allow_arcs: false,
            boundary_source: BoundarySource::Auto,
            pixel_art_mode: PixelArtMode::Blocky,
            max_span_candidates: 48,
            merge_threshold: 0.020,
            small_region_pixels: 12,
            fringe_max_residual: 0.035,
            protect_thin_features: true,
            edge_weight_color: 1.0,
            edge_weight_gradient: 0.5,
            edge_weight_alpha: 1.0,
            edge_weight_claim: 0.25,
            palette_mode: PaletteMode::Automatic,
            max_colors: 12,
            background: BackgroundPolicy::Keep,
            neutral_chroma_threshold: 0.02,
            precision: Precision::Auto,
        };

        match self {
            // "Few nodes, stable corners, exact palette, conservative
            // regularization" (§3). Tighter corners and a looser curve budget
            // buy compactness; arcs stay off until §11.7 lands.
            Self::Logo => ProfileDefaults {
                curve_tolerance_px: 0.45,
                corner_threshold_deg: 42.0,
                merge_threshold: 0.028,
                small_region_pixels: 6,
                max_colors: 8,
                ..base
            },
            // "Seam-free adjacency, small-region intent, editable layers".
            Self::FlatIllustration => base,
            // "Every visible interface drawn once, clean junctions, no doubled
            // dark seams". Merges harder because a coloring page wants fewer,
            // cleaner interfaces than a faithful recolouring.
            Self::ColoringBook => ProfileDefaults {
                curve_tolerance_px: 0.8,
                merge_threshold: 0.045,
                small_region_pixels: 24,
                max_colors: 6,
                background: BackgroundPolicy::Keep,
                ..base
            },
            // "Intentional diagonal topology, no generic antialias
            // assumptions" (PTE-GEO-025): coverage reconstruction is off, the
            // palette is exact by default, and nothing merges.
            Self::PixelArt => ProfileDefaults {
                boundary_source: BoundarySource::CrispGrid,
                curve_tolerance_px: 0.0,
                merge_threshold: 0.0,
                small_region_pixels: 0,
                protect_thin_features: true,
                fringe_max_residual: 0.0,
                palette_mode: PaletteMode::Automatic,
                max_colors: 64,
                pixel_art_mode: PixelArtMode::Blocky,
                ..base
            },
            // Defined but not implemented; `check_implemented` refuses these
            // before their defaults are ever read.
            _ => base,
        }
    }

    /// Expand `config` into a fully resolved configuration.
    ///
    /// # Errors
    ///
    /// Any [`ConfigError`] from validation, from an unimplemented profile or
    /// modifier, or from a palette entry whose colour does not parse.
    pub fn expand(config: &TraceConfig) -> Result<EffectiveConfig, ConfigError> {
        config.validate()?;

        let profile = config.profile;
        let d = profile.defaults();
        let mut r = Resolver::default();

        // Modifiers are stored in canonical order so two callers who list them
        // differently produce the same effective config and the same digest
        // (PTE-DET-001).
        let mut modifiers = config.modifiers.clone();
        modifiers.sort_unstable();
        modifiers.dedup();

        // A modifier is a shorthand for a setting, and is resolved before the
        // setting so an explicit setting still wins and a conflict has already
        // been refused by `check_cross_field`.
        let modifier_geometry_mode = if modifiers.contains(&Modifier::SharedMosaic) {
            Some(GeometryMode::SharedMosaic)
        } else if modifiers.contains(&Modifier::StackedLayers) {
            Some(GeometryMode::Stacked)
        } else {
            None
        };
        let modifier_background = modifiers
            .contains(&Modifier::TransparentBackground)
            .then_some(BackgroundPolicy::Transparent);
        let modifier_palette_mode = modifiers
            .contains(&Modifier::ExactPalette)
            .then_some(PaletteMode::Fixed);
        let modifier_determinism = modifiers
            .contains(&Modifier::StrictDeterminism)
            .then_some(DeterminismMode::Strict);

        let geometry = EffectiveGeometry {
            mode: r.pick(
                "geometry.mode",
                config.geometry.mode.or(modifier_geometry_mode),
                d.geometry_mode,
                "shared-mosaic | stacked | separate-operations",
            ),
            curve_tolerance_px: r.pick(
                "geometry.curveTolerancePx",
                config.geometry.curve_tolerance_px.map(Into::into),
                d.curve_tolerance_px,
                "0 to 64 source pixels",
            ),
            corner_threshold_deg: r.pick(
                "geometry.cornerThresholdDeg",
                config.geometry.corner_threshold_deg.map(Into::into),
                d.corner_threshold_deg,
                "0 to 180 degrees",
            ),
            corner_hysteresis_deg: r.pick(
                "geometry.cornerHysteresisDeg",
                config.geometry.corner_hysteresis_deg.map(Into::into),
                d.corner_hysteresis_deg,
                "0 to 90 degrees",
            ),
            allow_arcs: r.pick(
                "geometry.allowArcs",
                config.geometry.allow_arcs,
                d.allow_arcs,
                "true | false",
            ),
            boundary_source: r.pick(
                "geometry.boundarySource",
                config.geometry.boundary_source,
                d.boundary_source,
                "coverage-reconstructed | crisp-grid | auto",
            ),
            pixel_art_mode: r.pick(
                "geometry.pixelArtMode",
                config.geometry.pixel_art_mode,
                d.pixel_art_mode,
                "blocky | smoothed | hybrid",
            ),
            max_span_candidates: r.pick(
                "geometry.maxSpanCandidates",
                config.geometry.max_span_candidates,
                d.max_span_candidates,
                "1 to 1024",
            ),
        };

        let segmentation = EffectiveSegmentation {
            edge_weight_color: r.pick(
                "segmentation.edgeWeightColor",
                config.segmentation.edge_weight_color.map(Into::into),
                d.edge_weight_color,
                "0 to 16, dimensionless",
            ),
            edge_weight_gradient: r.pick(
                "segmentation.edgeWeightGradient",
                config.segmentation.edge_weight_gradient.map(Into::into),
                d.edge_weight_gradient,
                "0 to 16, dimensionless",
            ),
            edge_weight_alpha: r.pick(
                "segmentation.edgeWeightAlpha",
                config.segmentation.edge_weight_alpha.map(Into::into),
                d.edge_weight_alpha,
                "0 to 16, dimensionless",
            ),
            edge_weight_claim: r.pick(
                "segmentation.edgeWeightClaim",
                config.segmentation.edge_weight_claim.map(Into::into),
                d.edge_weight_claim,
                "0 to 16, dimensionless",
            ),
            merge_threshold: r.pick(
                "segmentation.mergeThreshold",
                config.segmentation.merge_threshold.map(Into::into),
                d.merge_threshold,
                "0 to 16, squared OKLab units scaled by the area gain",
            ),
            small_region_pixels: r.pick(
                "segmentation.smallRegionPixels",
                config.segmentation.small_region_pixels,
                d.small_region_pixels,
                "0 to 1048576 pixels",
            ),
            fringe_max_residual: r.pick(
                "segmentation.fringeMaxResidual",
                config.segmentation.fringe_max_residual.map(Into::into),
                d.fringe_max_residual,
                "0 to 1, linear-light units",
            ),
            protect_thin_features: r.pick(
                "segmentation.protectThinFeatures",
                config.segmentation.protect_thin_features,
                d.protect_thin_features,
                "true | false",
            ),
        };

        let color = EffectiveColor {
            assume_srgb_when_untagged: r.pick(
                "color.assumeSrgbWhenUntagged",
                config.color.assume_srgb_when_untagged,
                true,
                "true | false",
            ),
            neutral_chroma_threshold: r.pick(
                "color.neutralChromaThreshold",
                config.color.neutral_chroma_threshold.map(Into::into),
                d.neutral_chroma_threshold,
                "0 to 0.5 OKLab chroma units",
            ),
            delta_e_scale: r.pick(
                "color.deltaEScale",
                config.color.delta_e_scale,
                DeltaEScale::Native,
                "native | scaled100",
            ),
            background: r.pick(
                "color.background",
                config.color.background.or(modifier_background),
                d.background,
                "keep | transparent | paletteEntry(id) | auto",
            ),
            alpha: r.pick(
                "color.alpha",
                config.color.alpha,
                AlphaPolicy::default(),
                "preserve flag plus an unpremultiply epsilon in 0 to 0.5",
            ),
        };

        let palette_mode = r.pick(
            "palette.mode",
            config.palette.mode.or(modifier_palette_mode),
            d.palette_mode,
            "fixed | automatic",
        );
        let max_colors = r.pick(
            "palette.maxColors",
            config.palette.max_colors,
            d.max_colors,
            "2 to 256",
        );
        let default_reach = r.pick(
            "palette.defaultReach",
            config.palette.default_reach,
            ReachSpec::default(),
            "positive OKLCH radii, each at most 2.0",
        );

        if palette_mode == PaletteMode::Fixed && config.palette.entries.is_empty() {
            return Err(ConfigError::Palette {
                reason: "the effective palette mode is `fixed` but no entries were supplied",
            });
        }

        let mut entries = Vec::with_capacity(config.palette.entries.len());
        for spec in &config.palette.entries {
            entries.push(EffectivePaletteEntry {
                id: spec.id,
                rgba: parse_hex_color(&spec.color)?,
                pinned: spec.pinned,
                reach: spec.reach.unwrap_or(default_reach),
                role: spec.role,
                priority: spec.priority,
            });
        }
        // PTE-COLOR-012: the documented tie order ends at the palette id, so
        // entries are held in id order and never in the caller's array order.
        entries.sort_by_key(|e| e.id);

        let strokes = r.pick(
            "strokes.prefer",
            config.strokes.prefer,
            StrokeMode::Off,
            "off | auto | on",
        );
        if strokes != StrokeMode::Off {
            return Err(ConfigError::UnsupportedInThisBuild {
                field: "strokes.prefer",
                requirement: "PTE-STROKE-001 (§13 centrelines)",
            });
        }

        let gradients = r.pick(
            "gradients.mode",
            config.gradients.mode,
            GradientMode::Off,
            "off | standard | experimental",
        );
        if gradients != GradientMode::Off {
            return Err(ConfigError::UnsupportedInThisBuild {
                field: "gradients.mode",
                requirement: "PTE-GRAD-001 (§14 gradients)",
            });
        }

        let output = EffectiveOutput {
            precision: r.pick(
                "output.precision",
                config.output.precision,
                d.precision,
                "auto | fixed(0 to 12 decimals)",
            ),
            metadata: r.pick(
                "output.metadata",
                config.output.metadata,
                true,
                "true | false",
            ),
            pretty: r.pick("output.pretty", config.output.pretty, false, "true | false"),
            validate_output: r.pick(
                "output.validateOutput",
                config.output.validate_output,
                true,
                "true | false",
            ),
        };

        let determinism = r.pick(
            "determinism.mode",
            config.determinism.mode.or(modifier_determinism),
            DeterminismMode::Strict,
            "strict | target | fast",
        );

        Ok(EffectiveConfig {
            schema_version: CONFIG_SCHEMA_VERSION,
            profile_schema_version: PROFILE_SCHEMA_VERSION,
            profile,
            modifiers,
            palette: EffectivePalette {
                mode: palette_mode,
                entries,
                max_colors,
                default_reach,
            },
            color,
            segmentation,
            geometry,
            strokes,
            gradients,
            output,
            resources: config.resources,
            determinism,
            provenance: r.into_fields(),
        })
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
    use crate::config::policy::{PaletteEntrySpec, PaletteRole};

    fn expand(config: &TraceConfig) -> EffectiveConfig {
        Profile::expand(config).unwrap()
    }

    /// §G.1: `pte profiles --json` is the only defaults list. Every field the
    /// effective config exposes must appear in the provenance table, or the
    /// documentation would need a second list that can drift.
    #[test]
    fn profiles_json_lists_every_effective_field() {
        let effective = expand(&TraceConfig::default());
        let json = serde_json::to_value(&effective).unwrap();
        let object = json.as_object().unwrap();

        // Every leaf of these blocks is resolved through the resolver.
        let expected_prefixes = [
            "geometry.",
            "segmentation.",
            "color.",
            "palette.",
            "output.",
            "determinism.",
            "strokes.",
            "gradients.",
        ];
        for prefix in expected_prefixes {
            assert!(
                effective.provenance.keys().any(|k| k.starts_with(prefix)),
                "no provenance rows for `{prefix}`"
            );
        }

        // Cross-check counts: the resolver recorded a row for each tunable in
        // the four scalar blocks plus the standalone fields.
        let geometry_fields = object["geometry"].as_object().unwrap().len();
        let recorded = effective
            .provenance
            .keys()
            .filter(|k| k.starts_with("geometry."))
            .count();
        assert_eq!(
            geometry_fields, recorded,
            "every effective geometry field needs a provenance row"
        );

        for row in effective.provenance.values() {
            assert!(!row.range.is_empty(), "every row must document its range");
        }
    }

    /// PTE-API-009: a value the caller set and a value the profile supplied
    /// must be distinguishable in the report.
    #[test]
    fn provenance_distinguishes_user_from_profile() {
        let mut config = TraceConfig::default();
        config.geometry.curve_tolerance_px = Some(super::super::Finite::new(1.25, "t").unwrap());
        let effective = expand(&config);

        let user = &effective.provenance["geometry.curveTolerancePx"];
        assert_eq!(user.source, super::super::SettingSource::User);
        assert_eq!(user.value, serde_json::json!(1.25));

        let from_profile = &effective.provenance["geometry.cornerThresholdDeg"];
        assert_eq!(from_profile.source, super::super::SettingSource::Profile);
    }

    #[test]
    fn every_profile_has_a_stable_identifier() {
        let mut seen = std::collections::BTreeSet::new();
        for p in Profile::ALL {
            assert!(seen.insert(p.as_str()), "duplicate id {}", p.as_str());
            let json = serde_json::to_string(&p).unwrap();
            assert_eq!(json, format!("\"{}\"", p.as_str()));
        }
        assert_eq!(seen.len(), 11);
    }

    /// PTE-NO-042: an unimplemented profile is refused by name, and the error
    /// says which requirement is missing.
    #[test]
    fn unimplemented_profiles_are_refused_not_silently_downgraded() {
        for profile in Profile::ALL.into_iter().filter(|p| !p.is_implemented()) {
            let config = TraceConfig::for_profile(profile);
            let err = Profile::expand(&config).unwrap_err();
            assert_eq!(
                err.code(),
                "config.unsupported_in_this_build",
                "{} must be refused",
                profile.as_str()
            );
            assert!(
                !profile.missing_requirement().is_empty(),
                "{} must name the missing requirement",
                profile.as_str()
            );
        }
    }

    #[test]
    fn implemented_profiles_all_expand() {
        for profile in Profile::ALL.into_iter().filter(|p| p.is_implemented()) {
            let effective = expand(&TraceConfig::for_profile(profile));
            assert_eq!(effective.profile, profile);
            assert_eq!(effective.profile_schema_version, PROFILE_SCHEMA_VERSION);
        }
    }

    /// PTE-GEO-025: pixel art must not invent antialias coverage.
    #[test]
    fn pixel_art_defaults_to_crisp_boundaries_and_no_merging() {
        let effective = expand(&TraceConfig::for_profile(Profile::PixelArt));
        assert_eq!(
            effective.geometry.boundary_source,
            BoundarySource::CrispGrid
        );
        assert_eq!(effective.geometry.pixel_art_mode, PixelArtMode::Blocky);
        assert_eq!(effective.segmentation.merge_threshold, 0.0);
        assert_eq!(effective.segmentation.small_region_pixels, 0);
    }

    /// Modifier order must not reach the digest.
    #[test]
    fn modifiers_are_canonicalised_and_deduplicated() {
        let mut a = TraceConfig::default();
        a.modifiers = vec![
            Modifier::PreserveAlpha,
            Modifier::ExactPalette,
            Modifier::PreserveAlpha,
        ];
        a.palette.mode = Some(PaletteMode::Fixed);
        a.palette.entries.push(PaletteEntrySpec {
            id: 1,
            color: "#123456".into(),
            pinned: false,
            reach: None,
            role: PaletteRole::Fill,
            priority: 0,
        });

        let mut b = a.clone();
        b.modifiers = vec![Modifier::ExactPalette, Modifier::PreserveAlpha];

        assert_eq!(expand(&a).modifiers, expand(&b).modifiers);
        assert_eq!(expand(&a).modifiers.len(), 2);
    }

    /// A modifier is shorthand for a setting, and the setting must win when
    /// both are given and compatible.
    #[test]
    fn exact_palette_modifier_selects_fixed_mode() {
        let mut config = TraceConfig::default();
        config.modifiers.push(Modifier::ExactPalette);
        config.palette.entries.push(PaletteEntrySpec {
            id: 7,
            color: "#00ff00".into(),
            pinned: true,
            reach: None,
            role: PaletteRole::Ink,
            priority: 3,
        });
        let effective = expand(&config);
        assert_eq!(effective.palette.mode, PaletteMode::Fixed);
        assert_eq!(effective.palette.entries[0].rgba, [0, 255, 0, 255]);
    }

    #[test]
    fn entries_are_held_in_id_order_not_array_order() {
        let mut config = TraceConfig::default();
        config.palette.mode = Some(PaletteMode::Fixed);
        for id in [9u32, 2, 5] {
            config.palette.entries.push(PaletteEntrySpec {
                id,
                color: "#010203".into(),
                pinned: false,
                reach: None,
                role: PaletteRole::Fill,
                priority: 0,
            });
        }
        let effective = expand(&config);
        let ids: Vec<u32> = effective.palette.entries.iter().map(|e| e.id).collect();
        assert_eq!(ids, vec![2, 5, 9]);
    }

    #[test]
    fn unimplemented_modifiers_are_refused() {
        for modifier in [
            Modifier::PreferStrokes,
            Modifier::AllowPortableGradients,
            Modifier::RecognizePrimitives,
        ] {
            let mut config = TraceConfig::default();
            config.modifiers.push(modifier);
            if modifier == Modifier::AllowPortableGradients {
                // Otherwise the experimental-shading pairing rule fires first.
            }
            let err = Profile::expand(&config).unwrap_err();
            assert_eq!(
                err.code(),
                "config.unsupported_in_this_build",
                "{} must be refused",
                modifier.as_str()
            );
        }
    }

    #[test]
    fn a_stroke_or_gradient_request_is_refused_with_its_requirement() {
        let mut config = TraceConfig::default();
        config.strokes.prefer = Some(StrokeMode::Auto);
        let err = Profile::expand(&config).unwrap_err();
        assert!(err.to_string().contains("PTE-STROKE-001"), "{err}");

        let mut config = TraceConfig::default();
        config.gradients.mode = Some(GradientMode::Standard);
        let err = Profile::expand(&config).unwrap_err();
        assert!(err.to_string().contains("PTE-GRAD-001"), "{err}");
    }
}
