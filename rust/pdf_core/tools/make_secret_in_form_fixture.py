"""Pages whose sensitive text is drawn through a Form XObject.

The security audit's probe: a card number that text extraction finds and the
page-level redaction pass could not reach, because the text object lives
inside the form. Three shapes of it, chosen by the second argument:

  once    the form drawn once, on one page — the page's own; cut in place
  twice   the same form drawn twice on the page — one stream, two drawings,
          which redaction refuses rather than clearing both
  shared  two pages drawing the same form — cut from a private copy on the
          page asked about, left as it is on the other
  nested  the form drawn through another form — a level further down than
          the cut follows, so it is reported rather than reached
"""
import zlib, sys

mode = sys.argv[2] if len(sys.argv) > 2 else "once"

# The card number is the classic test number, which passes Luhn.
inner = b"""BT /F1 14 Tf 1 0 0 1 10 20 Tm (Card on file: 4111 1111 1111 1111) Tj ET
"""
packed_inner = zlib.compress(inner)

def page_content(second_draw):
    content = (b"BT /F1 20 Tf 1 0 0 1 72 720 Tm (Account details) Tj ET\n"
               b"BT /F1 12 Tf 1 0 0 1 72 690 Tm (Telephone: 020 7946 0018) Tj ET\n"
               b"q 1 0 0 1 72 600 cm /Fm0 Do Q\n")
    if second_draw:
        content += b"q 1 0 0 1 72 400 cm /Fm0 Do Q\n"
    return zlib.compress(content)

objs = {}
objs[6] = (b"<< /Type /XObject /Subtype /Form /BBox [0 0 300 50] "
           b"/Resources << /Font << /F1 4 0 R >> >> /Filter /FlateDecode /Length %d >>\nstream\n" % len(packed_inner)
           + packed_inner + b"\nendstream")
objs[4] = b"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>"
objs[1] = b"<< /Type /Catalog /Pages 2 0 R >>"
if mode == "nested":
    # An outer form that does nothing but draw the inner one.
    outer = b"q 1 0 0 1 0 0 cm /Fm1 Do Q\n"
    packed_outer = zlib.compress(outer)
    objs[9] = (b"<< /Type /XObject /Subtype /Form /BBox [0 0 300 50] "
               b"/Resources << /XObject << /Fm1 6 0 R >> >> /Filter /FlateDecode /Length %d >>\nstream\n" % len(packed_outer)
               + packed_outer + b"\nendstream")
    resources = b"/Resources << /Font << /F1 4 0 R >> /XObject << /Fm0 9 0 R >> >>"
else:
    resources = b"/Resources << /Font << /F1 4 0 R >> /XObject << /Fm0 6 0 R >> >>"
packed = page_content(mode == "twice")
objs[5] = b"<< /Filter /FlateDecode /Length %d >>\nstream\n" % len(packed) + packed + b"\nendstream"
objs[3] = (b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Contents 5 0 R " + resources + b" >>")
if mode == "shared":
    packed2 = page_content(False)
    objs[8] = b"<< /Filter /FlateDecode /Length %d >>\nstream\n" % len(packed2) + packed2 + b"\nendstream"
    objs[7] = (b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Contents 8 0 R " + resources + b" >>")
    objs[2] = b"<< /Type /Pages /Kids [3 0 R 7 0 R] /Count 2 >>"
else:
    objs[2] = b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>"

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
print(f"wrote {sys.argv[1]} ({mode})")
