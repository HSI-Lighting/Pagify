//! `wordcount`'s counting, at the shell level: a page of text counts its
//! words, a page of nothing counts none.
//!
//! ```text
//! PAGIFY_PDFIUM_LIB=<pdfium> cargo test -p pagify_shell --test text_stats
//! ```

use pagify_shell::{stats, Session};

fn fixture(name: &str) -> String {
    format!(
        "{}/../../../rust/pdf_core/fixtures/{name}",
        env!("CARGO_MANIFEST_DIR")
    )
}

#[test]
fn a_page_of_text_counts_its_words() {
    let session = Session::open(fixture("two-column.pdf")).expect("open");

    let (words, characters) = stats::words_and_characters(&session, &[0]).expect("count");

    assert!(words > 20, "two columns of text counted {words} words");
    assert!(characters > words * 3, "characters ({characters}) should exceed words ({words})");
}

#[test]
fn a_page_with_no_text_counts_none() {
    let session = Session::open(fixture("quadrants.pdf")).expect("open");

    let (words, characters) = stats::words_and_characters(&session, &[0]).expect("count");

    assert_eq!((words, characters), (0, 0));
}
