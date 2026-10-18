//! The three read-only reports — `crossref`, `transitions` and `calcorder` —
//! against documents whose contents are known.
//!
//! The fixtures that ship with this tree have no internal links, no page
//! transitions and no `/CO` array between them, so the "none" path is tested
//! against real files and the positive path against small documents written
//! here: a PDF is only objects and a cross-reference table, and writing one
//! exact document beats claiming an untested path.
//!
//! ```text
//! PAGIFY_PDFIUM_LIB=<pdfium> cargo test -p pagify_shell --test reports
//! ```

use pagify_shell::Session;

fn fixture(name: &str) -> String {
    format!(
        "{}/../../../rust/pdf_core/fixtures/{name}",
        env!("CARGO_MANIFEST_DIR")
    )
}

/// A small, valid PDF with the objects given, written with a plain xref
/// table. Every page in the documents below references the same empty content
/// stream, as a PDFium-written file would.
fn pdf(objects: &[&str]) -> Vec<u8> {
    let mut out = b"%PDF-1.7\n".to_vec();
    let mut offsets = Vec::new();
    for (index, body) in objects.iter().enumerate() {
        offsets.push(out.len());
        out.extend_from_slice(format!("{} 0 obj\n{body}\nendobj\n", index + 1).as_bytes());
    }
    let xref_at = out.len();
    out.extend_from_slice(format!("xref\n0 {}\n0000000000 65535 f \n", objects.len() + 1).as_bytes());
    for offset in &offsets {
        out.extend_from_slice(format!("{offset:010} 00000 n \n").as_bytes());
    }
    out.extend_from_slice(
        format!(
            "trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref_at}\n%%EOF\n",
            objects.len() + 1
        )
        .as_bytes(),
    );
    out
}

/// Write `bytes` somewhere no other test uses, and hand back the path.
fn temp(name: &str, bytes: &[u8]) -> std::path::PathBuf {
    static NEXT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    let n = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let path =
        std::env::temp_dir().join(format!("pagify-reports-{name}-{}-{n}.pdf", std::process::id()));
    std::fs::write(&path, bytes).expect("the fixture could not be written");
    path
}

// ---------------------------------------------------------------------------
// crossref
// ---------------------------------------------------------------------------

/// Two pages; page 1 has two links to page 2 — one through a `/Dest`
/// destination array and one through a `/GoTo` action, the two shapes a file
/// spells an internal link in. Both are cross-references and both are reported.
fn linked_pdf() -> Vec<u8> {
    pdf(&[
        "<< /Type /Catalog /Pages 2 0 R >>",
        "<< /Type /Pages /Kids [3 0 R 4 0 R] /Count 2 >>",
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 200 300] /Contents 5 0 R \
         /Annots [6 0 R 7 0 R] >>",
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 200 300] /Contents 5 0 R >>",
        "<< /Length 0 >>\nstream\n\nendstream",
        "<< /Type /Annot /Subtype /Link /Rect [10 250 60 290] /Border [0 0 0] \
         /Dest [4 0 R /XYZ 0 300 0] >>",
        "<< /Type /Annot /Subtype /Link /Rect [20 260 30 270] /Border [0 0 0] \
         /A << /S /GoTo /D [4 0 R /Fit] >> >>",
    ])
}

#[test]
fn the_internal_links_the_file_has_are_reported_with_their_page_and_target() {
    let path = temp("links", &linked_pdf());
    let session = Session::open(&path).expect("open the linked fixture");
    let lines = pagify_shell::crossref::report(&session).expect("report");
    std::fs::remove_file(&path).ok();

    // Both annotations, in the page's own top-left space: the 300pt-high page
    // puts the 250..290pt PDF-space rectangle at 10..50 down.
    assert!(
        lines.iter().any(|l| l == "page 1: link at (10, 10) → page 2"),
        "the /Dest link is missing: {lines:?}"
    );
    assert!(
        lines.iter().any(|l| l == "page 1: link at (20, 30) → page 2"),
        "the /GoTo link is missing: {lines:?}"
    );
    assert!(
        lines.iter().any(|l| l.contains("2 cross-reference(s) over 2 page(s)")),
        "the count is missing: {lines:?}"
    );
}

#[test]
fn a_document_without_internal_links_answers_none() {
    // `spread.pdf` is two pages and its cross-reference table is a plain one —
    // three of the shipped fixtures carry a damaged table on purpose (`single-
    // page`, `pages-ladder`, `mixed-sizes`, for the xref-repair path), and the
    // raw reader is right to refuse those.
    let session = Session::open(fixture("spread.pdf")).expect("open");
    let lines = pagify_shell::crossref::report(&session).expect("report");

    assert_eq!(lines.len(), 1, "{lines:?}");
    assert!(lines[0].contains("no cross-references"), "{lines:?}");
    assert!(lines[0].contains("2 page(s)"), "{lines:?}");
    assert!(lines[0].contains("not built"), "{lines:?}");
}

// ---------------------------------------------------------------------------
// transitions
// ---------------------------------------------------------------------------

/// Two pages: the first carries a split transition with a display time, the
/// second carries nothing.
fn transitioning_pdf() -> Vec<u8> {
    pdf(&[
        "<< /Type /Catalog /Pages 2 0 R >>",
        "<< /Type /Pages /Kids [3 0 R 4 0 R] /Count 2 >>",
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 200 300] /Contents 5 0 R \
         /Trans << /S /Split /D 1.5 /Dm /H /M /I /Di 90 >> /Dur 3 >>",
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 200 300] /Contents 5 0 R >>",
        "<< /Length 0 >>\nstream\n\nendstream",
    ])
}

#[test]
fn a_page_transition_in_the_file_is_reported() {
    let path = temp("transitions", &transitioning_pdf());
    let session = Session::open(&path).expect("open the transition fixture");
    let lines = pagify_shell::transitions::report(&session).expect("report");
    std::fs::remove_file(&path).ok();

    assert_eq!(
        lines[0],
        "page 1: split, 1.5 s, horizontal, inward, 90°, displayed for 3 s",
        "{lines:?}"
    );
    assert!(
        lines.iter().any(|l| l.contains("1 page(s) with page timing over 2 page(s)")),
        "{lines:?}"
    );
    assert!(lines.iter().any(|l| l.contains("file on disk")), "{lines:?}");
}

#[test]
fn a_document_without_page_transitions_answers_none() {
    let session = Session::open(fixture("spread.pdf")).expect("open");
    let lines = pagify_shell::transitions::report(&session).expect("report");

    assert_eq!(lines.len(), 1, "{lines:?}");
    assert!(lines[0].contains("no page transitions"), "{lines:?}");
    assert!(lines[0].contains("Read from the file on disk"), "{lines:?}");
}

/// A file whose cross-reference table is a stream is legal, and this build's
/// raw reader refuses it whole. The report says it could not read the file
/// rather than answering "no transitions" — the one thing a report must never
/// do is call an unknown a none.
#[test]
fn a_file_the_raw_reader_cannot_read_is_an_error() {
    let session = Session::open(fixture("xref-stream.pdf")).expect("open");
    let problem = pagify_shell::transitions::report(&session).expect_err("must refuse");
    assert!(
        problem.to_string().contains("cross-reference stream"),
        "{problem}"
    );
}

// ---------------------------------------------------------------------------
// calcorder
// ---------------------------------------------------------------------------

/// A form whose calculation order is subtotal then total — deliberately not
/// the order the fields appear in `/Fields`, so the order read back can only
/// have come from `/CO`.
fn calculated_form() -> Vec<u8> {
    pdf(&[
        "<< /Type /Catalog /Pages 2 0 R /AcroForm 4 0 R >>",
        "<< /Type /Pages /Kids [3 0 R] /Count 1 >>",
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 200 300] /Contents 8 0 R >>",
        "<< /Fields [5 0 R 6 0 R] /CO [6 0 R 5 0 R] >>",
        "<< /FT /Tx /T (total) /Rect [10 10 100 30] >>",
        "<< /FT /Tx /T (subtotal) /Parent 7 0 R >>",
        "<< /T (invoice) /Kids [6 0 R] >>",
        "<< /Length 0 >>\nstream\n\nendstream",
    ])
}

#[test]
fn the_calculation_order_is_read_from_the_file() {
    let path = temp("calcorder", &calculated_form());
    let session = Session::open(&path).expect("open the calculated fixture");
    let lines = pagify_shell::calcorder::report(&session).expect("report");
    std::fs::remove_file(&path).ok();

    assert_eq!(lines[0], "1. invoice.subtotal", "{lines:?}");
    assert_eq!(lines[1], "2. total", "{lines:?}");
    assert!(
        lines.iter().any(|l| l.contains("2 field(s) in calculation order")),
        "{lines:?}"
    );
}

/// The shipped form fixture has an `/AcroForm` and no `/CO`: a form with no
/// stored calculation order, which is a different answer from "no form".
#[test]
fn a_form_without_a_calculation_order_answers_none() {
    let session = Session::open(fixture("form-fields.pdf")).expect("open");
    let lines = pagify_shell::calcorder::report(&session).expect("report");

    assert_eq!(lines.len(), 1, "{lines:?}");
    assert!(lines[0].contains("no calculation order"), "{lines:?}");
    assert!(lines[0].contains("no /CO entry"), "{lines:?}");
    assert!(lines[0].contains("Read from the file on disk"), "{lines:?}");
}

/// The same refusal as the transitions report: a file the raw reader cannot
/// read is an error, never a "none".
#[test]
fn an_unreadable_file_is_an_error_not_a_none() {
    let session = Session::open(fixture("xref-stream.pdf")).expect("open");
    let problem = pagify_shell::calcorder::report(&session).expect_err("must refuse");
    assert!(
        problem.to_string().contains("cross-reference stream"),
        "{problem}"
    );
}
