//! A run's words as the page **draws** them, not as PDFium's text layer folds them.
//!
//! PDFium reads a run of space glyphs back as one space: a line padded with
//! twenty reads as padded with one, in both a run's own text and the page's
//! character list. The editor built its buffer from that text, so a line somebody
//! had padded showed one space, and applying an edit from the buffer took the
//! other nineteen off the page. `PdfiumDocument::exact_run_texts` reads the
//! content stream's own codes and puts the spaces back.
//!
//! ```text
//! PAGIFY_PDFIUM_LIB=<pdfium> cargo test --test exact_text -- --nocapture
//! ```

mod harness;
use harness::{serial, skip_without_pdfium};

use pdf_core::document::pdfium_doc::PdfiumDocument;
use pdf_core::document::{Document, DocumentMut, TextStyle};

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

/// **The spaces are in the file, and the exact text has them.** Whatever PDFium
/// itself reports for the run is printed, so the fold is visible when it happens.
#[test]
fn a_run_padded_with_spaces_reads_back_with_all_of_them() {
    let Some(_) = skip_without_pdfium() else { return };
    let _lock = serial();
    let padded = format!("Driver Efficiency:{}>95%", " ".repeat(21));
    let doc = open(&format!("BT /F1 12 Tf 72 700 Td [({padded})] TJ ET"));
    let run = doc.text_runs(0).unwrap().into_iter().next().expect("the run");
    println!("PDFium reads {:?}", run.text);
    let exact = doc.exact_run_texts(0, &[run.object]).expect("exact");
    println!("the stream reads {:?}", exact[&run.object]);
    assert_eq!(exact[&run.object].trim_end(), padded);
    // Everything else of PDFium's text is kept: the same words, any trailing space of its own.
    assert_eq!(exact[&run.object].trim_end().replace(' ', ""), run.text.trim_end().replace(' ', ""));
}

/// **Spaces drawn as pieces of their own** — `( )-106( )-106` — are spaces too.
#[test]
fn spaces_drawn_one_piece_each_are_counted() {
    let Some(_) = skip_without_pdfium() else { return };
    let _lock = serial();
    let doc = open("BT /F1 12 Tf 72 700 Td [( )-106( )-106( )-106(1-)16(10)-9(v )-106(/ )-106(DMX)] TJ ET");
    let run = doc.text_runs(0).unwrap().into_iter().next().expect("the run");
    let exact = doc.exact_run_texts(0, &[run.object]).expect("exact");
    assert_eq!(exact[&run.object].trim_end(), "   1-10v / DMX", "PDFium read {:?}", run.text);
}

/// A run with no extra spaces is exactly what PDFium said, and an object that is
/// not asked for is not answered.
#[test]
fn an_ordinary_run_is_unchanged() {
    let Some(_) = skip_without_pdfium() else { return };
    let _lock = serial();
    let doc = open("BT /F1 12 Tf 72 700 Td (Just words here) Tj ET BT /F1 12 Tf 72 650 Td (And more) Tj ET");
    let runs = doc.text_runs(0).unwrap();
    let exact = doc.exact_run_texts(0, &[runs[0].object]).expect("exact");
    assert_eq!(exact.len(), 1);
    assert_eq!(exact[&runs[0].object], runs[0].text);
}

/// **The reported case, on the real file**: a line retyped with twenty-one spaces
/// reads back with one from PDFium and twenty-one from the stream.
#[test]
fn a_line_retyped_with_typed_spaces_reads_back_with_them() {
    let Some(_) = skip_without_pdfium() else { return };
    let path = r"C:\Users\hsili\Desktop\test pdf for pgify\TECHNICAL DATASHEET Q-2075-REV.pdf";
    if !std::path::Path::new(path).exists() {
        eprintln!("skipped: the datasheet is not at {path}");
        return;
    }
    let _lock = serial();
    let mut doc = PdfiumDocument::open_path(path, None).expect("open");
    let line = doc.text_runs(0).unwrap().into_iter().find(|r| r.text.contains("Driver Efficiency")).expect("line");
    let typed = format!("Driver Efficiency:{}>95%", " ".repeat(21));
    doc.set_text_run_styled(0, line.object, &typed, &TextStyle::default()).expect("retype");
    let again = doc.text_runs(0).unwrap().into_iter().find(|r| r.text.contains("Driver Efficiency")).expect("line");
    println!("PDFium reads {:?}", again.text);
    let exact = doc.exact_run_texts(0, &[again.object]).expect("exact");
    println!("the stream reads {:?}", exact[&again.object]);
    assert_eq!(exact[&again.object].trim_end(), typed);
}
