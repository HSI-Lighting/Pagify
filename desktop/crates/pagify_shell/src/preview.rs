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

use hayro::hayro_interpret::util::TransformExt;
use hayro::hayro_interpret::InterpreterSettings;
use hayro::hayro_syntax::object::{Array, Dict, Name, ObjectIdentifier, Stream};
use hayro::hayro_syntax::Pdf;
use hayro::kurbo::Affine;
use hayro::vello_cpu::color::palette::css::WHITE;
use hayro::vello_cpu::{Pixmap, RasterizerSettings, RenderContext, TargetInit};
use hayro::{RenderCache, RenderSettings};

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

    /// Whether the file has PDF layers (optional content), for the comparison
    /// tool's report: they are deliberately not a reason in `differs_because`.
    pub fn has_layers(&self) -> bool {
        self.pdf.xref().has_optional_content_groups()
    }

    /// Page `page` drawn `width` pixels wide on white, as RGBA, turned as the
    /// page turns itself (`/Rotate`), the way PDFium's thumbnail drew it; `None`
    /// where hayro cannot draw it.
    pub fn render(&self, page: usize, width: u32) -> Option<PageRaster> {
        let (w, _) = catch_unwind(AssertUnwindSafe(|| self.pdf.pages().get(page).map(|p| p.render_dimensions()))).ok()??;
        if !(w > 0.0) || width == 0 {
            return None;
        }
        self.render_scaled(page, width as f32 / w)
    }

    /// Page `page` drawn at `scale` pixels a point, as [`Self::render`] draws
    /// it — PDFium's `render_page` of the same page and scale, before any turn
    /// of the view ([`turned`]).
    pub fn render_scaled(&self, page: usize, scale: f32) -> Option<PageRaster> {
        self.draw(page, None, scale)
    }

    /// The part `crop` of page `page` (page points, top-left origin, in the
    /// page as it turns itself), drawn at `scale` — as PDFium's
    /// `render_page_region` draws it: the crop clipped to the page, its edges
    /// rounded to whole pixels.
    pub fn render_region(&self, page: usize, crop: pdf_core::document::Rect, scale: f32) -> Option<PageRaster> {
        self.draw(page, Some(crop), scale)
    }

    fn draw(&self, page: usize, crop: Option<pdf_core::document::Rect>, scale: f32) -> Option<PageRaster> {
        let drawn = catch_unwind(AssertUnwindSafe(|| {
            let page = self.pdf.pages().get(page)?;
            let (w, h) = page.render_dimensions();
            if !(w > 0.0 && h > 0.0 && scale.is_finite() && scale > 0.0) {
                return None;
            }
            let (left, top, right, bottom) = match crop {
                None => (0.0, 0.0, w, h),
                Some(c) => (c.left.min(c.right).max(0.0), c.top.min(c.bottom).max(0.0), c.left.max(c.right).min(w), c.top.max(c.bottom).min(h)),
            };
            let (x0, y0) = ((left * scale).round(), (top * scale).round());
            let (width, height) = ((right * scale).round() - x0, (bottom * scale).round() - y0);
            // hayro's pixmap is at most 65,535 pixels a side.
            if !(width >= 1.0 && height >= 1.0 && width <= u16::MAX as f32 && height <= u16::MAX as f32) {
                return None;
            }
            // `hayro::render`, with the crop's corner moved to the pixmap's.
            let mut ctx = RenderContext::new(width as u16, height as u16);
            let transform = Affine::translate((-x0 as f64, -y0 as f64))
                * Affine::scale(scale as f64)
                * page.initial_transform(true).to_kurbo();
            let settings = (InterpreterSettings::default(), RenderSettings::default());
            hayro::render_into(page, &RenderCache::new(), &settings.0, &settings.1, &mut ctx, transform);
            ctx.flush();
            let mut pixmap = Pixmap::new(ctx.width(), ctx.height());
            ctx.render_with(
                &mut pixmap,
                &mut hayro::vello_cpu::Resources::default(),
                RasterizerSettings { target_init: TargetInit::Clear(WHITE), ..Default::default() },
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

    /// Why hayro may draw page `page` differently from PDFium — what prints —
    /// or `None` when nothing on it is known to. Read from the page's
    /// dictionaries, not by drawing it; the first reason found is the answer.
    ///
    /// PDF layers (optional content) are deliberately not a reason: CAD sets are
    /// full of them, Pagify never turns one on or off, and hayro shows the ones
    /// the file shows.
    pub fn differs_because(&self, page: usize) -> Option<Difference> {
        let found = catch_unwind(AssertUnwindSafe(|| {
            let page = self.pdf.pages().get(page)?;
            let annots: Vec<Dict> =
                page.raw().get::<Array>(b"Annots").map(|a| a.iter::<Dict>().collect()).unwrap_or_default();
            let unseen = |a: &Dict| a.get::<u32>(b"F").is_some_and(|f| f & (2 | 32) != 0);
            let subtype = |a: &Dict| a.get::<Name>(b"Subtype").map(|n| n.as_ref().to_vec()).unwrap_or_default();
            if annots.iter().any(|a| {
                !matches!(&subtype(a)[..], b"Link" | b"Popup") && !a.contains_key(b"AP") && !unseen(a)
            }) {
                return Some(Difference::AnnotationWithoutAppearance);
            }
            let xref = self.pdf.xref();
            let needs = xref
                .get::<Dict>(xref.root_id())
                .and_then(|c| c.get::<Dict>(b"AcroForm"))
                .and_then(|f| f.get::<bool>(b"NeedAppearances"))
                == Some(true);
            if needs && annots.iter().any(|a| subtype(a) == b"Widget") {
                return Some(Difference::FormNeedsAppearances);
            }
            let r = page.resources();
            let mut seen = std::collections::HashSet::new();
            resources_differ(&r.fonts, &r.x_objects, 8, &mut seen)
        }));
        found.ok().flatten()
    }
}

/// Where "Exact" — every page picture drawn by PDFium only — is remembered,
/// beside `appearance.json` and in the same shape.
pub fn exact_pages_path() -> Option<std::path::PathBuf> {
    crate::state::state_dir().map(|dir| dir.join("exact-pages.json"))
}

/// A missing or unreadable file is off, never an error.
pub fn load_exact_pages_from(path: &std::path::Path) -> bool {
    std::fs::read_to_string(path).ok().and_then(|text| serde_json::from_str::<bool>(&text).ok()).unwrap_or(false)
}

pub fn save_exact_pages_to(path: &std::path::Path, on: bool) -> std::io::Result<()> {
    crate::state::write_own(path, if on { b"true" } else { b"false" })
}

pub fn load_exact_pages() -> bool {
    exact_pages_path().is_some_and(|path| load_exact_pages_from(&path))
}

pub fn save_exact_pages(on: bool) {
    if let Some(path) = exact_pages_path() {
        let _ = save_exact_pages_to(&path, on);
    }
}

/// `raster` turned a quarter clockwise a step, as PDFium's view rotation turns
/// a page.
pub fn turned(raster: PageRaster, rotation: pdf_core::Rotation) -> PageRaster {
    use pdf_core::Rotation;
    let (w, h) = (raster.width as usize, raster.height as usize);
    let (nw, nh) = match rotation {
        Rotation::None => return raster,
        Rotation::Clockwise180 => (w, h),
        Rotation::Clockwise90 | Rotation::Clockwise270 => (h, w),
    };
    let mut pixels = vec![0u8; raster.pixels.len()];
    for y in 0..h {
        for x in 0..w {
            let (nx, ny) = match rotation {
                Rotation::Clockwise90 => (h - 1 - y, x),
                Rotation::Clockwise180 => (w - 1 - x, h - 1 - y),
                _ => (y, w - 1 - x),
            };
            let (from, to) = ((y * w + x) * 4, (ny * nw + nx) * 4);
            pixels[to..to + 4].copy_from_slice(&raster.pixels[from..from + 4]);
        }
    }
    PageRaster { width: nw as u32, height: nh as u32, pixels, from_cache: raster.from_cache }
}

/// Why hayro may draw a page differently from PDFium.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Difference {
    /// A font the file does not hold, which each renderer draws from fonts of
    /// its own.
    FontNotEmbedded,
    /// An annotation without an appearance stream, which PDFium draws from
    /// what the annotation says and hayro does not draw.
    AnnotationWithoutAppearance,
    /// A form that asks for its fields to be drawn afresh, which PDFium does
    /// and hayro does not.
    FormNeedsAppearances,
    /// A JPEG picture with more or fewer colour channels than its colour
    /// space has: PDFium takes the channels it needs, hayro reads the bytes as
    /// the colour space says and draws something else.
    ImageNotAsDeclared,
}

impl Difference {
    /// For the session log: "page N drawn by PDFium: <reason>".
    pub fn reason(self) -> &'static str {
        match self {
            Difference::FontNotEmbedded => "a font on it is not in the file",
            Difference::AnnotationWithoutAppearance => "an annotation on it has no appearance of its own",
            Difference::FormNeedsAppearances => "its form asks for its fields to be drawn afresh",
            Difference::ImageNotAsDeclared => "a JPEG picture on it has other colour channels than the file says",
        }
    }
}

/// A font of `fonts` that is neither in the file nor one both renderers carry,
/// or a picture of `x_objects` hayro reads otherwise than PDFium — or either in
/// a form drawn through `x_objects`, to `depth` forms deep.
fn resources_differ(
    fonts: &Dict,
    x_objects: &Dict,
    depth: u8,
    seen: &mut std::collections::HashSet<ObjectIdentifier>,
) -> Option<Difference> {
    let not_embedded = |font: Dict| {
        if font.get::<Name>(b"Subtype").is_some_and(|n| n.as_ref() == b"Type3") {
            return false;
        }
        let descriptor = if font.get::<Name>(b"Subtype").is_some_and(|n| n.as_ref() == b"Type0") {
            font.get::<Array>(b"DescendantFonts")
                .and_then(|d| d.iter::<Dict>().next())
                .and_then(|d| d.get::<Dict>(b"FontDescriptor"))
        } else {
            font.get::<Dict>(b"FontDescriptor")
        };
        let held = descriptor
            .is_some_and(|d| [&b"FontFile"[..], b"FontFile2", b"FontFile3"].iter().any(|k| d.contains_key(k)));
        !held && !font.get::<Name>(b"BaseFont").is_some_and(|n| standard_like(n.as_ref()))
    };
    if fonts.keys().any(|k| fonts.get::<Dict>(&k).is_some_and(not_embedded)) {
        return Some(Difference::FontNotEmbedded);
    }
    for k in x_objects.keys() {
        let Some(x) = x_objects.get::<Stream>(&k) else { continue };
        if !seen.insert(x.obj_id()) {
            continue;
        }
        let d = x.dict();
        match d.get::<Name>(b"Subtype").as_ref().map(|n| n.as_ref()) {
            Some(b"Image") if jpeg_unlike_its_colour_space(&x) => return Some(Difference::ImageNotAsDeclared),
            Some(b"Form") if depth > 0 => {
                let r = d.get::<Dict>(b"Resources").unwrap_or_default();
                let (f, x) = (r.get::<Dict>(b"Font").unwrap_or_default(), r.get::<Dict>(b"XObject").unwrap_or_default());
                if let Some(found) = resources_differ(&f, &x, depth - 1, seen) {
                    return Some(found);
                }
            }
            _ => {}
        }
    }
    None
}

/// A font name both renderers draw alike without the file holding it, after
/// any `ABCDEF+` subset prefix: a standard font, or Arial, Times New Roman or
/// Courier New, in a style hayro draws — regular, bold, italic. hayro draws
/// any other Arial (Black, Narrow, Unicode MS) as plain Helvetica, where PDFium
/// has the face from Windows: "Meeting room lights", page 1, set in
/// Arial-Black, came out regular (the comparison tool, 2026-11-05).
fn standard_like(name: &[u8]) -> bool {
    let name = match name.get(..7) {
        Some(p) if p[6] == b'+' && p[..6].iter().all(u8::is_ascii_uppercase) => &name[7..],
        _ => name,
    };
    let mut n: Vec<u8> =
        name.iter().filter(|c| !matches!(c, b'-' | b',' | b' ' | b'_')).map(u8::to_ascii_lowercase).collect();
    if n.ends_with(b"mt") {
        n.truncate(n.len() - 2);
    }
    let families = ["helvetica", "arial", "timesnewromanps", "timesnewroman", "times", "couriernewps", "couriernew", "courier", "symbol", "zapfdingbats"];
    let styles: [&[u8]; 9] = [b"", b"regular", b"normal", b"roman", b"bold", b"italic", b"bolditalic", b"oblique", b"boldoblique"];
    families.iter().find_map(|f| n.strip_prefix(f.as_bytes())).is_some_and(|style| styles.contains(&style))
}

/// A JPEG picture whose colour channels are not as many as its colour space
/// has (`DeviceGray`, `DeviceRGB`, `DeviceCMYK` or `ICCBased`; any other is
/// not judged).
fn jpeg_unlike_its_colour_space(image: &Stream) -> bool {
    let d = image.dict();
    let filters: Vec<Vec<u8>> = match d.get::<Name>(b"Filter") {
        Some(n) => vec![n.as_ref().to_vec()],
        None => d.get::<Array>(b"Filter").map(|a| a.iter::<Name>().map(|n| n.as_ref().to_vec()).collect()).unwrap_or_default(),
    };
    if !matches!(&filters[..], [only] if only == b"DCTDecode" || only == b"DCT") {
        return false;
    }
    let declared = match d.get::<Name>(b"ColorSpace") {
        Some(n) => match n.as_ref() {
            b"DeviceGray" | b"G" => 1,
            b"DeviceRGB" | b"RGB" => 3,
            b"DeviceCMYK" | b"CMYK" => 4,
            _ => return false,
        },
        None => {
            let Some(space) = d.get::<Array>(b"ColorSpace") else { return false };
            let mut parts = space.flex_iter();
            if !parts.next::<Name>().is_some_and(|n| n.as_ref() == b"ICCBased") {
                return false;
            }
            let Some(n) = parts.next::<Stream>().and_then(|s| s.dict().get::<u8>(b"N")) else { return false };
            n
        }
    };
    jpeg_channels(&image.raw_data()).is_some_and(|channels| channels != declared)
}

/// The number of colour channels a JPEG's frame header gives.
fn jpeg_channels(data: &[u8]) -> Option<u8> {
    if !data.starts_with(&[0xFF, 0xD8]) {
        return None;
    }
    let mut at = 2;
    while at + 4 <= data.len() {
        if data[at] != 0xFF {
            return None;
        }
        match data[at + 1] {
            0xFF => at += 1,
            0xC0..=0xCF if !matches!(data[at + 1], 0xC4 | 0xC8 | 0xCC) => return data.get(at + 9).copied(),
            0xDA | 0xD9 => return None,
            _ => at += 2 + u16::from_be_bytes([data[at + 2], data[at + 3]]) as usize,
        }
    }
    None
}

/// Above this average difference a channel, two pictures of a page differ.
/// Antialiasing alone, the two renderers' edges a pixel apart, came to under 6
/// on every fixture at thumbnail size, and to 6.45 at 1.5 px a point on a
/// dense table of the owner's ("Meeting room lights", pages 12–13: the same
/// words and rules, none moved); the least real difference found was 8.77
/// (page 1, a title in Arial Black drawn regular). Measured with
/// `examples/compare_renderers.rs`, 2026-11-05.
pub const DIFF_MEAN: f64 = 7.5;
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
                assert!(c.mean_diff < 6.0, "{name} page {page}: hayro and PDFium differ by {:.1} a channel on average", c.mean_diff);
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

    /// A one-page PDF: `catalog` and `page` added to the catalog's and the
    /// page's dictionaries, `objects` numbered from 4.
    fn pdf_with(catalog: &str, page: &str, objects: &[&str]) -> Vec<u8> {
        let mut all = vec![
            format!("<< /Type /Catalog /Pages 2 0 R {catalog} >>"),
            "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_string(),
            format!("<< /Type /Page /Parent 2 0 R /MediaBox [0 0 200 300] {page} >>"),
        ];
        all.extend(objects.iter().map(|o| o.to_string()));
        let mut out = b"%PDF-1.7\n".to_vec();
        let mut offsets = Vec::new();
        for (i, body) in all.iter().enumerate() {
            offsets.push(out.len());
            out.extend_from_slice(format!("{} 0 obj\n{body}\nendobj\n", i + 1).as_bytes());
        }
        let xref_at = out.len();
        out.extend_from_slice(format!("xref\n0 {}\n0000000000 65535 f \n", all.len() + 1).as_bytes());
        for offset in &offsets {
            out.extend_from_slice(format!("{offset:010} 00000 n \n").as_bytes());
        }
        out.extend_from_slice(
            format!("trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref_at}\n%%EOF\n", all.len() + 1).as_bytes(),
        );
        out
    }

    fn differs_because(catalog: &str, page: &str, objects: &[&str]) -> Option<Difference> {
        Preview::new(pdf_with(catalog, page, objects)).expect("hayro reads it").differs_because(0)
    }

    /// A font that is not in the file is drawn by each renderer from fonts of
    /// its own — alike for the standard fonts, which both carry, but not for
    /// anything else (Calibri), wherever on the page it is used.
    #[test]
    fn a_font_not_in_the_file_is_a_difference_unless_both_renderers_carry_it() {
        let fonts = "/Resources << /Font << /F1 4 0 R >> >>";
        let helvetica = "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica-Bold >>";
        assert_eq!(differs_because("", fonts, &[helvetica]), None);
        for alike in ["Arial-BoldMT", "Arial,BoldItalic", "TimesNewRomanPS-BoldMT", "CourierNewPSMT", "Times-Roman"] {
            let font = format!("<< /Type /Font /Subtype /TrueType /BaseFont /{alike} >>");
            assert_eq!(differs_because("", fonts, &[&font]), None, "{alike}");
        }
        // hayro draws these as plain Helvetica; PDFium has the faces from
        // Windows. Arial-Black was the title of "Meeting room lights", page 1:
        // bold by PDFium, regular by hayro (the comparison tool, 2026-11-05).
        for unlike in ["Arial-Black", "ArialNarrow-Bold", "ArialUnicodeMS", "Helvetica-Light"] {
            let font = format!("<< /Type /Font /Subtype /TrueType /BaseFont /{unlike} >>");
            assert_eq!(differs_because("", fonts, &[&font]), Some(Difference::FontNotEmbedded), "{unlike}");
        }
        let calibri = "<< /Type /Font /Subtype /TrueType /BaseFont /Calibri /FontDescriptor 5 0 R >>";
        let bare = "<< /Type /FontDescriptor /FontName /Calibri /Flags 32 >>";
        assert_eq!(differs_because("", fonts, &[calibri, bare]), Some(Difference::FontNotEmbedded));
        let embedded = "<< /Type /FontDescriptor /FontName /Calibri /Flags 32 /FontFile2 6 0 R >>";
        let file = "<< /Length 4 >>\nstream\nabcd\nendstream";
        assert_eq!(differs_because("", fonts, &[calibri, embedded, file]), None);

        let form = "<< /Type /XObject /Subtype /Form /BBox [0 0 10 10] /Resources << /Font << /F1 5 0 R >> >> /Length 0 >>\nstream\n\nendstream";
        let subset = "<< /Type /Font /Subtype /TrueType /BaseFont /ABCDEF+Calibri /FontDescriptor 6 0 R >>";
        assert_eq!(
            differs_because("", "/Resources << /XObject << /X1 4 0 R >> >>", &[form, subset, bare]),
            Some(Difference::FontNotEmbedded)
        );
    }

    /// An annotation without an appearance of its own is drawn by PDFium from
    /// what it says (a highlight's colour and quads); hayro draws nothing. A
    /// link, a pop-up or a hidden annotation is drawn by neither.
    #[test]
    fn an_annotation_without_an_appearance_is_a_difference() {
        let annots = "/Annots [4 0 R]";
        let highlight = "<< /Type /Annot /Subtype /Highlight /Rect [10 10 50 20] /QuadPoints [10 20 50 20 10 10 50 10] /C [1 1 0] >>";
        assert_eq!(differs_because("", annots, &[highlight]), Some(Difference::AnnotationWithoutAppearance));
        let drawn = "<< /Type /Annot /Subtype /Highlight /Rect [10 10 50 20] /AP << /N 5 0 R >> >>";
        let appearance = "<< /Type /XObject /Subtype /Form /BBox [0 0 40 10] /Length 0 >>\nstream\n\nendstream";
        assert_eq!(differs_because("", annots, &[drawn, appearance]), None);
        let link = "<< /Type /Annot /Subtype /Link /Rect [10 10 50 20] /Border [0 0 0] >>";
        assert_eq!(differs_because("", annots, &[link]), None);
        let hidden = "<< /Type /Annot /Subtype /Highlight /Rect [10 10 50 20] /F 2 >>";
        assert_eq!(differs_because("", annots, &[hidden]), None);
    }

    /// A form that asks for its fields to be drawn afresh is, by PDFium, from
    /// the fields' values; hayro draws the appearances stored in the file.
    #[test]
    fn a_form_that_needs_appearances_is_a_difference() {
        let widget = "<< /Type /Annot /Subtype /Widget /FT /Tx /T (a) /V (b) /Rect [10 10 50 20] /AP << /N 5 0 R >> >>";
        let appearance = "<< /Type /XObject /Subtype /Form /BBox [0 0 40 10] /Length 0 >>\nstream\n\nendstream";
        assert_eq!(
            differs_because("/AcroForm << /Fields [4 0 R] /NeedAppearances true >>", "/Annots [4 0 R]", &[widget, appearance]),
            Some(Difference::FormNeedsAppearances)
        );
        assert_eq!(differs_because("/AcroForm << /Fields [4 0 R] >>", "/Annots [4 0 R]", &[widget, appearance]), None);
    }

    /// A JPEG picture with three colour channels the file says is grey
    /// (the scan fixtures, from `make_text_fixtures`): PDFium draws its first
    /// channel, hayro reads the three as grey and draws three narrow copies,
    /// found by the comparison tool. A picture as declared is no difference.
    #[test]
    fn a_picture_unlike_its_colour_space_is_a_difference() {
        let fixtures = format!("{}/../../../rust/pdf_core/fixtures", env!("CARGO_MANIFEST_DIR"));
        let read = |name: &str| Preview::new(std::fs::read(format!("{fixtures}/{name}")).expect("fixture")).expect("hayro reads it");
        for name in ["scan-300dpi.pdf", "scan-lowdpi.pdf", "scan-skewed.pdf"] {
            assert_eq!(read(name).differs_because(0), Some(Difference::ImageNotAsDeclared), "{name}");
        }
        assert_eq!(read("pictures.pdf").differs_because(0), None);
    }

    /// A picture turned a quarter clockwise a step, as PDFium's view rotation
    /// turns a page: red left of blue becomes red above blue.
    #[test]
    fn turned_turns_a_picture_a_quarter_clockwise_a_step() {
        let (red, blue) = ([255, 0, 0, 255], [0, 0, 255, 255]);
        let pair = || PageRaster { width: 2, height: 1, pixels: [red, blue].concat(), from_cache: false };
        let shape = |r: &PageRaster| (r.width, r.height, r.pixels.clone());
        assert_eq!(shape(&turned(pair(), pdf_core::Rotation::None)), (2, 1, [red, blue].concat()));
        assert_eq!(shape(&turned(pair(), pdf_core::Rotation::Clockwise90)), (1, 2, [red, blue].concat()));
        assert_eq!(shape(&turned(pair(), pdf_core::Rotation::Clockwise180)), (2, 1, [blue, red].concat()));
        assert_eq!(shape(&turned(pair(), pdf_core::Rotation::Clockwise270)), (1, 2, [blue, red].concat()));
    }

    /// A page picture drawn by hayro — at a scale, turned, or a part of it, as
    /// the page view asks — looks like the one PDFium draws, on fixtures with
    /// words, pictures, forms, annotations and turned pages (`/Rotate`).
    #[test]
    fn a_page_drawn_by_hayro_at_a_scale_turned_or_in_part_looks_like_pdfiums() {
        use pdf_core::Rotation;
        if std::env::var("PAGIFY_PDFIUM_LIB").is_err() {
            eprintln!("skipped: PAGIFY_PDFIUM_LIB is not set");
            return;
        }
        let fixtures = format!("{}/../../../rust/pdf_core/fixtures", env!("CARGO_MANIFEST_DIR"));
        for name in ["text-lines.pdf", "pictures.pdf", "forms.pdf", "hidden-things.pdf", "pages-ladder.pdf", "two-column.pdf"] {
            let session = crate::Session::open(format!("{fixtures}/{name}")).expect("open");
            let preview = Preview::new(session.current_bytes().expect("bytes")).expect("hayro reads it");
            for page in 0..preview.page_count() {
                for r in [Rotation::None, Rotation::Clockwise90, Rotation::Clockwise180, Rotation::Clockwise270] {
                    let ours = turned(preview.render_scaled(page, 1.5).expect("hayro draws it"), r);
                    let theirs = session.render_page_rotated(page, 1.5, r).expect("PDFium draws it");
                    let c = compare(&ours, &theirs).unwrap_or_else(|| {
                        panic!("{name} page {page} {r:?}: {}x{} against {}x{}", ours.width, ours.height, theirs.width, theirs.height)
                    });
                    assert!(!c.differs(), "{name} page {page} {r:?}: {c:?}");
                }
                let size = session.page_size(page).expect("size");
                let (w, h) = (size.width_pt, size.height_pt);
                let middle = pdf_core::document::Rect { left: w / 4.0, top: h / 4.0, right: 3.0 * w / 4.0, bottom: 3.0 * h / 4.0 };
                let ours = preview.render_region(page, middle, 3.0).expect("hayro draws it");
                let theirs = session.render_page_region(page, middle, 3.0).expect("PDFium draws it");
                let c = compare(&ours, &theirs).unwrap_or_else(|| {
                    panic!("{name} page {page} middle: {}x{} against {}x{}", ours.width, ours.height, theirs.width, theirs.height)
                });
                assert!(!c.differs(), "{name} page {page} middle: {c:?}");
            }
        }
    }

    /// "Exact" is remembered between runs; a missing or unreadable file is off,
    /// never an error.
    #[test]
    fn the_exact_switch_is_remembered() {
        let dir = std::env::temp_dir().join(format!("pagify-exact-pages-{}", std::process::id()));
        let path = dir.join("exact-pages.json");
        assert!(!load_exact_pages_from(&path));
        save_exact_pages_to(&path, true).expect("write");
        assert!(load_exact_pages_from(&path));
        save_exact_pages_to(&path, false).expect("write");
        assert!(!load_exact_pages_from(&path));
        std::fs::write(&path, b"not json").expect("write");
        assert!(!load_exact_pages_from(&path));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// What hayro cannot read is `None`, not a panic.
    #[test]
    fn a_file_hayro_cannot_read_is_none() {
        assert!(Preview::new(b"not a pdf at all".to_vec()).is_none());
        assert!(Preview::new(Vec::new()).is_none());
    }
}
