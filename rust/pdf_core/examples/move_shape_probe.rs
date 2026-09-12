//! Does a drawn shape move where it is sent, and leave the page alone?
use pdf_core::document::pdfium_doc::PdfiumDocument;
use pdf_core::document::{Document, DocumentMut, DrawnKind, Point};

fn main() {
    let path = std::env::args().nth(1).expect("a pdf path");
    let limit: usize = std::env::args().nth(2).and_then(|p| p.parse().ok()).unwrap_or(4);
    let by = Point { x: 20.0, y: 12.0 };
    let password = std::env::var("PDF_PASSWORD").ok();
    let open = |p: &str| PdfiumDocument::open_path(p, password.as_deref());

    let pages = open(&path).expect("open").page_count().min(limit);
    let (mut exact, mut wrong, mut refused, mut disturbed) = (0, 0, 0, 0);
    let mut reasons: Vec<String> = Vec::new();

    for page in 0..pages {
        let doc = open(&path).expect("open");
        let shapes: Vec<usize> = doc
            .drawn_objects(page)
            .unwrap_or_default()
            .into_iter()
            .filter(|d| d.depth == 0 && d.kind == DrawnKind::Shape)
            .map(|d| d.object)
            .collect();
        drop(doc);

        for object in shapes.iter().take(10) {
            let doc = open(&path).expect("open");
            let before = doc.drawn_objects(page).unwrap_or_default();
            let was = before.iter().find(|d| d.object == *object).cloned();
            drop(doc);
            let Some(was) = was else { continue };

            let mut doc = open(&path).expect("open");
            if let Err(e) = doc.move_object(page, *object, by) {
                refused += 1;
                let r = e.to_string();
                if !reasons.contains(&r) { reasons.push(r) }
                continue;
            }
            let after = doc.drawn_objects(page).unwrap_or_default();
            let Some(now) = after.iter().find(|d| d.object == *object) else {
                wrong += 1;
                continue;
            };
            let (dx, dy) = (now.rect.left - was.rect.left, now.rect.top - was.rect.top);
            if (dx - by.x).abs() < 0.5 && (dy - by.y).abs() < 0.5 {
                exact += 1;
            } else {
                wrong += 1;
                let r = format!("moved ({dx:+.1}, {dy:+.1}) when asked for ({:+.1}, {:+.1})", by.x, by.y);
                if !reasons.contains(&r) { reasons.push(r) }
                continue;
            }
            let mut moved_others = 0;
            for other in &before {
                if other.object == *object { continue }
                if let Some(is) = after.iter().find(|d| d.object == other.object) {
                    if (is.rect.left - other.rect.left).abs() > 0.5
                        || (is.rect.top - other.rect.top).abs() > 0.5
                    {
                        moved_others += 1;
                    }
                }
            }
            if moved_others > 0 { disturbed += 1 }
        }
    }
    println!("{path}");
    println!("  shapes moved exactly  : {exact}");
    println!("  moved wrongly         : {wrong}");
    println!("  refused               : {refused}");
    println!("  disturbed a neighbour : {disturbed}");
    for r in reasons.iter().take(5) { println!("     — {r}") }
}
