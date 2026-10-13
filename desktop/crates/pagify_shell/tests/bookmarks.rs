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
fn the_flat_reader_stops_at_the_top_level_it_promises() {
    let session = Session::open(fixture("tagged.pdf")).expect("open");

    let entries = session.bookmarks().expect("outline");

    // Top level only, and only what a click can go to: "The figures" is a
    // grouping heading with no destination of its own, so the flat list
    // skips it while the tree keeps it. Children never leak in.
    assert_eq!(
        entries,
        vec![("A Tagged Heading".to_string(), 0)],
        "bookmarks() is the flat, jumpable top level it promises"
    );
}

#[test]
fn the_outline_reads_the_whole_tree() {
    use pdf_core::document::OutlineEntry;
    let session = Session::open(fixture("tagged.pdf")).expect("open");

    let tree = session.outline().expect("tree");

    assert_eq!(tree.len(), 2, "two top-level entries: {tree:?}");
    let heading = &tree[0];
    assert_eq!(heading.title, "A Tagged Heading");
    assert_eq!(heading.page, Some(0));
    assert_eq!(
        heading.children,
        vec![OutlineEntry {
            title: "The paragraph".into(),
            page: Some(0),
            children: Vec::new(),
        }],
        "the heading's child"
    );

    let figures = &tree[1];
    assert_eq!(figures.page, None, "a grouping heading goes nowhere itself");
    assert_eq!(figures.children.len(), 1);
    assert_eq!(figures.children[0].title, "Deeper still");
    assert_eq!(figures.children[0].page, Some(0));
}
