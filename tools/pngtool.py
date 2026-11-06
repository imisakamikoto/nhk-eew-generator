"""Tiny stdlib-only PNG reader/writer (8-bit RGBA/RGB, non-interlaced).

Used for pixel-accurate analysis of the reference screenshots.
"""
import struct
import zlib


def read_png(path):
    data = open(path, 'rb').read()
    assert data[:8] == b'\x89PNG\r\n\x1a\n'
    pos = 8
    idat = bytearray()
    while pos < len(data):
        length = struct.unpack('>I', data[pos:pos + 4])[0]
        typ = data[pos + 4:pos + 8]
        chunk = data[pos + 8:pos + 8 + length]
        pos += 12 + length
        if typ == b'IHDR':
            w, h, depth, ctype, comp, filt, inter = struct.unpack('>IIBBBBB', chunk)
        elif typ == b'IDAT':
            idat += chunk
        elif typ == b'IEND':
            break
    assert depth == 8 and ctype in (2, 6) and inter == 0, (depth, ctype, inter)
    nch = 4 if ctype == 6 else 3
    raw = zlib.decompress(bytes(idat))
    stride = w * nch
    out = bytearray(w * h * 4)
    prev = bytearray(stride)
    p = 0
    for y in range(h):
        f = raw[p]
        p += 1
        line = bytearray(raw[p:p + stride])
        p += stride
        if f == 1:
            for i in range(nch, stride):
                line[i] = (line[i] + line[i - nch]) & 255
        elif f == 2:
            for i in range(stride):
                line[i] = (line[i] + prev[i]) & 255
        elif f == 3:
            for i in range(stride):
                a = line[i - nch] if i >= nch else 0
                line[i] = (line[i] + ((a + prev[i]) >> 1)) & 255
        elif f == 4:
            for i in range(stride):
                a = line[i - nch] if i >= nch else 0
                b = prev[i]
                c = prev[i - nch] if i >= nch else 0
                pa, pb, pc = abs(b - c), abs(a - c), abs(a + b - 2 * c)
                pr = a if pa <= pb and pa <= pc else (b if pb <= pc else c)
                line[i] = (line[i] + pr) & 255
        elif f != 0:
            raise ValueError('bad filter %d' % f)
        for x in range(w):
            out[(y * w + x) * 4:(y * w + x) * 4 + 4] = bytes(line[x * nch:x * nch + nch]) + (b'\xff' if nch == 3 else b'')
        prev = line
    return w, h, out


class Image:
    def __init__(self, w, h, buf=None, color=(0, 0, 0, 0)):
        self.w, self.h = w, h
        self.buf = bytearray(buf) if buf is not None else bytearray(bytes(color) * (w * h))

    @classmethod
    def open(cls, path):
        w, h, b = read_png(path)
        return cls(w, h, b)

    def px(self, x, y):
        i = (y * self.w + x) * 4
        return tuple(self.buf[i:i + 4])

    def set(self, x, y, c):
        i = (y * self.w + x) * 4
        self.buf[i:i + 4] = bytes(c)

    def crop(self, box):
        x0, y0, x1, y1 = box
        nw, nh = x1 - x0, y1 - y0
        out = bytearray(nw * nh * 4)
        for y in range(nh):
            out[y * nw * 4:(y + 1) * nw * 4] = self.buf[((y + y0) * self.w + x0) * 4:((y + y0) * self.w + x1) * 4]
        return Image(nw, nh, out)

    def scale(self, k):
        nw, nh = self.w * k, self.h * k
        out = bytearray(nw * nh * 4)
        for y in range(nh):
            for x in range(nw):
                out[(y * nw + x) * 4:(y * nw + x) * 4 + 4] = self.buf[((y // k) * self.w + x // k) * 4:((y // k) * self.w + x // k) * 4 + 4]
        return Image(nw, nh, out)

    def save(self, path, level=9):
        raw = bytearray()
        for y in range(self.h):
            raw.append(0)
            raw += self.buf[y * self.w * 4:(y + 1) * self.w * 4]
        comp = zlib.compress(bytes(raw), level)
        def chunk(t, d):
            return struct.pack('>I', len(d)) + t + d + struct.pack('>I', zlib.crc32(t + d) & 0xffffffff)
        png = b'\x89PNG\r\n\x1a\n'
        png += chunk(b'IHDR', struct.pack('>IIBBBBB', self.w, self.h, 8, 6, 0, 0, 0))
        png += chunk(b'IDAT', comp)
        png += chunk(b'IEND', b'')
        open(path, 'wb').write(png)
