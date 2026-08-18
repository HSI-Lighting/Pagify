//! open → command → save → reopen → assert.
//!
//! The primary defence for the whole write path. A command that appears to work
//! but does not survive a save is the most common editing bug there is, and it is
//! invisible to any test that stops before the reopen.
//!
//! **Written before the code they test**, so a week of page-tree wiring has a
//! signal from day one rather than day five. They are `#[ignore]`d rather than
//! left red: a suite that always fails teaches people to stop reading it. Run the
//! remaining work with `cargo test -- --ignored`, and delete an `#[ignore]` as
//! each operation lands.
//!
//! `fixtures_are_present_and_readable` is **not** ignored, and it is the one that
//! matters most right now: it separates "not implemented yet" from "the fixtures
//! or the binding are broken", which look identical from a failure message.
//!
//! Requires a desktop PDFium of the pinned build:
//!
//! ```text
//! PAGIFY_PDFIUM_LIB=/tmp/pdfw/bin/pdfium.dll cargo test --test round_trip
//! ```
//!
//! Every fixture page has a distinct width, so the page tree alone says which
//! page ended up where — no text, no pixels.

use pdf_core::command::{Command, CommandHistory};
use pdf_core::document::Document;

mod harness;
use harness::{open_fixture, page_widths, save_and_reopen, skip_without_pdfium};

#[test]
fn fixtures_are_present_and_readable() {
    let Some(pdfium) = skip_without_pdfium() else {
        return;
    };
    let doc = open_fixture(&pdfium, "pages-ladder.pdf");
    assert_eq!(5, doc.page_count());
    assert_eq!(
        vec![200, 250, 300, 350, 400],
        page_widths(&doc),
        "the ladder is what every other test reads positions from",
    );
}

#[test]
#[ignore = "Task 4: DocumentMut is not implemented against PDFium yet"]
fn a_deleted_page_is_still_gone_after_a_save() {
    let Some(pdfium) = skip_without_pdfium() else {
        return;
    };
    let mut doc = open_fixture(&pdfium, "pages-ladder.pdf");

    let mut history = CommandHistory::default();
    history
        .execute(
            Command::DeletePage { index: 2 },
            doc.as_document_mut().expect("the document must be mutable"),
        )
        .expect("delete");

    let reopened = save_and_reopen(&pdfium, &mut doc);
    assert_eq!(vec![200, 250, 350, 400], page_widths(&reopened));
}

#[test]
#[ignore = "Task 4: DocumentMut is not implemented against PDFium yet"]
fn a_reorder_survives_a_save() {
    let Some(pdfium) = skip_without_pdfium() else {
        return;
    };
    let mut doc = open_fixture(&pdfium, "pages-ladder.pdf");

    let mut history = CommandHistory::default();
    history
        .execute(
            // A rotation, not a swap: a swap is its own inverse and would pass
            // even if the permutation were applied backwards.
            Command::ReorderPages {
                order: vec![1, 2, 3, 4, 0],
            },
            doc.as_document_mut().expect("mutable"),
        )
        .expect("reorder");

    let reopened = save_and_reopen(&pdfium, &mut doc);
    assert_eq!(vec![400, 200, 250, 300, 350], page_widths(&reopened));
}

#[test]
#[ignore = "Task 4: DocumentMut is not implemented against PDFium yet"]
fn an_inserted_blank_page_survives_a_save() {
    let Some(pdfium) = skip_without_pdfium() else {
        return;
    };
    let mut doc = open_fixture(&pdfium, "pages-ladder.pdf");

    let mut history = CommandHistory::default();
    history
        .execute(
            Command::InsertBlankPage {
                at: 1,
                width_pt: 999.0,
                height_pt: 400.0,
            },
            doc.as_document_mut().expect("mutable"),
        )
        .expect("insert");

    let reopened = save_and_reopen(&pdfium, &mut doc);
    assert_eq!(vec![200, 999, 250, 300, 350, 400], page_widths(&reopened));
}

/// Undo has to be reversible *through* a save, not merely in memory.
#[test]
#[ignore = "Task 4: DocumentMut is not implemented against PDFium yet"]
fn a_deletion_undone_before_saving_leaves_the_document_untouched() {
    let Some(pdfium) = skip_without_pdfium() else {
        return;
    };
    let mut doc = open_fixture(&pdfium, "pages-ladder.pdf");

    let mut history = CommandHistory::default();
    history
        .execute(Command::DeletePage { index: 0 }, doc.as_document_mut().expect("mutable"))
        .expect("delete");
    history
        .undo(doc.as_document_mut().expect("mutable"))
        .expect("undo")
        .expect("something to undo");

    let reopened = save_and_reopen(&pdfium, &mut doc);
    assert_eq!(vec![200, 250, 300, 350, 400], page_widths(&reopened));
}

/// The §4.1 property, and the one the easy implementation quietly breaks.
///
/// `pdfium-render`'s `save_to_writer` hardcodes its flags to zero, which is a full
/// rewrite — measured in `examples/save_probe.rs`. A save built on it would pass
/// every other test in this file while relocating every object in the file, which
/// is exactly what destroys a signature's byte range.
#[test]
#[ignore = "Task 4: DocumentMut is not implemented against PDFium yet"]
fn an_incremental_save_appends_and_leaves_the_original_bytes_alone() {
    let Some(pdfium) = skip_without_pdfium() else {
        return;
    };
    let original = harness::fixture_bytes("pages-ladder.pdf");
    let mut doc = open_fixture(&pdfium, "pages-ladder.pdf");

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

    let mut saved = Vec::new();
    doc.as_document_mut()
        .expect("mutable")
        .save_incremental(&mut saved)
        .expect("save");

    assert!(
        saved.len() > original.len(),
        "an incremental save appends; it cannot come out smaller",
    );
    assert_eq!(
        original[..],
        saved[..original.len()],
        "the original bytes must survive verbatim, or every signature over them breaks",
    );
}
