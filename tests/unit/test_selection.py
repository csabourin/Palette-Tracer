import inkex

from palette_trace.document.selection import validate_and_get_selected_image
from palette_trace.document.settings_store import PT_NAMESPACE


def test_generated_group_selection_resolves_source_image_by_uuid():
    svg = inkex.SvgDocumentElement()

    img = inkex.Image()
    img.set("id", "img1")
    img.set(f"{{{PT_NAMESPACE}}}image-uuid", "uuid-123")
    svg.append(img)

    grp = inkex.Group()
    grp.set(f"{{{PT_NAMESPACE}}}generated", "true")
    grp.set(f"{{{PT_NAMESPACE}}}source-image-uuid", "uuid-123")
    svg.append(grp)

    selected = {"grp1": grp}
    resolved = validate_and_get_selected_image(svg, selected)
    assert resolved is img
