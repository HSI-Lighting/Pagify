//! Can a show-text operator be matched to the run PDFium reports?
//!
//! The question the whole text half turns on. `crate::pdf::content` can cut an
//! operation out of a stream without disturbing a byte either side — but only
//! if we can say *which* operation drew the words being locked.
//!
//! PDFium reports text runs in page-object order, and a content stream draws in
//! the order it is written. If those two orders agree, matching is counting. If
//! they do not, this needs something cleverer, and it is better to know that
//! before building on the assumption.
//!
//! ```text
//! PAGIFY_PDFIUM_LIB=<pdfium> cargo run --example content_match_probe -- <file.pdf> [page]
//! ```

use pdf_core::document::Document;
use pdf_core::pdf::{content, Object};

fn main() {
    let path = std::env::args().nth(1).expect("a pdf path");
    let page_index: usize = std::env::args().nth(2).and_then(|p| p.parse().ok()).unwrap_or(0);

    let doc = pdf_core::document::pdfium_doc::PdfiumDocument::open_path(&path, None).expect("open");
    let runs = doc.text_runs(page_index).expect("text runs");
    println!("PDFium reports {} text run(s) on page {}", runs.len(), page_index + 1);

    let bytes = std::fs::read(&path).expect("read");
    let file = pdf_core::pdf::File::parse(&bytes).expect("parse");

    // Walk down to the page and its content stream.
    let root = file.resolve(file.trailer().get(b"Root").expect("root")).expect("catalogue");
    let pages = file
        .resolve(root.as_dict().and_then(|d| d.get(b"Pages")).expect("pages"))
        .expect("page tree");
    let mut flat = Vec::new();
    collect(&file, &pages, &mut flat, 0);
    let Some(page) = flat.get(page_index) else {
        println!("no such page");
        return;
    };

    let Some(contents) = page.as_dict().and_then(|d| d.get(b"Contents")) else {
        println!("that page has no content stream");
        return;
    };
    let mut stream = Vec::new();
    match file.resolve(contents).expect("contents") {
        Object::Stream(dict, range) => match content::decode(&dict, &bytes[range]) {
            Some(data) => stream.extend_from_slice(&data),
            None => {
                println!("the content stream uses a filter this cannot read");
                return;
            }
        },
        Object::Array(parts) => {
            // Several streams, concatenated — legal, and common.
            for part in parts {
                if let Ok(Object::Stream(dict, range)) = file.resolve(&part) {
                    match content::decode(&dict, &bytes[range]) {
                        Some(data) => {
                            stream.extend_from_slice(&data);
                            stream.push(b'\n');
                        }
                        None => {
                            println!("one of the content streams uses an unreadable filter");
                            return;
                        }
                    }
                }
            }
        }
        other => {
            println!("the contents are neither a stream nor an array: {other:?}");
            return;
        }
    }
    println!("content stream: {} bytes decoded", stream.len());

    let operations = match content::parse(&stream) {
        Ok(operations) => operations,
        Err(e) => {
            println!("the content stream could not be read: {e}");
            return;
        }
    };
    let showing: Vec<_> = operations.iter().filter(|o| o.shows_text()).collect();
    println!("show-text operators: {}", showing.len());
    println!(
        "\n{:<6} {:<44} {}",
        "op",
        "what the operator draws",
        "what PDFium calls run N"
    );
    for (index, operation) in showing.iter().take(12).enumerate() {
        let drawn = shown_text(operation);
        let reported = runs.get(index).map(|r| r.text.as_str()).unwrap_or("(no run)");
        println!(
            "{:<6} {:<44} {}",
            String::from_utf8_lossy(&operation.operator),
            truncate(&drawn, 42),
            truncate(reported, 40)
        );
    }

    // Which operators the page actually uses — enough to say whether text
    // state this does not track (horizontal scaling, word or character
    // spacing) is in play.
    let mut tally: std::collections::BTreeMap<String, usize> = Default::default();
    for operation in &operations {
        *tally
            .entry(String::from_utf8_lossy(&operation.operator).into_owned())
            .or_default() += 1;
    }
    print!("OPERATOR TALLY:");
    for name in ["Tz", "Tc", "Tw", "TJ", "Tj", "Tf", "Tm", "Td", "TD", "T*", "'"] {
        if let Some(count) = tally.get(name) {
            print!("  {name}={count}");
        }
    }
    println!();

    let same = showing.len() == runs.len();
    println!(
        "\ncounts {}: {} operators, {} runs",
        if same { "AGREE" } else { "DISAGREE" },
        showing.len(),
        runs.len()
    );

    // The measurement that decides whether matching by position works: for
    // every run PDFium reports, is there an operator starting where it says?
    let placed = content::origins(&operations);
    let height = doc.page_size(page_index).expect("size").height_pt;
    let mut matched = 0usize;
    let mut worst = 0.0f32;
    for run in &runs {
        // Origins are y-up from the bottom; runs are y-down from the top.
        let (want_x, want_y) = (run.rect.left, height - run.rect.bottom);
        let nearest = placed
            .iter()
            .map(|o| ((o.x - want_x).powi(2) + (o.y - want_y).powi(2)).sqrt())
            .fold(f32::MAX, f32::min);
        if nearest <= 4.0 {
            matched += 1;
            worst = worst.max(nearest);
        }
    }
    println!(
        "runs with an operator within 4pt of where PDFium says: {matched}/{} (worst {worst:.2}pt)",
        runs.len()
    );
}

/// The bytes a show-text operator draws, as written in the stream.
fn shown_text(operation: &content::Operation) -> String {
    let mut out = String::new();
    for operand in &operation.operands {
        match operand {
            Object::LiteralString(raw) => out.push_str(&String::from_utf8_lossy(raw)),
            Object::HexString(raw) => out.push_str(&format!("<{}>", String::from_utf8_lossy(raw))),
            Object::Array(items) => {
                for item in items {
                    match item {
                        Object::LiteralString(raw) => {
                            out.push_str(&String::from_utf8_lossy(raw))
                        }
                        Object::HexString(raw) => {
                            out.push_str(&format!("<{}>", String::from_utf8_lossy(raw)))
                        }
                        _ => {}
                    }
                }
            }
            _ => {}
        }
    }
    out
}

fn truncate(text: &str, to: usize) -> String {
    let cleaned: String = text.chars().map(|c| if c.is_control() { ' ' } else { c }).collect();
    if cleaned.chars().count() <= to {
        cleaned
    } else {
        cleaned.chars().take(to - 1).chain("…".chars()).collect()
    }
}

fn collect(file: &pdf_core::pdf::File<'_>, node: &Object, out: &mut Vec<Object>, depth: usize) {
    if depth > 64 {
        return;
    }
    let Some(dict) = node.as_dict() else { return };
    if dict.get(b"Type").and_then(Object::as_name) == Some(&b"Page"[..]) {
        out.push(node.clone());
        return;
    }
    let Some(kids) = dict.get(b"Kids") else { return };
    if let Ok(Object::Array(items)) = file.resolve(kids) {
        for kid in items {
            if let Ok(kid) = file.resolve(&kid) {
                collect(file, &kid, out, depth + 1);
            }
        }
    }
}
