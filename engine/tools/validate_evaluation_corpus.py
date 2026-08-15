#!/usr/bin/env python3
"""Validate the clean-room evaluation corpus and its §25.2 manifest."""

from __future__ import annotations

import hashlib
import importlib.util
import json
import shutil
import struct
import sys
import tempfile
from collections import Counter
from pathlib import Path

TOOLS = Path(__file__).resolve().parent
ENGINE = TOOLS.parent
FIXTURES = ENGINE / "fixtures"
MANIFEST = FIXTURES / "manifests" / "evaluation-corpus-v1.json"


class CorpusError(AssertionError):
    pass


def load_module(name: str, path: Path):
    spec = importlib.util.spec_from_file_location(name, path)
    if spec is None or spec.loader is None:
        raise CorpusError(f"cannot load {path}")
    module = importlib.util.module_from_spec(spec)
    sys.modules[name] = module
    spec.loader.exec_module(module)
    return module


PNG = load_module("pte_png_to_ppm", TOOLS / "png_to_ppm.py")
GENERATOR = load_module("pte_make_evaluation_corpus", TOOLS / "make_evaluation_corpus.py")
SCORER = load_module("pte_svg_scorer_validator", TOOLS / "svg_scorer.py")


def sha256(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def check_digest(data: bytes, expected: str, fixture_id: str) -> None:
    actual = sha256(data)
    if actual != expected:
        raise CorpusError(f"{fixture_id}: sha256 {actual}, expected {expected}")


def jpeg_dimensions(data: bytes) -> tuple[int, int, int]:
    if not data.startswith(b"\xff\xd8"):
        raise CorpusError("not a JPEG container")
    offset = 2
    while offset + 4 <= len(data):
        if data[offset] != 0xFF:
            offset += 1
            continue
        marker = data[offset + 1]
        offset += 2
        if marker in (0xD8, 0xD9) or 0xD0 <= marker <= 0xD7:
            continue
        if offset + 2 > len(data):
            break
        length = struct.unpack(">H", data[offset : offset + 2])[0]
        if length < 2 or offset + length > len(data):
            raise CorpusError("truncated JPEG segment")
        if marker in (0xC0, 0xC1, 0xC2):
            precision, height, width, channels = struct.unpack(">BHHB", data[offset + 2 : offset + 8])
            if precision != 8:
                raise CorpusError(f"JPEG precision {precision}, expected 8")
            return width, height, channels
        offset += length
    raise CorpusError("JPEG has no supported start-of-frame marker")


def rgba_pixels(path: Path) -> tuple[int, int, list[tuple[int, int, int, int]]]:
    width, height, channels, raw = PNG.decode(path.read_bytes())
    pixels = []
    for offset in range(0, len(raw), channels):
        values = tuple(raw[offset : offset + channels])
        if channels == 4:
            pixels.append(values)
        elif channels == 3:
            pixels.append(values + (255,))
        elif channels == 2:
            pixels.append((values[0], values[0], values[0], values[1]))
        else:
            pixels.append((values[0], values[0], values[0], 255))
    return width, height, pixels


def parse_hex(value: str) -> tuple[int, int, int, int]:
    text = value.removeprefix("#")
    if len(text) == 6:
        text += "ff"
    if len(text) != 8:
        raise CorpusError(f"invalid RGBA value {value}")
    return tuple(int(text[index : index + 2], 16) for index in range(0, 8, 2))


def require_colors(pixels, required, fixture_id: str) -> None:
    palette = set(pixels)
    missing = [value for value in required if parse_hex(value) not in palette]
    if missing:
        raise CorpusError(f"{fixture_id}: required colours absent: {', '.join(missing)}")


def check_alpha_states(pixels, states, fixture_id: str) -> None:
    alphas = {pixel[3] for pixel in pixels}
    predicates = {
        "zero": any(value == 0 for value in alphas),
        "partial": any(0 < value < 255 for value in alphas),
        "opaque": 255 in alphas,
    }
    missing = [state for state in states if not predicates.get(state, False)]
    if missing:
        raise CorpusError(f"{fixture_id}: missing alpha states: {', '.join(missing)}")


def check_hidden_rgb(pixels, expected: str, fixture_id: str) -> None:
    rgb = parse_hex(expected)[:3]
    if not any(pixel[3] == 0 and pixel[:3] == rgb for pixel in pixels):
        raise CorpusError(f"{fixture_id}: expected RGB {expected} under alpha zero")


def check_uniform_grid(pixels, width: int, height: int, grid: int, fixture_id: str) -> None:
    for y0 in range(0, height, grid):
        for x0 in range(0, width, grid):
            expected = pixels[y0 * width + x0]
            for y in range(y0, min(y0 + grid, height)):
                for x in range(x0, min(x0 + grid, width)):
                    if pixels[y * width + x] != expected:
                        raise CorpusError(f"{fixture_id}: cell at ({x0}, {y0}) is not uniform")


def validate_topology_truth(entry: dict, path: Path) -> None:
    truth = entry.get("topologyTruth")
    if truth is None:
        return
    if not isinstance(truth, dict):
        raise CorpusError(f"{entry['id']}: topologyTruth must be an object")
    distance = truth.get("classificationMaxDistance")
    if isinstance(distance, bool) or not isinstance(distance, (int, float)) or not 0 < distance <= 2:
        raise CorpusError(f"{entry['id']}: invalid topology classification distance")
    labels = truth.get("labels")
    if not isinstance(labels, list) or len(labels) < 2:
        raise CorpusError(f"{entry['id']}: topologyTruth needs at least two labels")
    ids = [label.get("id") for label in labels if isinstance(label, dict)]
    if len(ids) != len(labels) or len(ids) != len(set(ids)):
        raise CorpusError(f"{entry['id']}: topology label IDs must be unique")
    required = {"id", "rgba", "components", "holes", "eulerCharacteristic"}
    for label in labels:
        missing = sorted(required - label.keys())
        if missing:
            raise CorpusError(f"{entry['id']}: topology label {label.get('id')} lacks {missing}")
        try:
            SCORER.parse_rgba(label["rgba"])
        except SCORER.ScoreError as error:
            raise CorpusError(f"{entry['id']}: {error}") from error
        for key in ("components", "holes", "eulerCharacteristic"):
            value = label[key]
            if isinstance(value, bool) or not isinstance(value, int):
                raise CorpusError(f"{entry['id']}: topology {key} must be an integer")
        if label["components"] < 0 or label["holes"] < 0:
            raise CorpusError(f"{entry['id']}: topology counts cannot be negative")
        if label["eulerCharacteristic"] != label["components"] - label["holes"]:
            raise CorpusError(f"{entry['id']}: topology Euler characteristic is inconsistent")
    image = SCORER.decode_png(path)
    classified = SCORER.classify(image, labels, distance)
    signature = SCORER.topology_signature(classified, image.width, image.height, labels)
    if signature["unclassifiedPixels"]:
        raise CorpusError(f"{entry['id']}: topology truth leaves reference pixels unclassified")
    gate = SCORER.compare_topology(signature, truth)
    if not gate["passed"]:
        raise CorpusError(f"{entry['id']}: topology truth disagrees with the reference raster")


def validate_entry(entry: dict) -> None:
    required_fields = {
        "id", "path", "sha256", "split", "class", "intendedProfiles",
        "knownFeatures", "protectedTopology", "permittedExclusions",
        "metricGates", "sourceVectorsAvailable", "generation",
    }
    missing = sorted(required_fields - entry.keys())
    if missing:
        raise CorpusError(f"{entry.get('id', '<unknown>')}: missing manifest fields {missing}")

    path = FIXTURES / entry["path"]
    if not path.is_file():
        raise CorpusError(f"{entry['id']}: missing {path}")
    data = path.read_bytes()
    check_digest(data, entry["sha256"], entry["id"])

    if path.suffix.lower() == ".jpg":
        width, height, channels = jpeg_dimensions(data)
        if (width, height, channels) != (960, 640, 3):
            raise CorpusError(f"{entry['id']}: JPEG is {width}x{height}x{channels}, expected 960x640x3")
        return

    width, height, pixels = rgba_pixels(path)
    expected_dimensions = (320, 240) if entry["class"] == "analytic" else (960, 640)
    if (width, height) != expected_dimensions:
        raise CorpusError(f"{entry['id']}: {width}x{height}, expected {expected_dimensions[0]}x{expected_dimensions[1]}")

    gates = entry["metricGates"]
    if "requiredColors" in gates:
        require_colors(pixels, gates["requiredColors"], entry["id"])
    if "alphaStates" in gates:
        check_alpha_states(pixels, gates["alphaStates"], entry["id"])
    if "hiddenRgb" in gates:
        check_hidden_rgb(pixels, gates["hiddenRgb"], entry["id"])
    if "rareColor" in gates:
        target = parse_hex(gates["rareColor"])
        fraction = sum(pixel == target for pixel in pixels) / len(pixels)
        if not 0 < fraction <= gates["maximumFraction"]:
            raise CorpusError(f"{entry['id']}: rare colour fraction {fraction:.6f} outside gate")
    if "uniformGrid" in gates:
        check_uniform_grid(pixels, width, height, gates["uniformGrid"], entry["id"])
    if "minimumUniqueColors" in gates and len(set(pixels)) < gates["minimumUniqueColors"]:
        raise CorpusError(f"{entry['id']}: only {len(set(pixels))} unique colours")
    validate_topology_truth(entry, path)


def check_regeneration(entries: list[dict]) -> None:
    with tempfile.TemporaryDirectory(prefix="pte-evaluation-") as temporary:
        root = Path(temporary)
        GENERATOR.generate(root)
        for entry in entries:
            if entry["class"] != "analytic":
                continue
            generated = root / entry["path"].removeprefix("synthetic/evaluation/")
            # generate() creates ROOT/synthetic/evaluation; retain the manifest path.
            generated = root / entry["path"]
            committed = FIXTURES / entry["path"]
            if generated.read_bytes() != committed.read_bytes():
                raise CorpusError(f"{entry['id']}: regeneration is not byte-identical")


def validate() -> dict:
    document = json.loads(MANIFEST.read_text(encoding="utf-8"))
    entries = document.get("fixtures", [])
    if len(entries) != 18:
        raise CorpusError(f"manifest contains {len(entries)} fixtures, expected 18")
    ids = [entry["id"] for entry in entries]
    paths = [entry["path"] for entry in entries]
    if len(ids) != len(set(ids)) or len(paths) != len(set(paths)):
        raise CorpusError("fixture IDs and paths must be unique")
    if Counter(entry["split"] for entry in entries) != Counter({"train": 6, "development": 6, "holdout": 6}):
        raise CorpusError("train/development/holdout splits must each contain six fixtures")
    if Counter(entry["class"] for entry in entries) != Counter({"analytic": 13, "generated": 5}):
        raise CorpusError("corpus must contain 13 analytic and 5 generated fixtures")
    topology_ids = {entry["id"] for entry in entries if "topologyTruth" in entry}
    expected_topology_ids = {
        "eval/logo/flat-exact-palette",
        "eval/color/tiny-accent",
        "eval/alpha/hidden-rgb",
        "eval/adversarial/open-vs-closed",
    }
    if topology_ids != expected_topology_ids:
        raise CorpusError("corpus v1 must retain its four machine-readable topology fixtures")
    for entry in entries:
        validate_entry(entry)
    check_regeneration(entries)
    return document


def main() -> int:
    document = validate()
    counts = Counter(entry["split"] for entry in document["fixtures"])
    total_bytes = sum((FIXTURES / entry["path"]).stat().st_size for entry in document["fixtures"])
    print(f"evaluation corpus: 18 valid fixtures ({counts['train']}/{counts['development']}/{counts['holdout']})")
    print(f"13 analytic fixtures regenerate byte-identically; 5 generated fixtures have fixed digests")
    print(f"asset bytes: {total_bytes}")
    return 0


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except CorpusError as error:
        print(f"evaluation corpus invalid: {error}", file=sys.stderr)
        raise SystemExit(1)
