"""A page with text and two pictures, placed the way real producers place them."""
import zlib, sys, struct

def img_stream(w, h, rgb):
    raw = bytes(rgb) * (w * h)
    return zlib.compress(raw)

objs = {}

# Two small images.
for n, (name, colour) in enumerate([("Im0", (220, 40, 40)), ("Im1", (40, 90, 220))], start=1):
    data = img_stream(4, 4, colour)
    objs[10 + n] = (
        b"<< /Type /XObject /Subtype /Image /Width 4 /Height 4 "
        b"/ColorSpace /DeviceRGB /BitsPerComponent 8 /Filter /FlateDecode /Length %d >>\nstream\n" % len(data)
        + data + b"\nendstream"
    )

content = b"""q
BT /F1 14 Tf 1 0 0 1 72 720 Tm (Pagify moving fixture) Tj ET
BT /F1 11 Tf 1 0 0 1 72 690 Tm (A paragraph that must not move when a picture does.) Tj ET
Q
q 120 0 0 60 72 560 cm /Im0 Do Q
q 1 0 0 1 300 400 cm
q 90 0 0 45 0 0 cm /Im1 Do Q
Q
BT /F1 11 Tf 1 0 0 1 72 360 Tm (Text below both pictures.) Tj ET
"""
packed = zlib.compress(content)
objs[5] = b"<< /Filter /FlateDecode /Length %d >>\nstream\n" % len(packed) + packed + b"\nendstream"
objs[1] = b"<< /Type /Catalog /Pages 2 0 R >>"
objs[2] = b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>"
objs[3] = (b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Contents 5 0 R "
           b"/Resources << /Font << /F1 4 0 R >> /XObject << /Im0 11 0 R /Im1 12 0 R >> >> >>")
objs[4] = b"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>"

out = bytearray(b"%PDF-1.7\n%\xe2\xe3\xcf\xd3\n")
offsets = {}
for num in sorted(objs):
    offsets[num] = len(out)
    out += b"%d 0 obj\n" % num + objs[num] + b"\nendobj\n"
start = len(out)
top = max(objs) + 1
out += b"xref\n0 %d\n" % top
out += b"0000000000 65535 f \n"
for num in range(1, top):
    out += (b"%010d 00000 n \n" % offsets[num]) if num in offsets else b"0000000000 65535 f \n"
out += b"trailer\n<< /Size %d /Root 1 0 R >>\nstartxref\n%d\n%%%%EOF\n" % (top, start)
open(sys.argv[1], "wb").write(bytes(out))
print(f"wrote {sys.argv[1]} ({len(out)} bytes)")
