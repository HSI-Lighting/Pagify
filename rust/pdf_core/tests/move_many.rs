//! Moving many things at once: a marquee selection moved as one group.
//!
//! ```text
//! PAGIFY_PDFIUM_LIB=<pdfium> cargo test --release --test move_many -- --nocapture
//! ```
//!
//! Reported from use (0.1.78, session log): 96 things moved together froze the
//! app, and on a low-spec machine it crashed. A group move was a `Batch` of one
//! `MoveObject` per member, and every one of them serialised the whole
//! document, parsed it, rewrote it and loaded it back into PDFium: the VIVO
//! lens, 93 shapes, took 4.0 s on the UI thread (43 ms a member), and its undo
//! as long again. `Command::MoveObjects` plans every member against one read of
//! the page and writes once.

mod harness;
use harness::{hand_built_pdf, serial, skip_without_pdfium, HandPage};

use std::time::Instant;

use pdf_core::command::{Command, CommandHistory};
use pdf_core::document::pdfium_doc::PdfiumDocument;
use pdf_core::document::{Document, DrawnKind, DrawnObject, Point, Rect};

const VIVO: &str = r"C:\Users\hsili\Desktop\VIVO mini.pdf";

fn open(bytes: Vec<u8>) -> PdfiumDocument {
    PdfiumDocument::open_bytes(bytes, None).expect("open")
}

fn top_level(doc: &PdfiumDocument) -> Vec<DrawnObject> {
    doc.drawn_objects(0).expect("drawn").into_iter().filter(|d| d.depth == 0).collect()
}

fn near(a: Rect, b: Rect, within: f32) -> bool {
    (a.left - b.left).abs() <= within
        && (a.top - b.top).abs() <= within
        && (a.right - b.right).abs() <= within
        && (a.bottom - b.bottom).abs() <= within
}

fn shifted(r: Rect, by: Point) -> Rect {
    Rect { left: r.left + by.x, top: r.top + by.y, right: r.right + by.x, bottom: r.bottom + by.y }
}

/// Every object in `moved` is where it was plus `by`; every other is where it was.
fn assert_moved(before: &[DrawnObject], after: &[DrawnObject], moved: &[usize], by: Point) {
    assert_eq!(before.len(), after.len(), "the page gained or lost something");
    for (was, now) in before.iter().zip(after) {
        assert_eq!(was.kind, now.kind);
        let want = if moved.contains(&was.object) { shifted(was.rect, by) } else { was.rect };
        assert!(near(now.rect, want, 0.5), "object {} ({:?}) is at {:?}, wanted {:?}", was.object, was.kind, now.rect, want);
    }
}

fn texts(doc: &PdfiumDocument) -> Vec<String> {
    doc.text_runs(0).expect("runs").into_iter().map(|r| r.text.trim().to_string()).collect()
}

const GREY_IMAGE: &str =
    "<< /Type /XObject /Subtype /Image /Width 1 /Height 1 /BitsPerComponent 8 /ColorSpace /DeviceGray /Length 1 >>\nstream\nx\nendstream";

/// Thirty squares in a grid, five words each in its own text object, a picture,
/// and one square and one word that stay where they are.
fn busy_page() -> PdfiumDocument {
    let mut content = String::new();
    for i in 0..30 {
        let (x, y) = (60 + (i % 10) * 45, 500 + (i / 10) * 45);
        content.push_str(&format!("0 0 {} rg {x} {y} 30 30 re f\n", i as f32 / 30.0));
    }
    for (i, word) in ["Alpha", "Beta", "Gamma", "Delta", "Epsilon"].iter().enumerate() {
        content.push_str(&format!("BT /F1 12 Tf {} 700 Td ({word}) Tj ET\n", 60 + i * 90));
    }
    content.push_str("q 40 0 0 40 300 380 cm /Im0 Do Q\n");
    content.push_str("1 0 0 rg 60 100 30 30 re f\nBT /F1 12 Tf 60 60 Td (Stays) Tj ET\n");
    open(hand_built_pdf(
        &[HandPage {
            content: Box::leak(content.into_boxed_str()),
            resources: "<< /Font << /F1 5 0 R >> /XObject << /Im0 6 0 R >> >>",
            extra: "",
        }],
        &["<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>", GREY_IMAGE],
    ))
}

#[test]
fn moving_many_things_at_once_moves_each_by_the_same_and_nothing_else() {
    let Some(()) = skip_without_pdfium() else { return };
    let _lock = serial();
    let mut doc = busy_page();
    let before = top_level(&doc);
    let words = texts(&doc);
    // Everything but the last square and the last word.
    let stays: Vec<usize> = before.iter().filter(|d| d.rect.top > 640.0).map(|d| d.object).collect();
    assert_eq!(stays.len(), 2, "{before:?}");
    let moved: Vec<usize> = before.iter().map(|d| d.object).filter(|o| !stays.contains(o)).collect();
    assert_eq!(moved.len(), 36);
    let by = Point { x: 10.0, y: 20.0 };

    let mut history = CommandHistory::new(8);
    history
        .execute(Command::MoveObjects { page_index: 0, objects: moved.clone(), by }, &mut doc)
        .expect("move them all");
    assert_moved(&before, &top_level(&doc), &moved, by);
    assert_eq!(texts(&doc), words, "the words changed");

    history.undo(&mut doc).expect("undo").expect("a step to undo");
    assert_moved(&before, &top_level(&doc), &[], by);
    history.redo(&mut doc).expect("redo").expect("a step to redo");
    assert_moved(&before, &top_level(&doc), &moved, by);
}

/// A picture's placeholder (object 1 of `framed.pdf`, under the picture)
/// moves with the picture's own unit, so asking for both must give exactly
/// the page asking for the picture alone gives — not a placeholder moved
/// twice and showing from under it.
#[test]
fn a_picture_moved_with_its_placeholder_moves_each_once() {
    let Some(()) = skip_without_pdfium() else { return };
    let _lock = serial();
    let open_framed = || PdfiumDocument::open_path(harness::fixture_path("framed.pdf").to_str().unwrap(), None).expect("open");
    let picture = open_framed().images_on(0).expect("images")[0].object;
    let by = Point { x: -15.0, y: 25.0 };
    let moved = |objects: Vec<usize>| {
        let mut doc = open_framed();
        CommandHistory::new(8).execute(Command::MoveObjects { page_index: 0, objects, by }, &mut doc).expect("move");
        render(&doc)
    };
    let (alone, with_placeholder) = (moved(vec![picture]), moved(vec![picture - 1, picture]));
    let differing = alone.iter().zip(&with_placeholder).filter(|(a, b)| a.abs_diff(**b) > 8).count();
    assert_eq!(differing, 0, "the placeholder moved twice");
}

fn render(doc: &PdfiumDocument) -> Vec<u8> {
    let size = doc.page_size(0).expect("size");
    doc.page(0)
        .expect("page")
        .render_region(&pdf_core::document::RegionRequest {
            crop: Rect { left: 0.0, top: 0.0, right: size.width_pt, bottom: size.height_pt },
            scale: 1.0,
            render_annotations: false,
            render_form_data: false,
        })
        .expect("render")
        .data
}

/// A group the page draws through a form cannot be planned with the others;
/// the move still happens, the slow way, and still as one undo.
#[test]
fn a_group_holding_something_only_the_slow_way_can_move_still_moves() {
    let Some(()) = skip_without_pdfium() else { return };
    let _lock = serial();
    let mut doc = open(hand_built_pdf(
        &[HandPage {
            content: "0 0 1 rg 60 600 30 30 re f q 1 0 0 1 200 600 cm /Fm0 Do Q",
            resources: "<< /XObject << /Fm0 5 0 R >> >>",
            extra: "",
        }],
        &["<< /Type /XObject /Subtype /Form /BBox [0 0 40 40] /Length 22 >>\nstream\n1 0 0 rg 0 0 40 40 re f\nendstream"],
    ));
    let before = top_level(&doc);
    assert_eq!(before.len(), 2, "{before:?}");
    let moved: Vec<usize> = before.iter().map(|d| d.object).collect();
    let by = Point { x: 30.0, y: 10.0 };
    let mut history = CommandHistory::new(8);
    history.execute(Command::MoveObjects { page_index: 0, objects: moved.clone(), by }, &mut doc).expect("move");
    assert_moved(&before, &top_level(&doc), &moved, by);
    history.undo(&mut doc).expect("undo").expect("a step to undo");
    assert_moved(&before, &top_level(&doc), &[], by);
}

/// The report: the VIVO lens, moved as one group, and moved back.
#[test]
fn moving_the_lens_as_a_group_takes_one_edit_not_one_per_piece() {
    let Some(()) = skip_without_pdfium() else { return };
    if !std::path::Path::new(VIVO).exists() {
        eprintln!("skipped: VIVO mini.pdf is not at {VIVO}");
        return;
    }
    let _lock = serial();
    let mut doc = PdfiumDocument::open_path(VIVO, None).expect("open VIVO");
    let before = top_level(&doc);
    let lens: Vec<usize> = before
        .iter()
        .filter(|d| matches!(d.kind, DrawnKind::Shape | DrawnKind::Picture))
        .filter(|d| d.rect.left < 490.0 && d.rect.right > 415.0 && d.rect.top < 180.0 && d.rect.bottom > 106.0)
        .map(|d| d.object)
        .collect();
    assert!(lens.len() > 80, "the lens is {} pieces", lens.len());
    let by = Point { x: 10.0, y: 20.0 };
    let mut history = CommandHistory::new(8);

    let started = Instant::now();
    history.execute(Command::MoveObjects { page_index: 0, objects: lens.clone(), by }, &mut doc).expect("move the lens");
    let moving = started.elapsed().as_millis();
    let started = Instant::now();
    history.undo(&mut doc).expect("undo").expect("a step to undo");
    let undoing = started.elapsed().as_millis();
    eprintln!("moved {} pieces in {moving} ms, moved back in {undoing} ms (was 4,000 ms each way)", lens.len());

    assert_moved(&before, &top_level(&doc), &[], by);
    assert!(moving < 1000 && undoing < 1000, "moving {moving} ms, undoing {undoing} ms");
}

/// "This also sometimes happens with text": the 9-line paragraph from the
/// owner's session (11 runs), moved as a group. One run at a time it took
/// 585 ms; and planned from one read of the page's runs, not one per run
/// (`text_run_at` loads the page's text every time it is asked).
#[test]
fn moving_a_paragraph_as_a_group_takes_one_edit() {
    let Some(()) = skip_without_pdfium() else { return };
    if !std::path::Path::new(VIVO).exists() {
        eprintln!("skipped: VIVO mini.pdf is not at {VIVO}");
        return;
    }
    let _lock = serial();
    let mut doc = PdfiumDocument::open_path(VIVO, None).expect("open VIVO");
    let before = top_level(&doc);
    let words = texts(&doc);
    let paragraph: Vec<usize> = before
        .iter()
        .filter(|d| d.kind == DrawnKind::Words)
        .filter(|d| d.rect.left >= 190.0 && d.rect.right <= 400.0 && d.rect.top >= 65.0 && d.rect.bottom <= 160.0)
        .map(|d| d.object)
        .collect();
    assert!(paragraph.len() >= 9, "the paragraph is {} runs", paragraph.len());
    let by = Point { x: 5.0, y: 5.0 };

    let started = Instant::now();
    CommandHistory::new(8)
        .execute(Command::MoveObjects { page_index: 0, objects: paragraph.clone(), by }, &mut doc)
        .expect("move the paragraph");
    let ms = started.elapsed().as_millis();
    eprintln!("moved a paragraph of {} runs in {ms} ms (was 585 ms)", paragraph.len());
    assert_moved(&before, &top_level(&doc), &paragraph, by);
    assert_eq!(texts(&doc), words, "the words changed");
    assert!(ms < 300, "{ms} ms");
}

/// Pasted words are written one text object per letter (architecture §21), so
/// moving a pasted word is always a group move — "sometimes text too". They
/// must go in the one pass, not the slow way round.
#[test]
fn pasted_words_written_a_letter_at_a_time_move_in_one_pass() {
    use pdf_core::document::{Annotation, Color, DocumentMut, Glyph};
    let Some(()) = skip_without_pdfium() else { return };
    let _lock = serial();
    let mut doc = open(hand_built_pdf(&[HandPage { content: "0 0 1 rg 60 600 30 30 re f", resources: "<< >>", extra: "" }], &[]));
    let word = "Description:";
    let glyphs: Vec<Glyph> =
        word.chars().enumerate().map(|(i, ch)| Glyph { id: 0, ch: ch.to_string(), x: 100.0 + i as f32 * 8.0, y: 300.0, radians: 0.0 }).collect();
    CommandHistory::new(8)
        .execute(
            Command::AddAnnotation {
                page_index: 0,
                annotation: Annotation::Text {
                    font_asset: None,
                    text: word.into(),
                    font: "Helvetica".into(),
                    size: 14.0,
                    color: Color { r: 0, g: 0, b: 0, a: 255 },
                    glyphs,
                    id: 0,
                    restore: String::new(),
                    frame: Vec::new(),
                    frame_width: 0.0,
                    scale_x: 1.0,
                    more: Vec::new(),
                    rules: Vec::new(),
                },
            },
            &mut doc,
        )
        .expect("paste the word");
    let before = top_level(&doc);
    let letters: Vec<usize> = before.iter().filter(|d| d.kind == DrawnKind::Words).map(|d| d.object).collect();
    eprintln!("the pasted word is {} text objects", letters.len());
    let words = texts(&doc);
    let by = Point { x: 25.0, y: -40.0 };

    doc.move_objects_mut(0, &letters, by).expect("the pasted word should move in one pass");
    assert_moved(&before, &top_level(&doc), &letters, by);
    assert_eq!(texts(&doc), words, "the words changed");
}

// ------------------------------------------------------------------ resize --
//
// "Fix that too": a group resize was a `Batch` of `ScaleObject`, one
// whole-document rewrite per member — two for a run of words, which was
// resized and then measured and moved. `Command::ScaleObjects` plans them all
// against one read of the page.

/// Where each object is after `command`, on a fresh copy of the page.
fn rects_after(make: &dyn Fn() -> PdfiumDocument, command: Command) -> Vec<(DrawnKind, Rect)> {
    let mut doc = make();
    CommandHistory::new(8).execute(command, &mut doc).expect("resize");
    top_level(&doc).into_iter().map(|d| (d.kind, d.rect)).collect()
}

/// The group resize lands every object where resizing it alone would have.
fn resizes_as_one_at_a_time_did(make: &dyn Fn() -> PdfiumDocument, objects: Vec<usize>, anchor: Point, sx: f32, sy: f32) {
    let one_at_a_time = Command::Batch {
        commands: objects.iter().map(|&object| Command::ScaleObject { page_index: 0, object, anchor, sx, sy }).collect(),
    };
    let was = rects_after(make, one_at_a_time);
    let now = rects_after(make, Command::ScaleObjects { page_index: 0, objects, anchor, sx, sy });
    assert_eq!(was.len(), now.len());
    for ((kind, a), (_, b)) in was.iter().zip(&now) {
        assert!(near(*a, *b, 0.5), "{kind:?}: one at a time {a:?}, together {b:?}");
    }
}

#[test]
fn resizing_many_things_at_once_lands_each_where_resizing_it_alone_did() {
    let Some(()) = skip_without_pdfium() else { return };
    let _lock = serial();
    let all: Vec<usize> = top_level(&busy_page()).into_iter().filter(|d| d.rect.top <= 640.0).map(|d| d.object).collect();
    assert_eq!(all.len(), 36);
    resizes_as_one_at_a_time_did(&busy_page, all, Point { x: 60.0, y: 92.0 }, 1.5, 0.8);
}

#[test]
fn a_group_resize_is_undone_in_one_step() {
    let Some(()) = skip_without_pdfium() else { return };
    let _lock = serial();
    let mut doc = busy_page();
    let before = top_level(&doc);
    let objects: Vec<usize> = before.iter().filter(|d| d.kind == DrawnKind::Shape).map(|d| d.object).collect();
    let mut history = CommandHistory::new(8);
    history
        .execute(Command::ScaleObjects { page_index: 0, objects, anchor: Point { x: 60.0, y: 92.0 }, sx: 2.0, sy: 1.5 }, &mut doc)
        .expect("resize");
    history.undo(&mut doc).expect("undo").expect("a step to undo");
    assert_moved(&before, &top_level(&doc), &[], Point { x: 0.0, y: 0.0 });
}

/// The lens and the paragraph from the owner's session, resized as groups:
/// where one at a time put them, and quickly.
#[test]
fn resizing_the_lens_and_the_paragraph_takes_one_edit_each() {
    let Some(()) = skip_without_pdfium() else { return };
    if !std::path::Path::new(VIVO).exists() {
        eprintln!("skipped: VIVO mini.pdf is not at {VIVO}");
        return;
    }
    let _lock = serial();
    let vivo = || PdfiumDocument::open_path(VIVO, None).expect("open VIVO");
    let drawn = top_level(&vivo());
    let inside = |d: &&DrawnObject, l: f32, t: f32, r: f32, b: f32| d.rect.left < r && d.rect.right > l && d.rect.top < b && d.rect.bottom > t;
    let lens: Vec<usize> = drawn
        .iter()
        .filter(|d| matches!(d.kind, DrawnKind::Shape | DrawnKind::Picture) && inside(d, 415.0, 106.0, 490.0, 180.0))
        .map(|d| d.object)
        .collect();
    let paragraph: Vec<usize> = drawn
        .iter()
        .filter(|d| d.kind == DrawnKind::Words)
        .filter(|d| d.rect.left >= 190.0 && d.rect.right <= 400.0 && d.rect.top >= 65.0 && d.rect.bottom <= 160.0)
        .map(|d| d.object)
        .collect();
    for (what, objects, anchor) in [("lens", lens, Point { x: 415.0, y: 106.0 }), ("paragraph", paragraph, Point { x: 194.0, y: 69.0 })] {
        let mut doc = vivo();
        let started = Instant::now();
        CommandHistory::new(8)
            .execute(Command::ScaleObjects { page_index: 0, objects: objects.clone(), anchor, sx: 1.2, sy: 1.2 }, &mut doc)
            .expect("resize");
        let ms = started.elapsed().as_millis();
        eprintln!("resized the {what} ({} pieces) in {ms} ms", objects.len());
        assert!(ms < 1000, "the {what} took {ms} ms");
        resizes_as_one_at_a_time_did(&vivo, objects, anchor, 1.2, 1.2);
    }
}

/// **Reported from use on 0.1.79:** 234 things moved together still took 11 s.
/// The one pass refused them — "moving these together would change words on
/// the page" — and the move went the slow way. Nothing had changed: PDFium
/// reads a hyphen at the end of a line as U+0002, and once moved with its
/// words it is no longer at a line end, so it reads as "-". The upper right
/// quarter of VIVO page 1 has such hyphens.
#[test]
fn moving_words_that_end_lines_in_a_hyphen_still_takes_one_pass() {
    use pdf_core::document::DocumentMut;
    let Some(()) = skip_without_pdfium() else { return };
    if !std::path::Path::new(VIVO).exists() {
        eprintln!("skipped: VIVO mini.pdf is not at {VIVO}");
        return;
    }
    let _lock = serial();
    let mut doc = PdfiumDocument::open_path(VIVO, None).expect("open VIVO");
    let size = doc.page_size(0).expect("size");
    let quarter = Rect { left: size.width_pt / 2.0, top: 0.0, right: size.width_pt, bottom: size.height_pt / 2.0 };
    let inside = |r: &Rect| r.left >= quarter.left && r.right <= quarter.right && r.top >= quarter.top && r.bottom <= quarter.bottom;
    // What a marquee over it picks: runs, pictures, top-level shapes.
    let mut objects: Vec<usize> = doc.text_runs(0).expect("runs").into_iter().filter(|r| inside(&r.rect)).map(|r| r.object).collect();
    objects.extend(doc.images_on(0).expect("images").into_iter().filter(|i| inside(&i.rect)).map(|i| i.object));
    objects.extend(
        top_level(&doc).into_iter().filter(|d| d.kind == DrawnKind::Shape && inside(&d.rect)).map(|d| d.object),
    );
    assert!(objects.len() > 200, "{} objects", objects.len());
    let before = top_level(&doc);
    let by = Point { x: 1.0, y: -48.0 };
    doc.move_objects_mut(0, &objects, by).expect("the one pass should take them");
    assert_moved(&before, &top_level(&doc), &objects, by);
}

// -------------------------------------------------------------- clipped --
//
// "The vector lines still go behind a layer that's invisible" (0.1.81, three
// screenshots): VIVO draws its lens inside `q <frame> re W n … Q`, a placed
// figure clipped to its frame. A move wrote each line's shift inside that
// scope, after the clip, so the lines moved and the clip did not: everything
// taken out of the frame was clipped away.

/// Two squares inside a clipped frame, and one outside it.
fn framed_squares() -> PdfiumDocument {
    open(hand_built_pdf(
        &[HandPage {
            content: "q 100 600 100 100 re W n 1 0 0 rg 110 610 30 30 re f 0 0 1 rg 150 650 30 30 re f Q 0 1 0 rg 400 100 30 30 re f",
            resources: "<< >>",
            extra: "",
        }],
        &[],
    ))
}

fn colour_at(doc: &PdfiumDocument, x: f32, y: f32) -> [u8; 3] {
    let size = doc.page_size(0).expect("size");
    let mut b = doc
        .page(0)
        .expect("page")
        .render_region(&pdf_core::document::RegionRequest {
            crop: Rect { left: 0.0, top: 0.0, right: size.width_pt, bottom: size.height_pt },
            scale: 1.0,
            render_annotations: false,
            render_form_data: false,
        })
        .expect("render");
    b.convert_to(pdf_core::render::PixelOrder::Rgba);
    let at = y as usize * b.stride + x as usize * 4;
    [b.data[at], b.data[at + 1], b.data[at + 2]]
}

#[test]
fn everything_in_a_clipped_frame_moved_out_of_it_is_still_seen() {
    let Some(()) = skip_without_pdfium() else { return };
    let _lock = serial();
    let mut doc = framed_squares();
    let before = top_level(&doc);
    let squares: Vec<usize> = before.iter().filter(|d| d.rect.left < 300.0).map(|d| d.object).collect();
    assert_eq!(squares.len(), 2, "{before:?}");
    let by = Point { x: 0.0, y: 300.0 };
    let mut history = CommandHistory::new(8);
    history.execute(Command::MoveObjects { page_index: 0, objects: squares.clone(), by }, &mut doc).expect("move");
    assert_moved(&before, &top_level(&doc), &squares, by);
    // The red square's middle was (125, 167) on the page; it is at (125, 467) now.
    assert_eq!(colour_at(&doc, 125.0, 467.0), [255, 0, 0], "the red square was clipped away");
    assert_eq!(colour_at(&doc, 165.0, 427.0), [0, 0, 255], "the blue square was clipped away");

    history.undo(&mut doc).expect("undo").expect("a step to undo");
    assert_moved(&before, &top_level(&doc), &[], by);
    assert_eq!(colour_at(&doc, 125.0, 167.0), [255, 0, 0], "undo did not bring the red square back");
}

/// The report itself: VIVO's lens drawing, marquee-selected whole and moved
/// out of its frame. The ink it takes away from where it was must turn up
/// where it went — none of it clipped away.
#[test]
fn the_vivo_drawing_moved_out_of_its_frame_is_still_drawn() {
    use pdf_core::document::DocumentMut;
    let Some(()) = skip_without_pdfium() else { return };
    if !std::path::Path::new(VIVO).exists() {
        eprintln!("skipped: VIVO mini.pdf is not at {VIVO}");
        return;
    }
    let _lock = serial();
    let mut doc = PdfiumDocument::open_path(VIVO, None).expect("open VIVO");
    let area = Rect { left: 405.0, top: 90.0, right: 590.0, bottom: 255.0 };
    let inside = |r: &Rect| r.left >= area.left && r.right <= area.right && r.top >= area.top && r.bottom <= area.bottom;
    let mut objects: Vec<usize> = doc.text_runs(0).expect("runs").into_iter().filter(|r| inside(&r.rect)).map(|r| r.object).collect();
    objects.extend(doc.images_on(0).expect("images").into_iter().filter(|i| inside(&i.rect)).map(|i| i.object));
    objects.extend(top_level(&doc).into_iter().filter(|d| d.kind == DrawnKind::Shape && inside(&d.rect)).map(|d| d.object));
    assert!(objects.len() > 80, "{} objects", objects.len());

    let by = Point { x: 0.0, y: 300.0 };
    let went = Rect { left: area.left, top: area.top + by.y, right: area.right, bottom: area.bottom + by.y };
    let ink = |doc: &PdfiumDocument, r: Rect| {
        let size = doc.page_size(0).expect("size");
        let mut b = doc
            .page(0)
            .expect("page")
            .render_region(&pdf_core::document::RegionRequest {
                crop: Rect { left: 0.0, top: 0.0, right: size.width_pt, bottom: size.height_pt },
                scale: 2.0,
                render_annotations: false,
                render_form_data: false,
            })
            .expect("render");
        b.convert_to(pdf_core::render::PixelOrder::Rgba);
        let mut dark = 0usize;
        for y in (r.top * 2.0) as usize..((r.bottom * 2.0) as usize).min(b.height as usize) {
            for x in (r.left * 2.0) as usize..((r.right * 2.0) as usize).min(b.width as usize) {
                let at = y * b.stride + x * 4;
                let lum = 0.299 * b.data[at] as f32 + 0.587 * b.data[at + 1] as f32 + 0.114 * b.data[at + 2] as f32;
                dark += usize::from(lum < 200.0);
            }
        }
        dark
    };
    let (was_here, was_there) = (ink(&doc, area), ink(&doc, went));
    doc.move_objects_mut(0, &objects, by).expect("move the drawing");
    let (now_here, now_there) = (ink(&doc, area), ink(&doc, went));
    let left = was_here.saturating_sub(now_here);
    let arrived = now_there.saturating_sub(was_there);
    eprintln!("ink taken away {left}, ink arrived {arrived}");
    assert!(left > 1000, "the drawing did not leave: {left}");
    assert!(arrived * 10 >= left * 9, "only {arrived} of the {left} dark pixels arrived — the rest were clipped away");
}
