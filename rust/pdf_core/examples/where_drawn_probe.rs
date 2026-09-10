//! Where a page's text actually lives, when it is not in the page's own stream.
//!
//! A page draws by executing its `/Contents`. But a `/Do` hands off to a form
//! XObject with a content stream of its own, and an annotation carries an
//! appearance stream that a reader draws over the page. Text in either is on
//! the page to look at and absent from the bytes the editor parses.
//!
//! ```text
//! PAGIFY_PDFIUM_LIB=<pdfium> cargo run --release --example where_drawn_probe -- <file.pdf> [page]
//! ```

use pdf_core::pdf::{content, File, Object};

fn main() {
    let path = std::env::args().nth(1).expect("a pdf path");
    let index: usize = std::env::args().nth(2).and_then(|p| p.parse().ok()).unwrap_or(0);

    // The bytes the editor sees, not the ones on disk: the file may use a
    // cross-reference stream, and PDFium's own save is what the edit path
    // parses.
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

    // The page's own stream.
    let own = page_content(&file, &bytes, &page).unwrap_or_default();
    let operations = content::parse(&own).unwrap_or_default();
    println!(
        "the page's own stream: {} bytes, {} operation(s), {} placed",
        own.len(),
        operations.len(),
        content::placed(&operations).len()
    );

    // Form XObjects it invokes.
    let resources = inherited(&file, &page, b"Resources");
    let xobjects = resources
        .as_ref()
        .and_then(|r| r.as_dict())
        .and_then(|d| d.get(b"XObject"))
        .and_then(|x| file.resolve(x).ok())
        .and_then(|x| x.as_dict().cloned());

    match &xobjects {
        None => println!("\nno XObjects on this page"),
        Some(dict) => {
            println!("\n{} XObject(s):", dict.0.len());
            for (name, entry) in dict.0.iter() {
                let Ok(Object::Stream(sub, range)) = file.resolve(entry) else { continue };
                let subtype = sub
                    .get(b"Subtype")
                    .and_then(Object::as_name)
                    .map(|n| String::from_utf8_lossy(n).into_owned())
                    .unwrap_or_default();
                let decoded = content::decode(&sub, &bytes[range]).unwrap_or_default();
                let placed = content::parse(&decoded)
                    .map(|ops| content::placed(&ops).len())
                    .unwrap_or(0);
                println!(
                    "  /{:<20} {subtype:<10} {} bytes, {placed} placed text operator(s)",
                    String::from_utf8_lossy(name),
                    decoded.len()
                );
            }
        }
    }

    // Annotations with appearance streams.
    let annotations = page
        .as_dict()
        .and_then(|d| d.get(b"Annots"))
        .and_then(|a| file.resolve(a).ok());
    match annotations {
        Some(Object::Array(items)) => {
            println!("\n{} annotation(s):", items.len());
            for item in items.iter().take(12) {
                let Ok(annot) = file.resolve(item) else { continue };
                let Some(dict) = annot.as_dict() else { continue };
                let subtype = dict
                    .get(b"Subtype")
                    .and_then(Object::as_name)
                    .map(|n| String::from_utf8_lossy(n).into_owned())
                    .unwrap_or_default();
                let has_appearance = dict.get(b"AP").is_some();
                println!("  {subtype:<14} appearance stream: {has_appearance}");
            }
        }
        _ => println!("\nno annotations on this page"),
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
