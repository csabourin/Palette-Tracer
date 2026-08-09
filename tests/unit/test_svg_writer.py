"""
SVG assembly (SPEC §9.4.4, §10.3).

The one thing that is easy to get subtly wrong here is holes. A fill rule
decides what is inside a shape by counting the subpaths a ray crosses, so
subpaths that should subtract from one another have to share a `<path>`.
"""

from palette_trace.svg_writer import build_svg_document


def scan(**overrides):
    base = {
        "entryId": "entry-1",
        "name": "Colour 1",
        "color": "#1414C8",
        "role": "primary_fill",
        "isBackground": False,
        "fillRule": "evenodd",
        "pathDatas": [],
    }
    base.update(overrides)
    return base


class TestHoles:
    def test_a_shape_and_its_hole_share_one_path_element(self):
        """
        Split across two elements, the inner subpath is not a hole at all — it
        is a second solid shape drawn over the gap it was supposed to leave.
        """
        svg = build_svg_document(
            [scan(pathDatas=[
                "M 0 0 L 40 0 L 40 40 L 0 40 Z",
                "M 10 10 L 20 10 L 20 20 L 10 20 Z",
            ])],
            40, 40, "image-uuid",
        )

        assert svg.count("<path ") == 1
        assert "M 0 0 L 40 0 L 40 40 L 0 40 Z M 10 10 L 20 10 L 20 20 L 10 20 Z" in svg
        assert "fill-rule:evenodd" in svg

    def test_each_scan_still_gets_its_own_path(self):
        svg = build_svg_document(
            [
                scan(pathDatas=["M 0 0 L 10 0 L 10 10 Z"]),
                scan(entryId="entry-2", color="#14C814", pathDatas=["M 20 20 L 30 20 L 30 30 Z"]),
            ],
            40, 40, "image-uuid",
        )

        assert svg.count("<path ") == 2
        assert "#1414C8" in svg and "#14C814" in svg

    def test_a_scan_with_no_geometry_emits_nothing(self):
        svg = build_svg_document([scan(pathDatas=[])], 40, 40, "image-uuid")
        assert "<path" not in svg

    def test_the_declared_fill_rule_is_carried_through(self):
        svg = build_svg_document(
            [scan(fillRule="nonzero", pathDatas=["M 0 0 L 40 0 L 40 40 L 0 40 Z"])],
            40, 40, "image-uuid",
        )
        assert "fill-rule:nonzero" in svg
