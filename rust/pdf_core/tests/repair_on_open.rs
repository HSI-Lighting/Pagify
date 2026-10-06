//! A file an earlier build damaged when it saved it opens anyway.
//!
//! Reported from use, twice: "pdfium error: PdfiumLibraryInternalError(Unknown)"
//! opening a document that was sound until Pagify saved it. The file is built
//! here with a real cross-reference stream and then damaged exactly as the old
//! repair damaged it, so what is asserted is that PDFium really cannot read it
//! and that, mended, it can — not that a function agrees with itself.
//!
//! ```text
//! PAGIFY_PDFIUM_LIB=<pdfium> cargo test --test repair_on_open
//! ```

mod harness;
use harness::{serial, skip_without_pdfium};

use pdf_core::document::pdfium_doc::PdfiumDocument;
use pdf_core::document::Document;

/// One blank page whose objects live in an **object stream**, reached through a
/// cross-reference *stream* whose binary table holds the bytes `<<`.
///
/// What makes it a faithful stand-in for the real file:
///
/// * The `<<` is in an *offset* — the object stream sits at byte 0x3C3C, so its
///   table entry is `01 00 00 3C 3C 00` — and the entries that matter come after
///   it. Writing `/Type/XRef` in there pushes them ten bytes on, so a reader
///   gets garbage for the catalogue, and cannot recover by rebuilding because
///   the objects it would look for are packed inside the object stream.
/// * A file of plain objects, or one whose damage falls before every entry that
///   matters, is read by PDFium whatever is wrong with the table, and proves
///   nothing.
///
/// `damaged` writes the type into the table and leaves it out of the dictionary
/// — what the repair before 0.1.36 did.
fn file(damaged: bool) -> Vec<u8> {
    let bodies = [
        "<< /Type /Catalog /Pages 3 0 R >>",
        "<< /Type /Pages /Kids [4 0 R] /Count 1 >>",
        "<< /Type /Page /Parent 3 0 R /MediaBox [0 0 200 300] >>",
    ];
    let mut header = String::new();
    let mut packed = String::new();
    for (i, body) in bodies.iter().enumerate() {
        header.push_str(&format!("{} {} ", i + 2, packed.len()));
        packed.push_str(body);
        packed.push('\n');
    }
    let first = header.len();
    let objstm = format!("{header}{packed}");

    let mut out = b"%PDF-1.5\n".to_vec();
    // A comment, to put the object stream where its offset holds `<<`.
    const OBJSTM_AT: usize = 0x3C3C;
    out.extend_from_slice(b"%");
    out.resize(OBJSTM_AT - 1, b'x');
    out.push(b'\n');
    assert_eq!(out.len(), OBJSTM_AT);
    out.extend_from_slice(
        format!(
            "1 0 obj\n<< /Type /ObjStm /N 3 /First {first} /Length {} >>\nstream\n{objstm}\nendstream\nendobj\n",
            objstm.len()
        )
        .as_bytes(),
    );
    let table_at = out.len();

    // /W [1 4 1]: type, field 2, field 3. Object 0 is free; 1 is the object
    // stream; 2-4 are packed in it (type 2: the stream, then the index in it);
    // 5 is this table.
    let mut data: Vec<u8> = vec![0, 0, 0, 0, 0, 255];
    data.push(1);
    data.extend_from_slice(&(OBJSTM_AT as u32).to_be_bytes());
    data.push(0);
    for index in 0..3u8 {
        data.push(2);
        data.extend_from_slice(&1u32.to_be_bytes());
        data.push(index);
    }
    data.push(1);
    data.extend_from_slice(&(table_at as u32).to_be_bytes());
    data.push(0);
    let length = data.len();
    if damaged {
        let at = data.windows(2).position(|w| w == b"<<").expect("the table holds <<") + 2;
        data.splice(at..at, b"/Type/XRef".iter().copied());
    }
    out.extend_from_slice(b"5 0 obj\n");
    out.extend_from_slice(
        format!(
            "<<{}/Root 2 0 R/Size 6/W[1 4 1]/Length {length}>>",
            if damaged { "" } else { "/Type/XRef" }
        )
        .as_bytes(),
    );
    out.extend_from_slice(b"stream\r\n");
    out.extend_from_slice(&data);
    out.extend_from_slice(b"\r\nendstream\nendobj\nstartxref\n");
    out.extend_from_slice(table_at.to_string().as_bytes());
    out.extend_from_slice(b"\n%%EOF\n");
    out
}

#[test]
fn the_undamaged_file_opens_and_is_not_reported_as_mended() {
    let Some(()) = skip_without_pdfium() else { return };
    let _lock = serial();

    let sound = PdfiumDocument::open_bytes(file(false), None).expect("the sound file does not open");
    assert_eq!(sound.page_count(), 1);
    assert!(!sound.repaired_on_open(), "a sound file was reported as mended");
}

#[test]
fn a_file_damaged_by_the_old_repair_opens_and_says_it_was_mended() {
    let Some(()) = skip_without_pdfium() else { return };
    let _lock = serial();

    let doc = PdfiumDocument::open_bytes(file(true), None).expect("the damaged file did not open");
    assert_eq!(doc.page_count(), 1, "the mended file lost its page");
    assert!(doc.repaired_on_open(), "the file opened without being mended, so this proves nothing");
}

#[test]
fn a_damaged_file_on_disk_opens_by_path_and_the_file_itself_is_left_as_it_was() {
    let Some(()) = skip_without_pdfium() else { return };
    let _lock = serial();

    let dir = std::env::temp_dir().join(format!("pagify-repair-on-open-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("damaged.pdf");
    let bytes = file(true);
    std::fs::write(&path, &bytes).unwrap();

    let doc = PdfiumDocument::open_path(path.to_str().unwrap(), None).expect("the damaged file did not open");
    assert_eq!(doc.page_count(), 1);
    assert!(doc.repaired_on_open());
    assert_eq!(std::fs::read(&path).unwrap(), bytes, "opening rewrote the file on disk");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_file_that_is_simply_not_a_pdf_still_says_so() {
    let Some(()) = skip_without_pdfium() else { return };
    let _lock = serial();

    assert!(PdfiumDocument::open_bytes(b"this is not a pdf".to_vec(), None).is_err());
}

/// A document known to be damaged, as it came off a user's machine. Run with
/// `PAGIFY_REAL_DAMAGED_PDF` set to it; does nothing otherwise.
#[test]
#[ignore = "needs a real damaged document: set PAGIFY_REAL_DAMAGED_PDF"]
fn a_real_damaged_document_opens_and_says_it_was_mended() {
    let Some(()) = skip_without_pdfium() else { return };
    let Ok(real) = std::env::var("PAGIFY_REAL_DAMAGED_PDF") else { return };
    let _lock = serial();

    let doc = PdfiumDocument::open_path(&real, None).expect("the damaged document did not open");
    eprintln!("pages = {}, repaired_on_open = {}", doc.page_count(), doc.repaired_on_open());
    assert!(doc.repaired_on_open(), "it opened without being mended");
}

/// What matters to a person with such a file: after it opens mended, **saving
/// writes a sound one** — by either kind of save — which opens again with every
/// page loadable and nothing left to mend. Set `PAGIFY_REAL_DAMAGED_PDF`; set
/// `PAGIFY_REAL_OUT_DIR` as well to keep the two files for another reader to check.
#[test]
#[ignore = "needs a real damaged document: set PAGIFY_REAL_DAMAGED_PDF"]
fn saving_a_mended_document_writes_a_sound_file_either_way() {
    use pdf_core::document::DocumentMut;
    let Some(()) = skip_without_pdfium() else { return };
    let Ok(real) = std::env::var("PAGIFY_REAL_DAMAGED_PDF") else { return };
    let _lock = serial();

    for incremental in [false, true] {
        let mut doc = PdfiumDocument::open_path(&real, None).expect("the damaged document did not open");
        assert!(doc.repaired_on_open());
        let pages = doc.page_count();
        let mut saved = Vec::new();
        if incremental {
            doc.save_incremental(&mut saved).expect("incremental save");
        } else {
            doc.save_full_copy(&mut saved).expect("full save");
        }
        if let Ok(dir) = std::env::var("PAGIFY_REAL_OUT_DIR") {
            std::fs::write(std::path::Path::new(&dir).join(if incremental { "incremental.pdf" } else { "full.pdf" }), &saved).unwrap();
        }
        let again = PdfiumDocument::open_bytes(saved, None).expect("the saved file does not open");
        assert!(!again.repaired_on_open(), "{}: the saved file still needs mending", if incremental { "incremental" } else { "full" });
        assert_eq!(again.page_count(), pages);
        for page in 0..pages {
            again.page_size(page).unwrap_or_else(|e| panic!("page {} of the saved file does not load: {e}", page + 1));
        }
    }
}
