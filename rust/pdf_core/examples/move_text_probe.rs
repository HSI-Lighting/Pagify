//! Moving a run of words: does it go where it is sent, and does the page survive?
//!
//! Reports per run: the distance it actually moved against the distance asked
//! for, whether every other run stayed put, and whether the page's words are
//! still the page's words.
//!
//! ```text
//! PAGIFY_PDFIUM_LIB=<pdfium> cargo run --release --example move_text_probe -- <file.pdf> [pages]
//! ```

use pdf_core::document::pdfium_doc::PdfiumDocument;
use pdf_core::document::{Document, DocumentMut, Point};

fn main() {
    let path = std::env::args().nth(1).expect("a pdf path");
    let limit: usize = std::env::args().nth(2).and_then(|p| p.parse().ok()).unwrap_or(4);
    let by = Point { x: 20.0, y: 12.0 };

    let password = std::env::var("PDF_PASSWORD").ok();
    let open = |path: &str| PdfiumDocument::open_path(path, password.as_deref());
    let pages = open(&path).expect("open").page_count().min(limit);
    let (mut exact, mut wrong, mut refused, mut disturbed, mut scrambled) = (0, 0, 0, 0, 0);
    let mut reasons: Vec<String> = Vec::new();

    for page in 0..pages {
        let doc = open(&path).expect("open");
        let runs = doc.text_runs(page).unwrap_or_default();
        drop(doc);

        for run in runs.iter().take(12) {
            let doc = open(&path).expect("open");
            let before = doc.text_runs(page).unwrap_or_default();
            let words: Vec<String> = before.iter().map(|r| r.text.trim().to_string()).collect();
            drop(doc);

            let mut doc = open(&path).expect("open");
            // The byte-safe path on its own, so its coverage is measured rather
            // than hidden behind the fallback that always works.
            let outcome = if std::env::var("BYTE_SAFE_ONLY").is_ok() {
                doc.try_move_run_in_stream(page, run.object, by)
            } else {
                doc.move_object(page, run.object, by)
            };
            match outcome {
                Err(e) => {
                    refused += 1;
                    let reason = e.to_string();
                    if !reasons.contains(&reason) {
                        reasons.push(reason);
                    }
                    continue;
                }
                Ok(()) => {}
            }

            let after = doc.text_runs(page).unwrap_or_default();
            let Some(now) = after.iter().find(|r| r.object == run.object) else {
                wrong += 1;
                continue;
            };
            let (dx, dy) = (now.rect.left - run.rect.left, now.rect.top - run.rect.top);
            if (dx - by.x).abs() < 0.5 && (dy - by.y).abs() < 0.5 {
                exact += 1;
            } else {
                wrong += 1;
                let reason = format!("moved ({dx:+.1}, {dy:+.1}) when asked for ({:+.1}, {:+.1})", by.x, by.y);
                if !reasons.contains(&reason) {
                    reasons.push(reason);
                }
                continue;
            }

            // Everything else where it was.
            let mut shifted = 0;
            for was in &before {
                if was.object == run.object { continue }
                if let Some(is) = after.iter().find(|r| r.object == was.object) {
                    if (is.rect.left - was.rect.left).abs() > 0.5
                        || (is.rect.top - was.rect.top).abs() > 0.5
                    {
                        shifted += 1;
                    }
                }
            }
            if shifted > 0 {
                disturbed += 1;
                for was in &before {
                    if was.object == run.object { continue }
                    if let Some(is) = after.iter().find(|r| r.object == was.object) {
                        let (ox, oy) = (is.rect.left - was.rect.left, is.rect.top - was.rect.top);
                        if ox.abs() > 0.5 || oy.abs() > 0.5 {
                            println!(
                                "  page {} object {}: moving {:?} shifted {:?} by ({ox:+.1}, {oy:+.1})",
                                page + 1, run.object,
                                run.text.trim().chars().take(30).collect::<String>(),
                                was.text.trim().chars().take(30).collect::<String>(),
                            );
                        }
                    }
                }
            }

            let mut a = words.clone();
            let mut b: Vec<String> = after.iter().map(|r| r.text.trim().to_string()).collect();
            a.sort();
            b.sort();
            if a != b {
                scrambled += 1;
            }
        }
    }

    println!("{path}");
    println!("  moved exactly     : {exact}");
    println!("  moved wrongly     : {wrong}");
    println!("  refused           : {refused}");
    println!("  disturbed a neighbour : {disturbed}");
    println!("  changed the words     : {scrambled}");
    for reason in reasons.iter().take(6) {
        println!("     — {reason}");
    }
}
