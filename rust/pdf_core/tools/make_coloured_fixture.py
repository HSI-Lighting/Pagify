"""A page whose text is in three distinct colours, plus a coloured panel.

Exists so that "did this edit repaint the page" is answerable by counting
colours in a render. Every other fixture here is black on white, where a
re-emission that changed a colour would look exactly like one that did not.
"""
import zlib, sys

# Deliberately far apart, so a render at thumbnail size still separates them
# and no anti-aliased edge can be mistaken for one of the others.
content = b"""q
0.9 0.9 0.2 rg
72 600 200 80 re f
Q
q
BT 1 0 0 0 k /F1 24 Tf 1 0 0 1 72 700 Tm (Black heading) Tj ET
BT 0.85 0.1 0.1 rg /F1 24 Tf 1 0 0 1 72 660 Tm (Red line of words) Tj ET
BT 0.1 0.5 0.85 rg /F1 24 Tf 1 0 0 1 72 500 Tm (Blue line of words) Tj ET
BT 0.1 0.6 0.2 rg /F1 24 Tf 1 0 0 1 72 460 Tm (Green line of words) Tj ET
Q
"""

objs = {}
packed = zlib.compress(content)
objs[5] = b"<< /Filter /FlateDecode /Length %d >>\nstream\n" % len(packed) + packed + b"\nendstream"
objs[1] = b"<< /Type /Catalog /Pages 2 0 R >>"
objs[2] = b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>"
objs[3] = (b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Contents 5 0 R "
           b"/Resources << /Font << /F1 4 0 R >> >> >>")
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
print(f"wrote {sys.argv[1]} ({len(out)} bytes)")
