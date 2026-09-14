"""Pages whose sensitive text is drawn through a Form XObject.

The security audit's probe: a card number that text extraction finds and the
page-level redaction pass could not reach, because the text object lives
inside the form. Three shapes of it, chosen by the second argument:

  once    the form drawn once, on one page — the page's own; cut in place
  twice   the same form drawn twice on the page — one stream, two drawings,
          which redaction refuses rather than clearing both
  shared  two pages drawing the same form — cut from a private copy on the
          page asked about, left as it is on the other
  nested  the form drawn through another form: the cut follows the chain
          down and edits the inner form in place, both being the page's own
  nested-inner-shared
          two pages, each with an outer form of its own, both drawing the
          same inner form: the inner is copied for the page asked about and
          that page's outer, its own, is pointed at the copy in place
  nested-outer-shared
          two pages drawing the same outer form, which draws the inner: the
          chain is copied from the outer down, and the other page keeps both
  predicted
          the form's stream deflated with a PNG predictor (15, each row its
          own filter type, sixteen columns) — undone by the byte-level
          reader, so the words are cut like any other
  lzw     the form's stream LZW-encoded, decoded by the byte-level reader
          like any other
  chained the form's stream deflated and then hex-encoded, a filter chain
          the byte-level reader does not follow: reported as nested content,
          never painted over — the honest refusal that remains
  kerned  the form's line drawn as two operators sharing one text object —
          `(HSI) Tj` then `[( Lighting)] TJ`, how a design program kerns —
          so cutting the first must leave the second where it was
"""
import zlib, sys

mode = sys.argv[2] if len(sys.argv) > 2 else "once"

# The card number is the classic test number, which passes Luhn.
inner = b"""BT /F1 14 Tf 1 0 0 1 10 20 Tm (Card on file: 4111 1111 1111 1111) Tj ET
"""
if mode == "kerned":
    inner = b"""BT /F1 14 Tf 1 0 0 1 10 20 Tm (HSI) Tj [( Lighting) -250 (Catalogue)] TJ ET
"""
packed_inner = zlib.compress(inner)
inner_extra = b""
inner_filter = b"/FlateDecode"
if mode == "predicted":
    # PNG predictors, sixteen columns of one byte, each row filtered with its
    # own type (None, Sub, Up, Average, Paeth in turn) — predictor 15.
    columns = 16
    padded = inner + b" " * (-len(inner) % columns)
    rows = [padded[i:i + columns] for i in range(0, len(padded), columns)]
    def paeth(a, b, c):
        p = a + b - c
        pa, pb, pc = abs(p - a), abs(p - b), abs(p - c)
        return a if pa <= pb and pa <= pc else (b if pb <= pc else c)
    stored = bytearray()
    previous = bytes(columns)
    for r, row in enumerate(rows):
        t = r % 5
        stored.append(t)
        for i in range(columns):
            left = row[i - 1] if i >= 1 else 0
            up = previous[i]
            ul = previous[i - 1] if i >= 1 else 0
            predicted = {0: 0, 1: left, 2: up, 3: (left + up) // 2, 4: paeth(left, up, ul)}[t]
            stored.append((row[i] - predicted) % 256)
        previous = row
    packed_inner = zlib.compress(bytes(stored))
    inner_extra = b" /DecodeParms << /Predictor 15 /Columns %d >>" % columns
elif mode == "lzw":
    # PDF LZW: 9- to 12-bit codes, clear 256, end 257, early change.
    def lzw(data):
        table = {bytes([i]): i for i in range(256)}
        next_code = 258
        width = 9
        out_bits = []
        def emit(code):
            out_bits.extend(((code >> (width - 1 - b)) & 1) for b in range(width))
        emit(256)
        w = b""
        for byte in data:
            wc = w + bytes([byte])
            if wc in table:
                w = wc
            else:
                emit(table[w])
                table[wc] = next_code
                next_code += 1
                if next_code + 1 >= (1 << width) and width < 12:
                    width += 1
                if next_code >= 4094:
                    emit(256)
                    table = {bytes([i]): i for i in range(256)}
                    next_code = 258
                    width = 9
                w = bytes([byte])
        if w:
            emit(table[w])
        emit(257)
        while len(out_bits) % 8:
            out_bits.append(0)
        return bytes(int("".join(map(str, out_bits[i:i + 8])), 2) for i in range(0, len(out_bits), 8))
    packed_inner = lzw(inner)
    inner_filter = b"/LZWDecode"
elif mode == "chained":
    packed_inner = zlib.compress(inner).hex().encode() + b">"
    inner_filter = b"[/ASCIIHexDecode /FlateDecode]"

def page_content(second_draw):
    content = (b"BT /F1 20 Tf 1 0 0 1 72 720 Tm (Account details) Tj ET\n"
               b"BT /F1 12 Tf 1 0 0 1 72 690 Tm (Telephone: 020 7946 0018) Tj ET\n"
               b"q 1 0 0 1 72 600 cm /Fm0 Do Q\n")
    if second_draw:
        content += b"q 1 0 0 1 72 400 cm /Fm0 Do Q\n"
    return zlib.compress(content)

objs = {}
objs[6] = (b"<< /Type /XObject /Subtype /Form /BBox [0 0 300 50] "
           b"/Resources << /Font << /F1 4 0 R >> >> /Filter " + inner_filter + inner_extra + b" /Length %d >>\nstream\n" % len(packed_inner)
           + packed_inner + b"\nendstream")
objs[4] = b"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>"
objs[1] = b"<< /Type /Catalog /Pages 2 0 R >>"
def outer_form(number, inner_number):
    # An outer form that does nothing but draw the inner one.
    outer = b"q 1 0 0 1 0 0 cm /Fm1 Do Q\n"
    packed_outer = zlib.compress(outer)
    objs[number] = (b"<< /Type /XObject /Subtype /Form /BBox [0 0 300 50] "
                    b"/Resources << /XObject << /Fm1 %d 0 R >> >> /Filter /FlateDecode /Length %d >>\nstream\n"
                    % (inner_number, len(packed_outer)) + packed_outer + b"\nendstream")

if mode.startswith("nested"):
    outer_form(9, 6)
    resources = b"/Resources << /Font << /F1 4 0 R >> /XObject << /Fm0 9 0 R >> >>"
    resources2 = resources
    if mode == "nested-inner-shared":
        outer_form(10, 6)
        resources2 = b"/Resources << /Font << /F1 4 0 R >> /XObject << /Fm0 10 0 R >> >>"
else:
    resources = b"/Resources << /Font << /F1 4 0 R >> /XObject << /Fm0 6 0 R >> >>"
    resources2 = resources
packed = page_content(mode == "twice")
objs[5] = b"<< /Filter /FlateDecode /Length %d >>\nstream\n" % len(packed) + packed + b"\nendstream"
objs[3] = (b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Contents 5 0 R " + resources + b" >>")
if mode in ("shared", "nested-inner-shared", "nested-outer-shared"):
    packed2 = page_content(False)
    objs[8] = b"<< /Filter /FlateDecode /Length %d >>\nstream\n" % len(packed2) + packed2 + b"\nendstream"
    objs[7] = (b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Contents 8 0 R " + resources2 + b" >>")
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
