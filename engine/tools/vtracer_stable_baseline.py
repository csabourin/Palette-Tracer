#!/usr/bin/env python3
"""Run the pinned VTracer stable baseline as an independent package (§29.1)."""

from __future__ import annotations

import argparse
import hashlib
import importlib.metadata
import importlib.util
import json
import platform
import resource
import sys
import time
from pathlib import Path

PINNED_VERSION = "0.6.15"
SETTINGS = {
    "colormode": "color",
    "hierarchical": "cutout",
    "mode": "spline",
    "filter_speckle": 0,
    "color_precision": 8,
    "layer_difference": 0,
    "corner_threshold": 60,
    "length_threshold": 4.0,
    "max_iterations": 10,
    "splice_threshold": 45,
    "path_precision": 8,
}


class BaselineError(RuntimeError):
    """A stable refusal from the reproducible baseline wrapper."""


def sha256_file(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as stream:
        for chunk in iter(lambda: stream.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def package_identity() -> dict:
    version = importlib.metadata.version("vtracer")
    if version != PINNED_VERSION:
        raise BaselineError(
            f"VTracer {version} is installed; this baseline requires {PINNED_VERSION}"
        )
    spec = importlib.util.find_spec("vtracer.vtracer")
    if spec is None or spec.origin is None:
        raise BaselineError("the VTracer native extension could not be located")
    native = Path(spec.origin).resolve()
    return {
        "name": "VTracer",
        "line": "stable",
        "package": "vtracer",
        "version": version,
        "license": "MIT",
        "nativeExtension": str(native),
        "nativeExtensionSha256": sha256_file(native),
    }


def peak_rss_bytes() -> int:
    value = resource.getrusage(resource.RUSAGE_SELF).ru_maxrss
    # Linux reports KiB; macOS reports bytes. Record the platform beside it.
    return int(value if sys.platform == "darwin" else value * 1024)


def run(input_path: Path, output_path: Path, report_path: Path) -> dict:
    identity = package_identity()
    if not input_path.is_file():
        raise BaselineError(f"input raster does not exist: {input_path}")
    output_path.parent.mkdir(parents=True, exist_ok=True)
    report_path.parent.mkdir(parents=True, exist_ok=True)

    import vtracer  # Imported only after the package/version refusal above.

    started = time.perf_counter()
    vtracer.convert_image_to_svg_py(
        str(input_path), str(output_path), **SETTINGS
    )
    elapsed = time.perf_counter() - started
    if not output_path.is_file():
        raise BaselineError("VTracer returned without producing its SVG")

    report = {
        "schemaVersion": 1,
        "baseline": identity,
        "wrapper": {
            "path": str(Path(__file__).resolve()),
            "sha256": sha256_file(Path(__file__)),
        },
        "input": {
            "path": str(input_path),
            "sha256": sha256_file(input_path),
            "preprocessing": "none",
        },
        "configuration": SETTINGS,
        "fairness": {
            "colorCount": "not capped; stable Python API exposes no maximum-color setting",
            "palette": "not supplied; stable Python API exposes no fixed-palette setting",
            "timeoutSeconds": None,
            "memoryLimitBytes": None,
            "unsupportedSemantics": [
                "fixed role-aware palette",
                "protected topology declaration",
                "fabrication semantics",
            ],
        },
        "measurement": {
            "wallTimeSeconds": round(elapsed, 6),
            "peakRssBytes": peak_rss_bytes(),
            "platform": platform.platform(),
            "python": platform.python_version(),
        },
        "output": {
            "path": str(output_path),
            "svgSha256": sha256_file(output_path),
            "bytes": output_path.stat().st_size,
        },
    }
    report_path.write_text(
        json.dumps(report, indent=2, sort_keys=True) + "\n", encoding="utf-8"
    )
    return report


def build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--version", action="store_true")
    parser.add_argument("--input")
    parser.add_argument("--output")
    parser.add_argument("--report")
    return parser


def main(argv: list[str] | None = None) -> int:
    args = build_parser().parse_args(argv)
    try:
        if args.version:
            identity = package_identity()
            print(f"{identity['name']} {identity['version']} ({identity['line']})")
            return 0
        missing = [name for name in ("input", "output", "report") if not getattr(args, name)]
        if missing:
            raise BaselineError(f"missing required arguments: {', '.join(missing)}")
        report = run(
            Path(args.input).resolve(),
            Path(args.output).resolve(),
            Path(args.report).resolve(),
        )
        print(
            f"{args.output}: VTracer {report['baseline']['version']} in "
            f"{report['measurement']['wallTimeSeconds']:.6f}s"
        )
        return 0
    except (BaselineError, OSError, ValueError) as error:
        print(f"VTracer baseline failed: {error}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
