"""A page whose text is inside a marked-content sequence with a very long tag.

PDFium reports a mark's name through a fixed buffer and tells the caller how
long the name *is*; a name longer than the buffer reports a length past its
end. The security audit found the check for Pagify's own marks slicing by
that length, which collapsed the document on open. This is that document.
"""
import zlib, sys

tag = b"/" + b"A" * 120
content = (tag + b" << /MCID 0 >> BDC\n"
           b"BT /F1 18 Tf 1 0 0 1 72 700 Tm (Marked with a very long tag) Tj ET\n"
           b"EMC\n"
           b"BT /F1 12 Tf 1 0 0 1 72 670 Tm (And a plain line after it) Tj ET\n")
packed = zlib.compress(content)

objs = {}
objs[1] = b"<< /Type /Catalog /Pages 2 0 R >>"
objs[2] = b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>"
objs[3] = (b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Contents 5 0 R "
           b"/Resources << /Font << /F1 4 0 R >> >> >>")
objs[4] = b"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>"
objs[5] = b"<< /Filter /FlateDecode /Length %d >>\nstream\n" % len(packed) + packed + b"\nendstream"

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
