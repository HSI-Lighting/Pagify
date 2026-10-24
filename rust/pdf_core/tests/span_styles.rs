//! Per-character looks in a line edit — a `Retype`'s `spans`: how they travel,
//! what is refused before anything is written, and how they are written into the
//! page's own text.

mod harness;
use harness::{check_both_saves, helvetica_page, page_stream, page_with_fonts, serial, skip_without_pdfium};

use pdf_core::document::pdfium_doc::PdfiumDocument;
use pdf_core::document::{
    Color, Document, DocumentMut, RunTextLook, SpanFont, SpanStyle, TextLineEdit, TextRun, TextStyle,
};
use pdf_core::error::PdfError;
use pdf_core::pdf::content;

/// A span that asks for nothing yet — the fields are filled in per test.
fn span(chars: std::ops::Range<usize>) -> SpanStyle {
    SpanStyle {
        chars,
        font: None,
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

const HELLO: &[u8] = b"BT /F1 12 Tf 72 700 Td (Hello there) Tj 0 -14 Td (next line) Tj ET";
const RED: Color = Color { r: 200, g: 0, b: 0, a: 255 };

fn open(content: &[u8]) -> PdfiumDocument {
    PdfiumDocument::open_bytes(helvetica_page(content), None).expect("open")
}

/// Every text object of page 1, as the page says it now.
fn runs(doc: &PdfiumDocument) -> Vec<TextRun> {
    doc.text_runs_unfiltered(0).expect("runs")
}

fn run_saying<'a>(runs: &'a [TextRun], words: &str) -> &'a TextRun {
    runs.iter()
        .find(|r| r.text.trim() == words)
        .unwrap_or_else(|| panic!("no run says {words:?}: {:?}", runs.iter().map(|r| &r.text).collect::<Vec<_>>()))
}

/// Each character of page 1 with its box — left, top, right, bottom, in page
/// points from the top left. ASCII pages only: one box to a character.
fn boxes(doc: &PdfiumDocument) -> Vec<(char, [f32; 4])> {
    let characters = doc.page(0).expect("page").characters().expect("characters");
    characters.text.chars().zip(characters.boxes.chunks(4)).map(|(c, b)| (c, [b[0], b[1], b[2], b[3]])).collect()
}

/// The box round the word at `chars` of the page's text: left of its first
/// character, top of its first, right of its last, bottom of its first.
fn word_box(boxes: &[(char, [f32; 4])], chars: std::ops::Range<usize>) -> [f32; 4] {
    let first = boxes[chars.start].1;
    let last = boxes[chars.end - 1].1;
    [first[0], first[1], last[2], first[3]]
}

/// Object 0 written again with its own words and `spans`.
fn in_place(doc: &mut PdfiumDocument, text: &str, spans: Vec<SpanStyle>) {
    doc.replace_text_lines(0, &[retype(0, text, spans)]).unwrap_or_else(|e| panic!("refused: {e}"));
}

// ---- Task 1: spans in a line edit --------------------------------------------

#[test]
fn a_line_edit_saved_without_spans_still_reads() {
    let decoded: TextLineEdit = serde_json::from_str(r#"{"kind":"retype","first":3,"text":"Hello"}"#).expect("decode");
    let TextLineEdit::Retype { spans, .. } = decoded else { panic!("not a retype") };
    assert!(spans.is_empty());
}

#[test]
fn a_line_edit_with_spans_round_trips_through_json() {
    let edit = retype(
        0,
        "Hello there",
        vec![
            SpanStyle {
                chars: 6..11,
                font: Some(SpanFont::Face("Montserrat-Bold".into())),
                size: Some(20.0),
                color: Some(RED),
                char_spacing: Some(1.5),
                width_pct: Some(90.0),
                rise: Some(3.0),
                underline: true,
                strike: false,
            },
            SpanStyle { font: Some(SpanFont::LikeObject(4)), ..span(0..2) },
        ],
    );
    let json = serde_json::to_string(&edit).expect("encode");
    assert!(json.contains("\"spans\""), "{json}");
    let back: TextLineEdit = serde_json::from_str(&json).expect("decode");
    assert_eq!(back, edit);
    // An edit with no spans writes none: the form older readers know.
    let plain = serde_json::to_string(&retype(0, "Hello", Vec::new())).expect("encode");
    assert!(!plain.contains("spans"), "{plain}");
}

#[test]
fn a_span_outside_its_words_refuses_the_batch() {
    let Some(_) = skip_without_pdfium() else { return };
    let _serial = serial();
    let mut doc = open(b"BT /F1 12 Tf 72 700 Td (Hello ther) Tj ET");
    let before = page_stream(&mut doc, 0);
    for (what, spans) in [
        ("past the end", vec![span(3..40)]),
        ("empty", vec![span(3..3)]),
        ("overlapping", vec![span(0..5), span(4..8)]),
    ] {
        let error = doc
            .replace_text_lines(0, &[retype(0, "Hello ther", spans)])
            .err()
            .unwrap_or_else(|| panic!("{what}: accepted"));
        assert!(matches!(error, PdfError::InvalidArgument(_)), "{what}: {error}");
    }
    assert_eq!(page_stream(&mut doc, 0), before, "the page changed");
}

// ---- Task 2: a piece restyled in place ----------------------------------------

#[test]
fn a_span_restyled_in_place_reads_back_in_its_size() {
    let Some(_) = skip_without_pdfium() else { return };
    let _serial = serial();
    let mut doc = open(HELLO);
    in_place(&mut doc, "Hello there", vec![SpanStyle { size: Some(20.0), ..span(6..11) }]);
    let after = runs(&doc);
    assert!((run_saying(&after, "Hello").size - 12.0).abs() < 0.05, "{:?}", run_saying(&after, "Hello"));
    assert!((run_saying(&after, "there").size - 20.0).abs() < 0.05, "{:?}", run_saying(&after, "there"));
    // Where "there" starts: where the pen is after "Hello " at 12 pt — what the
    // page itself puts there when the two are drawn as two.
    let control = open(b"BT /F1 12 Tf 72 700 Td (Hello ) Tj (there) Tj 0 -14 Td (next line) Tj ET");
    let expected = run_saying(&runs(&control), "there").origin.x;
    let got = run_saying(&after, "there").origin.x;
    assert!((got - expected).abs() < 0.05, "there starts at {got}, the page puts it at {expected}");
}

#[test]
fn the_state_a_span_sets_is_put_back_after_the_piece() {
    let Some(_) = skip_without_pdfium() else { return };
    let _serial = serial();
    let mut doc = open(HELLO);
    let before = run_saying(&runs(&doc), "next line").clone();
    in_place(
        &mut doc,
        "Hello there",
        vec![SpanStyle {
            size: Some(20.0),
            color: Some(RED),
            char_spacing: Some(2.0),
            width_pct: Some(50.0),
            rise: Some(3.0),
            ..span(6..11)
        }],
    );
    let after = runs(&doc);
    let next = run_saying(&after, "next line");
    assert!((next.size - 12.0).abs() < 0.05, "{next:?}");
    assert_eq!(next.color, before.color, "the colour was not put back");
    assert!(
        (next.origin.x - before.origin.x).abs() < 0.05 && (next.origin.y - before.origin.y).abs() < 0.05,
        "{next:?}"
    );
    let width = |r: &TextRun| r.rect.right - r.rect.left;
    assert!((width(next) - width(&before)).abs() < 0.05, "next line is {} wide, was {}", width(next), width(&before));
    assert_eq!(run_saying(&after, "there").color, RED);
}

#[test]
fn restyling_in_place_keeps_the_spacing_numbers() {
    let Some(_) = skip_without_pdfium() else { return };
    let _serial = serial();
    let mut doc = open(b"BT /F1 12 Tf 72 700 Td [(Hel) -120 (lo the) -80 (re)] TJ 0 -14 Td (next line) Tj ET");
    let before = boxes(&doc);
    in_place(&mut doc, "Hello there", vec![SpanStyle { color: Some(RED), ..span(0..5) }]);
    let stream = page_stream(&mut doc, 0);
    let kerns: Vec<f32> = content::parse(&stream)
        .expect("parse")
        .iter()
        .filter(|o| o.shows_text())
        .flat_map(content::pieces)
        .filter_map(|p| match p {
            content::Piece::Kern(n) => Some(n),
            _ => None,
        })
        .collect();
    assert!(kerns.contains(&-120.0) && kerns.contains(&-80.0), "the spacing numbers were dropped: {kerns:?}");
    let after = boxes(&doc);
    assert_eq!(after.len(), before.len());
    for ((c, was), (_, is)) in before.iter().zip(&after) {
        for (a, b) in was.iter().zip(is) {
            assert!((a - b).abs() < 0.01, "{c:?} moved: {was:?} -> {is:?}");
        }
    }
    assert_eq!(run_saying(&runs(&doc), "Hello").color, RED);
}

#[test]
fn letter_spacing_width_and_rise_reach_the_page_in_points() {
    let Some(_) = skip_without_pdfium() else { return };
    let _serial = serial();
    // Everything drawn twice its size: a point asked for is a point on the page.
    let content = b"q 2 0 0 2 0 0 cm BT /F1 6 Tf 36 350 Td (Hello there) Tj ET Q";
    let there = 6..11;
    let was = word_box(&boxes(&open(content)), there.clone());
    let (width_was, top_was) = (was[2] - was[0], was[1]);

    let mut spaced = open(content);
    in_place(&mut spaced, "Hello there", vec![SpanStyle { char_spacing: Some(2.0), ..span(there.clone()) }]);
    let now = word_box(&boxes(&spaced), there.clone());
    let width = now[2] - now[0];
    assert!((width - (width_was + 4.0 * 2.0)).abs() < 0.05, "there is {width} wide spaced, was {width_was}");

    let mut narrow = open(content);
    in_place(&mut narrow, "Hello there", vec![SpanStyle { width_pct: Some(50.0), ..span(there.clone()) }]);
    let now = word_box(&boxes(&narrow), there.clone());
    let width = now[2] - now[0];
    assert!((width - width_was / 2.0).abs() < 0.05, "there is {width} wide at 50%, was {width_was}");

    let mut raised = open(content);
    in_place(&mut raised, "Hello there", vec![SpanStyle { rise: Some(3.0), ..span(there) }]);
    let now = word_box(&boxes(&raised), 6..11);
    assert!((top_was - now[1] - 3.0).abs() < 0.05, "the top of there moved from {top_was} to {}", now[1]);
}

#[test]
fn nothing_outside_the_piece_changes() {
    let Some(_) = skip_without_pdfium() else { return };
    let _serial = serial();
    let mut doc = open(HELLO);
    let before = page_stream(&mut doc, 0);
    let piece = content::parse(&before).expect("parse").into_iter().find(|o| o.shows_text()).expect("a show").span;
    in_place(&mut doc, "Hello there", vec![SpanStyle { size: Some(20.0), ..span(6..11) }]);
    let after = page_stream(&mut doc, 0);
    assert_eq!(&after[..piece.start], &before[..piece.start], "the bytes before the piece changed");
    // Every replacement is wrapped in a newline of its own and the stream ends with
    // one (`content::splice`, `page_content`): the bytes between are the page's.
    let after_body = after.strip_suffix(b"\n").unwrap_or(&after);
    assert!(
        after_body.ends_with(&before[piece.end..]),
        "the bytes after the piece changed: {:?}",
        String::from_utf8_lossy(after_body)
    );
    assert_ne!(after, before, "test assumption: the piece itself was rewritten");
}

// ---- Task 3: a retyped line written stretch by stretch, in several fonts -------

/// A page with Helvetica as `/F1` and Helvetica-Bold as `/F2`.
fn open_with_bold(content: &[u8]) -> PdfiumDocument {
    PdfiumDocument::open_bytes(page_with_fonts(content, &[("F1", "Helvetica"), ("F2", "Helvetica-Bold")], "", &[]), None)
        .expect("open")
}

/// A bundled Montserrat's bytes, and the name the engine calls it by.
fn montserrat(weight: &str) -> (Vec<u8>, String) {
    let path = format!("{}/../../desktop/third_party/fonts/Montserrat-{weight}.ttf", env!("CARGO_MANIFEST_DIR"));
    let bytes = std::fs::read(&path).unwrap_or_else(|e| panic!("{path}: {e}"));
    let name = pdf_core::pdf::embed::face_name(&bytes).expect("a face name");
    (bytes, name)
}

fn font_name_of(doc: &PdfiumDocument, words: &str) -> String {
    let all = runs(doc);
    let run = run_saying(&all, words);
    doc.run_font_names(0).expect("font names").get(&run.object).cloned().unwrap_or_default()
}

#[test]
fn a_retyped_line_with_a_span_writes_each_segment_in_its_look() {
    let Some(_) = skip_without_pdfium() else { return };
    let _serial = serial();
    let mut doc = open(HELLO);
    let before = run_saying(&runs(&doc), "next line").clone();
    in_place(&mut doc, "Hi there", vec![SpanStyle { size: Some(20.0), ..span(3..8) }]);
    let after = runs(&doc);
    assert!((run_saying(&after, "Hi").size - 12.0).abs() < 0.05, "{:?}", run_saying(&after, "Hi"));
    assert!((run_saying(&after, "there").size - 20.0).abs() < 0.05, "{:?}", run_saying(&after, "there"));
    // Where the page puts "there" when it is drawn as a piece of its own.
    let control = open(b"BT /F1 12 Tf 72 700 Td (Hi ) Tj (there) Tj 0 -14 Td (next line) Tj ET");
    let expected = run_saying(&runs(&control), "there").origin.x;
    let got = run_saying(&after, "there").origin.x;
    assert!((got - expected).abs() < 0.05, "there starts at {got}, the page puts it at {expected}");
    // The line after it is drawn as it was.
    let next = run_saying(&after, "next line");
    assert!((next.size - before.size).abs() < 0.05 && next.color == before.color, "{next:?}");
    assert!((next.origin.x - before.origin.x).abs() < 0.05 && (next.origin.y - before.origin.y).abs() < 0.05, "{next:?}");
}

#[test]
fn a_span_like_another_piece_is_written_in_that_pieces_font() {
    let Some(_) = skip_without_pdfium() else { return };
    let _serial = serial();
    let mut doc = open_with_bold(b"BT /F1 12 Tf 72 700 Td (Hello) Tj /F2 12 Tf ( there) Tj ET");
    assert_eq!(runs(&doc).len(), 2, "test assumption: the line is two pieces");
    doc.replace_text_lines(
        0,
        &[TextLineEdit::Retype {
            first: 0,
            text: "Hello there!".into(),
            style: TextStyle::default(),
            remove: vec![1],
            justify_to: None,
            spans: vec![SpanStyle { font: Some(SpanFont::LikeObject(1)), ..span(5..12) }],
        }],
    )
    .unwrap_or_else(|e| panic!("refused: {e}"));
    let hello = font_name_of(&doc, "Hello");
    let there = font_name_of(&doc, "there!");
    assert!(!hello.contains("Bold"), "Hello is in {hello}");
    assert!(there.contains("Bold"), "there! is in {there:?}, not the bold font of the piece it was like");
}

#[test]
fn two_lines_with_a_bold_span_embed_the_face_once() {
    let Some(_) = skip_without_pdfium() else { return };
    let _serial = serial();
    let (bold, name) = montserrat("Bold");
    let mut doc = open(b"BT /F1 12 Tf 72 700 Td (Alpha beta) Tj 0 -14 Td (Gamma delta) Tj ET");
    doc.set_typing_fonts(vec![bold]);
    let bold_span = |chars| SpanStyle { font: Some(SpanFont::Face(name.clone())), ..span(chars) };
    doc.replace_text_lines(0, &[retype(0, "Alpha beta", vec![bold_span(6..10)]), retype(1, "Gamma delta", vec![bold_span(6..11)])])
        .unwrap_or_else(|e| panic!("refused: {e}"));
    assert!(font_name_of(&doc, "beta").contains(&name), "beta is in {:?}", font_name_of(&doc, "beta"));
    assert!(font_name_of(&doc, "delta").contains(&name), "delta is in {:?}", font_name_of(&doc, "delta"));
    assert!(!font_name_of(&doc, "Alpha").contains(&name), "Alpha went bold too");

    let mut boxed: Box<dyn Document> = Box::new(doc);
    let mut saved = Vec::new();
    boxed.as_document_mut().expect("mutable").save_full_copy(&mut saved).expect("save");
    let programs = saved.windows(9).filter(|w| *w == b"FontFile2").count();
    assert_eq!(programs, 1, "the face is embedded {programs} times");
    check_both_saves(&mut boxed, "two lines, one embedded bold");
}

#[test]
fn bold_and_regular_spans_in_one_line_embed_both() {
    let Some(_) = skip_without_pdfium() else { return };
    let _serial = serial();
    let ((bold, bold_name), (regular, regular_name)) = (montserrat("Bold"), montserrat("Regular"));
    let mut doc = open(b"BT /F1 12 Tf 72 700 Td (Hello big world) Tj ET");
    doc.set_typing_fonts(vec![bold, regular]);
    let face = |name: &String, chars| SpanStyle { font: Some(SpanFont::Face(name.clone())), ..span(chars) };
    doc.replace_text_lines(0, &[retype(0, "Hello big world", vec![face(&bold_name, 6..9), face(&regular_name, 10..15)])])
        .unwrap_or_else(|e| panic!("refused: {e}"));
    assert!(font_name_of(&doc, "big").contains(&bold_name), "big is in {:?}", font_name_of(&doc, "big"));
    assert!(font_name_of(&doc, "world").contains(&regular_name), "world is in {:?}", font_name_of(&doc, "world"));
    assert!(!font_name_of(&doc, "Hello").contains("Montserrat"), "Hello left its own font");
    let mut boxed: Box<dyn Document> = Box::new(doc);
    check_both_saves(&mut boxed, "bold and regular in one line");
}

/// **A tracked line keeps its tracking across a stretch boundary.** Typed over, a line
/// the producer spaced by a number after every glyph is given the same spacing; cut
/// into stretches by a look that moves nothing (a colour), it must come out exactly as
/// it does uncut — the glyph at each cut included.
#[test]
fn a_colour_span_in_a_retyped_tracked_line_moves_nothing() {
    let Some(_) = skip_without_pdfium() else { return };
    let _serial = serial();
    let page = b"BT /F1 12 Tf 72 700 Td [(H) -100 (e) -100 (l) -100 (l) -100 (o) -100 ( ) -100 (w) -100 (o) -100 (r) -100 (l) -100 (d)] TJ ET";
    let mut uncut = open(page);
    let original = boxes(&uncut);
    in_place(&mut uncut, "Hallo world", Vec::new());
    let mut cut = open(page);
    in_place(&mut cut, "Hallo world", vec![SpanStyle { color: Some(RED), ..span(3..8) }]);
    let (uncut, cut) = (boxes(&uncut), boxes(&cut));
    assert_eq!(cut.len(), uncut.len());
    for ((c, a), (_, b)) in uncut.iter().zip(&cut) {
        for (x, y) in a.iter().zip(b) {
            assert!((x - y).abs() < 0.01, "{c:?} moved by the cut: {a:?} -> {b:?}");
        }
    }
    // The line really is tracked: untracked, "world" would start well to the left.
    let spaced = original.iter().map(|(_, b)| b[2]).fold(0.0f32, f32::max) - original[0].1[0];
    let plain = open(b"BT /F1 12 Tf 72 700 Td (Hello world) Tj ET");
    let tight = boxes(&plain).iter().map(|(_, b)| b[2]).fold(0.0f32, f32::max) - boxes(&plain)[0].1[0];
    assert!(spaced > tight + 5.0, "test assumption: the page's line is spaced out ({spaced} against {tight})");
}

/// **The spacing after a retyped piece's last glyph stays after the new words.** It is
/// what places the piece that follows on the line.
#[test]
fn the_spacing_after_a_retyped_pieces_last_glyph_stays_with_a_style() {
    let Some(_) = skip_without_pdfium() else { return };
    let _serial = serial();
    let page = b"BT /F1 12 Tf 72 700 Td [(Hello ) -3000] TJ (world) Tj ET";
    let mut plain = open(page);
    doc_retype(&mut plain, 0, "Hullo ", Vec::new());
    let mut styled = open(page);
    doc_retype(&mut styled, 0, "Hullo ", vec![SpanStyle { color: Some(RED), ..span(0..2) }]);
    let (plain, styled) = (boxes(&plain), boxes(&styled));
    assert_eq!(plain.len(), styled.len());
    for ((c, a), (_, b)) in plain.iter().zip(&styled) {
        for (x, y) in a.iter().zip(b) {
            assert!((x - y).abs() < 0.01, "{c:?} moved by the style: {a:?} -> {b:?}");
        }
    }
    let world = word_box(&styled, styled.len() - 5..styled.len());
    assert!(world[0] > 72.0 + 60.0, "test assumption: world sits far to the right ({})", world[0]);
}

/// Object `object` written again with `text` and `spans`.
fn doc_retype(doc: &mut PdfiumDocument, object: usize, text: &str, spans: Vec<SpanStyle>) {
    doc.replace_text_lines(0, &[retype(object, text, spans)]).unwrap_or_else(|e| panic!("refused: {e}"));
}

// ---- Task 4: a styled line kept to its width -------------------------------------

/// **A styled line of a justified paragraph keeps the width it had.** The line is
/// set with wide gaps after its spaces; retyped with a larger size on its last two
/// words it comes out wider, and the gaps close until it spans what it did. One of
/// those gaps is in the larger stretch, so a number that is right for one stretch's
/// units is wrong for the other's.
#[test]
fn a_styled_line_stretched_to_its_width_spans_it() {
    let Some(_) = skip_without_pdfium() else { return };
    let _serial = serial();
    let mut doc = open(b"BT /F1 12 Tf 72 700 Td [(alpha ) -500 (beta ) -500 (gamma)] TJ 0 -14 Td (second line) Tj ET");
    let before = runs(&doc);
    let (line, second) = (before[0].clone(), run_saying(&before, "second line").clone());
    let width = line.rect.right - line.rect.left;
    assert_eq!(before.len(), 2, "test assumption: the line is one piece");

    doc.replace_text_lines(
        0,
        &[TextLineEdit::Retype {
            first: 0,
            text: "alpha beta gamma".into(),
            style: TextStyle::default(),
            remove: Vec::new(),
            justify_to: Some(width),
            spans: vec![SpanStyle { size: Some(15.0), ..span(6..16) }],
        }],
    )
    .unwrap_or_else(|e| panic!("refused: {e}"));

    let after = runs(&doc);
    assert_eq!(after.len(), 3, "the line is two stretches and the line after it: {:?}", after.iter().map(|r| &r.text).collect::<Vec<_>>());
    let spanned = after[1].rect.right - after[0].rect.left;
    assert!((spanned - width).abs() < 0.05, "the styled line spans {spanned} pt, the line was {width} pt");
    assert!((after[0].rect.left - line.rect.left).abs() < 0.05, "the line moved");
    assert!((after[1].size - 15.0).abs() < 0.05 && (after[0].size - 12.0).abs() < 0.05, "{:?}", after.iter().map(|r| r.size).collect::<Vec<_>>());
    let next = run_saying(&after, "second line");
    assert!((next.origin.x - second.origin.x).abs() < 0.05 && (next.origin.y - second.origin.y).abs() < 0.05, "{next:?}");
}

/// **Objects after a styled line are numbered higher in the measuring copy** — it is a
/// show operator for each stretch — so a plain line kept to its width after it must be
/// measured by its own object, not the styled line's.
#[test]
fn a_plain_line_kept_to_its_width_after_a_styled_one_is_measured_by_its_own_object() {
    let Some(_) = skip_without_pdfium() else { return };
    let _serial = serial();
    let mut doc = open(
        b"BT /F1 12 Tf 72 700 Td [(alpha ) -500 (beta ) -500 (gamma)] TJ 0 -14 Td [(delta ) -500 (epsilon ) -500 (zeta)] TJ ET",
    );
    let before = runs(&doc);
    assert_eq!(before.len(), 2, "test assumption: two lines of one piece each");
    let width = |r: &TextRun| r.rect.right - r.rect.left;
    let (first_width, second_width) = (width(&before[0]), width(&before[1]));

    doc.replace_text_lines(
        0,
        &[
            TextLineEdit::Retype {
                first: 0,
                text: "alpha beta gamma".into(),
                style: TextStyle::default(),
                remove: Vec::new(),
                justify_to: Some(first_width),
                spans: vec![SpanStyle { size: Some(15.0), ..span(6..16) }],
            },
            TextLineEdit::Retype {
                first: 1,
                text: "delta epsilon zetas".into(),
                style: TextStyle::default(),
                remove: Vec::new(),
                justify_to: Some(second_width),
                spans: Vec::new(),
            },
        ],
    )
    .unwrap_or_else(|e| panic!("refused: {e}"));
    let after = runs(&doc);
    assert_eq!(after.len(), 3, "{:?}", after.iter().map(|r| &r.text).collect::<Vec<_>>());
    let styled = after[1].rect.right - after[0].rect.left;
    assert!((styled - first_width).abs() < 0.05, "the styled line spans {styled}, was {first_width}");
    assert!((width(&after[2]) - second_width).abs() < 0.05, "the plain line spans {}, was {second_width}", width(&after[2]));
    assert!(clean_words(&after[2]) == "delta epsilon zetas", "{:?}", after[2].text);
}

fn clean_words(run: &TextRun) -> String {
    run.text.trim().to_string()
}

// ---- Task 5: underlines and strike-throughs ----------------------------------------

use pdf_core::document::{DrawnKind, DrawnObject};

/// The shapes page 1 draws — none, before an edit of these pages: only the rules.
fn shapes(doc: &PdfiumDocument) -> Vec<DrawnObject> {
    doc.drawn_objects(0).expect("drawn").into_iter().filter(|d| d.kind == DrawnKind::Shape).collect()
}

/// What the page's content stream says between the first `/PagifyRule` mark and its end.
fn rule_ops(doc: &mut PdfiumDocument) -> String {
    let stream = String::from_utf8_lossy(&page_stream(doc, 0)).to_string();
    let from = stream.find("/PagifyRule").expect("no rule mark in the stream");
    let to = stream[from..].find("EMC").expect("the rule mark is never closed") + from;
    stream[from..to].to_string()
}

/// Where the face the run is drawn in puts its lines — read the way the engine reads it:
/// from the font program PDFium has for the run (a system font it substitutes for a
/// base-14 name, here), and a tenth of an em below / three tenths above, a twentieth thick,
/// where there is none.
fn faces_lines(doc: &PdfiumDocument, run: &TextRun) -> pdf_core::text::LineMetrics {
    doc.run_font_data(0, run.object)
        .ok()
        .flatten()
        .and_then(|program| pdf_core::text::line_metrics_of(&program))
        .unwrap_or(pdf_core::text::LineMetrics::FALLBACK)
}

#[test]
fn an_underline_is_drawn_under_its_words() {
    let Some(_) = skip_without_pdfium() else { return };
    let _serial = serial();
    let mut doc = open(HELLO);
    assert!(shapes(&doc).is_empty(), "test assumption: the page draws no shape");
    in_place(&mut doc, "Hello there", vec![SpanStyle { underline: true, ..span(6..11) }]);
    let after = runs(&doc);
    let there = run_saying(&after, "there");
    let rules = shapes(&doc);
    assert_eq!(rules.len(), 1, "one line under there: {:?}", rules.iter().map(|r| r.rect).collect::<Vec<_>>());
    let rule = rules[0].rect;
    assert!((rule.left - there.origin.x).abs() <= 0.5, "starts at {}, there starts at {}", rule.left, there.origin.x);
    assert!((rule.right - there.rect.right).abs() <= 0.5, "ends at {}, there ends at {}", rule.right, there.rect.right);
    assert!(rule.top >= there.origin.y - 0.01, "the line ({}) is not below the baseline ({})", rule.top, there.origin.y);
    // At the face's own position and thickness, centred on the position.
    let lines = faces_lines(&doc, there);
    let centre = (rule.top + rule.bottom) / 2.0;
    assert!(
        (centre - (there.origin.y - lines.underline_position * 12.0)).abs() <= 0.05,
        "the line is centred {} below the baseline; the face puts it {} below",
        centre - there.origin.y,
        -lines.underline_position * 12.0
    );
    assert!(
        ((rule.bottom - rule.top) - lines.underline_thickness * 12.0).abs() <= 0.05,
        "the line is {} thick; the face makes it {}",
        rule.bottom - rule.top,
        lines.underline_thickness * 12.0
    );
}

#[test]
fn a_strike_through_crosses_its_words_at_the_strikeout_height() {
    let Some(_) = skip_without_pdfium() else { return };
    let _serial = serial();
    let mut doc = open(HELLO);
    in_place(&mut doc, "Hello there", vec![SpanStyle { strike: true, ..span(6..11) }]);
    let after = runs(&doc);
    let there = run_saying(&after, "there");
    let rules = shapes(&doc);
    assert_eq!(rules.len(), 1, "{:?}", rules.iter().map(|r| r.rect).collect::<Vec<_>>());
    let rule = rules[0].rect;
    let lines = faces_lines(&doc, there);
    let centre = (rule.top + rule.bottom) / 2.0;
    assert!(
        (there.origin.y - centre - lines.strikeout_position * 12.0).abs() <= 0.05,
        "the line is {} above the baseline; the face puts it {} above",
        there.origin.y - centre,
        lines.strikeout_position * 12.0
    );
    assert!(
        ((rule.bottom - rule.top) - lines.strikeout_thickness * 12.0).abs() <= 0.05,
        "the line is {} thick; the face makes it {}",
        rule.bottom - rule.top,
        lines.strikeout_thickness * 12.0
    );
}

#[test]
fn a_rule_takes_its_words_colour() {
    let Some(_) = skip_without_pdfium() else { return };
    let _serial = serial();
    let mut red = open(HELLO);
    in_place(&mut red, "Hello there", vec![SpanStyle { underline: true, color: Some(RED), ..span(6..11) }]);
    assert!(rule_ops(&mut red).contains("0.784 0.000 0.000 rg"), "{}", rule_ops(&mut red));
    // Words the page draws in a colour of its own: the line follows them.
    let mut blue = open(b"BT /F1 12 Tf 0 0 1 rg 72 700 Td (Hello there) Tj ET");
    in_place(&mut blue, "Hello there", vec![SpanStyle { underline: true, ..span(6..11) }]);
    assert!(rule_ops(&mut blue).contains("0.000 0.000 1.000 rg"), "{}", rule_ops(&mut blue));
}

/// **The line is drawn where the words are, whatever transform they are under.** It is
/// written in the page's own units after the text object, which the page's `cm` moved.
#[test]
fn a_rule_lands_under_words_drawn_through_a_transform() {
    let Some(_) = skip_without_pdfium() else { return };
    let _serial = serial();
    let mut doc = open(b"q 1 0 0 1 50 20 cm BT /F1 12 Tf 22 680 Td (Hello there) Tj ET Q");
    in_place(&mut doc, "Hello there", vec![SpanStyle { underline: true, ..span(6..11) }]);
    let after = runs(&doc);
    let there = run_saying(&after, "there");
    let rules = shapes(&doc);
    assert_eq!(rules.len(), 1, "{:?}", rules.iter().map(|r| r.rect).collect::<Vec<_>>());
    let rule = rules[0].rect;
    assert!((rule.left - there.origin.x).abs() <= 0.5, "starts at {}, there starts at {}", rule.left, there.origin.x);
    assert!(rule.top >= there.origin.y - 0.01 && rule.top - there.origin.y < 3.0, "line at {}, baseline at {}", rule.top, there.origin.y);
}

#[test]
fn one_undo_takes_the_rules_off_with_the_words() {
    use pdf_core::command::{Command, CommandHistory};
    let Some(_) = skip_without_pdfium() else { return };
    let _serial = serial();
    let mut doc = open(HELLO);
    let before = page_stream(&mut doc, 0);
    let mut history = CommandHistory::new(4);
    history
        .execute(
            Command::ReplaceTextLines {
                page_index: 0,
                edits: vec![retype(0, "Hello there", vec![SpanStyle { underline: true, ..span(6..11) }])],
            },
            &mut doc,
        )
        .unwrap_or_else(|e| panic!("refused: {e}"));
    assert_eq!(shapes(&doc).len(), 1);
    history.undo(&mut doc).expect("undo").expect("a step to undo");
    assert!(shapes(&doc).is_empty(), "the line was left behind");
    assert!(page_stream(&mut doc, 0) == before, "after undo the stream is not what it was");
    history.redo(&mut doc).expect("redo").expect("a step to redo");
    assert_eq!(shapes(&doc).len(), 1, "redo did not put the line back");
    let mut boxed: Box<dyn Document> = Box::new(doc);
    check_both_saves(&mut boxed, "an underlined word");
}

/// Two underlined stretches side by side — different sizes — are one line: the first
/// runs to where the second begins.
#[test]
fn rules_over_adjacent_stretches_meet() {
    let Some(_) = skip_without_pdfium() else { return };
    let _serial = serial();
    let mut doc = open(HELLO);
    in_place(
        &mut doc,
        "Hello there",
        vec![
            SpanStyle { underline: true, ..span(0..6) },
            SpanStyle { underline: true, size: Some(15.0), ..span(6..11) },
        ],
    );
    let mut rules: Vec<_> = shapes(&doc).into_iter().map(|d| d.rect).collect();
    rules.sort_by(|a, b| a.left.total_cmp(&b.left));
    assert_eq!(rules.len(), 2, "{rules:?}");
    assert!((rules[0].right - rules[1].left).abs() < 0.01, "the lines do not meet: {} then {}", rules[0].right, rules[1].left);
    assert!((rules[1].bottom - rules[1].top) > (rules[0].bottom - rules[0].top), "the larger stretch has the thicker line");
}

/// **A kept-to-width line's rule follows the final spacing**, not the plain version
/// the width was measured from.
#[test]
fn a_rule_under_a_stretched_line_follows_the_spread_words() {
    let Some(_) = skip_without_pdfium() else { return };
    let _serial = serial();
    let mut doc = open(b"BT /F1 12 Tf 72 700 Td [(alpha ) -500 (beta ) -500 (gamma)] TJ 0 -14 Td (second line) Tj ET");
    let width = {
        let line = &runs(&doc)[0];
        line.rect.right - line.rect.left
    };
    doc.replace_text_lines(
        0,
        &[TextLineEdit::Retype {
            first: 0,
            text: "alpha beta gamma".into(),
            style: TextStyle::default(),
            remove: Vec::new(),
            justify_to: Some(width),
            spans: vec![SpanStyle { size: Some(15.0), underline: true, ..span(6..16) }],
        }],
    )
    .unwrap_or_else(|e| panic!("refused: {e}"));
    let after = runs(&doc);
    let rules = shapes(&doc);
    assert_eq!(rules.len(), 1, "{:?}", rules.iter().map(|r| r.rect).collect::<Vec<_>>());
    let (stretch, rule) = (&after[1], rules[0].rect);
    assert!((rule.left - stretch.origin.x).abs() <= 0.5, "starts at {}, the stretch starts at {}", rule.left, stretch.origin.x);
    assert!((rule.right - stretch.rect.right).abs() <= 0.5, "ends at {}, the stretch ends at {}", rule.right, stretch.rect.right);
}

// ---- Task 6: a rewritten or removed piece takes its old rules with it ---------------

/// The stream's text, to look for what an edit left in it.
fn stream_text(doc: &mut PdfiumDocument) -> String {
    String::from_utf8_lossy(&page_stream(doc, 0)).to_string()
}

/// Page 1 with "there" underlined: objects 0 "Hello ", 1 "there", 2 "next line".
fn underlined_there() -> PdfiumDocument {
    let mut doc = open(HELLO);
    in_place(&mut doc, "Hello there", vec![SpanStyle { underline: true, ..span(6..11) }]);
    assert_eq!(shapes(&doc).len(), 1, "test assumption: the word is underlined");
    assert_eq!(runs(&doc).len(), 3, "test assumption: three pieces");
    doc
}

#[test]
fn restyling_an_underlined_word_without_the_underline_takes_the_rule_off() {
    let Some(_) = skip_without_pdfium() else { return };
    let _serial = serial();
    let mut doc = underlined_there();
    doc_retype(&mut doc, 1, "there", vec![SpanStyle { color: Some(RED), ..span(0..5) }]);
    assert!(shapes(&doc).is_empty(), "the old line is still there: {:?}", shapes(&doc).iter().map(|s| s.rect).collect::<Vec<_>>());
    assert!(!stream_text(&mut doc).contains("PagifyRule"), "the rule's mark was left in the stream");
    assert_eq!(run_saying(&runs(&doc), "there").color, RED, "test assumption: the word was restyled");
}

#[test]
fn retyping_an_underlined_word_moves_its_rule_with_it() {
    let Some(_) = skip_without_pdfium() else { return };
    let _serial = serial();
    let mut doc = underlined_there();
    doc_retype(&mut doc, 1, "theres", vec![SpanStyle { underline: true, ..span(0..6) }]);
    let rules = shapes(&doc);
    assert_eq!(rules.len(), 1, "one line, the new one: {:?}", rules.iter().map(|s| s.rect).collect::<Vec<_>>());
    let there = run_saying(&runs(&doc), "theres").clone();
    assert!((rules[0].rect.right - there.rect.right).abs() <= 0.5, "the line ends at {}, the word at {}", rules[0].rect.right, there.rect.right);
}

#[test]
fn removing_a_line_takes_its_rules_off() {
    let Some(_) = skip_without_pdfium() else { return };
    let _serial = serial();
    let mut doc = underlined_there();
    doc.replace_text_lines(0, &[TextLineEdit::Remove { objects: vec![0, 1] }]).unwrap_or_else(|e| panic!("refused: {e}"));
    assert!(shapes(&doc).is_empty(), "the old line is still there");
    assert_eq!(words_of(&doc), ["next line"]);
}

/// Cut from the page, the rule is cut from the stream: no mark, and no empty
/// marked-content block left where it was.
#[test]
fn nothing_of_a_removed_rule_is_left_in_the_stream() {
    let Some(_) = skip_without_pdfium() else { return };
    let _serial = serial();
    let mut doc = underlined_there();
    doc.replace_text_lines(0, &[TextLineEdit::Remove { objects: vec![0, 1] }]).unwrap_or_else(|e| panic!("refused: {e}"));
    let stream = stream_text(&mut doc);
    assert!(!stream.contains("PagifyRule") && !stream.contains("BDC") && !stream.contains("EMC"), "{stream}");
}

fn words_of(doc: &PdfiumDocument) -> Vec<String> {
    runs(doc).iter().map(|r| r.text.trim().to_string()).collect()
}

#[test]
fn a_piece_left_alone_keeps_its_rule() {
    let Some(_) = skip_without_pdfium() else { return };
    let _serial = serial();
    // Two words of one line underlined, side by side on the same baseline.
    let mut doc = open(HELLO);
    in_place(
        &mut doc,
        "Hello there",
        vec![SpanStyle { underline: true, ..span(0..5) }, SpanStyle { underline: true, ..span(6..11) }],
    );
    let two = |doc: &PdfiumDocument| {
        let mut rules: Vec<_> = shapes(doc).into_iter().map(|d| d.rect).collect();
        rules.sort_by(|a, b| a.left.total_cmp(&b.left));
        rules
    };
    let before = two(&doc);
    assert_eq!(before.len(), 2, "test assumption: a rule under each word: {before:?}");
    let there = runs(&doc).into_iter().find(|r| r.text.trim() == "there").expect("the word");
    doc_retype(&mut doc, there.object, "theres", vec![SpanStyle { underline: true, ..span(0..6) }]);
    let after = two(&doc);
    assert_eq!(after.len(), 2, "{after:?}");
    assert_eq!(after[0], before[0], "the other word's rule was disturbed");
    assert!(after[1].right > before[1].right + 1.0, "the retyped word's rule did not follow it: {:?} -> {:?}", before[1], after[1]);
}

#[test]
fn a_rule_other_programs_drew_is_left_alone() {
    let Some(_) = skip_without_pdfium() else { return };
    let _serial = serial();
    // An untagged rectangle under the words, drawn first.
    let mut doc = open(b"0 0 0 rg 72 697.5 40 0.6 re f BT /F1 12 Tf 72 700 Td (Hello there) Tj ET");
    let theirs = shapes(&doc);
    assert_eq!(theirs.len(), 1, "test assumption: the page draws one rectangle");
    let word = runs(&doc)[0].object;
    doc_retype(&mut doc, word, "Hello world", Vec::new());
    assert_eq!(shapes(&doc).iter().map(|s| s.rect).collect::<Vec<_>>(), theirs.iter().map(|s| s.rect).collect::<Vec<_>>());
    assert!(stream_text(&mut doc).contains("72 697.5 40 0.6 re"), "their rectangle was rewritten");
}

/// **Cutting a rule renumbers what is drawn after it.** The old line is a page object of
/// its own; with it gone, the line drawn after it is one object lower in the copy that
/// its width is measured from.
#[test]
fn a_line_after_a_cut_rule_is_still_measured_by_its_own_object() {
    let Some(_) = skip_without_pdfium() else { return };
    let _serial = serial();
    let mut doc = open(
        b"BT /F1 12 Tf 72 700 Td (Hello there) Tj ET BT /F1 12 Tf 72 672 Td [(delta ) -500 (epsilon ) -500 (zeta)] TJ ET",
    );
    in_place(&mut doc, "Hello there", vec![SpanStyle { underline: true, ..span(6..11) }]);
    // Objects now: 0 "Hello ", 1 "there", 2 the rule, 3 the second line.
    let all = runs(&doc);
    assert_eq!(all.len(), 3, "test assumption");
    let second = all[2].clone();
    assert_eq!(second.object, 3, "test assumption: the rule is an object of its own before the line");
    let width = second.rect.right - second.rect.left;
    doc.replace_text_lines(
        0,
        &[
            retype(1, "there", vec![SpanStyle { color: Some(RED), ..span(0..5) }]),
            TextLineEdit::Retype {
                first: 3,
                text: "delta epsilon zetas".into(),
                style: TextStyle::default(),
                remove: Vec::new(),
                justify_to: Some(width),
                spans: Vec::new(),
            },
        ],
    )
    .unwrap_or_else(|e| panic!("refused: {e}"));
    assert!(shapes(&doc).is_empty(), "the old line is still there");
    let after = runs(&doc);
    let line = after.iter().find(|r| r.text.trim() == "delta epsilon zetas").expect("the second line");
    assert!(((line.rect.right - line.rect.left) - width).abs() < 0.05, "the line spans {}, was {width}", line.rect.right - line.rect.left);
}

/// A line that cannot be kept to its width is refused with nothing written — the
/// refusals of a plain line hold for a styled one.
#[test]
fn a_styled_line_that_cannot_be_kept_to_its_width_is_refused_and_left_alone() {
    let Some(_) = skip_without_pdfium() else { return };
    let _serial = serial();
    let mut doc = open(b"BT /F1 12 Tf 72 700 Td [(alpha ) -500 (beta ) -500 (gamma)] TJ 0 -14 Td (second line) Tj ET");
    let width = {
        let line = &runs(&doc)[0];
        line.rect.right - line.rect.left
    };
    let before = page_stream(&mut doc, 0);
    // Three times the size on its last two words: nothing closes enough gap for that.
    let error = doc
        .replace_text_lines(
            0,
            &[TextLineEdit::Retype {
                first: 0,
                text: "alpha beta gamma".into(),
                style: TextStyle::default(),
                remove: Vec::new(),
                justify_to: Some(width),
                spans: vec![SpanStyle { size: Some(36.0), ..span(6..16) }],
            }],
        )
        .err()
        .expect("a line three times too wide was accepted");
    assert!(matches!(error, PdfError::InvalidArgument(_)), "{error}");
    assert_eq!(page_stream(&mut doc, 0), before, "the page changed");
}

/// **Words that are the line's own keep their own codes and spacing.** A bold span
/// on one word of a line the producer spaced by hand writes that word again; every
/// other word is the page's, glyph for glyph, where it was.
#[test]
fn a_font_span_on_unchanged_words_leaves_the_other_words_where_they_were() {
    let Some(_) = skip_without_pdfium() else { return };
    let _serial = serial();
    let (bold, name) = montserrat("Bold");
    let mut doc = open(b"BT /F1 12 Tf 72 700 Td [(Hel) -120 (lo big ) -80 (wor) -60 (ld)] TJ ET");
    doc.set_typing_fonts(vec![bold]);
    let before = boxes(&doc);
    assert_eq!(before.iter().map(|(c, _)| *c).collect::<String>(), "Hello big world", "test assumption: the page's own words");
    in_place(&mut doc, "Hello big world", vec![SpanStyle { font: Some(SpanFont::Face(name)), ..span(6..9) }]);
    let after = boxes(&doc);
    assert_eq!(after.len(), before.len());
    // Everything before the bold word is where it was.
    for ((c, was), (_, is)) in before.iter().zip(&after).take(6) {
        for (a, b) in was.iter().zip(is) {
            assert!((a - b).abs() < 0.01, "{c:?} (before the bold word) moved: {was:?} -> {is:?}");
        }
    }
    let stream = page_stream(&mut doc, 0);
    let kerns: Vec<f32> = content::parse(&stream)
        .expect("parse")
        .iter()
        .filter(|o| o.shows_text())
        .flat_map(content::pieces)
        .filter_map(|p| match p {
            content::Piece::Kern(n) => Some(n),
            _ => None,
        })
        .collect();
    assert!(kerns.contains(&-120.0) && kerns.contains(&-60.0), "the page's own spacing numbers were dropped: {kerns:?}");
}

// ---- Task 7: reading the page's looks back -------------------------------------------

fn looks_of(doc: &PdfiumDocument, objects: &[usize]) -> std::collections::HashMap<usize, RunTextLook> {
    doc.run_text_looks(0, &objects.iter().copied().collect()).expect("looks")
}

#[test]
fn a_pieces_spacing_width_and_rise_read_back_in_points() {
    let Some(_) = skip_without_pdfium() else { return };
    let _serial = serial();
    // Everything drawn twice its size. Character spacing is stretched by the scaling
    // along with the glyphs: 4 units at 50% is 2 units across, and 4 points on the page.
    let doc = open(b"q 2 0 0 2 0 0 cm BT /F1 6 Tf 4 Tc 50 Tz 3 Ts 36 350 Td (Hello there) Tj ET Q");
    let object = runs(&doc)[0].object;
    let look = looks_of(&doc, &[object])[&object];
    assert!((look.char_spacing - 4.0).abs() < 0.01, "{look:?}");
    assert!((look.width_pct - 50.0).abs() < 0.01, "{look:?}");
    assert!((look.rise - 6.0).abs() < 0.01, "{look:?}");
    assert!(!look.underline && !look.strike, "{look:?}");
}

/// **What is written reads back as it was asked** — the number the editor shows is the
/// one it would write again, whatever the matrices and the scaling make of it.
#[test]
fn a_look_written_reads_back_the_same() {
    let Some(_) = skip_without_pdfium() else { return };
    let _serial = serial();
    let mut doc = open(b"q 2 0 0 2 0 0 cm BT /F1 6 Tf 36 350 Td (Hello there) Tj ET Q");
    in_place(
        &mut doc,
        "Hello there",
        vec![SpanStyle { char_spacing: Some(2.0), width_pct: Some(50.0), rise: Some(3.0), ..span(6..11) }],
    );
    let all = runs(&doc);
    let (hello, there) = (run_saying(&all, "Hello").object, run_saying(&all, "there").object);
    let looks = looks_of(&doc, &[hello, there]);
    assert_eq!(looks[&hello], RunTextLook::default(), "the words left alone have no look of their own");
    let look = looks[&there];
    assert!((look.char_spacing - 2.0).abs() < 0.01, "{look:?}");
    assert!((look.width_pct - 50.0).abs() < 0.01, "{look:?}");
    assert!((look.rise - 3.0).abs() < 0.01, "{look:?}");
}

#[test]
fn an_underline_written_here_reads_back_on_its_piece_only() {
    let Some(_) = skip_without_pdfium() else { return };
    let _serial = serial();
    let mut doc = open(HELLO);
    in_place(
        &mut doc,
        "Hello there",
        vec![SpanStyle { strike: true, ..span(0..5) }, SpanStyle { underline: true, ..span(6..11) }],
    );
    let all = runs(&doc);
    let (hello, there, next) = (run_saying(&all, "Hello").object, run_saying(&all, "there").object, run_saying(&all, "next line").object);
    // A rule is a page object too, and not a text run: asking for it gets nothing.
    let rule = shapes(&doc)[0].object;
    let looks = looks_of(&doc, &[hello, there, next, rule]);
    assert_eq!(looks.len(), 3, "{looks:?}");
    assert!(looks[&hello].strike && !looks[&hello].underline, "{:?}", looks[&hello]);
    assert!(looks[&there].underline && !looks[&there].strike, "{:?}", looks[&there]);
    assert!(!looks[&next].underline && !looks[&next].strike, "the line below was not touched: {:?}", looks[&next]);
}

#[test]
fn an_untagged_line_under_words_is_not_read_as_an_underline() {
    let Some(_) = skip_without_pdfium() else { return };
    let _serial = serial();
    let doc = open(b"0 0 0 rg 72 697.5 40 0.6 re f BT /F1 12 Tf 72 700 Td (Hello there) Tj ET");
    let object = runs(&doc)[0].object;
    let look = looks_of(&doc, &[object])[&object];
    assert!(!look.underline && !look.strike, "a rectangle another program drew was read as ours: {look:?}");
}

/// A producer that letter-spaces a line writes a `Tc` and a `TJ` number after every glyph that
/// cancels most of it (§15). The look reads the `Tc` as the letter spacing, which is what Apply
/// writes back, and the numbers' share apart from it, which Apply carries without being told:
/// the box needs both to draw the letters where the page has them.
#[test]
fn a_producers_tracking_is_read_beside_its_tc() {
    let Some(_) = skip_without_pdfium() else { return };
    let _serial = serial();
    let doc = open(b"BT /F1 1 Tf 8 0 0 8 72 700 Tm -0.164 Tc [(H)-124(e)-124(l)-124(l)-124(o)] TJ ET");
    let hello = run_saying(&runs(&doc), "Hello").object;
    let look = looks_of(&doc, &[hello])[&hello];
    assert!((look.char_spacing - -1.312).abs() < 0.01, "Tc in points: {look:?}");
    assert!((look.tracking - 0.992).abs() < 0.01, "the TJ numbers in points: {look:?}");
    // A line with no numbers of its own has none.
    let plain = open(b"BT /F1 12 Tf 72 700 Td 1 Tc (Hello) Tj ET");
    let hello = run_saying(&runs(&plain), "Hello").object;
    assert_eq!(looks_of(&plain, &[hello])[&hello].tracking, 0.0);
}
