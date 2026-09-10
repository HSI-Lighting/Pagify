//! What is in a PDF that is not on its pages?
//!
//! "Hidden data" is a category, not a feature, and building the wrong half of
//! it is easy. So this asks real documents what they actually carry before
//! anything is written to remove it.
//!
//! ```text
//! PAGIFY_PDFIUM_LIB=<pdfium> cargo run --release --example hidden_data_probe -- <file.pdf>...
//! ```

use pdf_core::pdf::{File, Object};

fn main() {
    for path in std::env::args().skip(1) {
        let Ok(bytes) = std::fs::read(&path) else {
            println!("{path}: could not read");
            continue;
        };
        let Ok(file) = File::parse(&bytes) else {
            println!("{path}: this reader cannot parse it");
            continue;
        };
        println!("\n=== {} ({} bytes) ===", path.rsplit('/').next().unwrap_or(&path), bytes.len());

        // Earlier revisions. An incremental save leaves everything the file
        // used to say sitting in front of the new version — the classic way a
        // redaction leaks.
        let revisions = bytes.windows(9).filter(|w| *w == b"startxref").count();
        println!("saved revisions in the file : {revisions}");

        // Who made it and when.
        let info = file
            .trailer()
            .get(b"Info")
            .and_then(|i| file.resolve(i).ok())
            .and_then(|i| i.as_dict().cloned());
        match &info {
            Some(dict) => {
                let keys: Vec<String> = dict
                    .0
                    .iter()
                    .map(|(k, _)| String::from_utf8_lossy(k).into_owned())
                    .collect();
                println!("document information       : {}", keys.join(", "));
            }
            None => println!("document information       : none"),
        }

        let root = file
            .trailer()
            .get(b"Root")
            .and_then(|r| file.resolve(r).ok())
            .and_then(|r| r.as_dict().cloned());
        let has = |key: &[u8]| -> bool {
            root.as_ref().is_some_and(|r| r.get(key).is_some())
        };
        println!("XMP metadata stream        : {}", has(b"Metadata"));
        println!("an action on opening       : {}", has(b"OpenAction"));
        println!("optional content (layers)  : {}", has(b"OCProperties"));
        println!("an interactive form        : {}", has(b"AcroForm"));

        // The names tree carries embedded files and document-level JavaScript.
        let names = root
            .as_ref()
            .and_then(|r| r.get(b"Names"))
            .and_then(|n| file.resolve(n).ok())
            .and_then(|n| n.as_dict().cloned());
        println!(
            "embedded files             : {}",
            names.as_ref().is_some_and(|n| n.get(b"EmbeddedFiles").is_some())
        );
        println!(
            "document JavaScript        : {}",
            names.as_ref().is_some_and(|n| n.get(b"JavaScript").is_some())
        );

        // And what the sanitiser makes of it.
        match pdf_core::pdf::hidden::survey(&file, &bytes) {
            Ok(found) => println!("\nsurvey says: {}", found.describe()),
            Err(e) => println!("\nsurvey failed: {e}"),
        }
        if let Ok((clean, _)) = pdf_core::pdf::hidden::strip(&file, &bytes) {
            println!("cleaned: {} bytes -> {} bytes", bytes.len(), clean.len());
            match pdf_core::pdf::File::parse(&clean) {
                Ok(again) => match pdf_core::pdf::hidden::survey(&again, &clean) {
                    Ok(after) => println!("after cleaning: {}", after.describe()),
                    Err(e) => println!("after cleaning: survey failed: {e}"),
                },
                Err(e) => println!("the cleaned file will not parse: {e}"),
            }
            // The pages have to survive, which only a reader can say.
            match pdf_core::document::pdfium_doc::PdfiumDocument::open_bytes(clean, None) {
                Ok(doc) => {
                    use pdf_core::document::Document;
                    let text = doc
                        .page(0)
                        .and_then(|p| p.characters())
                        .map(|c| c.text)
                        .unwrap_or_default();
                    println!(
                        "reopened: {} page(s), first words {:?}",
                        doc.page_count(),
                        text.chars().take(40).collect::<String>()
                    );
                }
                Err(e) => println!("the cleaned file will not open: {e}"),
            }
        }

        // Objects nothing reaches from the catalogue — content that was
        // deleted from the pages but never taken out of the file.
        let mut reached = std::collections::BTreeSet::new();
        if let Some(Object::Reference(n, _)) = file.trailer().get(b"Root") {
            walk(&file, *n, &mut reached, 0);
        }
        let total: Vec<u32> = file.numbers().collect();
        let orphans = total.iter().filter(|n| !reached.contains(n)).count();
        println!("objects                    : {} total, {orphans} unreachable", total.len());
    }
}

/// Every object reachable from one, following references.
fn walk(file: &File<'_>, number: u32, seen: &mut std::collections::BTreeSet<u32>, depth: usize) {
    if depth > 96 || !seen.insert(number) {
        return;
    }
    let Ok(object) = file.object(number) else { return };
    references(&object, &mut |n| walk(file, n, seen, depth + 1));
}

fn references(object: &Object, found: &mut impl FnMut(u32)) {
    match object {
        Object::Reference(n, _) => found(*n),
        Object::Array(items) => {
            for item in items {
                references(item, found);
            }
        }
        Object::Dict(dict) => {
            for (_, value) in &dict.0 {
                references(value, found);
            }
        }
        Object::Stream(dict, _) => {
            for (_, value) in &dict.0 {
                references(value, found);
            }
        }
        _ => {}
    }
}
