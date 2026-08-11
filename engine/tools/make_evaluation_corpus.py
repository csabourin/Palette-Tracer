#!/usr/bin/env python3
"""Generate the deterministic half of the clean-room evaluation corpus.

The five generated, naturalistic assets are intentionally fixed inputs.  This
program owns the thirteen analytic/adversarial assets: each is rendered from
first-party code, without fonts, external images, or a raster dependency.

Usage:
    python3 tools/make_evaluation_corpus.py [OUTPUT_ROOT]

OUTPUT_ROOT defaults to ``fixtures`` beside the engine directory.
"""

from __future__ import annotations

import math
import struct
import sys
import zlib
from pathlib import Path

WIDTH = 320
HEIGHT = 240


def rgba(r: int, g: int, b: int, a: int = 255) -> tuple[int, int, int, int]:
    return (r, g, b, a)


class Canvas:
    def __init__(self, width: int = WIDTH, height: int = HEIGHT, color=rgba(255, 255, 255)):
        self.width = width
        self.height = height
        self.pixels = [color] * (width * height)

    def put(self, x: int, y: int, color) -> None:
        if 0 <= x < self.width and 0 <= y < self.height:
            self.pixels[y * self.width + x] = color

    def rect(self, x0: int, y0: int, x1: int, y1: int, color) -> None:
        for y in range(max(0, y0), min(self.height, y1)):
            start = y * self.width
            for x in range(max(0, x0), min(self.width, x1)):
                self.pixels[start + x] = color

    def circle(self, cx: float, cy: float, radius: float, color) -> None:
        r2 = radius * radius
        for y in range(max(0, int(cy - radius - 1)), min(self.height, int(cy + radius + 2))):
            for x in range(max(0, int(cx - radius - 1)), min(self.width, int(cx + radius + 2))):
                if (x + 0.5 - cx) ** 2 + (y + 0.5 - cy) ** 2 <= r2:
                    self.put(x, y, color)

    def polygon(self, points: list[tuple[float, float]], color) -> None:
        min_x = max(0, math.floor(min(x for x, _ in points)))
        max_x = min(self.width, math.ceil(max(x for x, _ in points)))
        min_y = max(0, math.floor(min(y for _, y in points)))
        max_y = min(self.height, math.ceil(max(y for _, y in points)))
        for y in range(min_y, max_y):
            for x in range(min_x, max_x):
                px, py = x + 0.5, y + 0.5
                inside = False
                previous = points[-1]
                for current in points:
                    x1, y1 = previous
                    x2, y2 = current
                    crosses = (y1 > py) != (y2 > py)
                    if crosses and px < (x2 - x1) * (py - y1) / (y2 - y1) + x1:
                        inside = not inside
                    previous = current
                if inside:
                    self.put(x, y, color)

    def line(self, x0: int, y0: int, x1: int, y1: int, color, width: int = 1) -> None:
        dx = abs(x1 - x0)
        sx = 1 if x0 < x1 else -1
        dy = -abs(y1 - y0)
        sy = 1 if y0 < y1 else -1
        error = dx + dy
        while True:
            half = width // 2
            self.rect(x0 - half, y0 - half, x0 - half + width, y0 - half + width, color)
            if x0 == x1 and y0 == y1:
                break
            twice = 2 * error
            if twice >= dy:
                error += dy
                x0 += sx
            if twice <= dx:
                error += dx
                y0 += sy


def png_bytes(canvas: Canvas) -> bytes:
    raw = bytearray()
    for y in range(canvas.height):
        raw.append(0)
        for pixel in canvas.pixels[y * canvas.width : (y + 1) * canvas.width]:
            raw.extend(pixel)

    def chunk(kind: bytes, body: bytes) -> bytes:
        return struct.pack(">I", len(body)) + kind + body + struct.pack(">I", zlib.crc32(kind + body))

    header = struct.pack(">IIBBBBB", canvas.width, canvas.height, 8, 6, 0, 0, 0)
    return b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", header) + chunk(b"IDAT", zlib.compress(bytes(raw), 9)) + chunk(b"IEND", b"")


def flat_logo() -> Canvas:
    c = Canvas(color=rgba(246, 243, 235))
    navy, coral, teal, cream = rgba(20, 44, 82), rgba(221, 74, 67), rgba(25, 145, 151), rgba(246, 243, 235)
    c.circle(105, 120, 72, navy)
    c.polygon([(38, 126), (105, 48), (172, 126), (105, 104)], coral)
    c.polygon([(52, 142), (105, 112), (158, 142), (105, 184)], teal)
    c.circle(105, 120, 21, cream)
    c.rect(204, 75, 284, 100, navy)
    c.rect(204, 108, 268, 133, coral)
    c.rect(204, 141, 292, 166, teal)
    return c


def faded_logo() -> Canvas:
    c = Canvas(color=rgba(31, 48, 72))
    pale = rgba(129, 156, 192)
    c.circle(112, 118, 68, pale)
    c.polygon([(54, 133), (112, 57), (168, 133), (112, 111)], rgba(157, 174, 197))
    c.rect(198, 72, 285, 98, pale)
    c.rect(198, 110, 267, 136, pale)
    c.rect(198, 148, 292, 174, pale)
    for y in range(c.height):
        for x in range(c.width):
            if ((x * 37 + y * 19 + (x * y) % 41) % 113) < 17:
                old = c.pixels[y * c.width + x]
                if old != rgba(31, 48, 72):
                    c.put(x, y, rgba(67, 82, 105))
    return c


def transparent_drop() -> Canvas:
    c = Canvas(color=rgba(39, 71, 123, 0))
    cx, cy = 160, 125
    for y in range(24, 216):
        for x in range(72, 248):
            nx = abs(x + 0.5 - cx) / 78
            ny = (y + 0.5 - cy) / 92
            shape = nx * nx + ny * ny
            if y < cy:
                shape += max(0.0, (cy - y) / 110 - (1 - nx)) * 1.4
            if shape <= 1.0:
                edge = max(0.0, min(1.0, (1.0 - shape) * 6.0))
                alpha = round(28 + 227 * edge)
                blue = round(205 - 75 * (y / c.height))
                c.put(x, y, rgba(36, 132, blue, alpha))
    return c


def hidden_rgb() -> Canvas:
    c = Canvas(color=rgba(223, 41, 97, 0))
    c.rect(48, 42, 272, 198, rgba(26, 146, 154, 255))
    c.circle(160, 120, 55, rgba(247, 190, 64, 160))
    return c


def tiny_accent() -> Canvas:
    c = Canvas(color=rgba(13, 27, 45))
    c.polygon([(35, 176), (160, 52), (285, 176)], rgba(232, 235, 230))
    c.polygon([(62, 176), (160, 85), (258, 176)], rgba(38, 82, 59))
    accent = rgba(245, 173, 41)
    c.circle(160, 138, 5, accent)
    c.line(157, 34, 163, 34, accent, 2)
    return c


def similar_blues() -> Canvas:
    colors = [rgba(190, 221, 244), rgba(149, 202, 238), rgba(106, 180, 228), rgba(66, 151, 211), rgba(35, 119, 187), rgba(19, 87, 153)]
    c = Canvas(color=rgba(242, 247, 250))
    for index, color in enumerate(colors):
        x0 = 20 + index * 49
        c.rect(x0, 25, x0 + 42, 215, color)
        for y in range(45 + index * 7, 196, 34):
            c.circle(x0 + 21, y, 9 + (index % 3), colors[max(0, index - 1)])
    return c


def pixel_art() -> Canvas:
    palette = [rgba(36, 80, 135), rgba(113, 190, 229), rgba(43, 105, 61), rgba(104, 176, 57), rgba(70, 70, 76), rgba(232, 178, 43), rgba(225, 77, 61), rgba(247, 243, 224)]
    c = Canvas(width=320, height=240, color=palette[1])
    grid = 8
    for gy in range(30):
        for gx in range(40):
            if gy > 21:
                color = palette[2 if (gx + gy) % 3 else 3]
            elif 8 < gx < 29 and 9 < gy < 22:
                color = palette[4] if gx in (9, 28) or gy in (10, 21) else palette[7]
            else:
                color = palette[1]
            c.rect(gx * grid, gy * grid, (gx + 1) * grid, (gy + 1) * grid, color)
    for gx, gy, color in [(18, 8, palette[5]), (19, 8, palette[5]), (17, 9, palette[5]), (18, 9, palette[6]), (19, 9, palette[5]), (20, 9, palette[5])]:
        c.rect(gx * grid, gy * grid, (gx + 1) * grid, (gy + 1) * grid, color)
    return c


def screen_print() -> Canvas:
    c = Canvas(color=rgba(245, 241, 222))
    navy, orange, overlap = rgba(12, 35, 77), rgba(239, 100, 27), rgba(92, 56, 62)
    c.circle(120, 120, 78, navy)
    c.circle(195, 120, 78, orange)
    for y in range(c.height):
        for x in range(c.width):
            if (x + 0.5 - 120) ** 2 + (y + 0.5 - 120) ** 2 <= 78**2 and (x + 0.5 - 195) ** 2 + (y + 0.5 - 120) ** 2 <= 78**2:
                c.put(x, y, overlap)
    return c


def vinyl_paths() -> Canvas:
    c = Canvas(color=rgba(255, 255, 255, 0))
    ink = rgba(15, 18, 20, 255)
    c.circle(160, 120, 100, ink)
    c.circle(160, 120, 87, rgba(255, 255, 255, 0))
    c.polygon([(71, 176), (126, 87), (160, 129), (196, 66), (253, 176)], ink)
    c.polygon([(88, 176), (126, 114), (160, 151), (194, 94), (236, 176)], rgba(255, 255, 255, 0))
    c.polygon([(125, 191), (160, 148), (195, 191)], ink)
    return c


def laser_features() -> Canvas:
    c = Canvas(color=rgba(231, 204, 159))
    ink = rgba(40, 31, 24)
    cx, cy = 160, 120
    for ring in range(8):
        count = 8 + ring * 4
        radius = 16 + ring * 12
        for index in range(count):
            angle = 2 * math.pi * index / count
            x = round(cx + math.cos(angle) * radius)
            y = round(cy + math.sin(angle) * radius)
            c.circle(x, y, 1 if ring > 5 else 2, ink)
    for index in range(24):
        angle = 2 * math.pi * index / 24
        c.line(cx, cy, round(cx + math.cos(angle) * 100), round(cy + math.sin(angle) * 100), ink, 1)
    c.circle(cx, cy, 9, ink)
    return c


def gradient_mountains() -> Canvas:
    c = Canvas()
    for y in range(c.height):
        t = y / (c.height - 1)
        color = rgba(round(74 + 181 * t), round(80 + 107 * t), round(151 - 45 * t))
        c.rect(0, y, c.width, y + 1, color)
    c.polygon([(0, 172), (58, 92), (116, 168), (180, 70), (246, 166), (320, 103), (320, 240), (0, 240)], rgba(47, 55, 112))
    c.polygon([(0, 196), (73, 137), (132, 191), (203, 119), (270, 188), (320, 155), (320, 240), (0, 240)], rgba(37, 62, 104))
    c.circle(160, 92, 18, rgba(248, 206, 133))
    return c


def noisy_paper() -> Canvas:
    c = Canvas(color=rgba(247, 240, 219))
    ink = rgba(42, 39, 34)
    for y in range(c.height):
        for x in range(c.width):
            if ((x * 17 + y * 31 + x * y) % 521) < 3:
                c.put(x, y, rgba(190, 179, 151))
    c.line(35, 185, 285, 185, ink, 2)
    c.polygon([(72, 184), (72, 103), (160, 54), (248, 103), (248, 184)], rgba(247, 240, 219))
    for points in [((72, 184), (72, 103)), ((72, 103), (160, 54)), ((160, 54), (248, 103)), ((248, 103), (248, 184)), ((55, 185), (265, 185))]:
        c.line(*points[0], *points[1], ink, 2)
    c.rect(98, 123, 130, 185, rgba(247, 240, 219))
    c.rect(191, 117, 222, 145, rgba(247, 240, 219))
    for x in range(100, 131, 10):
        c.line(x, 123, x, 184, ink, 1)
    for x in range(192, 223, 10):
        c.line(x, 117, x, 144, ink, 1)
    return c


def open_closed() -> Canvas:
    c = Canvas(color=rgba(250, 250, 250))
    ink, warning = rgba(22, 25, 28), rgba(215, 58, 65)
    left = [(24, 190), (55, 70), (122, 122), (145, 48)]
    right = [(177, 190), (207, 70), (274, 122), (297, 48), (177, 190)]
    for points in (left, right):
        for a, b in zip(points, points[1:]):
            c.line(*a, *b, ink, 3)
    c.circle(24, 190, 5, warning)
    c.circle(145, 48, 5, warning)
    c.circle(177, 190, 4, rgba(48, 150, 76))
    return c


FIXTURES = {
    "logos/flat-logo.png": flat_logo,
    "logos/faded-logo.png": faded_logo,
    "alpha/transparent-drop.png": transparent_drop,
    "alpha/hidden-rgb.png": hidden_rgb,
    "color/tiny-accent.png": tiny_accent,
    "color/similar-blues.png": similar_blues,
    "pixel-art/grid-sprite.png": pixel_art,
    "fabrication/screen-print-trapping.png": screen_print,
    "fabrication/vinyl-closed-paths.png": vinyl_paths,
    "fabrication/laser-tiny-features.png": laser_features,
    "gradients/mountain-gradient.png": gradient_mountains,
    "line-art/noisy-paper.png": noisy_paper,
    "adversarial/open-vs-closed.png": open_closed,
}


def generate(root: Path) -> list[Path]:
    destination = root / "synthetic" / "evaluation"
    written = []
    for relative, factory in FIXTURES.items():
        path = destination / relative
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_bytes(png_bytes(factory()))
        written.append(path)
    return written


def main() -> int:
    root = Path(sys.argv[1]) if len(sys.argv) == 2 else Path(__file__).resolve().parents[1] / "fixtures"
    if len(sys.argv) > 2:
        raise SystemExit(__doc__)
    for path in generate(root):
        print(path)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
