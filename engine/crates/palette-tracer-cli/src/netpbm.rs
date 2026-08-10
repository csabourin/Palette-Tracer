//! A minimal Netpbm reader (PTE-ARCH-001, PTE-API-004).
//!
//! §5.1 puts image codecs in `palette-tracer-codecs`, outside the algorithmic
//! core, and PTE-ARCH-001 forbids the core from depending on one. That crate is
//! not built, so this front end reads the two formats a decoder can be written
//! for in a hundred lines and refuses everything else *by name*:
//!
//! * `P6` binary PPM -- 8-bit RGB.
//! * `P7` PAM -- 8-bit RGB or RGBA.
//!
//! PNG, JPEG and WebP are **not** supported. PTE-NO-042's principle applies to
//! inputs as much as to settings: the reader says which format it found and
//! that no adapter is built, rather than guessing at the bytes.
//! `tools/png_to_ppm.py` converts a PNG for the documented end-to-end run.
//!
//! PTE-API-004: the same [`ResourceLimits`] apply here as to the raw-pixel API,
//! so an adversarial header cannot make the front end allocate what the engine
//! would have refused.

use palette_tracer_core::error::DecodeError;
use palette_tracer_core::image::PixelFormat;
use palette_tracer_core::limits::ResourceLimits;

/// Decoded pixels and their geometry.
#[derive(Debug, Clone)]
pub struct Decoded {
    /// Image width.
    pub width: u32,
    /// Image height.
    pub height: u32,
    /// Pixel layout.
    pub format: PixelFormat,
    /// Tightly packed pixel data.
    pub data: Vec<u8>,
}

/// Decode a Netpbm image.
///
/// # Errors
///
/// [`DecodeError::UnsupportedFormat`] for anything that is not `P6` or `P7`,
/// [`DecodeError::Malformed`] for a header that does not parse, and
/// [`DecodeError::BufferTooShort`] for truncated pixel data. Dimension limits
/// are enforced before any allocation (PTE-SEC-005).
pub fn decode(bytes: &[u8], limits: &ResourceLimits) -> Result<Decoded, DecodeError> {
    if bytes.len() >= 8 && bytes[..8] == [0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a] {
        return Err(DecodeError::UnsupportedFormat {
            detail: "PNG: no codec adapter is built into this front end \
                     (§5.1 palette-tracer-codecs). Convert with \
                     `tools/png_to_ppm.py` or supply pixels through the library \
                     API"
            .to_owned(),
        });
    }
    match bytes.first().copied() {
        Some(b'P') => {}
        _ => {
            return Err(DecodeError::UnsupportedFormat {
                detail: "not a Netpbm image; this front end reads P6 and P7 only".to_owned(),
            });
        }
    }

    match bytes.get(1).copied() {
        Some(b'6') => decode_ppm(bytes, limits),
        Some(b'7') => decode_pam(bytes, limits),
        Some(other) => Err(DecodeError::UnsupportedFormat {
            detail: format!("Netpbm P{} is not supported; use P6 or P7", other as char),
        }),
        None => Err(DecodeError::Malformed {
            detail: "truncated header".to_owned(),
        }),
    }
}

/// Read whitespace-separated header tokens, skipping `#` comments.
fn tokens(bytes: &[u8], start: usize, count: usize) -> Result<(Vec<u64>, usize), DecodeError> {
    let mut out = Vec::with_capacity(count);
    let mut i = start;
    while out.len() < count {
        while i < bytes.len() && bytes[i].is_ascii_whitespace() {
            i += 1;
        }
        if i < bytes.len() && bytes[i] == b'#' {
            while i < bytes.len() && bytes[i] != b'\n' {
                i += 1;
            }
            continue;
        }
        let begin = i;
        while i < bytes.len() && bytes[i].is_ascii_digit() {
            i += 1;
        }
        if i == begin {
            return Err(DecodeError::Malformed {
                detail: "expected a number in the header".to_owned(),
            });
        }
        let text = core::str::from_utf8(&bytes[begin..i]).map_err(|_| DecodeError::Malformed {
            detail: "header is not ASCII".to_owned(),
        })?;
        out.push(text.parse::<u64>().map_err(|_| DecodeError::Malformed {
            detail: format!("`{text}` is not a number"),
        })?);
    }
    // Exactly one whitespace byte separates the header from the data.
    Ok((out, i + 1))
}

fn check_geometry(
    width: u64,
    height: u64,
    limits: &ResourceLimits,
) -> Result<(u32, u32), DecodeError> {
    let width = u32::try_from(width).map_err(|_| DecodeError::DimensionOverflow {
        width: u32::MAX,
        height: 0,
    })?;
    let height = u32::try_from(height).map_err(|_| DecodeError::DimensionOverflow {
        width,
        height: u32::MAX,
    })?;
    limits
        .check_dimensions(width, height)
        .map_err(|e| DecodeError::UnsupportedFormat {
            detail: e.to_string(),
        })?;
    Ok((width, height))
}

fn decode_ppm(bytes: &[u8], limits: &ResourceLimits) -> Result<Decoded, DecodeError> {
    let (header, offset) = tokens(bytes, 2, 3)?;
    let (width, height) = check_geometry(header[0], header[1], limits)?;
    if header[2] != 255 {
        return Err(DecodeError::UnsupportedFormat {
            detail: format!("maximum value {} is not 255; only 8-bit is read", header[2]),
        });
    }

    let needed = palette_tracer_core::checked::pixel_count(width, height)? * 3;
    let data = bytes
        .get(offset..offset + needed)
        .ok_or(DecodeError::BufferTooShort {
            got: bytes.len().saturating_sub(offset),
            needed,
        })?
        .to_vec();

    Ok(Decoded {
        width,
        height,
        format: PixelFormat::Rgb8,
        data,
    })
}

fn decode_pam(bytes: &[u8], limits: &ResourceLimits) -> Result<Decoded, DecodeError> {
    // PAM's header is a sequence of `KEY value` lines ending with `ENDHDR`.
    let text_end =
        bytes
            .windows(7)
            .position(|w| w == b"ENDHDR\n")
            .ok_or(DecodeError::Malformed {
                detail: "PAM header has no ENDHDR".to_owned(),
            })?;
    let header = core::str::from_utf8(&bytes[..text_end]).map_err(|_| DecodeError::Malformed {
        detail: "PAM header is not ASCII".to_owned(),
    })?;

    let field = |name: &str| -> Option<u64> {
        header
            .lines()
            .find_map(|line| line.strip_prefix(name)?.trim().parse::<u64>().ok())
    };

    let width = field("WIDTH").ok_or(DecodeError::Malformed {
        detail: "PAM header has no WIDTH".to_owned(),
    })?;
    let height = field("HEIGHT").ok_or(DecodeError::Malformed {
        detail: "PAM header has no HEIGHT".to_owned(),
    })?;
    let depth = field("DEPTH").ok_or(DecodeError::Malformed {
        detail: "PAM header has no DEPTH".to_owned(),
    })?;
    let maxval = field("MAXVAL").unwrap_or(255);

    if maxval != 255 {
        return Err(DecodeError::UnsupportedFormat {
            detail: format!("MAXVAL {maxval} is not 255; only 8-bit is read"),
        });
    }
    let format = match depth {
        1 => PixelFormat::Gray8,
        2 => PixelFormat::GrayAlpha8,
        3 => PixelFormat::Rgb8,
        4 => PixelFormat::Rgba8,
        other => {
            return Err(DecodeError::UnsupportedFormat {
                detail: format!("PAM DEPTH {other} is not 1, 2, 3 or 4"),
            });
        }
    };

    let (width, height) = check_geometry(width, height, limits)?;
    let offset = text_end + 7;
    let needed =
        palette_tracer_core::checked::pixel_count(width, height)? * format.bytes_per_pixel();
    let data = bytes
        .get(offset..offset + needed)
        .ok_or(DecodeError::BufferTooShort {
            got: bytes.len().saturating_sub(offset),
            needed,
        })?
        .to_vec();

    Ok(Decoded {
        width,
        height,
        format,
        data,
    })
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
    fn a_binary_ppm_decodes() {
        let mut bytes = b"P6\n2 1\n255\n".to_vec();
        bytes.extend_from_slice(&[255, 0, 0, 0, 0, 255]);
        let decoded = decode(&bytes, &ResourceLimits::default()).unwrap();
        assert_eq!((decoded.width, decoded.height), (2, 1));
        assert_eq!(decoded.format, PixelFormat::Rgb8);
        assert_eq!(decoded.data, vec![255, 0, 0, 0, 0, 255]);
    }

    #[test]
    fn comments_in_the_header_are_skipped() {
        let mut bytes = b"P6\n# made by a tool\n1 1\n255\n".to_vec();
        bytes.extend_from_slice(&[1, 2, 3]);
        let decoded = decode(&bytes, &ResourceLimits::default()).unwrap();
        assert_eq!(decoded.data, vec![1, 2, 3]);
    }

    #[test]
    fn a_pam_with_alpha_decodes() {
        let mut bytes =
            b"P7\nWIDTH 1\nHEIGHT 1\nDEPTH 4\nMAXVAL 255\nTUPLTYPE RGB_ALPHA\nENDHDR\n".to_vec();
        bytes.extend_from_slice(&[9, 8, 7, 6]);
        let decoded = decode(&bytes, &ResourceLimits::default()).unwrap();
        assert_eq!(decoded.format, PixelFormat::Rgba8);
        assert_eq!(decoded.data, vec![9, 8, 7, 6]);
    }

    /// PTE-NO-042's principle applied to input: a PNG is named and refused,
    /// not guessed at.
    #[test]
    fn a_png_is_refused_by_name() {
        let png = [0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a, 0, 0];
        let err = decode(&png, &ResourceLimits::default()).unwrap_err();
        assert_eq!(err.code(), "decode.unsupported_format");
        assert!(err.to_string().contains("PNG"), "{err}");
        assert!(err.to_string().contains("png_to_ppm"), "{err}");
    }

    #[test]
    fn truncated_pixel_data_is_refused() {
        let mut bytes = b"P6\n4 4\n255\n".to_vec();
        bytes.extend_from_slice(&[0; 10]);
        let err = decode(&bytes, &ResourceLimits::default()).unwrap_err();
        assert_eq!(err.code(), "decode.buffer_too_short");
    }

    /// PTE-API-004: the same limits apply, so a hostile header cannot make the
    /// front end allocate what the engine would refuse.
    #[test]
    fn an_oversized_header_is_refused_before_allocation() {
        let bytes = b"P6\n100000 100000\n255\n".to_vec();
        let err = decode(&bytes, &ResourceLimits::small()).unwrap_err();
        assert_eq!(err.code(), "decode.unsupported_format");
        assert!(err.to_string().contains("max_"), "{err}");
    }

    #[test]
    fn a_sixteen_bit_ppm_is_refused_rather_than_truncated() {
        let bytes = b"P6\n1 1\n65535\n".to_vec();
        let err = decode(&bytes, &ResourceLimits::default()).unwrap_err();
        assert_eq!(err.code(), "decode.unsupported_format");
    }
}
