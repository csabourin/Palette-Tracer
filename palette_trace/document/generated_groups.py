"""
Generated trace-group creation and replacement (SPEC §10.3, §10.4, §28).

The commit is a single group swap: the new content is built in full, and only
then does it replace the previous group. A failure part-way leaves the document
exactly as it was (§24 Stage 13, §34.29).
"""

import uuid

import inkex

from palette_trace.settings import PT_NAMESPACE, PT_SCHEMA_VERSION


def _pt(name: str) -> str:
    return f"{{{PT_NAMESPACE}}}{name}"


def build_generated_group(
    image_elem: inkex.Image,
    image_uuid: str,
    scan_results: list,
    transform_matrix: inkex.Transform,
    provenance: dict,
    group_id: str,
) -> inkex.Group:
    """
    Builds the generated root group without attaching it to the document.

    `provenance` carries the §10.3 attributes. The full settings are deliberately
    *not* duplicated here — they live on the source image (§10.2).
    """
    root_group = inkex.Group()
    root_group.set("id", group_id)
    root_group.set("inkscape:label", f"Palette Trace — {image_elem.get('id', 'image')}")
    root_group.set(_pt("generated"), "true")
    root_group.set(_pt("source-image-uuid"), image_uuid)
    root_group.set(_pt("schema-version"), str(PT_SCHEMA_VERSION))
    root_group.set(_pt("settings-hash"), provenance.get("settingsHash", ""))
    root_group.set(_pt("source-hash"), provenance.get("sourceHash", ""))
    root_group.set(_pt("backend-id"), provenance.get("backendId", ""))
    root_group.set(_pt("backend-version"), provenance.get("backendVersion", ""))

    for index, scan in enumerate(scan_results):
        path_datas = [d for d in scan.get("pathDatas", []) if d]
        if not path_datas:
            continue

        hex_color = scan.get("color", "#000000")
        name = scan.get("name", f"Scan {index + 1}")
        role = scan.get("role", "primary_fill")

        scan_group = inkex.Group()
        scan_group.set("id", f"palette-trace-scan-{uuid.uuid4().hex[:8]}")
        scan_group.set("inkscape:label", f"{index + 1:02d} {name} — {hex_color}")
        scan_group.set(_pt("palette-entry-id"), scan["entryId"])
        scan_group.set(_pt("role"), role)
        if scan.get("isBackground"):
            scan_group.set(_pt("background"), "true")

        for path_data in path_datas:
            path_elem = inkex.PathElement()
            path_elem.set("d", path_data)
            path_elem.style["fill"] = hex_color
            path_elem.style["fill-rule"] = scan.get("fillRule", "evenodd")
            path_elem.style["stroke"] = "none"
            path_elem.transform = transform_matrix
            scan_group.append(path_elem)

        root_group.append(scan_group)

    return root_group


def find_generated_group(svg_root, group_id: str):
    """Returns the existing generated group with `group_id`, or None."""
    if not group_id:
        return None
    matches = svg_root.xpath(f'//*[@id="{group_id}"]')
    return matches[0] if matches else None


def commit_generated_trace_group(
    svg_root: inkex.SvgDocumentElement,
    image_elem: inkex.Image,
    image_uuid: str,
    scan_results: list,
    transform_matrix: inkex.Transform,
    existing_group_id: str = None,
    provenance: dict = None,
) -> str:
    """
    Creates or replaces the generated group, returning its id.

    Replacement is atomic in the sense that matters for §34.29: the replacement
    group is fully built before the previous one is detached, so an exception
    during construction cannot leave a half-written result in the document.
    """
    parent = image_elem.getparent()
    if parent is None:
        parent = svg_root

    existing = find_generated_group(svg_root, existing_group_id)
    group_id = (
        existing.get("id") if existing is not None
        else f"palette-trace-result-{uuid.uuid4().hex[:8]}"
    )

    new_group = build_generated_group(
        image_elem, image_uuid, scan_results, transform_matrix,
        provenance or {}, group_id,
    )

    if existing is not None:
        existing_parent = existing.getparent()
        index = list(existing_parent).index(existing)
        existing_parent.remove(existing)
        existing_parent.insert(index, new_group)
    else:
        parent.insert(list(parent).index(image_elem) + 1, new_group)

    return group_id
