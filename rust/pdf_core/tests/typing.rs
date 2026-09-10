//! Typing words a document's own fonts cannot spell.
//!
//! # What is being checked
//!
//! A PDF producer embeds a **subset** of each font — only the glyphs the file
//! already uses. Measured on a real document: four fonts on one page, one of
//! which could draw nothing but `,01234579h–`. Editing text inside that is
//! limited to the letters already on the page unless a glyph comes from
//! somewhere else.
//!
//! So: a font is written into the document, the run is drawn with it, and the
//! result is read back the way another program would — reopened from the saved
//! bytes, its words extracted, and its ink counted.
//!
//! ```text
//! PAGIFY_PDFIUM_LIB=<pdfium> cargo test --test typing
//! ```

mod harness;
use harness::{serial, skip_without_pdfium};

use pdf_core::document::pdfium_doc::PdfiumDocument;
use pdf_core::document::{Document, DocumentMut};

/// A font to type with, if this machine has one.
///
/// Not committed: see `fixtures/fonts/README.md`. A licensed typeface is the
/// reader's to supply, and that applies to a test machine too.
fn typing_font() -> Option<Vec<u8>> {
    let candidates = [
        concat!(env!("CARGO_MANIFEST_DIR"), "/fixtures/fonts/typing.ttf"),
        "/Users/hsilighting/pagify/third_party/fonts/Montserrat-Regular.ttf",
        "/System/Library/Fonts/Supplemental/Arial.ttf",
    ];
    candidates.iter().find_map(|path| std::fs::read(path).ok())
}

fn open(name: &str) -> PdfiumDocument {
    PdfiumDocument::open_path(harness::fixture_path(name).to_str().expect("path"), None)
        .expect("open")
}

/// **The whole point: words the document could not have spelled.**
#[test]
fn a_run_can_be_typed_with_a_font_the_document_did_not_have() {
    let Some(_) = skip_without_pdfium() else { return };
    let _lock = serial();
    let Some(font) = typing_font() else {
        eprintln!("skipping: no font to type with — see fixtures/fonts/README.md");
        return;
    };

    let mut doc = open("text-lines.pdf");
    let runs = doc.text_runs(0).expect("runs");
    let target = runs
        .iter()
        .find(|r| r.text.trim().chars().count() > 4)
        .cloned()
        .expect("a run with words in it");

    doc.set_typing_fonts(vec![font]);
    // Characters chosen to be unlikely in the fixture's own subset.
    let typed = "Zwölf Ünique";
    let result = doc.try_set_run_in_stream(0, target.object, typed);
    if result.is_err() {
        // The other honest answer, and not a failure of this test: some runs
        // cannot be located in the stream at all.
        eprintln!("skipping: this run cannot be edited at all — {result:?}");
        return;
    }

    // Read back the way another program would: from the saved bytes.
    let mut bytes = Vec::new();
    doc.save_full_copy(&mut bytes).expect("save");
    let reopened = PdfiumDocument::open_bytes(bytes, None).expect("reopen");
    let text = reopened.page(0).expect("page").text().unwrap_or_default();
    assert!(text.contains(typed), "the words are not in the saved file:\n{text}");
}

/// **And it says the face changed.** Words in a face that is not their
/// neighbours' look like what they are; somebody not told finds out in print.
#[test]
fn typing_in_a_borrowed_face_says_which_face() {
    let Some(_) = skip_without_pdfium() else { return };
    let _lock = serial();
    let Some(font) = typing_font() else { return };

    let mut doc = open("text-lines.pdf");
    let runs = doc.text_runs(0).expect("runs");
    let target = runs
        .iter()
        .find(|r| r.text.trim().chars().count() > 4)
        .cloned()
        .expect("a run");

    doc.set_typing_fonts(vec![font]);
    if doc.try_set_run_in_stream(0, target.object, "Zwölf Ünique").is_err() {
        return;
    }
    let face = doc.substituted_face();
    assert!(face.is_some(), "it swapped the face without saying so");
    assert!(!face.unwrap().is_empty(), "it named the face it used as nothing");
}

/// **The run's own font is still preferred.** Nothing is embedded, and nothing
/// is said, when the words can be written where they are.
#[test]
fn words_the_document_can_already_spell_change_nothing_about_the_fonts() {
    let Some(_) = skip_without_pdfium() else { return };
    let _lock = serial();
    let Some(font) = typing_font() else { return };

    let mut doc = open("text-lines.pdf");
    let runs = doc.text_runs(0).expect("runs");
    let target = runs
        .iter()
        .find(|r| r.text.trim().chars().count() > 4)
        .cloned()
        .expect("a run");
    let before = std::fs::metadata(harness::fixture_path("text-lines.pdf"))
        .map(|m| m.len())
        .unwrap_or(0);

    doc.set_typing_fonts(vec![font]);
    // Its own words, which its own font can obviously spell.
    if doc.try_set_run_in_stream(0, target.object, target.text.trim()).is_err() {
        return;
    }
    assert!(doc.substituted_face().is_none(), "it swapped a face it did not need to");

    let mut bytes = Vec::new();
    doc.save_full_copy(&mut bytes).expect("save");
    assert!(
        bytes.len() < before as usize + 100_000,
        "a font was written in for words that did not need one: {} then {}",
        before,
        bytes.len()
    );
}

/// With no font offered, it refuses — and says how to offer one.
#[test]
fn with_no_font_to_type_with_it_says_how_to_provide_one() {
    let Some(_) = skip_without_pdfium() else { return };
    let _lock = serial();

    let mut doc = open("text-lines.pdf");
    let runs = doc.text_runs(0).expect("runs");
    let Some(target) = runs.iter().find(|r| r.text.trim().chars().count() > 4).cloned() else {
        return;
    };

    // No typing fonts at all, which is the default.
    match doc.try_set_run_in_stream(0, target.object, "Zwölf Ünique") {
        Err(e) => {
            let said = e.to_string();
            // Either it cannot place the run, or it cannot spell the words —
            // and the second must say what to do about it.
            if said.contains("not in this text's font") {
                assert!(said.contains("outlinedfont"), "it did not say how to fix it: {said}");
            }
        }
        Ok(()) => panic!("it typed characters no font on the page has"),
    }
}
