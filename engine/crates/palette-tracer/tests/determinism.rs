//! Determinism and metamorphic tests (§20, §27).
//!
//! PTE-DET-001 requires the same semantic digest for repeated executions,
//! supported thread counts, legal tile sizes, and native versus WASM.
//!
//! * **Repeats** and **thread counts** are checked here. The engine is
//!   single-threaded (§19.6 parallelism is unimplemented), so tracing the same
//!   image from several threads at once is a real test of the claim that no
//!   shared mutable state exists.
//! * **Tile sizes** cannot be checked: tiling is unimplemented, so there is
//!   only one legal tile size and the requirement is vacuous.
//! * **Native versus WASM** is checked by building for `wasm32-unknown-unknown`
//!   in CI, not by running the corpus in a browser. `make engine-wasm` proves
//!   the code compiles and is host-free; it does not prove the digests match.
//!   `docs/IMPLEMENTATION_STATUS.md` records that PTE-NO-049 parity is
//!   therefore not established.

#![allow(
    // Integration tests are their own crates, so the workspace lints apply
    // without the exemptions a `mod tests` gets. Unwrapping a fixture that is
    // valid by construction, comparing an exact coordinate, and writing a bound
    // as two comparisons are all clearer here than the alternatives.
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::float_cmp,
    clippy::manual_range_contains
)]

use palette_tracer::Engine;
use palette_tracer_core::config::{
    Modifier, PaletteEntrySpec, PaletteMode, PaletteRole, Profile, TraceConfig,
};
use palette_tracer_core::control::NoControl;
use palette_tracer_core::image::{AlphaMode, ColorEncoding, ImageView, Orientation, PixelFormat};

const SIDE: u32 = 12;

/// A fixture with junctions, a diagonal, a hole and a border feature.
fn artwork() -> Vec<u8> {
    let mut data = Vec::with_capacity((SIDE * SIDE * 4) as usize);
    for y in 0..SIDE {
        for x in 0..SIDE {
            let color: [u8; 4] = if x == y {
                [10, 10, 10, 255]
            } else if x >= 4 && x < 8 && y >= 4 && y < 8 {
                [220, 30, 40, 255]
            } else if x < 6 {
                [240, 240, 240, 255]
            } else {
                [40, 90, 200, 255]
            };
            data.extend_from_slice(&color);
        }
    }
    data
}

fn digest_of(data: &[u8], config: &TraceConfig) -> String {
    let image = ImageView::rgba8(SIDE, SIDE, data).expect("valid fixture");
    Engine::new()
        .trace(image, config, &NoControl)
        .expect("valid trace")
        .digest
}

/// PTE-DET-001: repeated executions give the same digest.
#[test]
fn repeated_executions_agree() {
    let data = artwork();
    let config = TraceConfig::for_profile(Profile::FlatIllustration);
    let first = digest_of(&data, &config);
    assert!(first.starts_with("pte-semantic-v1-blake3:"));
    for _ in 0..8 {
        assert_eq!(digest_of(&data, &config), first);
    }
}

/// PTE-DET-001: thread count must not change the result. The engine is
/// single-threaded, so what this proves is the absence of shared mutable
/// state -- which is the property that would break first if it were added.
#[test]
fn concurrent_traces_agree() {
    let data = artwork();
    let expected = digest_of(&data, &TraceConfig::for_profile(Profile::FlatIllustration));

    let handles: Vec<_> = (0..8)
        .map(|_| {
            let data = data.clone();
            std::thread::spawn(move || {
                digest_of(&data, &TraceConfig::for_profile(Profile::FlatIllustration))
            })
        })
        .collect();
    for handle in handles {
        assert_eq!(handle.join().expect("no thread may panic"), expected);
    }
}

/// The SVG bytes themselves are stable within one build (§20.1 RECOMMENDS it).
#[test]
fn serialised_bytes_are_stable_within_one_build() {
    let data = artwork();
    let config = TraceConfig::for_profile(Profile::FlatIllustration);
    let image = ImageView::rgba8(SIDE, SIDE, &data).unwrap();
    let first = Engine::new().trace(image, &config, &NoControl).unwrap().svg;
    for _ in 0..4 {
        let image = ImageView::rgba8(SIDE, SIDE, &data).unwrap();
        assert_eq!(
            Engine::new().trace(image, &config, &NoControl).unwrap().svg,
            first
        );
    }
}

/// §27.1: randomising the colour under fully transparent pixels must leave the
/// semantic output identical (PTE-COLOR-008).
#[test]
fn alpha_zero_colour_randomisation_changes_nothing() {
    let mut base = Vec::with_capacity((SIDE * SIDE * 4) as usize);
    for y in 0..SIDE {
        for x in 0..SIDE {
            if x < 4 {
                // Transparent, with a colour underneath.
                base.extend_from_slice(&[0, 0, 0, 0]);
            } else if y < 6 {
                base.extend_from_slice(&[220, 30, 40, 255]);
            } else {
                base.extend_from_slice(&[40, 90, 200, 255]);
            }
        }
    }

    let config = TraceConfig::for_profile(Profile::FlatIllustration);
    let expected = digest_of(&base, &config);

    // Fill the transparent region with three different arbitrary colours.
    for fill in [[255u8, 0, 255], [0, 255, 0], [17, 99, 240]] {
        let mut variant = base.clone();
        for pixel in variant.chunks_mut(4) {
            if pixel[3] == 0 {
                pixel[0] = fill[0];
                pixel[1] = fill[1];
                pixel[2] = fill[2];
            }
        }
        assert_eq!(
            digest_of(&variant, &config),
            expected,
            "colour {fill:?} under transparent pixels changed the output"
        );
    }
}

/// §27.1: permuting the palette entries must give the same regions and colours
/// under the stable identifier mapping.
#[test]
fn palette_permutation_gives_the_same_colours() {
    let colors = ["#dc1e28", "#285ac8", "#f0f0f0", "#0a0a0a"];

    let build = |order: &[usize]| {
        let mut config = TraceConfig::for_profile(Profile::FlatIllustration);
        config.modifiers.push(Modifier::ExactPalette);
        config.palette.mode = Some(PaletteMode::Fixed);
        for (position, &index) in order.iter().enumerate() {
            config.palette.entries.push(PaletteEntrySpec {
                // The identifier travels with the colour, which is what makes
                // the mapping stable (PTE-COLOR-012).
                id: index as u32,
                color: colors[index].to_owned(),
                pinned: true,
                reach: None,
                role: PaletteRole::Ink,
                priority: 0,
            });
            let _ = position;
        }
        config
    };

    let data = artwork();
    let forward = build(&[0, 1, 2, 3]);
    let shuffled = build(&[2, 0, 3, 1]);

    assert_eq!(
        digest_of(&data, &forward),
        digest_of(&data, &shuffled),
        "the order the caller listed the entries in must not reach the output"
    );
}

/// §27.1: an integer translation with canvas expansion translates the geometry
/// and changes nothing else about the topology.
#[test]
fn integer_translation_translates_the_result() {
    let data = artwork();
    let image = ImageView::rgba8(SIDE, SIDE, &data).unwrap();
    let config = TraceConfig::for_profile(Profile::FlatIllustration);
    let original = Engine::new().trace(image, &config, &NoControl).unwrap();

    // The same artwork, shifted one pixel right and down into a larger canvas
    // whose margin repeats the artwork's own border colour, so no new region
    // is introduced.
    let big = SIDE + 2;
    let mut shifted = Vec::with_capacity((big * big * 4) as usize);
    for y in 0..big {
        for x in 0..big {
            let sx = x.saturating_sub(1).min(SIDE - 1);
            let sy = y.saturating_sub(1).min(SIDE - 1);
            let index = ((sy * SIDE + sx) * 4) as usize;
            shifted.extend_from_slice(&data[index..index + 4]);
        }
    }
    let image = ImageView::rgba8(big, big, &shifted).unwrap();
    let translated = Engine::new().trace(image, &config, &NoControl).unwrap();

    assert_eq!(
        original.report.topology.faces, translated.report.topology.faces,
        "translation must not change the face count"
    );
    assert_eq!(
        original.report.topology.holes, translated.report.topology.holes,
        "translation must not change the hole count"
    );
}

/// The pixel format must not change the meaning: the same colours delivered as
/// `Rgb8` and as `Rgba8` are the same artwork.
#[test]
fn an_equivalent_pixel_format_gives_the_same_digest() {
    let rgba = artwork();
    let rgb: Vec<u8> = rgba.chunks(4).flat_map(|p| [p[0], p[1], p[2]]).collect();

    let config = TraceConfig::for_profile(Profile::FlatIllustration);
    let a = Engine::new()
        .trace(
            ImageView::rgba8(SIDE, SIDE, &rgba).unwrap(),
            &config,
            &NoControl,
        )
        .unwrap()
        .digest;
    let b = Engine::new()
        .trace(
            ImageView::new(
                SIDE,
                SIDE,
                0,
                PixelFormat::Rgb8,
                &rgb,
                ColorEncoding::Srgb,
                AlphaMode::Opaque,
                Orientation::Identity,
            )
            .unwrap(),
            &config,
            &NoControl,
        )
        .unwrap()
        .digest;

    assert_eq!(a, b);
}

/// A change that *should* alter the output must alter the digest, or the
/// digest is not measuring anything.
#[test]
fn a_real_change_changes_the_digest() {
    let data = artwork();
    let config = TraceConfig::for_profile(Profile::FlatIllustration);
    let base = digest_of(&data, &config);

    let mut moved = data.clone();
    // Repaint one pixel in the middle of the red square.
    let index = ((6 * SIDE + 6) * 4) as usize;
    moved[index] = 0;
    moved[index + 1] = 255;
    moved[index + 2] = 0;
    assert_ne!(digest_of(&moved, &config), base);

    assert_ne!(
        digest_of(&data, &TraceConfig::for_profile(Profile::ColoringBook)),
        base,
        "a different profile is a different result"
    );
}
