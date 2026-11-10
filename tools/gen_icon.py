"""Generate Isle icons (pure stdlib): dark rounded square + white pill + blue dot.

Writes src-tauri/icons/{icon.ico,32x32.png,128x128.png,icon.png}.
The .ico uses a classic 32-bit BGRA bitmap entry so resource compilers accept it.
"""
import struct
import zlib
import os

OUT = os.path.join(os.path.dirname(__file__), "..", "src-tauri", "icons")


def render(size, ss=3):
    """Render the glyph at `size` px with `ss`x supersampling → RGBA bytes."""
    S = size * ss
    acc = [[0.0, 0.0, 0.0, 0.0] for _ in range(size * size)]

    def inside_rounded(px, py, x0, y0, x1, y1, rad):
        rx = max(x0 + rad - px, 0.0, px - (x1 - rad))
        ry = max(y0 + rad - py, 0.0, py - (y1 - rad))
        return rx * rx + ry * ry <= rad * rad

    for sy in range(S):
        for sx in range(S):
            u = (sx + 0.5) / S
            v = (sy + 0.5) / S
            if not inside_rounded(u, v, 0.06, 0.06, 0.94, 0.94, 0.24):
                continue
            if inside_rounded(u, v, 0.14, 0.40, 0.86, 0.60, 0.10):
                px = (u - 0.74) ** 2 + (v - 0.50) ** 2
                if px <= 0.055 * 0.055:
                    color = (76, 194, 255, 255)  # blue lens dot
                else:
                    color = (245, 245, 248, 255)  # white pill
            else:
                color = (31, 31, 39, 255)  # dark square
            idx = (sy // ss) * size + (sx // ss)
            cell = acc[idx]
            for i in range(4):
                cell[i] += color[i]

    out = bytearray(size * size * 4)
    n = ss * ss
    for i, cell in enumerate(acc):
        for c in range(4):
            out[i * 4 + c] = int(cell[c] / n + 0.5)
    return bytes(out)


def png_bytes(size, rgba):
    raw = bytearray()
    stride = size * 4
    for y in range(size):
        raw.append(0)  # filter: none
        raw.extend(rgba[y * stride : (y + 1) * stride])

    def chunk(tag, data):
        body = tag + data
        return (
            struct.pack(">I", len(data))
            + body
            + struct.pack(">I", zlib.crc32(body) & 0xFFFFFFFF)
        )

    png = b"\x89PNG\r\n\x1a\n"
    png += chunk(b"IHDR", struct.pack(">IIBBBBB", size, size, 8, 6, 0, 0, 0))
    png += chunk(b"IDAT", zlib.compress(bytes(raw), 9))
    png += chunk(b"IEND", b"")
    return png


def write_file(path, data):
    with open(path, "wb") as f:
        f.write(data)


def bmp_icon_entry(size, rgba):
    """Classic ICO entry: BITMAPINFOHEADER + bottom-up BGRA + empty AND mask."""
    # 32-bit BGRA rows, bottom-up.
    pixels = bytearray(size * size * 4)
    for y in range(size):
        src_row = rgba[(size - 1 - y) * size * 4 : (size - y) * size * 4]
        for x in range(size):
            r, g, b, a = src_row[x * 4 : x * 4 + 4]
            pixels[y * size * 4 + x * 4 + 0] = b
            pixels[y * size * 4 + x * 4 + 1] = g
            pixels[y * size * 4 + x * 4 + 2] = r
            pixels[y * size * 4 + x * 4 + 3] = a
    and_mask_rows = ((size + 31) // 32) * 4
    and_mask = bytes(and_mask_rows * size)
    header = struct.pack(
        "<IiiHHIIiiII",
        40,
        size,
        size * 2,
        1,
        32,
        0,
        len(pixels) + len(and_mask),
        0,
        0,
        0,
        0,
    )
    return header + bytes(pixels) + and_mask


def write_ico(path, size, rgba):
    entry = bmp_icon_entry(size, rgba)
    data_offset = 6 + 16
    ico = struct.pack("<HHH", 0, 1, 1)
    ico += struct.pack(
        "<BBBBHHII",
        0 if size >= 256 else size,
        0 if size >= 256 else size,
        0,
        0,
        1,
        32,
        len(entry),
        data_offset,
    )
    ico += entry
    write_file(path, ico)


os.makedirs(OUT, exist_ok=True)
rgba256 = render(256, ss=2)
write_ico(os.path.join(OUT, "icon.ico"), 256, rgba256)
write_file(os.path.join(OUT, "icon.png"), png_bytes(256, rgba256))
write_file(os.path.join(OUT, "128x128.png"), png_bytes(128, render(128, ss=3)))
write_file(os.path.join(OUT, "32x32.png"), png_bytes(32, render(32, ss=4)))
print("icons written to", os.path.abspath(OUT))
