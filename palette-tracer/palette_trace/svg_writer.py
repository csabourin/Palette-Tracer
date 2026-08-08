"""
Standalone SVG assembly (SPEC §9.4.4, §10.3, §10.4).

Produces the same group structure, labels and `pt:` attributes as the Inkscape
host, without importing `inkex` — the standalone host must work with no Inkscape
installation present (§9.4.1).
"""

import uuid
from xml.sax.saxutils import escape, quoteattr

from palette_trace.settings import PT_NAMESPACE, PT_SCHEMA_VERSION

SVG_NAMESPACE = "http://www.w3.org/2000/svg"
INKSCAPE_NAMESPACE = "http://www.inkscape.org/namespaces/inkscape"


def _attr(name: str, value) -> str:
    return f"{name}={quoteattr(str(value))}"


def build_svg_document(
    scan_results: list,
    width: int,
    height: int,
    image_uuid: str,
    provenance: dict = None,
    group_id: str = None,
) -> str:
    """
    Builds a complete standalone SVG document.

    Geometry is emitted in source-pixel coordinates and the root `viewBox`
    matches the intrinsic source dimensions, so no per-path transform is needed —
    the document *is* the source coordinate system (§9.4.4).
    """
    provenance = provenance or {}
    group_id = group_id or f"palette-trace-result-{uuid.uuid4().hex[:8]}"

    lines = [
        '<?xml version="1.0" encoding="UTF-8" standalone="no"?>',
        "<svg "
        + " ".join([
            _attr("xmlns", SVG_NAMESPACE),
            _attr("xmlns:inkscape", INKSCAPE_NAMESPACE),
            _attr("xmlns:pt", PT_NAMESPACE),
            _attr("width", width),
            _attr("height", height),
            _attr("viewBox", f"0 0 {width} {height}"),
            _attr("version", "1.1"),
        ])
        + ">",
        f"  <title>{escape('Palette Trace result')}</title>",
        "  <g "
        + " ".join([
            _attr("id", group_id),
            _attr("inkscape:label", "Palette Trace"),
            _attr("inkscape:groupmode", "layer"),
            _attr("pt:generated", "true"),
            _attr("pt:source-image-uuid", image_uuid),
            _attr("pt:schema-version", PT_SCHEMA_VERSION),
            _attr("pt:settings-hash", provenance.get("settingsHash", "")),
            _attr("pt:source-hash", provenance.get("sourceHash", "")),
            _attr("pt:backend-id", provenance.get("backendId", "")),
            _attr("pt:backend-version", provenance.get("backendVersion", "")),
        ])
        + ">",
    ]

    for index, scan in enumerate(scan_results):
        path_datas = [d for d in scan.get("pathDatas", []) if d]
        if not path_datas:
            continue

        hex_color = scan.get("color", "#000000")
        fill_rule = scan.get("fillRule", "evenodd")
        label = f"{index + 1:02d} {scan.get('name', f'Scan {index + 1}')} — {hex_color}"

        group_attrs = [
            _attr("id", f"palette-trace-scan-{uuid.uuid4().hex[:8]}"),
            _attr("inkscape:label", label),
            _attr("inkscape:groupmode", "layer"),
            _attr("pt:palette-entry-id", scan["entryId"]),
            _attr("pt:role", scan.get("role", "primary_fill")),
        ]
        if scan.get("isBackground"):
            group_attrs.append(_attr("pt:background", "true"))

        lines.append("    <g " + " ".join(group_attrs) + ">")
        # All sub-paths are combined into one <path> element so that the
        # fill-rule (even-odd) correctly punches holes: inner contours cancel
        # out the winding and produce transparent cutouts. Separate <path>
        # elements each get filled independently, so inner paths render solid.
        combined_d = " ".join(path_datas)
        style = f"fill:{hex_color};fill-rule:{fill_rule};stroke:none"
        lines.append(f"      <path {_attr('d', combined_d)} {_attr('style', style)}/>")
        lines.append("    </g>")

    lines.append("  </g>")
    lines.append("</svg>")
    return "\n".join(lines) + "\n"
