//! Do an operator's character codes line up with PDFium's characters?
//!
//! The question that decides whether text can be cut *mid-operator*. Cutting a
//! whole operator takes the line with it; cutting part of one means knowing
//! which bytes of its string are the characters being removed.
//!
//! Two things have to hold:
//!
//! 1. **Bytes per code is knowable.** A simple font uses one byte per glyph; a
//!    `Type0` font with `Identity-H` uses two. Both are readable from the font
//!    dictionary — no CMap parsing.
//! 2. **The counts agree.** If an operator draws N codes, PDFium's run for it
//!    should report N characters. Ligatures and multi-byte oddities break that,
//!    and where they do, the safe answer is to fall back to cutting the whole
//!    operator rather than to slice at the wrong offset.
//!
//! Measured before building on either.
//!
//! ```text
//! PAGIFY_PDFIUM_LIB=<pdfium> cargo run --example code_align_probe -- <file.pdf> [page]
//! ```

use pdf_core::document::Document;
use pdf_core::pdf::{content, Object};

fn main() {
    let path = std::env::args().nth(1).expect("a pdf path");
    let page_index: usize = std::env::args().nth(2).and_then(|p| p.parse().ok()).unwrap_or(0);

    let doc = pdf_core::document::pdfium_doc::PdfiumDocument::open_path(&path, None).expect("open");
    let runs = doc.text_runs(page_index).expect("runs");
    let height = doc.page_size(page_index).expect("size").height_pt;

    let bytes = std::fs::read(&path).expect("read");
    let file = pdf_core::pdf::File::parse(&bytes).expect("parse");
    let page = page_dict(&file, page_index).expect("page");
    let (stream, _) = page_content(&file, &bytes, &page).expect("content");
    let fonts = font_dict(&file, &page);

    let operations = content::parse(&stream).expect("operations");
    let placed = content::placed(&operations);

    let (mut agree, mut disagree, mut unknown_font) = (0usize, 0usize, 0usize);
    let mut examples: Vec<String> = Vec::new();

    for run in &runs {
        let (want_x, want_y) = (run.rect.left, height - run.rect.bottom);
        let Some(found) = placed
            .iter()
            .map(|p| (p, ((p.origin.x - want_x).powi(2) + (p.origin.y - want_y).powi(2)).sqrt()))
            .filter(|(_, d)| *d <= 4.0)
            .min_by(|a, b| a.1.total_cmp(&b.1))
            .map(|(p, _)| p)
        else {
            continue;
        };

        let Some(width) = found
            .font
            .as_ref()
            .and_then(|name| fonts.as_ref().map(|f| (f, name)))
            .and_then(|(f, name)| code_width(&file, f, name))
        else {
            unknown_font += 1;
            continue;
        };

        let codes = count_codes(&operations[found.origin.operation], width);
        let characters = run.text.chars().count();
        if codes == characters {
            agree += 1;
        } else {
            disagree += 1;
            if examples.len() < 6 {
                // What the font's own table says each code spells — the answer
                // to why the counts disagree.
                let unicode = found
                    .font
                    .as_ref()
                    .zip(fonts.as_ref())
                    .and_then(|(name, dict)| to_unicode(&file, &bytes, dict, name));
                if let Some(map) = &unicode {
                    let codes = codes_of(&operations[found.origin.operation], width);
                    let mut odd: Vec<String> = Vec::new();
                    for code in &codes {
                        match map.get(code) {
                            Some(text) if text.chars().count() == 1
                                && !text.chars().next().is_some_and(|c| c.is_control()
                                    || c == '\u{ad}' || c == '\u{200b}') => {}
                            Some(text) => odd.push(format!(
                                "{code:04X}->{:?}({})",
                                text,
                                text.chars().count()
                            )),
                            None => odd.push(format!("{code:04X}->unmapped")),
                        }
                        if odd.len() >= 4 {
                            break;
                        }
                    }
                    examples.push(format!(
                        "  {} codes vs {characters} chars — {:?}",
                        codes.len(),
                        odd
                    ));
                    continue;
                }
            }
            if false {
                // Which characters are not ordinary text — the ones PDFium may
                // be synthesising rather than reading from the stream.
                let odd: Vec<String> = run
                    .text
                    .chars()
                    .filter(|c| c.is_control() || *c == '\u{ad}')
                    .map(|c| format!("U+{:04X}", c as u32))
                    .collect();
                examples.push(format!(
                    "  {codes} codes vs {characters} chars ({}b/code) odd={:?}: {:?}",
                    width,
                    odd,
                    run.text.chars().take(30).collect::<String>()
                ));
            }
        }
    }

    println!("page {}: {} run(s)", page_index + 1, runs.len());
    println!("codes == characters : {agree}");
    println!("codes != characters : {disagree}");
    println!("font not resolvable : {unknown_font}");
    for example in &examples {
        println!("{example}");
    }
    let judged = agree + disagree;
    if judged > 0 {
        println!(
            "\nsliceable on {:.0}% of the runs whose font could be read",
            100.0 * agree as f32 / judged as f32
        );
    }
}

/// Bytes per character code for a font resource.
fn code_width(file: &pdf_core::pdf::File<'_>, fonts: &pdf_core::pdf::Dict, name: &[u8]) -> Option<usize> {
    let font = file.resolve(fonts.get(name)?).ok()?;
    let dict = font.as_dict()?;
    match dict.get(b"Subtype").and_then(Object::as_name) {
        Some(b"Type0") => {
            // Only the identity encodings are two bytes with no CMap to read.
            match dict.get(b"Encoding").and_then(Object::as_name) {
                Some(b"Identity-H") | Some(b"Identity-V") => Some(2),
                _ => None,
            }
        }
        Some(_) => Some(1),
        None => None,
    }
}

fn count_codes(operation: &content::Operation, width: usize) -> usize {
    fn bytes_in(object: &Object) -> usize {
        match object {
            Object::LiteralString(raw) => unescape_len(raw),
            // Hex digits, two to a byte.
            Object::HexString(raw) => {
                raw.iter().filter(|b| b.is_ascii_hexdigit()).count().div_ceil(2)
            }
            _ => 0,
        }
    }
    let total: usize = operation
        .operands
        .iter()
        .map(|operand| match operand {
            Object::Array(items) => items.iter().map(bytes_in).sum(),
            other => bytes_in(other),
        })
        .sum();
    total / width.max(1)
}

/// A literal string's length once its escapes are resolved.
fn unescape_len(raw: &[u8]) -> usize {
    let mut count = 0usize;
    let mut at = 0usize;
    while at < raw.len() {
        if raw[at] == b'\\' {
            at += 1;
            // An octal escape is up to three digits.
            let mut digits = 0;
            while digits < 3 && raw.get(at).is_some_and(|b| (b'0'..=b'7').contains(b)) {
                at += 1;
                digits += 1;
            }
            if digits == 0 {
                at += 1;
            }
        } else {
            at += 1;
        }
        count += 1;
    }
    count
}

fn font_dict(file: &pdf_core::pdf::File<'_>, page: &Object) -> Option<pdf_core::pdf::Dict> {
    let mut node = page.clone();
    for _ in 0..64 {
        let dict = node.as_dict()?;
        if let Some(resources) = dict.get(b"Resources") {
            let resources = file.resolve(resources).ok()?;
            if let Some(fonts) = resources.as_dict().and_then(|d| d.get(b"Font")) {
                return file.resolve(fonts).ok()?.as_dict().cloned();
            }
        }
        node = file.resolve(dict.get(b"Parent")?).ok()?;
    }
    None
}

fn page_dict(file: &pdf_core::pdf::File<'_>, index: usize) -> Option<Object> {
    let root = file.resolve(file.trailer().get(b"Root")?).ok()?;
    let pages = file.resolve(root.as_dict()?.get(b"Pages")?).ok()?;
    let mut flat = Vec::new();
    collect(file, &pages, &mut flat, 0);
    flat.into_iter().nth(index)
}

fn page_content(
    file: &pdf_core::pdf::File<'_>,
    bytes: &[u8],
    page: &Object,
) -> Option<(Vec<u8>, usize)> {
    let contents = file.resolve(page.as_dict()?.get(b"Contents")?).ok()?;
    let parts = match page.as_dict()?.get(b"Contents")? {
        Object::Array(items) => items.clone(),
        other => vec![other.clone()],
    };
    let _ = contents;

    let mut stream = Vec::new();
    let mut count = 0;
    for part in parts {
        if let Ok(Object::Stream(dict, range)) = file.resolve(&part) {
            stream.extend_from_slice(&content::decode(&dict, &bytes[range])?);
            stream.push(b'\n');
            count += 1;
        }
    }
    Some((stream, count))
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

/// The font's `/ToUnicode` table, if it has one.
fn to_unicode(
    file: &pdf_core::pdf::File<'_>,
    bytes: &[u8],
    fonts: &pdf_core::pdf::Dict,
    name: &[u8],
) -> Option<pdf_core::pdf::cmap::ToUnicode> {
    let font = file.resolve(fonts.get(name)?).ok()?;
    let entry = font.as_dict()?.get(b"ToUnicode")?;
    let Object::Stream(dict, range) = file.resolve(entry).ok()? else { return None };
    let decoded = content::decode(&dict, &bytes[range])?;
    Some(pdf_core::pdf::cmap::parse(&decoded))
}

/// The character codes an operator draws.
fn codes_of(operation: &content::Operation, width: usize) -> Vec<u32> {
    let mut out = Vec::new();
    for piece in content::pieces(operation) {
        if let content::Piece::Codes(raw) = piece {
            for chunk in raw.chunks(width.max(1)) {
                let mut code = 0u32;
                for byte in chunk {
                    code = code << 8 | u32::from(*byte);
                }
                out.push(code);
            }
        }
    }
    out
}
