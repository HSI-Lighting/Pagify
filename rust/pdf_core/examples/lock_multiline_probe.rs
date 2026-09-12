//! Does locking a selection that spans two lines take only the selection?
//!
//! The app turns a text selection into **one rectangle**: the union of the
//! selected lines' boxes. On one line that union is exactly the selection. On
//! two it is a box wide enough to hold both, so it swallows the start of the
//! first line and the end of the last — the words either side of what was
//! picked.
//!
//! ```text
//! PAGIFY_PDFIUM_LIB=<pdfium> cargo run --release --example lock_multiline_probe -- <file.pdf> [page]
//! ```

use pdf_core::document::{Document, DocumentMut, Rect, Redaction};

const PASSCODE: &[u8] = b"Passcode-1!";

fn text_of(doc: &dyn Document, page: usize) -> String {
    doc.page(page).and_then(|p| p.text()).unwrap_or_default()
}

/// One rect per line, exactly as `Characters::line_rects` does it.
fn line_rects(boxes: &[f32], range: std::ops::Range<usize>) -> Vec<Rect> {
    let at = |i: usize| Rect {
        left: boxes[i * 4],
        top: boxes[i * 4 + 1],
        right: boxes[i * 4 + 2],
        bottom: boxes[i * 4 + 3],
    };
    let mut lines: Vec<Rect> = Vec::new();
    let mut current = at(range.start);
    for i in range.start + 1..range.end {
        let b = at(i);
        if b.top < current.bottom && b.bottom > current.top {
            current = Rect {
                left: current.left.min(b.left),
                top: current.top.min(b.top),
                right: current.right.max(b.right),
                bottom: current.bottom.max(b.bottom),
            };
        } else {
            lines.push(current);
            current = b;
        }
    }
    lines.push(current);
    lines
}

fn main() {
    let path = std::env::args().nth(1).expect("a pdf path");
    let page: usize = std::env::args().nth(2).and_then(|p| p.parse().ok()).unwrap_or(0);

    let doc =
        pdf_core::document::pdfium_doc::PdfiumDocument::open_path(&path, None).expect("open");
    let before = text_of(&doc, page);
    let chars = doc.page(page).expect("page").characters().expect("characters");
    let per_char = chars.boxes.len() / 4;
    drop(doc);

    // A selection that starts mid-line and ends mid-*another* line: walk
    // forward until the vertical position breaks, then keep going a little.
    let at = |i: usize| Rect {
        left: chars.boxes[i * 4],
        top: chars.boxes[i * 4 + 1],
        right: chars.boxes[i * 4 + 2],
        bottom: chars.boxes[i * 4 + 3],
    };
    // Start a third of the way into a line that has a following line.
    let mut start = None;
    for i in 0..per_char.saturating_sub(40) {
        let a = at(i);
        // Find a line break after i, within 60 characters.
        let broke = (i + 1..(i + 60).min(per_char))
            .find(|j| { let b = at(*j); !(b.top < a.bottom && b.bottom > a.top) });
        if let Some(b) = broke {
            if b > i + 8 && b + 8 < per_char {
                start = Some((i + 4, b + 6));
                break;
            }
        }
    }
    let Some((from, to)) = start else {
        println!("no two-line selection available on page {}", page + 1);
        return;
    };

    let selected: String = before.chars().skip(from).take(to - from).collect();
    let head: String = before.chars().take(from).collect();
    let head_word = head.split_whitespace().last().unwrap_or("").to_string();
    let tail: String = before.chars().skip(to).collect();
    let tail_word = tail.split_whitespace().next().unwrap_or("").to_string();

    let rects = line_rects(&chars.boxes, from..to);
    println!("selection spans {} line(s)", rects.len());
    println!("  selected  : {selected:?}");
    println!("  just before: {head_word:?}   just after: {tail_word:?}");

    // Both ways round, so the difference is measured rather than asserted.
    let union_only: bool = std::env::var("UNION_ONLY").is_ok();
    let first = rects[0];
    let area = rects.iter().fold(first, |acc, r| Rect {
        left: acc.left.min(r.left),
        top: acc.top.min(r.top),
        right: acc.right.max(r.right),
        bottom: acc.bottom.max(r.bottom),
    });
    let request = if union_only {
        Redaction { require_complete: false, ..Redaction::new(page, area) }
    } else {
        Redaction {
            require_complete: false,
            ..Redaction::over(page, rects.clone()).expect("shapes")
        }
    };
    println!("  sending    : {}", if union_only { "the union (the old way)" } else { "one shape per line" });

    let mut doc =
        pdf_core::document::pdfium_doc::PdfiumDocument::open_path(&path, None).expect("open");
    match doc.lock_area(&request, PASSCODE, None) {
        Ok(report) => {
            let after = text_of(&doc, page);
            println!("  locked {} characters", report.characters);
            let kept_before = head_word.is_empty() || after.contains(&head_word);
            let kept_after = tail_word.is_empty() || after.contains(&tail_word);
            println!(
                "  word before the selection survived : {}",
                if kept_before { "yes" } else { "NO — it took more than was picked" }
            );
            println!(
                "  word after  the selection survived : {}",
                if kept_after { "yes" } else { "NO — it took more than was picked" }
            );
        }
        Err(e) => println!("  lock refused: {e}"),
    }
}
