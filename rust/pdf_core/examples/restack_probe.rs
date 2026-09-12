//! Does moving something through the drawing order put it there, and leave the
//! rest of the page as it was?
use pdf_core::document::pdfium_doc::PdfiumDocument;
use pdf_core::document::{Document, DocumentMut, Stacking};

fn main() {
    let path = std::env::args().nth(1).expect("a pdf path");
    let page: usize = std::env::args().nth(2).and_then(|p| p.parse().ok()).unwrap_or(0);
    let password = std::env::var("PDF_PASSWORD").ok();
    let open = |p: &str| PdfiumDocument::open_path(p, password.as_deref());

    let doc = open(&path).expect("open");
    let before = doc.drawn_objects(page).expect("objects");
    println!("page {} draws {} things, bottom first:", page + 1, before.len());
    for d in before.iter().take(20) {
        println!(
            "  {:>3}{} {:<8} {:?}{}",
            d.object,
            "  ".repeat(d.depth),
            d.kind.describe(),
            d.label,
            if d.movable { "" } else { "   (inside a group)" },
        );
    }
    drop(doc);

    for (label, to) in [("front", Stacking::Front), ("back", Stacking::Back)] {
        let (mut moved, mut refused, mut wrong, mut disturbed) = (0, 0, 0, 0);
        let mut reasons: Vec<String> = Vec::new();
        for target in before.iter().filter(|d| d.movable) {
            let mut doc = open(&path).expect("open");
            let was: Vec<String> = doc.drawn_objects(page).unwrap_or_default()
                .iter().map(|d| format!("{}:{}", d.kind.describe(), d.label)).collect();
            let words = doc.page(page).and_then(|p| p.text()).unwrap_or_default();

            match doc.restack(page, target.object, to) {
                Err(e) => {
                    refused += 1;
                    let r = format!("REFUSED {}:{}: {e}", target.kind.describe(), target.label.chars().take(24).collect::<String>());
                    if !reasons.contains(&r) { reasons.push(r) }
                    continue;
                }
                Ok(()) => {}
            }
            let after: Vec<String> = doc.drawn_objects(page).unwrap_or_default()
                .iter().map(|d| format!("{}:{}", d.kind.describe(), d.label)).collect();
            let me = format!("{}:{}", target.kind.describe(), target.label);

            // It should now be at the end (front) or the start (back).
            let at = match to {
                Stacking::Front => after.last(),
                Stacking::Back => after.first(),
                Stacking::Up | Stacking::Down => unreachable!("this probe measures the ends"),
            };
            if at != Some(&me) {
                wrong += 1;
                let r = format!("{me:?} wanted at the {label}, found {:?}", at);
                if !reasons.contains(&r) { reasons.push(r) }
                continue;
            }
            moved += 1;

            // Everything else in the same relative order, and the same words.
            let rest_before: Vec<&String> = was.iter().filter(|d| **d != me).collect();
            let rest_after: Vec<&String> = after.iter().filter(|d| **d != me).collect();
            let now_words = doc.page(page).and_then(|p| p.text()).unwrap_or_default();
            let mut a: Vec<char> = words.chars().filter(|c| !c.is_whitespace()).collect();
            let mut b: Vec<char> = now_words.chars().filter(|c| !c.is_whitespace()).collect();
            a.sort_unstable();
            b.sort_unstable();
            if a != b {
                disturbed += 1;
                if reasons.len() < 6 {
                    reasons.push(format!("{me:?}: the page LOST OR GAINED characters"));
                }
            } else if rest_before != rest_after {
                disturbed += 1;
                if reasons.len() < 6 {
                    reasons.push(format!(
                        "{me:?}: the others re-ordered\n        was {rest_before:?}\n        now {rest_after:?}"
                    ));
                }
            } else if now_words != words {
                // Extraction order follows the stream, so a deliberate restack
                // changes it. Not a defect — reported so it is not mistaken for one.
                if reasons.len() < 6 {
                    reasons.push(format!("{me:?}: same characters, read out in a new order (expected)"));
                }
            }
        }
        println!("\nto the {label}:  moved {moved}, refused {refused}, landed wrong {wrong}, disturbed {disturbed}");
        for r in reasons.iter().filter(|r| r.starts_with("REFUSED")).take(8) { println!("    — {r}") }
    }
}
