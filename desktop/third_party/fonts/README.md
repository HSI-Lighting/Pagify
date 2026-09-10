# Montserrat-{Regular,Bold}.ttf

Candidate faces for matching type converted to outlines — see
`crates/pagify_app`'s use of `pdf_core::document::glyphs::Catalogue` and the
engine's own `document/outlined.rs`. Bundled because it is the specific face
HSI's own catalogue was set in, named directly rather than guessed at from a
generic system-font list.

**SIL Open Font License 1.1** (see `LICENSE.txt`), which permits bundling and
redistribution — it may not be sold on its own, which embedding it in this
application does not do.

## Why static weights, not the one variable file Google Fonts ships

Google Fonts distributes Montserrat as a single variable font (`wght` axis,
100–900). Measured rather than assumed: `ttf-parser`'s default instance for
that file — the one it returns with no variation coordinates set — is weight
**100 (Thin)**, not the 400 most variable fonts default to. Matching against
Thin outlines when the document was actually set in Regular or Bold means
comparing the wrong stroke width, which changes a contour's shape enough to
cost real accuracy in a matcher that compares shapes.

## Why the instancing needs `--update-name-table`

`varLib.instancer` does not rewrite the name table unless asked. Without the
flag both files kept the name records of the Thin they were cut from, and the
bold one did not set its own bold bit:

| | name id 6 | OS/2 weight | `is_bold()` |
|---|---|---|---|
| instanced, no flag | `Montserrat-Thin` | Normal / Bold | false / **false** |
| instanced, flagged | `Montserrat-Regular` / `-Bold` | Normal / Bold | false / **true** |

The outlines were never affected — measured either way, `o` is 535x535 units at
Regular and 591x554 at Bold. What the name reaches is a *produced document*.
When a run of text is edited into words its own font cannot spell, this face is
written into the PDF and its name becomes the `/BaseFont` and `/FontName` — see
`pdf_core::pdf::embed`. A file claiming to hold Montserrat Thin while drawing
Regular misleads every reader that ever has to substitute for it, and it was
also what the editor told the user it had used. The bold bit matters for the
same reason: the descriptor's ForceBold flag is set from it.

## How they are made

```bash
python3 -m venv /tmp/fontenv && /tmp/fontenv/bin/pip install fonttools

B=https://raw.githubusercontent.com/google/fonts/main/ofl/montserrat
curl -sSL -o /tmp/Montserrat-Variable.ttf "$B/Montserrat%5Bwght%5D.ttf"
curl -sSL -o LICENSE.txt "$B/OFL.txt"

/tmp/fontenv/bin/fonttools varLib.instancer -q --update-name-table \
  -o Montserrat-Regular.ttf /tmp/Montserrat-Variable.ttf wght=400
/tmp/fontenv/bin/fonttools varLib.instancer -q --update-name-table \
  -o Montserrat-Bold.ttf /tmp/Montserrat-Variable.ttf wght=700
```

## Why not upstream's own static builds

The obvious alternative, and measured rather than assumed: they recognise the
`outlined-montserrat.pdf` fixture **worse**. Same catalogue, same page, same
build —

| fonts | words recognised |
|---|---|
| instanced from the variable file | **215** |
| upstream `JulietaUla/Montserrat` statics | 153 |

Two faces can draw the same shape from different points, and this matcher
compares shapes. Sixty-two fewer words is enough to take the vector pass below
the bar `recognise_outlined_words_if_trustworthy` sets, at which point the page
falls through to OCR — slower, and not what that path is for. The instanced
files stay.

Not subsetted, unlike the ribbon's icon font. A ribbon draws a fixed, known set
of 172 glyphs; a document can contain anything its author typed, so a matching
candidate needs the face's full glyph set rather than a curated slice of it.

## Measured, on a fixture built from this exact file

`examples/make_text_fixtures.rs` accepts `PAGIFY_OUTLINED_FONT` to build an
`outlined.pdf`-shaped fixture from any face, for exactly this kind of check:

```bash
PAGIFY_OUTLINED_FONT=third_party/fonts/Montserrat-Regular.ttf \
  cargo run --example make_text_fixtures -- /tmp/montserrat_fixtures
```

Recognition against that fixture, with a `Catalogue` built from the same file,
scored **0.685** similarity to the fixture's own known text (Arial, the
committed `outlined.pdf`'s face, scores 0.74 by the same measure). Genuinely
comparable, not merely passable, with a different failure shape than Arial's:
more spurious mid-word spaces, fewer dropped disjoint dots. Montserrat's
letters sit measurably wider apart than Arial's at the same nominal size, and
some of those gaps cross the quarter-em threshold `layout::assemble_line`
already uses to recover word breaks — a font-specific interaction with a fixed
threshold, named here rather than quietly accepted.
