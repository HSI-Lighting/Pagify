//! **Copying objects and drawings, and putting them back.**
//!
//! Reported from use, with two screenshots and a session log (0.1.67):
//!
//! 1. Selecting an object with Edit Object and pressing Ctrl+C said "1 page
//!    copied" — the page was taken, not the object. A page picked in the
//!    thumbnail rail stays selected until Escape, and `copy` (and `Delete`) give
//!    the rail's pages priority over anything picked on the canvas afterwards.
//! 2. A drawing of 93 pieces was copied and pasted: `undo` took it back one
//!    piece at a time (93 steps), and the paste was washed out — every piece had
//!    been pasted as a picture of its own box, each carrying the pixels of its
//!    neighbours, laid down in selection order rather than the page's.
//!
//! The VIVO file is not in the repository; the tests that need it skip.

use super::*;
use crate::ui_tests::{click, harness};

const VIVO: &str = r"C:\Users\hsili\Desktop\VIVO mini.pdf";

fn vivo() -> Option<PagifyApp> {
    if !std::path::Path::new(VIVO).exists() {
        eprintln!("skipped: VIVO mini.pdf is not at {VIVO}");
        return None;
    }
    let mut app = PagifyApp::new(Some(VIVO));
    app.submit("editobject");
    Some(app)
}

/// The centre of the first `w` × `h` point window of page 0 with nothing drawn in it.
fn bare_paper(app: &PagifyApp, w: f32, h: f32) -> AppPoint {
    let session = &app.tab().doc.as_ref().expect("open").session;
    let (pw, ph) = app.tab().doc.as_ref().and_then(|d| d.strip.size_of(0)).expect("size");
    let page = session
        .render_page_region(0, pdf_core::document::Rect { left: 0.0, top: 0.0, right: pw, bottom: ph }, 1.0)
        .expect("page");
    let blank = |x: usize, y: usize| page.pixels[(y * page.width as usize + x) * 4..][..3].iter().all(|&c| c >= 250);
    let (ww, wh) = (w.ceil() as usize + 4, h.ceil() as usize + 4);
    for y in (0..page.height as usize - wh).step_by(6) {
        for x in (0..page.width as usize - ww).step_by(6) {
            let clear = (0..wh).step_by(3).all(|dy| (0..ww).step_by(3).all(|dx| blank(x + dx, y + dy)));
            if clear {
                return AppPoint { x: (x + ww / 2) as f64, y: (y + wh / 2) as f64 };
            }
        }
    }
    panic!("no bare paper {w} x {h} on the page");
}

fn placed(app: &PagifyApp) -> usize {
    app.tab().doc.as_ref().expect("open").session.placed_image_marks(0).expect("marks").len()
}

/// The user's own sequence: a thumbnail was clicked at some point (the rail
/// keeps that page selected), Edit Object was picked, a picture was clicked —
/// and then Ctrl+C. The picture is what is meant.
#[test]
fn a_page_picked_in_the_rail_does_not_take_copy_from_an_object_picked_after_it() {
    let mut h = harness("pictures.pdf");
    let view = h.state().tab().view_state.last_view.expect("the page was never drawn");
    h.state_mut().submit("editobject");
    h.run_steps(1);
    h.state_mut().tab_mut().organize.organize_selected = vec![0];

    let image = h.state().tab().doc.as_ref().unwrap().session.images_on(0).unwrap().remove(0);
    let middle = view.to_screen(AppPoint {
        x: ((image.rect.left + image.rect.right) / 2.0) as f64,
        y: ((image.rect.top + image.rect.bottom) / 2.0) as f64,
    });
    click(&mut h, middle);
    assert!(h.state().tab().selection.selected.is_some(), "the click should have picked the picture");

    h.input_mut().events.push(egui::Event::Copy);
    h.run_steps(2);

    let app = h.state();
    assert!(
        app.clipboard_state.object_clipboard.is_some(),
        "the picture was not copied: {}",
        app.cmd.history().iter().map(|e| e.text.as_str()).collect::<Vec<_>>().join(" / ")
    );
    assert!(app.clipboard_state.page_clipboard.is_none(), "the page was copied instead of the picture");
}

/// A page picked in the rail is still what Ctrl+C copies when nothing was
/// picked on the canvas since — the fix must not take page copy away.
#[test]
fn a_page_picked_in_the_rail_is_still_copied_when_nothing_else_is_picked() {
    let mut h = harness("pictures.pdf");
    h.state_mut().tab_mut().organize.organize_selected = vec![0];
    h.input_mut().events.push(egui::Event::Copy);
    h.run_steps(2);
    assert!(h.state().clipboard_state.page_clipboard.is_some(), "the picked page was not copied");
}

/// Pieces of a drawing pasted together are taken back together. Since 0.1.70
/// two pictures copied together go down as the page's own pictures, not as
/// placed (annotation) pictures.
#[test]
fn a_pasted_group_is_taken_back_by_one_undo() {
    let mut app = PagifyApp::new(Some(&crate::ui_tests::fixture("pictures.pdf")));
    app.submit("editobject");
    let pictures = app.tab().doc.as_ref().expect("open").session.images_on(0).expect("images");
    assert_eq!(pictures.len(), 2);
    app.tab_mut().selection.group = pictures
        .iter()
        .map(|p| Selected { page: 0, object: p.object, rect: p.rect, what: "the picture" })
        .collect();
    assert!(app.copy_object_selection());
    let before = drawings(&app);

    app.paste_object_selection();
    app.place_paste_ghost(0, AppPoint { x: 300.0, y: 500.0 });
    assert_eq!(drawings(&app), before + 2, "both pieces should have been pasted: {}", said(&app));
    assert_eq!(placed(&app), 0, "the pieces were pasted as placed pictures");

    app.submit("undo");
    assert_eq!(drawings(&app), before, "one undo should take the whole paste back");
    app.submit("redo");
    assert_eq!(drawings(&app), before + 2, "one redo should put the whole paste back");
}

fn piece(what: &'static str, left: f32, top: f32, right: f32, bottom: f32) -> Selected {
    Selected { page: 0, object: 0, rect: pdf_core::document::Rect { left, top, right, bottom }, what }
}

/// Pieces that overlap, or all but touch, are one drawing; a piece far from the
/// rest is another; words are never folded in.
#[test]
fn overlapping_pieces_are_merged_into_one_box_and_the_rest_left_alone() {
    let merged = merge_overlapping(vec![
        piece("the shape", 0.0, 0.0, 10.0, 10.0),
        piece("the shape", 100.0, 100.0, 110.0, 110.0),
        piece("the shape", 8.0, 8.0, 20.0, 20.0),
        piece("the words", 5.0, 5.0, 15.0, 15.0),
    ]);
    assert_eq!(merged.len(), 3, "{merged:?}");
    assert_eq!(merged.iter().filter(|s| s.what == "the words").count(), 1, "words must pass through");
    let near = merged.iter().find(|s| s.rect.left == 0.0).expect("the merged drawing");
    assert_eq!((near.rect.right, near.rect.bottom), (20.0, 20.0));
    assert!(merged.iter().any(|s| s.rect.left == 100.0), "the far piece must be left alone");
}

/// A piece that bridges two separate drawings joins them, whichever order the
/// pieces arrive in — no two boxes that are left may overlap.
#[test]
fn a_bridging_piece_joins_the_drawings_it_touches() {
    for order in [[0, 1, 2], [2, 0, 1], [1, 2, 0]] {
        let pieces = [
            piece("the shape", 0.0, 0.0, 10.0, 10.0),
            piece("the shape", 20.0, 0.0, 30.0, 10.0),
            piece("the shape", 9.0, 2.0, 21.0, 8.0),
        ];
        let merged = merge_overlapping(order.iter().map(|&i| pieces[i].clone()).collect());
        assert_eq!(merged.len(), 1, "order {order:?}: {merged:?}");
        assert_eq!((merged[0].rect.left, merged[0].rect.right), (0.0, 30.0));
    }
}

/// A picture has paper of its own that must not be made see-through; shapes
/// alone have none, so a box of only shapes keeps the paper-clearing.
#[test]
fn a_box_with_a_picture_in_it_is_a_picture_and_one_of_shapes_is_a_shape() {
    let mixed = merge_overlapping(vec![piece("the shape", 0.0, 0.0, 10.0, 10.0), piece("the picture", 5.0, 5.0, 15.0, 15.0)]);
    assert_eq!(mixed.len(), 1);
    assert_eq!(mixed[0].what, "the picture");
    let shapes = merge_overlapping(vec![piece("the shape", 0.0, 0.0, 10.0, 10.0), piece("the shape", 5.0, 5.0, 15.0, 15.0)]);
    assert_eq!(shapes[0].what, "the shape");
}


/// **Reported from use: "after undoing, the object is stale on the page — I have
/// to zoom in and out to refresh it", page and thumbnail alike.** The pasted group
/// held a word copied at size 0 (PDFium's size for text turned on its side), so
/// its undo could not find it again: the batch's undo failed partway, after the
/// picture had already been taken off — and on an error neither the engine nor
/// the app dropped what it had cached of the page, so the picture stayed on
/// screen until a zoom asked for a render nothing had cached.
#[test]
fn a_paste_undone_is_gone_from_the_screen_even_when_part_of_its_undo_fails() {
    let mut app = PagifyApp::new(Some(&crate::ui_tests::fixture("pictures.pdf")));
    app.submit("editobject");
    let session = app.tab().doc.as_ref().expect("open").session.clone();
    // Rendered first, so the engine has the page cached as it was.
    let before = session.render_page(0, 1.0).expect("render");

    let picture = ObjectClipboard::Image {
        rgba: [200u8, 0, 0, 255].repeat(40 * 40),
        width: 40,
        height: 40,
        rect: pdf_core::document::Rect { left: 0.0, top: 0.0, right: 80.0, bottom: 80.0 },
    };
    let sideways = ObjectClipboard::Text {
        lines: vec!["108mm".into()],
        size: 0.0,
        color: pdf_core::document::Color { r: 0, g: 0, b: 0, a: 255 },
        face: None,
        track: 1.0,
    };
    app.clipboard_state.object_clipboard = Some(ObjectClipboard::Group(vec![(sideways, -60.0, 0.0), (picture, 0.0, 0.0)]));
    app.paste_object_selection();
    app.place_paste_ghost(0, AppPoint { x: 200.0, y: 200.0 });
    assert_eq!(placed(&app), 1, "the picture should be down");
    assert_ne!(session.render_page(0, 1.0).expect("render").pixels, before.pixels, "the paste should show");
    let epoch = app.tab().doc.as_ref().expect("open").render_epoch;

    app.submit("undo");

    assert_eq!(placed(&app), 0, "the picture should be taken back");
    assert!(app.tab().doc.as_ref().expect("open").render_epoch > epoch, "the app still holds the page as it was before the undo");
    assert!(
        session.render_page(0, 1.0).expect("render").pixels == before.pixels,
        "the engine still draws the undone picture: a stale render"
    );
}

/// The word in the report was VIVO's "108mm", set on its side: PDFium gives it
/// size 0, and pasted as words at size 0 it was invisible and could not be found
/// to undo. A run with no size is copied as what it looks like instead.
#[test]
fn a_word_with_no_size_is_copied_as_a_picture_of_itself() {
    let Some(mut app) = vivo() else { return };
    let run = app.tab().doc.as_ref().unwrap().session.text_runs(0).unwrap().into_iter().find(|r| !(r.size > 0.0)).expect("VIVO's sideways label");
    app.tab_mut().selection.selected = Some(Selected { page: 0, object: run.object, rect: run.rect, what: "the words" });
    assert!(app.copy_object_selection());
    assert!(
        matches!(app.clipboard_state.object_clipboard, Some(ObjectClipboard::Image { .. })),
        "copied as {:?}",
        app.clipboard_state.object_clipboard.as_ref().map(describe_clipboard)
    );
}

// ---- copying drawings as lines (Pagify-Vector-Copy-Plan.md, 0.1.70) ----
//
// Reported: the VIVO lens, copied and pasted, came back softer than the
// original — a copy was a picture of the page. Pictures and shapes are now
// lifted out of the page's own drawing and pasted as one group of it.

/// A hand-built one-page file (see `g4_edit_error_tests::page_with`), opened
/// with Edit Object armed.
fn opened(name: &str, content: &[u8], resources: &str, extra: &[Vec<u8>]) -> PagifyApp {
    let dir = std::env::temp_dir().join(format!("pagify-copy-lines-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("a scratch folder");
    let path = dir.join(name);
    std::fs::write(&path, crate::g4_edit_error_tests::page_with(content, resources, extra)).expect("a scratch pdf");
    let mut app = PagifyApp::new(Some(path.to_str().expect("a utf-8 path")));
    app.submit("editobject");
    app
}

/// How many pictures and shapes the page itself draws (not words, not what is
/// inside a group) — each pasted line is one more.
fn drawings(app: &PagifyApp) -> usize {
    let drawn = app.tab().doc.as_ref().expect("open").session.drawn_objects(0).expect("drawn");
    drawn.iter().filter(|d| d.depth == 0 && d.kind != pdf_core::document::DrawnKind::Words).count()
}

fn said(app: &PagifyApp) -> String {
    app.cmd.history().iter().map(|e| e.text.as_str()).collect::<Vec<_>>().join("\n")
}


/// The yardstick from the report: the mean step in luminance from each pixel to
/// its right and lower neighbours.
fn edge_energy(raster: &pagify_shell::session::PageRaster) -> f64 {
    let (w, h) = (raster.width as usize, raster.height as usize);
    let lum = |x: usize, y: usize| {
        let p = &raster.pixels[(y * w + x) * 4..];
        0.299 * p[0] as f64 + 0.587 * p[1] as f64 + 0.114 * p[2] as f64
    };
    let mut sum = 0.0;
    for y in 0..h - 1 {
        for x in 0..w - 1 {
            sum += (lum(x + 1, y) - lum(x, y)).abs() + (lum(x, y + 1) - lum(x, y)).abs();
        }
    }
    sum / ((w - 1) * (h - 1)) as f64
}

#[test]
fn the_lens_is_copied_and_pasted_as_lines() {
    let Some(mut app) = vivo() else { return };
    app.select_group_in(0, AppPoint { x: 415.0, y: 106.0 }, AppPoint { x: 490.0, y: 180.0 }, false);
    assert!(app.tab().selection.group.len() > 50);
    assert!(app.copy_object_selection());
    let Some(ObjectClipboard::Group(items)) = &app.clipboard_state.object_clipboard else { panic!("not a group") };
    assert!(items.iter().any(|(c, ..)| matches!(c, ObjectClipboard::Lifted { .. })), "the drawing was not lifted");

    // Onto bare paper: a window the size of the drawing with nothing on it.
    let bounds = app.group_bounds(0).expect("bounds");
    let (w, h) = (bounds.right - bounds.left, bounds.bottom - bounds.top);
    let at = bare_paper(&app, w, h);
    let landed = pdf_core::document::Rect {
        left: at.x as f32 - w / 2.0,
        right: at.x as f32 + w / 2.0,
        top: at.y as f32 - h / 2.0,
        bottom: at.y as f32 + h / 2.0,
    };
    let ink = |app: &PagifyApp| {
        let raster = app.tab().doc.as_ref().expect("open").session.render_page_region(0, landed, 2.0).expect("region");
        raster.pixels.chunks_exact(4).filter(|p| p[..3].iter().any(|&c| c < 200)).count()
    };
    assert_eq!(ink(&app), 0, "the spot was meant to be bare");

    // The file holds a picture of its own already.
    let (before_pictures, before_drawn) = (placed(&app), drawings(&app));
    app.paste_object_selection();
    app.place_paste_ghost(0, at);
    assert_eq!(placed(&app), before_pictures, "a picture was added: {}", said(&app));
    assert!(drawings(&app) > before_drawn + 50, "the lines did not go down as lines of the page: {}", said(&app));
    assert!(ink(&app) > 500, "the drawing is not on the page where it was put down");

    app.submit("undo");
    assert_eq!(drawings(&app), before_drawn, "one undo should take the drawing back");
    assert_eq!(ink(&app), 0, "the drawing is still on the page after undo");
}

#[test]
fn the_pasted_lens_is_as_sharp_as_the_original_at_every_zoom() {
    let Some(mut app) = vivo() else { return };
    app.select_group_in(0, AppPoint { x: 415.0, y: 106.0 }, AppPoint { x: 490.0, y: 180.0 }, false);
    let members: Vec<Selected> = app.tab().selection.group.clone();
    // The drawing alone, so the box measured holds nothing else of the page's.
    let drawn: Vec<Selected> = members.into_iter().filter(|s| s.what != "the words").collect();
    app.tab_mut().selection.group = drawn;
    assert!(app.copy_object_selection());
    let Some(ObjectClipboard::Group(items)) = &app.clipboard_state.object_clipboard else { panic!("not a group") };
    let rect = items.iter().find_map(|(c, ..)| match c { ObjectClipboard::Lifted { rect, .. } => Some(*rect), _ => None }).expect("lifted");
    let (w, h) = (rect.right - rect.left, rect.bottom - rect.top);

    // Onto bare paper, moved by a whole number of 2 pt steps, so every scale
    // below lands on the same pixel grid as the original.
    let bare = bare_paper(&app, w, h);
    let step = |d: f64| ((d / 2.0).round() * 2.0) as f32;
    let (dx, dy) = (step(bare.x - ((rect.left + rect.right) / 2.0) as f64), step(bare.y - ((rect.top + rect.bottom) / 2.0) as f64));
    let at = AppPoint { x: ((rect.left + rect.right) / 2.0 + dx) as f64, y: ((rect.top + rect.bottom) / 2.0 + dy) as f64 };
    app.paste_object_selection();
    app.place_paste_ghost(0, at);
    let moved = pdf_core::document::Rect { left: rect.left + dx, right: rect.right + dx, top: rect.top + dy, bottom: rect.bottom + dy };

    let session = &app.tab().doc.as_ref().expect("open").session;
    for scale in [1.5f32, 4.0, 8.0] {
        let original = edge_energy(&session.render_page_region(0, rect, scale).expect("original"));
        let pasted = edge_energy(&session.render_page_region(0, moved, scale).expect("pasted"));
        let ratio = pasted / original;
        eprintln!("scale {scale}: original {original:.3}, pasted {pasted:.3}, ratio {ratio:.3}");
        assert!(ratio >= 0.97, "at scale {scale} the paste keeps {ratio:.3} of the original's crispness");
    }
}

#[test]
fn a_copy_that_cannot_be_lifted_still_pastes_as_a_picture_and_says_so() {
    // PDFium lists the inline image as a picture too, so the page's pictures
    // cannot be matched to its `Do`s and the lift refuses.
    let image = b"<< /Type /XObject /Subtype /Image /Width 1 /Height 1 /BitsPerComponent 8 /ColorSpace /DeviceGray /Length 1 >>\nstream\nx\nendstream".to_vec();
    let mut app = opened(
        "inline-image.pdf",
        b"q 40 0 0 40 72 600 cm /Im0 Do Q q 40 0 0 40 200 600 cm BI /W 1 /H 1 /BPC 8 /CS /G ID x EI Q",
        "/XObject << /Im0 6 0 R >>",
        &[image],
    );
    let picture = app.tab().doc.as_ref().expect("open").session.images_on(0).expect("images").remove(0);
    app.tab_mut().selection.selected = Some(Selected { page: 0, object: picture.object, rect: picture.rect, what: "the picture" });
    assert!(app.copy_object_selection());
    assert!(matches!(app.clipboard_state.object_clipboard, Some(ObjectClipboard::Image { .. })), "{}", said(&app));
    assert!(said(&app).contains("picture copied"), "{}", said(&app));
}

#[test]
fn a_paste_the_page_refuses_falls_back_to_a_picture_in_the_same_click() {
    let mut app = opened("refusing.pdf", b"1 0 0 rg 72 600 50 50 re f", "", &[]);
    // A clip naming a picture its resources do not hold: the paste must refuse.
    let broken = crate::g4_edit_error_tests::page_with(b"q 40 0 0 40 72 600 cm /Im9 Do Q", "", &[]);
    let rect = pdf_core::document::Rect { left: 72.0, top: 142.0, right: 122.0, bottom: 192.0 };
    let picture = ObjectClipboard::Image { rgba: [255u8, 0, 0, 255].repeat(4), width: 2, height: 2, rect };
    app.clipboard_state.object_clipboard =
        Some(ObjectClipboard::Lifted { pdf: std::sync::Arc::new(broken), rect, picture: Box::new(picture) });
    let before = placed(&app);

    app.paste_object_selection();
    assert_eq!(placed(&app), before + 1, "no picture went down: {}", said(&app));
    assert!(said(&app).contains("pasted as a picture"), "{}", said(&app));

    app.submit("undo");
    assert_eq!(placed(&app), before, "one undo should take the picture back");
}

#[test]
fn words_and_lines_copied_together_paste_in_one_undo() {
    let mut app = opened(
        "label-and-squares.pdf",
        b"BT /F1 12 Tf 72 700 Td (Label) Tj ET 1 0 0 rg 72 600 50 50 re f 0 0 1 rg 140 600 50 50 re f",
        "",
        &[],
    );
    app.select_group_in(0, AppPoint { x: 60.0, y: 70.0 }, AppPoint { x: 200.0, y: 200.0 }, false);
    assert_eq!(app.tab().selection.group.len(), 3, "the marquee should catch the label and both squares");
    assert!(app.copy_object_selection());
    let session = app.tab().doc.as_ref().expect("open").session.clone();
    let runs_before = session.text_runs(0).expect("runs").len();
    let drawn_before = drawings(&app);

    app.paste_object_selection();
    app.place_paste_ghost(0, AppPoint { x: 300.0, y: 500.0 });
    // New words are written a letter to a text object (architecture §13).
    assert!(session.text_runs(0).expect("runs").len() > runs_before, "the label was not pasted as words: {}", said(&app));
    assert_eq!(drawings(&app), drawn_before + 2, "the squares were not pasted as lines: {}", said(&app));

    app.submit("undo");
    assert_eq!(session.text_runs(0).expect("runs").len(), runs_before, "one undo should take the words back");
    assert_eq!(drawings(&app), drawn_before, "one undo should take the squares back");
}

/// **Pasted lines are edited like the originals.** Reported from use on
/// 0.1.70: the pasted lens could be selected only as one group, which could
/// not be deleted or resized. Each pasted line is now picked on its own.
#[test]
fn a_pasted_line_is_picked_and_deleted_on_its_own() {
    let mut app = opened("two-squares.pdf", b"1 0 0 rg 72 600 50 50 re f 0 0 1 rg 140 600 50 50 re f", "", &[]);
    app.select_group_in(0, AppPoint { x: 60.0, y: 130.0 }, AppPoint { x: 200.0, y: 200.0 }, false);
    assert!(app.copy_object_selection());
    // The pair's centre (131, 167) goes to (300, 500): the red square lands
    // at 241..291 x 475..525, the blue one at 309..359.
    app.paste_object_selection();
    app.place_paste_ghost(0, AppPoint { x: 300.0, y: 500.0 });
    assert_eq!(drawings(&app), 4, "{}", said(&app));

    app.select_group_in(0, AppPoint { x: 235.0, y: 470.0 }, AppPoint { x: 297.0, y: 530.0 }, false);
    let picked: Vec<&'static str> = app.tab().selection.group.iter().map(|s| s.what)
        .chain(app.tab().selection.selected.iter().map(|s| s.what))
        .collect();
    assert_eq!(picked, ["the shape"], "the pasted red square should be picked alone");
    app.delete_selection();
    assert_eq!(drawings(&app), 3, "the pasted red square was not deleted: {}", said(&app));
    let left = app.tab().doc.as_ref().expect("open").session.drawn_objects(0).expect("drawn");
    assert!(left.iter().any(|d| (d.rect.left - 309.0).abs() < 1.0), "the pasted blue square went too");
}

#[test]
fn the_log_says_what_was_lifted_but_never_the_words() {
    let mut app = opened(
        "logged-lift.pdf",
        b"BT /F1 12 Tf 72 700 Td (Luminaire bracket) Tj ET 1 0 0 rg 72 600 50 50 re f",
        "",
        &[],
    );
    let dir = std::env::temp_dir().join(format!("pagify-test-lift-log-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    app.recording_state.session_log = pagify_shell::session_log::SessionLog::start_in(dir.clone());
    let path = app.recording_state.session_log.path().expect("a writable folder").to_path_buf();

    app.select_group_in(0, AppPoint { x: 60.0, y: 70.0 }, AppPoint { x: 200.0, y: 200.0 }, false);
    assert!(app.copy_object_selection());
    app.paste_object_selection();
    app.place_paste_ghost(0, AppPoint { x: 300.0, y: 500.0 });

    let lifts: Vec<String> = std::fs::read_to_string(&path)
        .expect("the log")
        .lines()
        .map(|l| serde_json::from_str::<serde_json::Value>(l).expect("json"))
        .filter(|l| l["kind"] == "lift")
        .map(|l| l["text"].as_str().unwrap_or_default().to_string())
        .collect();
    let _ = std::fs::remove_dir_all(&dir);
    assert!(lifts.iter().any(|l| l.contains("objects=") && l.contains("bytes=")), "{lifts:?}");
    for line in &lifts {
        for word in ["Luminaire", "bracket"] {
            assert!(!line.contains(word), "the log holds the page's word {word:?}: {line}");
        }
    }
}

// ---- words in a font with no cmap (0.1.73) ----
//
// Reported: VIVO's "Reflector" paragraph, copied with Edit Object and pasted,
// came out as a row of empty boxes. Its body font is an `/Identity-H` subset
// with no `cmap` table, so every letter shaped to glyph 0.

/// The non-space glyphs `face` shapes `text` to that are glyph 0, the box.
fn boxes(face: &str, text: &str) -> Vec<char> {
    let shaped = pdf_core::text::shape(face, text).expect("shapes");
    shaped
        .glyphs
        .iter()
        .filter(|g| g.id == 0)
        .filter_map(|g| text[g.cluster as usize..].chars().next())
        .filter(|c| !c.is_whitespace())
        .collect()
}

#[test]
fn words_copied_from_a_font_with_no_cmap_paste_as_letters_not_boxes() {
    let Some(mut app) = vivo() else { return };
    let run = app.tab().doc.as_ref().unwrap().session.text_runs(0).unwrap().into_iter().find(|r| r.text.contains("Anodized")).expect("VIVO's Reflector line");
    let face = app.registered_face_of_object(0, run.object).expect("the run's own font");
    assert_eq!(boxes(&face, "Anodized Aluminium coating"), Vec::<char>::new(), "the run's font shapes these letters to boxes");

    app.tab_mut().selection.selected = Some(Selected { page: 0, object: run.object, rect: run.rect, what: "the words" });
    assert!(app.copy_object_selection());
    let Some(ObjectClipboard::Text { face: Some(copied), .. }) = app.clipboard_state.object_clipboard.clone() else {
        panic!("not copied as words in a face of their own")
    };
    assert_eq!(boxes(&copied, run.text.trim()), Vec::<char>::new());

    let (w, h) = (run.rect.right - run.rect.left, run.rect.bottom - run.rect.top);
    // Words are put down from where the paste is clicked, not centred on it.
    let centre = bare_paper(&app, w + 20.0, h + 20.0);
    let at = AppPoint { x: centre.x - w as f64 / 2.0, y: centre.y - h as f64 / 2.0 };
    let before: std::collections::HashSet<usize> = app.tab().doc.as_ref().unwrap().session.text_runs(0).unwrap().iter().map(|r| r.object).collect();
    app.paste_object_selection();
    app.place_paste_ghost(0, at);
    let pasted: Vec<_> = app.tab().doc.as_ref().unwrap().session.text_runs(0).unwrap().into_iter().filter(|r| !before.contains(&r.object)).collect();
    assert!(!pasted.is_empty(), "nothing was pasted: {}", said(&app));
    let left = pasted.iter().map(|r| r.rect.left).fold(f32::MAX, f32::min);
    let right = pasted.iter().map(|r| r.rect.right).fold(f32::MIN, f32::max);
    // As wide as the original: the copy's tracking is measured from its letters,
    // which were boxes of another width before.
    let area = |r: &pdf_core::document::Rect| (r.left, r.top, r.right, r.bottom);
    let characters = app.tab().doc.as_ref().unwrap().session.characters(0).expect("characters");
    let original = characters.ink_right_in(area(&run.rect)).unwrap() - characters.ink_left_in(area(&run.rect)).unwrap();
    let copy = pdf_core::document::Rect { left, right, top: at.y as f32 - h, bottom: at.y as f32 + h };
    let copy = characters.ink_right_in(area(&copy)).unwrap() - characters.ink_left_in(area(&copy)).unwrap();
    eprintln!("original ink {original:.1} pt wide, pasted {copy:.1} pt");
    assert!((copy - original).abs() < original * 0.03, "pasted {copy:.1} pt wide, the original {original:.1}");

    // Look at it: a box is ink too, so no pixel count can tell letters from boxes.
    let region = pdf_core::document::Rect { left: left - 4.0, right: right + 4.0, top: at.y as f32 - h, bottom: at.y as f32 + h };
    let raster = app.tab().doc.as_ref().unwrap().session.render_page_region(0, region, 6.0).expect("region");
    let out = std::env::temp_dir().join("pagify-pasted-no-cmap.png");
    image::save_buffer(&out, &raster.pixels, raster.width, raster.height, image::ColorType::Rgba8).expect("png");
    eprintln!("pasted words drawn to {}", out.display());
}

/// Where a font cannot draw the words and the PDF cannot mend it (no
/// `/ToUnicode`), the paste is written in a face that can, and says so.
#[test]
fn words_a_font_cannot_draw_are_written_in_one_that_can_and_the_paste_says_which() {
    // Montserrat with its cmap hidden: the tag renamed, so no reader finds it.
    let mut font = BUNDLED_OUTLINED_FONTS[0].to_vec();
    let count = u16::from_be_bytes([font[4], font[5]]) as usize;
    let at = (0..count).map(|i| 12 + 16 * i).find(|&at| &font[at..at + 4] == b"cmap").expect("a cmap");
    font[at..at + 4].copy_from_slice(b"cmaq");
    let name = "test-montserrat-without-cmap";
    if !pdf_core::text::is_registered(name) {
        pdf_core::text::register(name, font).expect("registered");
    }
    assert!(!boxes(name, "Anodized").is_empty(), "the test font must shape to boxes");

    let mut app = opened("no-cmap-guard.pdf", b"", "", &[]);
    let black = pdf_core::document::Color { r: 0, g: 0, b: 0, a: 255 };
    let content = ObjectClipboard::Text { lines: vec!["Anodized".into()], size: 12.0, color: black, face: Some(name.into()), track: 1.0 };
    let commands = app.paste_commands(0, AppPoint { x: 100.0, y: 100.0 }, content.clone(), &mut Vec::new()).expect("commands");
    let [pdf_core::command::Command::AddAnnotation { annotation: pdf_core::document::Annotation::Text { glyphs, font, .. }, .. }] =
        commands.as_slice()
    else {
        panic!("not one line of words")
    };
    assert_ne!(font, name, "written in the font that cannot draw them");
    // Helvetica is written by character (id 0 means "by character" there).
    assert!(font == "Helvetica" || glyphs.iter().all(|g| g.id != 0 || g.ch.trim().is_empty()), "a box was written in {font}");

    assert!(app.place_clipboard_content(0, AppPoint { x: 100.0, y: 200.0 }, content));
    assert!(said(&app).contains(&format!("Words written in {font}")), "the paste did not say which face: {}", said(&app));
}

// ---- the command the log keeps (0.1.73) ----
//
// Reported from the owner's session log: `editobject` pressed while Edit Text
// waited for a click was stored as "that pick was cancelled.", the message the
// cancelling said first.

#[test]
fn a_command_started_over_a_pick_is_logged_as_itself() {
    let mut app = opened("command-over-pick.pdf", b"", "", &[]);
    let dir = std::env::temp_dir().join(format!("pagify-test-command-log-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    app.recording_state.session_log = pagify_shell::session_log::SessionLog::start_in(dir.clone());
    let path = app.recording_state.session_log.path().expect("a writable folder").to_path_buf();

    // A pick is waiting for its click.
    app.tab_mut().tool = Some(ArmedTool { kind: Tool::PickText, page: 0, objects: Vec::new(), points: Vec::new() });
    app.cmd.input_mut().clear();
    app.cmd.input_mut().push_str("editobject");
    let dispatch = app.cmd.submit(pagify_shell::command::Submit::Button).expect("a command");
    app.run_submitted(dispatch);

    let logged: Vec<String> = std::fs::read_to_string(&path)
        .expect("the log")
        .lines()
        .map(|l| serde_json::from_str::<serde_json::Value>(l).expect("json"))
        .filter(|l| l["kind"] == "command")
        .map(|l| l["text"].as_str().unwrap_or_default().to_string())
        .collect();
    let _ = std::fs::remove_dir_all(&dir);
    assert_eq!(logged, ["editobject"], "the log's command line");
    assert!(said(&app).contains("that pick was cancelled."), "the pick was still cancelled and said so");
}

// ---- a line-end hyphen is copied as a hyphen (0.1.74) ----
//
// Reported with two screenshots: VIVO's "re-quired" / "pre-pared" came out as
// "preÿpared". PDFium's text layer hands a line-end hyphen back as the control
// character U+0002, which no font has a glyph for: copied as words it shaped to
// glyph 0, and 0.1.73's guard then wrote it in Helvetica, which draws "ÿ".

#[test]
fn a_line_end_hyphen_is_copied_and_pasted_as_a_hyphen() {
    let Some(mut app) = vivo() else { return };
    let run = app
        .tab()
        .doc
        .as_ref()
        .unwrap()
        .session
        .text_runs(0)
        .unwrap()
        .into_iter()
        .find(|r| r.text.trim() == "\u{2}" && r.size > 0.0)
        .expect("VIVO's hyphen at the end of a line");
    app.tab_mut().selection.selected = Some(Selected { page: 0, object: run.object, rect: run.rect, what: "the words" });
    assert!(app.copy_object_selection());
    let Some(ObjectClipboard::Text { lines, face: Some(face), .. }) = app.clipboard_state.object_clipboard.clone() else {
        panic!("not copied as words in a face of their own")
    };
    assert_eq!(lines, ["-"], "the hyphen is copied as a hyphen, not as PDFium's marker for one");
    assert_eq!(boxes(&face, "-"), Vec::<char>::new(), "the hyphen's own font draws a hyphen");

    let before: std::collections::HashSet<usize> = app.tab().doc.as_ref().unwrap().session.text_runs(0).unwrap().iter().map(|r| r.object).collect();
    let centre = bare_paper(&app, 40.0, 30.0);
    app.paste_object_selection();
    app.place_paste_ghost(0, AppPoint { x: centre.x - 20.0, y: centre.y - 15.0 });
    let pasted: Vec<_> = app.tab().doc.as_ref().unwrap().session.text_runs(0).unwrap().into_iter().filter(|r| !before.contains(&r.object)).collect();
    assert_eq!(pasted.len(), 1, "one hyphen pasted: {}", said(&app));
    assert_eq!(pasted[0].text.trim(), "-", "the pasted mark reads as a hyphen: {}", said(&app));
    assert!(!said(&app).contains("Words written in"), "no other face was needed: {}", said(&app));

    // Look at it: the wrong mark was "ÿ", which is ink too.
    let region = pdf_core::document::Rect { left: centre.x as f32 - 20.0, right: centre.x as f32 + 20.0, top: centre.y as f32 - 15.0, bottom: centre.y as f32 + 15.0 };
    let raster = app.tab().doc.as_ref().unwrap().session.render_page_region(0, region, 12.0).expect("region");
    let out = std::env::temp_dir().join("pagify-pasted-hyphen.png");
    image::save_buffer(&out, &raster.pixels, raster.width, raster.height, image::ColorType::Rgba8).expect("png");
    eprintln!("pasted hyphen drawn to {}", out.display());
}

#[test]
fn a_paragraph_with_line_end_hyphens_pastes_with_no_control_character_and_no_other_face() {
    let Some(mut app) = vivo() else { return };
    let run = app.tab().doc.as_ref().unwrap().session.text_runs(0).unwrap().into_iter().find(|r| r.text.contains("Light fixing")).expect("the Fixing Device paragraph");
    // The paragraph is one column of short lines: its first line down a few more.
    app.select_group_in(0, AppPoint { x: run.rect.left as f64 - 3.0, y: run.rect.top as f64 - 3.0 }, AppPoint { x: run.rect.left as f64 + 150.0, y: run.rect.top as f64 + 60.0 }, false);
    assert!(app.tab().selection.group.len() >= 5, "selected {} things", app.tab().selection.group.len());
    assert!(app.copy_object_selection());
    let before: std::collections::HashSet<usize> = app.tab().doc.as_ref().unwrap().session.text_runs(0).unwrap().iter().map(|r| r.object).collect();
    let bounds = app.group_bounds(0).expect("bounds");
    let centre = bare_paper(&app, bounds.right - bounds.left + 20.0, bounds.bottom - bounds.top + 20.0);
    // What was copied is what is written. (Not what PDFium reads back: its text
    // layer reports any hyphen that ends a line as U+0002, a right one too.)
    let copied = app.clipboard_state.object_clipboard.clone();
    let Some(ObjectClipboard::Group(items)) = &copied else { panic!("not a group") };
    let lines: Vec<&String> = items.iter().filter_map(|(c, ..)| if let ObjectClipboard::Text { lines, .. } = c { Some(lines) } else { None }).flatten().collect();
    assert!(lines.iter().any(|l| l.ends_with('-')), "the paragraph's hyphens were copied as hyphens: {lines:?}");
    assert!(lines.iter().all(|l| !l.chars().any(char::is_control)), "a control code was copied: {lines:?}");
    app.paste_object_selection();
    app.place_paste_ghost(0, centre);
    assert!(placed_text(&app) > before.len(), "nothing pasted: {}", said(&app));
    assert!(!said(&app).contains("Words written in"), "a paste that needs no other face said so: {}", said(&app));
}

fn placed_text(app: &PagifyApp) -> usize {
    app.tab().doc.as_ref().unwrap().session.text_runs(0).unwrap().len()
}

// ---- a letter is words, not a picture (0.1.77) ----
//
// Reported from use, with the session log: two characters were copied with Edit
// Object and pasted, and neither could be edited. The log said `what=the letter`,
// then `copy image 25x24px`, then `picture pasted`: Edit Object calls a
// one-character text object "the letter" (clicking a word splits it into them),
// and the copy treated only "the words" as text, so every letter went down as a
// picture of itself — which Edit Text, rightly, finds no text in.

/// A page whose words are single letters, each a text object of its own — the
/// objects Edit Object calls "the letter".
fn page_of_letters(content: &[u8]) -> egui_kittest::Harness<'static, PagifyApp> {
    static NEXT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    let bytes = crate::g4_edit_error_tests::page_with(content, "", &[]);
    let dir = std::env::temp_dir().join("pagify-copy-letter-tests");
    std::fs::create_dir_all(&dir).expect("scratch dir");
    let path = dir.join(format!("letters-{}-{}.pdf", std::process::id(), NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)));
    std::fs::write(&path, bytes).expect("write the page");
    crate::ui_tests::harness_from(PagifyApp::new(Some(&path.to_string_lossy())))
}

fn spot(h: &egui_kittest::Harness<'static, PagifyApp>, x: f64, y: f64) -> egui::Pos2 {
    h.state().tab().view_state.last_view.expect("the page was never drawn").to_screen(AppPoint { x, y })
}

/// The page's text runs: `(object, text, rect)`.
fn text_of(h: &egui_kittest::Harness<'static, PagifyApp>) -> Vec<(usize, String, pdf_core::document::Rect)> {
    h.state().tab().doc.as_ref().unwrap().session.text_runs(0).unwrap().into_iter().map(|r| (r.object, r.text, r.rect)).collect()
}

/// A 24 pt "Q" at (100, 700): in the app's space, its box is about x 100-117, y 75-96.
const ONE_LETTER: &[u8] = b"BT /F1 24 Tf 100 700 Td (Q) Tj ET";

/// **The break this catches:** a letter copied as a picture of itself. Picked with
/// Edit Object (which calls it "the letter"), copied, it is the letter as text —
/// its size and colour with it — so that a paste is words.
#[test]
fn a_letter_picked_with_edit_object_is_copied_as_text_not_as_a_picture() {
    let mut h = page_of_letters(ONE_LETTER);
    h.state_mut().submit("editobject");
    h.run_steps(1);
    { let at = spot(&h, 108.0, 86.0); click(&mut h, at); }
    h.run_steps(2);
    let picked = h.state().tab().selection.selected.as_ref().map(|s| s.what);
    assert_eq!(picked, Some("the letter"), "setup: Edit Object should call a one-character text object a letter");

    h.event(egui::Event::Copy);
    h.run_steps(3);
    match &h.state().clipboard_state.object_clipboard {
        Some(ObjectClipboard::Text { lines, size, .. }) => {
            assert_eq!(lines, &["Q".to_string()]);
            assert!((size - 24.0).abs() < 0.5, "copied at {size} pt");
        }
        Some(ObjectClipboard::Image { .. }) => panic!("the letter was copied as a picture"),
        _ => panic!("nothing, or the wrong thing, was copied: {}", said(h.state())),
    }
}

/// **The reported break, end to end.** Copy a letter, paste it, put it down, and
/// click it with Edit Text: it opens as text, as the original does.
#[test]
fn a_pasted_letter_opens_in_edit_text_as_text() {
    let mut h = page_of_letters(ONE_LETTER);
    h.state_mut().submit("editobject");
    h.run_steps(1);
    { let at = spot(&h, 108.0, 86.0); click(&mut h, at); }
    h.run_steps(2);
    h.event(egui::Event::Copy);
    h.run_steps(3);
    h.event(egui::Event::Paste(COPIED_IN_PAGIFY.into()));
    h.run_steps(3);
    assert!(h.state().clipboard_state.paste_ghost.is_some(), "setup: the copy was not picked up; said {}", said(h.state()));
    { let at = spot(&h, 300.0, 200.0); click(&mut h, at); }
    h.run_steps(3);

    let pasted = text_of(&h).into_iter().find(|(_, text, rect)| text.trim() == "Q" && rect.left > 250.0).expect("the pasted letter is on the page as text");
    h.state_mut().submit("edittext");
    h.run_steps(1);
    { let at = spot(&h, ((pasted.2.left + pasted.2.right) / 2.0) as f64, ((pasted.2.top + pasted.2.bottom) / 2.0) as f64); click(&mut h, at); }
    h.run_steps(3);
    let editing = h.state().tab().edit.editing_run.as_ref().map(|e| e.buffer.trim().to_string());
    assert_eq!(editing.as_deref(), Some("Q"), "the pasted letter did not open as text; said {}", said(h.state()));
}

/// **The break:** the same fault in the group copy, where a marquee over letters
/// sent them down the picture path with the shapes. Every letter of the group is
/// copied as text.
#[test]
fn letters_picked_together_are_copied_as_text() {
    let mut h = page_of_letters(b"BT /F1 24 Tf 100 700 Td (A) Tj ET BT /F1 24 Tf 160 700 Td (B) Tj ET");
    let letters = text_of(&h);
    assert_eq!(letters.len(), 2, "setup: {letters:?}");
    let group: Vec<Selected> = letters.iter().map(|(object, _, rect)| Selected { page: 0, object: *object, rect: *rect, what: "the letter" }).collect();
    h.state_mut().tab_mut().selection.group = group;

    h.event(egui::Event::Copy);
    h.run_steps(3);
    let Some(ObjectClipboard::Group(items)) = &h.state().clipboard_state.object_clipboard else {
        panic!("a group was not copied: {}", said(h.state()));
    };
    let texts: Vec<&Vec<String>> = items.iter().filter_map(|(c, ..)| if let ObjectClipboard::Text { lines, .. } = c { Some(lines) } else { None }).collect();
    assert_eq!(texts.len(), 2, "the letters were not both copied as text: {} of {}", texts.len(), items.len());
}

/// **The break:** a letter picked with a picture dragging the picture down with it.
/// Words in a group are set apart before the rest is lifted as lines; with the
/// letter counted among the drawings, the lift was refused for all of them
/// ("words are copied as words") and the picture was copied as pixels.
#[test]
fn a_letter_picked_with_a_picture_does_not_stop_the_picture_being_lifted() {
    let image = b"<< /Type /XObject /Subtype /Image /Width 1 /Height 1 /ColorSpace /DeviceRGB /BitsPerComponent 8 /Length 3 >>\nstream\n\xff\x00\x00\nendstream".to_vec();
    let bytes = crate::g4_edit_error_tests::page_with(
        b"q 80 0 0 80 300 600 cm /Im Do Q BT /F1 24 Tf 100 700 Td (Q) Tj ET",
        "/XObject << /Im 6 0 R >>",
        &[image],
    );
    let dir = std::env::temp_dir().join("pagify-copy-letter-tests");
    std::fs::create_dir_all(&dir).expect("scratch dir");
    let path = dir.join(format!("mixed-{}.pdf", std::process::id()));
    std::fs::write(&path, bytes).expect("write");
    let mut h = crate::ui_tests::harness_from(PagifyApp::new(Some(&path.to_string_lossy())));

    let session = &h.state().tab().doc.as_ref().unwrap().session;
    let picture = session.images_on(0).expect("images").remove(0);
    let letter = session.text_runs(0).expect("runs").into_iter().find(|r| r.text.trim() == "Q").expect("the letter");
    let group = vec![
        Selected { page: 0, object: letter.object, rect: letter.rect, what: "the letter" },
        Selected { page: 0, object: picture.object, rect: picture.rect, what: "the picture" },
    ];
    h.state_mut().tab_mut().selection.group = group;
    h.event(egui::Event::Copy);
    h.run_steps(3);

    let Some(ObjectClipboard::Group(items)) = &h.state().clipboard_state.object_clipboard else {
        panic!("a group was not copied: {}", said(h.state()));
    };
    assert!(items.iter().any(|(c, ..)| matches!(c, ObjectClipboard::Lifted { .. })), "the picture was not lifted as lines: {}", said(h.state()));
    assert!(items.iter().any(|(c, ..)| matches!(c, ObjectClipboard::Text { lines, .. } if lines == &["Q".to_string()])), "the letter was not copied as text");
}

/// **Your file, your click.** The session log of 2026-10-14: on the VIVO datasheet
/// Edit Object picked a header letter (object 168, "the letter"), and copying it
/// made a picture. Now it is the letter as text, and pasted it opens in Edit Text.
/// Skipped when the file is not on this machine.
#[test]
fn the_vivo_headers_letter_is_copied_as_text_and_edits_when_pasted() {
    if !std::path::Path::new(VIVO).exists() {
        eprintln!("skipped: VIVO mini.pdf is not at {VIVO}");
        return;
    }
    let mut h = crate::ui_tests::harness_from(PagifyApp::new(Some(VIVO)));
    h.state_mut().submit("editobject");
    h.run_steps(1);
    // The centre of object 168, which the log names: (45.8, 26.0)-(54.4, 34.0).
    { let at = spot(&h, 50.1, 30.0); click(&mut h, at); }
    h.run_steps(3);
    let picked = h.state().tab().selection.selected.as_ref().map(|s| (s.what, s.object));
    assert_eq!(picked.map(|p| p.0), Some("the letter"), "setup: picked {picked:?}");

    h.event(egui::Event::Copy);
    h.run_steps(3);
    assert!(
        matches!(h.state().clipboard_state.object_clipboard, Some(ObjectClipboard::Text { .. })),
        "the letter was not copied as text: {}",
        said(h.state())
    );
    let before = text_of(&h).len();
    h.event(egui::Event::Paste(COPIED_IN_PAGIFY.into()));
    h.run_steps(3);
    // Bare paper well inside the page: the strip's top-right, beside the title.
    { let at = spot(&h, 440.0, 45.0); click(&mut h, at); }
    h.run_steps(3);
    let now = text_of(&h);
    let pasted = now.iter().skip(before.saturating_sub(1)).find(|(_, _, rect)| rect.left > 420.0 && rect.top < 60.0).expect("the pasted letter is on the page as text").clone();
    h.state_mut().submit("edittext");
    h.run_steps(1);
    { let at = spot(&h, ((pasted.2.left + pasted.2.right) / 2.0) as f64, ((pasted.2.top + pasted.2.bottom) / 2.0) as f64); click(&mut h, at); }
    h.run_steps(3);
    assert!(h.state().tab().edit.editing_run.is_some(), "the pasted letter did not open in Edit Text; said {}", said(h.state()));
}

/// **The break:** the letters of a word, whose boxes touch, merged into one box —
/// the way touching shapes are — so that all but one were lost from the copy. A
/// marquee over part of a word is exactly this selection.
#[test]
fn touching_letters_picked_together_are_all_copied() {
    let mut h = page_of_letters(b"BT /F1 24 Tf 100 700 Td (A) Tj ET BT /F1 24 Tf 116 700 Td (B) Tj ET");
    let letters = text_of(&h);
    assert_eq!(letters.len(), 2, "setup: {letters:?}");
    // Boxes made to overlap, as the letters of a word do.
    let group: Vec<Selected> = letters
        .iter()
        .map(|(object, _, rect)| Selected {
            page: 0,
            object: *object,
            rect: pdf_core::document::Rect { right: rect.right + 3.0, ..*rect },
            what: "the letter",
        })
        .collect();
    h.state_mut().tab_mut().selection.group = group;
    h.event(egui::Event::Copy);
    h.run_steps(3);

    let Some(ObjectClipboard::Group(items)) = &h.state().clipboard_state.object_clipboard else {
        panic!("a group was not copied: {}", said(h.state()));
    };
    let texts: Vec<String> = items.iter().filter_map(|(c, ..)| if let ObjectClipboard::Text { lines, .. } = c { Some(lines.concat()) } else { None }).collect();
    assert_eq!(texts.len(), 2, "letters were merged away: {texts:?}");
}

/// **The break:** a sideways letter (size 0, so copied as a picture) pasted with
/// the page's white paper round it, hiding what it is put down over. A letter is
/// sideways words like any other: the paper is made see-through.
#[test]
fn a_sideways_letter_is_a_picture_with_its_paper_cleared() {
    let Some(mut app) = vivo() else { return };
    let run = app.tab().doc.as_ref().unwrap().session.text_runs(0).unwrap().into_iter().find(|r| !(r.size > 0.0)).expect("VIVO's sideways label");
    app.tab_mut().selection.selected = Some(Selected { page: 0, object: run.object, rect: run.rect, what: "the letter" });
    assert!(app.copy_object_selection());
    let Some(ObjectClipboard::Image { rgba, .. }) = &app.clipboard_state.object_clipboard else { panic!("not a picture") };
    assert_eq!(rgba[3], 0, "the paper round the letter was left white (alpha {})", rgba[3]);
}

/// **Reported from use on 0.1.79: "undo also stopped working".** The session
/// log showed a placed picture dragged, then "nothing to undo" every time.
/// Moving, resizing or turning a placed picture (or a picture signature) went
/// straight to the document, never through the undo history — it had since
/// 0.1.x, and became common once pictures pasted from outside are placed ones.
#[test]
fn moving_and_turning_a_placed_picture_are_undone() {
    let mut app = opened("placed-picture.pdf", b"0 0 1 rg 72 600 50 50 re f", "", &[]);
    let rect = pdf_core::document::Rect { left: 100.0, top: 100.0, right: 140.0, bottom: 140.0 };
    app.clipboard_state.object_clipboard =
        Some(ObjectClipboard::Image { rgba: [255u8, 0, 0, 255].repeat(4), width: 2, height: 2, rect });
    app.paste_object_selection();
    let was = app_mark(&app);

    app.finish_placed_image_grab(
        PlacedImageSelected { page: 0, index: was.index, rect: was.rect, rotation: was.rotation },
        Grab { handle: None, from: AppPoint { x: 0.0, y: 0.0 }, by: (50.0, 30.0) },
    );
    assert!((app_mark(&app).rect.left - was.rect.left - 50.0).abs() < 0.5, "not moved: {:?}", app_mark(&app).rect);
    app.submit("undo");
    assert!((app_mark(&app).rect.left - was.rect.left).abs() < 0.5 && (app_mark(&app).rect.top - was.rect.top).abs() < 0.5,
        "the move was not undone: {:?} (was {:?}); said {}", app_mark(&app).rect, was.rect, said(&app));

    let now = app_mark(&app);
    app.finish_placed_image_grab(
        PlacedImageSelected { page: 0, index: now.index, rect: now.rect, rotation: now.rotation },
        Grab { handle: Some(Handle::Rotate), from: AppPoint { x: 160.0, y: 138.0 }, by: (-20.0, 20.0) },
    );
    assert!(app_mark(&app).rotation.abs() > 1.0, "not turned");
    app.submit("undo");
    assert!(app_mark(&app).rotation.abs() < 0.5, "the turn was not undone: {}", app_mark(&app).rotation);
}

fn app_mark(app: &PagifyApp) -> pdf_core::document::PlacedImageMark {
    let marks = app.tab().doc.as_ref().expect("open").session.placed_image_marks(0).expect("marks");
    assert_eq!(marks.len(), 1, "one placed picture");
    marks.into_iter().next().unwrap()
}

/// **Reported from use on 0.1.80: "the app freezes while I undo, and it goes
/// behind an invisible layer"** (session log: 227 things moved, then "brought
/// to the front", then every click "the group it is drawn in"). Bringing a
/// layer to the front went straight to the document, never into the undo
/// history — so undo could not take it back — and it renumbers the page's
/// objects, so the undo of the earlier group move then moved the wrong ones
/// (one of them the group, which only the slow way can try to move).
#[test]
fn bringing_a_layer_to_the_front_is_undone_and_the_move_before_it_still_undoes_right() {
    // An invisible page-size group at the bottom, then two squares.
    let form = b"<< /Type /XObject /Subtype /Form /BBox [0 0 612 792] /Resources << /ExtGState << /G0 << /ca 0 >> >> >> /Length 29 >>\nstream\n/G0 gs 1 1 1 rg 0 0 612 792 re f\nendstream".to_vec();
    let mut app = opened(
        "invisible-layer.pdf",
        b"q /Fm0 Do Q 1 0 0 rg 72 600 50 50 re f 0 0 1 rg 200 600 50 50 re f",
        "/XObject << /Fm0 6 0 R >>",
        &[form],
    );
    let top = |app: &PagifyApp| -> Vec<(pdf_core::document::DrawnKind, pdf_core::document::Rect)> {
        let drawn = app.tab().doc.as_ref().expect("open").session.drawn_objects(0).expect("drawn");
        drawn.into_iter().filter(|d| d.depth == 0).map(|d| (d.kind, d.rect)).collect()
    };
    let start = top(&app);
    assert_eq!(start.len(), 3, "{start:?}");
    assert_eq!(start[0].0, pdf_core::document::DrawnKind::Group);

    // The two squares moved together …
    app.tab_mut().selection.group = (1..3)
        .map(|object| Selected { page: 0, object, rect: start[object].1, what: "the shape" })
        .collect();
    app.finish_group_grab(Grab { handle: None, from: AppPoint { x: 0.0, y: 0.0 }, by: (10.0, 20.0) }, 1.0);
    let moved = top(&app);
    // … then the invisible group brought to the front, over everything.
    app.restack(0, 0, pdf_core::document::Stacking::Front).expect("bring to front");
    assert_eq!(top(&app).last().map(|d| d.0), Some(pdf_core::document::DrawnKind::Group));

    app.submit("undo");
    assert_eq!(top(&app), moved, "undo should take the group back to the bottom: {}", said(&app));
    let started = std::time::Instant::now();
    app.submit("undo");
    assert_eq!(top(&app), start, "the move should be undone, the squares back where they were: {}", said(&app));
    assert!(started.elapsed().as_millis() < 1000, "the undo took {} ms", started.elapsed().as_millis());
}
