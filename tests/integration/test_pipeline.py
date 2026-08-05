"""
Integration tests for complete end-to-end pipeline execution.
"""

import pytest
import numpy as np
from PIL import Image
from palette_trace.document.image_source import DecodedImageSource
from palette_trace.document.settings_store import create_default_settings
from palette_trace.pipeline.controller import PipelineController

def test_pipeline_execution_synthetic_image():
    # Create 30x30 test image with 2 color blocks (red left, blue right)
    img = Image.new("RGBA", (30, 30), (0, 0, 0, 255))
    arr = np.array(img)
    arr[:, :15] = [255, 0, 0, 255]   # Red
    arr[:, 15:] = [0, 0, 255, 255]   # Blue
    pil_img = Image.fromarray(arr)

    src = DecodedImageSource(pil_img)
    settings = create_default_settings("test-img-uuid")
    settings["scanCount"] = 2

    # Pin red color
    settings["palette"]["entries"][0] = {
        "id": "e_red",
        "name": "Red Scan",
        "enabled": True,
        "kind": "pinned",
        "sourceAnchor": { "srgb": "#FF0000" },
        "output": { "mode": "exact", "color": { "srgb": "#FF0000" } },
        "assignment": {
            "mode": "reserve_within_reach",
            "overallReach": 25,
            "channels": { "mode": "linked" },
        },
        "role": "primary_fill",
        "traceProfile": { "mode": "inherit" },
    }

    controller = PipelineController(src, settings)
    output = controller.run_pipeline()

    assert "scan_results" in output
    assert len(output["scan_results"]) == 2
    assert output["claims_stats"]["e_red"] > 40.0  # Should claim ~50% of image
