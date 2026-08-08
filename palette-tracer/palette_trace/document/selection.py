"""
SVG Selection validation and image source resolution.
"""

import os
from typing import Optional
import inkex
from palette_trace.errors import SelectionError, ImageSourceError
from palette_trace.document.settings_store import PT_NAMESPACE

def validate_and_get_selected_image(svg_root: inkex.SvgDocumentElement, selection: dict) -> inkex.Image:
    """
    Validates that exactly one valid SVG <image> element is selected.
    Returns the target inkex.Image element.
    """
    if not selection:
        raise SelectionError("No element selected. Please select exactly one bitmap image.")

    selected_nodes = list(selection.values())
    if len(selected_nodes) != 1:
        raise SelectionError(f"Expected 1 selected element, got {len(selected_nodes)}.")

    elem = selected_nodes[0]

    # Check if a generated trace group was selected instead of an image
    if elem.tag.endswith("g") and elem.get(f"{{{PT_NAMESPACE}}}generated") == "true":
        target_uuid = elem.get(f"{{{PT_NAMESPACE}}}source-image-uuid")
        if target_uuid:
            # Locate image by pt:image-uuid
            images = svg_root.xpath(f'//svg:image[@pt:image-uuid="{target_uuid}"]', namespaces={
                "svg": inkex.NSS["svg"],
                "pt": PT_NAMESPACE,
            })
            if images:
                return images[0]

    if not isinstance(elem, inkex.Image) and elem.tag != f"{{{inkex.NSS['svg']}}}image":
        raise SelectionError(f"Selected element is <{elem.tag.rsplit('}', 1)[-1]}>, not an <image>.")

    return elem

def resolve_image_uri(image_elem: inkex.Image, doc_path: Optional[str] = None) -> tuple[str, bool]:
    """
    Resolves image href to a local file path or data URI bytes.
    Returns (path_or_data, is_data_uri).
    """
    href = image_elem.get("{http://www.w3.org/1999/xlink}href") or image_elem.get("href")
    if not href:
        raise ImageSourceError("Selected <image> element has no href or xlink:href attribute.")

    if href.startswith("data:"):
        return href, True

    if href.startswith("http://") or href.startswith("https://"):
        raise ImageSourceError("Remote HTTP/HTTPS image URLs are not supported for local tracing.")

    # Resolve local file path
    if os.path.isabs(href):
        resolved_path = href
    else:
        if not doc_path:
            raise ImageSourceError("Document must be saved to resolve relative image paths.")
        doc_dir = os.path.dirname(doc_path)
        resolved_path = os.path.normpath(os.path.join(doc_dir, href))

    if not os.path.exists(resolved_path):
        raise ImageSourceError(f"Linked image file not found: {resolved_path}")

    return resolved_path, False
