"""Negative controls for the clean-room evaluation corpus validator."""

from __future__ import annotations

import importlib.util
import json
import unittest
from pathlib import Path

TOOLS = Path(__file__).resolve().parent
SPEC = importlib.util.spec_from_file_location("validator", TOOLS / "validate_evaluation_corpus.py")
assert SPEC is not None and SPEC.loader is not None
VALIDATOR = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(VALIDATOR)


class EvaluationCorpusTests(unittest.TestCase):
    def test_committed_corpus_is_valid(self):
        document = VALIDATOR.validate()
        self.assertEqual(len(document["fixtures"]), 18)

    def test_required_colour_absence_is_detected(self):
        with self.assertRaisesRegex(VALIDATOR.CorpusError, "required colours absent"):
            VALIDATOR.require_colors([(0, 0, 0, 255)], ["#f5ad29ff"], "negative/color")

    def test_one_byte_change_is_detected(self):
        original = b"fixed corpus input"
        expected = VALIDATOR.sha256(original)
        with self.assertRaisesRegex(VALIDATOR.CorpusError, "sha256"):
            VALIDATOR.check_digest(original[:-1] + b"X", expected, "negative/digest")

    def test_missing_hidden_rgb_is_detected(self):
        pixels = [(0, 0, 0, 0), (223, 41, 97, 255)]
        with self.assertRaisesRegex(VALIDATOR.CorpusError, "under alpha zero"):
            VALIDATOR.check_hidden_rgb(pixels, "#df2961", "negative/hidden-rgb")

    def test_topology_truth_must_match_the_reference_raster(self):
        document = json.loads(VALIDATOR.MANIFEST.read_text(encoding="utf-8"))
        entry = next(
            item for item in document["fixtures"]
            if item["id"] == "eval/logo/flat-exact-palette"
        )
        broken = json.loads(json.dumps(entry))
        broken["topologyTruth"]["labels"][0]["holes"] = 99
        broken["topologyTruth"]["labels"][0]["eulerCharacteristic"] = -97
        with self.assertRaisesRegex(VALIDATOR.CorpusError, "disagrees"):
            VALIDATOR.validate_topology_truth(
                broken, VALIDATOR.FIXTURES / broken["path"]
            )


if __name__ == "__main__":
    unittest.main()
