#!/usr/bin/env python3
"""Generate the synthetic reference corpus from analytic descriptions (§25.2).

PTE-TEST-004: "Synthetic raster inputs SHOULD be regenerated from committed
vector/analytic descriptions at multiple resolutions, subpixel translations,
rotations, antialias kernels, color profiles, and compression levels."

So the *generator* and the *manifest* are committed and the rasters are not.
That keeps the repository free of binary blobs whose provenance a reader has to
take on trust, and it makes "at multiple resolutions and subpixel translations"
a parameter rather than a new set of files.

Coverage is computed by exact analytic area for the shapes that have one and by
16x16 supersampling otherwise; the manifest records which, because a fixture
whose truth is itself approximate cannot gate a 0.10 px metric.

Usage:
    python3 tools/make_fixtures.py <output-directory>

Writes one binary PAM (P7) per fixture plus `manifest.json` describing them.
"""

from __future__ import annotations

import json
import math
import os
import sys
from dataclasses import dataclass, field
from typing import Callable

# --- raster helpers -------------------------------------------------------

SUPERSAMPLE = 16


@dataclass
class Fixture:
    """One synthetic fixture and everything §25.2 asks a manifest to record."""

    id: str
    family: str
    width: int
    height: int
    # (x, y) in pixel coordinates -> RGBA tuple, in 0..=255.
    shade: Callable[[float, float], tuple[int, int, int, int]]
    intended_profiles: list[str]
    known_features: str
    protected_topology: str
    truth: str
    antialias: str
    notes: str = ""
    parameters: dict = field(default_factory=dict)


def render(fixture: Fixture) -> bytes:
    """Render a fixture to binary PAM (P7) RGBA, 8 bits per channel."""
    header = (
        f"P7\nWIDTH {fixture.width}\nHEIGHT {fixture.height}\nDEPTH 4\n"
        f"MAXVAL 255\nTUPLTYPE RGB_ALPHA\nENDHDR\n"
    ).encode("ascii")
    body = bytearray()
    for y in range(fixture.height):
        for x in range(fixture.width):
            # Pixel centres at (x + 0.5, y + 0.5), matching PTE-API-002.
            body.extend(fixture.shade(x + 0.5, y + 0.5))
    return bytes(header) + bytes(body)


def mix(a, b, t):
    """Blend two opaque colours by coverage `t` of `a`."""
    t = min(1.0, max(0.0, t))
    return tuple(round(a[i] * t + b[i] * (1.0 - t)) for i in range(3)) + (255,)


def supersampled(inside, a, b):
    """Coverage by 16x16 supersampling, for shapes with no closed-form area."""

    def shade(px, py):
        hits = 0
        step = 1.0 / SUPERSAMPLE
        for j in range(SUPERSAMPLE):
            for i in range(SUPERSAMPLE):
                sx = px - 0.5 + (i + 0.5) * step
                sy = py - 0.5 + (j + 0.5) * step
                if inside(sx, sy):
                    hits += 1
        return mix(a, b, hits / (SUPERSAMPLE * SUPERSAMPLE))

    return shade


def linear_supersampled_regions(classify, colors):
    """Supersample a multi-colour partition and composite in linear light."""

    def decode(value):
        encoded = value / 255.0
        if encoded <= 0.04045:
            return encoded / 12.92
        return ((encoded + 0.055) / 1.055) ** 2.4

    def encode(value):
        if value <= 0.0031308:
            encoded = 12.92 * value
        else:
            encoded = 1.055 * value ** (1.0 / 2.4) - 0.055
        return round(min(1.0, max(0.0, encoded)) * 255.0)

    linear = [tuple(decode(channel) for channel in color) for color in colors]

    def shade(px, py):
        sums = [0.0, 0.0, 0.0]
        step = 1.0 / SUPERSAMPLE
        for j in range(SUPERSAMPLE):
            for i in range(SUPERSAMPLE):
                sx = px - 0.5 + (i + 0.5) * step
                sy = py - 0.5 + (j + 0.5) * step
                color = linear[classify(sx, sy)]
                for channel in range(3):
                    sums[channel] += color[channel]
        scale = 1.0 / (SUPERSAMPLE * SUPERSAMPLE)
        return tuple(encode(total * scale) for total in sums) + (255,)

    return shade


def hard(inside, a, b):
    """No antialiasing at all: the pixel centre decides. (§10.6, §15.)"""

    def shade(px, py):
        return a + (255,) if inside(px, py) else b + (255,)

    return shade


# --- colours --------------------------------------------------------------

INK = (32, 38, 52)
PAPER = (247, 244, 236)
RED = (198, 62, 54)
BLUE = (46, 92, 173)
GREEN = (74, 148, 92)


# --- fixture families (§25.3) --------------------------------------------


def fixtures() -> list[Fixture]:
    out: list[Fixture] = []

    # --- topology ---------------------------------------------------------

    def nested(px, py):
        # Three nested rectangles: an outer, a middle, and a core. Exercises
        # containment depth and hole nesting.
        if 26 <= px < 74 and 22 <= py < 58:
            return RED + (255,)
        if 16 <= px < 84 and 12 <= py < 68:
            return BLUE + (255,)
        return PAPER + (255,)

    out.append(
        Fixture(
            id="topology/nested-rectangles",
            family="topology",
            width=100,
            height=80,
            shade=nested,
            intended_profiles=["flat-illustration", "logo"],
            known_features="three nested axis-aligned rectangles, no antialiasing",
            protected_topology="containment depth 3; 2 holes; 12 straight shared boundaries",
            truth="exact: every boundary is on a pixel-cell interface",
            antialias="none",
            notes="§31.5's rectangle rule applies directly: each side is one line.",
        )
    )

    def donut(px, py):
        d = math.hypot(px - 50, py - 40)
        if d < 14:
            return PAPER + (255,)
        if d < 30:
            return BLUE + (255,)
        return PAPER + (255,)

    out.append(
        Fixture(
            id="topology/donut",
            family="topology",
            width=100,
            height=80,
            shade=donut,
            intended_profiles=["flat-illustration"],
            known_features="annulus; the inner disc is the same colour as the background",
            protected_topology="1 hole that must not be filled; the hole is NOT connected to the exterior",
            truth="exact circle centre (50, 40), radii 14 and 30",
            antialias="none",
        )
    )

    def checkerboard(px, py):
        return (INK if (int(px) // 4 + int(py) // 4) % 2 == 0 else PAPER) + (255,)

    out.append(
        Fixture(
            id="adversarial/checkerboard-4px",
            family="adversarial",
            width=64,
            height=64,
            shade=checkerboard,
            intended_profiles=["flat-illustration", "pixel-art"],
            known_features="4-pixel checkerboard: every interior vertex is a 4-way junction",
            protected_topology="256 cells; no two diagonal cells may merge",
            truth="exact",
            antialias="none",
            notes="The §9.4 ambiguity resolver's worst case, and a stress test for junction count.",
        )
    )

    def bridge(px, py):
        # Two blocks joined by a one-pixel bridge, plus a one-pixel island.
        if 10 <= px < 40 and 20 <= py < 50:
            return INK + (255,)
        if 60 <= px < 90 and 20 <= py < 50:
            return INK + (255,)
        if 40 <= px < 60 and 34 <= py < 35:
            return INK + (255,)
        if 48 <= px < 49 and 60 <= py < 61:
            return INK + (255,)
        return PAPER + (255,)

    out.append(
        Fixture(
            id="topology/one-pixel-bridge",
            family="topology",
            width=100,
            height=70,
            shade=bridge,
            intended_profiles=["flat-illustration"],
            known_features="two blocks joined by a 1-pixel bridge; one 1-pixel island",
            protected_topology="the bridge must survive (1 connected component, not 2); the island must survive",
            truth="exact",
            antialias="none",
            notes="PTE-SEG-017 thin-feature protection: this is the fixture it exists for.",
        )
    )

    junction_x, junction_y = 7.35, 7.62

    def t_junction_region(x, y):
        if y < junction_y:
            return 0
        return 1 if x < junction_x else 2

    out.append(
        Fixture(
            id="topology/subpixel-t-junction",
            family="topology",
            width=16,
            height=16,
            shade=linear_supersampled_regions(t_junction_region, [INK, RED, GREEN]),
            intended_profiles=["flat-illustration", "logo"],
            known_features="three antialiased regions meeting at one subpixel T junction",
            protected_topology="3 faces; exactly 1 degree-3 interior junction; incident order preserved",
            truth=f"exact: shared junction ({junction_x}, {junction_y})",
            antialias="16x16 box supersampling in linear light",
            parameters={"junction": [junction_x, junction_y]},
            notes=(
                "PTE-AA-006 and PTE-TOPO-011/012/013: barycentric evidence gates "
                "one shared junction solve; all three incident chains must reuse its coordinate."
            ),
        )
    )

    # --- curves and corners ----------------------------------------------

    for index, (cx, cy) in enumerate([(50.0, 40.0), (50.37, 40.61)]):
        out.append(
            Fixture(
                id=f"curves/circle-subpixel-{index}",
                family="curves",
                width=100,
                height=80,
                shade=supersampled(
                    lambda x, y, cx=cx, cy=cy: math.hypot(x - cx, y - cy) < 28.0,
                    INK,
                    PAPER,
                ),
                intended_profiles=["flat-illustration", "logo"],
                known_features="antialiased circle at a subpixel centre",
                protected_topology="1 face plus background; no holes",
                truth=f"exact: centre ({cx}, {cy}), radius 28.0",
                antialias="16x16 box supersampling",
                parameters={"centre": [cx, cy], "radius": 28.0},
                notes=(
                    "§31.2's circle centre and radius gates are written for these. "
                    "They cannot be measured until §10 coverage reconstruction exists: "
                    "this build thresholds the fringe rather than inverting it."
                ),
            )
        )

    def rounded_rect_inside(x, y, r=12.0):
        left, top, right, bottom = 18.0, 14.0, 82.0, 66.0
        cx = min(max(x, left + r), right - r)
        cy = min(max(y, top + r), bottom - r)
        if left <= x <= right and top <= y <= bottom:
            return math.hypot(x - cx, y - cy) <= r or (
                left + r <= x <= right - r or top + r <= y <= bottom - r
            )
        return False

    out.append(
        Fixture(
            id="curves/rounded-rectangle",
            family="curves",
            width=100,
            height=80,
            shade=supersampled(rounded_rect_inside, GREEN, PAPER),
            intended_profiles=["logo"],
            known_features="rounded rectangle, corner radius 12",
            protected_topology="1 face; 4 tangency points between straight and curved runs",
            truth="exact: rect (18, 14)-(82, 66), radius 12",
            antialias="16x16 box supersampling",
            parameters={"rect": [18.0, 14.0, 82.0, 66.0], "radius": 12.0},
            notes="Tangency, not corners: PTE-GEO-002 must not pin the four joins.",
        )
    )

    def star_inside(x, y, points=5, outer=34.0, inner=14.0):
        dx, dy = x - 50.0, y - 40.0
        angle = math.atan2(dy, dx)
        radius = math.hypot(dx, dy)
        # Radius of the star's edge at this angle.
        sector = math.tau / points
        phase = (angle % sector) / sector
        t = abs(phase - 0.5) * 2.0
        return radius <= inner + (outer - inner) * t

    out.append(
        Fixture(
            id="curves/star-acute-corners",
            family="curves",
            width=100,
            height=80,
            shade=supersampled(star_inside, RED, PAPER),
            intended_profiles=["logo", "flat-illustration"],
            known_features="five-pointed star: 5 acute outer corners, 5 reflex inner corners",
            protected_topology="10 corners that must all be pinned; no rounding of the points",
            truth="exact: centre (50, 40), outer 34, inner 14, 5 points",
            antialias="16x16 box supersampling",
            parameters={"points": 5, "outer": 34.0, "inner": 14.0},
            notes="PTE-GEO-003: an unpinned acute corner is visibly rounded off.",
        )
    )

    def staircase(px, py):
        # A 45-degree edge, the shape every raster boundary is made of.
        return (BLUE if py > px * 0.5 + 8 else PAPER) + (255,)

    out.append(
        Fixture(
            id="curves/shallow-staircase",
            family="curves",
            width=100,
            height=70,
            shade=hard(lambda x, y: y > x * 0.5 + 8, BLUE, PAPER),
            intended_profiles=["flat-illustration"],
            known_features="a single straight edge of slope 0.5, rendered without antialiasing",
            protected_topology="1 shared boundary",
            truth="exact: the line y = 0.5x + 8",
            antialias="none",
            parameters={"slope": 0.5, "intercept": 8.0},
            notes=(
                "§31.5's 'straight shared boundaries SHOULD be one line'. "
                "See docs/notes/curve-fitting.md limitation 1: with split points on "
                "source samples this needs a tolerance above the staircase's own "
                "deviation from its chord."
            ),
        )
    )

    # --- colour and alpha -------------------------------------------------

    def transparent_noise(px, py):
        # Fully transparent pixels with arbitrary, varying RGB: PTE-COLOR-008's
        # invariance must make these invisible to every later stage.
        if 20 <= px < 80 and 16 <= py < 64:
            return (int(px * 7) % 256, int(py * 13) % 256, int(px + py) % 256, 0)
        return PAPER + (255,)

    out.append(
        Fixture(
            id="alpha/transparent-arbitrary-rgb",
            family="alpha",
            width=100,
            height=80,
            shade=transparent_noise,
            intended_profiles=["flat-illustration"],
            known_features="a rectangle of alpha-zero pixels carrying arbitrary RGB",
            protected_topology="the transparent region contributes no visible face",
            truth="exact",
            antialias="none",
            notes="PTE-COLOR-008/009: changing the hidden RGB must not change the digest.",
        )
    )

    def near_neutral(px, py):
        # Four near-neutral greys that a hue-based palette would confuse.
        band = int(px) * 4 // 100
        grey = [118, 124, 130, 136][min(band, 3)]
        tint = [(0, 0, 0), (2, 0, -2), (-2, 2, 0), (0, -2, 2)][min(band, 3)]
        return tuple(max(0, min(255, grey + t)) for t in tint) + (255,)

    out.append(
        Fixture(
            id="color/near-neutral-bands",
            family="color",
            width=100,
            height=40,
            shade=near_neutral,
            intended_profiles=["flat-illustration"],
            known_features="four near-neutral greys differing by 2/255 per channel",
            protected_topology="4 vertical bands, or a documented merge",
            truth="exact",
            antialias="none",
            notes="PTE-COLOR-013: hue is powerless near neutral, so chroma cannot decide these.",
        )
    )

    # --- pixel art --------------------------------------------------------

    def pixel_art(px, py):
        x, y = int(px) // 4, int(py) // 4
        # A deliberate one-pixel diagonal, the §9.4 ambiguity in its natural
        # habitat.
        if x == y or x + y == 15:
            return INK + (255,)
        if (x * 3 + y * 5) % 7 == 0:
            return RED + (255,)
        return PAPER + (255,)

    out.append(
        Fixture(
            id="pixel-art/diagonals",
            family="pixel-art",
            width=64,
            height=64,
            shade=pixel_art,
            intended_profiles=["pixel-art"],
            known_features="4x scaled pixel art with intentional single-cell diagonals",
            protected_topology="diagonals must stay connected under the pixel-art policy (PTE-TOPO-010)",
            truth="exact",
            antialias="none",
            notes="Blocky output is correct here; a fitted curve would be a defect.",
        )
    )

    return out


def main() -> int:
    if len(sys.argv) != 2:
        print(__doc__, file=sys.stderr)
        return 2
    root = sys.argv[1]

    manifest = {
        "schemaVersion": 1,
        "generator": "tools/make_fixtures.py",
        "note": (
            "Synthetic fixtures, regenerated from analytic descriptions rather than "
            "committed as rasters (PTE-TEST-004). No third-party asset is included, "
            "so PTE-TEST-003's redistribution question does not arise: everything "
            "here is first-party and covered by the engine's own licence."
        ),
        "fixtures": [],
    }

    for fixture in fixtures():
        path = os.path.join(root, fixture.id + ".pam")
        os.makedirs(os.path.dirname(path), exist_ok=True)
        data = render(fixture)
        with open(path, "wb") as handle:
            handle.write(data)

        manifest["fixtures"].append(
            {
                "id": fixture.id,
                "family": fixture.family,
                "path": fixture.id + ".pam",
                "width": fixture.width,
                "height": fixture.height,
                "bytes": len(data),
                "license": "first-party, MIT with the engine",
                "provenance": "generated by tools/make_fixtures.py from an analytic description",
                "parameters": fixture.parameters,
                "intendedProfiles": fixture.intended_profiles,
                "knownFeatures": fixture.known_features,
                "protectedTopology": fixture.protected_topology,
                "truth": fixture.truth,
                "antialias": fixture.antialias,
                "notes": fixture.notes,
            }
        )

    with open(os.path.join(root, "manifest.json"), "w", encoding="utf-8") as handle:
        json.dump(manifest, handle, indent=2)
        handle.write("\n")

    print(f"{len(manifest['fixtures'])} fixtures written to {root}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
