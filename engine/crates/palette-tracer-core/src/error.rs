//! The error taxonomy.
//!
//! PTE-ARCH-007 requires decoder errors, configuration errors, resource-limit
//! errors, numerical failures, cancellation, and validation failures to be
//! *distinct* categories rather than one string type. Callers branch on the
//! category; the CLI maps categories to the §21.3 exit codes.
//!
//! Human messages may change between versions. The machine-readable
//! [`TraceError::code`] does not (PTE-API-012).

use core::fmt;

/// Every way a trace can fail.
///
/// The discriminants are the categories of PTE-ARCH-007. Adding a variant is a
/// breaking change for callers that match exhaustively, which is intended.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum TraceError {
    /// The configuration is invalid, unsupported, or self-contradictory.
    Config(ConfigError),
    /// Input pixels or their metadata could not be interpreted.
    Decode(DecodeError),
    /// A declared [`crate::ResourceLimits`] bound was reached.
    ResourceLimit(ResourceLimitError),
    /// A computation produced a non-finite or otherwise unusable value and no
    /// declared fallback applied.
    Numerical(NumericalError),
    /// The caller cancelled through [`crate::TraceControl`].
    Cancelled,
    /// Output failed a validation gate: topology, geometry, or SVG.
    Validation(ValidationError),
    /// An internal invariant was violated. This is always a defect in the
    /// engine, never a property of the input (PTE-SEC-008).
    Internal(InternalError),
}

impl TraceError {
    /// The stable machine-readable code for this error.
    ///
    /// Codes are dotted, lowercase, and permanent. Tests and callers match on
    /// these; prose is for humans only (PTE-API-012).
    #[must_use]
    pub fn code(&self) -> &'static str {
        match self {
            Self::Config(e) => e.code(),
            Self::Decode(e) => e.code(),
            Self::ResourceLimit(e) => e.code(),
            Self::Numerical(e) => e.code(),
            Self::Cancelled => "cancelled",
            Self::Validation(e) => e.code(),
            Self::Internal(_) => "internal.invariant",
        }
    }

    /// The §21.3 process exit code for this category.
    #[must_use]
    pub fn exit_code(&self) -> i32 {
        match self {
            Self::Config(_) => 2,
            Self::Decode(_) => 3,
            Self::ResourceLimit(_) => 4,
            Self::Numerical(_) => 5,
            Self::Validation(_) => 6,
            Self::Cancelled => 7,
            Self::Internal(_) => 8,
        }
    }
}

impl fmt::Display for TraceError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Config(e) => write!(f, "configuration error: {e}"),
            Self::Decode(e) => write!(f, "input error: {e}"),
            Self::ResourceLimit(e) => write!(f, "resource limit exceeded: {e}"),
            Self::Numerical(e) => write!(f, "numerical failure: {e}"),
            Self::Cancelled => write!(f, "cancelled by caller"),
            Self::Validation(e) => write!(f, "validation failure: {e}"),
            Self::Internal(e) => write!(f, "internal invariant violated: {e}"),
        }
    }
}

impl std::error::Error for TraceError {}

macro_rules! from_variant {
    ($ty:ty, $variant:ident) => {
        impl From<$ty> for TraceError {
            fn from(e: $ty) -> Self {
                Self::$variant(e)
            }
        }
    };
}
from_variant!(ConfigError, Config);
from_variant!(DecodeError, Decode);
from_variant!(ResourceLimitError, ResourceLimit);
from_variant!(NumericalError, Numerical);
from_variant!(ValidationError, Validation);
from_variant!(InternalError, Internal);

/// Configuration is rejected before any pixel is read.
///
/// PTE-PROFILE-001 forbids silently picking a winner among conflicting
/// settings, and PTE-NO-042 forbids ignoring a field the build cannot honour.
/// Both surface here.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum ConfigError {
    /// A named field is outside its documented range.
    OutOfRange {
        /// Dotted path of the offending field, e.g. `geometry.curve_tolerance`.
        field: &'static str,
        /// What the caller supplied.
        got: String,
        /// The documented valid range.
        expected: &'static str,
    },
    /// A float field was NaN or infinite (PTE-API-008).
    NotFinite {
        /// Dotted path of the offending field.
        field: &'static str,
    },
    /// An enum value or object key the schema does not define (PTE-API-007).
    UnknownValue {
        /// Dotted path of the offending field.
        field: &'static str,
        /// The unrecognised token.
        got: String,
    },
    /// Two settings that cannot both hold (PTE-PROFILE-001).
    IncompatibleCombination {
        /// First setting, as a dotted path plus value.
        first: String,
        /// Second setting, as a dotted path plus value.
        second: String,
        /// Why they conflict, in one clause.
        reason: &'static str,
    },
    /// The setting is specified, but this build does not implement it.
    ///
    /// This is the honest answer required by PTE-NO-042 and §0.2: the engine
    /// refuses rather than quietly producing something else and calling it the
    /// requested feature.
    UnsupportedInThisBuild {
        /// Dotted path of the requested field.
        field: &'static str,
        /// Requirement identifier that would govern the missing behaviour.
        requirement: &'static str,
    },
    /// The configuration schema version is not one this build understands.
    UnsupportedSchemaVersion {
        /// What the caller supplied.
        got: u32,
        /// What this build implements.
        supported: u32,
    },
    /// A palette was required but empty, or contained duplicate identifiers.
    Palette {
        /// What is wrong, in one clause.
        reason: &'static str,
    },
}

impl ConfigError {
    /// The stable machine-readable code.
    #[must_use]
    pub fn code(&self) -> &'static str {
        match self {
            Self::OutOfRange { .. } => "config.out_of_range",
            Self::NotFinite { .. } => "config.not_finite",
            Self::UnknownValue { .. } => "config.unknown_value",
            Self::IncompatibleCombination { .. } => "config.incompatible_combination",
            Self::UnsupportedInThisBuild { .. } => "config.unsupported_in_this_build",
            Self::UnsupportedSchemaVersion { .. } => "config.unsupported_schema_version",
            Self::Palette { .. } => "config.palette",
        }
    }
}

impl fmt::Display for ConfigError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::OutOfRange {
                field,
                got,
                expected,
            } => write!(f, "`{field}` is {got}, expected {expected}"),
            Self::NotFinite { field } => write!(f, "`{field}` is NaN or infinite"),
            Self::UnknownValue { field, got } => {
                write!(f, "`{field}` has unknown value `{got}`")
            }
            Self::IncompatibleCombination {
                first,
                second,
                reason,
            } => write!(f, "{first} and {second} cannot both apply: {reason}"),
            Self::UnsupportedInThisBuild { field, requirement } => write!(
                f,
                "`{field}` is not implemented in this build ({requirement}); \
                 it is refused rather than silently ignored"
            ),
            Self::UnsupportedSchemaVersion { got, supported } => {
                write!(
                    f,
                    "config schema version {got}, this build implements {supported}"
                )
            }
            Self::Palette { reason } => write!(f, "palette is invalid: {reason}"),
        }
    }
}

impl std::error::Error for ConfigError {}

/// The pixel view or its metadata could not be interpreted.
///
/// Every variant is produced *before* any algorithm reads a pixel
/// (PTE-API-001).
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum DecodeError {
    /// Width or height is zero.
    EmptyImage,
    /// `width * height`, `stride * height`, or a byte count overflowed.
    DimensionOverflow {
        /// Requested width.
        width: u32,
        /// Requested height.
        height: u32,
    },
    /// The stride is smaller than one row of the declared format.
    StrideTooSmall {
        /// Declared stride, in bytes.
        stride_bytes: usize,
        /// Bytes one row of the declared format needs.
        minimum: usize,
    },
    /// The stride is not a whole number of samples for the declared format.
    StrideMisaligned {
        /// Declared stride, in bytes.
        stride_bytes: usize,
        /// Bytes per pixel of the declared format.
        bytes_per_pixel: usize,
    },
    /// The buffer is shorter than the declared geometry requires.
    BufferTooShort {
        /// Bytes supplied.
        got: usize,
        /// Bytes the declared geometry needs.
        needed: usize,
    },
    /// The container or encoding is not one this build reads.
    UnsupportedFormat {
        /// What was detected, in one clause.
        detail: String,
    },
    /// The bytes claim a format but do not conform to it.
    Malformed {
        /// What is wrong, in one clause.
        detail: String,
    },
    /// An embedded colour profile could not be honoured and the policy is
    /// strict (PTE-COLOR-007).
    ColorProfile {
        /// What is wrong, in one clause.
        detail: String,
    },
}

impl DecodeError {
    /// The stable machine-readable code.
    #[must_use]
    pub fn code(&self) -> &'static str {
        match self {
            Self::EmptyImage => "decode.empty_image",
            Self::DimensionOverflow { .. } => "decode.dimension_overflow",
            Self::StrideTooSmall { .. } => "decode.stride_too_small",
            Self::StrideMisaligned { .. } => "decode.stride_misaligned",
            Self::BufferTooShort { .. } => "decode.buffer_too_short",
            Self::UnsupportedFormat { .. } => "decode.unsupported_format",
            Self::Malformed { .. } => "decode.malformed",
            Self::ColorProfile { .. } => "decode.color_profile",
        }
    }
}

impl fmt::Display for DecodeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EmptyImage => write!(f, "image has zero width or height"),
            Self::DimensionOverflow { width, height } => {
                write!(f, "{width}x{height} overflows address arithmetic")
            }
            Self::StrideTooSmall {
                stride_bytes,
                minimum,
            } => write!(
                f,
                "stride {stride_bytes} is below the {minimum} a row needs"
            ),
            Self::StrideMisaligned {
                stride_bytes,
                bytes_per_pixel,
            } => write!(
                f,
                "stride {stride_bytes} is not a multiple of {bytes_per_pixel} bytes per pixel"
            ),
            Self::BufferTooShort { got, needed } => {
                write!(f, "buffer has {got} bytes, geometry needs {needed}")
            }
            Self::UnsupportedFormat { detail } => write!(f, "unsupported input: {detail}"),
            Self::Malformed { detail } => write!(f, "malformed input: {detail}"),
            Self::ColorProfile { detail } => write!(f, "colour profile: {detail}"),
        }
    }
}

impl std::error::Error for DecodeError {}

/// A declared resource bound was reached.
///
/// PTE-SEC-007 requires a typed error or an authorised deterministic downgrade
/// -- never a crash, a hang, or a silent resize.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum ResourceLimitError {
    /// A named limit was exceeded.
    Exceeded {
        /// Which limit, e.g. `max_pixels` or `max_regions`.
        limit: &'static str,
        /// The configured bound.
        allowed: u64,
        /// What the input or the computation required.
        required: u64,
    },
    /// The deterministic work budget ran out (PTE-API-006).
    WorkBudgetExhausted {
        /// The configured budget in abstract work units.
        budget: u64,
        /// The stage that was running when the budget ran out.
        stage: &'static str,
    },
}

impl ResourceLimitError {
    /// The stable machine-readable code.
    #[must_use]
    pub fn code(&self) -> &'static str {
        match self {
            Self::Exceeded { .. } => "resource.exceeded",
            Self::WorkBudgetExhausted { .. } => "resource.work_budget_exhausted",
        }
    }
}

impl fmt::Display for ResourceLimitError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Exceeded {
                limit,
                allowed,
                required,
            } => write!(f, "{limit}: needed {required}, limit is {allowed}"),
            Self::WorkBudgetExhausted { budget, stage } => {
                write!(f, "work budget of {budget} units exhausted during {stage}")
            }
        }
    }
}

impl std::error::Error for ResourceLimitError {}

/// A computation could not produce a usable number and no fallback applied.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum NumericalError {
    /// A value that must be finite was not (PTE-GEO-006, PTE-SVG-003).
    NonFinite {
        /// Where it happened.
        context: &'static str,
    },
    /// A solve was singular and the declared fallback also failed.
    Singular {
        /// Where it happened.
        context: &'static str,
    },
    /// A bounded iteration did not converge and no fallback applied.
    NoConvergence {
        /// Where it happened.
        context: &'static str,
        /// The iteration cap that was hit.
        iterations: u32,
    },
}

impl NumericalError {
    /// The stable machine-readable code.
    #[must_use]
    pub fn code(&self) -> &'static str {
        match self {
            Self::NonFinite { .. } => "numeric.non_finite",
            Self::Singular { .. } => "numeric.singular",
            Self::NoConvergence { .. } => "numeric.no_convergence",
        }
    }
}

impl fmt::Display for NumericalError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NonFinite { context } => write!(f, "non-finite value in {context}"),
            Self::Singular { context } => write!(f, "singular system in {context}"),
            Self::NoConvergence {
                context,
                iterations,
            } => write!(f, "{context} did not converge in {iterations} iterations"),
        }
    }
}

impl std::error::Error for NumericalError {}

/// Output failed a gate. These are the §31.1 hard gates in error form.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum ValidationError {
    /// A topology invariant from Appendix B does not hold.
    Topology {
        /// Which check failed.
        check: &'static str,
        /// What was observed.
        detail: String,
    },
    /// Fitted geometry left its uncertainty corridor, self-intersected, or
    /// otherwise failed §11.5.
    Geometry {
        /// Which check failed.
        check: &'static str,
        /// What was observed.
        detail: String,
    },
    /// Serialised output is not valid SVG, or violates §18.
    Svg {
        /// Which check failed.
        check: &'static str,
        /// What was observed.
        detail: String,
    },
    /// A pixel in the domain has no owner in a shared partition
    /// (PTE-SEG-002, §31.1).
    UnassignedPixels {
        /// How many pixels lack an owner.
        count: u64,
    },
}

impl ValidationError {
    /// The stable machine-readable code.
    #[must_use]
    pub fn code(&self) -> &'static str {
        match self {
            Self::Topology { .. } => "validation.topology",
            Self::Geometry { .. } => "validation.geometry",
            Self::Svg { .. } => "validation.svg",
            Self::UnassignedPixels { .. } => "validation.unassigned_pixels",
        }
    }
}

impl fmt::Display for ValidationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Topology { check, detail } => write!(f, "topology check `{check}`: {detail}"),
            Self::Geometry { check, detail } => write!(f, "geometry check `{check}`: {detail}"),
            Self::Svg { check, detail } => write!(f, "svg check `{check}`: {detail}"),
            Self::UnassignedPixels { count } => {
                write!(f, "{count} pixels have no owner in the shared partition")
            }
        }
    }
}

impl std::error::Error for ValidationError {}

/// An engine defect. Never caused by input.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InternalError {
    /// What invariant was violated.
    pub invariant: &'static str,
    /// Observed state, for the bug report.
    pub detail: String,
}

impl fmt::Display for InternalError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.invariant, self.detail)
    }
}

impl std::error::Error for InternalError {}

/// Construct an [`InternalError`] with a formatted detail string.
#[macro_export]
macro_rules! internal_error {
    ($invariant:literal, $($arg:tt)*) => {
        $crate::error::TraceError::Internal($crate::error::InternalError {
            invariant: $invariant,
            detail: format!($($arg)*),
        })
    };
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

    /// §21.3 fixes the exit-code table. Changing a number here is a breaking
    /// change for every script that calls the CLI.
    #[test]
    fn exit_codes_match_the_spec_table() {
        let cases: [(TraceError, i32); 7] = [
            (TraceError::Config(ConfigError::NotFinite { field: "x" }), 2),
            (TraceError::Decode(DecodeError::EmptyImage), 3),
            (
                TraceError::ResourceLimit(ResourceLimitError::Exceeded {
                    limit: "max_pixels",
                    allowed: 1,
                    required: 2,
                }),
                4,
            ),
            (
                TraceError::Numerical(NumericalError::NonFinite { context: "fit" }),
                5,
            ),
            (
                TraceError::Validation(ValidationError::UnassignedPixels { count: 1 }),
                6,
            ),
            (TraceError::Cancelled, 7),
            (
                TraceError::Internal(InternalError {
                    invariant: "i",
                    detail: String::new(),
                }),
                8,
            ),
        ];
        for (err, expected) in cases {
            assert_eq!(err.exit_code(), expected, "for {err}");
        }
    }

    /// Codes are the machine contract (PTE-API-012). Prose is not.
    #[test]
    fn codes_are_category_prefixed_and_stable() {
        assert_eq!(
            TraceError::Decode(DecodeError::EmptyImage).code(),
            "decode.empty_image"
        );
        assert_eq!(TraceError::Cancelled.code(), "cancelled");
        assert_eq!(
            TraceError::Config(ConfigError::UnsupportedInThisBuild {
                field: "strokes.prefer",
                requirement: "PTE-STROKE-001",
            })
            .code(),
            "config.unsupported_in_this_build"
        );
    }
}
