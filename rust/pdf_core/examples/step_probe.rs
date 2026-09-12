//! Does moving something one step through the drawing order put it exactly one
//! step over, and leave everything else in its order?
use pdf_core::document::pdfium_doc::PdfiumDocument;
use pdf_core::document::{Document, DocumentMut, Stacking};

fn main() {
    let path = std::env::args().nth(1).expect("a pdf path");
    let page: usize = std::env::args().nth(2).and_then(|p| p.parse().ok()).unwrap_or(0);
    let password = std::env::var("PDF_PASSWORD").ok();
    let open = |p: &str| PdfiumDocument::open_path(p, password.as_deref());

    let doc = open(&path).expect("open");
    let before = doc.drawn_objects(page).expect("objects");
    let names = |doc: &PdfiumDocument| -> Vec<String> {
        doc.drawn_objects(page).unwrap_or_default().iter()
            .filter(|d| d.depth == 0)
            // Identity by kind and position, not label — a page can draw two
            // identical panels, and a label cannot tell them apart.
            .map(|d| format!("{}:{}@{:.0},{:.0}", d.kind.describe(), d.label.chars().take(14).collect::<String>(), d.rect.left, d.rect.top))
            .collect()
    };
    let was = names(&doc);
    drop(doc);

    for (label, to) in [("up", Stacking::Up), ("down", Stacking::Down)] {
        let (mut exact, mut refused, mut wrong) = (0, 0, 0);
        let mut reasons: Vec<String> = Vec::new();
        for (position, target) in before.iter().filter(|d| d.depth == 0).enumerate() {
            let mut doc = open(&path).expect("open");
            match doc.restack(page, target.object, to) {
                Err(e) => {
                    refused += 1;
                    let r = format!("{}: {e}", target.label.chars().take(20).collect::<String>());
                    if !reasons.contains(&r) && reasons.len() < 6 { reasons.push(r) }
                    continue;
                }
                Ok(()) => {}
            }
            let now = names(&doc);
            // The expected order: swap with the neighbour.
            let mut expected = was.clone();
            let swap_with = match to {
                Stacking::Up => position + 1,
                _ => position.wrapping_sub(1),
            };
            if swap_with < expected.len() {
                expected.swap(position, swap_with);
            }
            if now == expected {
                exact += 1;
            } else {
                // Landing out of a clip scope, or past a text object, moves it
                // further than one step. Legitimate, but reported.
                let me = &was[position];
                let mine_now = now.iter().position(|n| n == me);
                let mine_was = position;
                let direction_ok = match (to, mine_now) {
                    (Stacking::Up, Some(n)) => n > mine_was,
                    (Stacking::Down, Some(n)) => n < mine_was,
                    _ => false,
                };
                let rest_before: Vec<&String> = was.iter().filter(|d| *d != me).collect();
                let rest_after: Vec<&String> = now.iter().filter(|d| *d != me).collect();
                if direction_ok && rest_before == rest_after {
                    exact += 1;
                    let r = format!("{me}: went more than one step (a clip or text object was in the way)");
                    if !reasons.contains(&r) && reasons.len() < 6 { reasons.push(r) }
                } else {
                    wrong += 1;
                    let r = format!("{me}: WRONG order\n      was {was:?}\n      now {now:?}");
                    if !reasons.contains(&r) && reasons.len() < 6 { reasons.push(r) }
                }
            }
        }
        println!("{label:>4}: exact {exact}, refused {refused}, wrong {wrong}");
        for r in &reasons { println!("      — {r}") }
    }
}
