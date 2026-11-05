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

/// Above this average difference a channel, two pictures of a page differ.
/// Antialiasing alone, the two renderers' edges a pixel apart, came to under 6
/// on every fixture at thumbnail size.
pub const DIFF_MEAN: f64 = 6.0;
/// hayro's inked pixels over PDFium's, inside which the two drew about as much.
/// A pixel is inked when any of its channels is below 160.
pub const INK_RATIO: std::ops::RangeInclusive<f64> = 0.8..=1.25;
/// The share of inked pixels with no ink of the other picture within a pixel,
/// above which a mark is missing or somewhere else, whatever the average says.
pub const INK_MOVED: f64 = 0.05;

/// How far hayro's picture of a page is from PDFium's.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Comparison {
    /// The average difference a channel, 0–255.
    pub mean_diff: f64,
    /// hayro's inked pixels over PDFium's; 1 when both inked fewer than 20.
    pub ink_ratio: f64,
    /// The share of inked pixels, of both pictures, with no ink of the other
    /// picture within a pixel.
    pub ink_moved: f64,
}

impl Comparison {
    pub fn differs(&self) -> bool {
        self.mean_diff > DIFF_MEAN || !INK_RATIO.contains(&self.ink_ratio) || self.ink_moved > INK_MOVED
    }
}

/// hayro's picture of a page against PDFium's, over the area both cover;
/// `None` when the two differ in size by more than a pixel either way.
pub fn compare(hayro: &PageRaster, pdfium: &PageRaster) -> Option<Comparison> {
    if hayro.width.abs_diff(pdfium.width) > 1 || hayro.height.abs_diff(pdfium.height) > 1 {
        return None;
    }
    let (w, h) = (hayro.width.min(pdfium.width) as usize, hayro.height.min(pdfium.height) as usize);
    if w == 0 || h == 0 {
        return None;
    }
    let mut total = 0u64;
    for y in 0..h {
        for x in 0..w {
            let (a, b) = ((y * hayro.width as usize + x) * 4, (y * pdfium.width as usize + x) * 4);
            for c in 0..3 {
                total += hayro.pixels[a + c].abs_diff(pdfium.pixels[b + c]) as u64;
            }
        }
    }
    let inked = |r: &PageRaster| -> Vec<bool> {
        (0..h)
            .flat_map(|y| (0..w).map(move |x| (y, x)))
            .map(|(y, x)| r.pixels[(y * r.width as usize + x) * 4..][..3].iter().any(|&c| c < 160))
            .collect()
    };
    let (a, b) = (inked(hayro), inked(pdfium));
    // Inked pixels of `of` with no ink of `other` in the 3×3 around them.
    let lost = |of: &[bool], other: &[bool]| {
        (0..h * w)
            .filter(|&i| of[i])
            .filter(|&i| {
                let (y, x) = (i / w, i % w);
                !(y.saturating_sub(1)..=(y + 1).min(h - 1))
                    .any(|y| (x.saturating_sub(1)..=(x + 1).min(w - 1)).any(|x| other[y * w + x]))
            })
            .count()
    };
    let (a_ink, b_ink) = (a.iter().filter(|&&i| i).count(), b.iter().filter(|&&i| i).count());
    Some(Comparison {
        mean_diff: total as f64 / (w * h * 3) as f64,
        ink_ratio: if a_ink < 20 && b_ink < 20 { 1.0 } else { a_ink as f64 / b_ink.max(1) as f64 },
        ink_moved: (lost(&a, &b) + lost(&b, &a)) as f64 / (a_ink + b_ink).max(1) as f64,
    })
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
                let c = compare(&ours, &theirs).unwrap_or_else(|| {
                    panic!("{name} page {page}: {}x{} against PDFium's {}x{}", ours.width, ours.height, theirs.width, theirs.height)
                });
                assert!(c.mean_diff < DIFF_MEAN, "{name} page {page}: hayro and PDFium differ by {:.1} a channel on average", c.mean_diff);
                // A mostly white page drawn blank would pass the average alone:
                // the ink has to be there too, and about as much of it — looser
                // than `INK_RATIO`, as a 160-pixel thumbnail has little ink.
                assert!((0.5..=2.0).contains(&c.ink_ratio), "{name} page {page}: hayro inked {:.2} of PDFium's ink", c.ink_ratio);
            }
        }
    }

    /// A white page `w` by `h` with a black 20-pixel square at each of `squares`.
    fn squares(w: u32, h: u32, squares: &[(u32, u32)]) -> PageRaster {
        let mut pixels = vec![255u8; (w * h * 4) as usize];
        for &(left, top) in squares {
            for y in top..top + 20 {
                for x in left..left + 20 {
                    let at = ((y * w + x) * 4) as usize;
                    pixels[at..at + 3].fill(0);
                }
            }
        }
        PageRaster { width: w, height: h, pixels, from_cache: false }
    }

    /// The same picture does not differ, nor one a pixel to the side (two
    /// renderers' antialiasing puts an edge a pixel apart); a page hayro left
    /// blank does, and so does a mark one of them left out.
    #[test]
    fn two_pictures_alike_do_not_differ_and_a_missing_mark_does() {
        let a = squares(100, 100, &[(10, 10)]);
        let same = compare(&a, &a).expect("same size");
        assert_eq!(same, Comparison { mean_diff: 0.0, ink_ratio: 1.0, ink_moved: 0.0 });
        assert!(!same.differs());

        let shifted = compare(&a, &squares(100, 100, &[(11, 10)])).expect("same size");
        assert!(!shifted.differs(), "a square a pixel to the side differs: {shifted:?}");

        let blank = compare(&squares(100, 100, &[]), &a).expect("same size");
        assert_eq!(blank.ink_ratio, 0.0);
        assert!(blank.differs(), "a page hayro left blank does not differ: {blank:?}");

        let missing = compare(&a, &squares(100, 100, &[(10, 10), (60, 60)])).expect("same size");
        assert!(missing.differs(), "a square left out does not differ: {missing:?}");

        assert_eq!(compare(&squares(100, 100, &[]), &squares(103, 100, &[])), None);
    }

    /// What hayro cannot read is `None`, not a panic.
    #[test]
    fn a_file_hayro_cannot_read_is_none() {
        assert!(Preview::new(b"not a pdf at all".to_vec()).is_none());
        assert!(Preview::new(Vec::new()).is_none());
    }
}
