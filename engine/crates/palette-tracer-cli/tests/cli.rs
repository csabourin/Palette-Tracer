//! The committed `pte` command-surface suite (§21).
//!
//! `docs/IMPLEMENTATION_STATUS.md` recorded PTE-API-013/014/015 as "Manual; no
//! committed CLI test suite". This file is that suite. It runs the real binary
//! through `CARGO_BIN_EXE_pte`, so what it checks is the shipped command, not a
//! library call that happens to resemble it.
//!
//! What each group pins:
//!
//! * **PTE-API-013** — `-` means stdin and stdout, and the input format is
//!   resolved from the bytes.
//! * **PTE-API-014** — stream discipline. Nothing but SVG or JSON ever reaches
//!   stdout; every human-readable line goes to stderr. The strongest form of
//!   this test is not "stderr contains a summary" but "stdout is byte-for-byte
//!   the artefact and nothing else", so that is the assertion.
//! * **PTE-API-015** — a flag overrides the same field from `--config`.
//! * **§21.3** — the exit-code table, exercised through the command rather than
//!   through `TraceError::exit_code`, which `error.rs` already covers.
//! * **PTE-DET-001** — two identical invocations produce identical bytes.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

/// What one invocation produced.
struct Run {
    code: i32,
    stdout: Vec<u8>,
    stderr: String,
}

impl Run {
    fn stdout_text(&self) -> &str {
        std::str::from_utf8(&self.stdout).expect("stdout is UTF-8 in every test here")
    }
}

/// Run `pte` with `args`, feeding `stdin` if given.
fn pte(args: &[&str], stdin: Option<&[u8]>) -> Run {
    let mut child = Command::new(env!("CARGO_BIN_EXE_pte"))
        .args(args)
        .stdin(if stdin.is_some() {
            Stdio::piped()
        } else {
            Stdio::null()
        })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("the binary cargo just built must be runnable");

    if let Some(bytes) = stdin {
        child
            .stdin
            .as_mut()
            .expect("stdin was piped")
            .write_all(bytes)
            .expect("the child reads its whole input");
    }

    let output = child.wait_with_output().expect("the child terminates");
    Run {
        // `code()` is `None` only when a signal killed the process, which is
        // itself a failure worth reporting as a distinct number.
        code: output.status.code().unwrap_or(-1),
        stdout: output.stdout,
        stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
    }
}

/// A scratch directory that removes itself, so the suite leaves no litter and
/// two `cargo test` runs never collide.
struct Scratch(PathBuf);

impl Scratch {
    fn new(tag: &str) -> Self {
        // The process id keeps concurrent test binaries apart; the tag keeps
        // tests inside one binary apart, and they run in parallel.
        let dir = std::env::temp_dir().join(format!("pte-cli-{}-{tag}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("a temp directory can be created");
        Self(dir)
    }

    fn path(&self, name: &str) -> PathBuf {
        self.0.join(name)
    }

    fn write(&self, name: &str, bytes: &[u8]) -> PathBuf {
        let path = self.path(name);
        std::fs::write(&path, bytes).expect("the scratch directory is writable");
        path
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn s(path: &Path) -> String {
    path.to_str().expect("scratch paths are UTF-8").to_owned()
}

/// A binary PPM: a red square on a blue field, big enough to produce more than
/// one region and small enough to trace in milliseconds.
fn ppm_fixture() -> Vec<u8> {
    const W: usize = 24;
    const H: usize = 16;
    let mut out = format!("P6\n{W} {H}\n255\n").into_bytes();
    for y in 0..H {
        for x in 0..W {
            let inside = (6..18).contains(&x) && (4..12).contains(&y);
            out.extend_from_slice(if inside {
                &[0xd0, 0x20, 0x20]
            } else {
                &[0x20, 0x40, 0xd0]
            });
        }
    }
    out
}

// --- PTE-API-014: stream discipline ---------------------------------------

/// The load-bearing form of PTE-API-014: when the SVG goes to a file, stdout is
/// *empty*, not merely free of SVG. A summary line leaking to stdout would make
/// `pte trace in - > out.svg` produce a corrupt file, so "nothing else is ever
/// written to stdout" is checked as emptiness.
#[test]
fn tracing_to_a_file_writes_nothing_at_all_to_stdout() {
    let scratch = Scratch::new("to-file");
    let input = scratch.write("in.ppm", &ppm_fixture());
    let output = scratch.path("out.svg");

    let run = pte(&["trace", &s(&input), &s(&output)], None);

    assert_eq!(run.code, 0, "stderr was: {}", run.stderr);
    assert!(
        run.stdout.is_empty(),
        "stdout must be empty, got {:?}",
        run.stdout_text()
    );
    assert!(
        run.stderr.contains("pte:"),
        "the human summary belongs on stderr, got: {}",
        run.stderr
    );
    let svg = std::fs::read_to_string(&output).expect("the SVG was written");
    assert!(
        svg.starts_with("<svg"),
        "got: {}",
        &svg[..svg.len().min(40)]
    );
}

/// PTE-API-013: `-` is stdout, and then stdout carries the SVG and nothing else
/// — no leading progress line, no trailing summary.
#[test]
fn tracing_to_dash_puts_only_the_svg_on_stdout() {
    let scratch = Scratch::new("to-stdout");
    let input = scratch.write("in.ppm", &ppm_fixture());

    let run = pte(&["trace", &s(&input), "-"], None);

    assert_eq!(run.code, 0, "stderr was: {}", run.stderr);
    let svg = run.stdout_text();
    assert!(
        svg.starts_with("<svg"),
        "got: {}",
        &svg[..svg.len().min(40)]
    );
    assert!(svg.trim_end().ends_with("</svg>"));
    assert!(
        !svg.contains("pte:"),
        "the summary must not be interleaved with the SVG"
    );
}

/// The report may go to stdout, but only when the SVG did not. Both on stdout
/// at once would produce two concatenated documents.
#[test]
fn the_report_can_take_stdout_when_the_svg_took_a_file() {
    let scratch = Scratch::new("report-stdout");
    let input = scratch.write("in.ppm", &ppm_fixture());
    let output = scratch.path("out.svg");

    let run = pte(&["trace", "--report", "-", &s(&input), &s(&output)], None);

    assert_eq!(run.code, 0, "stderr was: {}", run.stderr);
    let report: serde_json::Value =
        serde_json::from_str(run.stdout_text()).expect("stdout is exactly one JSON document");
    assert!(report.get("schemaVersion").is_some());
    assert!(
        report.get("unimplemented").is_some(),
        "§0.2: the report says what the build did not do"
    );
}

/// PTE-API-013: stdin, with the format resolved from the bytes rather than a
/// name — a stream has none.
#[test]
fn input_can_arrive_on_stdin() {
    let run = pte(&["trace", "-", "-"], Some(&ppm_fixture()));
    assert_eq!(run.code, 0, "stderr was: {}", run.stderr);
    assert!(run.stdout_text().starts_with("<svg"));
}

// --- §21.3: exit codes ----------------------------------------------------

#[test]
fn an_unknown_subcommand_exits_with_the_configuration_code() {
    let run = pte(&["fly"], None);
    assert_eq!(run.code, 2);
    assert!(run.stdout.is_empty(), "errors never reach stdout");
    assert!(run.stderr.contains("code "), "the stable code is printed");
}

#[test]
fn an_unknown_flag_exits_with_the_configuration_code() {
    let run = pte(&["trace", "--sharpen", "in", "out"], None);
    assert_eq!(run.code, 2);
}

/// PTE-NO-042 through the front end: PNG is refused *by name*, with the reason,
/// rather than being guessed at or reported as a generic parse failure.
#[test]
fn a_png_is_refused_by_name_with_the_input_code() {
    let scratch = Scratch::new("png");
    let png = scratch.write(
        "in.png",
        &[0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a, 0, 0, 0, 0],
    );
    let output = scratch.path("out.svg");

    let run = pte(&["trace", &s(&png), &s(&output)], None);

    assert_eq!(run.code, 3, "input error, not configuration error");
    assert!(
        run.stderr.contains("PNG"),
        "the refusal names the format it found: {}",
        run.stderr
    );
    assert!(
        !output.exists(),
        "a refused trace must not leave a partial output file"
    );
}

#[test]
fn a_truncated_netpbm_exits_with_the_input_code() {
    let scratch = Scratch::new("truncated");
    let mut bytes = ppm_fixture();
    bytes.truncate(bytes.len() / 2);
    let input = scratch.write("short.ppm", &bytes);

    let run = pte(&["trace", &s(&input), &s(&scratch.path("out.svg"))], None);
    assert_eq!(run.code, 3);
}

/// `validate` is shallow by design, and its failure is a *validation* failure,
/// which is exit 6 — a different class from a bad input file.
#[test]
fn validate_separates_a_good_svg_from_a_bad_one_by_exit_code() {
    let scratch = Scratch::new("validate");
    let input = scratch.write("in.ppm", &ppm_fixture());
    let good = scratch.path("good.svg");
    assert_eq!(pte(&["trace", &s(&input), &s(&good)], None).code, 0);

    assert_eq!(pte(&["validate", &s(&good)], None).code, 0);

    let bad = scratch.write("bad.svg", b"<svg><path d=\"M NaN 0\"/></svg>");
    let run = pte(&["validate", &s(&bad)], None);
    assert_eq!(run.code, 6);
    assert!(run.stderr.contains("non-finite") || run.stderr.contains("finite"));
}

/// The engine's own output must pass the front end's checks. If this ever
/// fails, one of the two is wrong and the suite should not let a release
/// decide which.
#[test]
fn every_svg_the_engine_writes_passes_validate() {
    let scratch = Scratch::new("self-validate");
    let input = scratch.write("in.ppm", &ppm_fixture());
    for profile in ["flat-illustration", "coloring-book", "pixel-art", "logo"] {
        let svg = scratch.path(&format!("{profile}.svg"));
        let traced = pte(&["trace", "--profile", profile, &s(&input), &s(&svg)], None);
        assert_eq!(traced.code, 0, "{profile}: {}", traced.stderr);
        assert_eq!(
            pte(&["validate", &s(&svg)], None).code,
            0,
            "{profile} produced an SVG its own front end rejects"
        );
    }
}

/// PTE-NO-042 at the command surface: a profile the build does not implement is
/// refused, and the refusal names the requirement rather than silently
/// downgrading to a profile that is implemented.
#[test]
fn an_unimplemented_profile_is_refused_not_downgraded() {
    let scratch = Scratch::new("unimplemented-profile");
    let input = scratch.write("in.ppm", &ppm_fixture());
    let output = scratch.path("out.svg");

    let run = pte(
        &["trace", "--profile", "engraving", &s(&input), &s(&output)],
        None,
    );

    assert_eq!(run.code, 2, "stderr was: {}", run.stderr);
    assert!(
        !output.exists(),
        "nothing may be written for a refused configuration"
    );
}

// --- PTE-API-015: flag precedence -----------------------------------------

/// An empty `--palette` is refused by name.
///
/// It is still an override, so accepting it would clear a config file's
/// entries and then fail deep inside validation as "palette.mode is `fixed`
/// but no entries were supplied" -- a message that blames the file for what
/// the flag did.
#[test]
fn an_empty_palette_flag_is_refused_by_name() {
    let scratch = Scratch::new("empty-palette");
    let input = scratch.write("in.ppm", &ppm_fixture());

    let run = pte(
        &[
            "trace",
            "--palette",
            "",
            &s(&input),
            &s(&scratch.path("out.svg")),
        ],
        None,
    );

    assert_eq!(run.code, 2, "stderr was: {}", run.stderr);
    assert!(
        run.stderr.contains("--palette"),
        "the flag must be named: {}",
        run.stderr
    );
}

/// A flag beats the same field in `--config`. The check is not that the command
/// succeeded but that the *effective* value is the flag's, read back from
/// `--print-effective-config`, which is the engine's own account of what it
/// used.
#[test]
fn a_flag_overrides_the_same_field_from_the_config_file() {
    let scratch = Scratch::new("precedence");
    let input = scratch.write("in.ppm", &ppm_fixture());

    // The schema command emits the default configuration, so the file below is
    // valid by construction and cannot drift from the config type.
    let schema = pte(&["schema"], None);
    assert_eq!(schema.code, 0, "stderr was: {}", schema.stderr);
    let mut config: serde_json::Value =
        serde_json::from_str(schema.stdout_text()).expect("`pte schema` emits JSON");
    config["geometry"]["curveTolerancePx"] = serde_json::json!(0.9);
    let config_path = scratch.write("config.json", config.to_string().as_bytes());

    let run = pte(
        &[
            "trace",
            "--config",
            &s(&config_path),
            "--curve-tolerance",
            "0.25",
            "--print-effective-config",
            &s(&input),
            &s(&scratch.path("out.svg")),
        ],
        None,
    );

    assert_eq!(run.code, 0, "stderr was: {}", run.stderr);
    let start = run
        .stderr
        .find('{')
        .expect("the effective config is printed to stderr");
    let end = run
        .stderr
        .rfind('}')
        .expect("the effective config is printed to stderr");
    let effective: serde_json::Value =
        serde_json::from_str(&run.stderr[start..=end]).expect("it is JSON");

    let tolerance = effective["geometry"]["curveTolerancePx"]
        .as_f64()
        .expect("the field is a number");
    assert!(
        (tolerance - 0.25).abs() < 1e-12,
        "the flag must win over the config file, got {tolerance}"
    );
    assert_eq!(
        effective["provenance"]["geometry.curveTolerancePx"]["source"],
        serde_json::json!("user"),
        "and the provenance must say the user set it"
    );
}

/// PTE-API-015 is precedence, not argument ordering. A user may put the
/// profile before `--config`; it must still override the file's profile while
/// leaving unrelated file fields intact.
#[test]
fn a_profile_flag_before_the_config_file_still_wins() {
    let scratch = Scratch::new("profile-precedence");
    let input = scratch.write("in.ppm", &ppm_fixture());

    let schema = pte(&["schema"], None);
    assert_eq!(schema.code, 0, "stderr was: {}", schema.stderr);
    let mut config: serde_json::Value = serde_json::from_str(schema.stdout_text()).unwrap();
    config["profile"] = serde_json::json!("flat-illustration");
    config["palette"]["maxColors"] = serde_json::json!(3);
    let config_path = scratch.write("config.json", config.to_string().as_bytes());

    let run = pte(
        &[
            "trace",
            "--profile",
            "logo",
            "--config",
            &s(&config_path),
            "--print-effective-config",
            &s(&input),
            &s(&scratch.path("out.svg")),
        ],
        None,
    );

    assert_eq!(run.code, 0, "stderr was: {}", run.stderr);
    let start = run.stderr.find('{').expect("effective config on stderr");
    let end = run.stderr.rfind('}').expect("effective config on stderr");
    let effective: serde_json::Value =
        serde_json::from_str(&run.stderr[start..=end]).expect("effective config is JSON");

    assert_eq!(effective["profile"], serde_json::json!("logo"));
    assert_eq!(
        effective["palette"]["maxColors"],
        serde_json::json!(3),
        "an unrelated value from the config file must remain in force"
    );
}

/// The config file is still honoured for fields no flag touched: precedence is
/// per field, not "any flag discards the file".
#[test]
fn the_config_file_still_supplies_fields_no_flag_overrides() {
    let scratch = Scratch::new("file-fields");
    let input = scratch.write("in.ppm", &ppm_fixture());

    let schema = pte(&["schema"], None);
    let mut config: serde_json::Value = serde_json::from_str(schema.stdout_text()).unwrap();
    config["palette"]["maxColors"] = serde_json::json!(3);
    let config_path = scratch.write("config.json", config.to_string().as_bytes());

    let run = pte(
        &[
            "trace",
            "--config",
            &s(&config_path),
            "--curve-tolerance",
            "0.4",
            "--report",
            "-",
            &s(&input),
            &s(&scratch.path("out.svg")),
        ],
        None,
    );

    assert_eq!(run.code, 0, "stderr was: {}", run.stderr);
    let report: serde_json::Value = serde_json::from_str(run.stdout_text()).unwrap();
    let palette = report["palette"].as_array().expect("a palette array");
    assert!(
        palette.len() <= 3,
        "maxColors from the file must still bind, got {} entries",
        palette.len()
    );
}

/// An unknown key in a config file is refused (PTE-API-007), and the refusal is
/// a configuration error rather than a decode error.
#[test]
fn an_unknown_config_key_is_refused_at_the_command_surface() {
    let scratch = Scratch::new("unknown-key");
    let input = scratch.write("in.ppm", &ppm_fixture());
    let config = scratch.write(
        "config.json",
        br#"{"schemaVersion": 1, "sharpening": "lots"}"#,
    );

    let run = pte(
        &[
            "trace",
            "--config",
            &s(&config),
            &s(&input),
            &s(&scratch.path("out.svg")),
        ],
        None,
    );
    assert_eq!(run.code, 2);
}

// --- informational subcommands --------------------------------------------

#[test]
fn profiles_json_is_machine_readable_and_names_what_is_not_implemented() {
    let run = pte(&["profiles", "--json"], None);
    assert_eq!(run.code, 0, "stderr was: {}", run.stderr);

    let value: serde_json::Value = serde_json::from_str(run.stdout_text()).expect("JSON on stdout");
    let map = value.as_object().expect("an object keyed by profile");
    assert!(map.contains_key("flat-illustration"));
    assert!(
        map.values()
            .any(|v| v["implemented"] == serde_json::json!(false)),
        "§0.2: profiles this build does not implement are listed as such"
    );
    assert_eq!(
        map["flat-illustration"]["implemented"],
        serde_json::json!(true)
    );
}

/// Without `--json` the listing is for a human, so it goes to stderr and stdout
/// stays clean — the same rule as the trace summary.
#[test]
fn the_human_profile_listing_avoids_stdout() {
    let run = pte(&["profiles"], None);
    assert_eq!(run.code, 0);
    assert!(run.stdout.is_empty());
    assert!(run.stderr.contains("flat-illustration"));
}

/// `pte schema` must emit something the engine itself accepts. Round-tripping
/// it through `--config` is a stronger check than parsing it as JSON.
#[test]
fn the_emitted_schema_is_a_configuration_the_engine_accepts() {
    let scratch = Scratch::new("schema-roundtrip");
    let input = scratch.write("in.ppm", &ppm_fixture());
    let schema = pte(&["schema"], None);
    assert_eq!(schema.code, 0);
    let config = scratch.write("config.json", &schema.stdout);

    let run = pte(
        &[
            "trace",
            "--config",
            &s(&config),
            &s(&input),
            &s(&scratch.path("out.svg")),
        ],
        None,
    );
    assert_eq!(run.code, 0, "stderr was: {}", run.stderr);
}

#[test]
fn version_and_help_go_to_stderr_and_succeed() {
    for args in [vec!["--version"], vec!["--help"], vec![]] {
        let run = pte(&args, None);
        assert_eq!(run.code, 0, "for {args:?}");
        assert!(run.stdout.is_empty(), "for {args:?}");
        assert!(!run.stderr.is_empty(), "for {args:?}");
    }
}

#[test]
fn inspect_writes_a_report_and_no_svg() {
    let scratch = Scratch::new("inspect");
    let input = scratch.write("in.ppm", &ppm_fixture());

    let run = pte(&["inspect", &s(&input)], None);
    assert_eq!(run.code, 0, "stderr was: {}", run.stderr);

    let report: serde_json::Value =
        serde_json::from_str(run.stdout_text()).expect("one JSON document on stdout");
    assert!(report["topology"]["faces"].as_u64().unwrap_or(0) >= 2);
    assert!(
        !run.stdout_text().contains("<svg"),
        "inspect does not emit geometry"
    );
}

// --- PTE-DET-001 through the command --------------------------------------

/// Two machine-readable artefacts cannot share one stream. Concatenating an SVG
/// document and a JSON object yields something that parses as neither, so the
/// combination is refused before any work is done rather than producing a file
/// the caller will discover is corrupt later.
#[test]
fn the_svg_and_the_report_cannot_both_take_stdout() {
    let scratch = Scratch::new("two-artefacts");
    let input = scratch.write("in.ppm", &ppm_fixture());

    let run = pte(&["trace", "--report", "-", &s(&input), "-"], None);

    assert_eq!(run.code, 2, "stderr was: {}", run.stderr);
    assert!(
        run.stdout.is_empty(),
        "the refusal must come before anything is written"
    );
    assert!(
        run.stderr.contains("stdout"),
        "the message says what the conflict is: {}",
        run.stderr
    );
}

/// Determinism as a user sees it: same input, same flags, same bytes. Both
/// artefacts are compared, so a change that preserved the SVG by accident while
/// perturbing the report would still be caught.
#[test]
fn two_identical_invocations_produce_identical_bytes_and_digests() {
    let scratch = Scratch::new("determinism");
    let input = scratch.write("in.ppm", &ppm_fixture());

    let run = |tag: &str| {
        let report = scratch.path(&format!("report-{tag}.json"));
        let out = pte(&["trace", "--report", &s(&report), &s(&input), "-"], None);
        assert_eq!(out.code, 0, "stderr was: {}", out.stderr);
        (out.stdout, std::fs::read(&report).expect("a report"))
    };

    // Separate processes, so nothing in memory can carry between the two.
    let (svg_a, report_a) = run("a");
    let (svg_b, report_b) = run("b");

    assert_eq!(svg_a, svg_b, "the SVG differs between two identical runs");
    assert_eq!(
        report_a, report_b,
        "the report differs between two identical runs"
    );

    let report: serde_json::Value = serde_json::from_slice(&report_a).unwrap();
    let digest = report["semanticDigest"].as_str().expect("a digest");
    assert!(
        digest.starts_with("pte-semantic-v1-blake3:"),
        "got {digest}"
    );
}

/// The output path is written atomically enough that a failure late in the run
/// does not leave a half-SVG: the limit is checked before the file is opened.
#[test]
fn an_output_size_limit_fails_before_writing_a_partial_file() {
    let scratch = Scratch::new("output-limit");
    let input = scratch.write("in.ppm", &ppm_fixture());
    let schema = pte(&["schema"], None);
    let mut config: serde_json::Value = serde_json::from_str(schema.stdout_text()).unwrap();
    config["resources"]["maxOutputBytes"] = serde_json::json!(64);
    let config_path = scratch.write("config.json", config.to_string().as_bytes());
    let output = scratch.path("out.svg");

    let run = pte(
        &[
            "trace",
            "--config",
            &s(&config_path),
            &s(&input),
            &s(&output),
        ],
        None,
    );

    assert_eq!(run.code, 4, "a resource limit, stderr was: {}", run.stderr);
    assert!(!output.exists(), "no partial output file");
}
