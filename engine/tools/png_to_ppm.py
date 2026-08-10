#!/usr/bin/env python3
"""Convert a PNG to a binary PPM or PAM, using only the Python standard library.

The engine's core takes decoded pixels (PTE-ARCH-001), and `palette-tracer-codecs`
(SPEC.md §5.1) is not built, so `pte` reads Netpbm rather than PNG. This script
bridges the gap for the documented end-to-end run without adding a codec
dependency to the engine.

Supports the subset the repository's own fixtures use: 8-bit, non-interlaced,
colour types 0 (grey), 2 (RGB), 4 (grey+alpha) and 6 (RGBA). Anything else is
refused by name rather than approximated.

    python3 tools/png_to_ppm.py input.png output.ppm
"""

import struct
import sys
import zlib

CHANNELS = {0: 1, 2: 3, 4: 2, 6: 4}


def paeth(a, b, c):
    p = a + b - c
    pa, pb, pc = abs(p - a), abs(p - b), abs(p - c)
    if pa <= pb and pa <= pc:
        return a
    return b if pb <= pc else c


def decode(data):
    if data[:8] != b"\x89PNG\r\n\x1a\n":
        raise SystemExit("not a PNG")

    width = height = depth = color_type = interlace = None
    idat = bytearray()
    offset = 8
    while offset < len(data):
        (length,) = struct.unpack(">I", data[offset : offset + 4])
        kind = data[offset + 4 : offset + 8]
        body = data[offset + 8 : offset + 8 + length]
        offset += 12 + length

        if kind == b"IHDR":
            width, height, depth, color_type, _, _, interlace = struct.unpack(
                ">IIBBBBB", body
            )
        elif kind == b"IDAT":
            idat += body
        elif kind == b"IEND":
            break

    if depth != 8:
        raise SystemExit(f"bit depth {depth} is not supported; only 8 is")
    if interlace != 0:
        raise SystemExit("interlaced PNG is not supported")
    if color_type not in CHANNELS:
        raise SystemExit(f"colour type {color_type} is not supported")

    channels = CHANNELS[color_type]
    stride = width * channels
    raw = zlib.decompress(bytes(idat))

    out = bytearray()
    previous = bytearray(stride)
    pos = 0
    for _ in range(height):
        filter_type = raw[pos]
        pos += 1
        line = bytearray(raw[pos : pos + stride])
        pos += stride

        for i in range(stride):
            left = line[i - channels] if i >= channels else 0
            up = previous[i]
            up_left = previous[i - channels] if i >= channels else 0
            if filter_type == 1:
                line[i] = (line[i] + left) & 0xFF
            elif filter_type == 2:
                line[i] = (line[i] + up) & 0xFF
            elif filter_type == 3:
                line[i] = (line[i] + (left + up) // 2) & 0xFF
            elif filter_type == 4:
                line[i] = (line[i] + paeth(left, up, up_left)) & 0xFF
            elif filter_type != 0:
                raise SystemExit(f"unknown filter {filter_type}")
        out += line
        previous = line

    return width, height, channels, bytes(out)


def main():
    if len(sys.argv) != 3:
        raise SystemExit(__doc__)
    source, destination = sys.argv[1], sys.argv[2]
    width, height, channels, pixels = decode(open(source, "rb").read())

    with open(destination, "wb") as handle:
        if channels == 3:
            handle.write(b"P6\n%d %d\n255\n" % (width, height))
            handle.write(pixels)
        else:
            tuple_type = {1: "GRAYSCALE", 2: "GRAYSCALE_ALPHA", 4: "RGB_ALPHA"}[channels]
            handle.write(
                b"P7\nWIDTH %d\nHEIGHT %d\nDEPTH %d\nMAXVAL 255\nTUPLTYPE %s\nENDHDR\n"
                % (width, height, channels, tuple_type.encode())
            )
            handle.write(pixels)
    print(f"{destination}: {width}x{height}, {channels} channels", file=sys.stderr)


if __name__ == "__main__":
    main()
