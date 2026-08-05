"""
SVG Generated Trace Group creation and replacement.
"""

import uuid
import inkex
from palette_trace.document.settings_store import PT_NAMESPACE

def commit_generated_trace_group(
    svg_root: inkex.SvgDocumentElement,
    image_elem: inkex.Image,
    image_uuid: str,
    scan_results: list[dict],
    transform_matrix: inkex.Transform,
    existing_group_id: str = None,
) -> str:
    """
    Creates or atomically updates the generated SVG group for traced scans.
    Returns the root group ID.
    """
    parent = image_elem.getparent()
    if parent is None:
        parent = svg_root

    # Search for existing generated group
    root_group = None
    if existing_group_id:
        existing = svg_root.xpath(f'//*[@id="{existing_group_id}"]')
        if existing:
            root_group = existing[0]

    if root_group is None:
        # Create new root group
        root_id = f"palette-trace-result-{uuid.uuid4().hex[:8]}"
        root_group = inkex.Group()
        root_group.set("id", root_id)
        root_group.set("inkscape:label", f"Palette Trace — {image_elem.get('id', 'image')}")
        root_group.set(f"{{{PT_NAMESPACE}}}generated", "true")
        root_group.set(f"{{{PT_NAMESPACE}}}source-image-uuid", image_uuid)

        # Insert immediately above the source image element
        idx = list(parent).index(image_elem)
        parent.insert(idx + 1, root_group)
    else:
        # Clear existing children for atomic update
        root_id = root_group.get("id")
        for child in list(root_group):
            root_group.remove(child)

    # Populate scan groups
    for idx, scan in enumerate(scan_results):
        entry_id = scan["entryId"]
        name = scan.get("name", f"Scan {idx+1}")
        hex_color = scan.get("color", "#000000")
        role = scan.get("role", "primary_fill")
        path_datas = scan.get("pathDatas", [])
        fill_rule = scan.get("fillRule", "evenodd")

        if not path_datas:
            continue

        scan_group = inkex.Group()
        scan_group.set("id", f"palette-trace-scan-{uuid.uuid4().hex[:8]}")
        scan_group.set("inkscape:label", f"{idx+1:02d} {name} — {hex_color}")
        scan_group.set(f"{{{PT_NAMESPACE}}}palette-entry-id", entry_id)
        scan_group.set(f"{{{PT_NAMESPACE}}}role", role)

        for d_str in path_datas:
            if not d_str:
                continue
            path_elem = inkex.PathElement()
            path_elem.set("d", d_str)
            path_elem.style["fill"] = hex_color
            path_elem.style["fill-rule"] = fill_rule
            path_elem.style["stroke"] = "none"

            # Apply transform matrix to path element or scan group
            path_elem.transform = transform_matrix
            scan_group.append(path_elem)

        root_group.append(scan_group)

    return root_id
