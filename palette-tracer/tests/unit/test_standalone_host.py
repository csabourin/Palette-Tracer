"""
Standalone host tests (SPEC §9.4).
"""

import json
import re
import subprocess
import sys

import numpy as np
import pytest
from PIL import Image

from palette_trace import sidecar
from palette_trace.settings import PT_NAMESPACE, create_default_settings
from palette_trace.standalone import build_parser, default_output_path, load_source
from palette_trace.svg_writer import build_svg_document


@pytest.fixture
def image_file(tmp_path):
    arr = np.zeros((20, 20, 4), dtype=np.uint8)
    arr[:, :10] = [255, 0, 0, 255]
    arr[:, 10:] = [0, 0, 255, 255]
    path = tmp_path / "sample.png"
    Image.fromarray(arr, mode="RGBA").save(path)
    return path


# --------------------------------------------------------------------------- #
#  §9.4.1 host contract                                                        #
# --------------------------------------------------------------------------- #

class TestHostSeparation:
    def test_standalone_host_never_imports_inkex(self):
        """
        §9.4.1: the headless core must not depend on Inkscape.

        Run in a subprocess so an `inkex` already imported by another test
        cannot mask a real dependency.
        """
        script = (
            "import sys; import palette_trace.standalone; "
            "sys.exit(1 if 'inkex' in sys.modules else 0)"
        )
        result = subprocess.run([sys.executable, "-c", script], capture_output=True, text=True)
        assert result.returncode == 0, (
            "importing palette_trace.standalone pulled in inkex:\n" + result.stderr
        )

    @pytest.mark.parametrize("module", [
        "palette_trace.settings",
        "palette_trace.sidecar",
        "palette_trace.svg_writer",
        "palette_trace.image_source",
        "palette_trace.pipeline.controller",
        "palette_trace.tracing.registry",
        "palette_trace.color.claims",
        "palette_trace.masks.geometry_policy",
        "palette_trace.presets.profiles",
    ])
    def test_core_module_is_inkex_free(self, module):
        script = (
            f"import sys; import {module}; "
            "sys.exit(1 if 'inkex' in sys.modules else 0)"
        )
        result = subprocess.run([sys.executable, "-c", script], capture_output=True, text=True)
        assert result.returncode == 0, f"{module} imports inkex:\n{result.stderr}"


# --------------------------------------------------------------------------- #
#  §9.4.3 sidecar persistence                                                  #
# --------------------------------------------------------------------------- #

class TestSidecar:
    def test_sidecar_sits_beside_the_image(self, image_file):
        path = sidecar.sidecar_path(image_file)
        assert path.parent == image_file.parent
        assert path.name == "sample.png.palettetrace.json"

    def test_missing_sidecar_yields_defaults(self, image_file):
        settings = sidecar.load_settings(image_file)
        assert settings["schemaVersion"] == 1
        assert settings["scanCount"] == 4

    def test_round_trip_preserves_settings(self, image_file):
        settings = create_default_settings("uuid-1")
        settings["scanCount"] = 7
        sidecar.save_settings(image_file, settings)

        assert sidecar.load_settings(image_file)["scanCount"] == 7

    def test_malformed_sidecar_falls_back_to_defaults(self, image_file):
        """§9.4.3: the sidecar is advisory and must never block opening."""
        sidecar.sidecar_path(image_file).write_text("{not json", encoding="utf-8")
        assert sidecar.load_settings(image_file)["schemaVersion"] == 1

    def test_wrong_schema_version_falls_back_to_defaults(self, image_file):
        sidecar.sidecar_path(image_file).write_text(
            json.dumps({"schemaVersion": 99, "scanCount": 3}), encoding="utf-8")
        assert sidecar.load_settings(image_file)["scanCount"] == 4

    def test_non_object_sidecar_falls_back_to_defaults(self, image_file):
        sidecar.sidecar_path(image_file).write_text("[1, 2, 3]", encoding="utf-8")
        assert sidecar.load_settings(image_file)["schemaVersion"] == 1

    def test_orphaned_sidecar_is_tolerated(self, tmp_path):
        """§9.4.3: deleting the image does not delete the sidecar."""
        orphan = tmp_path / "gone.png"
        sidecar.save_settings(orphan, create_default_settings("uuid-1"))
        assert not orphan.exists()
        assert sidecar.load_settings(orphan)["schemaVersion"] == 1

    def test_saved_sidecar_is_valid_json(self, image_file):
        sidecar.save_settings(image_file, create_default_settings("uuid-1"))
        json.loads(sidecar.sidecar_path(image_file).read_text(encoding="utf-8"))


class TestAtomicWrite:
    def test_content_is_written(self, tmp_path):
        target = tmp_path / "out.svg"
        sidecar.write_atomically(target, "<svg/>")
        assert target.read_text(encoding="utf-8") == "<svg/>"

    def test_existing_file_is_replaced(self, tmp_path):
        target = tmp_path / "out.svg"
        target.write_text("old", encoding="utf-8")
        sidecar.write_atomically(target, "new")
        assert target.read_text(encoding="utf-8") == "new"

    def test_no_temporary_files_are_left_behind(self, tmp_path):
        target = tmp_path / "out.svg"
        sidecar.write_atomically(target, "<svg/>")
        assert [p.name for p in tmp_path.iterdir()] == ["out.svg"]

    def test_missing_parent_directory_is_created(self, tmp_path):
        target = tmp_path / "nested" / "deep" / "out.svg"
        sidecar.write_atomically(target, "<svg/>")
        assert target.exists()


# --------------------------------------------------------------------------- #
#  §9.4.4 standalone output                                                    #
# --------------------------------------------------------------------------- #

class TestSvgWriter:
    @pytest.fixture
    def scans(self):
        return [
            {"entryId": "e1", "name": "Red", "color": "#FF0000", "role": "primary_fill",
             "pathDatas": ["M 0 0 L 10 0 L 10 10 Z"], "fillRule": "evenodd"},
            {"entryId": "e2", "name": "Blue", "color": "#0000FF", "role": "background",
             "isBackground": True, "pathDatas": ["M 0 0 L 20 0 L 20 20 L 0 20 Z"],
             "fillRule": "nonzero"},
        ]

    def test_document_is_well_formed_xml(self, scans):
        from xml.etree import ElementTree

        ElementTree.fromstring(build_svg_document(scans, 20, 20, "uuid-1"))

    def test_viewbox_matches_intrinsic_dimensions(self, scans):
        svg = build_svg_document(scans, 640, 480, "uuid-1")
        assert 'viewBox="0 0 640 480"' in svg

    def test_pt_namespace_is_declared(self, scans):
        assert f'xmlns:pt="{PT_NAMESPACE}"' in build_svg_document(scans, 20, 20, "uuid-1")

    def test_provenance_attributes_are_written(self, scans):
        """§10.3 applies to both hosts."""
        svg = build_svg_document(scans, 20, 20, "uuid-1", provenance={
            "settingsHash": "sh", "sourceHash": "src",
            "backendId": "potrace", "backendVersion": "1.16",
        })
        for fragment in (
            'pt:generated="true"', 'pt:source-image-uuid="uuid-1"',
            'pt:schema-version="1"', 'pt:settings-hash="sh"', 'pt:source-hash="src"',
            'pt:backend-id="potrace"', 'pt:backend-version="1.16"',
        ):
            assert fragment in svg

    def test_scan_groups_are_labelled(self, scans):
        """Criterion 21."""
        svg = build_svg_document(scans, 20, 20, "uuid-1")
        assert 'inkscape:label="01 Red — #FF0000"' in svg
        assert 'inkscape:label="02 Blue — #0000FF"' in svg

    def test_background_scan_is_marked(self, scans):
        assert 'pt:background="true"' in build_svg_document(scans, 20, 20, "uuid-1")

    def test_fill_and_fill_rule_are_applied(self, scans):
        svg = build_svg_document(scans, 20, 20, "uuid-1")
        assert "fill:#FF0000;fill-rule:evenodd" in svg
        assert "fill:#0000FF;fill-rule:nonzero" in svg

    def test_empty_scans_produce_no_group(self):
        svg = build_svg_document(
            [{"entryId": "e1", "name": "Empty", "color": "#000000", "pathDatas": []}],
            20, 20, "uuid-1")
        assert "<path" not in svg

    def test_no_raster_data_is_embedded(self, scans):
        """§9.4.4: the source bitmap is not embedded unless requested."""
        assert "data:image" not in build_svg_document(scans, 20, 20, "uuid-1")

    def test_path_data_is_attribute_escaped(self):
        svg = build_svg_document(
            [{"entryId": "e1", "name": 'Quote " & <tag>', "color": "#000000",
              "pathDatas": ["M 0 0 Z"]}],
            10, 10, "uuid-1")
        from xml.etree import ElementTree

        ElementTree.fromstring(svg)
        assert "<tag>" not in svg.replace("&lt;tag&gt;", "")


# --------------------------------------------------------------------------- #
#  CLI                                                                          #
# --------------------------------------------------------------------------- #

class TestCommandLine:
    def test_default_output_path_is_derived_from_the_image(self, tmp_path):
        assert default_output_path(tmp_path / "photo.png").name == "photo.palette-trace.svg"

    def test_image_argument_is_optional(self):
        """
        §9.4.2: omitting the path opens the interface's image-loading screen.

        Requiring a path on the command line makes the standalone host
        unusable from a device that has no shell and no access to the server's
        filesystem, which is every phone.
        """
        assert build_parser().parse_args([]).image is None

    def test_output_and_flags_parse(self, tmp_path):
        args = build_parser().parse_args(
            [str(tmp_path / "a.png"), "-o", str(tmp_path / "b.svg"), "--force", "--no-browser"])
        assert args.force and args.no_browser
        assert args.output.name == "b.svg"

    def test_missing_file_exits_with_a_message(self, tmp_path, capsys):
        from palette_trace.standalone import main

        assert main([str(tmp_path / "nope.png")]) == 2
        assert "no such file" in capsys.readouterr().err

    def test_existing_output_is_not_overwritten_without_force(self, image_file, capsys):
        """§9.4.4: never overwrite without an explicit instruction."""
        from palette_trace.standalone import main

        output = default_output_path(image_file)
        output.write_text("existing", encoding="utf-8")

        assert main([str(image_file), "--no-browser"]) == 2
        assert "already exists" in capsys.readouterr().err
        assert output.read_text(encoding="utf-8") == "existing"

    def test_source_decodes_through_the_shared_representation(self, image_file):
        source = load_source(image_file)
        assert (source.intrinsic_width, source.intrinsic_height) == (20, 20)
        assert source.fingerprint
        assert source.oklch.shape == (20, 20, 3)


# --------------------------------------------------------------------------- #
#  §9.4.2 starting with no image                                               #
# --------------------------------------------------------------------------- #

class TestStartingWithoutAnImage:
    """
    A host launched with no path serves the interface's loading screen, and
    delivers whatever the user then traces as a download.
    """

    @pytest.fixture
    def captured_session(self, monkeypatch):
        """Runs `main` without actually serving, capturing the session built."""
        from palette_trace import standalone

        captured = {}

        def fake_launch(session, **kwargs):
            captured["session"] = session
            return session.is_applied

        monkeypatch.setattr(standalone, "launch_palette_trace_app", fake_launch)
        return captured

    def test_no_path_serves_a_session_with_no_image(self, captured_session, capsys):
        from palette_trace.standalone import main

        assert main(["--no-browser"]) == 0

        session = captured_session["session"]
        assert session.has_image is False
        assert session.settings["schemaVersion"] == 1
        assert session.commit_target == "download"
        assert session.can_load_image is True

    def test_no_path_writes_nothing_to_disk(self, captured_session, tmp_path, monkeypatch):
        monkeypatch.chdir(tmp_path)
        from palette_trace.standalone import main

        main(["--no-browser"])
        assert list(tmp_path.iterdir()) == []

    def test_a_path_still_binds_the_session_to_that_file(self, captured_session, image_file):
        from palette_trace.standalone import main

        main([str(image_file), "--no-browser"])

        session = captured_session["session"]
        assert session.image_name == "sample.png"
        assert session.output_path == default_output_path(image_file)
        assert session.commit_target == "file"

    def test_swapping_the_image_in_the_browser_leaves_the_named_output_alone(
        self, captured_session, image_file, monkeypatch, capsys
    ):
        """
        The command line named one picture; the user traced a different one.
        Writing the result to the first picture's output path would overwrite
        an unrelated file (§9.4.4).
        """
        from palette_trace import standalone
        from palette_trace.server.session import ORIGIN_UPLOAD

        def fake_launch(session, **kwargs):
            captured_session["session"] = session
            session.origin = ORIGIN_UPLOAD      # as /api/load_image would
            session.output_path = None
            session.is_applied = True
            return True

        monkeypatch.setattr(standalone, "launch_palette_trace_app", fake_launch)

        assert standalone.main([str(image_file), "--no-browser"]) == 0
        assert not default_output_path(image_file).exists()
        assert "downloaded" in capsys.readouterr().out


# --------------------------------------------------------------------------- #
#  §9.4 host parity                                                            #
# --------------------------------------------------------------------------- #

class TestHostParity:
    def test_both_hosts_produce_the_same_geometry(self, image_file):
        """
        §9.4: identical inputs must produce identical geometry in either host.

        The hosts differ only in where settings come from and where output goes,
        so the same settings must drive the same controller to the same paths.
        """
        from palette_trace.pipeline.controller import PipelineController

        settings = create_default_settings("uuid-1")
        settings["scanCount"] = 2

        via_sidecar = sidecar.save_settings(image_file, settings) and \
            sidecar.load_settings(image_file)
        standalone_output = PipelineController(load_source(image_file), via_sidecar).run_pipeline()

        # The Inkscape host would arrive at the same settings dict from
        # pt:settings, which is the same JSON round trip.
        inkscape_equivalent = json.loads(json.dumps(settings))
        extension_output = PipelineController(
            load_source(image_file), inkscape_equivalent).run_pipeline()

        assert [s["pathDatas"] for s in standalone_output["scan_results"]] == \
               [s["pathDatas"] for s in extension_output["scan_results"]]
