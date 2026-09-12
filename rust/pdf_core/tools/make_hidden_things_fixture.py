"""A page carrying everything `hiddendata` looks for.

Document information, an XMP packet, an attachment, JavaScript in every
place it can hang — the name tree, the opening action, a page's own
actions, a link's action — an object nothing reaches, and a second revision.
The audit's probe: `hiddendata clean` said it had removed the attachment and
the script while both were still in the file.
"""
import zlib, sys

content = b"BT /F1 20 Tf 1 0 0 1 72 720 Tm (A page with things behind it) Tj ET\n"
packed = zlib.compress(content)
xmp = (b'<?xpacket begin="" id="W5M0MpCehiHzreSzNTczkc9d"?><x:xmpmeta xmlns:x="adobe:ns:meta/">'
       b'<rdf:RDF xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#"><rdf:Description '
       b'xmlns:dc="http://purl.org/dc/elements/1.1/"><dc:creator>Hidden Author</dc:creator>'
       b'</rdf:Description></rdf:RDF></x:xmpmeta><?xpacket end="w"?>')
payload = b"ATTACHED-PAYLOAD: the text of the attachment\n"

objs = {}
objs[1] = (b"<< /Type /Catalog /Pages 2 0 R /Metadata 8 0 R /OpenAction 9 0 R "
           b"/Names << /EmbeddedFiles 10 0 R /JavaScript 13 0 R >> >>")
objs[2] = b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>"
objs[3] = (b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Contents 5 0 R "
           b"/Resources << /Font << /F1 4 0 R >> >> /AA << /O 15 0 R >> /Annots [16 0 R] >>")
objs[4] = b"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>"
objs[5] = b"<< /Filter /FlateDecode /Length %d >>\nstream\n" % len(packed) + packed + b"\nendstream"
objs[6] = b"<< /Author (Hidden Author) /Producer (A tool) /CreationDate (D:20260912120000Z) >>"
objs[7] = b"<< /Type /XObject /Subtype /Form /BBox [0 0 10 10] /Length 0 >>\nstream\n\nendstream"  # unreachable
objs[8] = b"<< /Type /Metadata /Subtype /XML /Length %d >>\nstream\n" % len(xmp) + xmp + b"\nendstream"
objs[9] = b"<< /S /JavaScript /JS (app.alert\\(\"opened\"\\);) >>"
objs[10] = b"<< /Names [(secret.txt) 11 0 R] >>"
objs[11] = b"<< /Type /Filespec /F (secret.txt) /UF (secret.txt) /EF << /F 12 0 R >> >>"
objs[12] = b"<< /Type /EmbeddedFile /Length %d >>\nstream\n" % len(payload) + payload + b"\nendstream"
objs[13] = b"<< /Names [(init) 14 0 R] >>"
objs[14] = b"<< /S /JavaScript /JS (app.alert\\(\"named\"\\);) >>"
objs[15] = b"<< /S /JavaScript /JS (app.alert\\(\"page\"\\);) >>"
objs[16] = (b"<< /Type /Annot /Subtype /Link /Rect [72 600 300 640] /Border [0 0 0] "
            b"/A << /S /JavaScript /JS (app.alert\\(\"link\"\\);) >> >>")

out = bytearray(b"%PDF-1.7\n%\xe2\xe3\xcf\xd3\n")
offsets = {}
for num in sorted(objs):
    offsets[num] = len(out)
    out += b"%d 0 obj\n" % num + objs[num] + b"\nendobj\n"
first_xref = len(out)
top = max(objs) + 1
out += b"xref\n0 %d\n" % top + b"0000000000 65535 f \n"
for num in range(1, top):
    out += (b"%010d 00000 n \n" % offsets[num]) if num in offsets else b"0000000000 65535 f \n"
out += b"trailer\n<< /Size %d /Root 1 0 R /Info 6 0 R >>\nstartxref\n%d\n%%%%EOF\n" % (top, first_xref)

# A second revision: the information dictionary written again, so the file
# holds an earlier version of it in front of the new one.
info_at = len(out)
out += b"6 0 obj\n<< /Author (Hidden Author) /Producer (A tool, again) >>\nendobj\n"
second_xref = len(out)
out += b"xref\n0 1\n0000000000 65535 f \n6 1\n%010d 00000 n \n" % info_at
out += (b"trailer\n<< /Size %d /Root 1 0 R /Info 6 0 R /Prev %d >>\nstartxref\n%d\n%%%%EOF\n"
        % (top, first_xref, second_xref))
open(sys.argv[1], "wb").write(bytes(out))
print(f"wrote {sys.argv[1]}")
