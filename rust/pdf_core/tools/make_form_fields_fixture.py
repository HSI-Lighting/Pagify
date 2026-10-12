"""A one-page AcroForm: text, checkbox, radio group and combo fields.

The widgets carry values but no appearance streams — this fixture is about
what the fields *hold* (names, kinds, values, checked states), which is what
the engine reads; nothing here draws a field, so the page looks blank on
purpose. Rebuild with:

    python3 tools/make_form_fields_fixture.py fixtures/form-fields.pdf
"""
import sys

objs = {}

objs[1] = b"<< /Type /Catalog /Pages 2 0 R /AcroForm 9 0 R >>"
objs[2] = b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>"
objs[3] = (b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Contents 5 0 R "
           b"/Annots [11 0 R 12 0 R 13 0 R 14 0 R 15 0 R 16 0 R] "
           b"/Resources << /Font << /F1 4 0 R >> >> >>")
objs[4] = b"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>"

content = b"""BT /F1 14 Tf 1 0 0 1 72 720 Tm (Pagify form fixture) Tj ET
BT /F1 11 Tf 1 0 0 1 72 668 Tm (Name:) Tj ET
BT /F1 11 Tf 1 0 0 1 72 628 Tm (Email:) Tj ET
BT /F1 11 Tf 1 0 0 1 72 588 Tm (Agree:) Tj ET
BT /F1 11 Tf 1 0 0 1 72 548 Tm (Speed:) Tj ET
BT /F1 11 Tf 1 0 0 1 72 508 Tm (Colour:) Tj ET
"""
objs[5] = b"<< /Length %d >>\nstream\n" % len(content) + content + b"endstream"

objs[9] = (b"<< /Type /AcroForm /Fields [11 0 R 12 0 R 13 0 R 14 0 R 15 0 R 16 0 R] "
           b"/DR << /Font << /Helv 10 0 R >> >> /DA (/Helv 0 Tf 0 g) >>")
objs[10] = b"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>"

# The widgets, in page order — the order the engine's reader walks them.
#
# The check and radio widgets carry minimal appearance streams: PDFium's
# field state machine reads a widget's checked state through its /AP /N
# dictionary, and a widget without one reads every state as `Off` no matter
# what /V says. `Off` is empty, each on-state draws a small X.
objs[11] = (b"<< /Type /Annot /Subtype /Widget /FT /Tx /T (name) /V (Ada Lovelace) "
            b"/Rect [140 664 380 688] /F 4 /P 3 0 R /DA (/Helv 11 Tf 0 g) >>")
objs[12] = (b"<< /Type /Annot /Subtype /Widget /FT /Tx /T (email) "
            b"/Rect [140 624 380 648] /F 4 /P 3 0 R /DA (/Helv 11 Tf 0 g) >>")
objs[13] = (b"<< /Type /Annot /Subtype /Widget /FT /Btn /T (agree) /V /Yes /AS /Yes "
            b"/Rect [140 584 154 598] /F 4 /P 3 0 R "
            b"/AP << /N << /Yes 18 0 R /Off 17 0 R >> >> >>")
# A radio group is two widgets sharing a name; the checked one carries the value.
objs[14] = (b"<< /Type /Annot /Subtype /Widget /FT /Btn /Ff 32768 /T (speed) "
            b"/V /slow /AS /slow /Rect [140 544 154 558] /F 4 /P 3 0 R "
            b"/AP << /N << /slow 18 0 R /Off 17 0 R >> >> >>")
objs[15] = (b"<< /Type /Annot /Subtype /Widget /FT /Btn /Ff 32768 /T (speed) "
            b"/AS /Off /Rect [180 544 194 558] /F 4 /P 3 0 R "
            b"/AP << /N << /fast 18 0 R /Off 17 0 R >> >> >>")
# /Ff 131072 is the combo flag.
objs[16] = (b"<< /Type /Annot /Subtype /Widget /FT /Ch /Ff 131072 /T (colour) "
            b"/Opt [(Red) (Green) (Blue)] /V (Green) /Rect [140 504 260 528] /F 4 /P 3 0 R >>")

objs[17] = b"<< /Type /XObject /Subtype /Form /BBox [0 0 14 14] /Length 0 >>\nstream\n\nendstream"
mark = b"0 0 0 RG 2 w 2 2 m 12 12 l S 2 12 m 12 2 l S\n"
objs[18] = (b"<< /Type /XObject /Subtype /Form /BBox [0 0 14 14] /Length %d >>\nstream\n"
            % len(mark) + mark + b"endstream")

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
