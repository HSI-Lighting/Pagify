//! What does extraction see after a text layer is written?
use pdf_core::document::pdfium_doc::PdfiumDocument;
use pdf_core::document::{Document, RecognisedWord, Rect};

fn main() {
    let path = std::env::args().nth(1).unwrap_or_else(|| "fixtures/single-page.pdf".into());
    let mut doc: Box<dyn Document> =
        Box::new(PdfiumDocument::open_path(&path, None).expect("open"));

    let words = vec![RecognisedWord {
        text: "LUMINAIRE".into(),
        rect: Rect { left: 40.0, top: 80.0, right: 140.0, bottom: 96.0 },
        confidence: 1.0, char_confidence: Vec::new(),
    }];
    let asked = words[0].placement().expect("a placement");
    println!(
        "asked: {:?} at x={:.2} baseline={:.2} size={:.2}",
        asked.text, asked.left, asked.baseline, asked.size
    );

    doc.as_document_mut().unwrap().add_text_layer(0, &words).expect("write");

    let mut bytes = Vec::new();
    doc.as_document_mut().unwrap().save_full_copy(&mut bytes).expect("save");
    let reopened = PdfiumDocument::open_bytes(bytes, None).expect("reopen");

    let page = reopened.page(0).expect("page");
    let chars = page.characters().expect("characters");
    println!("extracted {:?}", chars.text);
    for (i, ch) in chars.text.chars().enumerate() {
        let b = &chars.boxes[i * 4..i * 4 + 4];
        println!(
            "  {i:>2} {ch:?}  l={:>7.2} t={:>7.2} r={:>7.2} b={:>7.2}  width={:>5.2}",
            b[0], b[1], b[2], b[3], b[2] - b[0]
        );
    }
}
