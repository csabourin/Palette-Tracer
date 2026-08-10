//! The session adapter a WebAssembly binding wraps (§22).
//!
//! # What is here
//!
//! §22.2 asks for a session that "caches immutable analysis/segmentation so a
//! UI can change curve tolerance, layer policy, or SVG optimization without
//! repeating the most expensive stages when dependencies allow", with explicit
//! cache dependency keys (PTE-API-020) and a `dispose` (PTE-API-021). That is
//! [`TraceSession`], and it is plain Rust: no JavaScript type crosses into it,
//! which is what PTE-ARCH-001 and §22.1's "do not leak JS types into core"
//! require of the layer beneath the binding.
//!
//! # What is not
//!
//! **There is no `wasm-bindgen` binding.** `wasm-bindgen` is not a dependency
//! of this build, so `tracePixels`, the `TraceResult` TypeScript declaration
//! and the `AbortSignal` adapter of §22.1 and §22.3 do not exist. What
//! `make engine-wasm` proves is the load-bearing half: the entire workspace
//! compiles for `wasm32-unknown-unknown` with no OS stubs (PTE-ARCH-003), so
//! the engine is genuinely host-free and a binding is an adapter rather than a
//! port.
//!
//! It also means PTE-NO-049's native-versus-WASM parity is **not** established:
//! the conformance manifest has not been run in a browser or a JavaScript
//! runtime. `docs/IMPLEMENTATION_STATUS.md` records both gaps.
//!
//! Threads are not used anywhere, so §22's single-thread conformance target is
//! met by construction and PTE-NO-038 cannot be violated.

use palette_tracer::Segmentation;
use palette_tracer::{Analysis, Engine, TraceOutput};
use palette_tracer_core::config::{EffectiveConfig, TraceConfig};
use palette_tracer_core::control::TraceControl;
use palette_tracer_core::error::TraceError;
use palette_tracer_core::image::ImageView;
use palette_tracer_core::limits::WorkBudget;

/// Which cached artefacts a configuration change invalidates (Appendix E.1).
///
/// The table in E.1 is the contract; this enum is that table's outcome for one
/// comparison. PTE-API-020: "Changing palette reach invalidates segmentation;
/// changing decimal output precision does not."
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Invalidation {
    /// Nothing cached needs recomputing; only serialisation changes.
    SerializationOnly,
    /// Geometry and everything after it must be recomputed.
    Geometry,
    /// Segmentation and everything after it must be recomputed.
    Segmentation,
    /// The colour analysis, and therefore everything, must be recomputed.
    Analysis,
}

impl Invalidation {
    /// What changing from `before` to `after` invalidates.
    ///
    /// Comparisons are on the *effective* configuration, because that is what
    /// the stages consumed -- comparing the caller's config would miss a change
    /// that only the profile expansion made visible.
    #[must_use]
    pub fn between(before: &EffectiveConfig, after: &EffectiveConfig) -> Self {
        if before.palette != after.palette
            || before.color != after.color
            || before.profile != after.profile
        {
            return Self::Analysis;
        }
        if before.segmentation != after.segmentation {
            return Self::Segmentation;
        }
        if before.geometry != after.geometry || before.determinism != after.determinism {
            return Self::Geometry;
        }
        Self::SerializationOnly
    }
}

/// A session holding the expensive artefacts (§22.2, PTE-ARCH-009).
#[derive(Debug)]
pub struct TraceSession {
    engine: Engine,
    width: u32,
    height: u32,
    pixels: Vec<u8>,
    config: EffectiveConfig,
    analysis: Option<Analysis>,
    segmentation: Option<Segmentation>,
}

impl TraceSession {
    /// Take ownership of a pixel buffer and expand the configuration.
    ///
    /// PTE-API-018 requires copy behaviour to be explicit: this takes the
    /// `Vec` by value and never copies it again. A binding that receives a
    /// transferred `ArrayBuffer` hands it straight in.
    ///
    /// # Errors
    ///
    /// [`TraceError::Config`] if the configuration is invalid, or
    /// [`TraceError::Decode`] if the buffer does not match the geometry.
    pub fn new(
        width: u32,
        height: u32,
        pixels: Vec<u8>,
        config: &TraceConfig,
    ) -> Result<Self, TraceError> {
        let engine = Engine::new();
        let effective = engine.validate_config(config)?;
        // Validate the geometry now rather than at the first trace, so a
        // binding reports a bad buffer where the caller can act on it.
        let _ = ImageView::rgba8(width, height, &pixels)?;
        Ok(Self {
            engine,
            width,
            height,
            pixels,
            config: effective,
            analysis: None,
            segmentation: None,
        })
    }

    /// The expanded configuration.
    #[must_use]
    pub const fn config(&self) -> &EffectiveConfig {
        &self.config
    }

    /// Whether the analysis is cached.
    #[must_use]
    pub const fn has_analysis(&self) -> bool {
        self.analysis.is_some()
    }

    /// Whether the segmentation is cached.
    #[must_use]
    pub const fn has_segmentation(&self) -> bool {
        self.segmentation.is_some()
    }

    /// Bytes retained by artefact, for `retained_bytes_by_artifact`
    /// (PTE-PERF-013).
    #[must_use]
    pub fn retained_bytes(&self) -> [(&'static str, u64); 3] {
        [
            (
                "pixels",
                palette_tracer_core::checked::usize_to_u64(self.pixels.len()),
            ),
            (
                "analysis",
                self.analysis.as_ref().map_or(0, |a| a.claims.bytes()),
            ),
            (
                "segmentation",
                self.segmentation.as_ref().map_or(0, |s| s.labels.bytes()),
            ),
        ]
    }

    /// Apply a new configuration, dropping only what it invalidates.
    ///
    /// # Errors
    ///
    /// [`TraceError::Config`] if the new configuration is invalid, in which
    /// case the session keeps the old one and its caches.
    pub fn reconfigure(&mut self, config: &TraceConfig) -> Result<Invalidation, TraceError> {
        let effective = self.engine.validate_config(config)?;
        let invalidation = Invalidation::between(&self.config, &effective);
        match invalidation {
            Invalidation::Analysis => {
                self.analysis = None;
                self.segmentation = None;
            }
            Invalidation::Segmentation => self.segmentation = None,
            // Geometry and serialisation changes reuse both cached artefacts.
            Invalidation::Geometry | Invalidation::SerializationOnly => {}
        }
        self.config = effective;
        Ok(invalidation)
    }

    /// Trace, reusing whatever is cached.
    ///
    /// # Errors
    ///
    /// Any [`TraceError`].
    pub fn trace(&mut self, control: &dyn TraceControl) -> Result<TraceOutput, TraceError> {
        let image = ImageView::rgba8(self.width, self.height, &self.pixels)?;
        let budget = WorkBudget::new(&self.config.resources);

        if self.analysis.is_none() {
            self.analysis = Some(
                self.engine
                    .analyze(&image, &self.config, &budget, control)?,
            );
        }
        let analysis = self.analysis.as_ref().ok_or_else(|| {
            palette_tracer_core::internal_error!("analysis_missing", "analysis was not cached")
        })?;

        if self.segmentation.is_none() {
            self.segmentation =
                Some(
                    self.engine
                        .segment(&image, analysis, &self.config, &budget, control)?,
                );
        }
        let segmentation = self.segmentation.as_ref().ok_or_else(|| {
            palette_tracer_core::internal_error!(
                "segmentation_missing",
                "segmentation was not cached"
            )
        })?;

        self.engine.vectorize(
            &image,
            analysis,
            segmentation,
            &self.config,
            &budget,
            control,
        )
    }

    /// Release every cached artefact and the pixel buffer (PTE-API-021).
    ///
    /// Explicit, not left to finalisation: §22.2 requires that "WASM memory
    /// must not depend on nondeterministic garbage-collection timing for
    /// normal release".
    pub fn dispose(&mut self) {
        self.analysis = None;
        self.segmentation = None;
        self.pixels = Vec::new();
        self.pixels.shrink_to_fit();
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
    use palette_tracer_core::config::{Finite, Precision, Profile};
    use palette_tracer_core::control::NoControl;

    fn pixels() -> Vec<u8> {
        let mut data = Vec::new();
        for y in 0..8u32 {
            for x in 0..8u32 {
                data.extend_from_slice(if x < 4 {
                    &[220u8, 30, 40, 255]
                } else {
                    &[30u8, 40, 220, 255]
                });
                let _ = y;
            }
        }
        data
    }

    fn session() -> TraceSession {
        TraceSession::new(8, 8, pixels(), &TraceConfig::default()).unwrap()
    }

    #[test]
    fn a_session_caches_the_expensive_stages() {
        let mut s = session();
        assert!(!s.has_analysis() && !s.has_segmentation());
        let first = s.trace(&NoControl).unwrap();
        assert!(s.has_analysis() && s.has_segmentation());
        let second = s.trace(&NoControl).unwrap();
        assert_eq!(
            first.digest, second.digest,
            "reuse must not change the result"
        );
    }

    /// PTE-API-020, both halves of the requirement's own example.
    #[test]
    fn precision_reuses_the_caches_and_reach_does_not() {
        let mut s = session();
        s.trace(&NoControl).unwrap();

        let mut config = TraceConfig::default();
        config.output.precision = Some(Precision::Fixed(4));
        assert_eq!(
            s.reconfigure(&config).unwrap(),
            Invalidation::SerializationOnly
        );
        assert!(
            s.has_analysis() && s.has_segmentation(),
            "a precision change must not invalidate anything"
        );

        let mut config = TraceConfig::default();
        config.palette.default_reach = Some(palette_tracer_core::config::ReachSpec {
            lightness: Finite::new(0.2, "l").unwrap(),
            chroma: Finite::new(0.2, "c").unwrap(),
            hue: Finite::new(0.2, "h").unwrap(),
        });
        assert_eq!(s.reconfigure(&config).unwrap(), Invalidation::Analysis);
        assert!(
            !s.has_analysis() && !s.has_segmentation(),
            "a reach change must invalidate the palette claims and the partition"
        );
    }

    #[test]
    fn a_merge_policy_change_invalidates_segmentation_but_not_analysis() {
        let mut s = session();
        s.trace(&NoControl).unwrap();

        let mut config = TraceConfig::default();
        config.segmentation.merge_threshold = Some(Finite::new(0.5, "m").unwrap());
        assert_eq!(s.reconfigure(&config).unwrap(), Invalidation::Segmentation);
        assert!(s.has_analysis(), "the colour analysis survives");
        assert!(!s.has_segmentation(), "the partition does not");
    }

    #[test]
    fn a_curve_tolerance_change_keeps_both_caches() {
        let mut s = session();
        s.trace(&NoControl).unwrap();
        let mut config = TraceConfig::default();
        config.geometry.curve_tolerance_px = Some(Finite::new(2.0, "t").unwrap());
        assert_eq!(s.reconfigure(&config).unwrap(), Invalidation::Geometry);
        assert!(s.has_analysis() && s.has_segmentation());
    }

    /// An invalid reconfiguration must leave the session usable.
    #[test]
    fn a_refused_reconfiguration_changes_nothing() {
        let mut s = session();
        s.trace(&NoControl).unwrap();
        let before = s.config().clone();

        let err = s
            .reconfigure(&TraceConfig::for_profile(Profile::Laser))
            .unwrap_err();
        assert_eq!(err.code(), "config.unsupported_in_this_build");
        assert_eq!(s.config(), &before);
        assert!(s.has_analysis() && s.has_segmentation());
    }

    /// PTE-API-021: release is explicit and complete.
    #[test]
    fn dispose_releases_every_artefact() {
        let mut s = session();
        s.trace(&NoControl).unwrap();
        assert!(s.retained_bytes().iter().any(|(_, bytes)| *bytes > 0));

        s.dispose();
        assert!(!s.has_analysis() && !s.has_segmentation());
        assert!(
            s.retained_bytes().iter().all(|(_, bytes)| *bytes == 0),
            "{:?}",
            s.retained_bytes()
        );
    }

    /// PTE-API-019: a buffer that does not match the geometry is refused at
    /// construction, where a binding can report it.
    #[test]
    fn an_undersized_buffer_is_refused_at_construction() {
        let err = TraceSession::new(8, 8, vec![0; 4], &TraceConfig::default()).unwrap_err();
        assert_eq!(err.code(), "decode.buffer_too_short");
    }
}
