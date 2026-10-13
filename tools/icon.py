#!/usr/bin/env python3
"""Draws Lokal's mark and writes the page's icons from one definition.

    python3 tools/icon.py

The mark is the prompt's chevron and the breathing dot on a dark tile. It is
plain geometry, so it is drawn here directly, each pixel sampled 8 x 8 times
for smooth edges, and every file comes from the same numbers:

    site/lokal-logo.svg   the mark, for the page and the browser's tab
    site/lokal-icon.png   256 px, for lists of apps and link previews
    site/favicon.ico      16, 32 and 48 px

Small sizes get their own drawing, heavier, so the chevron survives 16 px.
Only the standard library: a PNG is a zlib stream with a few chunks around it.
"""

import math
import struct
import zlib
from pathlib import Path

TILE = (0x13, 0x12, 0x11)
CHEVRON = (0xEC, 0xE8, 0xE3)
DOT = (0xD7, 0x87, 0x5F)

# On a 64-unit square: the tile's corner radius, the chevron's three points
# and stroke width, and the dot's centre and radius. The chevron and the dot
# keep a gap between them, and together they sit centred on the tile.
LARGE = {"corner": 14, "chevron": [(14, 19), (28, 32), (14, 45)], "stroke": 7, "dot": (44, 32, 9.5)}
SMALL = {"corner": 12, "chevron": [(12, 17), (27, 32), (12, 47)], "stroke": 9.5, "dot": (46, 32, 10.5)}

SAMPLES = 8
ROOT = Path(__file__).resolve().parent.parent


def drawing_for(size: int) -> dict:
    return SMALL if size <= 24 else LARGE


def in_tile(x: float, y: float, corner: float) -> bool:
    # Distance outside the square shrunk by the corner radius.
    dx = max(corner - x, 0, x - (64 - corner))
    dy = max(corner - y, 0, y - (64 - corner))
    return dx * dx + dy * dy <= corner * corner


def near_segment(x, y, a, b, half_width) -> bool:
    (ax, ay), (bx, by) = a, b
    vx, vy = bx - ax, by - ay
    t = max(0.0, min(1.0, ((x - ax) * vx + (y - ay) * vy) / (vx * vx + vy * vy)))
    px, py = ax + t * vx - x, ay + t * vy - y
    return px * px + py * py <= half_width * half_width


def colour_at(x: float, y: float, drawing: dict):
    """The colour at a point of the 64-unit square, or None outside the tile."""
    if not in_tile(x, y, drawing["corner"]):
        return None
    cx, cy, r = drawing["dot"]
    if (x - cx) ** 2 + (y - cy) ** 2 <= r * r:
        return DOT
    points, half = drawing["chevron"], drawing["stroke"] / 2
    if any(near_segment(x, y, points[i], points[i + 1], half) for i in range(len(points) - 1)):
        return CHEVRON
    return TILE


def render(size: int) -> bytes:
    """Straight-alpha RGBA, row by row."""
    drawing = drawing_for(size)
    scale = 64 / size
    out = bytearray()
    for row in range(size):
        for column in range(size):
            totals, covered = [0, 0, 0], 0
            for sy in range(SAMPLES):
                for sx in range(SAMPLES):
                    x = (column + (sx + 0.5) / SAMPLES) * scale
                    y = (row + (sy + 0.5) / SAMPLES) * scale
                    colour = colour_at(x, y, drawing)
                    if colour:
                        covered += 1
                        for i in range(3):
                            totals[i] += colour[i]
            if covered:
                out += bytes(round(total / covered) for total in totals)
                out.append(round(255 * covered / (SAMPLES * SAMPLES)))
            else:
                out += b"\x00\x00\x00\x00"
    return bytes(out)


def png(size: int, rgba: bytes) -> bytes:
    def chunk(kind: bytes, data: bytes) -> bytes:
        body = kind + data
        return struct.pack(">I", len(data)) + body + struct.pack(">I", zlib.crc32(body) & 0xFFFFFFFF)

    stride = size * 4
    # Filter type 0 (none) on every row; zlib does the rest.
    raw = b"".join(b"\x00" + rgba[y * stride:(y + 1) * stride] for y in range(size))
    header = struct.pack(">IIBBBBB", size, size, 8, 6, 0, 0, 0)
    return (b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", header)
            + chunk(b"IDAT", zlib.compress(raw, 9)) + chunk(b"IEND", b""))


def ico(images) -> bytes:
    # ICONDIR, then one 16-byte entry per image, then the images, as PNG.
    offset = 6 + 16 * len(images)
    out = struct.pack("<HHH", 0, 1, len(images))
    blobs = b""
    for size, blob in images:
        dim = 0 if size >= 256 else size
        out += struct.pack("<BBBBHHII", dim, dim, 0, 0, 1, 32, len(blob), offset)
        offset += len(blob)
        blobs += blob
    return out + blobs


def svg() -> str:
    d = LARGE
    (ax, ay), (bx, by), (cx2, cy2) = d["chevron"]
    x, y, r = d["dot"]
    hex_of = lambda c: "#%02x%02x%02x" % c
    return (
        '<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 64 64">'
        f'<rect width="64" height="64" rx="{d["corner"]}" fill="{hex_of(TILE)}"/>'
        f'<path d="M{ax} {ay}L{bx} {by}L{cx2} {cy2}" fill="none" stroke="{hex_of(CHEVRON)}" '
        f'stroke-width="{d["stroke"]}" stroke-linecap="round" stroke-linejoin="round"/>'
        f'<circle cx="{x}" cy="{y}" r="{r}" fill="{hex_of(DOT)}"/>'
        "</svg>\n"
    )


def main() -> None:
    site = ROOT / "site"
    site.mkdir(exist_ok=True)
    (site / "lokal-logo.svg").write_text(svg())
    (site / "lokal-icon.png").write_bytes(png(256, render(256)))
    (site / "favicon.ico").write_bytes(ico([(size, png(size, render(size))) for size in (16, 32, 48)]))
    for name in ("lokal-logo.svg", "lokal-icon.png", "favicon.ico"):
        print(f"site/{name}: {(site / name).stat().st_size} bytes")


if __name__ == "__main__":
    main()
