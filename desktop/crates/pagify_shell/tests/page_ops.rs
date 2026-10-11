//! The page phase's two commands — `replacepage` and `interleave` — against
//! the width-ladder fixtures.
//!
//! Every page in these fixtures has a distinct width on purpose (see the
//! fixtures README), so *where* each page landed is readable from the page
//! tree alone: no text to extract and no pixels to compare.
//!
//! ```text
//! PAGIFY_PDFIUM_LIB=<pdfium> cargo test -p pagify_shell --test page_ops
//! ```

use pagify_shell::Session;

fn fixture(name: &str) -> String {
    format!(
        "{}/../../../rust/pdf_core/fixtures/{name}",
        env!("CARGO_MANIFEST_DIR")
    )
}

/// The width of every page, in round points — the fixture's own fingerprint.
fn widths(session: &Session) -> Vec<u32> {
    (0..session.page_count().expect("page count"))
        .map(|i| session.page_size(i).expect("page size").width_pt.round() as u32)
        .collect()
}

#[test]
fn replacing_a_page_puts_the_source_page_where_it_was() {
    let session = Session::open(fixture("pages-ladder.pdf")).expect("open");
    let before = session.page_count().expect("count");

    // The 595pt page of `mixed-sizes` replaces the 250pt page.
    let total = session
        .replace_page_from(std::path::Path::new(&fixture("mixed-sizes.pdf")), &[0], 1)
        .expect("replace");

    assert_eq!(total, before, "one page in, one page out keeps the count");
    assert_eq!(widths(&session), vec![200, 595, 300, 350, 400]);
}

#[test]
fn replacing_with_more_pages_than_one_grows_the_document() {
    let session = Session::open(fixture("pages-ladder.pdf")).expect("open");

    // The 420pt and 842pt pages replace the 350pt one.
    let total = session
        .replace_page_from(std::path::Path::new(&fixture("mixed-sizes.pdf")), &[1, 3], 3)
        .expect("replace");

    assert_eq!(total, 6, "two pages replace one");
    assert_eq!(widths(&session), vec![200, 250, 300, 420, 842, 400]);
}

#[test]
fn interleaving_alternates_the_source_after_each_page() {
    let session = Session::open(fixture("pages-ladder.pdf")).expect("open");

    let total = session
        .interleave_from(
            std::path::Path::new(&fixture("mixed-sizes.pdf")),
            &[0, 1, 2, 3],
            0,
        )
        .expect("interleave");

    assert_eq!(total, 9);
    assert_eq!(widths(&session), vec![200, 595, 250, 420, 300, 612, 350, 842, 400]);
}

#[test]
fn interleaving_past_the_end_appends_the_rest() {
    let session = Session::open(fixture("single-page.pdf")).expect("open");

    // One page in the document, three in the source: the first alternates,
    // the rest are appended — they cannot be alternated with pages that are
    // not there.
    let total = session
        .interleave_from(std::path::Path::new(&fixture("pages-ladder.pdf")), &[0, 1, 2], 0)
        .expect("interleave");

    assert_eq!(total, 4);
    assert_eq!(widths(&session), vec![200, 200, 250, 300]);
}
