"""
Color space conversion functions (sRGB, OKLab, OKLCH).
Implements exact formulas defined by Björn Ottosson (2020).
"""

import math
import numpy as np

def hex_to_srgb(hex_str: str) -> tuple[float, float, float]:
    """Convert '#RRGGBB' to (r, g, b) floats in range [0, 1]."""
    hex_str = hex_str.lstrip('#')
    if len(hex_str) != 6:
        raise ValueError(f"Invalid hex color: {hex_str}")
    r = int(hex_str[0:2], 16) / 255.0
    g = int(hex_str[2:4], 16) / 255.0
    b = int(hex_str[4:6], 16) / 255.0
    return (r, g, b)

def srgb_to_hex(r: float, g: float, b: float) -> str:
    """Convert (r, g, b) floats in [0, 1] to '#RRGGBB'."""
    r_int = max(0, min(255, round(r * 255)))
    g_int = max(0, min(255, round(g * 255)))
    b_int = max(0, min(255, round(b * 255)))
    return f"#{r_int:02X}{g_int:02X}{b_int:02X}"

def srgb_to_linear(c: float) -> float:
    """sRGB gamma correction to linear RGB."""
    if c <= 0.04045:
        return c / 12.92
    else:
        return math.pow((c + 0.055) / 1.055, 2.4)

def linear_to_srgb(c: float) -> float:
    """Linear RGB to sRGB gamma correction."""
    if c <= 0.0031308:
        return max(0.0, min(1.0, 12.92 * c))
    else:
        return max(0.0, min(1.0, 1.055 * math.pow(max(0.0, c), 1.0 / 2.4) - 0.055))

def srgb_to_oklab(r: float, g: float, b: float) -> tuple[float, float, float]:
    """Convert sRGB (0..1) to OKLab (L, a, b)."""
    lr = srgb_to_linear(r)
    lg = srgb_to_linear(g)
    lb = srgb_to_linear(b)

    l = 0.4122214708 * lr + 0.5363325363 * lg + 0.0514459929 * lb
    m = 0.2119034982 * lr + 0.6806995451 * lg + 0.1073969566 * lb
    s = 0.0883024619 * lr + 0.2817188376 * lg + 0.6299787005 * lb

    l_ = math.cbrt(l)
    m_ = math.cbrt(m)
    s_ = math.cbrt(s)

    L = 0.2104542553 * l_ + 0.7936177850 * m_ - 0.0040720468 * s_
    a = 1.9779984951 * l_ - 2.4285922050 * m_ + 0.4505937099 * s_
    b_val = 0.0259040371 * l_ + 0.7827717662 * m_ - 0.8086757993 * s_

    return (L, a, b_val)

def oklab_to_srgb(L: float, a: float, b_val: float) -> tuple[float, float, float]:
    """Convert OKLab (L, a, b) to sRGB (r, g, b)."""
    l_ = L + 0.3963377774 * a + 0.2158037573 * b_val
    m_ = L - 0.1055613458 * a - 0.0638541728 * b_val
    s_ = L - 0.0894841775 * a - 1.2914855480 * b_val

    l = l_ * l_ * l_
    m = m_ * m_ * m_
    s = s_ * s_ * s_

    lr = +4.0767416621 * l - 3.3077115913 * m + 0.2309699292 * s
    lg = -1.2684380046 * l + 2.6097574011 * m - 0.3413193965 * s
    lb = -0.0041960863 * l - 0.7034186147 * m + 1.7076147010 * s

    return (linear_to_srgb(lr), linear_to_srgb(lg), linear_to_srgb(lb))

def oklab_to_oklch(L: float, a: float, b_val: float) -> tuple[float, float, float]:
    """Convert OKLab (L, a, b) to OKLCH (L, C, h in degrees 0..360)."""
    C = math.sqrt(a * a + b_val * b_val)
    h_rad = math.atan2(b_val, a)
    h_deg = math.degrees(h_rad) % 360.0
    return (L, C, h_deg)

def oklch_to_oklab(L: float, C: float, h_deg: float) -> tuple[float, float, float]:
    """Convert OKLCH (L, C, h) to OKLab (L, a, b)."""
    h_rad = math.radians(h_deg)
    a = C * math.cos(h_rad)
    b_val = C * math.sin(h_rad)
    return (L, a, b_val)

def srgb_to_oklch(r: float, g: float, b: float) -> tuple[float, float, float]:
    """Convert sRGB (0..1) directly to OKLCH (L, C, h)."""
    L, a, b_val = srgb_to_oklab(r, g, b)
    return oklab_to_oklch(L, a, b_val)

def shortest_hue_distance(h1: float, h2: float) -> float:
    """Calculate shortest circular distance between two hue angles in degrees [0, 180]."""
    diff = abs(h1 - h2) % 360.0
    return min(diff, 360.0 - diff)
