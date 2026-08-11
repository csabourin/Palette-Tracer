//! The Palette Tracer Engine (§6.2, Appendix D.1).
//!
//! ```no_run
//! use palette_tracer::{Engine, TraceConfig};
//! use palette_tracer_core::{ImageView, control::NoControl};
//!
//! # fn main() -> Result<(), palette_tracer_core::TraceError> {
//! let pixels = vec![0u8; 4 * 4 * 4];
//! let image = ImageView::rgba8(4, 4, &pixels)?;
//! let output = Engine::new().trace(image, &TraceConfig::default(), &NoControl)?;
//! println!("{}", output.svg);
//! # Ok(())
//! # }
//! ```
//!
//! # Why the facade, and not `core`
//!
//! §5.1 puts orchestration in `palette-tracer-core`, which cannot work: the
//! stage crates need `core`'s types, so `core` cannot depend on the stage
//! crates. `docs/decisions/ADR-0002-crate-split.md` records the split.
//!
//! # What this build does
//!
//! Profiles `flat-illustration`, `coloring-book`, `pixel-art` and `logo`, end
//! to end: validated pixels, colour assignment against an exact or automatic
//! palette, a minimum-spanning-forest partition, deterministic region merging,
//! antialias-fringe reassignment, one shared boundary graph with the 2x2
//! ambiguity resolved by energy, subpixel boundary reconstruction, and
//! deterministic SVG.
//!
//! Shared chains are then fitted to lines and cubic Béziers with bidirectional
//! error validation (§11), so a boundary that should be one line is one line.
//!
//! Before that, §10 reconstructs subpixel boundary positions: the compositing
//! transfer is estimated from the evidence, coverage is inverted to a signed
//! offset in closed form, and §10.5's objective moves each boundary sample
//! along its own normal inside a trust region. §31.2's synthetic geometry
//! gates are measured in `tests/subpixel_gates.rs`.
//!
//! # What it does not
//!
//! Multi-colour junctions are optimised once as shared vertices, using §10.4's
//! barycentric evidence and PTE-TOPO-013's legality guards. Arcs (§11.4),
//! primitive recognition (§11.7), strokes (§13), gradients (§14) and
//! fabrication (§16) are **not** implemented, and anything that would need
//! them is refused by name at configuration time rather than silently
//! approximated (PTE-NO-042). `docs/IMPLEMENTATION_STATUS.md` is the authority
//! on what is and is not built.

use palette_tracer_color::{Palette, PaletteEntry};
use palette_tracer_core::config::{
    BackgroundPolicy, EffectiveConfig, PaletteMode, PixelArtMode, Profile,
};
use palette_tracer_core::control::{ProgressEvent, Stage, TraceControl};
use palette_tracer_core::digest::digest_document;
use palette_tracer_core::error::{ConfigError, TraceError};
use palette_tracer_core::image::ImageView;
use palette_tracer_core::ir::{Color, Paint, VectorDocument};
use palette_tracer_core::labels::LabelPlane;
use palette_tracer_core::limits::WorkBudget;
use palette_tracer_core::report::{
    AlgorithmVersions, BoundarySourceReport, EngineInfo, Fallback, InputInfo, PaletteReportEntry,
    REPORT_SCHEMA_VERSION, RepresentationReport, SegmentationReport, TopologyReport, TraceReport,
    Warning,
};
use palette_tracer_geometry::{FitOptions, FitStats};
use palette_tracer_topology::{AmbiguityProfile, ExtractOptions, Extraction};
use std::borrow::Cow;

// Re-exported so a host adapter depends on the facade alone, keeping the
// stage crates an implementation detail (ADR-0002).
pub use palette_tracer_segment::Segmentation;
// `TraceConfig` is already in scope from the `use` above; re-export it so a
// host adapter needs only this crate.
pub use palette_tracer_core::config::TraceConfig;
pub use palette_tracer_core::{ImageView as Pixels, TraceError as Error};

/// What the engine can do in this build.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Capabilities {
    /// Whether §11 curve fitting is available.
    pub curves: bool,
    /// Whether §13 stroke reconstruction is available.
    pub strokes: bool,
    /// Whether §14 gradient reconstruction is available.
    pub gradients: bool,
    /// Whether §16 fabrication transforms are available.
    pub fabrication: bool,
}

/// Requirements this build does not implement.
///
/// Reported under `unimplemented` on every trace, so a caller learns what the
/// engine did *not* do rather than inferring it from silence (§0.2).
pub const UNIMPLEMENTED: &[&str] = &[
    "PTE-GEO-010/011 (§11.7 primitive recognition)",
    "the §11.4 circular arc model; lines and cubics only",
    "PTE-GEO-015..021 (§12 logo and lettering regularization)",
    "PTE-STROKE-001..009 (§13 centrelines and widths)",
    "PTE-GRAD-001..010 (§14 gradient reconstruction)",
    "PTE-FAB-001..014 (§16 fabrication geometry)",
    "PTE-SEG-018/019 (§8.8 tiled segmentation)",
    "PTE-PERF-006..011 (§19.5, §19.6 tiling and parallelism)",
    "§18.7 cross-renderer compatibility matrix",
];

/// The engine.
#[derive(Debug, Clone, Copy, Default)]
pub struct Engine {
    capabilities: Capabilities,
}

/// The fitter's view of the effective configuration (§11, Appendix G.1).
///
/// One place where geometry settings cross into `palette-tracer-geometry`, so
/// the stage crate stays independent of the configuration model (ADR-0002).
fn fit_options(config: &EffectiveConfig) -> FitOptions {
    FitOptions {
        // §15: `pixel-art` sets this to zero, which is not "fit very tightly"
        // but "the pixel grid is the intended geometry". `FitOptions::enabled`
        // reads it that way and the chains are left alone.
        tolerance_px: config.geometry.curve_tolerance_px,
        corner_threshold_deg: config.geometry.corner_threshold_deg,
        corner_hysteresis_deg: config.geometry.corner_hysteresis_deg,
        // Always false here: `Profile::expand` refuses a request for arcs by
        // name (PTE-NO-042), so this cannot be true at this point.
        allow_arcs: config.geometry.allow_arcs,
        max_span_candidates: config.geometry.max_span_candidates,
    }
}

/// Immutable analysis products (§5.3 stage B, PTE-ARCH-009).
#[derive(Debug, Clone)]
pub struct Analysis {
    /// The resolved palette.
    pub palette: Palette,
    /// Per-pixel palette claim.
    pub claims: LabelPlane,
    /// Pixels claimed inside some entry's reach.
    pub claimed_pixels: u64,
    /// Pixels no entry claimed (§26.4).
    pub unclaimed_pixels: u64,
    /// Pixels with no colour evidence.
    pub transparent_pixels: u64,
    /// Pixels per palette entry.
    pub pixels_per_entry: Vec<u64>,
    /// Assignment cache hits.
    pub cache_hits: u64,
    /// Whether sRGB was assumed (PTE-COLOR-005).
    pub assumed_srgb: bool,
}

/// Everything one trace produced (PTE-API-005).
#[derive(Debug, Clone)]
pub struct TraceOutput {
    /// The typed vector document.
    pub document: VectorDocument,
    /// The serialised SVG.
    pub svg: String,
    /// The expanded configuration.
    pub config: EffectiveConfig,
    /// The machine-readable report.
    pub report: TraceReport,
    /// The semantic digest (Appendix F).
    pub digest: String,
}

impl Engine {
    /// An engine with this build's capabilities.
    #[must_use]
    pub fn new() -> Self {
        Self {
            capabilities: Capabilities {
                // §11 line and cubic fitting with bidirectional validation.
                // Arcs (§11.4) and primitive recognition (§11.7) are not
                // included, and `UNIMPLEMENTED` names both.
                curves: true,
                strokes: false,
                gradients: false,
                fabrication: false,
            },
        }
    }

    /// What this build implements.
    #[must_use]
    pub const fn capabilities(&self) -> Capabilities {
        self.capabilities
    }

    /// Validate and expand a configuration (§6.2).
    ///
    /// # Errors
    ///
    /// Any [`ConfigError`]; see [`Profile::expand`].
    pub fn validate_config(&self, config: &TraceConfig) -> Result<EffectiveConfig, ConfigError> {
        Profile::expand(config)
    }

    /// Stage B: colour statistics, palette, and per-pixel claims.
    ///
    /// # Errors
    ///
    /// [`TraceError::Cancelled`], [`TraceError::ResourceLimit`],
    /// [`TraceError::Decode`], or [`TraceError::Config`] for an unusable
    /// palette.
    pub fn analyze(
        &self,
        image: &ImageView<'_>,
        config: &EffectiveConfig,
        budget: &WorkBudget,
        control: &dyn TraceControl,
    ) -> Result<Analysis, TraceError> {
        budget.enter(Stage::Analyze);
        control.progress(ProgressEvent::StageStarted {
            stage: Stage::Analyze,
        });
        config
            .resources
            .check_dimensions(image.width(), image.height())?;

        let palette = match config.palette.mode {
            PaletteMode::Fixed => Palette::from_effective(&config.palette),
            _ => {
                let histogram =
                    palette_tracer_color::histogram(image, &config.color, budget, control)?;
                // Pinned entries carry through untouched (PTE-COLOR-019); the
                // automatic centres fill the rest.
                let pinned: Vec<PaletteEntry> = Palette::from_effective(&config.palette)
                    .entries()
                    .iter()
                    .filter(|e| e.pinned)
                    .cloned()
                    .collect();
                palette_tracer_color::auto::generate(
                    &histogram,
                    config.palette.max_colors,
                    &pinned,
                    config.palette.default_reach,
                    budget,
                    control,
                )?
            }
        };

        if palette.is_empty() {
            return Err(TraceError::Config(ConfigError::Palette {
                reason: "no palette could be resolved: the image has no visible pixels",
            }));
        }

        let assignment =
            palette_tracer_color::assign(image, &palette, &config.color, budget, control)?;

        control.progress(ProgressEvent::StageFinished {
            stage: Stage::Analyze,
        });

        Ok(Analysis {
            palette,
            claims: assignment.labels,
            claimed_pixels: assignment.claimed_pixels,
            unclaimed_pixels: assignment.unclaimed_pixels,
            transparent_pixels: assignment.transparent_pixels,
            pixels_per_entry: assignment.pixels_per_entry,
            cache_hits: assignment.cache_hits,
            assumed_srgb: config.color.assume_srgb_when_untagged,
        })
    }

    /// Stage C: the exclusive partition (§8).
    ///
    /// # Errors
    ///
    /// As [`palette_tracer_segment::segment`].
    pub fn segment(
        &self,
        image: &ImageView<'_>,
        analysis: &Analysis,
        config: &EffectiveConfig,
        budget: &WorkBudget,
        control: &dyn TraceControl,
    ) -> Result<Segmentation, TraceError> {
        let pinned: Vec<bool> = analysis
            .palette
            .entries()
            .iter()
            .map(|e| e.pinned)
            .collect();
        palette_tracer_segment::segment(
            image,
            &analysis.claims,
            &pinned,
            &config.color,
            &config.segmentation,
            &config.resources,
            budget,
            control,
        )
    }

    /// Stages D through I: topology, lowering, serialisation, validation.
    ///
    /// # Errors
    ///
    /// [`TraceError::Cancelled`], [`TraceError::ResourceLimit`],
    /// [`TraceError::Validation`] if a gate fails, or [`TraceError::Numerical`]
    /// if the digest cannot be computed.
    pub fn vectorize(
        &self,
        image: &ImageView<'_>,
        analysis: &Analysis,
        segmentation: &Segmentation,
        config: &EffectiveConfig,
        budget: &WorkBudget,
        control: &dyn TraceControl,
    ) -> Result<TraceOutput, TraceError> {
        budget.enter(Stage::Topology);
        control.progress(ProgressEvent::StageStarted {
            stage: Stage::Topology,
        });

        let mut extraction = self.extract(analysis, segmentation, config, budget, control)?;
        // PTE-TOPO-005: the validator is not optional.
        palette_tracer_topology::validate(&extraction.topology)?;
        control.progress(ProgressEvent::StageFinished {
            stage: Stage::Topology,
        });

        // Stage E.5 (§10): reconstruct subpixel boundary positions before any
        // fitting happens. Order matters in one direction only -- §11 fits
        // curves *to* boundary samples, so the samples have to be right first.
        // Fitting and then re-reconstructing would mean moving a cubic's
        // control points, which is a different problem with a different error
        // bound.
        //
        // §15 and PTE-TOPO-010: `pixel-art` never borrows the antialias
        // mixture assumptions. Its boundaries are already labelled
        // `pixel_art_policy` and are left exactly where the grid put them.
        let reconstruction = if config.profile == Profile::PixelArt {
            palette_tracer_aa::ReconstructionStats::default()
        } else {
            let face_colors = face_colors(&extraction.topology, segmentation);
            let stats = palette_tracer_aa::reconstruct(
                &mut extraction.topology,
                image,
                &face_colors,
                &palette_tracer_aa::ReconstructOptions::default(),
                control,
            )?;
            // PTE-GEO-014's rule applies to any post-extraction change of
            // shared geometry, and §10.5 is one: the boundary moved, so the
            // corridor and intersection checks run again rather than being
            // assumed to still hold.
            palette_tracer_topology::validate(&extraction.topology)?;
            stats
        };
        // The per-sample counts are not in the report schema, which censuses
        // edges rather than samples (PTE-AA-009). They are the useful number
        // for judging how much evidence a trace actually had, so they go out
        // as progress counters.
        control.progress(ProgressEvent::Counted {
            stage: Stage::Topology,
            name: "coverage_constrained_samples",
            value: u64::from(reconstruction.constrained_samples),
        });
        control.progress(ProgressEvent::Counted {
            stage: Stage::Topology,
            name: "coverage_unconstrained_samples",
            value: u64::from(reconstruction.unconstrained_samples),
        });
        control.progress(ProgressEvent::Counted {
            stage: Stage::Topology,
            name: "coverage_reconstructed_junctions",
            value: u64::from(reconstruction.reconstructed_junctions),
        });
        control.progress(ProgressEvent::Counted {
            stage: Stage::Topology,
            name: "coverage_low_confidence_junctions",
            value: u64::from(reconstruction.low_confidence_junctions),
        });
        control.progress(ProgressEvent::Counted {
            stage: Stage::Topology,
            name: "coverage_crisp_junctions",
            value: u64::from(reconstruction.crisp_junctions),
        });

        // Stage F (§5.3): fit the shared chains. This replaces geometry and
        // nothing else -- vertices, half-edges, faces and cycles are untouched,
        // and each chain is fitted once and inherited by both its faces
        // (PTE-GEO-012, PTE-AA-008).
        budget.enter(Stage::Fit);
        control.progress(ProgressEvent::StageStarted { stage: Stage::Fit });
        let fit = palette_tracer_geometry::fit_topology(
            &mut extraction.topology,
            &fit_options(config),
            control,
        )?;
        // PTE-GEO-014: "Any post-fit optimization that changes shared geometry
        // MUST rerun topology corridor and intersection validation." Fitting
        // changes shared geometry, so the validator runs again rather than
        // being trusted to have covered it the first time.
        palette_tracer_topology::validate(&extraction.topology)?;
        control.progress(ProgressEvent::StageFinished { stage: Stage::Fit });

        let mut warnings = Vec::new();
        let paints = self.paints(analysis, segmentation, config, &mut warnings);

        budget.enter(Stage::Serialize);
        let mut document = palette_tracer_svg::lower(
            &extraction.topology,
            &paints,
            config,
            image.width(),
            image.height(),
        );
        config
            .resources
            .check_output_elements(document.element_count())?;

        let features: Vec<&str> = Vec::new();
        let fallbacks: Vec<Fallback> = Vec::new();
        let digest = digest_document(
            &document,
            config,
            &AlgorithmVersions::default(),
            &features,
            &fallbacks,
        )?;
        document.provenance.semantic_digest = digest.clone();

        let svg = palette_tracer_svg::serialize(&document, config)?;
        palette_tracer_core::limits::ResourceLimits::check_count(
            "max_output_bytes",
            palette_tracer_core::checked::usize_to_u64(svg.len()),
            config.resources.max_output_bytes,
        )?;

        let report = self.build_report(
            image,
            analysis,
            segmentation,
            &extraction,
            &fit,
            &document,
            &svg,
            config,
            budget,
            warnings,
            fallbacks,
            &digest,
        );

        control.progress(ProgressEvent::StageFinished {
            stage: Stage::Serialize,
        });

        Ok(TraceOutput {
            document,
            svg,
            config: config.clone(),
            report,
            digest,
        })
    }

    /// Trace an image end to end (Appendix D.1).
    ///
    /// # Errors
    ///
    /// Any [`TraceError`].
    pub fn trace(
        &self,
        image: ImageView<'_>,
        config: &TraceConfig,
        control: &dyn TraceControl,
    ) -> Result<TraceOutput, TraceError> {
        let effective = self.validate_config(config)?;
        let budget = WorkBudget::new(&effective.resources);
        budget.enter(Stage::Validate);

        let analysis = self.analyze(&image, &effective, &budget, control)?;
        let segmentation = self.segment(&image, &analysis, &effective, &budget, control)?;
        self.vectorize(
            &image,
            &analysis,
            &segmentation,
            &effective,
            &budget,
            control,
        )
    }

    fn extract(
        &self,
        analysis: &Analysis,
        segmentation: &Segmentation,
        config: &EffectiveConfig,
        budget: &WorkBudget,
        control: &dyn TraceControl,
    ) -> Result<Extraction, TraceError> {
        // Region colour, for the §9.4 energy's data and mixture terms.
        let mut colors = Vec::with_capacity(segmentation.labels.len());
        for i in 0..segmentation.labels.len() {
            let region = segmentation.labels.get_index(i).index();
            colors.push(
                segmentation
                    .summaries
                    .get(region)
                    .map_or(palette_tracer_color::Oklab::ZERO, |s| s.mean()),
            );
        }

        let palette_of: Vec<Option<palette_tracer_core::ir::PaletteId>> = segmentation
            .summaries
            .iter()
            .map(|s| {
                analysis
                    .palette
                    .entries()
                    .get(s.claim.index())
                    .map(|_| palette_tracer_core::ir::PaletteId(s.claim.0))
            })
            .collect();
        let pinned_of: Vec<bool> = segmentation.summaries.iter().map(|s| s.pinned).collect();

        let options = ExtractOptions {
            weights: palette_tracer_topology::AmbiguityWeights::default(),
            // PTE-TOPO-010: pixel art gets its own connectivity policy and
            // never borrows the antialias mixture assumptions.
            profile: if config.profile == Profile::PixelArt {
                AmbiguityProfile::PixelArt
            } else {
                AmbiguityProfile::Antialiased
            },
            evidence: if config.profile == Profile::PixelArt {
                palette_tracer_core::ir::BoundaryEvidence::PixelArtPolicy
            } else {
                // Extraction starts on the pixel-cell interface. §10 later
                // upgrades individual edges and junctions when the raster
                // supplies usable coverage evidence (PTE-AA-009).
                palette_tracer_core::ir::BoundaryEvidence::CrispGrid
            },
        };

        palette_tracer_topology::extract(
            &segmentation.labels,
            &colors,
            &palette_of,
            &pinned_of,
            &options,
            &config.resources,
            budget,
            control,
        )
    }

    /// The paint for each palette entry, applying background policy.
    fn paints(
        &self,
        analysis: &Analysis,
        segmentation: &Segmentation,
        config: &EffectiveConfig,
        warnings: &mut Vec<Warning>,
    ) -> Vec<Paint> {
        // PTE-SVG-012: the caller's bytes, unchanged. The OKLab anchor used for
        // matching never appears here.
        let mut paints: Vec<Paint> = analysis
            .palette
            .entries()
            .iter()
            .map(|e| Paint::Solid(e.output))
            .collect();

        match config.color.background {
            BackgroundPolicy::Transparent | BackgroundPolicy::Auto => {
                // The background is the palette entry owning the largest area
                // that touches the image border. PTE-COLOR-023: it stays a face
                // in the topology; only its paint becomes transparent, so the
                // holes around it are unaffected.
                if let Some(entry) = self.background_entry(segmentation) {
                    if let Some(paint) = paints.get_mut(entry) {
                        *paint = Paint::Solid(Color::TRANSPARENT);
                    }
                    if config.color.background == BackgroundPolicy::Auto {
                        warnings.push(
                            Warning::info(
                                "background.chosen_automatically",
                                format!("palette entry {entry} was treated as the background"),
                            )
                            .about("PTE-API-009"),
                        );
                    }
                }
            }
            BackgroundPolicy::PaletteEntry(id) => {
                if let Some(index) = analysis.palette.entries().iter().position(|e| e.id == id) {
                    paints[index] = Paint::Solid(Color::TRANSPARENT);
                } else {
                    warnings.push(
                        Warning::warn(
                            "background.entry_not_found",
                            format!("no palette entry has id {id}"),
                        )
                        .about("PTE-COLOR-023"),
                    );
                }
            }
            _ => {}
        }

        paints
    }

    /// The palette entry most likely to be the background.
    fn background_entry(&self, segmentation: &Segmentation) -> Option<usize> {
        let width = segmentation.labels.width();
        let height = segmentation.labels.height();
        let mut border_area = vec![0u64; segmentation.summaries.len()];

        for x in 0..width {
            for y in [0, height - 1] {
                let region = segmentation.labels.get(x, y).index();
                border_area[region] += 1;
            }
        }
        for y in 0..height {
            for x in [0, width - 1] {
                let region = segmentation.labels.get(x, y).index();
                border_area[region] += 1;
            }
        }

        // Most border pixels wins; ties go to the lower region index, which is
        // raster order of first appearance.
        let region = border_area
            .iter()
            .enumerate()
            .max_by_key(|(index, area)| (**area, std::cmp::Reverse(*index)))
            .filter(|(_, area)| **area > 0)
            .map(|(index, _)| index)?;
        segmentation.summaries.get(region).map(|s| s.claim.index())
    }

    #[allow(clippy::too_many_arguments)]
    fn build_report(
        &self,
        image: &ImageView<'_>,
        analysis: &Analysis,
        segmentation: &Segmentation,
        extraction: &Extraction,
        fit: &FitStats,
        document: &VectorDocument,
        svg: &str,
        config: &EffectiveConfig,
        budget: &WorkBudget,
        mut warnings: Vec<Warning>,
        fallbacks: Vec<Fallback>,
        digest: &str,
    ) -> TraceReport {
        let (lines, cubics, arcs, control_points) = palette_tracer_svg::count_segments(document);

        if analysis.unclaimed_pixels > 0 {
            warnings.push(
                Warning::info(
                    "palette.pixels_outside_every_reach",
                    format!(
                        "{} pixels fell outside every palette entry's reach and were \
                         assigned to the nearest",
                        analysis.unclaimed_pixels
                    ),
                )
                .about("PTE-COLOR-012"),
            );
        }
        // §0.2: a trace with no usable coverage evidence says so explicitly.
        // This is an input outcome, not a missing implementation: §10 ran and
        // retained the honest crisp estimate (§10.6).
        warnings.push(
            Warning::info(
                "geometry.evidence_is_the_pixel_grid",
                "some boundary positions may remain on pixel-cell interfaces \
                 where §10 found no usable subpixel coverage evidence"
                    .to_owned(),
            )
            .about("PTE-AA-009"),
        );
        let chains = fit.chains_fitted
            + fit.polyline_fallbacks
            + fit.chains_already_minimal
            + fit.chains_trivial;
        if fit.polyline_fallbacks > 0 {
            warnings.push(
                Warning::warn(
                    "geometry.polyline_fallbacks",
                    format!(
                        "{} of {chains} shared chains exhausted the fitting budget and \
                         kept their grid polyline",
                        fit.polyline_fallbacks
                    ),
                )
                .about("PTE-GEO-005"),
            );
        }
        // Not a warning: a short jagged boundary has no simpler faithful
        // representation, so the search succeeded and returned it. Saying so
        // keeps a reader from reading the count above as if it included these.
        if fit.chains_already_minimal > 0 {
            warnings.push(
                Warning::info(
                    "geometry.chains_already_minimal",
                    format!(
                        "{} of {chains} shared chains were already their own simplest \
                         faithful representation",
                        fit.chains_already_minimal
                    ),
                )
                .about("PTE-GEO-005"),
            );
        }
        if config.profile == Profile::PixelArt
            && config.geometry.pixel_art_mode != PixelArtMode::Blocky
        {
            warnings.push(
                Warning::warn(
                    "geometry.pixel_art_mode_unsupported",
                    "only `blocky` is implemented; the request was refused rather \
                     than approximated"
                        .to_owned(),
                )
                .about("PTE-GEO-024"),
            );
        }

        let palette = analysis
            .palette
            .entries()
            .iter()
            .enumerate()
            .map(|(index, entry)| PaletteReportEntry {
                id: entry.id,
                color: entry.output.to_hex_rgb(),
                pinned: entry.pinned,
                pixels: analysis.pixels_per_entry.get(index).copied().unwrap_or(0),
                regions: segmentation
                    .summaries
                    .iter()
                    .filter(|s| s.claim.index() == index)
                    .count() as u32,
            })
            .collect();

        TraceReport {
            schema_version: REPORT_SCHEMA_VERSION,
            engine: EngineInfo::default(),
            input: InputInfo {
                width: image.width(),
                height: image.height(),
                format: image.format(),
                encoding: image.color_encoding(),
                alpha_mode: image.alpha_mode(),
                profile_action: Cow::Borrowed(if analysis.assumed_srgb {
                    "assumed-srgb"
                } else {
                    "declared-by-caller"
                }),
            },
            effective_config: config.clone(),
            palette,
            segmentation: SegmentationReport {
                initial_regions: segmentation.initial_region_count,
                final_regions: segmentation.region_count,
                merges: segmentation.merge_outcome.merges,
                fringe_reassignments: segmentation.fringe.len() as u32,
                fringe_pixels: segmentation.fringe.iter().map(|f| f.pixels).sum(),
                protected_regions: segmentation.merge_outcome.protected,
                label_width: Some(segmentation.labels.label_width()),
            },
            topology: TopologyReport {
                faces: extraction.topology.interior_face_count() as u32,
                shared_edges: extraction.topology.edges.len() as u32,
                junctions: extraction.junctions,
                holes: extraction.topology.hole_count() as u32,
                ambiguous_cells: extraction.ambiguous_cells,
                ambiguous_resolved_by_continuity: extraction.ambiguous_resolved_by_continuity,
                // The partition was checked by `assert_complete_ownership`
                // before topology was built, so this is a measurement of a
                // proven fact rather than an estimate.
                unassigned_pixels: 0,
            },
            representation: RepresentationReport {
                paths: document.element_count() as u32,
                lines,
                cubics,
                arcs,
                primitives: 0,
                strokes: 0,
                gradients: 0,
                gradient_stops: 0,
                control_points,
                polyline_fallbacks: fit.polyline_fallbacks,
                svg_bytes: palette_tracer_core::checked::usize_to_u64(svg.len()),
                coordinate_decimals: palette_tracer_svg::choose_serialization_precision(
                    document, config,
                )
                .unwrap_or(0),
            },
            // PTE-AA-009: a census of what each boundary's position actually
            // came from, counted from the edges themselves. It used to be
            // derived from the profile, which could only ever restate the
            // configuration; now §10 records its outcome on every edge and
            // this counts them.
            boundary_sources: boundary_source_census(&extraction.topology),
            resources: budget.statistics(),
            warnings,
            fallbacks,
            unimplemented: UNIMPLEMENTED.iter().map(|s| Cow::Borrowed(*s)).collect(),
            semantic_digest: digest.to_owned(),
        }
    }
}

/// Count each boundary's evidence class (PTE-AA-009).
fn boundary_source_census(topology: &palette_tracer_core::ir::Topology) -> BoundarySourceReport {
    use palette_tracer_core::ir::BoundaryEvidence;
    let mut report = BoundarySourceReport {
        coverage_reconstructed: 0,
        crisp_grid: 0,
        low_confidence_fallback: 0,
        pixel_art_policy: 0,
    };
    for edge in &topology.edges {
        match edge.confidence.evidence {
            BoundaryEvidence::CoverageReconstructed => report.coverage_reconstructed += 1,
            BoundaryEvidence::CrispGrid => report.crisp_grid += 1,
            BoundaryEvidence::LowConfidenceFallback => report.low_confidence_fallback += 1,
            BoundaryEvidence::PixelArtPolicy => report.pixel_art_policy += 1,
            _ => report.crisp_grid += 1,
        }
    }
    report
}

/// Each face's mean colour as a premultiplied linear sample, indexed by
/// [`FaceId`], for §10's mixture evidence.
///
/// The exterior face has no raster region and gets a transparent entry, which
/// `reconstruct` skips by identity rather than by colour.
fn face_colors(
    topology: &palette_tracer_core::ir::Topology,
    segmentation: &Segmentation,
) -> Vec<palette_tracer_color::alpha::PremulLinearRgba> {
    topology
        .faces
        .iter()
        .map(|face| {
            if face.exterior {
                return palette_tracer_color::alpha::PremulLinearRgba::TRANSPARENT;
            }
            segmentation.summaries.get(face.region.index()).map_or(
                palette_tracer_color::alpha::PremulLinearRgba::TRANSPARENT,
                palette_tracer_segment::RegionSummary::premultiplied_mean,
            )
        })
        .collect()
}
