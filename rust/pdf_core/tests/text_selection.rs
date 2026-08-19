//! Per-character text geometry, against real PDFium.
//!
//! Selection is the one feature that cannot be built on runs. A run is a whole
//! line, so a selection made of runs can only begin and end at a line — dragging
//! across half a sentence would copy both lines it touched, whole. These tests
//! are about the contract that makes character-accurate selection possible: the
//! text and the boxes are one walk of the page, aligned, in reading order.

mod harness;

use harness::{open_fixture, serial, skip_without_pdfium};

/// What `text-lines.pdf` says, in the order it says it.
const LINES: [&str; 3] = [
    "The quick brown fox",
    "jumps over the lazy dog",
    "Pack my box with five dozen jugs",
];

fn characters() -> pdf_core::document::PageCharacters {
    let _lock = serial();
    let pdfium = ();
    let doc = open_fixture(&pdfium, "text-lines.pdf");
    let page = doc.page(0).expect("page 0");
    page.characters().expect("characters")
}

#[test]
fn every_character_of_the_page_comes_back_in_reading_order() {
    let Some(()) = skip_without_pdfium() else {
        return;
    };

    let characters = characters();
    for line in LINES {
        assert!(
            characters.text.contains(line),
            "{line:?} is missing from {:?}",
            characters.text,
        );
    }

    // Order, not just presence: a selection is an interval over this sequence, so
    // the sequence being right is the whole foundation.
    let first = characters.text.find(LINES[0]).expect("first line");
    let second = characters.text.find(LINES[1]).expect("second line");
    let third = characters.text.find(LINES[2]).expect("third line");
    assert!(first < second && second < third, "the lines came back out of order");
}

#[test]
fn there_is_exactly_one_box_per_code_unit_of_the_text() {
    let Some(()) = skip_without_pdfium() else {
        return;
    };

    // The alignment *is* the contract. Off by one anywhere and every selection
    // past that point covers the wrong characters — which looks like a slightly
    // sloppy selection rather than a bug, and is the reason this is asserted
    // rather than assumed.
    let characters = characters();
    assert_eq!(
        characters.text.encode_utf16().count() * 4,
        characters.boxes.len(),
    );
}

#[test]
fn a_characters_box_is_where_that_character_is() {
    let Some(()) = skip_without_pdfium() else {
        return;
    };

    let characters = characters();
    let at = characters.text.find(LINES[0]).expect("first line");
    let box_of = |index: usize| {
        let start = index * 4;
        (
            characters.boxes[start],
            characters.boxes[start + 1],
            characters.boxes[start + 2],
            characters.boxes[start + 3],
        )
    };

    // The 'T' of "The", placed at x = 40 pt in the fixture.
    let (left, top, right, bottom) = box_of(at);
    assert!((left - 40.0).abs() < 4.0, "the line starts at {left}, expected ~40");
    assert!(right > left, "a character with no width");
    assert!(bottom > top, "a character with no height");

    // Characters run left to right along a line, at the same height.
    let (next_left, next_top, _, _) = box_of(at + 1);
    assert!(next_left > left, "the second character is not to the right of the first");
    assert!((next_top - top).abs() < 1.0, "the two are not on the same line");
}

#[test]
fn a_later_line_sits_below_an_earlier_one() {
    let Some(()) = skip_without_pdfium() else {
        return;
    };

    // Top-left origin with y increasing downwards — decision 4.4. Getting this
    // inverted would put every selection on the mirror image of the page, and it
    // is invisible on a page whose lines happen to be symmetric.
    let characters = characters();
    let top_of = |line: &str| {
        let at = characters.text.find(line).expect("line");
        characters.boxes[at * 4 + 1]
    };

    assert!(top_of(LINES[0]) < top_of(LINES[1]));
    assert!(top_of(LINES[1]) < top_of(LINES[2]));
}

#[test]
fn a_page_with_no_text_gives_nothing_rather_than_failing() {
    let Some(()) = skip_without_pdfium() else {
        return;
    };

    let _lock = serial();
    let pdfium = ();
    let doc = open_fixture(&pdfium, "quadrants.pdf");
    let page = doc.page(0).expect("page 0");
    let characters = page.characters().expect("characters");

    assert_eq!("", characters.text);
    assert!(characters.boxes.is_empty());
}

#[test]
fn the_boxes_cover_the_line_rather_than_each_glyphs_outline() {
    let Some(()) = skip_without_pdfium() else {
        return;
    };

    // Loose bounds, so a run of characters joins into an unbroken band. Tight
    // ones would make a selection a row of glyph-shaped bites, with the gaps
    // between letters showing through.
    let characters = characters();
    let at = characters.text.find(LINES[0]).expect("first line");

    let height = |index: usize| characters.boxes[index * 4 + 3] - characters.boxes[index * 4 + 1];
    // 'T' and 'h' are both tall; 'e' is not. With loose bounds all three are the
    // height of the line.
    let tall = height(at);
    let short = height(at + 2);
    assert!(
        (tall - short).abs() < 0.5,
        "characters have different heights ({tall} vs {short}), so these are tight bounds",
    );
}
