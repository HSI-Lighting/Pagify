//! Does adding a text layer change a single rendered pixel?
//!
//! The OCR plan makes this the acceptance test for the whole feature, and it is
//! the right one: it catches a render mode left at fill, a stray stroke colour,
//! a leaked clip, or a font that shifts the content stream — none of which are
//! visible by eye until someone prints the page.
//!
//! Also checks the second trap: running it twice must replace, not stack.
use pdf_core::document::pdfium_doc::PdfiumDocument;
use pdf_core::document::{Document, RecognisedWord, Rect, RenderRequest};
use pdf_core::render::{bitmap::PixelOrder, RenderTarget};

fn render(doc: &dyn Document, scale: f32) -> (u32, u32, Vec<u8>) {
    let page = doc.page(0).expect("page");
    let (w, h) = page.size().pixel_size(scale);
    let mut pixels = vec![0u8; w as usize * h as usize * 4];
    {
        let mut target =
            RenderTarget::new(w, h, w as usize * 4, PixelOrder::Rgba, &mut pixels).expect("target");
        page.render_into(
            &RenderRequest { scale, ..Default::default() },
            &mut target,
        )
        .expect("render");
    }
    (w, h, pixels)
}

fn words() -> Vec<RecognisedWord> {
    vec![
        RecognisedWord {
            text: "LUMINAIRE".into(),
            rect: Rect { left: 40.0, top: 80.0, right: 140.0, bottom: 96.0 },
            confidence: 0.99, char_confidence: Vec::new(),
        },
        RecognisedWord {
            text: "IP65".into(),
            rect: Rect { left: 40.0, top: 110.0, right: 80.0, bottom: 126.0 },
            confidence: 0.97, char_confidence: Vec::new(),
        },
    ]
}

fn main() {
    // A page with real ink on it, so a change is detectable at all.
    let source = std::env::args().nth(1).unwrap_or_else(|| "fixtures/text-lines.pdf".into());

    for scale in [1.0f32, 2.0, 4.1667] {
        let mut doc: Box<dyn Document> =
            Box::new(PdfiumDocument::open_path(&source, None).expect("open"));

        let (w, h, before) = render(&*doc, scale);
        doc.as_document_mut().unwrap().add_text_layer(0, &words()).expect("layer");
        let (w2, h2, after) = render(&*doc, scale);

        assert_eq!((w, h), (w2, h2), "the page changed size");
        let differing = before
            .chunks_exact(4)
            .zip(after.chunks_exact(4))
            .filter(|(a, b)| a != b)
            .count();

        let worst = before
            .iter()
            .zip(&after)
            .map(|(a, b)| (*a as i32 - *b as i32).abs())
            .max()
            .unwrap_or(0);

        println!(
            "{:>8.4} dpi-scale  {w}x{h}  differing pixels: {differing:>7}   worst channel delta: {worst}",
            scale * 72.0
        );
    }

    // The "run it twice" trap.
    let mut doc: Box<dyn Document> =
        Box::new(PdfiumDocument::open_path(&source, None).expect("open"));
    let before = doc.page(0).expect("page").text().expect("text");
    doc.as_document_mut().unwrap().add_text_layer(0, &words()).expect("first");
    doc.as_document_mut().unwrap().add_text_layer(0, &words()).expect("second");
    let after = doc.page(0).expect("page").text().expect("text");

    let count = |haystack: &str| haystack.matches("LUMINAIRE").count();
    println!(
        "\nrun twice: \"LUMINAIRE\" appears {} time(s) — {}",
        count(&after),
        if count(&after) > 1 { "STACKED (doubled search hits)" } else { "replaced" }
    );
    let _ = before;
}
