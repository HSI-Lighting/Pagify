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

/// Pieces of a drawing pasted together are taken back together.
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

    app.paste_object_selection();
    app.place_paste_ghost(0, AppPoint { x: 500.0, y: 500.0 });
    assert_eq!(placed(&app), 2, "both pieces should have been pasted");

    app.submit("undo");
    assert_eq!(placed(&app), 0, "one undo should take the whole paste back");
    app.submit("redo");
    assert_eq!(placed(&app), 2, "one redo should put the whole paste back");
}

/// The drawing from the report (90 pieces of a lens and its leader line): it
/// goes down as a few pictures, not ninety, shows on the page, and one undo
/// takes it all back.
///
/// Not asserted: how sharp it is. A copy is the page's pixels — the engine cannot
/// lift a drawing's vectors out of the page's stream — and measured against the
/// original at several zooms the paste keeps 50–85% of its edge crispness, by
/// the ratio of the zoom to the copy's own resolution, whichever way the pieces
/// are put down. Copying the vectors is the real fix and a feature of its own.
#[test]
fn a_pasted_drawing_goes_down_as_a_few_pictures_and_comes_back_in_one_undo() {
    let Some(mut app) = vivo() else { return };
    app.select_group_in(0, AppPoint { x: 415.0, y: 106.0 }, AppPoint { x: 490.0, y: 180.0 }, false);
    let members = app.tab().selection.group.len();
    assert!(members > 50, "the marquee should have caught the drawing's pieces, got {members}");
    let bounds = app.group_bounds(0).expect("bounds");
    assert!(app.copy_object_selection());

    // Onto bare paper: a window the size of the drawing with nothing on it.
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
    let before = placed(&app);
    app.paste_object_selection();
    app.place_paste_ghost(0, at);
    let added = placed(&app) - before;
    assert!((1..=5).contains(&added), "{members} pieces went down as {added} pictures");
    assert!(ink(&app) > 500, "the drawing is not on the page where it was put down");

    app.submit("undo");
    assert_eq!(placed(&app), before, "one undo should take the whole drawing back");
    assert_eq!(ink(&app), 0, "the drawing is still on the page after undo");
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
