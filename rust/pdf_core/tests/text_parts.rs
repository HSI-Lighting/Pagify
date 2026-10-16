//! A text mark of several parts — each in its own font, size and colour — with
//! lines under or through it, and a part drawn narrower than its own width. One
//! mark, so one undo takes all of it.

mod harness;
use harness::{check_both_saves, open_fixture, serial, skip_without_pdfium};

use pdf_core::command::{Command, CommandHistory};
use pdf_core::document::{Annotation, Color, DrawnKind, Glyph, Point, TextPart, TextRule};

const RED: Color = Color { r: 200, g: 0, b: 0, a: 255 };
const BLUE: Color = Color { r: 0, g: 0, b: 200, a: 255 };

fn glyph(ch: &str, x: f32, y: f32) -> Glyph {
    Glyph { ch: ch.into(), id: 0, x, y, radians: 0.0 }
}

/// "Big" at 24 pt in red, then "small" at 10 pt in blue, and a line under them.
fn mark(id: i32) -> Annotation {
    Annotation::Text {
        text: "Big small".into(),
        font: "Helvetica".into(),
        font_asset: None,
        size: 24.0,
        color: RED,
        glyphs: vec![glyph("Big", 100.0, 200.0)],
        id,
        restore: String::new(),
        frame: Vec::new(),
        frame_width: 0.0,
        scale_x: 1.0,
        more: vec![TextPart {
            font: "Helvetica".into(),
            font_asset: None,
            size: 10.0,
            color: BLUE,
            glyphs: vec![glyph("small", 150.0, 200.0)],
            scale_x: 1.0,
        }],
        rules: vec![TextRule { from: Point { x: 100.0, y: 203.0 }, to: Point { x: 180.0, y: 203.0 }, width: 1.0, color: RED }],
    }
}

fn add(doc: &mut Box<dyn pdf_core::document::Document>, history: &mut CommandHistory, annotation: Annotation) {
    history
        .execute(Command::AddAnnotation { page_index: 0, annotation }, doc.as_document_mut().expect("mutable"))
        .expect("the mark is written");
}

fn shapes(doc: &Box<dyn pdf_core::document::Document>) -> usize {
    doc.drawn_objects(0).expect("drawn").iter().filter(|d| d.kind == DrawnKind::Shape).count()
}

#[test]
fn a_mark_of_several_parts_writes_each_part_in_its_own_look() {
    let Some(pdfium) = skip_without_pdfium() else { return };
    let _serial = serial();
    let mut doc = open_fixture(&pdfium, "single-page.pdf");
    let mut history = CommandHistory::default();
    add(&mut doc, &mut history, mark(7));

    let runs = doc.text_runs(0).expect("runs");
    let big = runs.iter().find(|r| r.text.trim() == "Big").expect("the first part");
    let small = runs.iter().find(|r| r.text.trim() == "small").expect("the second part");
    assert!((big.size - 24.0).abs() < 0.5 && big.color == RED, "{big:?}");
    assert!((small.size - 10.0).abs() < 0.5 && small.color == BLUE, "{small:?}");
    check_both_saves(&mut doc, "a text mark of several parts");
}

#[test]
fn a_rule_is_drawn_as_a_line_where_it_was_put() {
    let Some(pdfium) = skip_without_pdfium() else { return };
    let _serial = serial();
    let mut doc = open_fixture(&pdfium, "single-page.pdf");
    let before = shapes(&doc);
    add(&mut doc, &mut CommandHistory::default(), mark(8));

    assert_eq!(shapes(&doc), before + 1, "one line, and nothing else drawn");
    let drawn = doc.drawn_objects(0).expect("drawn");
    let shown: Vec<_> = drawn.iter().filter(|d| d.kind == DrawnKind::Shape).map(|d| d.rect).collect();
    let line = drawn
        .iter()
        .filter(|d| d.kind == DrawnKind::Shape)
        // PDFium's bounds take in the stroke, a point either side here.
        .find(|d| (d.rect.left - 100.0).abs() <= 1.01 && (d.rect.right - 180.0).abs() <= 1.01)
        .unwrap_or_else(|| panic!("no line from 100 to 180 among {shown:?}"));
    assert!(line.rect.top <= 203.5 && line.rect.bottom >= 202.5, "{:?}", line.rect);
}

#[test]
fn one_undo_takes_every_part_and_rule_off() {
    let Some(pdfium) = skip_without_pdfium() else { return };
    let _serial = serial();
    let mut doc = open_fixture(&pdfium, "single-page.pdf");
    let runs_before = doc.text_runs(0).expect("runs").len();
    let shapes_before = shapes(&doc);
    let mut history = CommandHistory::default();
    add(&mut doc, &mut history, mark(9));

    history.undo(doc.as_document_mut().expect("mutable")).expect("undone");
    assert_eq!(doc.text_runs(0).expect("runs").len(), runs_before, "a part was left behind");
    assert_eq!(shapes(&doc), shapes_before, "the line was left behind");

    history.redo(doc.as_document_mut().expect("mutable")).expect("redone");
    let runs = doc.text_runs(0).expect("runs");
    assert!(runs.iter().any(|r| r.text.trim() == "Big") && runs.iter().any(|r| r.text.trim() == "small"));
    assert_eq!(shapes(&doc), shapes_before + 1);
}

/// **A caption's ring goes with its words.** It always meant to — it is tagged with
/// the words' id — but the byte-level removal only ever looked for text, so undoing a
/// ringed caption left the ring on the page.
#[test]
fn a_captions_ring_goes_with_its_words_on_undo() {
    let Some(pdfium) = skip_without_pdfium() else { return };
    let _serial = serial();
    let mut doc = open_fixture(&pdfium, "single-page.pdf");
    let before = shapes(&doc);
    let Annotation::Text { text, font, font_asset, size, color, glyphs, restore, .. } = mark(0) else { unreachable!() };
    let ringed = Annotation::Text {
        text,
        font,
        font_asset,
        size,
        color,
        glyphs,
        id: 13,
        restore,
        frame: vec![Point { x: 90.0, y: 180.0 }, Point { x: 200.0, y: 180.0 }, Point { x: 200.0, y: 210.0 }, Point { x: 90.0, y: 210.0 }],
        frame_width: 1.0,
        scale_x: 1.0,
        more: Vec::new(),
        rules: Vec::new(),
    };
    let mut history = CommandHistory::default();
    add(&mut doc, &mut history, ringed);
    assert_eq!(shapes(&doc), before + 1, "test assumption: the ring is drawn");
    history.undo(doc.as_document_mut().expect("mutable")).expect("undone");
    assert_eq!(shapes(&doc), before, "the ring was left behind");
}

#[test]
fn a_part_drawn_at_half_width_is_half_as_wide() {
    let Some(pdfium) = skip_without_pdfium() else { return };
    let _serial = serial();
    let mut doc = open_fixture(&pdfium, "single-page.pdf");
    let mut history = CommandHistory::default();
    for (id, y, scale_x) in [(11, 300.0, 1.0), (12, 400.0, 0.5)] {
        let Annotation::Text { text, font, font_asset, size, color, id: _, restore, frame, frame_width, more, rules, .. } = mark(0) else {
            unreachable!()
        };
        let _ = (text, more, rules);
        add(
            &mut doc,
            &mut history,
            Annotation::Text {
                text: "WWWW".into(),
                font,
                font_asset,
                size,
                color,
                glyphs: vec![glyph("WWWW", 100.0, y)],
                id,
                restore,
                frame,
                frame_width,
                scale_x,
                more: Vec::new(),
                rules: Vec::new(),
            },
        );
    }
    let runs = doc.text_runs(0).expect("runs");
    let width_at = |y: f32| {
        let run = runs.iter().find(|r| r.text.trim() == "WWWW" && (r.origin.y - y).abs() < 1.0).expect("a WWWW run");
        run.rect.right - run.rect.left
    };
    let ratio = width_at(400.0) / width_at(300.0);
    assert!((ratio - 0.5).abs() < 0.02, "the narrow part is {ratio} of the wide one");
}

/// **Two parts in one face keep their own letters.** PDFium names a page's fonts by
/// their BaseFont as it writes the content, so a subset per part put two fonts of one
/// name on the page and the second part's glyphs were read through the first's:
/// "there" came back as letters of "Hello".
#[test]
fn two_parts_in_one_face_each_keep_their_own_letters() {
    let Some(pdfium) = skip_without_pdfium() else { return };
    let _serial = serial();
    let face = "text-parts-montserrat";
    if !pdf_core::text::is_registered(face) {
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../desktop/third_party/fonts/Montserrat-Regular.ttf");
        pdf_core::text::register(face, std::fs::read(path).expect("the bundled Montserrat")).expect("registered");
    }
    let set = |text: &str, x: f32, size: f32| -> Vec<Glyph> {
        let mut pen = x;
        let shaped = pdf_core::text::shape(face, text).expect("shaped");
        shaped
            .glyphs
            .iter()
            .map(|g| {
                let ch = text[g.cluster as usize..].chars().next().expect("a letter").to_string();
                let placed = Glyph { ch, id: g.id, x: pen, y: 200.0, radians: 0.0 };
                pen += g.advance * size;
                placed
            })
            .collect()
    };
    let mut doc = open_fixture(&pdfium, "single-page.pdf");
    let mark = Annotation::Text {
        text: "Hello there".into(),
        font: face.into(),
        font_asset: Some(face.into()),
        size: 12.0,
        color: RED,
        glyphs: set("Hello", 100.0, 12.0),
        id: 21,
        restore: String::new(),
        frame: Vec::new(),
        frame_width: 0.0,
        scale_x: 1.0,
        more: vec![TextPart { font: face.into(), font_asset: Some(face.into()), size: 20.0, color: BLUE, glyphs: set("there", 150.0, 20.0), scale_x: 1.0 }],
        rules: Vec::new(),
    };
    add(&mut doc, &mut CommandHistory::default(), mark);

    let mut runs: Vec<_> = doc.text_runs(0).expect("runs").into_iter().filter(|r| (r.origin.y - 200.0).abs() < 1.0).collect();
    runs.sort_by(|a, b| a.origin.x.total_cmp(&b.origin.x));
    let read: String = runs.iter().map(|r| r.text.trim()).collect();
    assert_eq!(read, "Hellothere");
    check_both_saves(&mut doc, "two parts in one face");
}

/// **Undo reads the page once, not once per letter.** A mark is one text object per
/// glyph, and a page open costs as much as the page is busy: on 50,000 shapes a
/// 27-letter word took 2.7 s to undo, a single letter 0.35 s.
#[test]
fn undoing_a_long_word_on_a_busy_page_costs_what_a_short_one_does() {
    let Some(_) = skip_without_pdfium() else { return };
    let _serial = serial();
    let face = "text-parts-montserrat";
    if !pdf_core::text::is_registered(face) {
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../desktop/third_party/fonts/Montserrat-Regular.ttf");
        pdf_core::text::register(face, std::fs::read(path).expect("the bundled Montserrat")).expect("registered");
    }
    let mut content = String::new();
    for i in 0..20_000 {
        content.push_str(&format!("{} {} 1 1 re f\n", i % 600, (i / 600) % 780));
    }
    let content: &'static str = Box::leak(content.into_boxed_str());
    let bytes = harness::hand_built_pdf(&[harness::HandPage { content, resources: "", extra: "" }], &[]);
    let mut doc: Box<dyn pdf_core::document::Document> =
        Box::new(pdf_core::document::pdfium_doc::PdfiumDocument::open_bytes(bytes, None).expect("open"));

    let mut undo_of = |letters: &str| {
        let mut pen = 100.0;
        let glyphs = pdf_core::text::shape(face, letters)
            .expect("shaped")
            .glyphs
            .iter()
            .map(|g| {
                let ch = letters[g.cluster as usize..].chars().next().expect("a letter").to_string();
                let placed = Glyph { ch, id: g.id, x: pen, y: 300.0, radians: 0.0 };
                pen += g.advance * 12.0;
                placed
            })
            .collect();
        let Annotation::Text { more, rules, frame, frame_width, restore, .. } = mark(0) else { unreachable!() };
        let mut history = CommandHistory::default();
        add(
            &mut doc,
            &mut history,
            Annotation::Text {
                text: letters.into(),
                font: face.into(),
                font_asset: Some(face.into()),
                size: 12.0,
                color: RED,
                glyphs,
                id: 31,
                restore,
                frame,
                frame_width,
                scale_x: 1.0,
                more: { let _ = more; Vec::new() },
                rules: { let _ = rules; Vec::new() },
            },
        );
        let started = std::time::Instant::now();
        history.undo(doc.as_document_mut().expect("mutable")).expect("undone");
        started.elapsed()
    };
    let short = undo_of("H");
    let long = undo_of("Hello there, a longer paste");
    eprintln!("undo of 1 letter {short:?}, of 27 letters {long:?}");
    assert!(long.as_secs_f64() < short.as_secs_f64() * 2.0, "27 letters took {long:?} to undo, 1 letter {short:?}");
}

#[test]
fn a_text_mark_saved_before_parts_reads_as_one_unscaled_part() {
    let mut value = serde_json::to_value(mark(3)).expect("to json");
    let object = value.as_object_mut().expect("an object");
    for key in ["scaleX", "more", "rules"] {
        assert!(object.remove(key).is_some(), "test assumption: the mark writes {key}");
    }
    let back: Annotation = serde_json::from_value(value).expect("an older mark still reads");
    let Annotation::Text { scale_x, more, rules, .. } = back else { panic!("not a text mark") };
    assert_eq!(scale_x, 1.0);
    assert!(more.is_empty() && rules.is_empty());
}

#[test]
fn a_marks_bounds_cover_its_parts_and_rules() {
    let bounds = mark(4).bounds().expect("bounds");
    for (x, y) in [(100.0, 200.0), (150.0, 200.0), (100.0, 203.0), (180.0, 203.0)] {
        assert!(
            bounds.left <= x && x <= bounds.right && bounds.top <= y && y <= bounds.bottom,
            "({x}, {y}) is outside {bounds:?}"
        );
    }
}
