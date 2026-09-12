//! How many of a page's painted shapes also set a clip — the case that could
//! not be selected or moved.
use pdf_core::document::pdfium_doc::PdfiumDocument;
use pdf_core::document::Document;

fn main() {
    let path = std::env::args().nth(1).expect("a pdf path");
    let limit: usize = std::env::args().nth(2).and_then(|p| p.parse().ok()).unwrap_or(6);
    let password = std::env::var("PDF_PASSWORD").ok();
    let doc = PdfiumDocument::open_path(&path, password.as_deref()).expect("open");
    for page in 0..doc.page_count().min(limit) {
        let Ok(stream) = doc.page_stream(page) else { continue };
        let Ok(ops) = pdf_core::pdf::content::parse(&stream) else { continue };
        let (mut painted, mut painted_and_clip, mut open, mut clip) = (0, 0, false, false);
        let mut inside_clip_depth = 0usize;
        let mut levels = vec![false];
        let mut drawn_inside_a_clip = 0usize;
        for op in &ops {
            match op.operator.as_slice() {
                b"q" => levels.push(false),
                b"Q" => { if levels.len() > 1 { levels.pop(); } }
                b"m" | b"re" => { if !open { open = true; clip = false; } }
                b"W" | b"W*" => clip = true,
                b"S" | b"s" | b"f" | b"F" | b"f*" | b"B" | b"B*" | b"b" | b"b*" | b"n" => {
                    if open {
                        if op.operator.as_slice() != b"n" {
                            painted += 1;
                            if clip { painted_and_clip += 1; }
                            if levels.iter().any(|c| *c) { drawn_inside_a_clip += 1; }
                        }
                        if clip { if let Some(l) = levels.last_mut() { *l = true; } }
                        open = false; clip = false;
                    }
                }
                b"Do" | b"Tj" | b"TJ" => {
                    if levels.iter().any(|c| *c) { inside_clip_depth += 1; }
                }
                _ => {}
            }
        }
        println!(
            "  page {:>2}: {painted:>3} painted shapes, {painted_and_clip:>3} of them also clip; \
             {drawn_inside_a_clip:>3} shapes and {inside_clip_depth:>3} words/pictures drawn inside a clip",
            page + 1
        );
    }
}
