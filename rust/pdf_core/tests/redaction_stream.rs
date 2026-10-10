//! A redaction takes out what it covers and **nothing else on the page**.
//!
//! # What this guards
//!
//! Redaction used to remove objects through PDFium's model and then call
//! `FPDFPage_GenerateContent`, which re-emits every operator on the page. On pages
//! from Illustrator or InDesign that is lossy — fonts that share a name merge,
//! `Tc`/`Tw` go, CMYK colours and shadings turn black — so redacting one word
//! changed 3–35% of a real datasheet outside the area. It is now applied to the
//! page's own content stream: the covered operators are cut out, a mark is
//! appended, and every other byte is copied through.
//!
//! These tests are built so that they **fail on the old path**: the page carries
//! the operators PDFium rewrote (a CMYK fill, character and word spacing), and
//! the assertions look for those operators surviving *byte for byte*, which no
//! regeneration can give.
//!
//! ```text
//! PAGIFY_PDFIUM_LIB=<pdfium> cargo test --test redaction_stream
//! ```

mod harness;
use harness::{serial, skip_without_pdfium};

use pdf_core::document::pdfium_doc::PdfiumDocument;
use pdf_core::document::{Color, Document, DocumentMut, Rect, Redaction, RegionRequest};

fn stream(data: &str) -> String {
    format!("<< /Length {} >>\nstream\n{data}\nendstream", data.len())
}

/// A one-page PDF drawing `content` on Helvetica as `/F1`.
fn page_with(content: &str) -> Vec<u8> {
    let objects = [
        "<< /Type /Catalog /Pages 2 0 R >>".to_string(),
        "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_string(),
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Contents 4 0 R \
         /Resources << /Font << /F1 5 0 R >> >> >>"
            .to_string(),
        stream(content),
        "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>".to_string(),
    ];
    let mut out = b"%PDF-1.4\n".to_vec();
    let mut offsets = Vec::new();
    for (i, body) in objects.iter().enumerate() {
        offsets.push(out.len());
        out.extend_from_slice(format!("{} 0 obj\n{body}\nendobj\n", i + 1).as_bytes());
    }
    let xref_at = out.len();
    out.extend_from_slice(format!("xref\n0 {}\n0000000000 65535 f \n", objects.len() + 1).as_bytes());
    for offset in &offsets {
        out.extend_from_slice(format!("{offset:010} 00000 n \n").as_bytes());
    }
    out.extend_from_slice(
        format!("trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref_at}\n%%EOF", objects.len() + 1)
            .as_bytes(),
    );
    out
}

fn open(content: &str) -> PdfiumDocument {
    PdfiumDocument::open_bytes(page_with(content), None).expect("open")
}

/// [`page_with`], plus square annotations at these rectangles (PDF space).
fn page_with_annotations(content: &str, rects: &[[f32; 4]]) -> Vec<u8> {
    let first = 6;
    let refs: Vec<String> = (0..rects.len()).map(|i| format!("{} 0 R", first + i)).collect();
    let mut objects = vec![
        "<< /Type /Catalog /Pages 2 0 R >>".to_string(),
        "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_string(),
        format!(
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Contents 4 0 R              /Resources << /Font << /F1 5 0 R >> >> /Annots [{}] >>",
            refs.join(" ")
        ),
        stream(content),
        "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>".to_string(),
    ];
    for r in rects {
        objects.push(format!(
            "<< /Type /Annot /Subtype /Square /Rect [{} {} {} {}] /C [1 0 0] /F 4 >>",
            r[0], r[1], r[2], r[3]
        ));
    }
    let mut out = b"%PDF-1.4
".to_vec();
    let mut offsets = Vec::new();
    for (i, body) in objects.iter().enumerate() {
        offsets.push(out.len());
        out.extend_from_slice(format!("{} 0 obj
{body}
endobj
", i + 1).as_bytes());
    }
    let xref_at = out.len();
    out.extend_from_slice(format!("xref
0 {}
0000000000 65535 f 
", objects.len() + 1).as_bytes());
    for offset in &offsets {
        out.extend_from_slice(format!("{offset:010} 00000 n 
").as_bytes());
    }
    out.extend_from_slice(
        format!("trailer
<< /Size {} /Root 1 0 R >>
startxref
{xref_at}
%%EOF", objects.len() + 1)
            .as_bytes(),
    );
    out
}

/// The box round `needle`'s characters on page 0, page space.
fn box_around(doc: &PdfiumDocument, needle: &str) -> Rect {
    let chars = doc.page(0).expect("page").characters().expect("characters");
    let at = chars.text.find(needle).unwrap_or_else(|| panic!("{needle:?} is not on the page: {:?}", chars.text));
    let start = chars.text[..at].chars().count();
    let mut bounds: Option<Rect> = None;
    for index in start..start + needle.chars().count() {
        let b = &chars.boxes[index * 4..index * 4 + 4];
        bounds = Some(match bounds {
            None => Rect { left: b[0], top: b[1], right: b[2], bottom: b[3] },
            Some(r) => Rect {
                left: r.left.min(b[0]),
                top: r.top.min(b[1]),
                right: r.right.max(b[2]),
                bottom: r.bottom.max(b[3]),
            },
        });
    }
    bounds.expect("a non-empty needle")
}

fn black() -> Option<Color> {
    Some(Color { r: 0, g: 0, b: 0, a: 255 })
}

fn redaction(page_index: usize, area: Rect) -> Redaction {
    Redaction { page_index, area, fill: black(), require_complete: true, parts: Vec::new() }
}

/// The whole page as pixels: `(data, width, height, stride)`. **The bitmap's own
/// stride**, never one derived from the page's size in points: A4 is 841.89 high
/// and a derived stride drifts by a row's worth across the page.
fn render(doc: &PdfiumDocument, page: usize) -> (Vec<u8>, usize, usize, usize) {
    let size = doc.page_size(page).expect("size");
    let bitmap = doc
        .page(page)
        .expect("page")
        .render_region(&RegionRequest {
            crop: Rect { left: 0.0, top: 0.0, right: size.width_pt, bottom: size.height_pt },
            scale: 1.0,
            ..RegionRequest::default()
        })
        .expect("render");
    (bitmap.data.clone(), bitmap.width as usize, bitmap.height as usize, bitmap.stride)
}

/// How many pixels differ visibly between two renders of a page, outside `area`
/// and `margin` points round it.
fn pixels_changed_outside(a: &PdfiumDocument, b: &PdfiumDocument, area: &Rect, margin: f32) -> usize {
    let (before, width, height, stride) = render(a, 0);
    let (after, ..) = render(b, 0);
    let mut changed = 0;
    for y in 0..height {
        for x in 0..width {
            let (px, py) = (x as f32, y as f32);
            if px >= area.left - margin && px <= area.right + margin && py >= area.top - margin && py <= area.bottom + margin {
                continue;
            }
            let at = y * stride + x * 4;
            let difference: i32 = (0..3).map(|c| (i32::from(before[at + c]) - i32::from(after[at + c])).abs()).sum();
            if difference > 30 {
                changed += 1;
            }
        }
    }
    changed
}

fn reopened(doc: &mut PdfiumDocument) -> PdfiumDocument {
    let mut bytes = Vec::new();
    doc.save_full_copy(&mut bytes).expect("save");
    PdfiumDocument::open_bytes(bytes, None).expect("reopen")
}

const FIDELITY_PAGE: &str = "0 1 1 0 k 20 20 60 60 re f\n\
    0.2 0.4 0.6 rg\n\
    BT /F1 12 Tf 1.5 Tc 72 700 Td (Alpha ) Tj (SECRET) Tj ( omega) Tj ET\n\
    BT /F1 12 Tf 0 Tc 3 Tw 72 650 Td (second line stays put) Tj ET";

/// **Nothing but the covered word changes — not one other operator.**
///
/// The page has a CMYK fill and character and word spacing, which a regenerated
/// page loses or rewrites. All of them have to be in the stream afterwards, as
/// written, and the word has to be gone from the file as a reader sees it.
#[test]
fn redacting_a_word_leaves_every_other_operator_of_the_page_as_it_was() {
    let Some(_) = skip_without_pdfium() else { return };
    let _lock = serial();

    let mut doc = open(FIDELITY_PAGE);
    let area = box_around(&doc, "SECRET");
    let alpha = box_around(&doc, "Alpha");
    let omega = box_around(&doc, "omega");
    let second = box_around(&doc, "second");
    let before = doc.page_stream(0).expect("stream");

    doc.redact(&redaction(0, area), None).expect("redact");
    let after = doc.page_stream(0).expect("stream");
    let text = String::from_utf8_lossy(&after).to_string();

    // What a regeneration cannot leave alone.
    assert!(text.contains("0 1 1 0 k"), "the CMYK fill was rewritten:\n{text}");
    assert!(text.contains("1.5 Tc"), "the character spacing was lost:\n{text}");
    assert!(text.contains("3 Tw"), "the word spacing was lost:\n{text}");
    assert!(text.contains("(second line stays put) Tj"), "an untouched line was rewritten:\n{text}");
    assert!(text.contains("0.2 0.4 0.6 rg"), "the text colour was rewritten:\n{text}");
    assert!(!text.contains("SECRET"), "the word is still in the stream:\n{text}");

    // Everything before the redacted operator is the same bytes.
    let cut = before.windows(8).position(|w| w == b"(SECRET)").expect("setup");
    let head = &before[..cut];
    assert!(after.starts_with(head), "the stream before the redaction changed");

    // The words either side stayed where they were, and the other line too.
    let reread = reopened(&mut doc);
    let (alpha_after, omega_after, second_after) =
        (box_around(&reread, "Alpha"), box_around(&reread, "omega"), box_around(&reread, "second"));
    for (name, was, now) in [("Alpha", alpha, alpha_after), ("omega", omega, omega_after), ("second", second, second_after)] {
        assert!(
            (was.left - now.left).abs() < 0.5 && (was.top - now.top).abs() < 0.5,
            "{name} moved: {was:?} -> {now:?}"
        );
    }
    let all = reread.page(0).expect("page").characters().expect("characters").text;
    assert!(!all.contains("SECRET"), "the word survived into the saved file: {all:?}");
    assert!(all.contains("Alpha") && all.contains("omega"), "the words around it were lost: {all:?}");
}

/// **The page, pixel for pixel, outside the redaction.**
#[test]
fn a_redaction_changes_no_pixel_outside_its_area() {
    let Some(_) = skip_without_pdfium() else { return };
    let _lock = serial();

    let original = open(FIDELITY_PAGE);
    let mut doc = open(FIDELITY_PAGE);
    let area = box_around(&doc, "SECRET");
    doc.redact(&redaction(0, area), None).expect("redact");

    let changed = pixels_changed_outside(&original, &doc, &area, 3.0);
    assert_eq!(changed, 0, "{changed} pixels outside the redacted area changed");
}

/// A shape wholly inside the area goes with the words; one outside stays.
#[test]
fn a_shape_inside_the_area_is_removed_and_one_outside_is_not() {
    let Some(_) = skip_without_pdfium() else { return };
    let _lock = serial();

    let page = "0.5 g 100 400 50 20 re f\n\
        0.9 0.1 0.1 rg 300 400 50 20 re f\n\
        BT /F1 12 Tf 72 700 Td (label) Tj ET";
    let mut doc = open(page);
    // Page space counts downwards: the first rectangle spans y 372..392.
    let area = Rect { left: 95.0, top: 365.0, right: 155.0, bottom: 395.0 };
    doc.redact(&redaction(0, area), None).expect("redact");

    let text = String::from_utf8_lossy(&doc.page_stream(0).expect("stream")).to_string();
    assert!(!text.contains("100 400 50 20 re"), "the shape inside the area is still drawn:\n{text}");
    assert!(text.contains("300 400 50 20 re"), "the shape outside the area was removed:\n{text}");
    assert!(text.contains("(label) Tj"), "the text elsewhere was touched:\n{text}");
}

/// A line that goes on after the covered word keeps its words where they were,
/// and no stray text of the same line is taken with it.
#[test]
fn words_after_the_redacted_one_on_a_line_stay_where_they_were() {
    let Some(_) = skip_without_pdfium() else { return };
    let _lock = serial();

    let page = "BT /F1 12 Tf 72 700 Td (one ) Tj (two ) Tj (three ) Tj (four) Tj ET";
    let mut doc = open(page);
    let before = {
        let d = open(page);
        (box_around(&d, "three"), box_around(&d, "four"), box_around(&d, "one"))
    };
    let area = box_around(&doc, "two");
    doc.redact(&redaction(0, area), None).expect("redact");

    let reread = reopened(&mut doc);
    let text = reread.page(0).expect("page").characters().expect("characters").text;
    assert!(!text.contains("two"), "the word survived: {text:?}");
    for (name, was) in [("three", before.0), ("four", before.1), ("one", before.2)] {
        let now = box_around(&reread, name);
        assert!(
            (was.left - now.left).abs() < 0.5,
            "{name} moved from {} to {}: the line closed up around the gap",
            was.left,
            now.left
        );
    }
}

/// **A real datasheet, A4 as Illustrator wrote it** — the page that lost 18% of
/// its pixels to one redacted word. Skipped where the file is not.
#[test]
fn redacting_one_word_on_the_users_datasheet_changes_nothing_else() {
    let Some(_) = skip_without_pdfium() else { return };
    let _lock = serial();

    let path = r"C:\Users\hsili\Desktop\test pdf for pgify\TECHNICAL DATASHEET Q-2075-REV.pdf";
    if !std::path::Path::new(path).is_file() {
        eprintln!("skipping: the datasheet is not on this machine");
        return;
    }
    // Pages that were damaged by the old path, with the words to take out.
    for (page, needle) in [(0usize, "Tunable Function: 2700K - 6500K"), (20, "225 x 43 x 30.2 mm"), (40, "Output")] {
        let original = PdfiumDocument::open_path(path, None).expect("open");
        let mut doc = PdfiumDocument::open_path(path, None).expect("open");
        let run = doc
            .text_runs(page)
            .expect("runs")
            .into_iter()
            .find(|r| r.text.contains(needle))
            .unwrap_or_else(|| panic!("page {page} has no {needle:?}"));
        let area = Rect {
            left: run.rect.left - 1.0,
            top: run.rect.top - 1.0,
            right: run.rect.right + 1.0,
            bottom: run.rect.bottom + 1.0,
        };
        doc.redact(&Redaction { page_index: page, area, fill: black(), require_complete: false, parts: Vec::new() }, None)
            .unwrap_or_else(|e| panic!("page {page}: {e}"));

        // `render` always draws page 0, so compare the page itself.
        let size = doc.page_size(page).expect("size");
        let draw = |d: &PdfiumDocument| {
            let b = d
                .page(page)
                .expect("page")
                .render_region(&RegionRequest {
                    crop: Rect { left: 0.0, top: 0.0, right: size.width_pt, bottom: size.height_pt },
                    scale: 1.0,
                    ..RegionRequest::default()
                })
                .expect("render");
            (b.data.clone(), b.width as usize, b.height as usize, b.stride)
        };
        let ((a, w, h, stride), (b, ..)) = (draw(&original), draw(&doc));
        let mut changed = 0;
        for y in 0..h {
            for x in 0..w {
                let (px, py) = (x as f32, y as f32);
                if px >= area.left - 3.0 && px <= area.right + 3.0 && py >= area.top - 3.0 && py <= area.bottom + 3.0 {
                    continue;
                }
                let at = y * stride + x * 4;
                let d: i32 = (0..3).map(|c| (i32::from(a[at + c]) - i32::from(b[at + c])).abs()).sum();
                if d > 30 {
                    changed += 1;
                }
            }
        }
        // A glyph whose box merely touches the area goes with it, by the same
        // rule the old path used; a handful of pixels is that, thousands is a
        // rewritten page.
        assert!(changed <= 20, "page {page}: {changed} pixels outside the redacted {needle:?} changed");
        let runs_after = doc.text_runs(page).expect("runs");
        assert!(!runs_after.iter().any(|r| r.text.contains(needle)), "page {page}: {needle:?} is still on the page");
    }
}

/// **An annotation wholly inside the area goes, one outside stays** — removed
/// through PDFium after the stream has been rewritten, which regenerates nothing.
#[test]
fn an_annotation_inside_the_area_is_removed_and_one_outside_is_not() {
    let Some(_) = skip_without_pdfium() else { return };
    let _lock = serial();

    let content = "BT /F1 12 Tf 72 700 Td (some words here) Tj ET";
    let mut doc = PdfiumDocument::open_bytes(
        page_with_annotations(content, &[[100.0, 400.0, 150.0, 420.0], [300.0, 400.0, 350.0, 420.0]]),
        None,
    )
    .expect("open");
    assert_eq!(doc.annotation_count(0).expect("count"), 2, "setup");

    // Page space counts downwards: the first annotation spans y 372..392.
    let area = Rect { left: 95.0, top: 365.0, right: 155.0, bottom: 395.0 };
    doc.redact(&redaction(0, area), None).expect("redact");

    assert_eq!(doc.annotation_count(0).expect("count"), 1, "only the annotation outside the area should be left");
    let text = String::from_utf8_lossy(&doc.page_stream(0).expect("stream")).to_string();
    assert!(text.contains("(some words here) Tj"), "the text elsewhere was touched:
{text}");
}
