#!/usr/bin/env python3
"""Blindly score an arbitrary SVG against a corpus raster.

The scorer deliberately consumes only the SVG bytes, independently rendered
PNG bytes, and evaluation-corpus truth.  It never reads Palette Tracer's typed
IR or trace report, so the same command can grade an external tracer (§29.1).

The renderer is an explicit command template rather than a hidden library
dependency.  Required placeholders are ``{svg}``, ``{output}``, ``{width}``,
``{height}``, and ``{background}``; ``{scale}`` is also available.  The source
raster is never disclosed to the renderer.  Commands run without a shell.

Example with a renderer adapter named ``render-svg``::

    python3 tools/svg_scorer.py \
      --fixture eval/logo/flat-exact-palette \
      --svg candidate.svg \
      --renderer-command \
        'render-svg {svg} {output} {width} {height} {background}' \
      --renderer-version-command 'render-svg --version' \
      --output score.json

This first slice supports the corpus's 8-bit, non-interlaced PNG references.
The one JPEG fixture needs an explicitly decoded PNG before it can acquire
metric truth; generated fixtures remain comparative-only in corpus v1.
"""

from __future__ import annotations

import argparse
import gzip
import hashlib
import heapq
import importlib.util
import json
import math
import re
import shlex
import shutil
import subprocess
import sys
import tempfile
import xml.etree.ElementTree as ET
from dataclasses import dataclass
from pathlib import Path

TOOLS = Path(__file__).resolve().parent
ENGINE = TOOLS.parent
FIXTURES = ENGINE / "fixtures"
DEFAULT_MANIFEST = FIXTURES / "manifests" / "evaluation-corpus-v1.json"
DEFAULT_SCALES = (1, 4, 16)
DEFAULT_BACKGROUNDS = ("transparent", "white", "black", "diagnostic")
BACKGROUNDS = {
    "transparent": None,
    "white": (255, 255, 255),
    "black": (0, 0, 0),
    "diagnostic": (255, 0, 255),
}
PATH_COMMAND = re.compile(r"[AaCcHhLlMmQqSsTtVvZz]")
NUMBER = re.compile(r"[-+]?(?:\d+\.?(?:\d*)?|\.\d+)(?:[eE][-+]?\d+)?")
EXTERNAL_REFERENCE = re.compile(r"(?:https?|file):", re.IGNORECASE)
SRGB_TO_LINEAR = tuple(
    value / 255 / 12.92
    if value / 255 <= 0.04045
    else ((value / 255 + 0.055) / 1.055) ** 2.4
    for value in range(256)
)


class ScoreError(RuntimeError):
    """A stable, user-facing scoring refusal."""


def _load_png_decoder():
    spec = importlib.util.spec_from_file_location("pte_scorer_png", TOOLS / "png_to_ppm.py")
    if spec is None or spec.loader is None:
        raise ScoreError("cannot load the first-party PNG decoder")
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


PNG = _load_png_decoder()


@dataclass(frozen=True)
class Image:
    width: int
    height: int
    rgba: bytes

    def pixel(self, x: int, y: int) -> tuple[int, int, int, int]:
        offset = (y * self.width + x) * 4
        return tuple(self.rgba[offset : offset + 4])


def decode_png(path: Path) -> Image:
    try:
        width, height, channels, raw = PNG.decode(path.read_bytes())
    except SystemExit as error:
        raise ScoreError(f"{path}: {error}") from error
    rgba = bytearray(width * height * 4)
    destination = 0
    for offset in range(0, len(raw), channels):
        values = raw[offset : offset + channels]
        if channels == 4:
            pixel = values
        elif channels == 3:
            pixel = values + b"\xff"
        elif channels == 2:
            pixel = bytes((values[0], values[0], values[0], values[1]))
        else:
            pixel = bytes((values[0], values[0], values[0], 255))
        rgba[destination : destination + 4] = pixel
        destination += 4
    return Image(width, height, bytes(rgba))


def parse_rgba(value: str) -> tuple[int, int, int, int]:
    text = value.removeprefix("#")
    if len(text) == 6:
        text += "ff"
    if len(text) != 8:
        raise ScoreError(f"invalid RGBA colour {value!r}")
    try:
        return tuple(int(text[index : index + 2], 16) for index in range(0, 8, 2))
    except ValueError as error:
        raise ScoreError(f"invalid RGBA colour {value!r}") from error


def local_name(tag: str) -> str:
    return tag.rsplit("}", 1)[-1]


def has_external_reference(attribute: str, value: str) -> bool:
    stripped = value.strip().strip("\"'")
    if EXTERNAL_REFERENCE.search(stripped):
        return True
    if local_name(attribute) in {"href", "src"}:
        return not stripped.startswith(("#", "data:"))
    for match in re.finditer(r"url\(([^)]*)\)", value, re.IGNORECASE):
        target = match.group(1).strip().strip("\"'")
        if not target.startswith(("#", "data:")):
            return True
    return False


def sha256_file(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as handle:
        while block := handle.read(1024 * 1024):
            digest.update(block)
    return digest.hexdigest()


def _path_counts(data: str) -> dict[str, int]:
    token_pattern = re.compile(rf"(?:{PATH_COMMAND.pattern})|(?:{NUMBER.pattern})")
    tokens = token_pattern.findall(data)
    residual = token_pattern.sub("", data)
    if residual.replace(",", "").strip():
        raise ScoreError("path data contains an unsupported token")
    counts = {"lines": 0, "arcs": 0, "quadratics": 0, "cubics": 0, "controlPoints": 0}
    parameter_counts = {
        "M": 2, "L": 2, "H": 1, "V": 1, "T": 2, "Q": 4,
        "C": 6, "S": 4, "A": 7, "Z": 0,
    }
    point_weights = {
        "M": 1, "L": 1, "H": 1, "V": 1, "T": 1,
        "Q": 2, "S": 2, "C": 3, "A": 1,
    }
    index = 0
    current = None
    while index < len(tokens):
        token = tokens[index]
        if PATH_COMMAND.fullmatch(token):
            current = token.upper()
            index += 1
            if current == "Z":
                counts["lines"] += 1
                current = None
                continue
        elif current is None:
            raise ScoreError("path data begins with coordinates rather than a command")

        arity = parameter_counts[current]
        available = 0
        while index + available < len(tokens) and not PATH_COMMAND.fullmatch(tokens[index + available]):
            available += 1
        if available == 0:
            raise ScoreError(f"path command {current} has no coordinates")
        if available % arity:
            raise ScoreError(f"path command {current} has an incomplete coordinate group")
        groups = available // arity
        for group in range(groups):
            effective = "L" if current == "M" and group > 0 else current
            if effective in {"L", "H", "V"}:
                counts["lines"] += 1
            elif effective == "A":
                counts["arcs"] += 1
            elif effective in {"Q", "S", "T"}:
                counts["quadratics"] += 1
            elif effective == "C":
                counts["cubics"] += 1
            counts["controlPoints"] += point_weights[effective]
        index += available
    return counts


def svg_complexity(path: Path) -> dict:
    data = path.read_bytes()
    if len(data) > 256 * 1024 * 1024:
        raise ScoreError("SVG exceeds the scorer's 256 MiB input limit")
    lowered = data.lower()
    if b"<!doctype" in lowered or b"<!entity" in lowered or b"<?xml-stylesheet" in lowered:
        raise ScoreError("SVG DTDs, entities and external stylesheets are not accepted")
    try:
        root = ET.fromstring(data)
    except ET.ParseError as error:
        raise ScoreError(f"SVG is not well-formed XML: {error}") from error
    if local_name(root.tag) != "svg":
        raise ScoreError("document root is not <svg>")

    result = {
        "visibleElements": 0,
        "images": 0,
        "paths": 0,
        "lines": 0,
        "arcs": 0,
        "quadratics": 0,
        "cubics": 0,
        "controlPoints": 0,
        "primitives": 0,
        "strokes": 0,
        "gradients": 0,
        "gradientStops": 0,
        "groups": 0,
        "uncompressedBytes": len(data),
        "gzipBytes": len(gzip.compress(data, compresslevel=9, mtime=0)),
    }
    visible = {"path", "circle", "ellipse", "rect", "polygon", "polyline", "line", "text", "image"}
    primitives = {"circle", "ellipse", "rect", "polygon"}
    for element in root.iter():
        tag = local_name(element.tag)
        for name, value in element.attrib.items():
            if has_external_reference(name, value):
                raise ScoreError("external SVG references are not accepted by the scorer")
        if tag in {"script", "foreignObject"}:
            raise ScoreError(f"SVG <{tag}> is not accepted by the scorer")
        if tag == "style" and element.text and (
            "@import" in element.text.lower() or has_external_reference("style", element.text)
        ):
            raise ScoreError("external SVG styles are not accepted by the scorer")
        if tag in visible:
            result["visibleElements"] += 1
        if tag == "image":
            result["images"] += 1
        if tag == "path":
            result["paths"] += 1
            counts = _path_counts(element.attrib.get("d", ""))
            for key, value in counts.items():
                result[key] += value
        elif tag == "line":
            result["lines"] += 1
            result["controlPoints"] += 2
        elif tag in {"polyline", "polygon"}:
            point_count = len(NUMBER.findall(element.attrib.get("points", ""))) // 2
            result["lines"] += max(0, point_count - (0 if tag == "polygon" else 1))
            result["controlPoints"] += point_count
        if tag in primitives:
            result["primitives"] += 1
        if tag in {"linearGradient", "radialGradient"}:
            result["gradients"] += 1
        if tag == "stop":
            result["gradientStops"] += 1
        if tag == "g":
            result["groups"] += 1
        style = element.attrib.get("style", "").replace(" ", "").lower()
        stroke = element.attrib.get("stroke", "").strip().lower()
        if (stroke and stroke != "none") or ("stroke:" in style and "stroke:none" not in style):
            result["strokes"] += 1
    return result


def _composite(pixel: tuple[int, int, int, int], background: tuple[int, int, int] | None):
    r, g, b, alpha = pixel
    a = alpha / 255
    if background is None:
        return (SRGB_TO_LINEAR[r] * a, SRGB_TO_LINEAR[g] * a, SRGB_TO_LINEAR[b] * a, a)
    composed = tuple(round(channel * a + back * (1 - a)) for channel, back in zip((r, g, b), background))
    return (*[SRGB_TO_LINEAR[channel] for channel in composed], 1.0)


def _oklab(linear: tuple[float, float, float]) -> tuple[float, float, float]:
    r, g, b = linear
    l = 0.4122214708 * r + 0.5363325363 * g + 0.0514459929 * b
    m = 0.2119034982 * r + 0.6806995451 * g + 0.1073969566 * b
    s = 0.0883024619 * r + 0.2817188376 * g + 0.6299787005 * b
    l_, m_, s_ = math.cbrt(l), math.cbrt(m), math.cbrt(s)
    return (
        0.2104542553 * l_ + 0.7936177850 * m_ - 0.0040720468 * s_,
        1.9779984951 * l_ - 2.4285922050 * m_ + 0.4505937099 * s_,
        0.0259040371 * l_ + 0.7827717662 * m_ - 0.8086757660 * s_,
    )


def _percentile(histogram: list[int], count: int, quantile: float, width: float) -> float:
    target = max(1, math.ceil(count * quantile))
    seen = 0
    for index, value in enumerate(histogram):
        seen += value
        if seen >= target:
            return round(index * width, 6)
    return round((len(histogram) - 1) * width, 6)


def reconstruction_metrics(reference: Image, output: Image, scale: int, background_name: str) -> dict:
    expected = (reference.width * scale, reference.height * scale)
    if (output.width, output.height) != expected:
        raise ScoreError(
            f"rendered PNG is {output.width}x{output.height}, expected {expected[0]}x{expected[1]}"
        )
    background = BACKGROUNDS[background_name]
    count = output.width * output.height
    squared_error = 0.0
    alpha_error = 0.0
    missing = 0
    sum_x = sum_y = sum_xx = sum_yy = sum_xy = 0.0
    de_width = 0.0005
    de_histogram = [0] * 4001
    max_de = 0.0

    for y in range(output.height):
        reference_y = y // scale
        for x in range(output.width):
            ref = _composite(reference.pixel(x // scale, reference_y), background)
            got = _composite(output.pixel(x, y), background)
            channel_error = sum((ref[index] - got[index]) ** 2 for index in range(3))
            squared_error += channel_error
            alpha_error += abs(ref[3] - got[3])
            ref_luma = 0.2126 * ref[0] + 0.7152 * ref[1] + 0.0722 * ref[2]
            got_luma = 0.2126 * got[0] + 0.7152 * got[1] + 0.0722 * got[2]
            sum_x += ref_luma
            sum_y += got_luma
            sum_xx += ref_luma * ref_luma
            sum_yy += got_luma * got_luma
            sum_xy += ref_luma * got_luma
            ref_lab = _oklab(ref[:3])
            got_lab = _oklab(got[:3])
            delta = math.sqrt(sum((a - b) ** 2 for a, b in zip(ref_lab, got_lab)))
            max_de = max(max_de, delta)
            de_histogram[min(len(de_histogram) - 1, round(delta / de_width))] += 1
            if background is None:
                missing += ref[3] >= 0.5 and got[3] <= 0.1
            else:
                back_linear = tuple(SRGB_TO_LINEAR[value] for value in background)
                ref_from_back = math.sqrt(sum((ref[i] - back_linear[i]) ** 2 for i in range(3)))
                got_from_back = math.sqrt(sum((got[i] - back_linear[i]) ** 2 for i in range(3)))
                missing += ref_from_back > 0.08 and got_from_back < 0.015

    mse = squared_error / (count * 3)
    mean_x, mean_y = sum_x / count, sum_y / count
    variance_x = max(0.0, sum_xx / count - mean_x * mean_x)
    variance_y = max(0.0, sum_yy / count - mean_y * mean_y)
    covariance = sum_xy / count - mean_x * mean_y
    c1, c2 = 0.01**2, 0.03**2
    denominator = (mean_x * mean_x + mean_y * mean_y + c1) * (variance_x + variance_y + c2)
    global_ssim = ((2 * mean_x * mean_y + c1) * (2 * covariance + c2)) / denominator
    return {
        "linearRgbMse": round(mse, 12),
        "psnrDb": None if mse == 0 else round(-10 * math.log10(mse), 6),
        "globalSsim": round(global_ssim, 9),
        "alphaMeanAbsoluteError": round(alpha_error / count, 9),
        "missingPatchFraction": round(missing / count, 9),
        "deltaEOk": {
            "p50": _percentile(de_histogram, count, 0.50, de_width),
            "p95": _percentile(de_histogram, count, 0.95, de_width),
            "max": round(max_de, 6),
        },
    }


def classify(image: Image, labels: list[dict], maximum_delta: float) -> list[int]:
    palette = []
    for label in labels:
        rgba = parse_rgba(label["rgba"])
        premultiplied = _composite(rgba, None)
        palette.append(_oklab(premultiplied[:3]) + (premultiplied[3],))
    classified = []
    for offset in range(0, len(image.rgba), 4):
        pixel = tuple(image.rgba[offset : offset + 4])
        premultiplied = _composite(pixel, None)
        lab = _oklab(premultiplied[:3])
        value = lab + (premultiplied[3],)
        distances = [math.sqrt(sum((a - b) ** 2 for a, b in zip(value, target))) for target in palette]
        winner = min(range(len(distances)), key=lambda index: (distances[index], index))
        classified.append(winner if distances[winner] <= maximum_delta else -1)
    return classified


def _components(mask: list[bool], width: int, height: int) -> tuple[int, int]:
    visited = bytearray(width * height)
    components = 0
    border_components = 0
    for start, present in enumerate(mask):
        if not present or visited[start]:
            continue
        components += 1
        stack = [start]
        visited[start] = 1
        touches_border = False
        while stack:
            current = stack.pop()
            x, y = current % width, current // width
            touches_border |= x == 0 or y == 0 or x == width - 1 or y == height - 1
            for neighbour in (
                current - 1 if x else -1,
                current + 1 if x + 1 < width else -1,
                current - width if y else -1,
                current + width if y + 1 < height else -1,
            ):
                if neighbour >= 0 and mask[neighbour] and not visited[neighbour]:
                    visited[neighbour] = 1
                    stack.append(neighbour)
        border_components += touches_border
    return components, border_components


def topology_signature(classified: list[int], width: int, height: int, labels: list[dict]) -> dict:
    by_label = {}
    for index, label in enumerate(labels):
        mask = [value == index for value in classified]
        components, _ = _components(mask, width, height)
        complement_components, complement_border = _components([not value for value in mask], width, height)
        holes = complement_components - complement_border
        by_label[label["id"]] = {
            "components": components,
            "holes": holes,
            "eulerCharacteristic": components - holes,
            "pixels": sum(mask),
        }
    return {"labels": by_label, "unclassifiedPixels": classified.count(-1)}


def _boundary(classified: list[int], width: int, height: int) -> list[int]:
    boundary = []
    for index, value in enumerate(classified):
        x, y = index % width, index // width
        neighbours = []
        if x:
            neighbours.append(index - 1)
        if x + 1 < width:
            neighbours.append(index + 1)
        if y:
            neighbours.append(index - width)
        if y + 1 < height:
            neighbours.append(index + width)
        if any(classified[neighbour] != value for neighbour in neighbours):
            boundary.append(index)
    return boundary


def _distance_map(width: int, height: int, seeds: list[int]) -> list[float]:
    distances = [math.inf] * (width * height)
    queue = []
    for seed in seeds:
        distances[seed] = 0.0
        heapq.heappush(queue, (0.0, seed))
    steps = ((-1, 0, 1.0), (1, 0, 1.0), (0, -1, 1.0), (0, 1, 1.0),
             (-1, -1, math.sqrt(2)), (1, -1, math.sqrt(2)),
             (-1, 1, math.sqrt(2)), (1, 1, math.sqrt(2)))
    while queue:
        distance, index = heapq.heappop(queue)
        if distance != distances[index]:
            continue
        x, y = index % width, index // width
        for dx, dy, cost in steps:
            nx, ny = x + dx, y + dy
            if not (0 <= nx < width and 0 <= ny < height):
                continue
            neighbour = ny * width + nx
            candidate = distance + cost
            if candidate < distances[neighbour]:
                distances[neighbour] = candidate
                heapq.heappush(queue, (candidate, neighbour))
    return distances


def boundary_metrics(reference: list[int], output: list[int], width: int, height: int, tolerance: float) -> dict:
    truth, candidate = _boundary(reference, width, height), _boundary(output, width, height)
    if not truth or not candidate:
        return {
            "referenceSamples": len(truth), "outputSamples": len(candidate),
            "symmetricChamferPx": None, "approximateHausdorffPx": None,
            "precision": 0.0, "recall": 0.0, "fScore": 0.0,
        }
    to_truth = _distance_map(width, height, truth)
    to_candidate = _distance_map(width, height, candidate)
    candidate_distances = [to_truth[index] for index in candidate]
    truth_distances = [to_candidate[index] for index in truth]
    precision = sum(value <= tolerance for value in candidate_distances) / len(candidate_distances)
    recall = sum(value <= tolerance for value in truth_distances) / len(truth_distances)
    f_score = 0.0 if precision + recall == 0 else 2 * precision * recall / (precision + recall)
    return {
        "referenceSamples": len(truth),
        "outputSamples": len(candidate),
        "symmetricChamferPx": round(
            (sum(candidate_distances) / len(candidate_distances) + sum(truth_distances) / len(truth_distances)) / 2,
            6,
        ),
        "approximateHausdorffPx": round(max(max(candidate_distances), max(truth_distances)), 6),
        "precision": round(precision, 9),
        "recall": round(recall, 9),
        "fScore": round(f_score, 9),
        "tolerancePx": tolerance,
    }


def compare_topology(observed: dict, truth: dict) -> dict:
    failures = []
    for label in truth["labels"]:
        expected = {key: label[key] for key in ("components", "holes", "eulerCharacteristic")}
        actual = observed["labels"][label["id"]]
        for key, value in expected.items():
            if actual[key] != value:
                failures.append({"label": label["id"], "metric": key, "expected": value, "actual": actual[key]})
    if observed["unclassifiedPixels"]:
        failures.append({"metric": "unclassifiedPixels", "expected": 0, "actual": observed["unclassifiedPixels"]})
    return {"passed": not failures, "failures": failures}


def load_fixture(manifest_path: Path, fixture_id: str) -> tuple[dict, dict]:
    document = json.loads(manifest_path.read_text(encoding="utf-8"))
    matches = [entry for entry in document.get("fixtures", []) if entry.get("id") == fixture_id]
    if len(matches) != 1:
        raise ScoreError(f"fixture {fixture_id!r} was not found exactly once")
    return document, matches[0]


def renderer_metadata(command_template: str, version_command: str | None) -> dict:
    tokens = shlex.split(command_template)
    if not tokens:
        raise ScoreError("renderer command is empty")
    executable = shutil.which(tokens[0]) or tokens[0]
    executable_path = Path(executable)
    if not executable_path.is_file():
        raise ScoreError(f"renderer executable {tokens[0]!r} was not found")
    command_files = []
    seen_files = set()
    for token in tokens:
        if "{" in token:
            continue
        candidate = Path(shutil.which(token) or token)
        if candidate.is_file():
            resolved = candidate.resolve()
            if resolved not in seen_files:
                seen_files.add(resolved)
                command_files.append({"path": str(resolved), "sha256": sha256_file(resolved)})
    version_tokens = shlex.split(version_command) if version_command else [executable, "--version"]
    try:
        result = subprocess.run(version_tokens, capture_output=True, text=True, timeout=10, check=False)
        version = (result.stdout or result.stderr).strip()[:500]
    except (OSError, subprocess.SubprocessError) as error:
        version = f"unavailable: {error}"
    return {
        "commandTemplate": command_template,
        "commandFileDigests": command_files,
        "executable": str(executable_path.resolve()),
        "executableSha256": sha256_file(executable_path),
        "version": version,
    }


def render(command_template: str, values: dict[str, object], output_path: Path, timeout: int) -> None:
    required = {"svg", "output", "width", "height", "background"}
    missing = sorted(name for name in required if "{" + name + "}" not in command_template)
    if missing:
        raise ScoreError(f"renderer command omits required placeholders: {', '.join(missing)}")
    try:
        command = [token.format_map(values) for token in shlex.split(command_template)]
    except KeyError as error:
        raise ScoreError(f"unknown renderer placeholder {error}") from error
    try:
        result = subprocess.run(command, capture_output=True, timeout=timeout, check=False)
    except (OSError, subprocess.SubprocessError) as error:
        raise ScoreError(f"renderer failed to execute: {error}") from error
    if result.returncode != 0:
        stderr = result.stderr.decode(errors="replace")[:1000]
        raise ScoreError(f"renderer exited {result.returncode}: {stderr}")
    if not output_path.is_file():
        raise ScoreError("renderer returned success without creating its output PNG")


def score(args: argparse.Namespace) -> dict:
    manifest_path = Path(args.manifest).resolve()
    document, fixture = load_fixture(manifest_path, args.fixture)
    source = (manifest_path.parent.parent / fixture["path"]).resolve()
    if not source.is_file():
        raise ScoreError(f"reference raster does not exist: {source}")
    source_digest = sha256_file(source)
    if source_digest != fixture.get("sha256"):
        raise ScoreError(f"{fixture['id']}: reference raster digest does not match the manifest")
    if source.suffix.lower() != ".png":
        raise ScoreError(
            f"{fixture['id']}: corpus v1 marks generated assets comparative-only; "
            "this scorer slice requires an 8-bit PNG reference"
        )
    reference = decode_png(source)
    svg = Path(args.svg).resolve()
    if not svg.is_file():
        raise ScoreError(f"SVG does not exist: {svg}")
    complexity = svg_complexity(svg)
    metadata = renderer_metadata(args.renderer_command, args.renderer_version_command)
    scales = tuple(int(value) for value in args.scales.split(",") if value)
    if not scales or any(value <= 0 or value > 16 for value in scales):
        raise ScoreError("scales must be comma-separated integers in 1..16")
    backgrounds = tuple(value for value in args.backgrounds.split(",") if value)
    if not backgrounds or any(value not in BACKGROUNDS for value in backgrounds):
        raise ScoreError(f"backgrounds must be chosen from {', '.join(BACKGROUNDS)}")

    rendered_metrics = []
    base_transparent = None
    with tempfile.TemporaryDirectory(prefix="pte-svg-score-") as temporary:
        temporary_path = Path(temporary)
        for scale in scales:
            for background in backgrounds:
                output_path = temporary_path / f"render-{scale}-{background}.png"
                values = {
                    "svg": str(svg), "output": str(output_path),
                    "width": reference.width * scale, "height": reference.height * scale,
                    "scale": scale, "background": background,
                }
                render(args.renderer_command, values, output_path, args.timeout)
                rendered = decode_png(output_path)
                metrics = reconstruction_metrics(reference, rendered, scale, background)
                rendered_metrics.append({
                    "scale": scale,
                    "background": background,
                    "renderedPngSha256": sha256_file(output_path),
                    **metrics,
                })
                if scale == 1 and background == "transparent":
                    base_transparent = rendered

    topology = None
    truth = fixture.get("topologyTruth")
    if truth:
        if base_transparent is None:
            raise ScoreError("fixtures with topology truth require scale 1 on a transparent background")
        maximum_delta = truth["classificationMaxDistance"]
        reference_labels = classify(reference, truth["labels"], maximum_delta)
        output_labels = classify(base_transparent, truth["labels"], maximum_delta)
        reference_signature = topology_signature(
            reference_labels, reference.width, reference.height, truth["labels"]
        )
        if not compare_topology(reference_signature, truth)["passed"]:
            raise ScoreError(
                f"{fixture['id']}: manifest topology truth does not match its reference raster"
            )
        observed = topology_signature(output_labels, reference.width, reference.height, truth["labels"])
        topology = {
            "reference": reference_signature,
            "observed": observed,
            "gate": compare_topology(observed, truth),
            "boundary": boundary_metrics(
                reference_labels, output_labels, reference.width, reference.height, args.boundary_tolerance
            ),
        }

    return {
        "schemaVersion": 1,
        "scorer": "pte-blind-svg-v1",
        "scorerSha256": sha256_file(Path(__file__)),
        "corpus": {"id": document["id"], "manifestSha256": sha256_file(manifest_path)},
        "fixture": {
            "id": fixture["id"],
            "split": fixture["split"],
            "class": fixture["class"],
            "referenceSha256": source_digest,
        },
        "candidate": {"svgSha256": sha256_file(svg), "complexity": complexity},
        "renderer": metadata,
        "renders": rendered_metrics,
        "topology": topology,
        "limitations": [
            "globalSsim is a deterministic whole-image statistic, not windowed SSIM",
            "boundary distances use an eight-neighbour raster distance approximation",
            "corpus v1 generated assets are comparative-only and carry no metric truth",
        ],
    }


def build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--manifest", default=str(DEFAULT_MANIFEST))
    parser.add_argument("--fixture", required=True)
    parser.add_argument("--svg", required=True)
    parser.add_argument("--renderer-command", required=True)
    parser.add_argument("--renderer-version-command")
    parser.add_argument("--scales", default=",".join(map(str, DEFAULT_SCALES)))
    parser.add_argument("--backgrounds", default=",".join(DEFAULT_BACKGROUNDS))
    parser.add_argument("--boundary-tolerance", type=float, default=1.0)
    parser.add_argument("--timeout", type=int, default=60)
    parser.add_argument("--output", required=True)
    return parser


def main(argv: list[str] | None = None) -> int:
    parser = build_parser()
    args = parser.parse_args(argv)
    try:
        report = score(args)
        output = Path(args.output)
        output.write_text(json.dumps(report, indent=2, sort_keys=True) + "\n", encoding="utf-8")
        print(f"{output}: scored {args.fixture}")
        return 0
    except (ScoreError, OSError, ValueError, json.JSONDecodeError) as error:
        print(f"SVG scoring failed: {error}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
