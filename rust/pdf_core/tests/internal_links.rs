//! Links that go to another page of the same document: a contents list, a
//! "back to the index". Read from a file built here by hand, so nothing but the
//! file's own bytes says where each link goes.
//!
//! ```text
//! PAGIFY_PDFIUM_LIB=<pdfium> cargo test --test internal_links
//! ```

mod harness;
use harness::{serial, skip_without_pdfium};

use pdf_core::document::pdfium_doc::PdfiumDocument;
use pdf_core::document::Document;

/// Three pages, 200 x 300. Page 1 carries four links:
///   - a `/Dest` to page 3,
///   - a `/GoTo` action to page 2,
///   - a `/URI` (not an internal link),
///   - a `/Dest` to a page that does not exist.
/// Page 2 carries one `/Dest` back to page 1.
fn linked_pdf() -> Vec<u8> {
    let page = |content: &str, annots: &str| {
        format!(
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 200 300] /Contents {content} {annots} >>"
        )
    };
    let objects = [
        "<< /Type /Catalog /Pages 2 0 R >>".to_string(),
        "<< /Type /Pages /Kids [3 0 R 4 0 R 5 0 R] /Count 3 >>".to_string(),
        page("6 0 R", "/Annots [7 0 R 8 0 R 9 0 R 10 0 R]"),
        page("6 0 R", "/Annots [11 0 R]"),
        page("6 0 R", ""),
        "<< /Length 0 >>\nstream\n\nendstream".to_string(),
        "<< /Type /Annot /Subtype /Link /Rect [10 250 60 290] /Border [0 0 0] /Dest [5 0 R /XYZ 0 300 0] >>"
            .to_string(),
        "<< /Type /Annot /Subtype /Link /Rect [10 200 60 240] /Border [0 0 0] \
         /A << /S /GoTo /D [4 0 R /Fit] >> >>"
            .to_string(),
        "<< /Type /Annot /Subtype /Link /Rect [10 150 60 190] /Border [0 0 0] \
         /A << /S /URI /URI (https://example.com) >> >>"
            .to_string(),
        "<< /Type /Annot /Subtype /Link /Rect [10 100 60 140] /Border [0 0 0] /Dest [99 0 R /Fit] >>"
            .to_string(),
        "<< /Type /Annot /Subtype /Link /Rect [10 250 60 290] /Border [0 0 0] /Dest [3 0 R /Fit] >>"
            .to_string(),
    ];
    let mut out = b"%PDF-1.7\n".to_vec();
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
        format!("trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref_at}\n%%EOF\n", objects.len() + 1)
            .as_bytes(),
    );
    out
}

#[test]
fn links_to_other_pages_are_read_with_where_they_are_and_where_they_go() {
    let Some(()) = skip_without_pdfium() else { return };
    let _lock = serial();
    let doc = PdfiumDocument::open_bytes(linked_pdf(), None).expect("open");

    let links = doc.internal_links(0).expect("links");
    let mut found: Vec<(usize, i32, i32)> = links
        .iter()
        .map(|l| (l.page, l.rect.left.round() as i32, l.rect.top.round() as i32))
        .collect();
    found.sort();
    // A `/Dest` to page 3 at the top of the page (rect top 300-290 = 10), and a
    // `/GoTo` to page 2 below it (300-240 = 60). The `/URI` and the link to a
    // page that is not there are not reported.
    assert_eq!(found, [(1, 10, 60), (2, 10, 10)], "{links:?}");

    let back = doc.internal_links(1).expect("links");
    assert_eq!(back.len(), 1, "{back:?}");
    assert_eq!(back[0].page, 0);

    assert!(doc.internal_links(2).expect("links").is_empty(), "a page with no links");
}

#[test]
fn a_page_outside_the_document_is_an_error_not_an_empty_answer() {
    let Some(()) = skip_without_pdfium() else { return };
    let _lock = serial();
    let doc = PdfiumDocument::open_bytes(linked_pdf(), None).expect("open");
    assert!(doc.internal_links(3).is_err());
}
