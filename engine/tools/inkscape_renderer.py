#!/usr/bin/env python3
"""Adapt the blind scorer command contract to a pinned Inkscape CLI."""

from __future__ import annotations

import argparse
import subprocess
import sys
from pathlib import Path

PINNED_VERSION_PREFIX = "Inkscape 1.4.2 "
BACKGROUNDS = {
    "transparent": ("#000000", "0"),
    "white": ("#ffffff", "255"),
    "black": ("#000000", "255"),
    "diagnostic": ("#ff00ff", "255"),
}


class RendererError(RuntimeError):
    """A stable refusal from the Inkscape adapter."""


def inkscape_version(executable: Path) -> str:
    try:
        result = subprocess.run(
            [str(executable), "--version"],
            capture_output=True,
            text=True,
            timeout=10,
            check=False,
        )
    except (OSError, subprocess.SubprocessError) as error:
        raise RendererError(f"could not query Inkscape: {error}") from error
    version = (result.stdout or result.stderr).strip()
    if result.returncode != 0:
        raise RendererError(f"Inkscape version command exited {result.returncode}: {version}")
    if not version.startswith(PINNED_VERSION_PREFIX):
        raise RendererError(
            f"renderer version {version!r} does not match {PINNED_VERSION_PREFIX!r}"
        )
    return version


def windows_path(path: Path) -> str:
    """Translate WSL paths only when invoking a Windows renderer."""
    try:
        result = subprocess.run(
            ["wslpath", "-w", str(path)],
            capture_output=True,
            text=True,
            timeout=5,
            check=False,
        )
    except (OSError, subprocess.SubprocessError) as error:
        raise RendererError(f"could not translate WSL path {path}: {error}") from error
    if result.returncode != 0:
        raise RendererError(f"could not translate WSL path {path}: {result.stderr.strip()}")
    return result.stdout.strip()


def render(
    executable: Path,
    svg: Path,
    output: Path,
    width: int,
    height: int,
    background: str,
) -> None:
    if background not in BACKGROUNDS:
        raise RendererError(f"unsupported scorer background: {background}")
    if width <= 0 or height <= 0:
        raise RendererError("render dimensions must be positive")
    color, opacity = BACKGROUNDS[background]
    is_windows = executable.suffix.lower() == ".exe"
    svg_arg = windows_path(svg) if is_windows else str(svg)
    output_arg = windows_path(output) if is_windows else str(output)
    command = [
        str(executable),
        svg_arg,
        "--export-area-page",
        "--export-overwrite",
        f"--export-filename={output_arg}",
        f"--export-width={width}",
        f"--export-height={height}",
        f"--export-background={color}",
        f"--export-background-opacity={opacity}",
    ]
    try:
        result = subprocess.run(command, capture_output=True, timeout=60, check=False)
    except (OSError, subprocess.SubprocessError) as error:
        raise RendererError(f"Inkscape render failed to execute: {error}") from error
    if result.returncode != 0:
        stderr = result.stderr.decode(errors="replace")[:1000]
        raise RendererError(f"Inkscape render exited {result.returncode}: {stderr}")
    if not output.is_file():
        raise RendererError("Inkscape returned without producing its PNG")


def build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--inkscape", required=True)
    parser.add_argument("--version", action="store_true")
    parser.add_argument("svg", nargs="?")
    parser.add_argument("output", nargs="?")
    parser.add_argument("width", nargs="?", type=int)
    parser.add_argument("height", nargs="?", type=int)
    parser.add_argument("background", nargs="?")
    return parser


def main(argv: list[str] | None = None) -> int:
    args = build_parser().parse_args(argv)
    try:
        executable = Path(args.inkscape).resolve()
        if not executable.is_file():
            raise RendererError(f"Inkscape executable does not exist: {executable}")
        version = inkscape_version(executable)
        if args.version:
            print(version)
            return 0
        missing = [
            name for name in ("svg", "output", "width", "height", "background")
            if getattr(args, name) is None
        ]
        if missing:
            raise RendererError(f"missing render arguments: {', '.join(missing)}")
        render(
            executable,
            Path(args.svg).resolve(),
            Path(args.output).resolve(),
            args.width,
            args.height,
            args.background,
        )
        return 0
    except (RendererError, OSError, ValueError) as error:
        print(f"Inkscape adapter failed: {error}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
