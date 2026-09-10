//! Independent check: does the pixel-identity test notice STACKED text layers?
use pdf_core::document::pdfium_doc::PdfiumDocument;
use pdf_core::document::{
    Annotation, Color, Document, Glyph, RecognisedWord, Rect, RenderRequest,
};
use pdf_core::render::{bitmap::PixelOrder, RenderTarget};

fn render(doc: &dyn Document, scale: f32) -> (u32, u32, Vec<u8>) {
    let page = doc.page(0).expect("page");
    let (w, h) = page.size().pixel_size(scale);
    let mut pixels = vec![0u8; w as usize * h as usize * 4];
    {
        let mut target =
            RenderTarget::new(w, h, w as usize * 4, PixelOrder::Rgba, &mut pixels).expect("target");
        page.render_into(&RenderRequest { scale, ..Default::default() }, &mut target)
            .expect("render");
    }
    (w, h, pixels)
}

fn words() -> Vec<RecognisedWord> {
    vec![
        RecognisedWord {
            text: "LUMINAIRE".into(),
            rect: Rect { left: 40.0, top: 80.0, right: 140.0, bottom: 96.0 },
            confidence: 0.99,
            char_confidence: Vec::new(),
        },
        RecognisedWord {
            text: "IP65".into(),
            rect: Rect { left: 40.0, top: 110.0, right: 80.0, bottom: 126.0 },
            confidence: 0.97,
            char_confidence: Vec::new(),
        },
    ]
}

/// Exactly what `add_text_layer` writes, but under a caller-chosen id so the
/// dedupe at the top of that function does not remove the previous layer.
/// This is what a "replace rather than stack" bug looks like in the file.
fn write_layer(doc: &mut dyn pdf_core::document::DocumentMut, id: i32) {
    for word in words() {
        let placed = word.placement().expect("placement");
        let glyphs = vec![Glyph {
            ch: placed.text.clone(),
            id: 0,
            x: placed.left,
            y: placed.baseline,
            radians: 0.0,
        }];
        let annotation = Annotation::Text {
            text: word.text.clone(),
            font: "Helvetica".into(),
            font_asset: None,
            size: placed.size,
            color: Color { r: 0, g: 0, b: 0, a: 0 },
            glyphs,
            id,
            restore: String::new(),
            frame: Vec::new(),
            frame_width: 0.0,
        };
        doc.add_annotation(0, &annotation).expect("write");
    }
}

fn main() {
    let source = std::env::args().nth(1).unwrap_or_else(|| "fixtures/text-lines.pdf".into());
    let layers: i32 = std::env::args().nth(2).and_then(|s| s.parse().ok()).unwrap_or(5);

    for scale in [1.0f32, 2.0, 4.1667] {
        let mut doc: Box<dyn Document> =
            Box::new(PdfiumDocument::open_path(&source, None).expect("open"));
        let (w, h, before) = render(&*doc, scale);
        for n in 0..layers {
            write_layer(doc.as_document_mut().unwrap(), 0x004F_4352 + n);
        }
        let (w2, h2, after) = render(&*doc, scale);
        assert_eq!((w, h), (w2, h2));

        let differing_bytes = before.iter().zip(&after).filter(|(a, b)| a != b).count();
        let differing_px = before
            .chunks_exact(4)
            .zip(after.chunks_exact(4))
            .filter(|(a, b)| a != b)
            .count();
        println!(
            "{layers} layers @ {:>8.4} dpi  {w}x{h}  differing bytes: {differing_bytes} / {}  (differing pixels: {differing_px})",
            scale * 72.0,
            before.len()
        );
    }

    // And what extraction sees.
    let mut doc: Box<dyn Document> =
        Box::new(PdfiumDocument::open_path(&source, None).expect("open"));
    for n in 0..layers {
        write_layer(doc.as_document_mut().unwrap(), 0x004F_4352 + n);
    }
    let text = doc.page(0).expect("page").text().expect("text");
    println!(
        "\nafter {layers} stacked layers: \"LUMINAIRE\" appears {} time(s), \"IP65\" {} time(s)",
        text.matches("LUMINAIRE").count(),
        text.matches("IP65").count()
    );
    println!("--- extracted text ---\n{text}\n--- end ---");
    let objs = doc.page(0).expect("page").characters().expect("chars");
    println!("char count after stacking: {}", objs.text.chars().count());
}
