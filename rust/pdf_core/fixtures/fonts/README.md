# Fonts for text replacement

`centurygothic.ttf` and `centurygothic_bold.ttf` live here but are **not
committed**. See `.gitignore` in this directory.

## Why they are here

The HSI 2017 catalogue is set in Century Gothic, and 92 of its 95 pages carry no
text at all — the words are vector outlines, left behind when the artwork was
exported from Illustrator with type converted to paths. Editing one of those
words means deleting its paths and drawing replacement text, and the replacement
only matches its surroundings if it is set in the same face. `CenturyGothic-Bold`
is the one the catalogue's own (12-glyph, subset) font names, so the bold weight
is the one that matters.

## Why they are not committed

    Typeface (c) The Monotype Corporation plc.

A licensed commercial typeface, and embedding one into a PDF that is then
distributed is a licensing question separate from having the file. Committing the
binaries into a shared repository is a third thing again. Whoever runs this needs
their own licence; the path is what is shared here, not the font.

## Where to put them

Drop both `.ttf` files in this directory. The engine looks for them by name.
