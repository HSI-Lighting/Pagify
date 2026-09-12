"""A page where a picture is drawn *before* an opaque panel that covers it.

Exists because "my picture went behind a layer" is not something the other
fixtures can express: in every one of them the pictures are drawn last, so they
are on top and no amount of moving them can put them underneath anything.
"""
import zlib, sys

def img(w, h, rgb):
    return zlib.compress(bytes(rgb) * (w * h))

objs = {}
data = img(4, 4, (220, 40, 40))
objs[11] = (b"<< /Type /XObject /Subtype /Image /Width 4 /Height 4 "
            b"/ColorSpace /DeviceRGB /BitsPerComponent 8 /Filter /FlateDecode /Length %d >>\nstream\n" % len(data)
            + data + b"\nendstream")

# The picture first, then a grey panel over most of it — the ordinary layout of
# a brochure, and the ordinary way a picture ends up underneath something.
content = b"""q 200 0 0 120 100 500 cm /Im0 Do Q
q 0.45 0.45 0.45 rg 140 460 320 200 re f Q
BT /F1 14 Tf 1 0 0 1 160 560 Tm (Panel over the picture) Tj ET
"""
packed = zlib.compress(content)
objs[5] = b"<< /Filter /FlateDecode /Length %d >>\nstream\n" % len(packed) + packed + b"\nendstream"
objs[1] = b"<< /Type /Catalog /Pages 2 0 R >>"
objs[2] = b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>"
objs[3] = (b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Contents 5 0 R "
           b"/Resources << /Font << /F1 4 0 R >> /XObject << /Im0 11 0 R >> >> >>")
objs[4] = b"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>"

out = bytearray(b"%PDF-1.7\n%\xe2\xe3\xcf\xd3\n")
offsets = {}
for num in sorted(objs):
    offsets[num] = len(out)
    out += b"%d 0 obj\n" % num + objs[num] + b"\nendobj\n"
start = len(out)
top = max(objs) + 1
out += b"xref\n0 %d\n" % top + b"0000000000 65535 f \n"
for num in range(1, top):
    out += (b"%010d 00000 n \n" % offsets[num]) if num in offsets else b"0000000000 65535 f \n"
out += b"trailer\n<< /Size %d /Root 1 0 R >>\nstartxref\n%d\n%%%%EOF\n" % (top, start)
open(sys.argv[1], "wb").write(bytes(out))
print(f"wrote {sys.argv[1]}")
