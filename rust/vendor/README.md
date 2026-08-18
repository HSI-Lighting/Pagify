# Vendored `pdfium-render`

A copy of `pdfium-render` 0.9.3 with **one change**, pulled in through
`[patch.crates-io]` in `rust/pdf_core/Cargo.toml`.

## The change

`PdfDocument::handle()` is `pub` instead of `pub(crate)`.
See `src/pdf/document.rs`; the doc comment there explains why.

## Why it was necessary

The roadmap's §4.1 commits to **incremental save**, because a digital signature
covers a byte range of the file and a full rewrite relocates every object in it.
That is not a preference — get it wrong and no signature can ever survive an
edit, and the decision is unrecoverable later.

`save_to_writer` in this crate hardcodes its save flags to `0`:

```rust
// TODO: AJRC - 25/5/22 - investigate supporting the FPDF_INCREMENTAL, ...
let flags = 0;
```

Zero is not incremental, so every save through the safe API rewrites the file.
Measured rather than assumed — `cargo run --example save_probe` shows the output
coming back *smaller* than the input with the original bytes gone, while
`incremental_probe` calls `FPDF_SaveWithVersion` with `FPDF_INCREMENTAL` through
the raw bindings and gets an exact prefix plus a second `%%EOF`.

The raw functions are all on the public `PdfiumLibraryBindings` trait. The only
thing missing was the `FPDF_DOCUMENT` handle to pass them, which also gates
`FPDFPage_Delete` and `FPDF_MovePages` — so one line unblocks page deletion,
reordering and incremental save together.

## Updating

Upstream 0.9.3 is the newest release as of this writing. To move to a later one:

1. copy the new version over this directory;
2. re-apply the single `pub(crate) fn handle` → `pub fn handle` change;
3. run `cargo test` in `rust/pdf_core` — `round_trip.rs` covers everything the
   patch unblocks, including the byte-prefix property that distinguishes an
   incremental save from a rewrite.

If upstream ever exposes the handle, or adds a flags-taking save, delete this
directory and the `[patch.crates-io]` entry. Nothing else depends on the fork.
