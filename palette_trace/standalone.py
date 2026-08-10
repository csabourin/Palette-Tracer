"""
Standalone host entry point (SPEC §9.4).

Runs Palette Trace as a local web application over an image file, with no
Inkscape installation involved. This module must not import `inkex`, directly or
transitively (§9.4.1) — `palette_trace.document` is the Inkscape host and is
deliberately absent from the imports below.

    palette-trace-web path/to/image.png
"""

import argparse
import os
import sys
import uuid
from pathlib import Path

from PIL import Image

from palette_trace import sidecar
from palette_trace.errors import ImageSourceError, PaletteTraceError
from palette_trace.image_source import DecodedImageSource
from palette_trace.pipeline.controller import PipelineController
from palette_trace.presets.preview_scaling import (
    PREVIEW_BUDGET_PIXELS,
    preview_scale_for,
)
from palette_trace.server.app_server import launch_palette_trace_app
from palette_trace.server.session import AppSession
from palette_trace.settings import (
    compute_settings_hash,
    create_default_settings,
    record_generation_provenance,
    source_has_changed,
)
from palette_trace.svg_writer import build_svg_document

#: Formats Pillow can decode that make sense as a trace source.
SUPPORTED_SUFFIXES = {
    ".png", ".jpg", ".jpeg", ".gif", ".bmp", ".tif", ".tiff", ".webp", ".ppm", ".pgm",
}


def default_output_path(image_path: Path) -> Path:
    """`photo.png` becomes `photo.palette-trace.svg`."""
    return image_path.with_name(f"{image_path.stem}.palette-trace.svg")


def load_source(image_path: Path) -> DecodedImageSource:
    """Decodes a local image file into the shared source representation."""
    try:
        with Image.open(image_path) as image:
            image.load()
            return DecodedImageSource(image, mime_type=Image.MIME.get(image.format, "image/png"))
    except (OSError, ValueError) as exc:
        raise ImageSourceError(f"Could not read image {image_path}: {exc}") from exc


def build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(
        prog="palette-trace-web",
        description="Art-directed multicolour bitmap tracing, as a local web application.",
    )
    parser.add_argument(
        "image", nargs="?", type=Path,
        help="path to the source bitmap; omit it to choose one in the browser",
    )
    parser.add_argument(
        "-o", "--output", type=Path,
        help="output SVG path (default: <image stem>.palette-trace.svg)",
    )
    parser.add_argument(
        "--force", action="store_true",
        help="overwrite the output file if it already exists",
    )
    parser.add_argument(
        "--no-browser", action="store_true",
        help="print the interface URL instead of opening a browser",
    )
    parser.add_argument(
        "--ignore-sidecar", action="store_true",
        help="start from destination defaults, ignoring any saved settings",
    )
    parser.add_argument(
        "--host",
        help="interface bind address (default: 127.0.0.1, or 0.0.0.0 when a "
             "PORT environment variable is set, e.g. under Replit)",
    )
    parser.add_argument(
        "--port", type=int,
        help="interface port (default: an ephemeral port, or $PORT when set)",
    )
    return parser


def _default_host_and_port(args: argparse.Namespace) -> tuple[str, int]:
    """
    §9.1 requires binding only to loopback by default. `$PORT` is how
    container platforms such as Replit assign the single externally-reachable
    port, and its presence has no other plausible reading than "this process
    is not running on the user's own desktop" — so its presence, and only its
    presence, opts into the wider bind address needed to be reachable there.
    """
    env_port = os.environ.get("PORT")
    host = args.host or ("0.0.0.0" if env_port else "127.0.0.1")
    port = args.port if args.port is not None else int(env_port) if env_port else 0
    return host, port


def main(argv=None) -> int:
    args = build_parser().parse_args(argv)

    if args.image is None:
        # No path given: open on the interface's image-loading screen and let
        # the user choose in the browser. This is the only workable entry point
        # when the interface is reached from a phone, where the command line is
        # not available and the server's filesystem is not the user's (§9.4.2).
        return run_without_source(args)

    image_path = args.image.expanduser()
    if not image_path.is_file():
        print(f"error: no such file: {image_path}", file=sys.stderr)
        return 2

    if image_path.suffix.lower() not in SUPPORTED_SUFFIXES:
        print(
            f"warning: {image_path.suffix or 'this file'} is not a recognised bitmap "
            "format; attempting to decode anyway.",
            file=sys.stderr,
        )

    output_path = (args.output or default_output_path(image_path)).expanduser()
    if output_path.exists() and not args.force:
        # §9.4.4: never overwrite without an explicit instruction.
        print(
            f"error: {output_path} already exists. Pass --force to overwrite, "
            "or choose another path with --output.",
            file=sys.stderr,
        )
        return 2

    try:
        source = load_source(image_path)
    except PaletteTraceError as exc:
        print(f"error: {exc}", file=sys.stderr)
        return 1

    settings = (
        sidecar.load_settings(image_path) if not args.ignore_sidecar
        else sidecar.load_settings(image_path.with_name("\0nonexistent"))
    )
    settings.setdefault("source", {}).update({
        "intrinsicWidth": source.intrinsic_width,
        "intrinsicHeight": source.intrinsic_height,
        "mimeType": source.mime_type,
    })

    session = AppSession(doc_path=str(image_path))
    session.image_source = source
    session.image_name = image_path.name
    session.output_path = output_path
    session.settings = settings
    session.source_changed = source_has_changed(settings, source.fingerprint)

    # The trace that runs before the browser opens is a preview like any other
    # (§17.4): nothing has been asked for yet, and on a large photograph this is
    # the wait that used to happen before the interface appeared at all. Apply
    # re-traces at full resolution, so the cost moves to the moment the result
    # is actually wanted instead of being paid up front every run — including
    # the runs that end in Cancel.
    session.controller = PipelineController(
        source, settings,
        preview_scale_for(
            source.intrinsic_width, source.intrinsic_height, PREVIEW_BUDGET_PIXELS
        ),
    )

    try:
        session.pipeline_output = session.controller.run_pipeline()
        session.pipeline_output_is_preview = session.controller.is_preview
    except PaletteTraceError as exc:
        print(f"error: {exc}", file=sys.stderr)
        return 1

    if session.source_changed:
        print(
            "note: the source bitmap has changed since these settings were last applied.",
            file=sys.stderr,
        )

    host, port = _default_host_and_port(args)
    if not launch_palette_trace_app(
        session, open_browser=not args.no_browser, host=host, port=port
    ):
        print("Cancelled. Nothing was written.")
        return 0

    if not session.can_write_to_disk:
        # The user replaced the command-line bitmap with one chosen in the
        # browser. Its result was delivered as a download, and the path given
        # on the command line names a different image — writing there would
        # overwrite an unrelated file's output.
        print("Finished. The result was downloaded through the browser.")
        return 0

    return commit(session, image_path, output_path)


def run_without_source(args) -> int:
    """
    Serves the interface with no bitmap, ready for the user to load one.

    Nothing is written to disk in this mode: without a source path there is
    nowhere a sidecar belongs (§9.4.3) and no output path to honour (§9.4.4),
    so results leave through the browser as downloads.
    """
    session = AppSession()
    session.settings = create_default_settings(str(uuid.uuid4()))

    host, port = _default_host_and_port(args)
    launch_palette_trace_app(
        session, open_browser=not args.no_browser, host=host, port=port
    )

    if session.is_cancelled:
        print("Cancelled. Nothing was written.")
    else:
        print("Finished. Any result was downloaded through the browser.")
    return 0


def commit(session: AppSession, image_path: Path, output_path: Path) -> int:
    """Writes the SVG and the sidecar after the user applies."""
    output = session.pipeline_output
    if not output:
        print("error: nothing to write.", file=sys.stderr)
        return 1

    settings = session.settings
    source = session.image_source
    backend = output.get("backend", {})

    record_generation_provenance(
        settings,
        source_hash=source.fingerprint,
        backend_id=backend.get("id", ""),
        backend_version=backend.get("version", ""),
        generated_group_id=str(output_path.name),
    )

    svg = build_svg_document(
        output["scan_results"],
        source.intrinsic_width,
        source.intrinsic_height,
        settings["imageUuid"],
        provenance={
            "settingsHash": compute_settings_hash(settings),
            "sourceHash": source.fingerprint,
            "backendId": backend.get("id", ""),
            "backendVersion": backend.get("version", ""),
        },
    )

    # §9.4.4: atomic write, so an interrupted run cannot truncate the output.
    sidecar.write_atomically(output_path, svg)
    sidecar.save_settings(image_path, settings)

    scan_count = sum(1 for s in output["scan_results"] if s.get("pathDatas"))
    print(f"Wrote {output_path} ({scan_count} scan(s), backend {backend.get('id', '?')}).")
    print(f"Settings saved to {sidecar.sidecar_path(image_path)}.")
    return 0


if __name__ == "__main__":
    sys.exit(main())
