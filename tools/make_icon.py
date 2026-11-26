"""Render assets/logo.svg to PNG and ICO with the Python standard library."""

import math
import struct
import xml.etree.ElementTree as ET
import zlib
from pathlib import Path

ASSETS = Path(__file__).resolve().parents[1] / "assets"
SIZES = (16, 20, 24, 32, 40, 48, 64, 128, 256)
SAMPLES = 4


def rounded_rect(x, y, width, height, radius):
    def contains(px, py):
        if not (x <= px <= x + width and y <= py <= y + height):
            return False
        cx = min(max(px, x + radius), x + width - radius)
        cy = min(max(py, y + radius), y + height - radius)
        return (px - cx) ** 2 + (py - cy) ** 2 <= radius**2

    return contains


def polygon(points):
    edges = list(zip(points, points[1:] + points[:1]))

    def contains(x, y):
        inside = False
        for (ax, ay), (bx, by) in edges:
            if (ay > y) != (by > y) and x < ax + (y - ay) * (bx - ax) / (by - ay):
                inside = not inside
        return inside

    return contains


def load_svg():
    """Read the untransformed, solid-fill shapes used in this logo."""
    svg = ET.parse(ASSETS / "logo.svg").getroot()
    x, y, width, height = map(float, svg.attrib["viewBox"].split())
    if (x, y) != (0, 0) or width != height:
        raise ValueError("Expected a square viewBox starting at the origin")
    shapes = []
    for node in svg:
        tag = node.tag.rsplit("}", 1)[-1]
        if tag == "title":
            continue
        if tag == "rect":
            values = [float(node.get(key, "0")) for key in ("x", "y", "width", "height", "rx")]
            contains = rounded_rect(*values)
        elif tag == "polygon":
            values = list(map(float, node.attrib["points"].replace(",", " ").split()))
            contains = polygon(list(zip(values[::2], values[1::2], strict=True)))
        else:
            raise ValueError(f"Unsupported SVG element: {tag}")
        colour = bytes.fromhex(node.attrib["fill"].removeprefix("#"))
        if len(colour) != 3:
            raise ValueError("Expected a solid RGB fill")
        shapes.append((contains, colour))
    return width, shapes[::-1]


def render(size, width, shapes):
    offsets = [(i + 0.5) / SAMPLES for i in range(SAMPLES)]
    coords = [[(i + offset) * width / size for offset in offsets] for i in range(size)]
    pixels = bytearray()
    for ys in coords:
        for xs in coords:
            r = g = b = hits = 0
            for y in ys:
                for x in xs:
                    for contains, colour in shapes:
                        if contains(x, y):
                            r += colour[0]
                            g += colour[1]
                            b += colour[2]
                            hits += 1
                            break
            pixels.extend(
                (round(r / hits), round(g / hits), round(b / hits), round(255 * hits / SAMPLES**2))
                if hits else (0, 0, 0, 0)
            )
    return pixels


def png(size, pixels):
    def chunk(kind, data):
        return struct.pack(">I", len(data)) + kind + data + struct.pack(">I", zlib.crc32(kind + data))

    stride = size * 4
    rows = b"".join(b"\0" + pixels[i:i + stride] for i in range(0, len(pixels), stride))
    return (
        b"\x89PNG\r\n\x1a\n"
        + chunk(b"IHDR", struct.pack(">IIBBBBB", size, size, 8, 6, 0, 0, 0))
        + chunk(b"IDAT", zlib.compress(rows, level=9))
        + chunk(b"IEND", b"")
    )


def dib(size, pixels):
    """32-bit Windows icon bitmap plus its 1-bit transparency mask."""
    stride = size * 4
    bitmap = bytearray()
    mask = bytearray()
    for row in range(size - 1, -1, -1):
        mask_row = bytearray(math.ceil(size / 32) * 4)
        for col in range(size):
            i = row * stride + col * 4
            r, g, b, a = pixels[i:i + 4]
            bitmap.extend((b, g, r, a))
            if a == 0:
                mask_row[col // 8] |= 0x80 >> (col % 8)
        mask.extend(mask_row)
    header = struct.pack("<IiiHHIIiiII", 40, size, size * 2, 1, 32, 0, 0, 0, 0, 0, 0)
    return header + bitmap + mask


def main():
    width, shapes = load_svg()
    entries = bytearray(struct.pack("<HHH", 0, 1, len(SIZES)))
    images = bytearray()
    offset = len(entries) + len(SIZES) * 16
    for size in SIZES:
        pixels = render(size, width, shapes)
        if size == 256:
            image = png(size, pixels)
            (ASSETS / "logo-256.png").write_bytes(image)
        else:
            image = dib(size, pixels)
        entries.extend(struct.pack("<BBBBHHII", size % 256, size % 256, 0, 0, 1, 32, len(image), offset))
        images.extend(image)
        offset += len(image)
    (ASSETS / "icon.ico").write_bytes(entries + images)


if __name__ == "__main__":
    main()
