//! Everything the engine knows about one page: what it draws, what is locked,
//! and whether any lock badge stands over a picture that is still there.
use pdf_core::document::pdfium_doc::PdfiumDocument;
use pdf_core::document::{Document, DocumentMut};

fn main() {
    let path = std::env::args().nth(1).expect("a pdf path");
    let page: usize = std::env::args().nth(2).and_then(|p| p.parse().ok()).unwrap_or(0);
    let password = std::env::var("PDF_PASSWORD").ok();
    let doc = PdfiumDocument::open_path(&path, password.as_deref()).expect("open");
    println!("{} pages; page {}: {:?}", doc.page_count(), page + 1, doc.page_size(page).ok());

    println!("\n== locked items on this page ==");
    for item in doc.locked_items_on(page).unwrap_or_default() {
        println!("  {} {:?}  area={} STALE={}", item.id, item.rect, item.is_area, item.stale);
    }
    let all: usize = (0..doc.page_count()).map(|p| doc.locked_items_on(p).map(|i| i.len()).unwrap_or(0)).sum();
    let stale: usize = (0..doc.page_count()).map(|p| doc.locked_items_on(p).unwrap_or_default().iter().filter(|i| i.stale).count()).sum();
    println!("  (whole document: {all} locked items, {stale} stale)");

    println!("\n== what the page draws, bottom first ==");
    for d in doc.drawn_objects(page).unwrap_or_default() {
        println!(
            "  {:>3}{} {:<8} {:<34} {:>4.0},{:>4.0} {:>4.0}×{:<4.0}{}",
            d.object, "  ".repeat(d.depth), d.kind.describe(),
            d.label.chars().take(34).collect::<String>(),
            d.rect.left, d.rect.top, d.rect.right - d.rect.left, d.rect.bottom - d.rect.top,
            if d.movable { "" } else { "  (in group)" },
        );
    }

    println!("\n== annotations on this page ==");
    for a in doc.annotations(page).unwrap_or_default() {
        println!("  {:?}", a);
    }
}
