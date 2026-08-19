//! Are the files this engine writes actually valid PDFs?
//!
//! ```text
//! PAGIFY_PDFIUM_LIB=/tmp/pdfw/bin/pdfium.dll \
//!   cargo run --release --example save_validity_probe -- <out-dir> [pdf]
//! ```
//!
//! Every round-trip test in this crate saves with PDFium and reopens with PDFium.
//! That proves a command's effect was written; it cannot prove the file is
//! well-formed, because the only reader asked is the one that did the writing.
//! PDFium is tolerant of its own output — it reconstructs a broken cross-reference
//! table without complaint — so a structurally damaged save passes the whole suite.
//!
//! `qpdf --check` on a file this app saved says otherwise:
//!
//! ```text
//!   WARNING: expected endobj (xref stream: object 169 0, offset 287493)
//!   WARNING: file is damaged
//!   WARNING: xref not found (offset 286610)
//!   WARNING: Attempting to reconstruct cross-reference table
//! ```
//!
//! while the same document before the edit checks clean. This writes both save
//! paths out for the same input so an external reader can say which one does it,
//! which is what the work order's Task 4 acceptance actually asks for.
use pdf_core::command::{Command, CommandHistory};
use pdf_core::document::pdfium_doc::PdfiumDocument;
use pdf_core::document::{Annotation, Color, Document, Rect};

fn save(doc: &mut Box<dyn Document>, incremental: bool, path: &std::path::Path) {
    let mut bytes = Vec::new();
    let mutable = doc.as_document_mut().expect("mutable");
    let result = if incremental {
        mutable.save_incremental(&mut bytes)
    } else {
        mutable.save_full_copy(&mut bytes)
    };
    match result {
        Ok(()) => {
            std::fs::write(path, &bytes).expect("write");
            println!("  wrote {} ({} bytes)", path.display(), bytes.len());
        }
        Err(e) => println!("  FAILED to save {}: {e}", path.display()),
    }
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let out = std::path::PathBuf::from(args.get(1).expect("usage: save_validity_probe <out-dir>"));
    std::fs::create_dir_all(&out).expect("out dir");

    let fixtures = concat!(env!("CARGO_MANIFEST_DIR"), "/fixtures");
    let source = args
        .get(2)
        .cloned()
        .unwrap_or_else(|| format!("{fixtures}/pages-ladder.pdf"));

    // Untouched: the baseline. If this one is damaged too then the save is not
    // what broke it, and the input was never clean.
    {
        let mut doc: Box<dyn Document> =
            Box::new(PdfiumDocument::open_path(&source, None).expect("open"));
        println!("untouched:");
        save(&mut doc, true, &out.join("untouched-incremental.pdf"));
        save(&mut doc, false, &out.join("untouched-full.pdf"));
    }

    // A page-tree edit.
    {
        let mut doc: Box<dyn Document> =
            Box::new(PdfiumDocument::open_path(&source, None).expect("open"));
        let mut history = CommandHistory::default();
        history
            .execute(
                Command::SetPageRotation {
                    index: 0,
                    quarter_turns: 1,
                },
                doc.as_document_mut().expect("mutable"),
            )
            .expect("rotate");
        println!("after a rotation:");
        save(&mut doc, true, &out.join("rotated-incremental.pdf"));
        save(&mut doc, false, &out.join("rotated-full.pdf"));
    }

    // An annotation, which is what the damaged file on the device had.
    {
        let mut doc: Box<dyn Document> =
            Box::new(PdfiumDocument::open_path(&source, None).expect("open"));
        doc.as_document_mut()
            .expect("mutable")
            .add_annotation(
                0,
                &Annotation::Highlight {
                    rects: vec![Rect {
                        left: 20.0,
                        top: 30.0,
                        right: 120.0,
                        bottom: 44.0,
                    }],
                    color: Color {
                        r: 255,
                        g: 224,
                        b: 102,
                        a: 128,
                    },
                },
            )
            .expect("annotate");
        println!("after a highlight:");
        save(&mut doc, true, &out.join("marked-incremental.pdf"));
        save(&mut doc, false, &out.join("marked-full.pdf"));
    }

    println!("\nNow run qpdf --check over {}", out.display());
}
