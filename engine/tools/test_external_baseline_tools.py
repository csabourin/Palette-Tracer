"""Controls for the pinned external-baseline and renderer adapters (§29)."""

from __future__ import annotations

import importlib.util
import json
import sys
import tempfile
import types
import unittest
from pathlib import Path
from unittest import mock

TOOLS = Path(__file__).resolve().parent


def load(name: str, filename: str):
    spec = importlib.util.spec_from_file_location(name, TOOLS / filename)
    assert spec is not None and spec.loader is not None
    module = importlib.util.module_from_spec(spec)
    sys.modules[spec.name] = module
    spec.loader.exec_module(module)
    return module


VTRACER = load("pte_vtracer_baseline", "vtracer_stable_baseline.py")
INKSCAPE = load("pte_inkscape_renderer", "inkscape_renderer.py")


class ExternalBaselineToolTests(unittest.TestCase):
    def test_vtracer_pin_rejects_an_unreviewed_version(self):
        with mock.patch.object(VTRACER.importlib.metadata, "version", return_value="9.9.9"):
            with self.assertRaisesRegex(VTRACER.BaselineError, "requires 0.6.15"):
                VTRACER.package_identity()

    def test_vtracer_report_names_fairness_limits_and_digests(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            source = root / "input.png"
            output = root / "output.svg"
            report_path = root / "trace.json"
            source.write_bytes(b"controlled raster bytes")

            fake = types.SimpleNamespace(
                convert_image_to_svg_py=lambda _input, target, **_settings: Path(target).write_text(
                    '<svg xmlns="http://www.w3.org/2000/svg"/>', encoding="utf-8"
                )
            )
            identity = {
                "name": "VTracer", "line": "stable", "package": "vtracer",
                "version": "0.6.15", "license": "MIT",
                "nativeExtension": "/controlled/vtracer.so",
                "nativeExtensionSha256": "0" * 64,
            }
            with mock.patch.object(VTRACER, "package_identity", return_value=identity):
                with mock.patch.dict(sys.modules, {"vtracer": fake}):
                    report = VTRACER.run(source, output, report_path)

            self.assertEqual(report["configuration"]["color_precision"], 8)
            self.assertEqual(report["input"]["preprocessing"], "none")
            self.assertIsNone(report["fairness"]["timeoutSeconds"])
            self.assertIn("no fixed-palette", report["fairness"]["palette"])
            self.assertEqual(report["output"]["svgSha256"], VTRACER.sha256_file(output))
            self.assertEqual(json.loads(report_path.read_text()), report)

    def test_inkscape_pin_rejects_an_unreviewed_version(self):
        result = types.SimpleNamespace(returncode=0, stdout="Inkscape 2.0\n", stderr="")
        with mock.patch.object(INKSCAPE.subprocess, "run", return_value=result):
            with self.assertRaisesRegex(INKSCAPE.RendererError, "does not match"):
                INKSCAPE.inkscape_version(Path("inkscape"))

    def test_inkscape_background_contract_maps_diagnostic_magenta(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            svg = root / "input.svg"
            output = root / "output.png"
            executable = root / "inkscape"
            svg.write_text('<svg xmlns="http://www.w3.org/2000/svg"/>')
            executable.write_bytes(b"controlled executable")

            def invoke(command, **_kwargs):
                output.write_bytes(b"controlled PNG")
                self.assertIn("--export-background=#ff00ff", command)
                self.assertIn("--export-background-opacity=255", command)
                self.assertIn("--export-width=40", command)
                self.assertIn("--export-height=30", command)
                return types.SimpleNamespace(returncode=0, stderr=b"")

            with mock.patch.object(INKSCAPE.subprocess, "run", side_effect=invoke):
                INKSCAPE.render(executable, svg, output, 40, 30, "diagnostic")

    def test_committed_first_baseline_evidence_is_internally_consistent(self):
        baseline = TOOLS.parent / "baselines" / "vtracer-0.6.15"
        svg = baseline / "eval-logo-flat-exact-palette.svg"
        trace_path = baseline / "eval-logo-flat-exact-palette-trace.json"
        score_path = baseline / "eval-logo-flat-exact-palette-score.json"
        trace = json.loads(trace_path.read_text(encoding="utf-8"))
        score = json.loads(score_path.read_text(encoding="utf-8"))

        svg_digest = VTRACER.sha256_file(svg)
        self.assertEqual(trace["baseline"]["version"], VTRACER.PINNED_VERSION)
        self.assertEqual(trace["output"]["svgSha256"], svg_digest)
        self.assertEqual(score["candidate"]["svgSha256"], svg_digest)
        self.assertEqual(
            trace["wrapper"]["sha256"],
            VTRACER.sha256_file(TOOLS / "vtracer_stable_baseline.py"),
        )
        self.assertEqual(
            score["scorerSha256"], VTRACER.sha256_file(TOOLS / "svg_scorer.py")
        )
        command_digests = {
            Path(item["path"]).name: item["sha256"]
            for item in score["renderer"]["commandFileDigests"]
        }
        self.assertEqual(
            command_digests["inkscape_renderer.py"],
            VTRACER.sha256_file(TOOLS / "inkscape_renderer.py"),
        )
        self.assertTrue(score["renderer"]["version"].startswith("Inkscape 1.4.2 "))
        self.assertEqual(len(score["renders"]), 12)
        self.assertEqual(
            {(item["scale"], item["background"]) for item in score["renders"]},
            {
                (scale, background)
                for scale in (1, 4, 16)
                for background in ("transparent", "white", "black", "diagnostic")
            },
        )
        self.assertFalse(score["topology"]["gate"]["passed"])
        self.assertEqual(score["topology"]["observed"]["unclassifiedPixels"], 706)


if __name__ == "__main__":
    unittest.main()
