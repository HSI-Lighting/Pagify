# en-US.txt

A plain English word list, one word per line, lowercase — the dictionary
`crates/pagify_app`'s spelling checker looks a word up in and suggests
close matches from. Not affix-based like a real Hunspell dictionary (no
`en_US.aff`, no stemming): a flat list is a `HashSet` lookup away from
"is this spelled right," which is all the feature asks for, and needs no
new dependency to read.

Source: [`dwyl/english-words`](https://github.com/dwyl/english-words),
`words_alpha.txt` (alphabetic entries only — the repository's own
`words.txt` also includes a handful of entries with digits and
punctuation, which a word list meant to be split on non-letters has no
use for). 370,105 words.

**Public domain** (The Unlicense — see `LICENSE.txt`), which permits
bundling and redistribution without attribution.

## Why a flat word list rather than `hunspell-rs`/`symspell`

Looked up against the ladder before adding either:

- `hunspell-rs` links a C library — a third native dependency alongside
  `pdfium-render` and `rten`, for a feature that only needs "is this word
  known" and "what looks close to it."
- `symspell`'s whole pitch is fast fuzzy lookup at large scale (autocomplete
  over millions of queries). A document's own misspelled words number in
  the tens, checked once per click — a `HashSet` contains-check plus a
  length-bucketed Levenshtein scan (`spelling.rs`) is smaller code, no new
  dependency, and fast enough that it was never worth measuring separately
  from the rest of a spelling pass.

## How it was fetched

```bash
curl -sL -o en-US.txt \
  https://raw.githubusercontent.com/dwyl/english-words/master/words_alpha.txt
curl -sL -o LICENSE.txt \
  https://raw.githubusercontent.com/dwyl/english-words/master/LICENSE.md
```

---

# ar/ — Arabic (Hunspell)

`ar/ar.dic` and `ar/ar.aff`: the Arabic Hunspell dictionary from the
Ayaspell project, as packaged in LibreOffice's `dictionaries` repository
(<https://github.com/LibreOffice/dictionaries/tree/master/ar>), by Mohamed
Kebdani (2006–2008; see `ar/AUTHORS.txt`). 465,928 entries with affix rules,
read by the `spellbook` crate (MPL-2.0, pure Rust).

**Licence.** Ayaspell publishes it under the GPL family (its page says
"GPL/LGPL/MPL"); the repository this was fetched from carries no licence file
of its own for it. It is a data file read at run time, not linked code, and it
is fine for HSI's own use. **Check the exact terms before Pagify is
distributed outside HSI.**

```bash
base=https://raw.githubusercontent.com/LibreOffice/dictionaries/master/ar
curl -sL -o ar/ar.dic $base/ar.dic
curl -sL -o ar/ar.aff $base/ar.aff
curl -sL -o ar/AUTHORS.txt $base/AUTHORS.txt
```

# han-common.txt — Chinese characters in everyday use

One hex code point or `FIRST-LAST` range per line: the union of the Unicode
Unihan fields `kTGH` (the 8,105 characters of the Table of General Standard
Chinese Characters), `kGB0` (GB 2312), `kBigFive` (Big5, traditional) and
`kJis0` (JIS X 0208, so Japanese kanji are not flagged either). 16,928
characters. From `Unihan_OtherMappings.txt` in
<https://www.unicode.org/Public/UCD/latest/ucd/Unihan.zip>, under the Unicode
licence (free to use and redistribute with the notice).

**What it can and cannot do.** A list of characters finds one that is
*unusual* — what OCR and bad font encodings produce. Chinese has no spaces
between words and any real character is a real character, so a wrong but
valid character cannot be found by a word list. That needs a language model.

**This repository is public**, so committing `ar/ar.dic` and `ar/ar.aff` here
redistributes them. They are kept with their `AUTHORS.txt` and source link, and
the licence question above is still open: if HSI wants the repository to carry
no third-party dictionary, remove `ar/` and the `include_str!` lines for it in
`crates/pagify_app/src/spelling.rs` (Arabic then goes back to "not judged"), or
fetch it at build time with the commands above instead of committing it.
