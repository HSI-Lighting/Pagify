//! Which stream operators draw a given run, and what else they draw.
use pdf_core::document::pdfium_doc::PdfiumDocument;
use pdf_core::document::Document;

fn main() {
    let path = std::env::args().nth(1).expect("a pdf path");
    let page: usize = std::env::args().nth(2).and_then(|p| p.parse().ok()).unwrap_or(0);
    let object: usize = std::env::args().nth(3).and_then(|p| p.parse().ok()).unwrap_or(0);

    let doc = PdfiumDocument::open_path(&path, None).expect("open");
    let runs = doc.text_runs(page).expect("runs");
    let height = doc.page_size(page).expect("size").height_pt;
    let stream = doc.page_stream(page).expect("stream");
    let operations = pdf_core::pdf::content::parse(&stream).expect("parse");
    let placed = pdf_core::pdf::content::placed(&operations);

    for run in runs.iter().filter(|r| (r.object as i64 - object as i64).abs() <= 2) {
        let (wx, wy) = (run.rect.left, height - run.rect.bottom);
        let near = placed
            .iter()
            .map(|p| (p, ((p.origin.x - wx).powi(2) + (p.origin.y - wy).powi(2)).sqrt()))
            .min_by(|a, b| a.1.total_cmp(&b.1));
        match near {
            Some((p, d)) => println!(
                "object {:>3} {:?}\n      rect x {:.1}..{:.1}  -> operation {} line {} at x {:.1} (distance {:.2})\n      operator {:?}",
                run.object,
                run.text.trim().chars().take(24).collect::<String>(),
                run.rect.left, run.rect.right,
                p.origin.operation, p.line, p.origin.x, d,
                String::from_utf8_lossy(&stream[operations[p.origin.operation].span.clone()])
                    .chars().take(70).collect::<String>(),
            ),
            None => println!("object {}: nothing near", run.object),
        }
    }
}
