//! The Tags panel's reader, against a fixture made for it: a Document tag
//! holding an H1 and an (alt-texted) P, and untagged documents that must
//! read empty.
//!
//! ```text
//! PAGIFY_PDFIUM_LIB=<pdfium> cargo test -p pagify_shell --test tags
//! ```

use pagify_shell::Session;

fn fixture(name: &str) -> String {
    format!(
        "{}/../../../rust/pdf_core/fixtures/{name}",
        env!("CARGO_MANIFEST_DIR")
    )
}

#[test]
fn the_tagged_fixture_reads_its_structure_tree() {
    let session = Session::open(fixture("tagged.pdf")).expect("open");

    let outline = session.structure_outline(0).expect("outline");

    assert_eq!(outline.len(), 1, "one root: {outline:?}");
    let document = &outline[0];
    assert_eq!(document.kind, "Document");
    let kinds: Vec<&str> = document.children.iter().map(|c| c.kind.as_str()).collect();
    assert_eq!(kinds, ["H1", "P", "Figure", "Figure"], "the Document's children: {outline:?}");
}

#[test]
fn the_alt_text_reading_finds_the_described_figure_and_the_bare_one() {
    let session = Session::open(fixture("tagged.pdf")).expect("open");

    let (with_alt, missing) = session.alt_text_report(&[0]).expect("report");

    assert_eq!(with_alt, 1, "one figure carries alternate text");
    assert_eq!(missing, vec![(0, "Figure".to_string())], "one figure does not");
}

#[test]
fn an_untagged_page_has_no_alt_text_reading() {
    let session = Session::open(fixture("two-column.pdf")).expect("open");

    let (with_alt, missing) = session.alt_text_report(&[0]).expect("report");

    assert_eq!((with_alt, missing.len()), (0, 0));
}

#[test]
fn an_untagged_page_reads_no_tags() {
    let session = Session::open(fixture("two-column.pdf")).expect("open");

    let outline = session.structure_outline(0).expect("outline");

    assert!(outline.is_empty(), "an untagged page reported tags: {outline:?}");
}
