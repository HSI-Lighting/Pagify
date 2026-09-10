//! Moving a picture: does the page around it survive?
use pdf_core::document::{Document, DocumentMut, Point};
use pdf_core::document::pdfium_doc::PdfiumDocument;

fn main() {
    let path = std::env::args().nth(1).expect("a pdf path");
    let page: usize = std::env::args().nth(2).and_then(|p| p.parse().ok()).unwrap_or(0);

    let doc = PdfiumDocument::open_path(&path, None).expect("open");
    let images = doc.images_on(page).expect("images");
    let before: Vec<String> = doc.text_runs(page).expect("runs")
        .iter().map(|r| r.text.trim().to_string()).collect();
    println!("page {}: {} image(s), {} run(s)", page + 1, images.len(), before.len());
    let Some(picture) = images.first().cloned() else { return };
    println!("moving image at object {} — {:?}", picture.object, picture.rect);
    drop(doc);

    let mut doc = PdfiumDocument::open_path(&path, None).expect("open");
    doc.move_object(page, picture.object, Point { x: 20.0, y: 12.0 }).expect("move");

    let after: Vec<String> = doc.text_runs(page).expect("runs")
        .iter().map(|r| r.text.trim().to_string()).collect();
    let now = doc.images_on(page).expect("images");
    let moved = now.iter().find(|i| i.object == picture.object);
    if let Some(m) = moved {
        println!(
            "the picture moved by ({:+.2}, {:+.2})",
            m.rect.left - picture.rect.left,
            m.rect.top - picture.rect.top
        );
    }

    let mut a = before.clone();
    let mut b = after.clone();
    a.sort();
    b.sort();
    println!(
        "the page's words are {}",
        if a == b { "the same" } else { "DIFFERENT" }
    );
    for (x, y) in a.iter().zip(b.iter()).filter(|(x, y)| x != y).take(3) {
        println!("   was {x:?}\n   now {y:?}");
    }
    // And the other pictures.
    let mut shifted = 0;
    for was in &images {
        if was.object == picture.object { continue }
        if let Some(is) = now.iter().find(|i| i.object == was.object) {
            if (is.rect.left - was.rect.left).abs() > 0.1 || (is.rect.top - was.rect.top).abs() > 0.1 {
                shifted += 1;
            }
        }
    }
    println!("{shifted} other picture(s) moved");
}
