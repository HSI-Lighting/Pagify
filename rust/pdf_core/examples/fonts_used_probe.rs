//! Which faces a page names, and whether their programs are in the file.
use pdf_core::pdf::{File, Object};

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
    let Some(fonts) = inherited(&file, &page, b"Resources")
        .and_then(|r| r.as_dict().and_then(|d| d.get(b"Font")).cloned())
        .and_then(|f| file.resolve(&f).ok())
        .and_then(|f| f.as_dict().cloned())
    else {
        println!("page {} names no fonts", index + 1);
        return;
    };

    println!("page {}:", index + 1);
    for (name, entry) in fonts.0.iter() {
        let Ok(font) = file.resolve(entry) else { continue };
        let Some(dict) = font.as_dict() else { continue };
        let base = dict
            .get(b"BaseFont")
            .and_then(Object::as_name)
            .map(|n| String::from_utf8_lossy(n).into_owned())
            .unwrap_or_default();
        let subtype = dict
            .get(b"Subtype")
            .and_then(Object::as_name)
            .map(|n| String::from_utf8_lossy(n).into_owned())
            .unwrap_or_default();
        // Is the program itself in the file?
        let descriptor = dict
            .get(b"FontDescriptor")
            .or_else(|| dict.get(b"DescendantFonts"))
            .and_then(|d| file.resolve(d).ok());
        let embedded = descriptor
            .as_ref()
            .and_then(|d| match d {
                Object::Array(items) => items.first().and_then(|f| file.resolve(f).ok()),
                other => Some(other.clone()),
            })
            .and_then(|d| d.as_dict().and_then(|x| x.get(b"FontDescriptor")).cloned().or(Some(Object::Null)))
            .and_then(|d| file.resolve(&d).ok())
            .and_then(|d| d.as_dict().cloned())
            .map(|d| {
                d.get(b"FontFile").is_some()
                    || d.get(b"FontFile2").is_some()
                    || d.get(b"FontFile3").is_some()
            })
            .unwrap_or(false);
        println!("  /{:<10} {base:<40} {subtype:<12} embedded: {embedded}", String::from_utf8_lossy(name));
    }
}

fn inherited(file: &File<'_>, page: &Object, key: &[u8]) -> Option<Object> {
    let mut node = page.clone();
    for _ in 0..64 {
        let dict = node.as_dict()?;
        if let Some(found) = dict.get(key) {
            return file.resolve(found).ok();
        }
        node = file.resolve(dict.get(b"Parent")?).ok()?;
    }
    None
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
