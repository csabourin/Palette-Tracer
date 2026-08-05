#!/usr/bin/env python3
"""
Palette Trace for Inkscape — Main Extension Entrypoint.
"""

import sys
import os

# Add local extension directory to sys.path
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))

import inkex
from palette_trace.errors import PaletteTraceError
from palette_trace.document.selection import validate_and_get_selected_image, resolve_image_uri
from palette_trace.document.image_source import load_image_source
from palette_trace.document.settings_store import load_or_initialize_settings, save_settings
from palette_trace.document.transforms import get_image_to_document_transform
from palette_trace.document.generated_groups import commit_generated_trace_group
from palette_trace.pipeline.controller import PipelineController
from palette_trace.server.session import AppSession
from palette_trace.server.app_server import launch_palette_trace_app

class PaletteTraceExtension(inkex.EffectExtension):
    """Inkscape Effect Extension for Palette Trace."""

    def effect(self):
        # 1. Validate selection
        image_elem = validate_and_get_selected_image(self.svg, self.svg.selected)

        # 2. Resolve image source URI
        doc_path = self.options.input_file if hasattr(self.options, "input_file") else None
        href_or_data, is_data_uri = resolve_image_uri(image_elem, doc_path)

        # 3. Decode raster source
        image_source = load_image_source(href_or_data, is_data_uri)

        # 4. Load or initialize image settings
        settings = load_or_initialize_settings(image_elem)

        # 5. Initialize session & controller
        session = AppSession(image_elem=image_elem, doc_path=doc_path)
        session.image_source = image_source
        session.settings = settings
        session.controller = PipelineController(image_source, settings)
        session.pipeline_output = session.controller.run_pipeline()

        # 6. Launch Web App UI & Wait for user action
        applied = launch_palette_trace_app(session)

        if applied and session.pipeline_output:
            # Update image settings
            save_settings(image_elem, session.settings)

            # Compute image to document transform
            transform = get_image_to_document_transform(
                image_elem,
                image_source.intrinsic_width,
                image_source.intrinsic_height,
            )

            # Commit generated group atomically
            image_uuid = session.settings["imageUuid"]
            last_group_id = session.settings.get("generation", {}).get("lastGeneratedGroupId")

            root_id = commit_generated_trace_group(
                self.svg,
                image_elem,
                image_uuid,
                session.pipeline_output["scan_results"],
                transform,
                existing_group_id=last_group_id,
            )

            session.settings["generation"]["lastGeneratedGroupId"] = root_id
            save_settings(image_elem, session.settings)

            # Hide source image if requested
            if session.settings.get("output", {}).get("hideSourceImageAfterApply"):
                image_elem.style["display"] = "none"


if __name__ == "__main__":
    PaletteTraceExtension().run()
