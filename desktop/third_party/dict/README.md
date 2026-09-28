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
