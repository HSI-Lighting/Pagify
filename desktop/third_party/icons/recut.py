"""Re-cut pagify-icons.ttf from the upstream Material Symbols font.

    python third_party/icons/recut.py <MaterialSymbols.ttf>      (run from desktop/)

Keeps exactly the codepoints `crates/pagify_app/src/*.rs` writes as `\\u{XXXX}`
and that MaterialSymbols.codepoints lists, so the subset follows the source and
a symbol from outside the set (⌘, ✓…) is left to egui's own fonts. Prints the
new file's sha256 for third_party/CHECKSUMS.sha256.
"""
import glob
import hashlib
import re
import sys

from fontTools import subset
from fontTools.ttLib import TTFont
from fontTools.varLib import instancer

ICONS = "third_party/icons"
OUT = f"{ICONS}/pagify-icons.ttf"

known = set()
with open(f"{ICONS}/MaterialSymbols.codepoints", encoding="utf-8") as f:
    for line in f:
        parts = line.split()
        if len(parts) == 2:
            known.add(int(parts[1], 16))

used = set()
for path in glob.glob("crates/pagify_app/src/*.rs"):
    with open(path, encoding="utf-8") as f:
        used.update(int(m, 16) for m in re.findall(r"\\u\{([0-9A-Fa-f]+)\}", f.read()))
keep = sorted(used & known)

# The source's own timestamp, so the same source gives the same bytes.
font = TTFont(sys.argv[1], recalcTimestamp=False)
font = instancer.instantiateVariableFont(font, {"wght": 400, "opsz": 24, "FILL": 0, "GRAD": 0})

options = subset.Options()
options.hinting = False
options.desubroutinize = True
options.name_IDs = []
options.notdef_glyph = True
options.notdef_outline = True
subsetter = subset.Subsetter(options)
subsetter.populate(unicodes=keep)
subsetter.subset(font)
font.save(OUT)

with open(OUT, "rb") as f:
    print(f"{len(keep)} glyphs  {hashlib.sha256(f.read()).hexdigest()}  icons/pagify-icons.ttf")
