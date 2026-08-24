//! Does a caption's whole life leave the page intact?
//!
//! ```text
//! PAGIFY_PDFIUM_LIB=/tmp/pdfw/bin/pdfium.dll \
//!   cargo run --example text_lifecycle_probe -- <in.pdf> <scratch-dir>
//! ```
//!
//! Written after a document went blank on the device: one A4 page, no objects,
//! no text, 3.8 MB of it. The app had done nothing exotic — placed a caption and
//! pressed Save, several times over. Somewhere in write / save / read / erase /
//! save the page's own content stopped being there.
//!
//! So this walks the same round on a real page and counts what is left after
//! every step. The count is the point: a step that quietly takes the document's
//! own content with it looks exactly like a step that works, right up until the
//! page is empty.

use pdf_core::command::{Command, CommandHistory};
use pdf_core::document::pdfium_doc::PdfiumDocument;
use pdf_core::document::{Annotation, Color, Document, Glyph, Point};

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let source = args.get(1).expect("usage: text_lifecycle_probe <in.pdf> <scratch>");
    let scratch = args.get(2).expect("usage: text_lifecycle_probe <in.pdf> <scratch>");

    // The engine binds PDFium itself from this variable.
    std::env::var("PAGIFY_PDFIUM_LIB").expect("set PAGIFY_PDFIUM_LIB");

    let mut doc: Box<dyn Document> =
        Box::new(PdfiumDocument::open_path(source, None).expect("open"));
    report("opened", &doc);

    let ink = Color { r: 200, g: 30, b: 30, a: 255 };

    // Four rounds, because the document that went blank had been saved over and
    // over. One round of anything looks fine; the question is what stacking does.
    for round in 1..=4 {
        let glyphs: Vec<Glyph> = "Caption"
            .chars()
            .enumerate()
            .map(|(at, ch)| Glyph {
                id: 0,
                ch: ch.to_string(),
                x: 100.0 + at as f32 * 12.0,
                y: 200.0 + round as f32 * 30.0,
                radians: 0.0,
            })
            .collect();
        let top = 180.0 + round as f32 * 30.0;
        let frame = vec![
            Point { x: 90.0, y: top },
            Point { x: 200.0, y: top },
            Point { x: 200.0, y: top + 35.0 },
            Point { x: 90.0, y: top + 35.0 },
        ];
        let build = |restore: String| Annotation::Text {
            font_asset: None,
            text: "Caption".into(),
            font: "Helvetica".into(),
            size: 18.0,
            color: ink,
            glyphs: glyphs.clone(),
            id: round,
            restore,
            frame: frame.clone(),
            frame_width: 1.4,
        };
        let blob = serde_json::to_string(&build(String::new())).expect("encode");

        let mut history = CommandHistory::default();
        history
            .execute(
                Command::AddAnnotation {
                    page_index: 0,
                    annotation: build(blob),
                },
                doc.as_document_mut().expect("mutable"),
            )
            .expect("write the caption");

        doc = save_and_reopen(doc, &format!("{scratch}/life-{round}.pdf"));
        let marks = doc.text_marks(0).expect("read the text marks");
        report(&format!("round {round}: after save, {} marks", marks.len()), &doc);
    }

    // And now erase one, as the eraser does.
    let mut after = CommandHistory::default();
    after
        .execute(
            Command::RemoveText { page_index: 0, id: 2 },
            doc.as_document_mut().expect("mutable"),
        )
        .expect("erase the caption");
    let doc = save_and_reopen(doc, &format!("{scratch}/life-erased.pdf"));
    report("after erasing one and saving", &doc);
}

/// Objects and text on page 0 — the two numbers that say whether the page is
/// still a page.
fn report(step: &str, doc: &Box<dyn Document>) {
    let text = doc
        .page(0)
        .and_then(|p| p.text())
        .map(|t| t.chars().filter(|c| !c.is_whitespace()).count())
        .unwrap_or(0);
    println!("{step}: {text} characters of text on page 0");
}

fn save_and_reopen(mut doc: Box<dyn Document>, to: &str) -> Box<dyn Document> {
    let mut bytes = Vec::new();
    doc.as_document_mut()
        .expect("mutable")
        .save_incremental(&mut bytes)
        .expect("save");
    std::fs::write(to, &bytes).expect("write");
    println!("   wrote {} bytes to {to}", bytes.len());
    Box::new(PdfiumDocument::open_bytes(bytes, None).expect("reopen"))
}
