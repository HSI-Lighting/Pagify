//! A font written to type with is written once per document, not once per edit.
//!
//! # What this guards
//!
//! Typing letters a page's own subset font lacks (a Bold word, a font chosen by hand)
//! embeds a whole font program, about 158 KB. Every edit that needed the face embedded
//! it again, so a person who bolded one word after another grew the file by a font each
//! time. A face already in the document - on the page, or written for an earlier edit
//! on any page - is now pointed at, not written again.
//!
//! ```text
//! PAGIFY_PDFIUM_LIB=<pdfium> cargo test --release --test typing_font_reuse
//! ```

mod harness;
use harness::{check_both_saves, hand_built_pdf, helvetica_page, serial, skip_without_pdfium, HandPage};

use pdf_core::command::{Command, CommandHistory};
use pdf_core::document::pdfium_doc::PdfiumDocument;
use pdf_core::document::{Document, DocumentMut, SpanFont, SpanStyle, TextLineEdit, TextStyle};

/// A bundled Montserrat's bytes, and the name the engine calls it by.
fn montserrat() -> (Vec<u8>, String) {
    let path = format!("{}/../../desktop/third_party/fonts/Montserrat-Bold.ttf", env!("CARGO_MANIFEST_DIR"));
    let bytes = std::fs::read(&path).unwrap_or_else(|e| panic!("{path}: {e}"));
    let name = pdf_core::pdf::embed::face_name(&bytes).expect("a face name");
    (bytes, name)
}

/// `chars` of a line, in the face called `name`.
fn in_face(name: &str, chars: std::ops::Range<usize>) -> SpanStyle {
    SpanStyle {
        chars,
        font: Some(SpanFont::Face(name.into())),
        size: None,
        color: None,
        char_spacing: None,
        width_pct: None,
        rise: None,
        underline: false,
        strike: false,
    }
}

fn retype(first: usize, text: &str, spans: Vec<SpanStyle>) -> TextLineEdit {
    TextLineEdit::Retype { first, text: text.into(), style: TextStyle::default(), remove: Vec::new(), justify_to: None, spans }
}

/// One page, two lines, in Helvetica.
fn one_page(font: Vec<u8>) -> PdfiumDocument {
    let page = helvetica_page(b"BT /F1 12 Tf 72 700 Td (Hello there) Tj 0 -14 Td (next line) Tj ET");
    let mut doc = PdfiumDocument::open_bytes(page, None).expect("open");
    doc.set_typing_fonts(vec![font]);
    doc
}

/// Two pages, each with a line, sharing one Helvetica.
fn two_pages(font: Vec<u8>) -> PdfiumDocument {
    const RESOURCES: &str = "<< /Font << /F1 7 0 R >> >>";
    let pages = [
        HandPage { content: "BT /F1 12 Tf 72 700 Td (Hello there) Tj ET", resources: RESOURCES, extra: "" },
        HandPage { content: "BT /F1 12 Tf 72 700 Td (Second page) Tj ET", resources: RESOURCES, extra: "" },
    ];
    let bytes = hand_built_pdf(&pages, &["<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>"]);
    let mut doc = PdfiumDocument::open_bytes(bytes, None).expect("open");
    doc.set_typing_fonts(vec![font]);
    doc
}

fn saved(doc: &mut PdfiumDocument) -> Vec<u8> {
    let mut bytes = Vec::new();
    doc.save_full_copy(&mut bytes).expect("save");
    bytes
}

/// How many font programs the saved file holds.
fn programs(bytes: &[u8]) -> usize {
    bytes.windows(9).filter(|w| *w == b"FontFile2").count()
}

fn font_name_of(doc: &PdfiumDocument, page: usize, words: &str) -> String {
    let run = doc.text_runs_unfiltered(page).expect("runs").into_iter().find(|r| r.text.trim() == words).expect("the words");
    doc.run_font_names(page).expect("font names").get(&run.object).cloned().unwrap_or_default()
}

#[test]
fn a_second_edit_in_the_same_face_adds_no_second_copy() {
    let Some(_) = skip_without_pdfium() else { return };
    let _serial = serial();
    let (font, name) = montserrat();
    let mut doc = one_page(font);
    doc.replace_text_lines(0, &[retype(0, "Hello there", vec![in_face(&name, 6..11)])]).unwrap_or_else(|e| panic!("refused: {e}"));
    let first = saved(&mut doc);
    doc.replace_text_lines(0, &[retype(1, "next line", vec![in_face(&name, 5..9)])]).unwrap_or_else(|e| panic!("refused: {e}"));
    let second = saved(&mut doc);

    assert!(first.len() > 100_000, "test assumption: the face is embedded whole ({} bytes)", first.len());
    assert!(second.len() < first.len() + 10_000, "the second edit grew the file by {} bytes", second.len() - first.len());
    assert_eq!(programs(&second), 1, "font programs in the file");
    assert!(font_name_of(&doc, 0, "line").contains(&name), "the second word is not in the face: {:?}", font_name_of(&doc, 0, "line"));
}

#[test]
fn an_edit_on_another_page_reuses_the_copy() {
    let Some(_) = skip_without_pdfium() else { return };
    let _serial = serial();
    let (font, name) = montserrat();
    let mut doc = two_pages(font);
    doc.replace_text_lines(0, &[retype(0, "Hello there", vec![in_face(&name, 6..11)])]).unwrap_or_else(|e| panic!("refused: {e}"));
    let first = saved(&mut doc);
    doc.replace_text_lines(1, &[retype(0, "Second page", vec![in_face(&name, 7..11)])]).unwrap_or_else(|e| panic!("refused: {e}"));
    let second = saved(&mut doc);

    assert!(first.len() > 100_000, "test assumption: the face is embedded whole ({} bytes)", first.len());
    assert!(second.len() < first.len() + 10_000, "the second page's edit grew the file by {} bytes", second.len() - first.len());
    assert_eq!(programs(&second), 1, "font programs in the file");
    assert!(font_name_of(&doc, 1, "page").contains(&name), "the word on page 2 is in {:?}", font_name_of(&doc, 1, "page"));
    let mut boxed: Box<dyn Document> = Box::new(doc);
    check_both_saves(&mut boxed, "one copy of a font on two pages");
}

/// Undo takes the first edit's font out with it; the next edit must write the face again
/// rather than point at an object that is no longer there.
#[test]
fn after_undoing_the_first_edit_the_next_one_still_writes_its_font() {
    let Some(_) = skip_without_pdfium() else { return };
    let _serial = serial();
    let (font, name) = montserrat();
    let mut doc = one_page(font);
    let mut history = CommandHistory::new(8);
    let edit = |first: usize, text: &str, chars: std::ops::Range<usize>| Command::ReplaceTextLines {
        page_index: 0,
        edits: vec![retype(first, text, vec![in_face(&name, chars)])],
    };
    history.execute(edit(0, "Hello there", 6..11), &mut doc).unwrap_or_else(|e| panic!("refused: {e}"));
    history.undo(&mut doc).expect("undo").expect("a step to undo");
    history.execute(edit(1, "next line", 5..9), &mut doc).unwrap_or_else(|e| panic!("refused: {e}"));

    assert!(font_name_of(&doc, 0, "line").contains(&name), "the word is in {:?}", font_name_of(&doc, 0, "line"));
    let bytes = saved(&mut doc);
    assert_eq!(programs(&bytes), 1, "font programs in the file");
    let mut boxed: Box<dyn Document> = Box::new(doc);
    check_both_saves(&mut boxed, "a face written again after an undo");
}
