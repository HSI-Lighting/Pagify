//! What each XObject a page draws actually is.
use pdf_core::document::pdfium_doc::PdfiumDocument;
use pdf_core::document::Document;
use pdf_core::pdf::Object;

fn main() {
    let path = std::env::args().nth(1).expect("a pdf path");
    let page: usize = std::env::args().nth(2).and_then(|p| p.parse().ok()).unwrap_or(0);
    let password = std::env::var("PDF_PASSWORD").ok();
    let doc = PdfiumDocument::open_path(&path, password.as_deref()).expect("open");

    // The stream, for the order the names are drawn in.
    let stream = doc.page_stream(page).expect("stream");
    let ops = pdf_core::pdf::content::parse(&stream).expect("parse");
    let drawn: Vec<String> = ops
        .iter()
        .filter(|o| o.operator == b"Do")
        .filter_map(|o| match o.operands.first() {
            Some(Object::Name(n)) => Some(String::from_utf8_lossy(n).to_string()),
            _ => None,
        })
        .collect();
    println!("drawn, in order: {drawn:?}\n");

    // The resources, from the file itself.
    let bytes = doc.readable_bytes_for_probe().expect("bytes");
    let file = pdf_core::pdf::File::parse(&bytes).expect("parse");
    let names = doc.xobject_names_for_probe(&file, page).expect("names");
    for (name, number) in names {
        let what = match file.object(number) {
            Ok(Object::Stream(dict, span)) => {
                let get = |k: &[u8]| match dict.get(k) {
                    Some(Object::Name(n)) => String::from_utf8_lossy(n).to_string(),
                    Some(Object::Number(n)) => String::from_utf8_lossy(n).to_string(),
                    Some(Object::Bool(b)) => b.to_string(),
                    Some(other) => format!("{other:?}").chars().take(20).collect(),
                    None => "-".into(),
                };
                format!(
                    "obj {number}: /Subtype {} /Width {} /Height {} /ImageMask {} /Filter {} ({} bytes)",
                    get(b"Subtype"), get(b"Width"), get(b"Height"), get(b"ImageMask"), get(b"Filter"), span.len()
                )
            }
            other => format!("obj {number}: {other:?}"),
        };
        println!("  /{:<5} {}", String::from_utf8_lossy(&name), what.chars().take(140).collect::<String>());
    }
}
