#!/usr/bin/env python3
"""Turns the `frame.argb` that `AXON_SELFTEST=<folder>` writes (width and height as little-endian u32, then each
pixel's premultiplied ARGB u32) into a PNG, laid over a dark grey so the glass and the shadow can be seen.
Usage: argb2png.py frame.argb frame.png"""
import struct, sys, zlib
src, dst = sys.argv[1], sys.argv[2]
raw = open(src, 'rb').read()
w, h = struct.unpack('<II', raw[:8])
px = struct.unpack('<%dI' % (w * h), raw[8:8 + w * h * 4])
rows = []
for y in range(h):
    row = bytearray([0])
    for p in px[y * w:(y + 1) * w]:
        a = p >> 24
        r, g, b = (p >> 16) & 255, (p >> 8) & 255, p & 255
        # premultiplied over a grey checker-free dark-grey backdrop so the glass is visible
        bg = 40
        r, g, b = [min(255, c + bg * (255 - a) // 255) for c in (r, g, b)]
        row += bytes((r, g, b))
    rows.append(bytes(row))
def chunk(t, d):
    c = struct.pack('>I', len(d)) + t + d
    return c + struct.pack('>I', zlib.crc32(t + d) & 0xffffffff)
png = b'\x89PNG\r\n\x1a\n' + chunk(b'IHDR', struct.pack('>IIBBBBB', w, h, 8, 2, 0, 0, 0)) + chunk(b'IDAT', zlib.compress(b''.join(rows), 6)) + chunk(b'IEND', b'')
open(dst, 'wb').write(png)
print(w, h)
