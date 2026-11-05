//! Thumbnails drawn by hayro — the pure-Rust renderer PdfCraft draws with —
//! instead of PDFium.
//!
//! PDFium draws on one thread at a time, and every use of it waits behind the
//! same lock; a thumbnail of a page costs what the page does, because the cost is
//! in what the page draws, not how big. On a CAD set of 720,000-line pages the
//! rail's thumbnails held PDFium for seconds while the page in view waited
//! (reported from use, 2026-11-05). hayro can run on as many threads as there
//! are, from one parse of the file shared between them, so the rail fills beside
//! PDFium instead of in front of it. The owner's choice: thumbnails only — the
//! pages themselves, editing, printing and export stay PDFium's.
//!
//! A [`Preview`] is one state of a document, read from [`crate::Session::current_bytes`].
//! Where hayro cannot read the file, or draw a page, it says so (`None`) and the
//! caller asks PDFium instead.

use std::panic::{catch_unwind, AssertUnwindSafe};

use hayro::hayro_interpret::InterpreterSettings;
use hayro::hayro_syntax::Pdf;
use hayro::vello_cpu::color::palette::css::WHITE;
use hayro::{PixmapSettings, RenderCache, RenderSettings};

use crate::session::PageRaster;

/// One state of a document, parsed by hayro once for every page and thread.
pub struct Preview {
    pdf: Pdf,
}

impl Preview {
    /// The document in `bytes` — `None` when hayro cannot read it (or panics
    /// trying: a malformed file must cost a thumbnail, never the program).
    pub fn new(bytes: Vec<u8>) -> Option<Preview> {
        let pdf = catch_unwind(AssertUnwindSafe(|| Pdf::new(bytes))).ok()?.ok()?;
        Some(Preview { pdf })
    }

    pub fn page_count(&self) -> usize {
        self.pdf.pages().len()
    }

    /// Page `page` drawn `width` pixels wide on white, as RGBA, turned as the
    /// page turns itself (`/Rotate`), the way PDFium's thumbnail drew it; `None`
    /// where hayro cannot draw it.
    pub fn render(&self, page: usize, width: u32) -> Option<PageRaster> {
        let drawn = catch_unwind(AssertUnwindSafe(|| {
            let page = self.pdf.pages().get(page)?;
            let (w, h) = page.render_dimensions();
            if !(w > 0.0 && h > 0.0) || width == 0 {
                return None;
            }
            let scale = width as f32 / w;
            // hayro's pixmap is at most 65,535 pixels a side.
            if !(h * scale >= 1.0 && h * scale <= u16::MAX as f32 && width <= u16::MAX as u32) {
                return None;
            }
            let pixmap = hayro::render(
                page,
                &RenderCache::new(),
                &InterpreterSettings::default(),
                &RenderSettings::default(),
                &PixmapSettings { x_scale: scale, y_scale: scale, bg_color: WHITE },
            );
            Some(PageRaster {
                width: pixmap.width() as u32,
                height: pixmap.height() as u32,
                // Premultiplied; on a white background every pixel is opaque, so
                // it is the same as straight RGBA.
                pixels: pixmap.data_as_u8_slice().to_vec(),
                from_cache: false,
            })
        }));
        drawn.ok().flatten()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A page drawn by hayro, at thumbnail size, looks like the page PDFium
    /// draws: the average difference per channel is small on fixtures with
    /// words, pictures, forms, annotations and turned pages. A guard against a
    /// renderer that draws something else, not a claim the two agree to the
    /// pixel — their antialiasing differs.
    #[test]
    fn a_thumbnail_drawn_by_hayro_looks_like_the_one_pdfium_draws() {
        if std::env::var("PAGIFY_PDFIUM_LIB").is_err() {
            eprintln!("skipped: PAGIFY_PDFIUM_LIB is not set");
            return;
        }
        let fixtures = format!("{}/../../../rust/pdf_core/fixtures", env!("CARGO_MANIFEST_DIR"));
        for name in ["text-lines.pdf", "pictures.pdf", "forms.pdf", "hidden-things.pdf", "pages-ladder.pdf", "two-column.pdf"] {
            let session = crate::Session::open(format!("{fixtures}/{name}")).expect("open");
            let preview = Preview::new(session.current_bytes().expect("bytes")).expect("hayro reads it");
            assert_eq!(preview.page_count(), session.page_count().expect("pages"), "{name}");
            for page in 0..preview.page_count() {
                let ours = preview.render(page, 160).unwrap_or_else(|| panic!("{name} page {page}: hayro drew nothing"));
                let width_pt = session.page_size(page).expect("size").width_pt;
                let theirs = session.render_page(page, 160.0 / width_pt).expect("PDFium");
                assert!(
                    ours.width.abs_diff(theirs.width) <= 1 && ours.height.abs_diff(theirs.height) <= 1,
                    "{name} page {page}: {}x{} against PDFium's {}x{}",
                    ours.width, ours.height, theirs.width, theirs.height
                );
                let (w, h) = (ours.width.min(theirs.width) as usize, ours.height.min(theirs.height) as usize);
                let mut total = 0u64;
                for y in 0..h {
                    for x in 0..w {
                        for c in 0..3 {
                            let a = ours.pixels[(y * ours.width as usize + x) * 4 + c] as i64;
                            let b = theirs.pixels[(y * theirs.width as usize + x) * 4 + c] as i64;
                            total += (a - b).unsigned_abs();
                        }
                    }
                }
                let mean = total as f64 / (w * h * 3) as f64;
                assert!(mean < 6.0, "{name} page {page}: hayro and PDFium differ by {mean:.1} a channel on average");
                // A mostly white page drawn blank would pass the average alone:
                // the ink has to be there too, and about as much of it.
                let ink = |r: &PageRaster| r.pixels.chunks_exact(4).filter(|p| p[..3].iter().any(|&c| c < 160)).count();
                let (ours_ink, theirs_ink) = (ink(&ours), ink(&theirs));
                if theirs_ink > 20 {
                    let ratio = ours_ink as f64 / theirs_ink as f64;
                    assert!(
                        (0.5..=2.0).contains(&ratio),
                        "{name} page {page}: hayro inked {ours_ink} pixels where PDFium inked {theirs_ink}"
                    );
                }
            }
        }
    }

    /// What hayro cannot read is `None`, not a panic.
    #[test]
    fn a_file_hayro_cannot_read_is_none() {
        assert!(Preview::new(b"not a pdf at all".to_vec()).is_none());
        assert!(Preview::new(Vec::new()).is_none());
    }
}
