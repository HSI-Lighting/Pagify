"""A page whose panel both paints itself and sets a clip for what follows.

`W f` is one path doing two jobs: it paints, so PDFium reports it as an object
somebody can click, and it clips, so everything drawn after it is held inside
it. Wrapping such a path in `q`/`Q` to move it would restore the clipping path
the instant the wrapper closed, and the words it was holding in would spill out
across the page — so it is refused, and this is what proves the refusal fires.
"""
import zlib, sys

content = b"""q
0.85 0.85 0.85 rg
100 400 200 150 re W f
BT /F1 30 Tf 1 0 0 1 110 430 Tm (Held inside the panel by its clip) Tj ET
Q
BT /F1 14 Tf 1 0 0 1 72 700 Tm (Outside, unclipped) Tj ET
"""
packed = zlib.compress(content)

objs = {}
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
print(f"wrote {sys.argv[1]}")
