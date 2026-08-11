//! `pte`, the command-line front end (§21).
//!
//! ```text
//! pte trace [OPTIONS] <INPUT> <OUTPUT>
//! pte inspect [OPTIONS] <INPUT>
//! pte validate <SVG>
//! pte profiles [--json]
//! pte schema [--json]
//! ```
//!
//! # Stream discipline (PTE-API-014)
//!
//! "Binary SVG bytes and machine-readable reports MUST not be mixed with human
//! progress on stdout." SVG goes to the output path, or to stdout when that is
//! `-`. The report goes where `--report` says. Everything a human reads goes to
//! stderr. Nothing else is ever written to stdout.
//!
//! # Exit codes (§21.3)
//!
//! Taken from [`TraceError::exit_code`], so the table lives in one place and
//! the CLI cannot drift from it.

mod netpbm;

use palette_tracer::Engine;
use palette_tracer_core::config::{
    BackgroundPolicy, Finite, PaletteEntrySpec, PaletteMode, PaletteRole, Profile, TraceConfig,
};
use palette_tracer_core::control::NoControl;
use palette_tracer_core::error::{ConfigError, DecodeError, TraceError};
use palette_tracer_core::image::{AlphaMode, ColorEncoding, ImageView, Orientation, PixelFormat};
use std::io::{Read, Write};

fn main() -> std::process::ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match run(&args) {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("pte: {error}");
            eprintln!("pte: code {}", error.code());
            std::process::ExitCode::from(error.exit_code() as u8)
        }
    }
}

fn run(args: &[String]) -> Result<(), TraceError> {
    match args.first().map(String::as_str) {
        Some("trace") => trace(&args[1..]),
        Some("inspect") => inspect(&args[1..]),
        Some("validate") => validate(&args[1..]),
        Some("profiles") => profiles(&args[1..]),
        Some("schema") => schema(),
        Some("--help" | "-h") | None => {
            eprint!("{USAGE}");
            Ok(())
        }
        Some("--version" | "-V") => {
            eprintln!(
                "{} {}",
                palette_tracer_core::ENGINE_NAME,
                palette_tracer_core::ENGINE_VERSION
            );
            Ok(())
        }
        Some(other) => Err(TraceError::Config(ConfigError::UnknownValue {
            field: "command",
            got: other.to_owned(),
        })),
    }
}

const USAGE: &str = "\
pte -- the Palette Tracer Engine

  pte trace [OPTIONS] <INPUT> <OUTPUT>   trace an image to SVG
  pte inspect [OPTIONS] <INPUT>          report on an image without writing SVG
  pte validate <SVG>                     check a produced SVG
  pte profiles [--json]                  every profile and its effective settings
  pte schema [--json]                    the configuration schema

Options
  --profile <NAME>            logo | flat-illustration | coloring-book | pixel-art
  --config <FILE>             a JSON configuration; flags below override it
  --palette <#rrggbb,...>     an exact palette; implies --exact-palette
  --max-colors <N>            target size for an automatic palette
  --background <keep|transparent|auto>
  --curve-tolerance <PX>
  --report <FILE|->           write the JSON report
  --print-effective-config    write the expanded configuration to stderr
  --pretty                    indent the SVG

INPUT and OUTPUT accept `-` for stdin and stdout. Input must be a binary PPM
(P6) or PAM (P7); PNG is refused by name, because no codec adapter is built
into this front end. `tools/png_to_ppm.py` converts one.

Exit codes: 0 success, 2 configuration, 3 input, 4 resource limit,
5 numerical, 6 output validation, 7 cancelled, 8 internal.
";

#[derive(Default)]
struct Options {
    config: TraceConfig,
    report: Option<String>,
    print_effective_config: bool,
    positional: Vec<String>,
}

/// Configuration values supplied directly on the command line.
///
/// They are collected separately and applied after every `--config` file has
/// been parsed. PTE-API-015 defines precedence by source, not by token order:
/// a flag must win even when it appears before the file it overrides.
#[derive(Default)]
struct ConfigOverrides {
    profile: Option<Profile>,
    palette: Option<Vec<PaletteEntrySpec>>,
    max_colors: Option<u32>,
    background: Option<BackgroundPolicy>,
    curve_tolerance_px: Option<Finite>,
    pretty: bool,
    exact_palette: bool,
}

impl ConfigOverrides {
    fn apply(self, config: &mut TraceConfig) {
        if let Some(profile) = self.profile {
            config.profile = profile;
        }
        if let Some(entries) = self.palette {
            config.palette.entries = entries;
        }
        if let Some(max_colors) = self.max_colors {
            config.palette.max_colors = Some(max_colors);
        }
        if let Some(background) = self.background {
            config.color.background = Some(background);
        }
        if let Some(curve_tolerance_px) = self.curve_tolerance_px {
            config.geometry.curve_tolerance_px = Some(curve_tolerance_px);
        }
        if self.pretty {
            config.output.pretty = Some(true);
        }
        if self.exact_palette {
            config
                .modifiers
                .push(palette_tracer_core::config::Modifier::ExactPalette);
        }
    }
}

fn parse(args: &[String]) -> Result<Options, TraceError> {
    let mut out = Options {
        config: TraceConfig::default(),
        ..Default::default()
    };
    let mut overrides = ConfigOverrides::default();
    let mut index = 0;

    while index < args.len() {
        let arg = &args[index];
        let mut value = || -> Result<String, TraceError> {
            index += 1;
            args.get(index)
                .cloned()
                .ok_or_else(|| missing_value(arg.clone()))
        };

        match arg.as_str() {
            "--config" => {
                let path = value()?;
                let text = read_input(&path)?;
                let text = String::from_utf8(text).map_err(|_| {
                    TraceError::Config(ConfigError::UnknownValue {
                        field: "config",
                        got: "not UTF-8".to_owned(),
                    })
                })?;
                out.config = TraceConfig::from_json(&text)?;
            }
            "--profile" => {
                let name = value()?;
                overrides.profile = Some(
                    Profile::ALL
                        .into_iter()
                        .find(|p| p.as_str() == name)
                        .ok_or(TraceError::Config(ConfigError::UnknownValue {
                            field: "profile",
                            got: name.clone(),
                        }))?,
                );
            }
            "--palette" => {
                let list = value()?;
                let entries: Vec<PaletteEntrySpec> = list
                    .split(',')
                    .filter(|s| !s.trim().is_empty())
                    .enumerate()
                    .map(|(i, color)| PaletteEntrySpec {
                        id: i as u32,
                        color: color.trim().to_owned(),
                        pinned: true,
                        reach: None,
                        role: PaletteRole::Fill,
                        priority: 0,
                    })
                    .collect();
                // An empty list is still an override, so it would silently
                // clear a config file's palette and then fail three layers
                // down as "mode is `fixed` but no entries were supplied" --
                // blaming the file for what the flag did. Refuse it by name.
                if entries.is_empty() {
                    return Err(TraceError::Config(ConfigError::Palette {
                        reason: "`--palette` was given no colours",
                    }));
                }
                overrides.palette = Some(entries);
            }
            "--max-colors" => {
                let n = value()?;
                overrides.max_colors = Some(
                    n.parse()
                        .map_err(|_| out_of_range("palette.maxColors", &n))?,
                );
            }
            "--background" => {
                let policy = value()?;
                overrides.background = Some(match policy.as_str() {
                    "keep" => BackgroundPolicy::Keep,
                    "transparent" => BackgroundPolicy::Transparent,
                    "auto" => BackgroundPolicy::Auto,
                    other => {
                        return Err(TraceError::Config(ConfigError::UnknownValue {
                            field: "color.background",
                            got: other.to_owned(),
                        }));
                    }
                });
            }
            "--curve-tolerance" => {
                let text = value()?;
                let number: f64 = text
                    .parse()
                    .map_err(|_| out_of_range("geometry.curveTolerancePx", &text))?;
                overrides.curve_tolerance_px =
                    Some(Finite::new(number, "geometry.curveTolerancePx")?);
            }
            "--report" => out.report = Some(value()?),
            "--print-effective-config" => out.print_effective_config = true,
            "--pretty" => overrides.pretty = true,
            "--exact-palette" => overrides.exact_palette = true,
            other if other.starts_with("--") => {
                return Err(TraceError::Config(ConfigError::UnknownValue {
                    field: "option",
                    got: other.to_owned(),
                }));
            }
            other => out.positional.push(other.to_owned()),
        }
        index += 1;
    }

    overrides.apply(&mut out.config);
    if !out.config.palette.entries.is_empty() {
        out.config.palette.mode = Some(PaletteMode::Fixed);
    }
    Ok(out)
}

fn missing_value(flag: String) -> TraceError {
    TraceError::Config(ConfigError::UnknownValue {
        field: "option",
        got: format!("{flag} needs a value"),
    })
}

fn out_of_range(field: &'static str, got: &str) -> TraceError {
    TraceError::Config(ConfigError::OutOfRange {
        field,
        got: got.to_owned(),
        expected: "a number",
    })
}

fn read_input(path: &str) -> Result<Vec<u8>, TraceError> {
    // PTE-API-013: `-` is stdin, and the format is resolved from the bytes
    // rather than from a name, because a stream has none.
    if path == "-" {
        let mut buffer = Vec::new();
        std::io::stdin()
            .read_to_end(&mut buffer)
            .map_err(|e| io_error("stdin", &e))?;
        Ok(buffer)
    } else {
        std::fs::read(path).map_err(|e| io_error(path, &e))
    }
}

fn write_output(path: &str, bytes: &[u8]) -> Result<(), TraceError> {
    if path == "-" {
        std::io::stdout()
            .write_all(bytes)
            .map_err(|e| io_error("stdout", &e))
    } else {
        std::fs::write(path, bytes).map_err(|e| io_error(path, &e))
    }
}

fn io_error(what: &str, error: &std::io::Error) -> TraceError {
    TraceError::Decode(DecodeError::Malformed {
        detail: format!("{what}: {error}"),
    })
}

fn load(path: &str, config: &TraceConfig) -> Result<netpbm::Decoded, TraceError> {
    let bytes = read_input(path)?;
    palette_tracer_core::limits::ResourceLimits::check_count(
        "max_input_bytes",
        palette_tracer_core::checked::usize_to_u64(bytes.len()),
        config.resources.max_input_bytes,
    )?;
    Ok(netpbm::decode(&bytes, &config.resources)?)
}

fn view<'a>(decoded: &'a netpbm::Decoded) -> Result<ImageView<'a>, TraceError> {
    Ok(ImageView::new(
        decoded.width,
        decoded.height,
        0,
        decoded.format,
        &decoded.data,
        ColorEncoding::Srgb,
        // PTE-API-003: Netpbm carries no orientation metadata, so declaring
        // `Identity` is a statement of fact rather than a guess.
        if decoded.format == PixelFormat::Rgb8 || decoded.format == PixelFormat::Gray8 {
            AlphaMode::Opaque
        } else {
            AlphaMode::Straight
        },
        Orientation::Identity,
    )?)
}

fn trace(args: &[String]) -> Result<(), TraceError> {
    let options = parse(args)?;
    let [input, output] = options.positional.as_slice() else {
        eprint!("{USAGE}");
        return Err(TraceError::Config(ConfigError::UnknownValue {
            field: "arguments",
            got: format!(
                "expected <INPUT> <OUTPUT>, got {}",
                options.positional.len()
            ),
        }));
    };

    // PTE-API-014: two machine-readable artefacts cannot share one stream. If
    // both were allowed on stdout the caller would receive an SVG document with
    // a JSON object appended, which parses as neither. Refusing is the only
    // honest answer; silently dropping one would lose data the caller asked for.
    if output == "-" && options.report.as_deref() == Some("-") {
        return Err(TraceError::Config(ConfigError::UnknownValue {
            field: "report",
            got: "the SVG and the report cannot both go to stdout; \
                  send one of them to a file"
                .to_owned(),
        }));
    }

    let engine = Engine::new();
    let effective = engine.validate_config(&options.config)?;
    if options.print_effective_config {
        eprintln!("{}", effective.to_json());
    }

    let decoded = load(input, &options.config)?;
    let image = view(&decoded)?;
    let result = engine.trace(image, &options.config, &NoControl)?;

    write_output(output, result.svg.as_bytes())?;
    if let Some(path) = &options.report {
        write_report(path, &result.report.to_json())?;
    }

    // Human-readable summary: stderr only (PTE-API-014).
    eprintln!(
        "pte: {} faces, {} shared edges, {} bytes, digest {}",
        result.report.topology.faces,
        result.report.topology.shared_edges,
        result.report.representation.svg_bytes,
        result.digest
    );
    for warning in result
        .report
        .warnings
        .iter()
        .filter(|w| w.severity == palette_tracer_core::report::Severity::Warning)
    {
        eprintln!("pte: warning [{}] {}", warning.code, warning.message);
    }
    Ok(())
}

/// The report may go to stdout, but only when the SVG did not.
fn write_report(path: &str, json: &str) -> Result<(), TraceError> {
    write_output(path, json.as_bytes())?;
    if path == "-" {
        write_output("-", b"\n")?;
    }
    Ok(())
}

fn inspect(args: &[String]) -> Result<(), TraceError> {
    let options = parse(args)?;
    let [input] = options.positional.as_slice() else {
        return Err(TraceError::Config(ConfigError::UnknownValue {
            field: "arguments",
            got: "expected <INPUT>".to_owned(),
        }));
    };

    let engine = Engine::new();
    let decoded = load(input, &options.config)?;
    let image = view(&decoded)?;
    let result = engine.trace(image, &options.config, &NoControl)?;
    write_output(
        options.report.as_deref().unwrap_or("-"),
        result.report.to_json().as_bytes(),
    )?;
    write_output(options.report.as_deref().unwrap_or("-"), b"\n")
}

fn validate(args: &[String]) -> Result<(), TraceError> {
    let [path] = args else {
        return Err(TraceError::Config(ConfigError::UnknownValue {
            field: "arguments",
            got: "expected <SVG>".to_owned(),
        }));
    };
    let bytes = read_input(path)?;
    let text = String::from_utf8(bytes).map_err(|_| {
        TraceError::Validation(palette_tracer_core::error::ValidationError::Svg {
            check: "utf8",
            detail: "the file is not UTF-8".to_owned(),
        })
    })?;

    // What a front end can check without an XML parser. It is deliberately
    // shallow, and says so, rather than implying a conformance check.
    let mut problems = Vec::new();
    if !text.trim_start().starts_with("<svg") {
        problems.push("does not start with an <svg> element");
    }
    if !text.trim_end().ends_with("</svg>") {
        problems.push("does not end with </svg>");
    }
    if text.matches('<').count() != text.matches('>').count() {
        problems.push("angle brackets are unbalanced");
    }
    for bad in ["NaN", "Infinity"] {
        if text.contains(bad) {
            problems.push("contains a non-finite number");
        }
    }
    if text.contains("<script") {
        problems.push("contains a script element");
    }

    if problems.is_empty() {
        eprintln!(
            "pte: {path} passes the shallow checks (well-formedness, finiteness, \
             no scripts). This is not a conformance check: no XML parser or \
             renderer is built into this front end."
        );
        Ok(())
    } else {
        Err(TraceError::Validation(
            palette_tracer_core::error::ValidationError::Svg {
                check: "shallow_svg_checks",
                detail: problems.join("; "),
            },
        ))
    }
}

/// §G.1: the *only* list of defaults. Documentation reads this rather than
/// keeping a second copy that can drift.
fn profiles(args: &[String]) -> Result<(), TraceError> {
    let json = args.iter().any(|a| a == "--json");
    let mut all = serde_json::Map::new();

    for profile in Profile::ALL {
        let mut entry = serde_json::Map::new();
        entry.insert("implemented".into(), profile.is_implemented().into());
        match Profile::expand(&TraceConfig::for_profile(profile)) {
            Ok(effective) => {
                entry.insert(
                    "profileSchemaVersion".into(),
                    effective.profile_schema_version.into(),
                );
                entry.insert(
                    "settings".into(),
                    serde_json::to_value(&effective.provenance).unwrap_or_default(),
                );
            }
            Err(error) => {
                entry.insert("refusedBecause".into(), error.to_string().into());
            }
        }
        all.insert(profile.as_str().to_owned(), entry.into());
    }

    let text = serde_json::to_string_pretty(&all).unwrap_or_else(|_| "{}".into());
    if json {
        write_output("-", text.as_bytes())?;
        write_output("-", b"\n")
    } else {
        for profile in Profile::ALL {
            eprintln!(
                "{:<22} {}",
                profile.as_str(),
                if profile.is_implemented() {
                    "implemented"
                } else {
                    "defined, not implemented in this build"
                }
            );
        }
        eprintln!("\nUse --json for every effective field, its value, source and range.");
        Ok(())
    }
}

fn schema() -> Result<(), TraceError> {
    // The schema is the default configuration, which is the shape a caller
    // must produce, plus the version it must declare.
    let text = TraceConfig::default().to_json()?;
    write_output("-", text.as_bytes())?;
    write_output("-", b"\n")
}
