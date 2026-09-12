"""A page whose sensitive text is drawn through a Form XObject.

The audit's probe: a card number that text extraction finds and page-level
redaction cannot reach, because the text object lives inside the form and the
redaction pass does not descend into one. What Pagify says about redacting it
is what the fixture is for.
"""
import zlib, sys

# The card number is the classic test number, which passes Luhn.
inner = b"""BT /F1 14 Tf 1 0 0 1 10 20 Tm (Card on file: 4111 1111 1111 1111) Tj ET
"""
packed_inner = zlib.compress(inner)

content = b"""BT /F1 20 Tf 1 0 0 1 72 720 Tm (Account details) Tj ET
BT /F1 12 Tf 1 0 0 1 72 690 Tm (Telephone: 020 7946 0018) Tj ET
q 1 0 0 1 72 600 cm /Fm0 Do Q
"""
packed = zlib.compress(content)

objs = {}
objs[6] = (b"<< /Type /XObject /Subtype /Form /BBox [0 0 300 50] "
           b"/Resources << /Font << /F1 4 0 R >> >> /Filter /FlateDecode /Length %d >>\nstream\n" % len(packed_inner)
           + packed_inner + b"\nendstream")
objs[5] = b"<< /Filter /FlateDecode /Length %d >>\nstream\n" % len(packed) + packed + b"\nendstream"
objs[1] = b"<< /Type /Catalog /Pages 2 0 R >>"
objs[2] = b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>"
objs[3] = (b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Contents 5 0 R "
           b"/Resources << /Font << /F1 4 0 R >> /XObject << /Fm0 6 0 R >> >> >>")
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
