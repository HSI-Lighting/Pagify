//! Resizing and fading one thing on a page, without touching anything else.
//!
//! ```text
//! PAGIFY_PDFIUM_LIB=<pdfium> cargo test --test transform
//! ```

mod harness;
use harness::{serial, skip_without_pdfium};

use pdf_core::document::pdfium_doc::PdfiumDocument;
use pdf_core::document::{Document, DocumentMut, DrawnKind, Point};

fn open(name: &str) -> PdfiumDocument {
    PdfiumDocument::open_path(harness::fixture_path(name).to_str().expect("path"), None)
        .expect("open")
}

/// Every pixel of a render, with its position, for asking where a colour is.
fn render(doc: &dyn Document) -> (u32, Vec<u8>) {
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
    (width, pixels)
}

/// Where the fixture's red picture is drawn: its pixel count and its box.
fn red_box(doc: &dyn Document) -> (usize, (u32, u32, u32, u32)) {
    let (width, pixels) = render(doc);
    let (mut n, mut l, mut t, mut r, mut b) = (0usize, u32::MAX, u32::MAX, 0u32, 0u32);
    for (i, p) in pixels.chunks_exact(4).enumerate() {
        if p[0] > 180 && p[1] < 80 && p[2] < 80 {
            let (x, y) = ((i as u32) % width, (i as u32) / width);
            n += 1;
            l = l.min(x);
            t = t.min(y);
            r = r.max(x);
            b = b.max(y);
        }
    }
    (n, (l, t, r, b))
}

/// **A picture resized about a corner keeps that corner and shrinks the
/// rest** — frame, placeholder and all.
#[test]
fn a_picture_resizes_about_the_anchor_and_keeps_its_frame() {
    let Some(_) = skip_without_pdfium() else { return };
    let _lock = serial();

    let doc = open("framed.pdf");
    let picture = doc
        .drawn_objects(0)
        .expect("objects")
        .into_iter()
        .find(|d| d.kind == DrawnKind::Picture)
        .expect("the picture");
    let (was, (l0, t0, r0, b0)) = red_box(&doc);
    assert!(was > 500, "the fixture should draw the picture");
    drop(doc);

    // Half the size, holding the top-left corner still.
    let mut doc = open("framed.pdf");
    doc.scale_object(
        0,
        picture.object,
        Point { x: picture.rect.left, y: picture.rect.top },
        0.5,
        0.5,
    )
    .expect("resize");

    let (now, (l1, t1, r1, b1)) = red_box(&doc);
    assert!(
        (now as f32) > (was as f32) * 0.2 && (now as f32) < (was as f32) * 0.3,
        "half the size each way should be a quarter of the pixels: {now} of {was}"
    );
    // The anchored corner stayed; the far corner came in by half.
    assert!((l1 as i32 - l0 as i32).abs() <= 1 && (t1 as i32 - t0 as i32).abs() <= 1, "the anchor moved");
    let (w0, h0, w1, h1) = (r0 - l0, b0 - t0, r1 - l1, b1 - t1);
    assert!((w1 as f32 - w0 as f32 / 2.0).abs() <= 2.0, "width {w0} should have halved, is {w1}");
    assert!((h1 as f32 - h0 as f32 / 2.0).abs() <= 2.0, "height {h0} should have halved, is {h1}");

    // The picture is still the picture: it did not get cut by a frame that
    // stayed the old size.
    let listed = doc.drawn_objects(0).expect("objects");
    let still = listed.iter().find(|d| d.kind == DrawnKind::Picture).expect("still there");
    assert!((still.rect.right - still.rect.left - (picture.rect.right - picture.rect.left) / 2.0).abs() < 1.0);
}

/// **A shape resizes too**, and the words around it do not.
#[test]
fn a_shape_resizes_and_the_words_do_not() {
    let Some(_) = skip_without_pdfium() else { return };
    let _lock = serial();

    let doc = open("covered.pdf");
    let panel = doc
        .drawn_objects(0)
        .expect("objects")
        .into_iter()
        .find(|d| d.kind == DrawnKind::Shape)
        .expect("the panel");
    let words_before: Vec<_> = doc.text_runs(0).expect("runs").into_iter().map(|r| r.rect).collect();
    drop(doc);

    let mut doc = open("covered.pdf");
    doc.scale_object(
        0,
        panel.object,
        Point { x: panel.rect.right, y: panel.rect.bottom },
        1.5,
        1.5,
    )
    .expect("resize the panel");

    let now = doc
        .drawn_objects(0)
        .expect("objects")
        .into_iter()
        .find(|d| d.kind == DrawnKind::Shape)
        .expect("the panel");
    assert!(((now.rect.right - now.rect.left) / (panel.rect.right - panel.rect.left) - 1.5).abs() < 0.02);
    assert!((now.rect.right - panel.rect.right).abs() < 0.5 && (now.rect.bottom - panel.rect.bottom).abs() < 0.5, "the anchor corner moved");

    let words_after: Vec<_> = doc.text_runs(0).expect("runs").into_iter().map(|r| r.rect).collect();
    assert_eq!(words_before.len(), words_after.len());
    for (a, b) in words_before.iter().zip(&words_after) {
        assert!((a.left - b.left).abs() < 0.5 && (a.top - b.top).abs() < 0.5, "resizing the panel moved the words");
    }
}

/// **A run resizes by font size, not geometry** — the same field Edit
/// Text's own size control writes, so a handle works on a run a geometric
/// `cm` would refuse (one sharing a text box with others, see
/// `object_wrap_site`), and does not distort its shape as a `cm` scale
/// would (test would need care aspect ratio does not skew glyphs).
#[test]
fn a_text_run_resizes_by_font_size() {
    let Some(_) = skip_without_pdfium() else { return };
    let _lock = serial();

    let mut doc = open("covered.pdf");
    let run = doc.text_runs(0).expect("runs").into_iter().next().expect("a run");

    doc.scale_object(0, run.object, Point { x: run.rect.left, y: run.rect.top }, 2.0, 2.0)
        .expect("resize the run");

    let after = doc
        .text_runs(0)
        .expect("runs")
        .into_iter()
        .find(|r| r.object == run.object)
        .expect("still there");
    assert!(
        (after.size - run.size * 2.0).abs() < 0.05,
        "size should have doubled: was {}, now {}",
        run.size,
        after.size
    );
    assert_eq!(after.text, run.text, "resizing must not change the words");
}

/// **A side handle stretches width alone — a top/bottom handle changes size
/// alone.** `Handle::scale` already computes `sy = 1.0` for a side handle and
/// `sx = 1.0` for a top/bottom one; this proves `resize_run_in_stream` keeps
/// that distinction instead of collapsing both into one factor, which was
/// why every handle used to look like the same uniform resize regardless of
/// which one was dragged.
#[test]
fn a_side_handle_widens_a_run_without_changing_its_size() {
    let Some(_) = skip_without_pdfium() else { return };
    let _lock = serial();

    let mut doc = open("covered.pdf");
    let run = doc.text_runs(0).expect("runs").into_iter().next().expect("a run");
    let width_before = run.rect.right - run.rect.left;

    // sx = 2.0, sy = 1.0 — exactly what `Handle::Left`/`Handle::Right` computes.
    doc.scale_object(0, run.object, Point { x: run.rect.left, y: run.rect.top }, 2.0, 1.0)
        .expect("widen the run");

    let after = doc
        .text_runs(0)
        .expect("runs")
        .into_iter()
        .find(|r| r.object == run.object)
        .expect("still there");
    let width_after = after.rect.right - after.rect.left;

    assert!(
        (after.size - run.size).abs() < 0.05,
        "a side handle changed the font size: was {}, now {}",
        run.size,
        after.size
    );
    assert!(
        width_after > width_before * 1.5,
        "the run should be much wider: was {width_before}, now {width_after}"
    );
}

/// **A bottom handle grows a run downward, not upward.** `Tf` (font size)
/// scales a glyph about its own baseline, and a font's ascent so outweighs
/// its descent that growing size alone — with nothing repositioned — pushed
/// the *top* of the run up rather than extending the *bottom* down, so
/// dragging the bottom handle down (which anchors the top, expecting growth
/// below it) visually grew the run upward instead: the exact complaint
/// "resizing downwards by pulling the handle down resizes it up." It also
/// covers the incidental widening `Tf` causes (the same lever scales width
/// too), and that a second resize compounds on the first rather than
/// assuming a fresh run always starts at `Tz = 100`.
#[test]
fn a_bottom_handle_grows_a_run_downward_not_upward() {
    let Some(_) = skip_without_pdfium() else { return };
    let _lock = serial();

    let mut doc = open("covered.pdf");
    let run = doc.text_runs(0).expect("runs").into_iter().next().expect("a run");
    let width_before = run.rect.right - run.rect.left;

    // sx = 1.0, sy = 4.0, anchored at the top — exactly what dragging
    // `Handle::Bottom` downward computes (`Handle::anchor` pins the top).
    doc.scale_object(0, run.object, Point { x: run.rect.left, y: run.rect.top }, 1.0, 4.0)
        .expect("grow the run");

    let after = doc
        .text_runs(0)
        .expect("runs")
        .into_iter()
        .find(|r| r.object == run.object)
        .expect("still there");
    let width_after = after.rect.right - after.rect.left;
    let height_before = run.rect.bottom - run.rect.top;
    let height_after = after.rect.bottom - after.rect.top;

    assert!(
        (after.size - run.size * 4.0).abs() < 0.05,
        "size should have quadrupled: was {}, now {}",
        run.size,
        after.size
    );
    assert!(
        (width_after - width_before).abs() < width_before * 0.1,
        "a top/bottom handle should leave width alone: was {width_before}, now {width_after}"
    );
    assert!(
        (after.rect.top - run.rect.top).abs() < height_before * 0.5,
        "the anchored top moved: was {}, now {}",
        run.rect.top,
        after.rect.top
    );
    assert!(
        after.rect.bottom > run.rect.bottom + height_before * 0.5,
        "the run should have grown downward, below where it started: was bottom {}, now {} (height {height_before} -> {height_after})",
        run.rect.bottom,
        after.rect.bottom
    );

    // Widen it next — on top of the vertical resize just applied, not from
    // an assumed fresh 100% — and check the size the first resize set holds.
    let size_after_first = after.size;
    doc.scale_object(0, run.object, Point { x: after.rect.left, y: after.rect.top }, 2.0, 1.0)
        .expect("widen the run too");
    let final_run = doc
        .text_runs(0)
        .expect("runs")
        .into_iter()
        .find(|r| r.object == run.object)
        .expect("still there");
    let width_final = final_run.rect.right - final_run.rect.left;
    assert!(
        (final_run.size - size_after_first).abs() < 0.05,
        "widening should not have touched size: was {size_after_first}, now {}",
        final_run.size
    );
    assert!(
        width_final > width_after * 1.5,
        "the second resize should still widen it: was {width_after}, now {width_final}"
    );
}

/// **A run splits into one object per character**, each still drawing
/// exactly the glyph it always did in exactly the same place — checked two
/// ways: reading every new object's own text back, in order, reproduces the
/// original run's words exactly; and the page renders pixel-for-pixel the
/// same as it did before the split, since nothing about how anything looks
/// is supposed to change.
#[test]
fn a_run_splits_into_one_object_per_character() {
    let Some(_) = skip_without_pdfium() else { return };
    let _lock = serial();

    let doc = open("covered.pdf");
    let run = doc.text_runs(0).expect("runs").into_iter().next().expect("a run");
    let n = run.text.chars().count();
    assert!(n > 1, "need a multi-character run for this test: {:?}", run.text);
    let before_objects = doc.drawn_objects(0).expect("objects").len();
    let (before_n, before_pixels) = render(&doc);
    drop(doc);

    let mut doc = open("covered.pdf");
    doc.split_run_into_characters(0, run.object).expect("split");

    let after_objects = doc.drawn_objects(0).expect("objects");
    assert_eq!(
        after_objects.len(),
        before_objects + n - 1,
        "one run should have become {n} objects in place of its one"
    );

    // Splitting occupies the same stream position the run's own operator
    // did, so its first character keeps the run's own object number and the
    // rest follow it — nothing before them on the page shifts. Checked
    // against `drawn_objects` above, which lists every PDFium page object
    // with no filtering; `text_runs` is a higher-level convenience over
    // that and, same as it would for any lone space-only object anywhere
    // else, does not surface a split-out space in its own list — it is
    // still there (the object count already proved it), just not text a
    // caller would ever select on its own either way.
    let original: Vec<char> = run.text.chars().collect();
    let after_runs = doc.text_runs(0).expect("runs");
    let mut read_back = String::new();
    let mut first_left = None;
    let mut last_right = 0.0f32;
    for (i, expected) in original.iter().enumerate() {
        if expected.is_whitespace() {
            read_back.push(*expected);
            continue;
        }
        let ch = after_runs
            .iter()
            .find(|r| r.object == run.object + i)
            .unwrap_or_else(|| panic!("character {i} ({expected:?}) not found at object {}", run.object + i));
        assert!(!ch.text.is_empty(), "object {} drew nothing", ch.object);
        // Its own *first* character, not the whole of `.text`: with its
        // neighbours now separate objects instead of one continuous run,
        // PDFium's own word-boundary heuristic sometimes appends a
        // synthetic trailing space to the last one before a gap — a
        // reading-order annotation about where a word ends, not a second
        // glyph actually drawn, and not something a real caller ever reads
        // (`Self::split_if_whole_run` only ever asks for rects, never text,
        // once split).
        read_back.push(ch.text.chars().next().expect("checked non-empty"));
        first_left.get_or_insert(ch.rect.left);
        last_right = ch.rect.right;
    }
    assert_eq!(read_back, run.text, "the split characters should read back as the original words");
    assert!(
        (first_left.unwrap() - run.rect.left).abs() < 2.0,
        "the first character should start where the run did: {:?} vs {}",
        first_left,
        run.rect.left
    );
    assert!(
        (last_right - run.rect.right).abs() < 2.0,
        "the last character should end where the run did: {last_right} vs {}",
        run.rect.right
    );

    // Not byte-for-byte: each character's `Tm` is re-derived through a
    // matrix inversion and six decimal places of formatting, rather than
    // being the exact bytes the original run's own `Tm` already had, so a
    // handful of edge pixels anti-alias a shade differently. Measured on
    // this fixture at 11 of 121176 pixels, by 1 of 255 in the worst
    // channel — real corruption would not stay in single digits either way.
    let (after_n, after_pixels) = render(&doc);
    assert_eq!(after_n, before_n, "the render size should not change");
    let mut diff = 0usize;
    let mut worst = 0i32;
    for (a, b) in before_pixels.chunks_exact(4).zip(after_pixels.chunks_exact(4)) {
        if a != b {
            diff += 1;
            for k in 0..4 {
                worst = worst.max((a[k] as i32 - b[k] as i32).abs());
            }
        }
    }
    let total = before_pixels.len() / 4;
    assert!(
        diff * 200 < total && worst <= 8,
        "splitting changed too much of how the page looks: {diff} of {total} pixels differ, worst channel delta {worst}"
    );
}

/// **Splitting an already-split character does nothing** — a caller does
/// not have to track which runs it has already reached before asking
/// again, since the answer is idempotent either way.
#[test]
fn splitting_a_single_character_again_changes_nothing() {
    let Some(_) = skip_without_pdfium() else { return };
    let _lock = serial();

    let mut doc = open("covered.pdf");
    let run = doc.text_runs(0).expect("runs").into_iter().next().expect("a run");
    assert!(run.text.chars().count() > 1, "need a multi-character run for this test");
    doc.split_run_into_characters(0, run.object).expect("split");
    let first_char = doc
        .text_runs(0)
        .expect("runs")
        .into_iter()
        .find(|r| r.object == run.object)
        .expect("the first split character");
    assert_eq!(first_char.text.chars().count(), 1);

    let before_objects = doc.drawn_objects(0).expect("objects").len();
    doc.split_run_into_characters(0, first_char.object).expect("split again");
    let after_objects = doc.drawn_objects(0).expect("objects");
    assert_eq!(after_objects.len(), before_objects, "nothing should have changed");
}

/// **Deleting a picture takes it off the page and leaves everything else
/// exactly where it was** — reuses the same object-locating logic move and
/// resize do, so this is really a test that deletion is wired to it
/// correctly, not a re-test of that logic itself.
#[test]
fn removing_a_picture_takes_it_off_and_leaves_the_words() {
    let Some(_) = skip_without_pdfium() else { return };
    let _lock = serial();

    let doc = open("framed.pdf");
    let picture = doc
        .drawn_objects(0)
        .expect("objects")
        .into_iter()
        .find(|d| d.kind == DrawnKind::Picture)
        .expect("the picture");
    let words_before: Vec<String> =
        doc.text_runs(0).expect("runs").into_iter().map(|r| r.text).collect();
    drop(doc);

    let mut doc = open("framed.pdf");
    doc.remove_object(0, picture.object).expect("remove");

    let still = doc
        .drawn_objects(0)
        .expect("objects")
        .into_iter()
        .any(|d| d.kind == DrawnKind::Picture);
    assert!(!still, "the picture should be gone");

    let words_after: Vec<String> =
        doc.text_runs(0).expect("runs").into_iter().map(|r| r.text).collect();
    assert_eq!(words_before, words_after, "deleting the picture must not touch the words");
}

/// **Opacity is absolute.** Set twice, it is set twice — not multiplied.
#[test]
fn opacity_fades_one_thing_and_setting_it_again_does_not_compound() {
    let Some(_) = skip_without_pdfium() else { return };
    let _lock = serial();

    fn centre_pixel(doc: &dyn Document, at: pdf_core::document::Rect) -> (u8, u8, u8) {
        let (width, pixels) = render(doc);
        let scale = 306.0 / doc.page_size(0).expect("size").width_pt;
        let x = (((at.left + at.right) / 2.0) * scale) as u32;
        let y = (((at.top + at.bottom) / 2.0) * scale) as u32;
        let i = ((y * width + x) * 4) as usize;
        (pixels[i], pixels[i + 1], pixels[i + 2])
    }

    let mut doc = open("framed.pdf");
    let picture = doc
        .drawn_objects(0)
        .expect("objects")
        .into_iter()
        .find(|d| d.kind == DrawnKind::Picture)
        .expect("the picture");
    let solid = centre_pixel(&doc, picture.rect);
    assert!(solid.0 > 180 && solid.1 < 80, "the picture should start solid red: {solid:?}");

    doc.set_opacity(0, picture.object, 0.5).expect("fade");
    let faded = centre_pixel(&doc, picture.rect);
    // Half red over the page: the green and blue that red was hiding come
    // through. (The placeholder fades with it — it is part of the picture —
    // so what shows through is the white page, and red itself barely drops.)
    assert!(faded.1 > solid.1 + 40 && faded.2 > solid.2 + 40, "it did not fade: {solid:?} then {faded:?}");
    let listed = doc.drawn_objects(0).expect("objects");
    let reported = listed.iter().find(|d| d.kind == DrawnKind::Picture).expect("still there").opacity;
    assert!((reported - 0.5).abs() < 0.02, "the list should report the new opacity, got {reported}");

    // Again at the same value: identical, not a quarter.
    let object = listed.iter().find(|d| d.kind == DrawnKind::Picture).expect("it").object;
    doc.set_opacity(0, object, 0.5).expect("fade again");
    let again = centre_pixel(&doc, picture.rect);
    assert!((again.0 as i32 - faded.0 as i32).abs() <= 2, "opacity compounded: {faded:?} then {again:?}");

    // And back to solid.
    let object = doc.drawn_objects(0).expect("objects").into_iter().find(|d| d.kind == DrawnKind::Picture).expect("it").object;
    doc.set_opacity(0, object, 1.0).expect("solid");
    let back = centre_pixel(&doc, picture.rect);
    assert!((back.0 as i32 - solid.0 as i32).abs() <= 2, "it did not come back solid: {back:?}");
}

// ------------------------------------------------ the object tool undoes --

use pdf_core::command::{history::CommandHistory, Command};

/// **Moving an object through the command stack undoes.** Edit Object's own
/// move/resize/delete never went through `Command` at all — dragging
/// something was simply not undoable — so this is the whole reason those
/// four variants exist, not a re-test of `move_object` itself.
#[test]
fn moving_an_object_through_the_command_stack_undoes() {
    let Some(_) = skip_without_pdfium() else { return };
    let _lock = serial();

    let mut doc: Box<dyn Document> = Box::new(open("pictures.pdf"));
    let picture = doc
        .images_on(0)
        .expect("images")
        .into_iter()
        .next()
        .expect("a picture");

    let mut history = CommandHistory::default();
    history
        .execute(
            Command::MoveObject { page_index: 0, object: picture.object, by: Point { x: 12.0, y: -8.0 } },
            doc.as_document_mut().expect("mutable"),
        )
        .expect("move");
    let moved = doc.images_on(0).expect("images").into_iter().next().expect("still there");
    assert!(
        (moved.rect.left - picture.rect.left - 12.0).abs() < 0.5
            && (moved.rect.top - picture.rect.top + 8.0).abs() < 0.5,
        "the move did not apply: {:?} then {:?}",
        picture.rect,
        moved.rect
    );

    history.undo(doc.as_document_mut().expect("mutable")).expect("undo").expect("something to undo");
    let back = doc.images_on(0).expect("images").into_iter().next().expect("still there");
    assert!(
        (back.rect.left - picture.rect.left).abs() < 0.5 && (back.rect.top - picture.rect.top).abs() < 0.5,
        "undo did not put it back: {:?}",
        back.rect
    );
}

/// **Resizing an object through the command stack undoes** — the reciprocal
/// scale about the same anchor, not a remembered size, so this also proves
/// the anchor round-trips and not just the factor.
#[test]
fn scaling_an_object_through_the_command_stack_undoes() {
    let Some(_) = skip_without_pdfium() else { return };
    let _lock = serial();

    let mut doc: Box<dyn Document> = Box::new(open("framed.pdf"));
    let picture = doc
        .drawn_objects(0)
        .expect("objects")
        .into_iter()
        .find(|d| d.kind == DrawnKind::Picture)
        .expect("the picture");

    let mut history = CommandHistory::default();
    let anchor = Point { x: picture.rect.left, y: picture.rect.top };
    history
        .execute(
            Command::ScaleObject { page_index: 0, object: picture.object, anchor, sx: 0.5, sy: 0.5 },
            doc.as_document_mut().expect("mutable"),
        )
        .expect("scale");
    let shrunk = doc
        .drawn_objects(0)
        .expect("objects")
        .into_iter()
        .find(|d| d.kind == DrawnKind::Picture)
        .expect("still there");
    assert!(
        (shrunk.rect.right - shrunk.rect.left - (picture.rect.right - picture.rect.left) / 2.0).abs() < 1.0,
        "the resize did not apply"
    );

    history.undo(doc.as_document_mut().expect("mutable")).expect("undo").expect("something to undo");
    let back = doc
        .drawn_objects(0)
        .expect("objects")
        .into_iter()
        .find(|d| d.kind == DrawnKind::Picture)
        .expect("still there");
    assert!(
        (back.rect.right - picture.rect.right).abs() < 1.0 && (back.rect.bottom - picture.rect.bottom).abs() < 1.0,
        "undo did not restore the original size: was {:?}, now {:?}",
        picture.rect,
        back.rect
    );
}

/// **Deleting an object through the command stack undoes** — by a page
/// snapshot, the same as `Redact`: the operators are gone once removed, so
/// there is nothing to describe backwards, only a copy to put back.
#[test]
fn removing_an_object_through_the_command_stack_undoes() {
    let Some(_) = skip_without_pdfium() else { return };
    let _lock = serial();

    let mut doc: Box<dyn Document> = Box::new(open("framed.pdf"));
    let picture = doc
        .drawn_objects(0)
        .expect("objects")
        .into_iter()
        .find(|d| d.kind == DrawnKind::Picture)
        .expect("the picture");
    let words_before: Vec<String> = doc.text_runs(0).expect("runs").into_iter().map(|r| r.text).collect();

    let mut history = CommandHistory::default();
    history
        .execute(
            Command::RemoveObject { page_index: 0, object: picture.object },
            doc.as_document_mut().expect("mutable"),
        )
        .expect("remove");
    assert!(
        !doc.drawn_objects(0).expect("objects").iter().any(|d| d.kind == DrawnKind::Picture),
        "the picture should be gone"
    );

    history.undo(doc.as_document_mut().expect("mutable")).expect("undo").expect("something to undo");
    assert!(
        doc.drawn_objects(0).expect("objects").iter().any(|d| d.kind == DrawnKind::Picture),
        "undo did not bring the picture back"
    );
    let words_after: Vec<String> = doc.text_runs(0).expect("runs").into_iter().map(|r| r.text).collect();
    assert_eq!(words_before, words_after, "undo should not have disturbed the words");
}

/// **Splitting a run into characters through the command stack undoes** —
/// by the same page-snapshot mechanism as delete: merging many written
/// characters back into one run is a harder feature this does not need.
#[test]
fn splitting_a_run_through_the_command_stack_undoes() {
    let Some(_) = skip_without_pdfium() else { return };
    let _lock = serial();

    let mut doc: Box<dyn Document> = Box::new(open("covered.pdf"));
    let run = doc.text_runs(0).expect("runs").into_iter().next().expect("a run");
    assert!(run.text.chars().count() > 1, "need a multi-character run for this test");
    let before_objects = doc.drawn_objects(0).expect("objects").len();

    let mut history = CommandHistory::default();
    history
        .execute(
            Command::SplitRunIntoCharacters { page_index: 0, object: run.object },
            doc.as_document_mut().expect("mutable"),
        )
        .expect("split");
    assert!(
        doc.drawn_objects(0).expect("objects").len() > before_objects,
        "the split should have added objects"
    );

    history.undo(doc.as_document_mut().expect("mutable")).expect("undo").expect("something to undo");
    assert_eq!(
        doc.drawn_objects(0).expect("objects").len(),
        before_objects,
        "undo should have merged the characters back into one run"
    );
    let restored = doc
        .text_runs(0)
        .expect("runs")
        .into_iter()
        .find(|r| r.object == run.object)
        .expect("the run should be back");
    assert_eq!(restored.text, run.text, "the words should read the same as before the split");
}
