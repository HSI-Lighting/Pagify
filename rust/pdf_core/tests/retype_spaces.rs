//! Retyping runs whose pages break assumptions the lookup used to make:
//! a page whose CropBox is inset (print-ready bleeds, trim marks) and words
//! whose spaces the producer drew as `TJ` pen movements rather than space
//! glyphs.
//!
//! Both shapes refused every retype — "that text is drawn in a way this cannot
//! edit" and "that run's codes cannot be lined up with its text" — on documents
//! that are otherwise perfectly ordinary. Reported from use, on files of
//! exactly these two shapes.
//!
//! ```text
//! PAGIFY_PDFIUM_LIB=<pdfium> cargo test --test retype_spaces
//! ```

mod harness;
use harness::{serial, skip_without_pdfium};

use pdf_core::document::pdfium_doc::PdfiumDocument;
use pdf_core::document::{Document, DocumentMut, TextStyle};

/// A one-page PDF with one Helvetica font: `boxes` is the page's own geometry
/// entries as they would be written (`/MediaBox … /CropBox …`), and `content`
/// its content stream.
fn build_page(boxes: &str, content: &str) -> Vec<u8> {
    let content = format!("{content}\n");
    let objs: Vec<String> = vec![
        "<< /Type /Catalog /Pages 2 0 R >>".into(),
        "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".into(),
        format!(
            "<< /Type /Page /Parent 2 0 R {boxes} \
             /Resources << /Font << /F1 4 0 R >> >> /Contents 5 0 R >>"
        ),
        "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>".into(),
        format!("<< /Length {} >>\nstream\n{content}endstream", content.len()),
    ];
    let mut out = String::from("%PDF-1.4\n");
    let mut offsets = Vec::new();
    for (i, o) in objs.iter().enumerate() {
        offsets.push(out.len());
        out.push_str(&format!("{} 0 obj\n{}\nendobj\n", i + 1, o));
    }
    let xref = out.len();
    out.push_str(&format!("xref\n0 {}\n", objs.len() + 1));
    out.push_str("0000000000 65535 f \n");
    for off in &offsets {
        out.push_str(&format!("{off:010} 00000 n \n"));
    }
    out.push_str(&format!(
        "trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n",
        objs.len() + 1
    ));
    out.into_bytes()
}

fn open(bytes: Vec<u8>) -> PdfiumDocument {
    PdfiumDocument::open_bytes(bytes, None).expect("open the built page")
}

fn saved_and_reopened(doc: &mut PdfiumDocument) -> PdfiumDocument {
    let mut bytes = Vec::new();
    doc.save_full_copy(&mut bytes).expect("save");
    PdfiumDocument::open_bytes(bytes, None).expect("reopen")
}

/// **A page whose CropBox is inset — a print-ready bleed.**
///
/// PDFium reports text geometry from the CropBox; the stream walk works in
/// MediaBox space. With the two boxes inset from each other every run had no
/// operator near it within any matching tolerance, and retyping refused with
/// "that text is drawn in a way this cannot edit" on plain two-line text.
#[test]
fn a_run_on_a_page_whose_crop_box_is_inset_is_retyped() {
    let Some(_) = skip_without_pdfium() else { return };
    let _lock = serial();

    let bytes = build_page(
        "/MediaBox [0 0 612 792] /CropBox [9 9 603 783]",
        "BT /F1 12 Tf 1 0 0 1 72 700 Tm (Crop line one) Tj ET\n\
         BT /F1 12 Tf 1 0 0 1 72 670 Tm (Crop line two) Tj ET",
    );
    let mut doc = open(bytes);
    let before = doc.text_runs(0).expect("runs");
    assert_eq!(before.len(), 2, "the page has two lines of text");

    for run in &before {
        let wanted = format!("{}!", run.text);
        let (previous, _) = doc
            .set_text_run_styled(0, run.object, &wanted, &TextStyle::default())
            .unwrap_or_else(|e| panic!("retyping {:?} was refused: {e}", run.text));
        assert_eq!(previous, run.text, "the words before the change are reported");
    }

    let reopened = saved_and_reopened(&mut doc);
    let after = reopened.text_runs(0).expect("runs");
    assert_eq!(after.len(), 2);
    for (was, now) in before.iter().zip(&after) {
        assert_eq!(now.text, format!("{}!", was.text), "the change is in the saved file");
        assert!(
            (now.origin.x - was.origin.x).abs() < 0.01 && (now.origin.y - was.origin.y).abs() < 0.01,
            "the run moved: {:?} -> {:?}",
            was.origin,
            now.origin
        );
    }
}

/// **Spaces the producer drew by moving the pen, not by drawing a space.**
///
/// `[(Colour) -250 (Temperature)] TJ` spells `ColourTemperature` in codes —
/// no space code anywhere — while PDFium reports the run with the gaps as
/// spaces. Aligning the codes against the words failed outright, so retyping
/// refused with "that run's codes cannot be lined up with its text".
#[test]
fn a_run_whose_words_are_spaced_by_tj_gaps_is_retyped() {
    let Some(_) = skip_without_pdfium() else { return };
    let _lock = serial();

    let bytes = build_page(
        "/MediaBox [0 0 612 792]",
        "BT /F1 12 Tf 1 0 0 1 72 700 Tm \
         [(Colour) -250 (Temperature) -250 (6500) -250 (k)] TJ ET",
    );
    let mut doc = open(bytes);
    let before = doc.text_runs(0).expect("runs");
    assert_eq!(before.len(), 1);
    assert_eq!(before[0].text, "Colour Temperature 6500 k");

    let wanted = "Colour Temperature 6500 K";
    let (previous, _) = doc
        .set_text_run_styled(0, before[0].object, wanted, &TextStyle::default())
        .unwrap_or_else(|e| panic!("retyping the kerned run was refused: {e}"));
    assert_eq!(previous, "Colour Temperature 6500 k");

    let reopened = saved_and_reopened(&mut doc);
    let after = reopened.text_runs(0).expect("runs");
    assert_eq!(after.len(), 1, "still one run");
    assert_eq!(after[0].text, wanted, "the new words are in the saved file");
    assert!(
        (after[0].origin.x - before[0].origin.x).abs() < 0.01
            && (after[0].origin.y - before[0].origin.y).abs() < 0.01,
        "the run moved: {:?} -> {:?}",
        before[0].origin,
        after[0].origin
    );
}
