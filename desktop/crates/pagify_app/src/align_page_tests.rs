//! **Align to page** (`align_page.rs`): what is selected goes against an edge of the page, or into
//! the middle of it.
//!
//! **The ground truth is the page's own picture**, or a position read back from the engine after the
//! edit, never the number the code worked out (two earlier tests could not fail because they compared
//! against the value the code had used). Each test below was shown to fail with the delta's sign
//! flipped and with the text object's box used where the ink should be.

use super::wrap_hyphen_tests::open_page;
use super::*;
use crate::align_page::Aligned;
use egui_kittest::kittest::Queryable as _;
use crate::ui_tests::{fixture, harness_from};
use pagify_shell::align_page::AlignTo;

/// Pixels per page point in the renders below.
const PX: f32 = 4.0;
const LETTER: (f32, f32) = (612.0, 792.0);
/// A red square, 50 pt, whose lower-left corner is at PDF (100, 100): on screen x 100..150, y 642..692.
const SQUARE: &str = "1 0 0 rg 100 100 50 50 re f\n";

fn is_red(p: &[u8]) -> bool {
    p[0] > 180 && p[1] < 80 && p[2] < 80
}

fn is_dark(p: &[u8]) -> bool {
    p[0] < 110 && p[1] < 110 && p[2] < 110
}

/// The box (left, top, right, bottom), in page points as the page is shown, round the pixels of the
/// page's render that `keep` picks, looking only at `rows` (page points from the top) if given.
fn ink(app: &PagifyApp, rows: Option<(f32, f32)>, keep: fn(&[u8]) -> bool) -> [f32; 4] {
    let session = &app.tab().doc.as_ref().expect("open").session;
    let raster = session.render_page(0, PX).expect("a render");
    let (w, h) = (raster.width as usize, raster.height as usize);
    let (from, to) = rows.map_or((0, h), |(a, b)| ((a * PX) as usize, ((b * PX) as usize).min(h)));
    let mut found = [f32::MAX, f32::MAX, f32::MIN, f32::MIN];
    for y in from..to {
        for x in 0..w {
            if keep(&raster.pixels[(y * w + x) * 4..]) {
                found = [found[0].min(x as f32), found[1].min(y as f32), found[2].max(x as f32 + 1.0), found[3].max(y as f32 + 1.0)];
            }
        }
    }
    assert!(found[0] != f32::MAX, "nothing of that colour on the page");
    found.map(|v| v / PX)
}

fn mid_x(b: [f32; 4]) -> f32 {
    (b[0] + b[2]) / 2.0
}

fn mid_y(b: [f32; 4]) -> f32 {
    (b[1] + b[3]) / 2.0
}

/// The page with the square picked by Edit Object.
fn with_the_square_picked() -> PagifyApp {
    let mut app = open_page("align-square", &[], SQUARE);
    app.submit("editobject");
    assert!(app.select_thing_at_drilling(0, AppPoint { x: 125.0, y: 667.0 }, false), "the square was not picked");
    app
}

fn said(app: &PagifyApp) -> String {
    app.cmd.history().iter().map(|e| e.text.as_str()).collect::<Vec<_>>().join("\n")
}

#[test]
fn each_of_the_six_puts_the_square_where_the_page_shows_it_there() {
    for how in AlignTo::ALL {
        let mut app = with_the_square_picked();
        let before = ink(&app, None, is_red);
        app.align_to_page(how);
        let after = ink(&app, None, is_red);
        let tol = 0.5;
        match how {
            AlignTo::Left => assert!(after[0].abs() < tol, "{how:?}: left edge at {}", after[0]),
            AlignTo::CentreH => assert!((mid_x(after) - LETTER.0 / 2.0).abs() < tol, "{how:?}: centre at {}", mid_x(after)),
            AlignTo::Right => assert!((after[2] - LETTER.0).abs() < tol, "{how:?}: right edge at {}", after[2]),
            AlignTo::Top => assert!(after[1].abs() < tol, "{how:?}: top edge at {}", after[1]),
            AlignTo::Middle => assert!((mid_y(after) - LETTER.1 / 2.0).abs() < tol, "{how:?}: middle at {}", mid_y(after)),
            AlignTo::Bottom => assert!((after[3] - LETTER.1).abs() < tol, "{how:?}: bottom edge at {}", after[3]),
        }
        // Across leaves the height alone, and down leaves the width.
        if matches!(how, AlignTo::Left | AlignTo::CentreH | AlignTo::Right) {
            assert!((mid_y(before) - mid_y(after)).abs() < tol, "{how:?} moved it up or down: {before:?} to {after:?}");
        } else {
            assert!((mid_x(before) - mid_x(after)).abs() < tol, "{how:?} moved it across: {before:?} to {after:?}");
        }
    }
}

#[test]
fn one_undo_puts_the_square_back() {
    let mut app = with_the_square_picked();
    let before = ink(&app, None, is_red);
    app.align_to_page(AlignTo::CentreH);
    assert!((mid_x(ink(&app, None, is_red)) - mid_x(before)).abs() > 100.0, "test assumption: it moved");
    app.submit("undo");
    let back = ink(&app, None, is_red);
    for i in 0..4 {
        assert!((back[i] - before[i]).abs() < 0.3, "one undo left it at {back:?}, not {before:?}\n{}", said(&app));
    }
}

#[test]
fn a_square_already_in_the_middle_is_not_moved_again() {
    let mut app = with_the_square_picked();
    app.align_to_page(AlignTo::CentreH);
    let once = ink(&app, None, is_red);
    app.align_to_page(AlignTo::CentreH);
    assert_eq!(ink(&app, None, is_red), once);
    assert!(said(&app).contains("Already there"), "{}", said(&app));
}

/// A word and a square, picked together by a marquee.
fn a_word_and_a_square_picked_together() -> PagifyApp {
    let mut app = open_page("align-group", &[(300.0, 400.0, "hello there")], SQUARE);
    app.submit("editobject");
    app.select_group_in(0, AppPoint { x: 90.0, y: 370.0 }, AppPoint { x: 420.0, y: 700.0 }, false);
    assert_eq!(app.tab().selection.group.len(), 2, "the marquee should take the word and the square");
    app
}

/// The group's place on the page: the union of the red square and the dark word.
fn group_box(app: &PagifyApp) -> ([f32; 4], [f32; 4]) {
    (ink(app, None, is_red), ink(app, None, is_dark))
}

#[test]
fn a_group_is_centred_as_one_and_keeps_its_members_where_they_were_to_each_other() {
    let mut app = a_word_and_a_square_picked_together();
    let (square, word) = group_box(&app);
    let gap = word[0] - square[2];
    let rise = mid_y(word) - mid_y(square);
    app.align_to_page(AlignTo::CentreH);
    let (square2, word2) = group_box(&app);
    let (left, right) = (square2[0].min(word2[0]), square2[2].max(word2[2]));
    assert!(((left + right) / 2.0 - LETTER.0 / 2.0).abs() < 1.0, "the group is centred at {}", (left + right) / 2.0);
    assert!((word2[0] - square2[2] - gap).abs() < 0.3, "the members moved relative to each other");
    assert!((mid_y(word2) - mid_y(square2) - rise).abs() < 0.3, "one of them moved up or down");

    app.align_to_page(AlignTo::Bottom);
    let (square3, word3) = group_box(&app);
    assert!((square3[3].max(word3[3]) - LETTER.1).abs() < 1.0, "the group's lowest edge is at {}", square3[3].max(word3[3]));
}

#[test]
fn one_undo_puts_a_whole_group_back() {
    let mut app = a_word_and_a_square_picked_together();
    let (square, word) = group_box(&app);
    app.align_to_page(AlignTo::CentreH);
    app.submit("undo");
    let (square2, word2) = group_box(&app);
    for i in 0..4 {
        assert!((square2[i] - square[i]).abs() < 0.3 && (word2[i] - word[i]).abs() < 0.3, "not back: {square2:?} {word2:?}\n{}", said(&app));
    }
}

/// A line whose text object's box runs 50 pt past its letters (a gap, then a trailing space, as a
/// table cell's padding does); the letters are what a person sees.
const PADDED_LINE: &str = "BT /F1 10 Tf 100 700 Td [(alpha beta gamma) -5000 ( )] TJ ET\n";

#[test]
fn words_are_centred_by_their_letters_and_not_by_the_box_that_runs_past_them() {
    let mut app = open_page("align-padded", &[], PADDED_LINE);
    app.submit("editobject");
    assert!(app.select_thing_at_drilling(0, AppPoint { x: 110.0, y: 89.0 }, false), "the words were not picked");
    let sel = app.tab().selection.selected.clone().expect("picked");
    let letters = ink(&app, None, is_dark);
    assert!(sel.rect.right - letters[2] > 20.0, "test assumption: the box ({}) runs past the letters ({})", sel.rect.right, letters[2]);

    app.align_to_page(AlignTo::CentreH);
    let after = ink(&app, None, is_dark);
    assert!((mid_x(after) - LETTER.0 / 2.0).abs() < 1.5, "the letters are centred at {}, not {}", mid_x(after), LETTER.0 / 2.0);

    app.align_to_page(AlignTo::Right);
    let right = ink(&app, None, is_dark);
    assert!((LETTER.0 - right[2]).abs() < 1.5, "the last letter ends {} from the edge", LETTER.0 - right[2]);
}

/// A page turned a quarter by its own `/Rotate`, with the red square on it; shown 792 wide.
fn turned_square_page() -> PagifyApp {
    let content = SQUARE;
    let objects = [
        "<< /Type /Catalog /Pages 2 0 R >>".to_string(),
        "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_string(),
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Rotate 90 /Contents 4 0 R /Resources << >> >>".to_string(),
        format!("<< /Length {} >>\nstream\n{content}endstream", content.len()),
    ];
    let mut out = b"%PDF-1.4\n".to_vec();
    let mut offsets = Vec::new();
    for (i, body) in objects.iter().enumerate() {
        offsets.push(out.len());
        out.extend_from_slice(format!("{} 0 obj\n{body}\nendobj\n", i + 1).as_bytes());
    }
    let xref_at = out.len();
    out.extend_from_slice(format!("xref\n0 {}\n0000000000 65535 f \n", objects.len() + 1).as_bytes());
    for offset in &offsets {
        out.extend_from_slice(format!("{offset:010} 00000 n \n").as_bytes());
    }
    out.extend_from_slice(format!("trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref_at}\n%%EOF", objects.len() + 1).as_bytes());
    let dir = std::env::temp_dir().join("pagify-app-tests");
    std::fs::create_dir_all(&dir).expect("scratch dir");
    let path = dir.join("align-turned.pdf");
    std::fs::write(&path, out).expect("write the page");
    let app = PagifyApp::new(Some(&path.to_string_lossy()));
    assert!(app.tab().doc.is_some(), "the turned page did not open");
    app
}

#[test]
fn on_a_turned_page_the_middle_is_the_middle_of_what_is_shown() {
    let mut app = turned_square_page();
    let shown = app.tab().doc.as_ref().unwrap().session.render_page(0, 1.0).expect("render");
    assert_eq!((shown.width, shown.height), (792, 612), "test assumption: the page is shown on its side");
    app.submit("editobject");
    let before = ink(&app, None, is_red);
    let at = AppPoint { x: ((before[0] + before[2]) / 2.0) as f64, y: ((before[1] + before[3]) / 2.0) as f64 };
    assert!(app.select_thing_at_drilling(0, at, false), "the square was not picked where it is shown");

    app.align_to_page(AlignTo::CentreH);
    let after = ink(&app, None, is_red);
    assert!((mid_x(after) - 396.0).abs() < 0.5, "centred at {} on a page shown 792 wide", mid_x(after));
    assert!((mid_y(after) - mid_y(before)).abs() < 0.5, "it left its row");
    app.align_to_page(AlignTo::Middle);
    assert!((mid_y(ink(&app, None, is_red)) - 306.0).abs() < 0.5, "middle of a page shown 612 high");
}

/// A red picture put on the page the way a paste puts it, and picked.
fn with_a_placed_picture() -> PagifyApp {
    let mut app = open_page("align-picture", &[], "");
    let rect = pdf_core::document::Rect { left: 100.0, top: 100.0, right: 140.0, bottom: 140.0 };
    app.clipboard_state.object_clipboard = Some(ObjectClipboard::Image { rgba: [255u8, 0, 0, 255].repeat(4), width: 2, height: 2, rect });
    app.paste_object_selection();
    let mark = placed_mark(&app);
    app.tab_mut().selection.placed_image_selected =
        Some(PlacedImageSelected { page: 0, index: mark.index, rect: mark.rect, rotation: mark.rotation });
    app
}

fn placed_mark(app: &PagifyApp) -> pdf_core::document::PlacedImageMark {
    let marks = app.tab().doc.as_ref().expect("open").session.placed_image_marks(0).expect("marks");
    assert_eq!(marks.len(), 1, "one placed picture");
    marks.into_iter().next().unwrap()
}

#[test]
fn a_placed_picture_is_aligned_and_one_undo_takes_it_back() {
    let mut app = with_a_placed_picture();
    let was = placed_mark(&app).rect;
    app.align_to_page(AlignTo::CentreH);
    let now = placed_mark(&app).rect;
    assert!(((now.left + now.right) / 2.0 - LETTER.0 / 2.0).abs() < 0.5, "centred at {}", (now.left + now.right) / 2.0);
    assert!((now.top - was.top).abs() < 0.5, "it left its row");
    app.align_to_page(AlignTo::Bottom);
    assert!((placed_mark(&app).rect.bottom - LETTER.1).abs() < 0.5, "bottom edge at {}", placed_mark(&app).rect.bottom);

    app.submit("undo");
    app.submit("undo");
    let back = placed_mark(&app).rect;
    assert!((back.left - was.left).abs() < 0.5 && (back.top - was.top).abs() < 0.5, "two undos left it at {back:?}, not {was:?}\n{}", said(&app));
}

#[test]
fn a_turned_picture_is_aligned_by_the_outline_it_shows() {
    let mut app = with_a_placed_picture();
    let mark = placed_mark(&app);
    // A 40 x 40 picture turned 45 degrees shows a diamond 56.6 across.
    app.finish_placed_image_grab(
        PlacedImageSelected { page: 0, index: mark.index, rect: mark.rect, rotation: mark.rotation },
        Grab { handle: Some(Handle::Rotate), from: AppPoint { x: 160.0, y: 120.0 }, by: (-20.0, 20.0) },
        false,
    );
    let turned = placed_mark(&app);
    assert!(turned.rotation.abs() > 1.0, "test assumption: it was turned");
    let half = (turned.rect.right - turned.rect.left) * (turned.rotation.to_radians().cos().abs() + turned.rotation.to_radians().sin().abs()) / 2.0;
    app.align_to_page(AlignTo::Left);
    let after = placed_mark(&app).rect;
    assert!(((after.left + after.right) / 2.0 - half).abs() < 0.6, "its shown outline should start at the edge: centre {}, half-width {half}", (after.left + after.right) / 2.0);
}

#[test]
fn drawn_shapes_are_aligned_by_their_own_outline() {
    let mut app = PagifyApp::new(Some(&fixture("single-page.pdf")));
    app.submit("circle");
    app.take_pick(AppPoint { x: 80.0, y: 80.0 });
    app.take_pick(AppPoint { x: 120.0, y: 80.0 });
    app.submit("all");
    let page_w = app.tab().doc.as_ref().unwrap().session.page_size(0).unwrap().width_pt;
    app.align_to_page(AlignTo::CentreH);
    let (lo, hi) = app.tab_mut().markup.existing(0).unwrap().objects()[0].geom.bbox();
    assert!((((lo.x + hi.x) / 2.0) as f32 - page_w / 2.0).abs() < 0.5, "centred at {}", (lo.x + hi.x) / 2.0);
    app.align_to_page(AlignTo::Top);
    let (_, hi) = app.tab_mut().markup.existing(0).unwrap().objects()[0].geom.bbox();
    let page_h = app.tab().doc.as_ref().unwrap().session.page_size(0).unwrap().height_pt;
    assert!((hi.y as f32 - page_h).abs() < 0.5, "the top of the shape is {} (the page is {page_h} high; kernel space counts upward)", hi.y);
}

#[test]
fn a_new_text_box_is_aligned_before_it_is_added() {
    let mut app = PagifyApp::new(Some(&fixture("text-lines.pdf")));
    app.begin_text_box(0, AppPoint { x: 20.0, y: 100.0 }, AppPoint { x: 120.0, y: 160.0 }).expect("a box");
    let page = app.tab().doc.as_ref().unwrap().session.page_size(0).unwrap();
    app.align_to_page(AlignTo::CentreH);
    let r = app.tab().edit.new_text_box.as_ref().expect("still open").rect;
    assert!((((r.left + r.right) / 2.0) - page.width_pt / 2.0).abs() < 0.01, "centred at {}", (r.left + r.right) / 2.0);
    assert!((r.right - r.left - 100.0).abs() < 0.01 && (r.bottom - r.top - 60.0).abs() < 0.01, "the box changed size: {r:?}");
    app.align_to_page(AlignTo::Bottom);
    assert!((app.tab().edit.new_text_box.as_ref().unwrap().rect.bottom - page.height_pt).abs() < 0.01);
}

const LINES: [(f32, f32, &str); 3] = [(100.0, 700.0, "alpha beta gamma"), (100.0, 688.0, "delta epsilon zeta"), (100.0, 676.0, "eta theta iota")];

fn with_the_paragraph_open() -> PagifyApp {
    let mut app = open_page("align-paragraph", &LINES, "");
    let run = app.tab().doc.as_ref().unwrap().session.text_runs(0).unwrap().into_iter().find(|r| r.text.contains("delta")).expect("a line");
    app.submit("edittext");
    let at = AppPoint { x: ((run.rect.left + run.rect.right) / 2.0) as f64, y: ((run.rect.top + run.rect.bottom) / 2.0) as f64 };
    app.pick_text_run(0, at).expect("picked");
    assert!(app.tab().edit.editing_run.is_some(), "the paragraph did not open");
    app
}

#[test]
fn an_open_paragraph_moves_as_one_block_and_opens_again_where_it_landed() {
    let mut app = with_the_paragraph_open();
    let before = ink(&app, Some((60.0, 130.0)), is_dark);
    let first_line = ink(&app, Some((80.0, 94.0)), is_dark);
    app.align_to_page(AlignTo::CentreH);
    let after = ink(&app, Some((60.0, 130.0)), is_dark);
    assert!((mid_x(after) - LETTER.0 / 2.0).abs() < 1.5, "the paragraph is centred at {}\n{}", mid_x(after), said(&app));
    assert!((after[1] - before[1]).abs() < 0.5 && (after[3] - before[3]).abs() < 0.5, "it moved up or down");
    let line_after = ink(&app, Some((80.0, 94.0)), is_dark);
    assert!(((line_after[0] - after[0]) - (first_line[0] - before[0])).abs() < 0.5, "the lines moved relative to each other");
    assert!(app.tab().edit.editing_run.is_some(), "the box was not opened again\n{}", said(&app));

    // An open box takes a command line as the words to type, so it is put down first.
    app.put_down_page_editors("test");
    app.submit("undo");
    let back = ink(&app, Some((60.0, 130.0)), is_dark);
    for i in 0..4 {
        assert!((back[i] - before[i]).abs() < 0.3, "one undo left {back:?}, not {before:?}");
    }
}

#[test]
fn a_paragraph_with_changed_words_is_not_moved_and_says_why() {
    let mut app = with_the_paragraph_open();
    app.tab_mut().edit.editing_run.as_mut().unwrap().buffer.push('x');
    let before = ink(&app, Some((60.0, 130.0)), is_dark);
    let why = app.align_refusal(Aligned::Paragraph).expect("refused");
    assert!(why.contains("Apply or cancel"), "{why}");
    app.align_to_page(AlignTo::CentreH);
    assert_eq!(ink(&app, Some((60.0, 130.0)), is_dark), before, "a refused alignment moved it");
}

/// The panel offers the six buttons, for a picked object, and they work.
#[test]
fn the_properties_panel_opens_for_a_picked_object_and_its_buttons_align_it() {
    let mut h = harness_from(with_the_square_picked());
    h.run_steps(3);
    let before = ink(h.state(), None, is_red);
    for label in ["Left", "Centre", "Right", "Top", "Middle", "Bottom"] {
        h.get_by_label(label);
    }
    h.get_by_label("Centre").click();
    h.run_steps(3);
    let after = ink(h.state(), None, is_red);
    assert!((mid_x(after) - LETTER.0 / 2.0).abs() < 0.5, "centred at {} (was {})", mid_x(after), mid_x(before));
}

#[test]
fn the_panel_is_up_with_edit_object_in_hand_and_greyed_until_something_is_picked() {
    let mut app = open_page("align-nothing", &[], SQUARE);
    app.submit("editobject");
    let mut h = harness_from(app);
    h.run_steps(3);
    let before = h.state().tab().view_state.last_view.expect("drawn").to_screen(AppPoint { x: 100.0, y: 100.0 });
    assert!(h.query_by_label("Centre").is_some(), "Align to page was not up with Edit Object in hand");
    assert_eq!(h.state().align_subject(), Some(Aligned::Waiting));
    assert!(h.state().align_refusal(Aligned::Waiting).is_some(), "the buttons should be greyed with nothing picked");
    // Picking something does not move the page: the panel was already up.
    let state = h.state_mut();
    assert!(state.select_thing_at_drilling(0, AppPoint { x: 125.0, y: 667.0 }, false));
    h.run_steps(3);
    let after = h.state().tab().view_state.last_view.expect("drawn").to_screen(AppPoint { x: 100.0, y: 100.0 });
    assert_eq!(before, after, "the page slid when something was picked");
    assert_eq!(h.state().align_subject(), Some(Aligned::Object));
    assert!(h.state().align_refusal(Aligned::Object).is_none(), "the buttons stayed greyed with the square picked");
}

#[test]
fn the_panel_is_not_there_without_a_tool_or_a_selection() {
    let mut h = harness_from(open_page("align-no-tool", &[], SQUARE));
    h.run_steps(3);
    assert!(h.query_by_label("Centre").is_none(), "Align to page showed with nothing in hand");
}

/// A thing dragged near the middle of the page is pulled onto it, and a line is shown.
#[test]
fn a_drag_near_the_pages_middle_lands_exactly_on_it() {
    let mut app = with_the_square_picked();
    let sel = app.tab().selection.selected.clone().expect("picked");
    // Centre at 125; dropped 3 pt short of the middle, with 6 pt of pull at 1 px a point.
    let by = (LETTER.0 / 2.0 - 3.0 - 125.0, 0.0);
    app.tab_mut().selection.grab = Some(Grab { handle: None, from: AppPoint { x: 0.0, y: 0.0 }, by });
    app.snap_the_move(0, 1.0, true, false);
    let landed = app.tab().selection.grab.as_ref().expect("still dragging").by;
    let centre = (sel.rect.left + sel.rect.right) / 2.0 + landed.0;
    assert!((centre - LETTER.0 / 2.0).abs() < 0.01, "pulled to {centre}, not the page's middle");
    // And with Alt held (no pull) it is left where the pointer put it.
    app.tab_mut().selection.grab = Some(Grab { handle: None, from: AppPoint { x: 0.0, y: 0.0 }, by });
    app.snap_the_move(0, 1.0, false, false);
    assert_eq!(app.tab().selection.grab.as_ref().unwrap().by, by);
}

/// A placed picture, a signature and a drawn shape are pulled onto the page's middle as Edit Object's
/// things are, and a green line shows when they sit on it.
#[test]
fn a_dragged_picture_signature_or_shape_is_pulled_onto_the_pages_middle() {
    let hold = |by: (f32, f32)| Grab { handle: None, from: AppPoint { x: 0.0, y: 0.0 }, by };

    // A placed picture on a page 612 wide: dropped 3 pt short of the middle.
    let mut app = with_a_placed_picture();
    let mark = placed_mark(&app);
    let centre = (mark.rect.left + mark.rect.right) / 2.0;
    let by = (LETTER.0 / 2.0 - 3.0 - centre, 0.0);
    app.tab_mut().selection.placed_image_grab = Some(hold(by));
    app.snap_to_page_lines(0, 1.0, true, false);
    let landed = app.tab().selection.placed_image_grab.as_ref().unwrap().by;
    assert!((centre + landed.0 - LETTER.0 / 2.0).abs() < 0.01, "the picture was pulled to {landed:?}");
    let lines = app.page_lines_while_dragging(0).expect("lines while it is dragged");
    assert!(lines.vertical.iter().any(|l| l.exact && (l.at - LETTER.0 / 2.0).abs() < 0.01), "no green line on the middle: {lines:?}");
    // Alt held: the lines show and nothing pulls.
    app.tab_mut().selection.placed_image_grab = Some(hold(by));
    app.snap_to_page_lines(0, 1.0, false, false);
    assert_eq!(app.tab().selection.placed_image_grab.as_ref().unwrap().by, by);

    // A signature.
    let mut app = open_page("align-signature", &[], "");
    let rect = pdf_core::document::Rect { left: 100.0, top: 100.0, right: 140.0, bottom: 140.0 };
    app.tab_mut().selection.signature_selected = Some(SignatureSelected { page: 0, index: 0, rect, rotation: 0.0 });
    let by = (LETTER.0 / 2.0 - 3.0 - 120.0, 0.0);
    app.tab_mut().selection.signature_grab = Some(hold(by));
    app.snap_to_page_lines(0, 1.0, true, false);
    let landed = app.tab().selection.signature_grab.as_ref().unwrap().by;
    assert!((120.0 + landed.0 - LETTER.0 / 2.0).abs() < 0.01, "the signature was pulled to {landed:?}");

    // A drawn shape.
    let mut app = PagifyApp::new(Some(&fixture("single-page.pdf")));
    app.submit("circle");
    app.take_pick(AppPoint { x: 80.0, y: 80.0 });
    app.take_pick(AppPoint { x: 120.0, y: 80.0 });
    app.submit("all");
    let page_w = app.tab().doc.as_ref().unwrap().session.page_size(0).unwrap().width_pt;
    let bounds = app.markup_selection_bounds(0).expect("the circle is selected");
    let by = (page_w / 2.0 - 3.0 - (bounds.left + bounds.right) / 2.0, 0.0);
    app.tab_mut().selection.markup_grab = Some(hold(by));
    app.snap_to_page_lines(0, 1.0, true, false);
    let landed = app.tab().selection.markup_grab.as_ref().unwrap().by;
    assert!(((bounds.left + bounds.right) / 2.0 + landed.0 - page_w / 2.0).abs() < 0.01, "the shape was pulled to {landed:?}");
}

/// The same through the pointer: a placed picture dragged to 3 pt short of the middle lands on the
/// middle. (The test above calls the pull; this one proves the canvas calls it while the drag runs.)
#[test]
fn a_picture_dragged_by_the_pointer_lands_on_the_pages_middle() {
    let mut h = harness_from(with_a_placed_picture());
    h.run_steps(3);
    let mark = placed_mark(h.state()).rect;
    let (cx, cy) = ((mark.left + mark.right) / 2.0, (mark.top + mark.bottom) / 2.0);
    let view = h.state().tab().view_state.last_view.expect("the page was drawn");
    let from = view.to_screen(AppPoint { x: cx as f64, y: cy as f64 });
    // A hand starts a drag with a few pixels, which is where egui decides it is one: the grab
    // begins where the pointer is by then, so a first jump past the picture would miss it.
    let button = |pos, pressed| egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() };
    h.input_mut().events.push(egui::Event::PointerMoved(from));
    h.run_steps(1);
    h.input_mut().events.push(button(from, true));
    h.run_steps(1);
    h.input_mut().events.push(egui::Event::PointerMoved(from + egui::vec2(10.0, 0.0)));
    h.run_steps(2);
    let start = h.state().tab().selection.placed_image_grab.as_ref().expect("the picture was not taken hold of").from;
    // The drag is measured from where it was recognised: end 3 pt short of the middle.
    let to = view.to_screen(AppPoint { x: start.x + (LETTER.0 / 2.0 - 3.0 - cx) as f64, y: start.y });
    let from = view.to_screen(start);
    for i in 1..=4 {
        h.input_mut().events.push(egui::Event::PointerMoved(from + (to - from) * (i as f32 / 4.0)));
        h.run_steps(1);
    }
    h.input_mut().events.push(button(to, false));
    h.run_steps(2);
    let now = placed_mark(h.state()).rect;
    assert!(((now.left + now.right) / 2.0 - LETTER.0 / 2.0).abs() < 0.6, "it landed with its centre at {}, the middle is {}", (now.left + now.right) / 2.0, LETTER.0 / 2.0);
}
