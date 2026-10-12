//! `compare`'s verdicts, against fixtures whose relationship is the point:
//! `two-column.pdf` and `shredded.pdf` carry identical words in different
//! emission orders — that distinction is what the comparison must keep.
//!
//! ```text
//! PAGIFY_PDFIUM_LIB=<pdfium> cargo test -p pagify_shell --test compare
//! ```

use pagify_shell::{compare, Session};

fn fixture(name: &str) -> String {
    format!(
        "{}/../../../rust/pdf_core/fixtures/{name}",
        env!("CARGO_MANIFEST_DIR")
    )
}

#[test]
fn a_document_against_itself_is_identical_on_every_page() {
    let session = Session::open(fixture("two-column.pdf")).expect("open");
    let pages = session.page_count().expect("count");

    let comparisons = compare::compare(&session, &session, pages).expect("compare");

    assert_eq!(comparisons.len(), pages);
    assert!(
        comparisons.iter().all(|c| matches!(c.verdict, compare::Verdict::Identical)),
        "a document differed from itself"
    );
}

#[test]
fn the_same_words_in_a_different_order_are_not_called_identical() {
    let here = Session::open(fixture("two-column.pdf")).expect("open");
    let there = Session::open(fixture("shredded.pdf")).expect("open");
    let pages = here.page_count().expect("count").min(there.page_count().expect("count"));

    let comparisons = compare::compare(&here, &there, pages).expect("compare");

    assert!(
        comparisons
            .iter()
            .all(|c| matches!(c.verdict, compare::Verdict::SameWordsDifferentOrder)),
        "the fixtures' whole reason for existing is this distinction: {:?}",
        comparisons.iter().map(compare::describe).collect::<Vec<_>>()
    );
    assert!(compare::describe(&comparisons[0]).contains("different order"));
}

#[test]
fn a_text_page_against_a_blank_one_lists_the_words_that_only_one_has() {
    let here = Session::open(fixture("two-column.pdf")).expect("open");
    let there = Session::open(fixture("pages-ladder.pdf")).expect("open");

    let comparisons = compare::compare(&here, &there, 1).expect("compare");

    match &comparisons[0].verdict {
        compare::Verdict::Different { only_here, only_there } => {
            assert!(!only_here.is_empty(), "the text page's words went missing");
            assert!(only_there.is_empty(), "a blank page has no words of its own");
        }
        _ => panic!("a text page against a blank one was not Different"),
    }
    let line = compare::describe(&comparisons[0]);
    assert!(line.contains("only here"), "no listing: {line}");
}
