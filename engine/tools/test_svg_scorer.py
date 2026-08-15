"""Evidence and negative controls for the blind SVG scorer (§39 issue 1)."""

from __future__ import annotations

import importlib.util
import json
import sys
import tempfile
import unittest
from pathlib import Path

TOOLS = Path(__file__).resolve().parent
SPEC = importlib.util.spec_from_file_location("pte_svg_scorer", TOOLS / "svg_scorer.py")
assert SPEC is not None and SPEC.loader is not None
SCORER = importlib.util.module_from_spec(SPEC)
sys.modules[SPEC.name] = SCORER
SPEC.loader.exec_module(SCORER)


def image(width: int, height: int, pixels: list[tuple[int, int, int, int]]):
    return SCORER.Image(width, height, bytes(channel for pixel in pixels for channel in pixel))


def truth_from(reference: list[int], width: int, height: int, labels: list[dict]):
    signature = SCORER.topology_signature(reference, width, height, labels)
    return {
        "classificationMaxDistance": 0.12,
        "labels": [
            {"id": label["id"], **{
                key: signature["labels"][label["id"]][key]
                for key in ("components", "holes", "eulerCharacteristic")
            }}
            for label in labels
        ],
    }


class SvgScorerTests(unittest.TestCase):
    def test_exact_pixels_have_zero_error(self):
        candidate = image(2, 2, [
            (255, 0, 0, 255), (0, 255, 0, 255),
            (0, 0, 255, 128), (17, 29, 41, 0),
        ])
        metrics = SCORER.reconstruction_metrics(candidate, candidate, 1, "transparent")
        self.assertEqual(metrics["linearRgbMse"], 0)
        self.assertIsNone(metrics["psnrDb"])
        self.assertEqual(metrics["alphaMeanAbsoluteError"], 0)
        self.assertEqual(metrics["deltaEOk"]["max"], 0)

    def test_a_missing_hole_fails_the_topology_gate(self):
        labels = [{"id": "background"}, {"id": "ring"}]
        reference = [
            0, 0, 0, 0, 0,
            0, 1, 1, 1, 0,
            0, 1, 0, 1, 0,
            0, 1, 1, 1, 0,
            0, 0, 0, 0, 0,
        ]
        filled = reference.copy()
        filled[12] = 1
        expected = truth_from(reference, 5, 5, labels)
        observed = SCORER.topology_signature(filled, 5, 5, labels)
        gate = SCORER.compare_topology(observed, expected)
        self.assertFalse(gate["passed"])
        self.assertIn("holes", {failure["metric"] for failure in gate["failures"]})

    def test_a_missing_tiny_accent_fails_even_when_the_background_survives(self):
        labels = [{"id": "background"}, {"id": "accent"}]
        reference = [0] * 25
        reference[12] = 1
        expected = truth_from(reference, 5, 5, labels)
        observed = SCORER.topology_signature([0] * 25, 5, 5, labels)
        gate = SCORER.compare_topology(observed, expected)
        self.assertFalse(gate["passed"])
        self.assertIn(
            {"label": "accent", "metric": "components", "expected": 1, "actual": 0},
            gate["failures"],
        )

    def test_a_diagnostic_background_exposes_a_missing_seam(self):
        red = (255, 0, 0, 255)
        magenta = (255, 0, 255, 255)
        reference = image(3, 3, [red] * 9)
        output = image(3, 3, [red, magenta, red] * 3)
        metrics = SCORER.reconstruction_metrics(reference, output, 1, "diagnostic")
        self.assertAlmostEqual(metrics["missingPatchFraction"], 1 / 3)
        self.assertGreater(metrics["linearRgbMse"], 0)

    def test_source_pixels_are_not_a_renderer_placeholder(self):
        values = {
            "svg": "candidate.svg",
            "output": "candidate.png",
            "width": 10,
            "height": 10,
            "background": "transparent",
        }
        command = "renderer {svg} {output} {width} {height} {background} {source}"
        with self.assertRaisesRegex(SCORER.ScoreError, "unknown renderer placeholder"):
            SCORER.render(command, values, Path("candidate.png"), 1)

    def test_complexity_is_measured_from_svg_not_an_engine_report(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            simple = root / "simple.svg"
            complex_svg = root / "complex.svg"
            simple.write_text('<svg xmlns="http://www.w3.org/2000/svg"><circle cx="5" cy="5" r="4"/></svg>')
            complex_svg.write_text(
                '<svg xmlns="http://www.w3.org/2000/svg"><g><path '
                'd="M0 0 L10 0 10 10 Z" stroke="#000"/></g></svg>'
            )
            simple_metrics = SCORER.svg_complexity(simple)
            complex_metrics = SCORER.svg_complexity(complex_svg)
            self.assertEqual(simple_metrics["primitives"], 1)
            self.assertEqual(complex_metrics["lines"], 3)
            self.assertGreater(complex_metrics["controlPoints"], simple_metrics["controlPoints"])

    def test_malformed_and_active_svg_are_refused(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            malformed = root / "malformed.svg"
            active = root / "active.svg"
            remote = root / "remote.svg"
            malformed.write_text("<svg><path></svg>")
            active.write_text('<svg xmlns="http://www.w3.org/2000/svg"><script/></svg>')
            remote.write_text('<svg xmlns="http://www.w3.org/2000/svg"><image href="relative.png"/></svg>')
            with self.assertRaisesRegex(SCORER.ScoreError, "well-formed"):
                SCORER.svg_complexity(malformed)
            with self.assertRaisesRegex(SCORER.ScoreError, "script"):
                SCORER.svg_complexity(active)
            with self.assertRaisesRegex(SCORER.ScoreError, "external"):
                SCORER.svg_complexity(remote)

    def test_cli_records_renderer_identity_and_passes_exact_topology(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            svg = root / "candidate.svg"
            renderer = root / "renderer.py"
            report_path = root / "report.json"
            source = SCORER.FIXTURES / "synthetic/evaluation/logos/flat-logo.png"
            svg.write_text('<svg xmlns="http://www.w3.org/2000/svg"><path d="M0 0L1 1"/></svg>')
            renderer.write_text(
                "import shutil, sys\n"
                "# argv: svg, output, width, height, background\n"
                f"shutil.copyfile({str(source)!r}, sys.argv[2])\n"
            )
            command = (
                f"{sys.executable} {renderer} {{svg}} {{output}} "
                "{width} {height} {background}"
            )
            result = SCORER.main([
                "--fixture", "eval/logo/flat-exact-palette",
                "--svg", str(svg),
                "--renderer-command", command,
                "--renderer-version-command", f"{sys.executable} --version",
                "--scales", "1",
                "--backgrounds", "transparent",
                "--output", str(report_path),
            ])
            self.assertEqual(result, 0)
            report = json.loads(report_path.read_text())
            self.assertTrue(report["topology"]["gate"]["passed"])
            self.assertEqual(report["renders"][0]["linearRgbMse"], 0)
            self.assertRegex(report["renderer"]["executableSha256"], r"^[0-9a-f]{64}$")
            self.assertRegex(report["fixture"]["referenceSha256"], r"^[0-9a-f]{64}$")
            self.assertRegex(report["renders"][0]["renderedPngSha256"], r"^[0-9a-f]{64}$")
            self.assertNotIn("{source}", report["renderer"]["commandTemplate"])
            digested_files = {
                item["path"]: item["sha256"]
                for item in report["renderer"]["commandFileDigests"]
            }
            self.assertRegex(digested_files[str(renderer.resolve())], r"^[0-9a-f]{64}$")


if __name__ == "__main__":
    unittest.main()
