//! What objects a page holds, by kind — the ground truth under a classifier.
//!
//! Reads the page's content stream directly, so nothing between here and the
//! file gets to have an opinion.
use pdf_core::pdf::{content, File, Object};

fn main() {
    let path = std::env::args().nth(1).expect("a pdf path");
    let index: usize = std::env::args().nth(2).and_then(|p| p.parse().ok()).unwrap_or(0);

    let bytes = {
        use pdf_core::document::DocumentMut;
        let mut doc =
            pdf_core::document::pdfium_doc::PdfiumDocument::open_path(&path, None).expect("open");
        let mut out = Vec::new();
        doc.save_full_copy(&mut out).expect("save");
        out
    };
    let file = File::parse(&bytes).expect("parse");
    let page = page_dict(&file, index).expect("page");
    let stream = page_content(&file, &bytes, &page).unwrap_or_default();
    let operations = content::parse(&stream).expect("operations");

    let mut counts: std::collections::BTreeMap<String, usize> = Default::default();
    for op in &operations {
        let name = String::from_utf8_lossy(&op.operator).to_string();
        *counts.entry(name).or_default() += 1;
    }
    println!("page {}: {} operation(s) in {} bytes", index + 1, operations.len(), stream.len());
    // Only the ones that say what the page is made of.
    for key in ["Tj", "TJ", "'", "\"", "m", "l", "c", "re", "f", "f*", "S", "B", "Do", "sh", "BI"] {
        if let Some(n) = counts.get(key) {
            println!("   {key:<3} {n}");
        }
    }
}

fn page_dict(file: &File<'_>, index: usize) -> Option<Object> {
    let root = file.resolve(file.trailer().get(b"Root")?).ok()?;
    let pages = file.resolve(root.as_dict()?.get(b"Pages")?).ok()?;
    let mut flat = Vec::new();
    collect(file, &pages, &mut flat, 0);
    flat.into_iter().nth(index)
}
fn collect(file: &File<'_>, node: &Object, out: &mut Vec<Object>, depth: usize) {
    if depth > 64 { return }
    let Some(dict) = node.as_dict() else { return };
    if dict.get(b"Type").and_then(Object::as_name) == Some(&b"Page"[..]) {
        out.push(node.clone());
        return;
    }
    let Some(kids) = dict.get(b"Kids") else { return };
    if let Ok(Object::Array(items)) = file.resolve(kids) {
        for kid in items {
            if let Ok(kid) = file.resolve(&kid) { collect(file, &kid, out, depth + 1) }
        }
    }
}
fn page_content(file: &File<'_>, bytes: &[u8], page: &Object) -> Option<Vec<u8>> {
    let parts = match page.as_dict()?.get(b"Contents")? {
        Object::Array(items) => items.clone(),
        other => vec![other.clone()],
    };
    let mut stream = Vec::new();
    for part in parts {
        if let Ok(Object::Stream(dict, range)) = file.resolve(&part) {
            stream.extend_from_slice(&content::decode(&dict, &bytes[range])?);
            stream.push(b'\n');
        }
    }
    Some(stream)
}
