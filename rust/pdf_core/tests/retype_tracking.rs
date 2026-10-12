//! Retyping a run keeps the spacing the producer tracked it with.
//!
//! # What this guards
//!
//! InDesign and Illustrator justify or letter-space a line by writing a `TJ`
//! number after **every** glyph, next to a `Tc` that offsets it:
//!
//! ```text
//! 8 0 0 8 100 700 Tm /F1 1 Tf -0.164 Tc [(C)-124(O)-124(B) …] TJ
//! ```
//!
//! Retyping the line used to drop every number inside it while `Tc` stayed in
//! force, so each glyph came out about a point tighter than it was drawn: the same
//! words ran 30 pt short and the words ran together. A typed space was worse, as a
//! space adds only its own width plus `Tc` and the producer's gap had gone.
//!
//! ```text
//! PAGIFY_PDFIUM_LIB=<pdfium> cargo test --test retype_tracking
//! ```

mod harness;
use harness::{serial, skip_without_pdfium};

use pdf_core::document::pdfium_doc::PdfiumDocument;
use pdf_core::document::{Document, DocumentMut, TextRun, TextStyle};

const LINE: &str = "COB Efficacy: Max 180 lm/Watt";

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

/// `text` as a tracked `TJ`: `number` after every glyph but the first, which the
/// producer kerned on its own (as the real line's `-109` is).
fn tracked(text: &str, number: f32) -> String {
    let glyphs: Vec<String> = text.chars().map(|c| format!("({c})")).collect();
    let mut out = String::from("[");
    for (i, glyph) in glyphs.iter().enumerate() {
        out.push_str(glyph);
        if i + 1 < glyphs.len() {
            out.push_str(&format!("{}", if i == 0 { number + 15.0 } else { number }));
        }
    }
    out.push(']');
    out
}

fn open(tracking: &str) -> PdfiumDocument {
    let content = format!(
        "BT 0 0 0 0.8 k 8 0 0 8 27 547 Tm /F1 1 Tf -0.164 Tc {} TJ ET\n\
         BT 12 0 0 12 27 500 Tm /F1 1 Tf (A neighbour) Tj ET",
        tracking
    );
    PdfiumDocument::open_bytes(page_with(&content), None).expect("open")
}

fn the_line(doc: &PdfiumDocument) -> TextRun {
    doc.text_runs(0).expect("runs").into_iter().find(|r| r.text.contains("COB")).expect("the tracked line")
}

fn width(run: &TextRun) -> f32 {
    run.rect.right - run.rect.left
}

/// **A line retyped with its own words is as wide as it was.**
#[test]
fn retyping_a_tracked_line_unchanged_keeps_its_width() {
    let Some(_) = skip_without_pdfium() else { return };
    let _lock = serial();
    let mut doc = open(&tracked(LINE, -124.0));
    let before = the_line(&doc);
    doc.set_text_run_styled(0, before.object, LINE, &TextStyle::default()).expect("retype");
    let after = the_line(&doc);
    println!("width {:.2} -> {:.2}", width(&before), width(&after));
    assert!(
        (width(&after) - width(&before)).abs() < 0.5,
        "the retyped line is {:.2} pt wide, it was {:.2}",
        width(&after),
        width(&before)
    );
    assert!((after.rect.left - before.rect.left).abs() < 0.5, "the line moved");
    let neighbour = doc.text_runs(0).unwrap().into_iter().find(|r| r.text.contains("neighbour")).unwrap();
    assert!((neighbour.rect.left - 27.0).abs() < 1.0, "the line below moved");
}

/// **A typed space adds about a space.** Without the tracking it added a bare
/// space plus `Tc` (under a point), so a person typing several to open a gap
/// barely moved anything.
#[test]
fn a_typed_space_in_a_tracked_line_opens_a_normal_gap() {
    let Some(_) = skip_without_pdfium() else { return };
    let _lock = serial();
    let mut doc = open(&tracked(LINE, -124.0));
    let before = the_line(&doc);
    let spaced = LINE.replace("Efficacy: Max", &format!("Efficacy:{}Max", " ".repeat(11)));
    doc.set_text_run_styled(0, before.object, &spaced, &TextStyle::default()).expect("retype");
    let after = the_line(&doc);
    let each = (width(&after) - width(&before)) / 10.0;
    println!("ten more spaces added {:.2} pt each", each);
    // A Helvetica space is 0.278 em at 8: 2.2 pt, and the tracking adds ~1 pt of
    // its own less `Tc`'s 1.3 pt. (No `Tw` on the page: the real line's font has
    // two-byte codes, which `Tw` never reaches.)
    assert!((1.5..2.3).contains(&each), "a typed space added {each:.2} pt");
}

/// The paragraph path writes through the batch; it keeps the spacing too.
#[test]
fn a_batch_retype_keeps_the_width_as_well() {
    let Some(_) = skip_without_pdfium() else { return };
    let _lock = serial();
    let mut doc = open(&tracked(LINE, -124.0));
    let before = the_line(&doc);
    doc.set_text_runs_styled(0, &[(before.object, LINE.to_string(), TextStyle::default())]).expect("retype");
    let after = the_line(&doc);
    assert!(
        (width(&after) - width(&before)).abs() < 0.5,
        "the batch-retyped line is {:.2} pt wide, it was {:.2}",
        width(&after),
        width(&before)
    );
}

/// A line with no tracking is written exactly as before: nothing to carry.
#[test]
fn an_untracked_line_is_left_alone() {
    let Some(_) = skip_without_pdfium() else { return };
    let _lock = serial();
    let mut doc = open(&format!("[({LINE})]"));
    let before = the_line(&doc);
    doc.set_text_run_styled(0, before.object, LINE, &TextStyle::default()).expect("retype");
    let after = the_line(&doc);
    assert!((width(&after) - width(&before)).abs() < 0.5, "{:.2} -> {:.2}", width(&before), width(&after));
}

const DATASHEET: &str = r"C:\Users\hsili\Desktop\test pdf for pgify\TECHNICAL DATASHEET Q-2075-REV.pdf";

/// **The line the report came from**, on the real file: page 1's "COB Efficacy"
/// (Identity-H, `Tc -0.164`, `-124` after every glyph) was 30 pt short once retyped.
/// Then every other run of the page is retyped with its own words, to see what the
/// change does to text that was never tracked. Skips without the datasheet.
#[test]
fn the_reported_line_on_the_datasheet_keeps_its_width() {
    let Some(_) = skip_without_pdfium() else { return };
    if !std::path::Path::new(DATASHEET).exists() {
        eprintln!("skipped: the datasheet is not at {DATASHEET}");
        return;
    }
    let _lock = serial();
    let mut doc = PdfiumDocument::open_path(DATASHEET, None).expect("open");
    let runs = doc.text_runs(0).expect("runs");
    let line = runs.iter().find(|r| r.text.contains("COB Efficacy")).expect("the line").clone();
    let text = line.text.trim_end().to_string();
    doc.set_text_run_styled(0, line.object, &text, &TextStyle::default()).expect("retype");
    let after = doc.text_runs(0).unwrap().into_iter().find(|r| r.text.contains("COB Efficacy")).unwrap();
    println!("COB Efficacy: right edge {:.2} -> {:.2}", line.rect.right, after.rect.right);
    // Every run on the page, retyped as it stands: how many change width?
    let (mut tried, mut refused, mut moved) = (0, 0, Vec::new());
    for run in &runs {
        let words = run.text.trim_end().to_string();
        if words.is_empty() || run.object == line.object {
            continue;
        }
        tried += 1;
        let mut fresh = PdfiumDocument::open_path(DATASHEET, None).expect("open");
        if fresh.set_text_run_styled(0, run.object, &words, &TextStyle::default()).is_err() {
            refused += 1;
            continue;
        }
        let Some(again) = fresh.text_runs(0).unwrap().into_iter().find(|r| r.object == run.object) else { continue };
        let change = (again.rect.right - again.rect.left) - (run.rect.right - run.rect.left);
        if change.abs() > 1.0 {
            moved.push(format!("{words:?} {change:+.1}"));
        }
        if tried >= 60 {
            break;
        }
    }
    println!("{tried} runs retyped unchanged, {refused} refused, {} changed width by over 1 pt:", moved.len());
    for m in &moved {
        println!("   {m}");
    }

    // Not 0.5: the producer drew "ffi" as an `f` and an `fi` ligature and kerned a
    // few pairs (W-a, a-t) on top of its tracking, and a retype spells every letter
    // on its own — 2.6 pt over the line, which was 30.3 before the tracking came
    // across. Keeping the untouched letters' own codes would close the rest.
    assert!((after.rect.right - line.rect.right).abs() < 4.0, "the line is {:.2} pt shorter", line.rect.right - after.rect.right);
}

/// With `PAGIFY_SAVE_TRACKING_PNG=<path>` set, saves the reported line three ways,
/// one above the other — as the producer drew it, retyped with its own words, and
/// retyped with sixteen spaces typed in — to **look at**: a width in points does
/// not say whether the picture is right.
#[test]
fn the_reported_line_looks_right() {
    let Some(_) = skip_without_pdfium() else { return };
    let Ok(out) = std::env::var("PAGIFY_SAVE_TRACKING_PNG") else { return };
    if !std::path::Path::new(DATASHEET).exists() {
        return;
    }
    let _lock = serial();
    let needle = std::env::var("PAGIFY_TRACKING_NEEDLE").unwrap_or_else(|_| "COB Efficacy".into());
    let mut doc = PdfiumDocument::open_path(DATASHEET, None).expect("open");
    let line = doc.text_runs(0).unwrap().into_iter().find(|r| r.text.contains(&needle)).expect("the line");
    let crop = pdf_core::document::Rect {
        left: line.rect.left - 6.0,
        top: line.rect.top - 8.0,
        right: line.rect.left + 194.0,
        bottom: line.rect.top + 14.0,
    };
    let strip = |doc: &PdfiumDocument| {
        let bitmap = doc
            .page(0)
            .expect("page")
            .render_region(&pdf_core::document::RegionRequest { crop, scale: 6.0, ..Default::default() })
            .expect("render");
        let mut rgba = Vec::new();
        for row in 0..bitmap.height as usize {
            for px in bitmap.data[row * bitmap.stride..][..bitmap.width as usize * 4].chunks(4) {
                let (r, b) = if bitmap.order == pdf_core::render::PixelOrder::Bgra { (px[2], px[0]) } else { (px[0], px[2]) };
                rgba.extend_from_slice(&[r, px[1], b, 255]);
            }
        }
        (rgba, bitmap.width, bitmap.height)
    };
    let mut strips = vec![strip(&doc)];
    let words = line.text.trim_end().to_string();
    for text in [words.clone(), words.replacen(' ', &" ".repeat(12), 1)] {
        let mut copy = PdfiumDocument::open_path(DATASHEET, None).expect("open");
        copy.set_text_run_styled(0, line.object, &text, &TextStyle::default()).expect("retype");
        strips.push(strip(&copy));
    }
    let (width, height) = (strips[0].1, strips[0].2);
    let mut all = Vec::new();
    for (rgba, _, _) in &strips {
        all.extend_from_slice(rgba);
    }
    image::save_buffer(out, &all, width, height * strips.len() as u32, image::ColorType::Rgba8).expect("png");
}
