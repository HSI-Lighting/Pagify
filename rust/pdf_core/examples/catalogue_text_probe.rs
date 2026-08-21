//! Why does text extraction find nothing in a document that plainly has text?
//!
//! ```text
//! PAGIFY_PDFIUM_LIB=/tmp/pdfw/bin/pdfium.dll \
//!   cargo run --release --example catalogue_text_probe -- <pdf> [first_page] [count]
//! ```
//!
//! `text_objects.rs` reported this document as "text converted to outlines" and
//! that conclusion was wrong. Its sweep counted characters like this:
//!
//! ```ignore
//! let chars = page.text().map(|t| t.len()).unwrap_or(0);
//! ```
//!
//! `unwrap_or(0)` turns a *failed* `FPDFText_LoadPage` into "this page has no
//! text", which is a completely different fact. Every conclusion drawn from that
//! sweep rests on not being able to tell the two apart.
//!
//! The file's own structure says text is there: every page carries
//! `/ProcSet [ /PDF /Text ]` and a `/Font` resource, and the font is
//! `/Subtype /TrueType` with `/Encoding /WinAnsiEncoding` — a standard encoding
//! that needs no `ToUnicode` to map to Unicode.
//!
//! So this probe separates what that one conflated, per page:
//!
//! - did loading the text page *fail*, and with what error;
//! - `FPDFText_CountChars`, which is PDFium's own count;
//! - what `all()` returns, which goes through `FPDFText_GetBoundedText` and can
//!   disagree with the count;
//! - top-level object types, **and** objects nested inside form XObjects, since
//!   page-object enumeration does not recurse and this document draws through
//!   forms on nearly every page.
use pdfium_render::prelude::*;

/// Counts by type, so text nested inside a form is not invisible.
#[derive(Default, Debug, Clone, Copy)]
struct Counts {
    text: usize,
    path: usize,
    image: usize,
    form: usize,
    other: usize,
}

impl Counts {
    fn add(&mut self, other: Counts) {
        self.text += other.text;
        self.path += other.path;
        self.image += other.image;
        self.form += other.form;
        self.other += other.other;
    }
}

/// Walk a page's objects, descending into form XObjects.
///
/// `depth` guards against a form that (directly or otherwise) contains itself;
/// a malformed document can do that and it would otherwise recurse forever.
fn count_objects<'a>(
    objects: impl Iterator<Item = PdfPageObject<'a>>,
    depth: usize,
    nested: &mut Counts,
) -> Counts {
    let mut top = Counts::default();

    for object in objects {
        match object.object_type() {
            PdfPageObjectType::Text => top.text += 1,
            PdfPageObjectType::Path => top.path += 1,
            PdfPageObjectType::Image => top.image += 1,
            PdfPageObjectType::XObjectForm => {
                top.form += 1;
                if depth < 8 {
                    if let Some(form) = object.as_x_object_form_object() {
                        let inner = count_objects(form.iter(), depth + 1, nested);
                        nested.add(inner);
                    }
                }
            }
            _ => top.other += 1,
        }
    }

    top
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let path = args
        .get(1)
        .expect("usage: catalogue_text_probe <pdf> [first] [count]");
    let first: usize = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(0);
    let count: usize = args.get(3).and_then(|s| s.parse().ok()).unwrap_or(6);

    let lib = std::env::var("PAGIFY_PDFIUM_LIB").expect("set PAGIFY_PDFIUM_LIB");
    let pdfium = Pdfium::new(Pdfium::bind_to_library(&lib).expect("bind pdfium"));

    println!("opening {path} ...");
    let doc = match pdfium.load_pdf_from_file(path, None) {
        Ok(doc) => doc,
        Err(e) => {
            println!("FAILED TO OPEN: {e}");
            return;
        }
    };

    let pages = doc.pages().len() as usize;
    println!("{pages} pages\n");

    let last = (first + count).min(pages);
    for index in first..last {
        let pages_ref = doc.pages();
        let page = match pages_ref.get(index as i32) {
            Ok(page) => page,
            Err(e) => {
                println!("page {index}: FAILED TO LOAD PAGE: {e}");
                continue;
            }
        };

        let mut nested = Counts::default();
        let top = count_objects(page.objects().iter(), 0, &mut nested);

        // The distinction the old sweep threw away. Bound to a local declared
        // after `page` so it drops first and the borrow checker is satisfied.
        let loaded = page.text();
        match loaded {
            Err(e) => {
                println!(
                    "page {index}: TEXT PAGE FAILED TO LOAD: {e}\n           \
                     top-level {top:?}\n           inside forms {nested:?}",
                );
            }
            Ok(text) => {
                let chars = text.len();
                let extracted = text.all();
                let shown: String = extracted.chars().take(70).collect();

                println!(
                    "page {index}: CountChars={chars}  all()={} chars  \
                     top-level text={} path={} image={} form={}  \
                     nested text={} path={} image={}",
                    extracted.chars().count(),
                    top.text,
                    top.path,
                    top.image,
                    top.form,
                    nested.text,
                    nested.path,
                    nested.image,
                );
                if !shown.trim().is_empty() {
                    println!("           text: {:?}", shown.replace('\n', " "));
                }
                // The two disagreeing is itself the finding: PDFium found
                // characters, and the call we actually ship returned none of them.
                if chars > 0 && extracted.trim().is_empty() {
                    println!(
                        "           ^^ PDFium counts {chars} characters but all() \
                         returned nothing — the bug is in the extraction call, not \
                         the document",
                    );
                }
            }
        }
    }
}
