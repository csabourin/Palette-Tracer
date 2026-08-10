//! Policy blocks: the tunables, their ranges, and their units.
//!
//! PTE-SEG-011 asks for the merge objective to be "dimensionally documented"
//! and forbids thresholds with different physical dimensions sharing one
//! unexplained slider. Every field here therefore carries its unit in its name
//! (`_px`, `_mm`, `_deg`) or is a dimensionless normalised score with the
//! range stated in its documentation.

use super::{Finite, Validate, check_range, check_u32_range};
use crate::error::ConfigError;
use serde::{Deserialize, Serialize};

/// How the palette is obtained (§7.4, §7.6).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
#[non_exhaustive]
pub enum PaletteMode {
    /// Use exactly the supplied entries (PTE-GOAL-002).
    Fixed,
    /// Derive entries deterministically from the image (PTE-COLOR-018).
    Automatic,
}

/// What a palette entry is for (§7.4).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
#[non_exhaustive]
pub enum PaletteRole {
    /// An ordinary fill colour.
    Fill,
    /// The background, subject to [`BackgroundPolicy`].
    Background,
    /// A named ink or tool. Distinct roles stay distinct even at equal RGB
    /// (PTE-COLOR-013).
    Ink,
    /// A line or outline colour.
    Line,
}

/// Per-entry colour reach, as anisotropic OKLCH tolerances (§7.4).
///
/// Each radius is the distance at which the normalised score reaches 1, so an
/// entry claims a sample when the weighted sum of squared normalised
/// differences is at most 1.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct ReachSpec {
    /// Lightness radius, in OKLab `L` units (`L` spans `[0, 1]`).
    pub lightness: Finite,
    /// Chroma radius, in OKLab chroma units.
    pub chroma: Finite,
    /// Hue radius, as an OKLab chord length at the sample's chroma. Expressed
    /// as a chord rather than an angle because an angular tolerance means
    /// something different at every chroma (PTE-COLOR-011).
    pub hue: Finite,
}

impl Default for ReachSpec {
    fn default() -> Self {
        // A moderate default reach. Calibration against the reference corpus
        // is Phase 0 work that has not been done; these are starting values,
        // not measured ones.
        Self {
            lightness: Finite(0.08),
            chroma: Finite(0.06),
            hue: Finite(0.06),
        }
    }
}

impl Validate for ReachSpec {
    fn validate(&self) -> Result<(), ConfigError> {
        check_range(
            "palette.entries[].reach.lightness",
            Some(self.lightness),
            1e-6,
            2.0,
            "a positive lightness radius at most 2.0",
        )?;
        check_range(
            "palette.entries[].reach.chroma",
            Some(self.chroma),
            1e-6,
            2.0,
            "a positive chroma radius at most 2.0",
        )?;
        check_range(
            "palette.entries[].reach.hue",
            Some(self.hue),
            1e-6,
            2.0,
            "a positive hue chord radius at most 2.0",
        )?;
        Ok(())
    }
}

/// One user-declared palette entry.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct PaletteEntrySpec {
    /// Stable identifier. Two entries may share an output colour but never an
    /// identifier (PTE-COLOR-013).
    pub id: u32,
    /// Output colour as `#rrggbb` or `#rrggbbaa`. Emitted verbatim in exact
    /// mode; the internal OKLab anchor is never substituted (PTE-SVG-012).
    pub color: String,
    /// A pinned entry never moves and never merges away (PTE-COLOR-019,
    /// PTE-SEG-012).
    #[serde(default)]
    pub pinned: bool,
    /// Reach, or the profile default.
    #[serde(default)]
    pub reach: Option<ReachSpec>,
    /// What the entry is for.
    #[serde(default = "default_role")]
    pub role: PaletteRole,
    /// Tie-break priority; higher wins. Consulted after the normalised score
    /// and before the identifier (PTE-COLOR-012).
    #[serde(default)]
    pub priority: i16,
}

fn default_role() -> PaletteRole {
    PaletteRole::Fill
}

/// Palette policy.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase", default)]
pub struct PalettePolicy {
    /// Fixed or automatic.
    pub mode: Option<PaletteMode>,
    /// Entries, for [`PaletteMode::Fixed`].
    pub entries: Vec<PaletteEntrySpec>,
    /// Target entry count, for [`PaletteMode::Automatic`].
    pub max_colors: Option<u32>,
    /// Default reach applied to entries that declare none.
    pub default_reach: Option<ReachSpec>,
}

impl Validate for PalettePolicy {
    fn validate(&self) -> Result<(), ConfigError> {
        check_u32_range(
            "palette.maxColors",
            self.max_colors,
            2,
            256,
            "between 2 and 256 colours",
        )?;
        if let Some(r) = &self.default_reach {
            r.validate()?;
        }
        let mut seen: Vec<u32> = self.entries.iter().map(|e| e.id).collect();
        seen.sort_unstable();
        if seen.windows(2).any(|w| w[0] == w[1]) {
            return Err(ConfigError::Palette {
                reason: "two entries share an id; ids must be distinct (PTE-COLOR-013)",
            });
        }
        if matches!(self.mode, Some(PaletteMode::Fixed)) && self.entries.is_empty() {
            return Err(ConfigError::Palette {
                reason: "palette.mode is `fixed` but no entries were supplied",
            });
        }
        for entry in &self.entries {
            parse_hex_color(&entry.color)?;
            if let Some(r) = &entry.reach {
                r.validate()?;
            }
        }
        Ok(())
    }
}

/// Parse `#rgb`, `#rrggbb`, or `#rrggbbaa` into straight 8-bit RGBA.
///
/// # Errors
///
/// [`ConfigError::Palette`] for any other shape. The engine does not accept
/// named CSS colours: a palette is an exact instruction, and resolving names
/// would put a colour table between the caller and their own inks.
pub fn parse_hex_color(text: &str) -> Result<[u8; 4], ConfigError> {
    const BAD: ConfigError = ConfigError::Palette {
        reason: "colour must be #rgb, #rrggbb, or #rrggbbaa",
    };
    let hex = text.strip_prefix('#').ok_or(BAD)?;
    let nibble = |c: u8| -> Result<u8, ConfigError> {
        match c {
            b'0'..=b'9' => Ok(c - b'0'),
            b'a'..=b'f' => Ok(c - b'a' + 10),
            b'A'..=b'F' => Ok(c - b'A' + 10),
            _ => Err(BAD),
        }
    };
    let b = hex.as_bytes();
    match b.len() {
        3 => {
            let mut out = [255u8; 4];
            for i in 0..3 {
                let v = nibble(b[i])?;
                out[i] = v * 17;
            }
            Ok(out)
        }
        6 | 8 => {
            let mut out = [255u8; 4];
            for i in 0..b.len() / 2 {
                out[i] = nibble(b[2 * i])? * 16 + nibble(b[2 * i + 1])?;
            }
            Ok(out)
        }
        _ => Err(BAD),
    }
}

/// Which numeric convention `ΔE_OK` is reported in (PTE-COLOR-004).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
#[non_exhaustive]
pub enum DeltaEScale {
    /// Native OKLab units, where `L` spans `[0, 1]`.
    Native,
    /// Multiplied by 100, the convention some tools display.
    Scaled100,
}

/// What happens to the background (§17.3).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
#[non_exhaustive]
pub enum BackgroundPolicy {
    /// Retain the background as an ordinary face.
    Keep,
    /// Make it transparent while retaining foreground holes (PTE-COLOR-023).
    Transparent,
    /// Treat the entry with this identifier as the removable background.
    PaletteEntry(u32),
    /// Choose from image evidence and report the choice.
    Auto,
}

/// Alpha handling (§7.3).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct AlphaPolicy {
    /// Preserve source alpha in the output. Distinct from background removal
    /// (PTE-COLOR-010).
    pub preserve: bool,
    /// Alpha below this is treated as fully transparent when unpremultiplying,
    /// so a near-zero divisor cannot amplify arbitrary colour into palette
    /// evidence (PTE-COLOR-009).
    pub unpremultiply_epsilon: Finite,
}

impl Default for AlphaPolicy {
    fn default() -> Self {
        Self {
            preserve: true,
            // One half of an 8-bit step. Below this, the stored colour carries
            // less than one code point of information.
            unpremultiply_epsilon: Finite(1.0 / 510.0),
        }
    }
}

/// Colour management policy (§7.2, §7.3).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase", default)]
pub struct ColorPolicy {
    /// Assume sRGB when the caller reports no embedded profile. The assumption
    /// is recorded in the report either way (PTE-COLOR-005).
    pub assume_srgb_when_untagged: Option<bool>,
    /// Chroma below which hue becomes powerless. The hue term's weight decays
    /// smoothly to zero rather than switching off (PTE-COLOR-011).
    pub neutral_chroma_threshold: Option<Finite>,
    /// Reporting convention for `ΔE_OK`.
    pub delta_e_scale: Option<DeltaEScale>,
    /// Background policy.
    pub background: Option<BackgroundPolicy>,
    /// Alpha policy.
    pub alpha: Option<AlphaPolicy>,
}

impl Validate for ColorPolicy {
    fn validate(&self) -> Result<(), ConfigError> {
        check_range(
            "color.neutralChromaThreshold",
            self.neutral_chroma_threshold,
            0.0,
            0.5,
            "between 0 and 0.5 OKLab chroma units",
        )?;
        if let Some(a) = self.alpha {
            check_range(
                "color.alpha.unpremultiplyEpsilon",
                Some(a.unpremultiply_epsilon),
                0.0,
                0.5,
                "between 0 and 0.5",
            )?;
        }
        Ok(())
    }
}

/// Segmentation policy (§8).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase", default)]
pub struct SegmentationPolicy {
    /// Weight of perceptual colour difference in the edge cost (§8.2, `λ_c`).
    /// Dimensionless; the term it multiplies is normalised to `[0, 1]`.
    pub edge_weight_color: Option<Finite>,
    /// Weight of the multi-scale gradient response (§8.2, `λ_g`).
    pub edge_weight_gradient: Option<Finite>,
    /// Weight of the alpha discontinuity term (§8.2, `λ_α`).
    pub edge_weight_alpha: Option<Finite>,
    /// Weight of palette-claim disagreement (§8.2, `λ_r`).
    pub edge_weight_claim: Option<Finite>,
    /// Merge cost above which two regions stay separate (§8.5). In squared
    /// OKLab units scaled by the area gain, so it is comparable across image
    /// sizes.
    pub merge_threshold: Option<Finite>,
    /// Regions below this many pixels are candidates for reassignment -- never
    /// for deletion (PTE-SEG-003, PTE-NO-007).
    pub small_region_pixels: Option<u32>,
    /// Largest mixture residual, in linear-light units, at which a thin region
    /// is accepted as antialias fringe rather than an intended colour
    /// (§8.6, PTE-NO-006).
    pub fringe_max_residual: Option<Finite>,
    /// Protect thin features from cleanup using the §8.7 score rather than
    /// area alone (PTE-SEG-017).
    pub protect_thin_features: Option<bool>,
}

impl Validate for SegmentationPolicy {
    fn validate(&self) -> Result<(), ConfigError> {
        for (field, value) in [
            ("segmentation.edgeWeightColor", self.edge_weight_color),
            ("segmentation.edgeWeightGradient", self.edge_weight_gradient),
            ("segmentation.edgeWeightAlpha", self.edge_weight_alpha),
            ("segmentation.edgeWeightClaim", self.edge_weight_claim),
        ] {
            check_range(field, value, 0.0, 16.0, "between 0 and 16")?;
        }
        check_range(
            "segmentation.mergeThreshold",
            self.merge_threshold,
            0.0,
            16.0,
            "between 0 and 16",
        )?;
        check_range(
            "segmentation.fringeMaxResidual",
            self.fringe_max_residual,
            0.0,
            1.0,
            "between 0 and 1 in linear-light units",
        )?;
        check_u32_range(
            "segmentation.smallRegionPixels",
            self.small_region_pixels,
            0,
            1 << 20,
            "at most 1048576 pixels",
        )?;
        Ok(())
    }
}

/// Which filled-region semantics apply (§17.1).
///
/// PTE-TOPO-016 forbids conflating these. A stacked trace can look seam-free
/// without being a partition; a partition can be unsuitable for screen-print
/// traps.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
#[non_exhaustive]
pub enum GeometryMode {
    /// Exclusive faces tile the domain and share interfaces.
    SharedMosaic,
    /// Larger shapes may continue behind foreground shapes.
    Stacked,
    /// Shapes grouped by ink, tool, or operation, possibly overlapping.
    SeparateOperations,
}

/// Where boundary positions come from (§10, PTE-AA-009).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
#[non_exhaustive]
pub enum BoundarySource {
    /// Invert antialias coverage to a subpixel position (§10.2--10.3).
    CoverageReconstructed,
    /// Place boundaries on the pixel-cell interface (§10.6).
    CrispGrid,
    /// Choose per edge from the available evidence.
    Auto,
}

/// Pixel-art rendering mode (PTE-GEO-024).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
#[non_exhaustive]
pub enum PixelArtMode {
    /// Exact orthogonal logical-pixel outlines.
    Blocky,
    /// Topology-preserving curve fitting.
    Smoothed,
    /// Preserve intentional corners while smoothing long staircases.
    Hybrid,
}

/// Geometry policy (§10, §11).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase", default)]
pub struct GeometryPolicy {
    /// Partition semantics.
    pub mode: Option<GeometryMode>,
    /// Largest accepted boundary error, in source pixels, before physical
    /// scaling (PTE-GEO-008).
    pub curve_tolerance_px: Option<Finite>,
    /// Turning angle above which a stable multi-scale corner is declared, in
    /// degrees (§11.3).
    pub corner_threshold_deg: Option<Finite>,
    /// Hysteresis band around the corner threshold, in degrees, so small
    /// numeric changes do not flip whole curve runs (PTE-GEO-004).
    pub corner_hysteresis_deg: Option<Finite>,
    /// Emit circular arcs where they fit better than a cubic (PTE-GEO-021).
    pub allow_arcs: Option<bool>,
    /// Boundary evidence policy.
    pub boundary_source: Option<BoundarySource>,
    /// Pixel-art mode, used only by the `pixel-art` profile.
    pub pixel_art_mode: Option<PixelArtMode>,
    /// Cap on candidate spans considered per chain position, bounding the
    /// worst case of §11.6 (PTE-GEO-009).
    pub max_span_candidates: Option<u32>,
}

impl Validate for GeometryPolicy {
    fn validate(&self) -> Result<(), ConfigError> {
        check_range(
            "geometry.curveTolerancePx",
            self.curve_tolerance_px,
            0.0,
            64.0,
            "between 0 and 64 source pixels",
        )?;
        check_range(
            "geometry.cornerThresholdDeg",
            self.corner_threshold_deg,
            0.0,
            180.0,
            "between 0 and 180 degrees",
        )?;
        check_range(
            "geometry.cornerHysteresisDeg",
            self.corner_hysteresis_deg,
            0.0,
            90.0,
            "between 0 and 90 degrees",
        )?;
        check_u32_range(
            "geometry.maxSpanCandidates",
            self.max_span_candidates,
            1,
            1024,
            "between 1 and 1024",
        )?;
        Ok(())
    }
}

/// Whether stroke reconstruction is attempted (§13).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
#[non_exhaustive]
pub enum StrokeMode {
    /// Never; filled outlines only.
    Off,
    /// Where the medial model passes its gates.
    Auto,
    /// Wherever a stroke is representable.
    On,
}

/// Stroke policy (§13).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase", default)]
pub struct StrokePolicy {
    /// Stroke inference mode.
    pub prefer: Option<StrokeMode>,
}

impl Validate for StrokePolicy {
    fn validate(&self) -> Result<(), ConfigError> {
        Ok(())
    }
}

/// Whether gradients are reconstructed (§14).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
#[non_exhaustive]
pub enum GradientMode {
    /// Solid fills only.
    Off,
    /// Portable `linearGradient` and `radialGradient` (§14.1).
    Standard,
    /// Non-portable shading, under `experimental` conformance (§14.7).
    Experimental,
}

/// Gradient policy (§14).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase", default)]
pub struct GradientPolicy {
    /// Gradient mode.
    pub mode: Option<GradientMode>,
    /// Largest number of stops in one gradient (PTE-GRAD-008).
    pub max_stops: Option<u32>,
}

impl Validate for GradientPolicy {
    fn validate(&self) -> Result<(), ConfigError> {
        check_u32_range(
            "gradients.maxStops",
            self.max_stops,
            2,
            256,
            "between 2 and 256",
        )
    }
}

/// Fabrication policy (§16).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase", default)]
pub struct FabricationPolicy {
    /// Physical output width in millimetres, if known (PTE-FAB-001).
    pub physical_width_mm: Option<Finite>,
    /// Physical output height in millimetres, if known.
    pub physical_height_mm: Option<Finite>,
    /// Kerf compensation in millimetres.
    pub kerf_mm: Option<Finite>,
    /// Trap width in millimetres, for screen-print separations.
    pub trap_mm: Option<Finite>,
}

impl Validate for FabricationPolicy {
    fn validate(&self) -> Result<(), ConfigError> {
        for (field, value) in [
            ("fabrication.physicalWidthMm", self.physical_width_mm),
            ("fabrication.physicalHeightMm", self.physical_height_mm),
        ] {
            check_range(
                field,
                value,
                0.01,
                100_000.0,
                "a positive size in millimetres",
            )?;
        }
        for (field, value) in [
            ("fabrication.kerfMm", self.kerf_mm),
            ("fabrication.trapMm", self.trap_mm),
        ] {
            check_range(field, value, 0.0, 100.0, "between 0 and 100 millimetres")?;
        }
        Ok(())
    }
}

/// Coordinate precision policy (§18.3).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
#[non_exhaustive]
pub enum Precision {
    /// Search for the fewest decimals that preserve topology and stay inside
    /// the boundary-error budget. PTE-SVG-008 forbids a fixed rule.
    Auto,
    /// Use exactly this many decimals, accepting whatever error results. For
    /// reproducing a specific historical output, not for quality work.
    Fixed(u8),
}

/// Output and serialisation policy (§18).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase", default)]
pub struct OutputPolicy {
    /// Coordinate precision.
    pub precision: Option<Precision>,
    /// Emit `<metadata>` naming the engine and digest. Never contains paths,
    /// usernames, or timestamps (§18.6).
    pub metadata: Option<bool>,
    /// Indent the output for reading. Semantics are identical either way.
    pub pretty: Option<bool>,
    /// Run the §31.1 output gates before returning. Cheap; on by default.
    pub validate_output: Option<bool>,
}

impl Validate for OutputPolicy {
    fn validate(&self) -> Result<(), ConfigError> {
        if let Some(Precision::Fixed(d)) = self.precision
            && d > 12
        {
            return Err(ConfigError::OutOfRange {
                field: "output.precision",
                got: format!("{d}"),
                expected: "at most 12 decimals",
            });
        }
        Ok(())
    }
}

/// Determinism level (§20.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
#[non_exhaustive]
pub enum DeterminismMode {
    /// Identical semantic digest across repeats, thread counts, legal tile
    /// sizes, and targets (PTE-DET-001).
    Strict,
    /// Identical within one target and build feature set.
    Target,
    /// Approximations permitted where they are reported.
    Fast,
}

/// Determinism policy (§20).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase", default)]
pub struct DeterminismPolicy {
    /// Determinism level.
    pub mode: Option<DeterminismMode>,
}

impl Validate for DeterminismPolicy {
    fn validate(&self) -> Result<(), ConfigError> {
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
    fn hex_colours_parse_in_every_accepted_length() {
        assert_eq!(parse_hex_color("#f00").unwrap(), [255, 0, 0, 255]);
        assert_eq!(parse_hex_color("#FF0000").unwrap(), [255, 0, 0, 255]);
        assert_eq!(parse_hex_color("#ff000080").unwrap(), [255, 0, 0, 128]);
        assert_eq!(parse_hex_color("#abc").unwrap(), [0xaa, 0xbb, 0xcc, 255]);
    }

    #[test]
    fn a_colour_name_or_bad_hex_is_refused() {
        for bad in ["red", "#gg0000", "ff0000", "#ff00", "#"] {
            assert!(parse_hex_color(bad).is_err(), "{bad} should not parse");
        }
    }

    /// PTE-COLOR-013: two inks may print the same visible colour and must stay
    /// distinguishable. Sharing an *id* is what is forbidden.
    #[test]
    fn duplicate_ids_are_refused_but_duplicate_colours_are_not() {
        let entry = |id: u32, color: &str| PaletteEntrySpec {
            id,
            color: color.into(),
            pinned: true,
            reach: None,
            role: PaletteRole::Ink,
            priority: 0,
        };

        let same_colour = PalettePolicy {
            mode: Some(PaletteMode::Fixed),
            entries: vec![entry(1, "#101010"), entry(2, "#101010")],
            ..Default::default()
        };
        same_colour.validate().unwrap();

        let same_id = PalettePolicy {
            mode: Some(PaletteMode::Fixed),
            entries: vec![entry(1, "#101010"), entry(1, "#202020")],
            ..Default::default()
        };
        assert_eq!(same_id.validate().unwrap_err().code(), "config.palette");
    }

    #[test]
    fn a_fixed_palette_with_no_entries_is_refused() {
        let p = PalettePolicy {
            mode: Some(PaletteMode::Fixed),
            ..Default::default()
        };
        assert_eq!(p.validate().unwrap_err().code(), "config.palette");
    }

    #[test]
    fn ranges_are_enforced_with_their_units_named() {
        let g = GeometryPolicy {
            corner_threshold_deg: Some(Finite(400.0)),
            ..Default::default()
        };
        let err = g.validate().unwrap_err();
        assert!(err.to_string().contains("degrees"), "{err}");
    }

    #[test]
    fn precision_beyond_twelve_decimals_is_refused() {
        let o = OutputPolicy {
            precision: Some(Precision::Fixed(13)),
            ..Default::default()
        };
        assert_eq!(o.validate().unwrap_err().code(), "config.out_of_range");
    }
}
