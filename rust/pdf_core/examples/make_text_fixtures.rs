//! Generates the fixtures the extraction and OCR work is tested against.
//!
//! ```text
//! PAGIFY_PDFIUM_LIB=<pdfium> cargo run --example make_text_fixtures -- fixtures
//! ```
//!
//! Written as **raw PDF bytes** rather than through PDFium, and that is the
//! point rather than an eccentricity. What separates a well-authored page from
//! a browser print is the *order its content stream is emitted in*, and no
//! writing API lets you choose to emit it badly. These have to be built by hand
//! to be wrong in the specific way real documents are wrong.
//!
//! Following `fixtures/README.md`: what cannot be generated honestly is listed
//! as a gap rather than approximated, because a fixture that looks like the
//! thing it stands for and is not leaves a hole that looks covered.

use std::fmt::Write as _;

// ---------------------------------------------------------------------------
// A very small PDF writer
// ---------------------------------------------------------------------------

/// Objects are numbered from 1 and written in order; the xref is built from
/// where each one landed. Enough for a page, a font and an image, and
/// deliberately no more.
struct Pdf {
    objects: Vec<Vec<u8>>,
}

impl Pdf {
    fn new() -> Self {
        Pdf { objects: Vec::new() }
    }

    /// Add an object, returning its number.
    fn add(&mut self, body: impl Into<Vec<u8>>) -> usize {
        self.objects.push(body.into());
        self.objects.len()
    }

    /// A stream object: dictionary entries plus the bytes, with `/Length`
    /// filled in.
    fn add_stream(&mut self, extra: &str, data: &[u8]) -> usize {
        let mut body = format!("<< /Length {}{} >>\nstream\n", data.len(), extra).into_bytes();
        body.extend_from_slice(data);
        body.extend_from_slice(b"\nendstream");
        self.add(body)
    }

    fn finish(self, root: usize) -> Vec<u8> {
        let mut out = b"%PDF-1.7\n%\xE2\xE3\xCF\xD3\n".to_vec();
        let mut offsets = Vec::with_capacity(self.objects.len());

        for (index, body) in self.objects.iter().enumerate() {
            offsets.push(out.len());
            out.extend_from_slice(format!("{} 0 obj\n", index + 1).as_bytes());
            out.extend_from_slice(body);
            out.extend_from_slice(b"\nendobj\n");
        }

        let xref_at = out.len();
        out.extend_from_slice(format!("xref\n0 {}\n", self.objects.len() + 1).as_bytes());
        out.extend_from_slice(b"0000000000 65535 f \n");
        for offset in &offsets {
            out.extend_from_slice(format!("{offset:010} 00000 n \n").as_bytes());
        }
        out.extend_from_slice(
            format!(
                "trailer\n<< /Size {} /Root {root} 0 R >>\nstartxref\n{xref_at}\n%%EOF\n",
                self.objects.len() + 1
            )
            .as_bytes(),
        );
        out
    }
}

/// One page, one content stream, optionally one font and one image.
fn one_page(width: f32, height: f32, content: &[u8], font: bool, image: Option<&[u8]>) -> Vec<u8> {
    let mut pdf = Pdf::new();

    let mut resources = String::from("<<");
    if font {
        // Helvetica: one of the standard 14, so nothing has to be embedded and
        // the fixture stays small enough to commit.
        resources.push_str(" /Font << /F1 <<");
        resources.push_str(" /Type /Font /Subtype /Type1 /BaseFont /Helvetica >> >>");
    }
    let image_object = image.map(|jpeg| {
        // Dimensions are read back from the JPEG's own SOF marker rather than
        // passed in, so the two can never disagree.
        let (w, h) = jpeg_size(jpeg).expect("a JPEG with a readable SOF marker");
        pdf.add_stream(
            &format!(
                " /Type /XObject /Subtype /Image /Width {w} /Height {h} \
                  /ColorSpace /DeviceGray /BitsPerComponent 8 /Filter /DCTDecode"
            ),
            jpeg,
        )
    });
    if let Some(number) = image_object {
        let _ = write!(resources, " /XObject << /Im1 {number} 0 R >>");
    }
    resources.push_str(" >>");

    let contents = pdf.add_stream("", content);
    let pages = pdf.objects.len() + 2; // page is next, then Pages
    let page = pdf.add(format!(
        "<< /Type /Page /Parent {pages} 0 R /MediaBox [0 0 {width} {height}] \
         /Resources {resources} /Contents {contents} 0 R >>"
    ));
    let pages = pdf.add(format!("<< /Type /Pages /Kids [{page} 0 R] /Count 1 >>"));
    let root = pdf.add(format!("<< /Type /Catalog /Pages {pages} 0 R >>"));

    pdf.finish(root)
}

/// Width and height from a JPEG's start-of-frame marker.
fn jpeg_size(data: &[u8]) -> Option<(u16, u16)> {
    let mut i = 2usize; // past SOI
    while i + 9 < data.len() {
        if data[i] != 0xFF {
            i += 1;
            continue;
        }
        let marker = data[i + 1];
        let length = u16::from_be_bytes([data[i + 2], data[i + 3]]) as usize;
        // SOF0..SOF3 and SOF5..SOF15, skipping the DHT/JPG/DAC markers.
        if (0xC0..=0xCF).contains(&marker) && !matches!(marker, 0xC4 | 0xC8 | 0xCC) {
            return Some((
                u16::from_be_bytes([data[i + 7], data[i + 8]]),
                u16::from_be_bytes([data[i + 5], data[i + 6]]),
            ));
        }
        i += 2 + length;
    }
    None
}

/// One absolutely positioned run of text.
///
/// Its own `BT`/`ET` block with an explicit text matrix, which is how a
/// layout engine emits a fragment and *not* how a word processor emits a
/// paragraph. Ordering these blocks is the whole experiment.
fn run(text: &str, x: f32, y: f32, size: f32) -> String {
    let escaped = text.replace('\\', r"\\").replace('(', r"\(").replace(')', r"\)");
    format!("BT /F1 {size} Tf 1 0 0 1 {x} {y} Tm ({escaped}) Tj ET\n")
}

// ---------------------------------------------------------------------------
// The fixtures
// ---------------------------------------------------------------------------

const LEFT: [&str; 8] = [
    "The luminaire housing is formed from",
    "extruded aluminium with a powder",
    "coated finish. Ingress protection is",
    "rated to IP65 throughout the range,",
    "and the diffuser is opal polycarbonate",
    "with a nominal transmission of eighty",
    "two percent measured at the centre",
    "of the emitting surface.",
];

const RIGHT: [&str; 8] = [
    "Control gear is supplied loose or",
    "integral depending on the variant",
    "ordered. DALI-2 dimming is available",
    "across every output, and emergency",
    "versions carry a three hour battery",
    "tested to the relevant standard for",
    "self contained luminaires used in",
    "commercial installations.",
];

/// Two columns emitted the way a sane producer emits them: the whole of the
/// left column, then the whole of the right.
///
/// **The regression guard.** Reconstruction must not fire on this. The trust
/// check firing when it should not is the single most damaging failure
/// available, because it would degrade every well-authored document in the name
/// of fixing the broken ones.
fn two_column() -> Vec<u8> {
    let mut content = String::new();
    for (row, line) in LEFT.iter().enumerate() {
        content.push_str(&run(line, 50.0, 700.0 - row as f32 * 16.0, 10.0));
    }
    for (row, line) in RIGHT.iter().enumerate() {
        content.push_str(&run(line, 320.0, 700.0 - row as f32 * 16.0, 10.0));
    }
    one_page(595.0, 842.0, content.as_bytes(), true, None)
}

/// The same text, emitted in paint order rather than reading order.
///
/// What a browser print produces: fragments placed absolutely, in whatever
/// sequence the layout engine happened to paint them. Character order is not
/// reading order here, and reconstruction must detect that.
fn shredded() -> Vec<u8> {
    let mut fragments: Vec<(f32, f32, &str)> = Vec::new();
    for (row, line) in LEFT.iter().enumerate() {
        fragments.push((50.0, 700.0 - row as f32 * 16.0, line));
    }
    for (row, line) in RIGHT.iter().enumerate() {
        fragments.push((320.0, 700.0 - row as f32 * 16.0, line));
    }

    // Deterministically scrambled — a fixture that differs between runs is a
    // fixture nobody can pin a golden output against.
    let order = [11, 2, 14, 5, 0, 9, 3, 12, 7, 1, 15, 4, 10, 6, 13, 8];
    let mut content = String::new();
    for index in order {
        let (x, y, text) = fragments[index];
        content.push_str(&run(text, x, y, 10.0));
    }
    one_page(595.0, 842.0, content.as_bytes(), true, None)
}

/// Type converted to outlines: real glyph contours, filled as paths, with no
/// text object anywhere on the page.
///
/// Drawn from a real font rather than from invented shapes, because the whole
/// point of the matcher is that the contours coincide with a font's own — and
/// invented squares would prove nothing about that.
fn outlined(font: &[u8]) -> Option<Vec<u8>> {
    let face = ttf_parser::Face::parse(font, 0).ok()?;
    let scale = 20.0 / face.units_per_em() as f32;

    let mut content = String::from("0 g\n");

    // A page of it, not a headline. A catalogue page that went through
    // Illustrator has hundreds of these, and a fixture carrying thirteen tests
    // a threshold rather than the thing the threshold is for.
    for (row, line) in LEFT.iter().chain(RIGHT.iter()).enumerate() {
        let mut pen_x = 60.0f32;
        let baseline = 740.0 - row as f32 * 34.0;

        for ch in line.chars() {
            if ch == ' ' {
                pen_x += 12.0;
                continue;
            }
            let Some(glyph) = face.glyph_index(ch) else { continue };
            let mut path = PathWriter { out: String::new(), x: pen_x, y: baseline, scale };
            if face.outline_glyph(glyph, &mut path).is_some() {
                content.push_str(&path.out);
                content.push_str("f\n");
            }
            pen_x += face.glyph_hor_advance(glyph).unwrap_or(600) as f32 * scale;
        }
    }

    Some(one_page(595.0, 842.0, content.as_bytes(), false, None))
}

/// Emits a glyph's contours as PDF path operators.
struct PathWriter {
    out: String,
    x: f32,
    y: f32,
    scale: f32,
}

impl PathWriter {
    fn at(&self, x: f32, y: f32) -> (f32, f32) {
        (self.x + x * self.scale, self.y + y * self.scale)
    }
}

impl ttf_parser::OutlineBuilder for PathWriter {
    fn move_to(&mut self, x: f32, y: f32) {
        let (x, y) = self.at(x, y);
        let _ = writeln!(self.out, "{x:.2} {y:.2} m");
    }
    fn line_to(&mut self, x: f32, y: f32) {
        let (x, y) = self.at(x, y);
        let _ = writeln!(self.out, "{x:.2} {y:.2} l");
    }
    fn quad_to(&mut self, x1: f32, y1: f32, x: f32, y: f32) {
        // PDF has no quadratic operator; raised to a cubic, which is exact.
        let (cx, cy) = self.at(x1, y1);
        let (px, py) = self.at(x, y);
        let _ = writeln!(self.out, "{cx:.2} {cy:.2} {cx:.2} {cy:.2} {px:.2} {py:.2} c");
    }
    fn curve_to(&mut self, x1: f32, y1: f32, x2: f32, y2: f32, x: f32, y: f32) {
        let (a, b) = self.at(x1, y1);
        let (c, d) = self.at(x2, y2);
        let (e, f) = self.at(x, y);
        let _ = writeln!(self.out, "{a:.2} {b:.2} {c:.2} {d:.2} {e:.2} {f:.2} c");
    }
    fn close(&mut self) {
        self.out.push_str("h\n");
    }
}

/// A page that is a picture of text and nothing else.
fn scan(jpeg: &[u8], width: f32, height: f32) -> Vec<u8> {
    let content = format!("q {width} 0 0 {height} 0 0 cm /Im1 Do Q\n");
    one_page(width, height, content.as_bytes(), false, Some(jpeg))
}

fn main() {
    let out = std::env::args().nth(1).unwrap_or_else(|| "fixtures".into());
    let write = |name: &str, bytes: &[u8]| {
        let path = format!("{out}/{name}");
        std::fs::write(&path, bytes).unwrap_or_else(|e| panic!("could not write {path}: {e}"));
        println!("  {:<24} {:>7} bytes", name, bytes.len());
    };

    println!("layout fixtures");
    write("two-column.pdf", &two_column());
    write("shredded.pdf", &shredded());

    println!("outlined type");
    // `PAGIFY_OUTLINED_FONT`, checked first, is for pointing this at an
    // arbitrary face during ad-hoc measurement — proving a *specific* font
    // actually matches well before bundling it as a candidate — without
    // touching what this generator writes by default for anyone who has not
    // set it.
    let overridden = std::env::var("PAGIFY_OUTLINED_FONT").ok();
    let fonts = [
        overridden.as_deref().unwrap_or_default(),
        "/System/Library/Fonts/Supplemental/Arial.ttf",
        "/System/Library/Fonts/Geneva.ttf",
        "/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf",
    ];
    match fonts.iter().find_map(|p| std::fs::read(p).ok()).and_then(|f| outlined(&f)) {
        Some(bytes) => write("outlined.pdf", &bytes),
        // Listed as a gap rather than approximated: shapes that are not a
        // font's own would prove nothing about a matcher whose whole claim is
        // that the contours coincide.
        None => println!("  outlined.pdf             SKIPPED — no usable TrueType font found"),
    }

    println!("scans");
    match render_to_jpeg(&format!("{out}/two-column.pdf"), 300.0) {
        Some(jpeg) => {
            write("scan-300dpi.pdf", &scan(&jpeg, 595.0, 842.0));
            if let Some(skewed) = render_to_jpeg_skewed(&format!("{out}/two-column.pdf"), 300.0, 2.0) {
                write("scan-skewed.pdf", &scan(&skewed, 595.0, 842.0));
            }
        }
        None => println!("  scans                    SKIPPED — set PAGIFY_PDFIUM_LIB"),
    }
    if let Some(jpeg) = render_to_jpeg(&format!("{out}/two-column.pdf"), 150.0) {
        write("scan-lowdpi.pdf", &scan(&jpeg, 595.0, 842.0));
    }
}

/// Render a PDF page to a greyscale JPEG — the picture a scanner would make.
fn render_to_jpeg(path: &str, dpi: f32) -> Option<Vec<u8>> {
    use pdf_core::document::pdfium_doc::PdfiumDocument;
    use pdf_core::document::{Document, RenderRequest};
    use pdf_core::render::{bitmap::PixelOrder, RenderTarget};

    let doc = PdfiumDocument::open_path(path, None).ok()?;
    let page = doc.page(0).ok()?;
    let scale = dpi / 72.0;
    let (w, h) = page.size().pixel_size(scale);

    let mut pixels = vec![0u8; w as usize * h as usize * 4];
    {
        let mut target = RenderTarget::new(w, h, w as usize * 4, PixelOrder::Rgba, &mut pixels).ok()?;
        page.render_into(&RenderRequest { scale, ..Default::default() }, &mut target).ok()?;
    }

    let grey = pdf_core::ocr::preprocess::to_grey(&pixels, w, h);
    encode_jpeg(&grey)
}

/// The same page, tilted — what a page photographed on a desk looks like.
fn render_to_jpeg_skewed(path: &str, dpi: f32, degrees: f32) -> Option<Vec<u8>> {
    use pdf_core::document::pdfium_doc::PdfiumDocument;
    use pdf_core::document::{Document, RenderRequest};
    use pdf_core::render::{bitmap::PixelOrder, RenderTarget};

    let doc = PdfiumDocument::open_path(path, None).ok()?;
    let page = doc.page(0).ok()?;
    let scale = dpi / 72.0;
    let (w, h) = page.size().pixel_size(scale);

    let mut pixels = vec![0u8; w as usize * h as usize * 4];
    {
        let mut target = RenderTarget::new(w, h, w as usize * 4, PixelOrder::Rgba, &mut pixels).ok()?;
        page.render_into(&RenderRequest { scale, ..Default::default() }, &mut target).ok()?;
    }

    let grey = pdf_core::ocr::preprocess::to_grey(&pixels, w, h);
    let tilted = pdf_core::ocr::preprocess::rotate(&grey, degrees.to_radians());
    encode_jpeg(&tilted)
}

fn encode_jpeg(grey: &pdf_core::ocr::GreyImage) -> Option<Vec<u8>> {
    let buffer =
        image::GrayImage::from_raw(grey.width, grey.height, grey.data.clone())?;
    let mut jpeg = Vec::new();
    // Quality 85: visibly a scan, without the artefacts of a hard compression
    // standing in for the recogniser's real problem.
    image::codecs::jpeg::JpegEncoder::new_with_quality(&mut jpeg, 85)
        .encode_image(&image::DynamicImage::ImageLuma8(buffer))
        .ok()?;
    Some(jpeg)
}
