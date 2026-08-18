# Round-trip fixtures

Small, committed PDFs for the write path. Regenerate with:

```
PAGIFY_PDFIUM_LIB=<desktop pdfium> \
  cargo run --example make_fixtures -- rust/pdf_core/fixtures
```

Every page has a **distinct size**. That is deliberate: it makes a reorder, a
deletion or an insertion verifiable from the page tree alone, with no text to
extract and no pixels to compare. A test can assert the exact sequence of widths
and know which page landed where.

| File | Pages | Widths (pt) | Purpose |
|---|---|---|---|
| `pages-ladder.pdf` | 5 | 200, 250, 300, 350, 400 | reorder, delete, insert, rotate |
| `single-page.pdf` | 1 | 200 | boundary: deleting the only page |
| `mixed-sizes.pdf` | 4 | 595, 420, 612, 842 | A4/A5/Letter/A3, non-uniform tree |

No passwords. Nothing here is encrypted.

## Gaps, deliberately named

These cannot be generated and have to be sourced. They are listed so the corpus
does not look complete when it is not:

- **signed** — needed for the §4.1 acceptance that an unrelated edit leaves the
  signed byte range intact. Verify with `pdfsig` (poppler-utils), never
  `qpdf --check`, which cannot inspect a signature at all. Assert byte-range
  integrity rather than trust-chain validity, or the test starts failing on the
  day the certificate expires.
- **encrypted** — commit the password beside it in this file when it lands.
- **scanned** — a page with no text layer. The highlighter has nothing to select
  on one, which is not a bug in the highlighter; see `SCAN_HINT` in a session
  recording.
- **malformed** — a truncated xref, for the error path.
- **CJK / right-to-left** — text extraction ordering.

## Not committed

The 2.9 GB catalogue stays out of the repository. The Phase A acceptance that a
one-page edit saves in under 2 s is therefore a **local** measurement, not a CI
gate, unless a large synthetic fixture is generated for it.
