//! An edit to one page must never change what another page draws.
//!
//! Reported from use (2026-11-16, WhatsApp screenshots of a datasheet): the
//! CAMINO page showed the TENDENCIA page's words over its own, with letters
//! missing. Reproduced exactly by drawing the TENDENCIA page's content stream
//! through the CAMINO page's resources (its fonts are other subsets, hence the
//! missing letters). How a file gets that way: Save for sharing (removed
//! 2026-11-17) wrote one `q` stream and one `Q` stream for the whole file and
//! put them in the content of *every* page it baked a picture into, and every
//! byte-level edit then wrote the edited page's whole content into the page's
//! **first** content stream — the shared `q` — and emptied the others, the
//! shared `Q` among them. Such files are still out there; an edit to one of
//! them must stay on its own page.
//!
//! ```text
//! PAGIFY_PDFIUM_LIB=<pdfium> cargo test --release --test shared_content_streams
//! ```

mod harness;
use harness::{serial, skip_without_pdfium};

use pdf_core::document::pdfium_doc::PdfiumDocument;
use pdf_core::document::{Annotation, Color, Document, DocumentMut, Glyph, Rect};

const ONE: &str = "BT /F1 24 Tf 72 700 Td (PAGEONE) Tj ET";
const TWO: &str = "BT /F1 24 Tf 72 700 Td (PAGETWO) Tj ET";

/// The words a page shows, joined.
fn words(doc: &PdfiumDocument, page: usize) -> String {
    doc.text_runs(page).expect("runs").iter().map(|r| r.text.trim().to_string()).collect::<Vec<_>>().join(" ")
}

/// Two pages, each with its own word, whose content is wrapped in the **same**
/// `q` stream (object 7) and `Q` stream (object 8) — what Save for sharing
/// wrote before 2026-11-16, and what such files still hold. Pagify no longer
/// writes one (one pair per page, then Flatten replaced the copy), but it
/// still opens them.
fn shared_copy() -> PdfiumDocument {
    let stream = |data: &str| format!("<< /Length {} >>
stream
{data}
endstream", data.len());
    let page = |own: u32| {
        format!("<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Contents [7 0 R {own} 0 R 8 0 R] /Resources << /Font << /F1 9 0 R >> >> >>")
    };
    let objects = [
        "<< /Type /Catalog /Pages 2 0 R >>".to_string(),
        "<< /Type /Pages /Kids [3 0 R 4 0 R] /Count 2 >>".to_string(),
        page(5),
        page(6),
        stream(ONE),
        stream(TWO),
        stream("q"),
        stream("Q"),
        "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica /Encoding /WinAnsiEncoding >>".to_string(),
    ];
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
    let xref = out.len();
    out.extend_from_slice(format!("xref
0 {}
0000000000 65535 f 
", objects.len() + 1).as_bytes());
    for offset in offsets {
        out.extend_from_slice(format!("{offset:010} 00000 n 
").as_bytes());
    }
    out.extend_from_slice(format!("trailer
<< /Size {} /Root 1 0 R >>
startxref
{xref}
%%EOF
", objects.len() + 1).as_bytes());
    PdfiumDocument::open_bytes(out, None).expect("the shared file opens")
}

/// **The reported fault.** Whiting out part of page 1 of a shared copy must
/// leave page 2 showing its own word and nothing of page 1's.
#[test]
fn whiting_out_one_page_of_a_shared_copy_leaves_the_other_page_alone() {
    let Some(_) = skip_without_pdfium() else { return };
    let _lock = serial();

    let mut flat = shared_copy();
    assert_eq!(words(&flat, 1), "PAGETWO", "setup: page 2 before the edit");

    let white = Color { r: 255, g: 255, b: 255, a: 255 };
    flat.whiteout(0, Rect { left: 400.0, top: 500.0, right: 450.0, bottom: 520.0 }, white)
        .expect("white out a corner of page 1");

    assert_eq!(words(&flat, 0), "PAGEONE", "page 1 keeps its word");
    assert_eq!(words(&flat, 1), "PAGETWO", "page 2 shows page 1's content as well as its own");

    let mut saved = Vec::new();
    flat.save_full_copy(&mut saved).expect("save");
    let reopened = PdfiumDocument::open_bytes(saved, None).expect("reopen");
    assert_eq!(words(&reopened, 1), "PAGETWO", "after a save, page 2 shows page 1's content");
}

/// The same through the edits people make most — moving a word, then taking it
/// off — which share the write with the whiteout.
#[test]
fn moving_or_removing_on_one_page_of_a_shared_copy_leaves_the_other_page_alone() {
    let Some(_) = skip_without_pdfium() else { return };
    let _lock = serial();

    let mut flat = shared_copy();
    let word = flat.text_runs(0).expect("runs").iter().find(|r| r.text.contains("PAGEONE")).expect("page 1's word").object;
    flat.move_object(0, word, pdf_core::document::Point { x: 10.0, y: 0.0 }).expect("move page 1's word");
    assert_eq!(words(&flat, 1), "PAGETWO", "a move on page 1 put page 1's content on page 2");

    let word = flat.text_runs(0).expect("runs").iter().find(|r| r.text.contains("PAGEONE")).expect("page 1's word").object;
    flat.remove_objects(0, &[word]).expect("remove page 1's word");
    assert_eq!(words(&flat, 0), "", "page 1's word is gone");
    assert_eq!(words(&flat, 1), "PAGETWO", "a removal on page 1 changed page 2");
}

/// **The PDFium regeneration path.** Words are written as page objects and
/// committed with `FPDFPage_GenerateContent`, which may re-emit the page's
/// content in place — and on a shared copy the page's stream objects are also
/// the other page's. Page 2 must still show only its own word, in memory and
/// after the file is written.
#[test]
fn writing_text_on_one_page_of_a_shared_copy_leaves_the_other_page_alone() {
    let Some(_) = skip_without_pdfium() else { return };
    let _lock = serial();

    let mut flat = shared_copy();
    let word = Annotation::Text {
        text: "A".into(),
        font: "Helvetica".into(),
        font_asset: None,
        size: 24.0,
        color: Color { r: 0, g: 0, b: 0, a: 255 },
        glyphs: vec![Glyph { id: 0, ch: "A".into(), x: 100.0, y: 200.0, radians: 0.0 }],
        id: 0,
        restore: String::new(),
        frame: Vec::new(),
        frame_width: 0.0,
        scale_x: 1.0,
        more: Vec::new(),
        rules: Vec::new(),
    };
    flat.add_annotation(0, &word).expect("write a word on page 1");

    assert_eq!(words(&flat, 1), "PAGETWO", "a word written on page 1 put page 1's content on page 2");

    let mut saved = Vec::new();
    flat.save_full_copy(&mut saved).expect("save");
    let reopened = PdfiumDocument::open_bytes(saved, None).expect("reopen");
    assert_eq!(words(&reopened, 1), "PAGETWO", "after a save, page 2 shows page 1's content");
}
