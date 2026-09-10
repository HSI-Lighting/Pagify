//! Does locking a phrase take only that phrase — everywhere, not just on the
//! one page it was first tried on?
//!
//! A fix demonstrated on a single page is a fix for a single page. This locks a
//! phrase in the middle of a line on every page it can, and reports three
//! things per page:
//!
//! - **precise** — the selected words went and their neighbours on the same
//!   line stayed;
//! - **spilled** — the words went but took the rest of their line, which is the
//!   documented fallback where a run's codes cannot be lined up with its
//!   glyphs;
//! - **wrong** — anything else, which is a bug.
//!
//! ```text
//! PAGIFY_PDFIUM_LIB=<pdfium> cargo run --release --example lock_precision_probe -- <file.pdf> [pages]
//! ```

use pdf_core::document::{Document, DocumentMut, Rect, Redaction};

fn main() {
    let path = std::env::args().nth(1).expect("a pdf path");
    let limit: usize = std::env::args().nth(2).and_then(|p| p.parse().ok()).unwrap_or(30);

    let (mut precise, mut spilled, mut wrong, mut skipped) = (0usize, 0usize, 0usize, 0usize);
    let mut shifted = 0usize;
    let mut failures: Vec<String> = Vec::new();

    let pages = {
        let doc =
            pdf_core::document::pdfium_doc::PdfiumDocument::open_path(&path, None).expect("open");
        doc.page_count().min(limit)
    };

    for page in 0..pages {
        // A fresh document each time: locking rewrites and reopens, and a page
        // already locked is not the case under test.
        let mut doc =
            pdf_core::document::pdfium_doc::PdfiumDocument::open_path(&path, None).expect("open");

        let before = text_of(&doc, page);
        let Some(chars) = doc.page(page).ok().and_then(|p| p.characters().ok()) else {
            skipped += 1;
            continue;
        };

        // A phrase with real text either side of it on the same line — which is
        // exactly the case that used to take the whole line.
        let Some((at, len, tail)) = a_phrase_mid_line(&before) else {
            skipped += 1;
            continue;
        };
        let phrase: String = before.chars().skip(at).take(len).collect();

        let mut area =
            Rect { left: f32::MAX, top: f32::MAX, right: f32::MIN, bottom: f32::MIN };
        for index in at..at + len {
            let Some(b) = chars.boxes.get(index * 4..index * 4 + 4) else { continue };
            area.left = area.left.min(b[0]);
            area.top = area.top.min(b[1]);
            area.right = area.right.max(b[2]);
            area.bottom = area.bottom.max(b[3]);
        }
        if area.right <= area.left {
            skipped += 1;
            continue;
        }

        let outcome = doc.lock_area(
            &Redaction { require_complete: false, ..Redaction::new(page, area) },
            b"a good passcode",
            None,
        );
        let Ok(report) = outcome else {
            skipped += 1;
            continue;
        };

        let after = text_of(&doc, page);
        let gone = !after.contains(phrase.trim());
        let neighbour_kept = after.contains(tail.trim());

        // **Did the surviving text move?** Extraction cannot say — it reports
        // the same words wherever they sit — so the witness word's box is
        // compared before and after. Reported from use as the text being shoved
        // sideways, which is what a mis-sized gap looks like.
        let moved = doc
            .page(page)
            .ok()
            .and_then(|p| p.characters().ok())
            .and_then(|now| {
                let was_at = box_of(&chars, &before, &tail)?;
                let is_at = box_of(&now, &after, &tail)?;
                Some((is_at.0 - was_at.0).abs().max((is_at.1 - was_at.1).abs()))
            })
            .unwrap_or(0.0);
        if gone && moved > 1.0 {
            shifted += 1;
            if failures.len() < 8 {
                failures.push(format!(
                    "  page {}: {:?} moved {moved:.1}pt",
                    page + 1,
                    tail.trim()
                ));
            }
            continue;
        }

        if report.characters == 0 && report.objects == 0 {
            // Ok, but nothing removed — the fallback path finding nothing to
            // do. Counted apart from a real miss, because the cause differs.
            skipped += 1;
            continue;
        }
        if !gone {
            wrong += 1;
            let occurrences = before.matches(phrase.trim()).count();
            failures.push(format!(
                "  page {}: {:?} still there (appears {occurrences}x on the page, report said {} chars / {} objects)",
                page + 1,
                phrase.trim(),
                report.characters,
                report.objects
            ));
        } else if neighbour_kept && report.spilled.is_empty() {
            precise += 1;
        } else if gone {
            spilled += 1;
        }
    }

    println!("{path}");
    println!("pages tried : {pages}");
    println!("  precise   : {precise}");
    println!("  spilled   : {spilled}   (whole line went — the documented fallback)");
    println!("  wrong     : {wrong}");
    println!("  shifted   : {shifted}   (words hidden, but the survivors moved)");
    println!("  skipped   : {skipped}   (no usable phrase, or the lock refused)");
    for failure in failures.iter().take(8) {
        println!("{failure}");
    }
    let judged = precise + spilled + wrong + shifted;
    if judged > 0 {
        println!(
            "\nprecise on {:.0}% of the pages that locked",
            100.0 * precise as f32 / judged as f32
        );
    }
}

fn text_of(doc: &dyn Document, page: usize) -> String {
    doc.page(page)
        .and_then(|p| p.characters())
        .map(|c| c.text)
        .unwrap_or_default()
}

/// A phrase with words either side of it on the same line: character offset,
/// length, and something after it that must survive.
fn a_phrase_mid_line(text: &str) -> Option<(usize, usize, String)> {
    for line in text.lines() {
        let words: Vec<&str> = line.split_whitespace().collect();
        if words.len() < 6 || line.chars().count() < 30 {
            continue;
        }
        // Two words from the middle, and the two after them as the witness.
        //
        // **Both must appear exactly once on the page.** Otherwise "is it still
        // there?" finds the other copy and reports a failure that never
        // happened — which is what the first run of this probe did, on twelve
        // pages out of forty.
        let phrase = format!("{} {}", words[2], words[3]);
        let tail = format!("{} {}", words[4], words[5]);
        if text.matches(&phrase).count() != 1 || text.matches(&tail).count() != 1 {
            continue;
        }
        let at = text.find(&phrase)?;
        // `find` gives a byte offset; the boxes are per character.
        let at = text[..at].chars().count();
        return Some((at, phrase.chars().count(), tail));
    }
    None
}

/// Where a word sits on the page — the top-left of its first character.
fn box_of(
    chars: &pdf_core::document::PageCharacters,
    text: &str,
    word: &str,
) -> Option<(f32, f32)> {
    let at = text.find(word)?;
    let at = text[..at].chars().count();
    let b = chars.boxes.get(at * 4..at * 4 + 4)?;
    Some((b[0], b[1]))
}
