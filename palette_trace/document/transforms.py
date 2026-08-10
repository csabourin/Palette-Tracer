"""
Coordinate transformations from intrinsic source-pixel space to SVG document viewport space.
"""

import inkex


def get_image_to_document_transform(
    image_elem: inkex.Image,
    intrinsic_w: int,
    intrinsic_h: int,
) -> inkex.Transform:
    """
    Computes complete inkex.Transform matrix mapping intrinsic pixel space (0..w, 0..h)
    to SVG document coordinates.
    """
    x = float(image_elem.get("x", 0))
    y = float(image_elem.get("y", 0))
    width = float(image_elem.get("width", intrinsic_w))
    height = float(image_elem.get("height", intrinsic_h))

    scale_x = width / float(intrinsic_w) if intrinsic_w > 0 else 1.0
    scale_y = height / float(intrinsic_h) if intrinsic_h > 0 else 1.0

    # Viewport transform
    viewport_transform = inkex.Transform(f"translate({x}, {y}) scale({scale_x}, {scale_y})")

    # Combine with image element's existing transform attribute
    elem_transform = image_elem.transform
    if elem_transform:
        return elem_transform * viewport_transform
    return viewport_transform
