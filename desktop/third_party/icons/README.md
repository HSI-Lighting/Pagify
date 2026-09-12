# pagify-icons.ttf

**Material Symbols Outlined**, Apache 2.0 (see `LICENSE`), instanced to one
weight and subsetted to the **187 glyphs the ribbon actually draws** — 10.6 MB
down to 29 kB. (One more, ⌘, is a standard Unicode symbol outside Material
Symbols entirely; it reaches the ribbon through egui's own fallback font.)

It is embedded in the binary with `include_bytes!`. An icon set that can go
missing is a toolbar that can come up blank.

## Why a font at all

egui's bundled fonts can draw 236 symbol characters, and the ribbon needs about
190. The overlap is poor in exactly the places a PDF editor cares about: there
is **no magnifier, no tick, no plus and almost no arrows**. The first pass used
what was available, so Search was a detective, Highlight was a shading block,
and two thirds of the toolbar were stand-ins. `every_ribbon_glyph_can_actually_be_drawn`
was written after a first attempt in which **193 of 198 glyphs** rendered as
empty boxes.

## Regenerating

Needed when a button is added whose icon is not already in the subset —
`every_ribbon_glyph_can_actually_be_drawn` fails loudly when that happens.

```bash
# 1. The upstream font and its name → codepoint table.
B=https://raw.githubusercontent.com/google/material-design-icons/master/variablefont
curl -sSL -o /tmp/MaterialSymbols.ttf \
  "$B/MaterialSymbolsOutlined%5BFILL%2CGRAD%2Copsz%2Cwght%5D.ttf"
curl -sSL -o MaterialSymbols.codepoints \
  "$B/MaterialSymbolsOutlined%5BFILL%2CGRAD%2Copsz%2Cwght%5D.codepoints"

# 2. Pick the new icon's *name* from MaterialSymbols.codepoints — never guess a
#    hex value by eye. A guessed codepoint that happens to land inside Material
#    Symbols' private-use range fails silently here and only surfaces as a
#    blank button, or worse, passes if it happens to collide with a real glyph.
awk '$1=="lock_open"' MaterialSymbols.codepoints   # -> e898

# 3. fonttools in a venv.
python3 -m venv /tmp/fontenv && /tmp/fontenv/bin/pip install fonttools

# 4. Pin the axes to one static instance. A variable font carries every
#    intermediate design and the app draws exactly one of them.
/tmp/fontenv/bin/fonttools varLib.instancer -o /tmp/MaterialSymbols-instance.ttf \
  /tmp/MaterialSymbols.ttf wght=400 opsz=24 FILL=0 GRAD=0

# 5. The codepoints to keep are exactly the ones `main.rs` references — pulled
#    from source rather than hand-copied, so the subset can never drift from
#    the ribbon it is for.
python3 -c '
import re
text = open("../../crates/pagify_app/src/main.rs", encoding="utf-8").read()
cps = sorted(set(int(m, 16) for m in re.findall(r"\\\\u\{([0-9A-Fa-f]+)\}", text)))
print(",".join(f"U+{c:04X}" for c in cps))
' > /tmp/ribbon_codepoints.txt

/tmp/fontenv/bin/fonttools subset /tmp/MaterialSymbols-instance.ttf \
  --unicodes="$(cat /tmp/ribbon_codepoints.txt)" \
  --output-file=pagify-icons.ttf \
  --no-hinting --desubroutinize --name-IDs='' \
  --notdef-glyph --notdef-outline
```

`MaterialSymbols.codepoints` is kept because it is the only thing that maps an
icon *name* to the codepoint the font stores it at — without it, choosing a new
icon means guessing, which is exactly how `lock` (`e899`, already in the
subset) ended up sitting one row above a button that guessed `e897` for
"Lock Area" and got a codepoint outside the whole table: not a real Material
Symbol, not caught by anything until `every_ribbon_glyph_can_actually_be_drawn`
failed. Always look the name up.

Subsetting by codepoints scraped from `main.rs`, rather than from a
hand-maintained list of names, means the two structures cannot drift apart:
the font can never be missing something the ribbon draws, and can never be
carrying a glyph nothing draws any more.

The full 10.6 MB source font is **not** committed. It is a download away and
nothing builds from it.
