"""A page carrying an attachment named as Pagify's lock that is not one.

The lock is read while drawing. An attachment with the lock's name whose
contents do not parse used to be pulled out of the file and re-parsed every
frame, because the failure was never cached (the security audit's M5). The
attachment here is large enough for that to have shown, and wrong from its
first byte.
"""
import zlib, sys

content = b"BT /F1 20 Tf 1 0 0 1 72 720 Tm (A page with a lock that is not one) Tj ET\n"
packed = zlib.compress(content)
payload = b"this is not a lock " * (512 * 1024)   # ~10 MB, wrong from the first byte

objs = {}
objs[1] = b"<< /Type /Catalog /Pages 2 0 R /Names << /EmbeddedFiles 6 0 R >> >>"
objs[2] = b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>"
objs[3] = (b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Contents 5 0 R "
           b"/Resources << /Font << /F1 4 0 R >> >> >>")
objs[4] = b"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>"
objs[5] = b"<< /Filter /FlateDecode /Length %d >>\nstream\n" % len(packed) + packed + b"\nendstream"
objs[6] = b"<< /Names [(pagify-lock.json) 7 0 R] >>"
objs[7] = b"<< /Type /Filespec /F (pagify-lock.json) /UF (pagify-lock.json) /EF << /F 8 0 R >> >>"
packed_payload = zlib.compress(payload)
objs[8] = (b"<< /Type /EmbeddedFile /Filter /FlateDecode /Length %d /Params << /Size %d >> >>\nstream\n"
           % (len(packed_payload), len(payload)) + packed_payload + b"\nendstream")

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
