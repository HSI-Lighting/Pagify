//! Auto Create Bookmarks' titles, against fixtures that have text and
//! fixtures that do not.
//!
//! ```text
//! PAGIFY_PDFIUM_LIB=<pdfium> cargo test -p pagify_shell --test bookmarks
//! ```

use pagify_shell::{bookmarks, Session};

fn fixture(name: &str) -> String {
    format!(
        "{}/../../../rust/pdf_core/fixtures/{name}",
        env!("CARGO_MANIFEST_DIR")
    )
}

#[test]
fn every_text_page_names_a_bookmark() {
    let session = Session::open(fixture("two-column.pdf")).expect("open");

    let titles = bookmarks::heading_titles(&session).expect("titles");

    assert_eq!(titles.len(), session.page_count().expect("count"), "every page here has text");
    let mut pages: Vec<usize> = titles.iter().map(|(page, _)| *page).collect();
    pages.sort();
    assert_eq!(pages, (0..titles.len()).collect::<Vec<_>>(), "pages must be in order");
    assert!(
        titles.iter().all(|(_, title)| !title.trim().is_empty()),
        "an empty title was written: {titles:?}"
    );
}

#[test]
fn pages_with_no_text_get_no_bookmark() {
    let session = Session::open(fixture("quadrants.pdf")).expect("open");

    let titles = bookmarks::heading_titles(&session).expect("titles");

    assert!(titles.is_empty(), "a page of solid colours named a bookmark: {titles:?}");
}

#[test]
fn the_tagged_fixture_ships_its_own_contents() {
    let session = Session::open(fixture("tagged.pdf")).expect("open");

    let entries = session.bookmarks().expect("outline");

    assert_eq!(
        entries,
        vec![
            ("A Tagged Heading".to_string(), 0),
            ("The paragraph".to_string(), 0),
        ],
        "the file's own table of contents must read back entry for entry"
    );
}
