#!/usr/bin/env python3
"""
Palette Trace for Inkscape — extension entry point (SPEC §8.1, §24).

This is the Inkscape host. The standalone host is `palette_trace.standalone`;
both drive the same `PipelineController`, so results are identical (§9.4).
"""

import os
import sys

# Inkscape runs extensions from their own directory, which is not on sys.path.
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))

import inkex

from palette_trace.diagnostics import generate_diagnostic_report
from palette_trace.document.generated_groups import commit_generated_trace_group
from palette_trace.document.selection import (
    resolve_image_uri,
    validate_and_get_selected_image,
)
from palette_trace.document.settings_store import (
    load_or_initialize_settings,
    save_settings,
)
from palette_trace.document.transforms import get_image_to_document_transform
from palette_trace.errors import PaletteTraceError
from palette_trace.image_source import load_image_source
from palette_trace.pipeline.controller import PipelineController
from palette_trace.server.app_server import launch_palette_trace_app
from palette_trace.server.session import AppSession
from palette_trace.settings import (
    compute_settings_hash,
    record_generation_provenance,
    source_has_changed,
)


class PaletteTraceExtension(inkex.EffectExtension):
    """Inkscape effect extension for Palette Trace."""

    def effect(self):
        try:
            self._run()
        except PaletteTraceError as exc:
            # Nothing has been committed at this point, so the document is
            # untouched (§30, §34.29).
            inkex.errormsg(str(exc))
        except Exception as exc:                                # noqa: BLE001
            report = generate_diagnostic_report(exc)
            inkex.errormsg(
                "Palette Trace failed and the document was not modified.\n\n"
                f"{type(exc).__name__}: {exc}\n\n"
                f"Diagnostics: {report}"
            )

    def _run(self):
        # Stage 1: startup and selection
        image_elem = validate_and_get_selected_image(self.svg, self.svg.selected)

        doc_path = getattr(self.options, "input_file", None)
        href_or_data, is_data_uri = resolve_image_uri(image_elem, doc_path)

        # Stage 2: source decoding
        image_source = load_image_source(href_or_data, is_data_uri)

        # Stage 4: settings
        settings = load_or_initialize_settings(image_elem)
        settings.setdefault("source", {}).update({
            "intrinsicWidth": image_source.intrinsic_width,
            "intrinsicHeight": image_source.intrinsic_height,
            "mimeType": image_source.mime_type,
        })

        # §27: a recorded fingerprint that no longer matches means the bitmap
        # changed since this configuration was last applied.
        session = AppSession(image_elem=image_elem, doc_path=doc_path)
        session.source_changed = source_has_changed(settings, image_source.fingerprint)
        # Shown as the interface's subject line. The Inkscape label is the name
        # the user gave the image; falling back to a generic phrase keeps local
        # filesystem paths out of browser-visible data (§9.1).
        session.image_name = image_elem.get("inkscape:label") or "Selected image"

        session.image_source = image_source
        session.settings = settings
        session.controller = PipelineController(image_source, settings)
        session.pipeline_output = session.controller.run_pipeline()

        # Stages 5-12 run inside the controller; the interface drives re-runs.
        if not launch_palette_trace_app(session):
            return

        settings = session.settings
        output = session.pipeline_output
        if not output:
            return

        # Stage 13: atomic commit
        transform = get_image_to_document_transform(
            image_elem, image_source.intrinsic_width, image_source.intrinsic_height
        )

        backend = output.get("backend", {})
        record_generation_provenance(
            settings,
            source_hash=image_source.fingerprint,
            backend_id=backend.get("id", ""),
            backend_version=backend.get("version", ""),
        )

        root_id = commit_generated_trace_group(
            self.svg,
            image_elem,
            settings["imageUuid"],
            output["scan_results"],
            transform,
            existing_group_id=settings.get("generation", {}).get("lastGeneratedGroupId"),
            provenance={
                "settingsHash": compute_settings_hash(settings),
                "sourceHash": image_source.fingerprint,
                "backendId": backend.get("id", ""),
                "backendVersion": backend.get("version", ""),
            },
        )

        settings["generation"]["lastGeneratedGroupId"] = root_id
        save_settings(image_elem, settings)

        if settings.get("output", {}).get("hideSourceImageAfterApply"):
            image_elem.style["display"] = "none"


if __name__ == "__main__":
    PaletteTraceExtension().run()
