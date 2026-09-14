//! Survey every page of a document for words drawn through forms, and say
//! what a redaction over each page's text would do about them.
//!
//! ```text
//! PDF_PASSWORD=… PAGIFY_PDFIUM_LIB=… cargo run --release --example form_survey_probe -- <pdf> [first] [count]
//! ```
//!
//! Read-only: previews, never applies.
use pdf_core::document::pdfium_doc::PdfiumDocument;
use pdf_core::document::{Document, DocumentMut, Redaction, Uncleared};

fn main() {
    let path = std::env::args().nth(1).expect("a pdf path");
    let first: usize = std::env::args().nth(2).and_then(|n| n.parse().ok()).unwrap_or(0);
    let count: usize = std::env::args().nth(3).and_then(|n| n.parse().ok()).unwrap_or(5);
    let password = std::env::var("PDF_PASSWORD").ok();
    let mut doc = PdfiumDocument::open_path(&path, password.as_deref()).expect("open");
    let pages = doc.page_count();
    let (mut cuttable, mut refused, mut shared) = (0usize, 0usize, 0usize);
    for page in first..(first + count).min(pages) {
        let runs = doc.text_runs(page).unwrap_or_default();
        let chars = doc.page(page).expect("page").characters().expect("chars");
        // The words drawn through forms on this page, and a box over the
        // first of them — the case this probe exists for. A mid-page box
        // where there are none.
        let drawn = doc.drawn_objects(page).unwrap_or_default();
        let nested: Vec<_> = drawn
            .iter()
            .filter(|d| d.depth > 0 && d.kind == pdf_core::document::DrawnKind::Words)
            .collect();
        let area = match nested.first() {
            Some(first) => {
                let r = first.rect;
                pdf_core::document::Rect {
                    left: r.left + 2.0,
                    top: r.top + 1.0,
                    right: (r.left + (r.right - r.left) * 0.6).max(r.left + 4.0),
                    bottom: r.bottom - 1.0,
                }
            }
            None => pdf_core::document::Rect { left: 60.0, top: 200.0, right: 400.0, bottom: 260.0 },
        };
        print!("p{}: {} nested text objects; ", page + 1, nested.len());
        let report = match doc.preview_redaction(&Redaction::new(page, area), None) {
            Ok(r) => r,
            Err(e) => {
                println!("p{}: refused outright — {e}", page + 1);
                continue;
            }
        };
        let forms = report.uncleared.iter().filter(|u| matches!(u, Uncleared::Form { .. })).count();
        let shared_here = report.uncleared.iter().filter(|u| matches!(u, Uncleared::SharedForm { .. })).count();
        println!(
            "p{}: {} chars on page, {} page-level runs; redaction of a mid-page box: {} chars, {} form blockers, {} shared forms, spilled {}",
            page + 1, chars.text.chars().count(), runs.len(), report.characters, forms, shared_here, report.spilled.len()
        );
        cuttable += usize::from(report.characters > 0 && forms == 0);
        refused += usize::from(forms > 0);
        shared += shared_here;
    }
    println!("cuttable pages {cuttable}, pages with form blockers {refused}, shared forms {shared}");
}
