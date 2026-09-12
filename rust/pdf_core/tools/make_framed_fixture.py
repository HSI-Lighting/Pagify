"""A page whose picture is placed the way a design program places one.

Grey placeholder rectangle, then a clip exactly the frame's size, then the
picture inside it, then a caption drawn over the top:

    0.659 0.662 0.664 rg  x y w h re f
    q  x y w h re W n  q  w 0 0 h x y cm  /Im0 Do  Q  Q
    BT ... (caption) ET

Moving the `Do` alone slides the picture out of its own clip and off into
nothing, leaving the grey rectangle showing — which is what a real brochure did.
"""
import zlib, sys

def img(w, h, rgb):
    return zlib.compress(bytes(rgb) * (w * h))

objs = {}
data = img(4, 4, (220, 40, 40))
objs[11] = (b"<< /Type /XObject /Subtype /Image /Width 4 /Height 4 /ColorSpace /DeviceRGB "
            b"/BitsPerComponent 8 /Filter /FlateDecode /Length %d >>\nstream\n" % len(data) + data + b"\nendstream")

content = b"""BT /F1 14 Tf 1 0 0 1 72 740 Tm (A framed picture, the way a brochure places one) Tj ET
0.659 0.662 0.664 rg
100 400 200 150 re
f
q
100 400 200 150 re
W
n
q
200 0 0 150 100 400 cm
/Im0 Do
Q
Q
BT /F1 12 Tf 1 1 1 rg 1 0 0 1 110 410 Tm (Caption over the picture) Tj ET
BT /F1 11 Tf 0 g 1 0 0 1 72 300 Tm (Text below it all.) Tj ET
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
