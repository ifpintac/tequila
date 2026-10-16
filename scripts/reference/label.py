"""Artwork for the bottle's label, for the M6 reference render.

    python scripts/reference/label.py OUT_DIR

Writes two PNGs of the same size, meant to be laid over each other as two
label layers with one mapping: `label.png`, printed paper with its ink, and
`foil.png`, the gold that is hot-stamped over it (a Foil layer). A made-up
brand, lettered with a 5x7 bitmap font so nothing but the standard library is
needed.
"""

import os
import struct
import sys
import zlib

W, H = 720, 600          # 72 mm by 60 mm at 10 px/mm
SS = 2                   # drawn at twice the size, then averaged down

FONT = {
    'A': ['01110', '10001', '10001', '11111', '10001', '10001', '10001'],
    'B': ['11110', '10001', '10001', '11110', '10001', '10001', '11110'],
    'C': ['01110', '10001', '10000', '10000', '10000', '10001', '01110'],
    'D': ['11110', '10001', '10001', '10001', '10001', '10001', '11110'],
    'E': ['11111', '10000', '10000', '11110', '10000', '10000', '11111'],
    'F': ['11111', '10000', '10000', '11110', '10000', '10000', '10000'],
    'G': ['01110', '10001', '10000', '10111', '10001', '10001', '01111'],
    'H': ['10001', '10001', '10001', '11111', '10001', '10001', '10001'],
    'I': ['01110', '00100', '00100', '00100', '00100', '00100', '01110'],
    'K': ['10001', '10010', '10100', '11000', '10100', '10010', '10001'],
    'L': ['10000', '10000', '10000', '10000', '10000', '10000', '11111'],
    'M': ['10001', '11011', '10101', '10101', '10001', '10001', '10001'],
    'N': ['10001', '11001', '10101', '10011', '10001', '10001', '10001'],
    'O': ['01110', '10001', '10001', '10001', '10001', '10001', '01110'],
    'R': ['11110', '10001', '10001', '11110', '10100', '10010', '10001'],
    'S': ['01111', '10000', '10000', '01110', '00001', '00001', '11110'],
    'T': ['11111', '00100', '00100', '00100', '00100', '00100', '00100'],
    'V': ['10001', '10001', '10001', '10001', '10001', '01010', '00100'],
    'W': ['10001', '10001', '10001', '10101', '10101', '10101', '01010'],
    'Y': ['10001', '10001', '01010', '00100', '00100', '00100', '00100'],
    '1': ['00100', '01100', '00100', '00100', '00100', '00100', '01110'],
    '2': ['01110', '10001', '00001', '00010', '00100', '01000', '11111'],
    '.': ['00000', '00000', '00000', '00000', '00000', '01100', '01100'],
    '%': ['11001', '11010', '00010', '00100', '01000', '01011', '10011'],
    '7': ['11111', '00001', '00010', '00100', '01000', '01000', '01000'],
    '4': ['00010', '00110', '01010', '10010', '11111', '00010', '00010'],
    '0': ['01110', '10001', '10011', '10101', '11001', '10001', '01110'],
    ' ': ['00000'] * 7,
}


class Canvas:
    """A coverage mask at SS times the output size."""

    def __init__(self):
        self.w, self.h = W * SS, H * SS
        self.px = bytearray(self.w * self.h)

    def rect(self, x0, y0, x1, y1, value=1):
        x0, y0, x1, y1 = (int(round(v * SS)) for v in (x0, y0, x1, y1))
        x0, x1 = max(0, x0), min(self.w, x1)
        for y in range(max(0, y0), min(self.h, y1)):
            row = y * self.w
            self.px[row + x0:row + x1] = bytes([value]) * max(0, x1 - x0)

    def frame(self, x0, y0, x1, y1, t):
        self.rect(x0, y0, x1, y0 + t)
        self.rect(x0, y1 - t, x1, y1)
        self.rect(x0, y0, x0 + t, y1)
        self.rect(x1 - t, y0, x1, y1)

    def ring(self, cx, cy, r0, r1):
        cx, cy, r0, r1 = cx * SS, cy * SS, r0 * SS, r1 * SS
        for y in range(int(cy - r1) - 1, int(cy + r1) + 2):
            for x in range(int(cx - r1) - 1, int(cx + r1) + 2):
                d2 = (x + 0.5 - cx) ** 2 + (y + 0.5 - cy) ** 2
                if r0 * r0 <= d2 <= r1 * r1 and 0 <= x < self.w and 0 <= y < self.h:
                    self.px[y * self.w + x] = 1

    def rounded(self, x0, y0, x1, y1, radius):
        self.rect(x0 + radius, y0, x1 - radius, y1)
        self.rect(x0, y0 + radius, x1, y1 - radius)
        for cx, cy in ((x0 + radius, y0 + radius), (x1 - radius, y0 + radius),
                       (x0 + radius, y1 - radius), (x1 - radius, y1 - radius)):
            self.ring(cx, cy, 0, radius)

    def text(self, s, cx, top, cell, spacing=1.0):
        """Centred on `cx`, each font pixel `cell` output pixels square."""
        advance = (5 + spacing) * cell
        width = len(s) * advance - spacing * cell
        x = cx - width / 2
        for ch in s:
            for row, bits in enumerate(FONT[ch]):
                for col, bit in enumerate(bits):
                    if bit == '1':
                        self.rect(x + col * cell, top + row * cell, x + (col + 1) * cell, top + (row + 1) * cell)
            x += advance

    def coverage(self):
        """Averaged down to the output size, 0 to 255."""
        out = bytearray(W * H)
        n = SS * SS
        for y in range(H):
            for x in range(W):
                total = 0
                for dy in range(SS):
                    row = (y * SS + dy) * self.w + x * SS
                    total += sum(self.px[row:row + SS])
                out[y * W + x] = total * 255 // n
        return out


def write_png(path, rgba):
    raw = b''.join(b'\x00' + bytes(rgba[y * W * 4:(y + 1) * W * 4]) for y in range(H))

    def chunk(kind, data):
        body = kind + data
        return struct.pack('>I', len(data)) + body + struct.pack('>I', zlib.crc32(body) & 0xffffffff)

    # 10 px/mm, said in the file, so the label arrives at its printed size.
    per_metre = 10000
    with open(path, 'wb') as f:
        f.write(b'\x89PNG\r\n\x1a\n')
        f.write(chunk(b'IHDR', struct.pack('>IIBBBBB', W, H, 8, 6, 0, 0, 0)))
        f.write(chunk(b'pHYs', struct.pack('>IIB', per_metre, per_metre, 1)))
        f.write(chunk(b'IDAT', zlib.compress(raw, 9)))
        f.write(chunk(b'IEND', b''))


def label(out_dir):
    paper, ink, foil = Canvas(), Canvas(), Canvas()
    paper.rounded(0, 0, W, H, 18)

    # Ink: a double rule inside the edge, the lettering, a rule under the name.
    ink.frame(22, 22, W - 22, H - 22, 3)
    ink.frame(32, 32, W - 32, H - 32, 1.2)
    ink.text('SINGLE MALT', W / 2, 262, 5.2, 1.4)
    ink.rect(160, 318, W - 160, 320.5)
    ink.text('AGED 12 YEARS', W / 2, 344, 3.6, 1.6)
    ink.text('SMALL BATCH', W / 2, 452, 2.8, 1.8)
    ink.text('70CL 40%', W / 2, 500, 2.8, 1.8)

    # Foil: the emblem, a ring round a monogram, and the name.
    foil.ring(W / 2, 122, 66, 74)
    foil.ring(W / 2, 122, 58, 60)
    foil.text('F', W / 2, 92, 9.0)
    foil.text('TEQUILA', W / 2, 210, 5.0, 1.3)
    foil.rect(210, 420, W - 210, 423)

    p, i, g = paper.coverage(), ink.coverage(), foil.coverage()
    cream, dark = (236, 226, 204), (58, 34, 22)
    printed = bytearray(W * H * 4)
    stamped = bytearray(W * H * 4)
    for k in range(W * H):
        a = i[k] / 255
        printed[k * 4:k * 4 + 3] = bytes(int(cream[c] * (1 - a) + dark[c] * a) for c in range(3))
        printed[k * 4 + 3] = p[k]
        stamped[k * 4:k * 4 + 3] = bytes((222, 176, 92))
        stamped[k * 4 + 3] = g[k]
    write_png(os.path.join(out_dir, 'label.png'), printed)
    write_png(os.path.join(out_dir, 'foil.png'), stamped)


if __name__ == '__main__':
    out_dir = sys.argv[1] if len(sys.argv) > 1 else '.'
    os.makedirs(out_dir, exist_ok=True)
    label(out_dir)
    print('wrote label.png and foil.png to', out_dir)
