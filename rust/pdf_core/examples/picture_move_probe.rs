//! Move every picture on a page and report where each one ended up.
use pdf_core::document::pdfium_doc::PdfiumDocument;
use pdf_core::document::{Document, DocumentMut, Point};

fn main() {
    let path = std::env::args().nth(1).expect("a pdf path");
    let page: usize = std::env::args().nth(2).and_then(|p| p.parse().ok()).unwrap_or(0);
    let by = Point { x: 20.0, y: 12.0 };

    let doc = PdfiumDocument::open_path(&path, None).expect("open");
    let pictures = doc.images_on(page).expect("images");
    drop(doc);
    println!("{} picture(s) on page {}", pictures.len(), page + 1);

    for picture in &pictures {
        let mut doc = PdfiumDocument::open_path(&path, None).expect("open");
        let before = doc.images_on(page).expect("images");
        let words: Vec<String> = doc.text_runs(page).unwrap_or_default()
            .iter().map(|r| r.text.trim().to_string()).collect();

        match doc.move_object(page, picture.object, by) {
            Err(e) => { println!("  object {}: refused — {e}", picture.object); continue }
            Ok(()) => {}
        }
        let after = doc.images_on(page).expect("images");
        let Some(now) = after.iter().find(|i| i.object == picture.object) else {
            println!("  object {}: GONE from the page", picture.object);
            continue;
        };
        let (dx, dy) = (now.rect.left - picture.rect.left, now.rect.top - picture.rect.top);
        let ok = (dx - by.x).abs() < 0.5 && (dy - by.y).abs() < 0.5;
        println!(
            "  object {}: {:?} -> {:?}   moved ({dx:+.1}, {dy:+.1}) {}",
            picture.object, picture.rect, now.rect,
            if ok { "exact" } else { "*** WRONG ***" },
        );
        // Did any other picture move?
        for was in &before {
            if was.object == picture.object { continue }
            if let Some(is) = after.iter().find(|i| i.object == was.object) {
                if (is.rect.left - was.rect.left).abs() > 0.5 || (is.rect.top - was.rect.top).abs() > 0.5 {
                    println!("      and it dragged object {} along with it", was.object);
                }
            } else {
                println!("      and object {} vanished", was.object);
            }
        }
        let now_words: Vec<String> = doc.text_runs(page).unwrap_or_default()
            .iter().map(|r| r.text.trim().to_string()).collect();
        if now_words != words {
            println!("      and the page's words changed: {words:?} -> {now_words:?}");
        }
    }
}
