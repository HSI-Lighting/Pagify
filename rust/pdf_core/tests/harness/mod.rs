//! Shared plumbing for the round-trip tests.
//!
//! Kept separate so the tests themselves read as statements about behaviour
//! rather than as PDFium setup.

use std::path::PathBuf;

use pdf_core::document::pdfium_doc::PdfiumDocument;
use pdf_core::document::Document;

/// The desktop PDFium, or `None` if it is not configured.
///
/// Skipping rather than failing is deliberate: these tests need a desktop build
/// of the *pinned* PDFium, which is fetched separately and is not on every
/// machine. A hard failure would train people to ignore a red suite; a skip that
/// says why does not. CI sets the variable and therefore runs them.
pub fn skip_without_pdfium() -> Option<()> {
    match std::env::var("PAGIFY_PDFIUM_LIB") {
        Ok(_) => Some(()),
        Err(_) => {
            eprintln!(
                "skipped: set PAGIFY_PDFIUM_LIB to a desktop PDFium of the pinned \
                 build (chromium/7881) to run the round-trip tests"
            );
            None
        }
    }
}

pub fn fixture_path(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("fixtures")
        .join(name)
}

pub fn fixture_bytes(name: &str) -> Vec<u8> {
    std::fs::read(fixture_path(name)).expect("read fixture")
}

pub fn open_fixture(_pdfium: &(), name: &str) -> Box<dyn Document> {
    let path = fixture_path(name);
    Box::new(
        PdfiumDocument::open_path(path.to_str().expect("fixture path"), None)
            .expect("open fixture"),
    )
}

/// Widths in whole points, which is how these tests identify pages.
pub fn page_widths(doc: &Box<dyn Document>) -> Vec<i32> {
    (0..doc.page_count())
        .map(|i| doc.page_size(i).expect("page size").width_pt.round() as i32)
        .collect()
}

/// Save through the engine and reopen from those bytes.
///
/// The reopen is the whole point: asserting against the in-memory document after
/// a command proves only that the command ran, not that what it did was written.
pub fn save_and_reopen(_pdfium: &(), doc: &mut Box<dyn Document>) -> Box<dyn Document> {
    let mut bytes = Vec::new();
    doc.as_document_mut()
        .expect("the document must be mutable")
        .save_incremental(&mut bytes)
        .expect("save");

    Box::new(PdfiumDocument::open_bytes(bytes, None).expect("reopen what we just wrote"))
}

/// Save through the rewriting path and reopen.
///
/// Used by the tests that can run today. It is *not* what the app should do —
/// a full rewrite relocates every object and breaks any signature — but it is
/// the only save reachable until the binding exposes the document handle, and it
/// still proves that a command's effect was written rather than merely applied.
pub fn save_full_copy_and_reopen(doc: &mut Box<dyn Document>) -> Box<dyn Document> {
    let mut bytes = Vec::new();
    doc.as_document_mut()
        .expect("the document must be mutable")
        .save_full_copy(&mut bytes)
        .expect("save");

    Box::new(PdfiumDocument::open_bytes(bytes, None).expect("reopen what we just wrote"))
}

/// Serialises whole tests against each other.
///
/// Not tidiness — a correctness requirement this suite discovered. Run in
/// parallel, tests that each hold an open document report each other's page
/// sizes: deleting a page and reopening gave `[200, 612, 612, 612]`, Letter being
/// what PDFium reports for a page whose geometry it cannot resolve. Single
/// threaded the same code is exact.
///
/// `pdfium-render` keys a process-global page-index cache on raw
/// `(FPDF_DOCUMENT, FPDF_PAGE)` addresses, which PDFium recycles freely. The cache
/// is behind a mutex, so this is not a data race — it is aliasing.
///
/// The obvious explanation is wrong, and `examples/handle_reuse_probe.rs` exists to
/// rule it out: opening documents one after another reuses the *same* address every
/// time, and each one still reports its own pages. Entries do not simply outlive a
/// close. What breaks it is *overlap* — one document being torn down while another
/// is built at the same address — which takes two threads and cannot happen
/// sequentially at all.
///
/// `examples/handle_race_probe.rs` measures the difference. Two threads cycling
/// open → read → close over two different files:
///
/// ```text
///                    opens failed    wrong page geometry
///   unserialised     771 of 800      3 of the 29 reads that got through
///   serialised         0 of 800      0 of 800
/// ```
///
/// The app is fixed rather than worked around: `registry::insert_with` holds the
/// registry lock across the open itself, so construction, destruction, mutation and
/// rendering are all serialised against one another. These tests talk to
/// `PdfiumDocument` directly and bypass the registry, which is why they still need
/// a lock of their own.
pub fn serial() -> std::sync::MutexGuard<'static, ()> {
    static LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
    LOCK.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// An external check that the bytes this engine wrote are a well-formed PDF.
///
/// Every other assertion in this suite saves with PDFium and reopens with PDFium,
/// which proves a command's effect was written and cannot prove the file is valid:
/// the only reader asked is the one that did the writing, and PDFium silently
/// reconstructs a broken cross-reference table rather than complaining.
///
/// That gap hid a real defect. `FPDF_INCREMENTAL` on a document using a
/// cross-reference *stream* produced a file qpdf called damaged — even with no
/// edit at all — while every test here stayed green. The fixtures could not catch
/// it either, because they all used classic `xref` tables; `xref-stream.pdf`
/// exists so this path is exercised.
///
/// Skipped, not failed, when qpdf is absent: it is a separate install, and a hard
/// failure would train people to ignore a red suite. Set `PAGIFY_QPDF` to the
/// binary, or have it on `PATH`.
pub fn check_with_qpdf(bytes: &[u8], label: &str) {
    let Some(qpdf) = qpdf_binary() else {
        eprintln!("skipped qpdf check of {label}: set PAGIFY_QPDF or put qpdf on PATH");
        return;
    };

    let path = std::env::temp_dir().join(format!("pagify-qpdf-{label}.pdf"));
    std::fs::write(&path, bytes).expect("write bytes for qpdf");

    let output = std::process::Command::new(&qpdf)
        .arg("--check")
        .arg(&path)
        .output()
        .expect("run qpdf");

    let report = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    );
    let _ = std::fs::remove_file(&path);

    // qpdf exits 2 on errors and 3 on warnings alone. Warnings are tolerated —
    // one of them ("xref entry for the xref stream itself is missing") qpdf
    // describes as common and handled correctly, and rewriting the table's binary
    // data to silence it would risk far more than it gains. Anything qpdf calls
    // damaged is a failure.
    assert!(
        !report.contains("file is damaged"),
        "qpdf calls {label} damaged:\n{report}",
    );
    assert_ne!(
        Some(2),
        output.status.code(),
        "qpdf reported errors in {label}:\n{report}",
    );
}

fn qpdf_binary() -> Option<std::path::PathBuf> {
    if let Ok(explicit) = std::env::var("PAGIFY_QPDF") {
        let path = std::path::PathBuf::from(explicit);
        return path.exists().then_some(path);
    }
    // A bare name lets the OS search PATH.
    std::process::Command::new("qpdf")
        .arg("--version")
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|_| std::path::PathBuf::from("qpdf"))
}

/// Save through both paths and hand each to qpdf.
pub fn check_both_saves(doc: &mut Box<dyn Document>, label: &str) {
    let mut incremental = Vec::new();
    doc.as_document_mut()
        .expect("mutable")
        .save_incremental(&mut incremental)
        .expect("incremental save");
    check_with_qpdf(&incremental, &format!("{label}-incremental"));

    let mut full = Vec::new();
    doc.as_document_mut()
        .expect("mutable")
        .save_full_copy(&mut full)
        .expect("full save");
    check_with_qpdf(&full, &format!("{label}-full"));
}
