//! Moving one thing on a page without disturbing the rest of it.
//!
//! # Why this is measured rather than assumed
//!
//! Moving an object means PDFium's object model, and committing a change to it
//! means `FPDFPage_GenerateContent` — which re-emits the whole content stream.
//! This crate avoids that everywhere it can, because it has been caught
//! rewriting paragraphs nobody touched: a signature covering 1458 bytes of a
//! 17826-byte re-save, a footer drawn twice, a line of a paragraph landing on
//! the line above it. Every one of those was a re-emission.
//!
//! So before the app offered a move at all, this asked what a move costs. The
//! answer, on a page of two columns: the object goes exactly where it is sent,
//! every other run stays where it was, and the page keeps the same words.
//!
//! ```text
//! PAGIFY_PDFIUM_LIB=<pdfium> cargo test --test moving
//! ```

mod harness;
use harness::{serial, skip_without_pdfium};

use pdf_core::document::pdfium_doc::PdfiumDocument;
use pdf_core::document::{Document, DocumentMut, Point};

fn open(name: &str) -> PdfiumDocument {
    PdfiumDocument::open_path(harness::fixture_path(name).to_str().expect("path"), None)
        .expect("open")
}

/// **It goes where it is sent.**
#[test]
fn an_object_moves_by_exactly_the_distance_asked_for() {
    let Some(_) = skip_without_pdfium() else { return };
    let _lock = serial();

    let mut doc = open("two-column.pdf");
    let was = doc.object_bounds(0, 0).expect("bounds");
    doc.move_object(0, 0, Point { x: 20.0, y: 12.0 }).expect("move");
    let now = doc.object_bounds(0, 0).expect("bounds");

    assert!((now.left - was.left - 20.0).abs() < 0.01, "across: {was:?} then {now:?}");
    // Page space counts downwards, so a positive move goes down the page.
    assert!((now.top - was.top - 12.0).abs() < 0.01, "down: {was:?} then {now:?}");
}

/// **And nothing else does.** The whole reason this is measured.
#[test]
fn moving_one_object_leaves_every_other_run_where_it_was() {
    let Some(_) = skip_without_pdfium() else { return };
    let _lock = serial();

    for object in [0usize, 3, 7] {
        let doc = open("two-column.pdf");
        let before = doc.text_runs(0).expect("runs");
        drop(doc);

        let mut doc = open("two-column.pdf");
        doc.move_object(0, object, Point { x: 20.0, y: 12.0 }).expect("move");
        let after = doc.text_runs(0).expect("runs");

        assert_eq!(after.len(), before.len(), "moving object {object} changed the run count");
        for (was, now) in before.iter().zip(after.iter()) {
            if was.object == object {
                continue;
            }
            assert!(
                (was.rect.left - now.rect.left).abs() < 0.1
                    && (was.rect.top - now.rect.top).abs() < 0.1,
                "moving object {object} shifted another run: {:?} then {:?}",
                was.rect,
                now.rect
            );
        }

        // Reading order follows position, so moving a *text* object reorders
        // the page — correctly. What must not change is which words are on it.
        let mut spoken: Vec<String> = before.iter().map(|r| r.text.trim().to_string()).collect();
        let mut now: Vec<String> = after.iter().map(|r| r.text.trim().to_string()).collect();
        spoken.sort();
        now.sort();
        assert_eq!(spoken, now, "moving object {object} changed the page's words");
    }
}

/// **A move that would rewrite the page is undone rather than kept.**
///
/// Committing a move needs `FPDFPage_GenerateContent`, which re-emits the whole
/// content stream — and on a real catalogue that came back scrambled:
/// `HSI Lighting I HUE . SATURATION` as `ABOThe Boer T UOur e xpiunpics`, the
/// page's vertically-set heading broken into pieces. Reported from use, with
/// screenshots, *after* the feature had been offered.
///
/// **No fixture here reproduces it** — every one of them is simple enough that
/// PDFium round-trips it cleanly, which is exactly why the first measurement
/// said moving was safe. So this checks the half that can be checked: a move
/// that goes through leaves the page's words alone. The other half is the guard
/// itself, and the evidence for it is a real document.
#[test]
fn a_move_that_goes_through_has_not_rewritten_anything() {
    let Some(_) = skip_without_pdfium() else { return };
    let _lock = serial();

    for name in ["two-column.pdf", "text-lines.pdf", "spread.pdf", "mixed-sizes.pdf"] {
        let doc = open(name);
        if doc.text_runs(0).map(|r| r.is_empty()).unwrap_or(true) {
            continue;
        }
        let mut before: Vec<String> =
            doc.text_runs(0).expect("runs").iter().map(|r| r.text.trim().to_string()).collect();
        drop(doc);

        let mut doc = open(name);
        if doc.move_object(0, 0, Point { x: 10.0, y: 6.0 }).is_err() {
            // The guard refused, which is the other honest answer.
            continue;
        }
        let mut after: Vec<String> =
            doc.text_runs(0).expect("runs").iter().map(|r| r.text.trim().to_string()).collect();
        before.sort();
        after.sort();
        assert_eq!(before, after, "{name}: a move that was allowed still rewrote the page");
    }
}

/// An object that is not there is said so, rather than moving something else.
#[test]
fn moving_an_object_that_is_not_there_is_refused() {
    let Some(_) = skip_without_pdfium() else { return };
    let _lock = serial();

    let mut doc = open("two-column.pdf");
    assert!(doc.move_object(0, 100_000, Point { x: 1.0, y: 1.0 }).is_err());
    assert!(doc.move_object(99, 0, Point { x: 1.0, y: 1.0 }).is_err());
}

/// Moving marks the document as owing a save, like any other change.
#[test]
fn moving_something_counts_as_a_change() {
    let Some(_) = skip_without_pdfium() else { return };
    let _lock = serial();

    let mut doc = open("two-column.pdf");
    assert!(!doc.is_dirty(), "a freshly opened document already looks changed");
    doc.move_object(0, 0, Point { x: 5.0, y: 0.0 }).expect("move");
    assert!(doc.is_dirty(), "a move was not counted as work owing a save");
}

/// **A picture moves without the page being re-emitted at all.**
///
/// Reported from use, with screenshots: moving something rearranged the page
/// around it — a heading broken into pieces, blocks of text overlapping, a
/// black rectangle where an image had been. The cause was
/// `FPDFPage_GenerateContent`, which every move through PDFium's object model
/// has to end with.
///
/// A picture is drawn by one operator, so wrapping *that* operator in its own
/// `q … Q` moves it and can reach nothing else — `Q` puts the graphics state
/// back exactly as it found it, and every other byte is copied through.
///
/// The distance is carried back through the transform in force at the operator.
/// Written naively it lands inside the picture's own scaling: measured, twenty
/// points came out as three thousand nine hundred.
#[test]
fn a_picture_moves_exactly_and_the_page_around_it_does_not() {
    let Some(_) = skip_without_pdfium() else { return };
    let _lock = serial();

    let mut tried = 0;
    for name in ["scan-300dpi.pdf", "scan-lowdpi.pdf", "two-column.pdf", "spread.pdf"] {
        let doc = open(name);
        let pictures = doc.images_on(0).unwrap_or_default();
        let Some(picture) = pictures.first().cloned() else { continue };
        let mut before: Vec<String> =
            doc.text_runs(0).expect("runs").iter().map(|r| r.text.trim().to_string()).collect();
        drop(doc);
        tried += 1;

        let mut doc = open(name);
        doc.move_object(0, picture.object, Point { x: 20.0, y: 12.0 })
            .unwrap_or_else(|e| panic!("{name}: a picture would not move — {e}"));

        let moved = doc
            .images_on(0)
            .expect("images")
            .into_iter()
            .find(|i| i.object == picture.object)
            .unwrap_or_else(|| panic!("{name}: the picture vanished"));
        assert!(
            (moved.rect.left - picture.rect.left - 20.0).abs() < 0.5,
            "{name}: went {:.2} across, not 20",
            moved.rect.left - picture.rect.left
        );
        assert!(
            (moved.rect.top - picture.rect.top - 12.0).abs() < 0.5,
            "{name}: went {:.2} down, not 12",
            moved.rect.top - picture.rect.top
        );

        let mut after: Vec<String> =
            doc.text_runs(0).expect("runs").iter().map(|r| r.text.trim().to_string()).collect();
        before.sort();
        after.sort();
        assert_eq!(before, after, "{name}: moving a picture rewrote the page's words");
    }
    assert!(tried > 0, "no fixture here has a picture on its first page");
}

/// **A run of words moves without the page being re-emitted.**
///
/// The guarded path works and its cost is refusing on the documents worth
/// editing — reported from use as *"while moving text I am getting a warning
/// saying moving rewrites the text so nothing was moved"*. So the run's text
/// matrix is shifted in the stream instead, and every other byte of the page is
/// copied through. Measured on a real report: 32 runs of 32 moved to within
/// half a point, none refused, none of the pages' words changed.
#[test]
fn a_run_of_words_moves_exactly_without_the_page_being_rewritten() {
    let Some(_) = skip_without_pdfium() else { return };
    let _lock = serial();

    let by = Point { x: 20.0, y: 12.0 };
    let doc = open("two-column.pdf");
    let runs = doc.text_runs(0).expect("runs");
    let first = runs.first().cloned().expect("a run");
    drop(doc);

    let mut doc = open("two-column.pdf");
    doc.try_move_run_in_stream(0, first.object, by)
        .expect("the byte-safe path should take this run");

    let after = doc.text_runs(0).expect("runs");
    let now = after.iter().find(|r| r.object == first.object).expect("the run is still there");
    assert!(
        (now.rect.left - first.rect.left - by.x).abs() < 0.5,
        "across: {:?} then {:?}",
        first.rect,
        now.rect
    );
    assert!(
        (now.rect.top - first.rect.top - by.y).abs() < 0.5,
        "down: {:?} then {:?}",
        first.rect,
        now.rect
    );
}

/// **And the lines after it stay where they were.**
///
/// The half that is easy to get wrong. `Td` moves relative to the *line*
/// matrix, and a `Tm` sets both matrices — so shifting the run and stopping
/// there drags every following line of the paragraph along with it. The
/// restoring `Tm` is what stops that, and this is what would catch its removal.
#[test]
fn moving_a_run_leaves_the_lines_after_it_alone() {
    let Some(_) = skip_without_pdfium() else { return };
    let _lock = serial();

    let by = Point { x: 20.0, y: 12.0 };
    let doc = open("two-column.pdf");
    let before = doc.text_runs(0).expect("runs");
    let target = before.first().cloned().expect("a run");
    drop(doc);

    let mut doc = open("two-column.pdf");
    doc.try_move_run_in_stream(0, target.object, by).expect("move");
    let after = doc.text_runs(0).expect("runs");

    assert_eq!(after.len(), before.len(), "the run count changed");
    let mut disturbed = Vec::new();
    for was in &before {
        if was.object == target.object {
            continue;
        }
        let Some(now) = after.iter().find(|r| r.object == was.object) else {
            panic!("run {} disappeared", was.object);
        };
        if (now.rect.left - was.rect.left).abs() > 0.5 || (now.rect.top - was.rect.top).abs() > 0.5
        {
            disturbed.push((was.text.trim().to_string(), was.rect, now.rect));
        }
    }
    assert!(disturbed.is_empty(), "moving one run shifted others: {disturbed:#?}");

    let mut spoken: Vec<String> = before.iter().map(|r| r.text.trim().to_string()).collect();
    let mut now: Vec<String> = after.iter().map(|r| r.text.trim().to_string()).collect();
    spoken.sort();
    now.sort();
    assert_eq!(spoken, now, "the page's words changed");
}

/// **A picture moves, whether or not it sits inside another transform.**
///
/// The second one on this fixture is placed by a `cm` inside a `cm`, which is
/// how a real producer nests a placed graphic. A translation written naively
/// inside that lands multiplied by the picture's own scaling — measured once at
/// 3916 points for a 20-point move.
#[test]
fn a_picture_moves_exactly_even_when_its_placement_is_nested() {
    let Some(_) = skip_without_pdfium() else { return };
    let _lock = serial();

    let by = Point { x: 20.0, y: 12.0 };
    let doc = open("pictures.pdf");
    let pictures = doc.images_on(0).expect("images");
    assert_eq!(pictures.len(), 2, "the fixture should have two pictures");
    let words: Vec<String> =
        doc.text_runs(0).expect("runs").iter().map(|r| r.text.trim().to_string()).collect();
    drop(doc);

    for picture in &pictures {
        let mut doc = open("pictures.pdf");
        doc.move_object(0, picture.object, by).expect("move the picture");

        let after = doc.images_on(0).expect("images");
        assert_eq!(after.len(), 2, "a picture disappeared");
        let now = after
            .iter()
            .find(|i| i.object == picture.object)
            .expect("the picture is still there");
        assert!(
            (now.rect.left - picture.rect.left - by.x).abs() < 0.5
                && (now.rect.top - picture.rect.top - by.y).abs() < 0.5,
            "object {} went to {:?} from {:?}",
            picture.object,
            now.rect,
            picture.rect
        );

        // The other picture, and the words, exactly as they were.
        for other in &pictures {
            if other.object == picture.object {
                continue;
            }
            let still = after.iter().find(|i| i.object == other.object).expect("the other picture");
            assert!(
                (still.rect.left - other.rect.left).abs() < 0.01
                    && (still.rect.top - other.rect.top).abs() < 0.01,
                "moving object {} dragged object {} along",
                picture.object,
                other.object
            );
        }
        let now_words: Vec<String> =
            doc.text_runs(0).expect("runs").iter().map(|r| r.text.trim().to_string()).collect();
        assert_eq!(now_words, words, "moving a picture changed the page's words");
    }
}

/// **An object index still names the same words after a move.**
///
/// The app depends on this and would fail quietly without it: the run editor
/// keeps the object it is editing across a drag of the move grip, nudges its
/// box, and writes the edited text to that same index when the reader presses
/// Enter. A move that renumbered the page would send those words to a
/// different run.
///
/// It is not obvious that it holds. The move inserts two `Tm` operators into
/// the middle of a text object, and PDFium builds its object list by grouping
/// the stream's operators — so a positioning operator in the wrong place could
/// split one run into two and shift every index after it. Measured on a real
/// report and on this fixture: 34 moves, no index shifted.
#[test]
fn moving_a_run_leaves_every_object_index_naming_what_it_named() {
    let Some(_) = skip_without_pdfium() else { return };
    let _lock = serial();

    let by = Point { x: 20.0, y: 12.0 };
    let doc = open("two-column.pdf");
    let before: Vec<(usize, String)> = doc
        .text_runs(0)
        .expect("runs")
        .iter()
        .map(|r| (r.object, r.text.trim().to_string()))
        .collect();
    drop(doc);

    for (object, _) in before.iter().take(6) {
        let mut doc = open("two-column.pdf");
        if doc.try_move_run_in_stream(0, *object, by).is_err() {
            continue;
        }
        let after: Vec<(usize, String)> = doc
            .text_runs(0)
            .expect("runs")
            .iter()
            .map(|r| (r.object, r.text.trim().to_string()))
            .collect();

        for (index, words) in &before {
            let now = after.iter().find(|(o, _)| o == index).map(|(_, w)| w.as_str());
            assert_eq!(
                now,
                Some(words.as_str()),
                "after moving object {object}, index {index} no longer names the \
                 words it did — the editor would write onto the wrong run"
            );
        }
    }
}

/// **Moving words does not repaint them.**
///
/// Reported from use: "the text still changes colours when moved." Nothing in
/// this crate sets a colour during a move — but the fallback path commits
/// through `FPDFPage_GenerateContent`, which re-emits the whole content stream,
/// and a re-emission can keep every word and every position and still come back
/// painted differently. The words-and-bounds guard let that through.
///
/// Asserted on a **render**, because that is the only thing that answers what
/// the reader sees. Every other fixture here is black on white, where a colour
/// change would look exactly like no change at all.
#[test]
fn moving_words_leaves_the_pages_colours_exactly_as_they_were() {
    let Some(_) = skip_without_pdfium() else { return };
    let _lock = serial();

    /// How many pixels of each colour a render of the page has.
    ///
    /// Quantised, so that anti-aliasing along a glyph edge does not invent
    /// hundreds of near-identical entries and drown the signal.
    fn palette(doc: &dyn Document, page: usize) -> std::collections::BTreeMap<(u8, u8, u8), usize> {
        let size = doc.page_size(page).expect("size");
        let width = 300u32;
        let scale = width as f32 / size.width_pt;
        let height = (size.height_pt * scale).max(1.0) as u32;

        let mut pixels = vec![0u8; (width * height * 4) as usize];
        let mut target = pdf_core::render::RenderTarget {
            width,
            height,
            stride: (width * 4) as usize,
            order: pdf_core::render::PixelOrder::Rgba,
            pixels: &mut pixels,
        };
        doc.page(page)
            .expect("page")
            .render_into(
                &pdf_core::document::RenderRequest { scale, ..Default::default() },
                &mut target,
            )
            .expect("render");

        let mut counts = std::collections::BTreeMap::new();
        for p in pixels.chunks_exact(4) {
            let key = (p[0] & 0xE0, p[1] & 0xE0, p[2] & 0xE0);
            *counts.entry(key).or_insert(0usize) += 1;
        }
        counts
    }

    let doc = open("coloured.pdf");
    let before = palette(&doc, 0);
    let runs = doc.text_runs(0).expect("runs");
    assert!(
        before.len() >= 4,
        "the fixture is not colourful enough to prove anything: {before:?}"
    );
    drop(doc);

    for run in runs.iter() {
        let mut doc = open("coloured.pdf");
        // Small enough that nothing leaves the page, so the same ink is still
        // being counted.
        if doc.move_object(0, run.object, Point { x: 6.0, y: 4.0 }).is_err() {
            continue;
        }
        let after = palette(&doc, 0);

        // Every colour the page is actually *painted* in is still there, in
        // roughly the count it had — the words moved, they did not change
        // colour or disappear.
        //
        // Buckets of a handful of pixels are the anti-aliased edges between two
        // real colours, and they shift by a pixel or two whenever anything
        // moves. Counting those as evidence would make this test fail on the
        // thing it is meant to permit, so the floor is what a line of 24-point
        // words covers at this size.
        const REAL: usize = 200;
        for (colour, was) in before.iter().filter(|(_, n)| **n >= REAL) {
            let now = after.get(colour).copied().unwrap_or(0);
            let drift = (now as f32 - *was as f32).abs() / *was as f32;
            assert!(
                drift < 0.15,
                "moving {:?} changed how much of the page is {colour:?}: {was} then {now}",
                run.text.trim()
            );
        }
        // And no colour the page did not have appeared in any quantity.
        for (colour, now) in after.iter().filter(|(_, n)| **n >= REAL) {
            assert!(
                before.contains_key(colour),
                "moving {:?} painted the page a colour it did not have: {colour:?} × {now}",
                run.text.trim()
            );
        }
    }
}

/// **A drawn shape moves like anything else.**
///
/// Asked for from use: *"I need to move shapes as well"*. A path is pure
/// graphics, so the operators that paint it can be wrapped in `q`/`Q` with a
/// `cm` between — which is legal where a text object's are not, and reaches
/// nothing else on the page. Measured on a real report: 26 shapes of 26 moved
/// to within half a point, none refused, none disturbing a neighbour.
#[test]
fn a_drawn_shape_moves_exactly_and_leaves_the_page_alone() {
    let Some(_) = skip_without_pdfium() else { return };
    let _lock = serial();

    let by = Point { x: 20.0, y: 12.0 };
    let doc = open("covered.pdf");
    let before = doc.drawn_objects(0).expect("objects");
    let shape = before
        .iter()
        .find(|d| d.kind == pdf_core::document::DrawnKind::Shape)
        .cloned()
        .expect("the fixture's panel");
    drop(doc);

    let mut doc = open("covered.pdf");
    doc.move_object(0, shape.object, by).expect("move the shape");

    let after = doc.drawn_objects(0).expect("objects");
    assert_eq!(after.len(), before.len(), "moving a shape changed what the page draws");
    let now = after.iter().find(|d| d.object == shape.object).expect("the shape is still there");
    assert!(
        (now.rect.left - shape.rect.left - by.x).abs() < 0.5
            && (now.rect.top - shape.rect.top - by.y).abs() < 0.5,
        "the shape went to {:?} from {:?}",
        now.rect,
        shape.rect
    );

    // And nothing else moved.
    for was in &before {
        if was.object == shape.object {
            continue;
        }
        let still = after.iter().find(|d| d.object == was.object).expect("it is still there");
        assert!(
            (still.rect.left - was.rect.left).abs() < 0.5
                && (still.rect.top - was.rect.top).abs() < 0.5,
            "moving the shape dragged {:?} along with it",
            was.label
        );
    }
}

/// **A shape that also sets a clip does not get wrapped.**
///
/// `W f` is one path doing two jobs: it paints, so it is an object somebody can
/// click, and it clips, so everything after it is held inside it. `q`/`Q` saves
/// and restores the clipping path — so wrapping that path to move it would put
/// the clip back the instant the wrapper closed, and the words it was holding
/// in would spill across the page.
///
/// Asserted on what the page *draws*, not on the error: the guarded path may
/// still take the move, and what must never happen is the clip being lost.
#[test]
fn a_shape_that_sets_a_clip_does_not_lose_its_clip() {
    let Some(_) = skip_without_pdfium() else { return };
    let _lock = serial();

    let doc = open("clipped.pdf");
    let stream = doc.page_stream(0).expect("stream");
    let operations = pdf_core::pdf::content::parse(&stream).expect("parse");
    assert!(
        operations.iter().any(|o| matches!(o.operator.as_slice(), b"W" | b"W*")),
        "the fixture no longer sets a clip, so this proves nothing"
    );
    let panel = doc
        .drawn_objects(0)
        .expect("objects")
        .into_iter()
        .find(|d| d.kind == pdf_core::document::DrawnKind::Shape)
        .expect("the clipping panel");
    // The clipped line is wider than the panel, so the clip is what keeps it in.
    let held = doc
        .text_runs(0)
        .expect("runs")
        .into_iter()
        .find(|r| r.text.contains("Held inside"))
        .expect("the clipped words");
    let width_before = held.rect.right - held.rect.left;
    drop(doc);

    let mut doc = open("clipped.pdf");
    // The byte-safe wrapper must decline this one outright.
    assert!(
        doc.try_move_run_in_stream(0, panel.object, Point { x: 20.0, y: 12.0 }).is_err(),
        "a clipping panel should not be wrapped"
    );

    // However the move is finally made — or refused — the clip must still be
    // holding those words in.
    let _ = doc.move_object(0, panel.object, Point { x: 20.0, y: 12.0 });
    if let Some(now) = doc
        .text_runs(0)
        .expect("runs")
        .into_iter()
        .find(|r| r.text.contains("Held inside"))
    {
        assert!(
            (now.rect.right - now.rect.left) <= width_before + 1.0,
            "the clip was lost — the words spread from {width_before:.0}pt to {:.0}pt",
            now.rect.right - now.rect.left
        );
    }
}

/// **A framed picture moves with its frame, and its placeholder goes too.**
///
/// The way a design program places every picture: a grey rectangle the size
/// of the frame, then a clip exactly that size, then the picture inside it.
/// Moving the `Do` alone slid the picture out of its own clip and off into
/// nothing, and left the grey rectangle showing — seen on a real brochure,
/// where it was reported as a grey layer that nothing could be brought in
/// front of. Nothing could: the picture was no longer being drawn at all.
///
/// Asserted on a render, because the object list said the picture had moved
/// and was still there — it was the clip that had the last word.
#[test]
fn a_framed_picture_is_still_visible_after_it_moves_and_takes_its_placeholder() {
    let Some(_) = skip_without_pdfium() else { return };
    let _lock = serial();

    fn red_pixels(doc: &dyn Document) -> Vec<(u32, u32)> {
        let size = doc.page_size(0).expect("size");
        let width = 306u32;
        let scale = width as f32 / size.width_pt;
        let height = (size.height_pt * scale).max(1.0) as u32;
        let mut pixels = vec![0u8; (width * height * 4) as usize];
        let mut target = pdf_core::render::RenderTarget {
            width,
            height,
            stride: (width * 4) as usize,
            order: pdf_core::render::PixelOrder::Rgba,
            pixels: &mut pixels,
        };
        doc.page(0)
            .expect("page")
            .render_into(
                &pdf_core::document::RenderRequest { scale, ..Default::default() },
                &mut target,
            )
            .expect("render");
        let mut found = Vec::new();
        for (i, p) in pixels.chunks_exact(4).enumerate() {
            if p[0] > 180 && p[1] < 80 && p[2] < 80 {
                found.push(((i as u32) % width, (i as u32) / width));
            }
        }
        found
    }
    fn grey_pixels(doc: &dyn Document) -> usize {
        let size = doc.page_size(0).expect("size");
        let width = 306u32;
        let scale = width as f32 / size.width_pt;
        let height = (size.height_pt * scale).max(1.0) as u32;
        let mut pixels = vec![0u8; (width * height * 4) as usize];
        let mut target = pdf_core::render::RenderTarget {
            width,
            height,
            stride: (width * 4) as usize,
            order: pdf_core::render::PixelOrder::Rgba,
            pixels: &mut pixels,
        };
        doc.page(0)
            .expect("page")
            .render_into(
                &pdf_core::document::RenderRequest { scale, ..Default::default() },
                &mut target,
            )
            .expect("render");
        // The placeholder's 0.659 0.662 0.664, within a shade.
        pixels
            .chunks_exact(4)
            .filter(|p| (p[0] as i32 - 168).abs() < 8 && (p[1] as i32 - 169).abs() < 8 && (p[2] as i32 - 169).abs() < 8)
            .count()
    }

    let doc = open("framed.pdf");
    let picture = doc.images_on(0).expect("images")[0].clone();
    let red_before = red_pixels(&doc);
    assert!(red_before.len() > 500, "the fixture should draw the picture: {} red pixels", red_before.len());
    // Under the picture to begin with — bar the anti-aliased edge.
    let grey_before = grey_pixels(&doc);
    drop(doc);

    let mut doc = open("framed.pdf");
    // Well past the frame's own edge.
    let by = Point { x: 60.0, y: 200.0 };
    doc.move_object(0, picture.object, by).expect("move");

    // The picture is still drawn, as much of it as before, and where it was
    // sent — the clip went with it.
    let red_after = red_pixels(&doc);
    assert!(
        red_after.len() as f32 > red_before.len() as f32 * 0.9,
        "the picture was cut off by the frame it moved out of: {} red pixels, was {}",
        red_after.len(),
        red_before.len()
    );
    let centre = |px: &[(u32, u32)]| {
        let n = px.len() as f32;
        (px.iter().map(|p| p.0 as f32).sum::<f32>() / n, px.iter().map(|p| p.1 as f32).sum::<f32>() / n)
    };
    let (was, now) = (centre(&red_before), centre(&red_after));
    let scale = 306.0 / 612.0;
    assert!(
        (now.0 - was.0 - by.x * scale).abs() < 2.0 && (now.1 - was.1 - by.y * scale).abs() < 2.0,
        "the picture did not land where it was sent: centre {was:?} then {now:?}"
    );

    // And no grey block was left behind: the placeholder went with it. Left
    // behind, it would be a 100 × 75 px block — thousands of grey pixels.
    let grey_after = grey_pixels(&doc);
    assert!(
        grey_after <= grey_before + 200,
        "the placeholder was left behind as a grey block: {grey_after} grey pixels, was {grey_before}"
    );
}
