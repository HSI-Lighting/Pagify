"""A one-page tagged PDF: a heading and a paragraph under a Document tag.

Minimal but canonical: the page's content carries marked-content sequences
(`/P <</MCID n>> BDC … EMC`), the page declares `/StructParents 0`, the
structure tree hangs off the catalog (`/StructTreeRoot`), and a
`/ParentTree` number tree maps the MCIDs back to their elements — which is
what PDFium's `FPDF_StructTree_GetForPage` walks. Rebuild with:

    python3 tools/make_tagged_fixture.py fixtures/tagged.pdf
"""
import sys

objs = {}

objs[1] = (b"<< /Type /Catalog /Pages 2 0 R /StructTreeRoot 20 0 R "
           b"/Outlines 30 0 R /MarkInfo << /Marked true >> /Lang (en-US) >>")
objs[2] = b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>"
objs[3] = (b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Contents 5 0 R "
           b"/StructParents 0 /Resources << /Font << /F1 4 0 R >> >> >>")
objs[4] = b"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>"

content = b"""q
/P <</MCID 0>> BDC
BT /F1 18 Tf 1 0 0 1 72 700 Tm (A Tagged Heading) Tj ET
EMC
/P <</MCID 1>> BDC
BT /F1 11 Tf 1 0 0 1 72 670 Tm (A tagged paragraph with some words.) Tj ET
EMC
/Figure <</MCID 2>> BDC
0.2 0.3 0.8 rg 72 600 100 40 re f
EMC
/Figure <</MCID 3>> BDC
0.8 0.3 0.2 rg 200 600 100 40 re f
EMC
Q
"""
objs[5] = b"<< /Length %d >>\nstream\n" % len(content) + content + b"endstream"

# The structure tree: a Document holding an H1 and a P; /K as the MCID in
# the page's own sequence (the page carries /StructParents 0).
objs[20] = (b"<< /Type /StructTreeRoot /K [21 0 R] /ParentTree 23 0 R "
            b"/ParentTreeNextKey 1 >>")
objs[21] = (b"<< /Type /StructElem /S /Document /P 20 0 R "
            b"/K [22 0 R 24 0 R 25 0 R 26 0 R] >>")
objs[22] = b"<< /Type /StructElem /S /H1 /P 21 0 R /K 0 >>"
objs[24] = (b"<< /Type /StructElem /S /P /P 21 0 R /K 1 "
            b"/Alt (the paragraph's alternate text) >>")
# One figure with alternate text, one without — the pair the alt-text
# reading is tested against.
objs[25] = b"<< /Type /StructElem /S /Figure /P 21 0 R /K 2 >>"
objs[26] = (b"<< /Type /StructElem /S /Figure /P 21 0 R /K 3 "
            b"/Alt (a blue bar, described) >>")
objs[23] = b"<< /Nums [0 [22 0 R 24 0 R 25 0 R 26 0 R]] >>"

# The file's own table of contents, so the Contents panel has a real one
# to list.
objs[30] = b"<< /Type /Outlines /First 31 0 R /Last 32 0 R /Count 2 >>"
objs[31] = (b"<< /Title (A Tagged Heading) /Parent 30 0 R /Next 32 0 R "
            b"/Dest [3 0 R /XYZ null null null] >>")
objs[32] = (b"<< /Title (The paragraph) /Parent 30 0 R "
            b"/Dest [3 0 R /XYZ null null null] >>")

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
    if num in offsets:
        out += b"%010d 00000 n \n" % offsets[num]
    else:
        out += b"0000000000 65535 f \n"
out += b"trailer\n<< /Size %d /Root 1 0 R >>\nstartxref\n%d\n%%%%EOF\n" % (top, start)
open(sys.argv[1], "wb").write(bytes(out))
