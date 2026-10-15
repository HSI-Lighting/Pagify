//! Arrow-key moves (`nudge.rs`) and Shift-held moves (`dragged_by`, `snap_the_move`).

use super::ui_tests::{fixture, harness};
use super::*;
use egui_kittest::Harness;

/// Press (and release) an arrow key, with or without Shift, for the next frame.
fn arrow(h: &mut Harness<'static, PagifyApp>, key: egui::Key, shift: bool) {
    let modifiers = egui::Modifiers { shift, ..Default::default() };
    // What `i.modifiers` reads is set by this event, not by the key's own.
    h.input_mut().events.push(egui::Event::ModifiersChanged(modifiers));
    for pressed in [true, false] {
        h.input_mut().events.push(egui::Event::Key { key, physical_key: None, pressed, repeat: false, modifiers });
    }
}

fn key(h: &mut Harness<'static, PagifyApp>, key: egui::Key, modifiers: egui::Modifiers) {
    h.input_mut().events.push(egui::Event::ModifiersChanged(modifiers));
    for pressed in [true, false] {
        h.input_mut().events.push(egui::Event::Key { key, physical_key: None, pressed, repeat: false, modifiers });
    }
}

/// The object tool in hand with the first picture of `pictures.pdf` picked.
fn with_a_picture_picked() -> (Harness<'static, PagifyApp>, Selected) {
    let mut h = harness("pictures.pdf");
    h.state_mut().submit("editobject");
    let picture = h.state().tab().doc.as_ref().unwrap().session.images_on(0).expect("images")[0].clone();
    let sel = Selected { page: 0, object: picture.object, rect: picture.rect, what: "the picture" };
    h.state_mut().tab_mut().selection.selected = Some(sel.clone());
    h.run_steps(2);
    (h, sel)
}

/// Where the picked picture is on the page now.
fn rect_of(h: &Harness<'static, PagifyApp>) -> pdf_core::document::Rect {
    h.state().tab().selection.selected.as_ref().expect("nothing selected").rect
}

/// One press, then long enough for it to land.
fn press_and_settle(h: &mut Harness<'static, PagifyApp>, key: egui::Key, shift: bool) {
    arrow(h, key, shift);
    h.run_steps(1);
    h.input_mut().events.push(egui::Event::ModifiersChanged(Default::default()));
    h.run_steps(4);
}

#[test]
fn an_arrow_moves_the_picked_thing_by_a_few_screen_pixels() {
    let (mut h, sel) = with_a_picture_picked();
    let step = PagifyApp::NUDGE_PX / h.state().nudge_scale();

    press_and_settle(&mut h, egui::Key::ArrowRight, false);
    let moved = rect_of(&h);
    assert!((moved.left - sel.rect.left - step).abs() < 0.05, "moved {} for a step of {step}", moved.left - sel.rect.left);
    assert!((moved.top - sel.rect.top).abs() < 0.05, "it left its row");

    press_and_settle(&mut h, egui::Key::ArrowUp, false);
    let moved = rect_of(&h);
    assert!((moved.top - sel.rect.top + step).abs() < 0.05, "up is towards the top of the page");
}

#[test]
fn a_press_moves_it_less_the_further_the_page_is_zoomed_in() {
    let (mut h, _) = with_a_picture_picked();
    let pending = |h: &mut Harness<'static, PagifyApp>, zoom: f32| {
        h.state_mut().tab_mut().view_state.zoom = ZoomMode::Factor(zoom);
        h.run_steps(2);
        arrow(h, egui::Key::ArrowRight, false);
        h.run_steps(1);
        // Read before it lands.
        let by = h.state().tab().selection.grab.as_ref().expect("no move building up").by.0;
        h.run_steps(4);
        by
    };
    let near = pending(&mut h, 1.0);
    let far = pending(&mut h, 4.0);
    assert!((near / far - 4.0).abs() < 0.01, "four times the zoom should be a quarter of the move: {near} vs {far}");
}

#[test]
fn shift_makes_a_press_ten_times_as_far() {
    let (mut h, _) = with_a_picture_picked();
    arrow(&mut h, egui::Key::ArrowLeft, false);
    h.run_steps(1);
    let one = h.state().tab().selection.grab.as_ref().unwrap().by.0;
    h.run_steps(4);
    arrow(&mut h, egui::Key::ArrowLeft, true);
    h.run_steps(1);
    let ten = h.state().tab().selection.grab.as_ref().unwrap().by.0;
    h.run_steps(4);
    assert!(one < 0.0 && (ten / one - 10.0).abs() < 0.01, "{one} then {ten}");
}

#[test]
fn a_run_of_presses_is_one_move_and_one_undo() {
    let (mut h, sel) = with_a_picture_picked();
    let step = PagifyApp::NUDGE_PX / h.state().nudge_scale();
    // Held down: a press a frame, each inside the settling time of the one before.
    for _ in 0..3 {
        arrow(&mut h, egui::Key::ArrowRight, false);
        h.run_steps(1);
    }
    // Nothing has been written to the page yet, but it is drawn building up.
    assert!((h.state().tab().selection.grab.as_ref().unwrap().by.0 - 3.0 * step).abs() < 1e-3);
    assert_eq!(rect_of(&h).left, sel.rect.left, "the page was rewritten while the keys were still down");
    h.run_steps(4);
    assert!((rect_of(&h).left - sel.rect.left - 3.0 * step).abs() < 0.05);
    assert!(h.state().tab().selection.grab.is_none() && h.state().tab().selection.nudge_at.is_none());

    // One undo takes the whole run back.
    h.state_mut().submit("undo");
    h.state_mut().tab_mut().selection.selected = None;
    h.state_mut().select_thing_at(0, AppPoint { x: ((sel.rect.left + sel.rect.right) / 2.0) as f64, y: ((sel.rect.top + sel.rect.bottom) / 2.0) as f64 });
    assert!((rect_of(&h).left - sel.rect.left).abs() < 0.05, "one undo should have put it back");
}

#[test]
fn undo_lands_a_move_still_building_up_and_then_takes_it_back() {
    let (mut h, sel) = with_a_picture_picked();
    arrow(&mut h, egui::Key::ArrowRight, true);
    h.run_steps(1);
    assert!(h.state().tab().selection.nudge_at.is_some());
    // Ctrl+Z straight away, inside the settling time.
    key(&mut h, egui::Key::Z, egui::Modifiers::COMMAND);
    h.run_steps(1);
    h.run_steps(3);
    assert!(h.state().tab().selection.grab.is_none());
    let said: String = h.state().cmd.history().iter().map(|e| e.text.as_str()).collect::<Vec<_>>().join("\n");
    assert!(said.contains("moved") || said.contains("Move"), "the move never landed before the undo:\n{said}");
    let back = h.state().tab().doc.as_ref().unwrap().session.images_on(0).unwrap()[0].rect;
    assert!((back.left - sel.rect.left).abs() < 0.05, "the undo did not take the nudge back: {back:?} vs {:?}
{said}", sel.rect);
}

#[test]
fn escape_drops_a_move_still_building_up() {
    let (mut h, sel) = with_a_picture_picked();
    arrow(&mut h, egui::Key::ArrowDown, true);
    h.run_steps(1);
    assert!(h.state().tab().selection.grab.is_some());
    key(&mut h, egui::Key::Escape, Default::default());
    h.run_steps(5);
    assert!(h.state().tab().selection.grab.is_none() && h.state().tab().selection.nudge_at.is_none());
    let now = h.state().tab().doc.as_ref().unwrap().session.images_on(0).unwrap()[0].rect;
    assert!((now.top - sel.rect.top).abs() < 0.01, "Escape should have dropped the move, not landed it");
}

#[test]
fn a_mouse_drag_is_not_added_to_by_the_arrows() {
    let (mut h, _) = with_a_picture_picked();
    h.state_mut().tab_mut().selection.grab =
        Some(Grab { handle: None, from: AppPoint { x: 0.0, y: 0.0 }, by: (5.0, 5.0) });
    arrow(&mut h, egui::Key::ArrowRight, false);
    h.run_steps(1);
    assert_eq!(h.state().tab().selection.grab.as_ref().unwrap().by, (5.0, 5.0));
    assert!(h.state().tab().selection.nudge_at.is_none());
}

#[test]
fn with_something_picked_the_arrows_do_not_turn_the_page_and_with_nothing_they_do() {
    let mut h = harness("pages-ladder.pdf");
    h.state_mut().submit("editobject");
    h.run_steps(2);
    // Nothing picked: the arrows turn the page, there and back.
    arrow(&mut h, egui::Key::ArrowRight, false);
    h.run_steps(2);
    assert_eq!(h.state().tab().view_state.page, 1, "no selection: the right arrow should turn the page");

    // Something picked (what it is does not matter to where the arrows go): neither arrow turns it.
    let rect = pdf_core::document::Rect { left: 10.0, top: 10.0, right: 50.0, bottom: 50.0 };
    h.state_mut().tab_mut().selection.selected = Some(Selected { page: 1, object: 0, rect, what: "the shape" });
    arrow(&mut h, egui::Key::ArrowLeft, false);
    h.run_steps(1);
    assert_eq!(h.state().tab().view_state.page, 1, "the left arrow turned the page instead of moving the selection");
    assert!(h.state().tab().selection.nudge_at.is_some(), "and it did not move the selection either");
    h.run_steps(4);
    // (Landing a move of something that is not there drops the selection; pick it again.)
    h.state_mut().tab_mut().selection.selected = Some(Selected { page: 1, object: 0, rect, what: "the shape" });
    arrow(&mut h, egui::Key::ArrowRight, false);
    h.run_steps(1);
    assert_eq!(h.state().tab().view_state.page, 1, "the right arrow turned the page instead of moving the selection");
}

#[test]
fn a_group_is_moved_by_the_arrows_too() {
    let mut h = harness("pictures.pdf");
    h.state_mut().submit("editobject");
    let pictures = h.state().tab().doc.as_ref().unwrap().session.images_on(0).expect("images");
    h.state_mut().tab_mut().selection.group = pictures
        .iter()
        .map(|p| Selected { page: 0, object: p.object, rect: p.rect, what: "the picture" })
        .collect();
    h.run_steps(2);
    let step = PagifyApp::NUDGE_PX / h.state().nudge_scale();
    press_and_settle(&mut h, egui::Key::ArrowDown, false);
    let after = h.state().tab().doc.as_ref().unwrap().session.images_on(0).unwrap();
    for (before, now) in pictures.iter().zip(after.iter()) {
        assert!((now.rect.top - before.rect.top - step).abs() < 0.05, "{:?} -> {:?}", before.rect, now.rect);
    }
}

// ---- Shift: straight across or straight up and down ----

#[test]
fn shift_holds_a_body_drag_to_the_axis_it_has_gone_furthest_along() {
    let body = Grab { handle: None, from: AppPoint { x: 10.0, y: 10.0 }, by: (0.0, 0.0) };
    let at = AppPoint { x: 40.0, y: 22.0 };
    assert_eq!(PagifyApp::dragged_by(&body, at, false), (30.0, 12.0));
    assert_eq!(PagifyApp::dragged_by(&body, at, true), (30.0, 0.0));
    let at = AppPoint { x: 14.0, y: -50.0 };
    assert_eq!(PagifyApp::dragged_by(&body, at, true), (0.0, -60.0));
    // A resize is not a move: Shift leaves it alone.
    let resize = Grab { handle: Some(Handle::Right), ..body };
    assert_eq!(PagifyApp::dragged_by(&resize, AppPoint { x: 40.0, y: 22.0 }, true), (30.0, 12.0));
}

#[test]
fn a_pull_onto_a_line_stays_on_the_axis_the_move_is_held_to() {
    assert_eq!(PagifyApp::along_the_held_axis((30.0, 0.0), (2.0, 3.0)), (2.0, 0.0));
    assert_eq!(PagifyApp::along_the_held_axis((0.0, -30.0), (2.0, 3.0)), (0.0, 3.0));
}

#[test]
fn a_held_move_is_not_pulled_off_its_axis_by_a_line_on_the_other() {
    let mut app = PagifyApp::new(Some(&fixture("covered.pdf")));
    let layers: Vec<_> = app.layers_on(0).to_vec();
    let page = app.tab().doc.as_ref().unwrap().strip.size_of(0).unwrap();
    let a = layers
        .iter()
        .find(|o| o.depth == 0 && (o.rect.right - o.rect.left) > 5.0 && (o.rect.right - o.rect.left) < page.0 * 0.9)
        .expect("an object");
    // Its top 3 pt under the page's top edge, which is a line within the snap.
    let height = a.rect.bottom - a.rect.top;
    let rect = pdf_core::document::Rect { top: 3.0, bottom: 3.0 + height, ..a.rect };
    let sel = Selected { page: 0, object: a.object, rect, what: "the shape" };
    app.tab_mut().selection.selected = Some(sel.clone());
    let across = || Some(Grab { handle: None, from: AppPoint { x: 0.0, y: 0.0 }, by: (40.0, 0.0) });

    // Free, it is pulled up onto the page's edge (or some other line near it)...
    app.tab_mut().selection.grab = across();
    app.snap_the_move(0, 1.0, true, false);
    let free = app.tab().selection.grab.as_ref().unwrap().by;
    assert!(free.1 != 0.0, "nothing to pull it, so this test shows nothing: {free:?}");

    // ...held to the straight, it is not.
    app.tab_mut().selection.grab = across();
    app.snap_the_move(0, 1.0, true, true);
    assert_eq!(app.tab().selection.grab.as_ref().unwrap().by.1, 0.0, "pulled off the straight");
}

#[test]
fn zoomed_far_in_a_press_still_moves_a_placed_picture() {
    // 800%: a press is about 0.4 pt, under what a placed picture's own jitter guard drops.
    let mut h = harness("two-column.pdf");
    h.state_mut()
        .place_image_at(0, AppPoint { x: 100.0, y: 400.0 }, vec![200u8; 4 * 4 * 4], 4, 4)
        .expect("placed");
    let mark = h.state().tab().doc.as_ref().unwrap().session.placed_image_marks(0).unwrap().remove(0);
    h.state_mut().tab_mut().selection.placed_image_selected =
        Some(PlacedImageSelected { page: 0, index: mark.index, rect: mark.rect, rotation: mark.rotation });
    h.state_mut().tab_mut().view_state.zoom = ZoomMode::Factor(8.0);
    h.run_steps(2);
    assert!(PagifyApp::NUDGE_PX / h.state().nudge_scale() < 0.5, "this test is about a step under the guard");

    press_and_settle(&mut h, egui::Key::ArrowRight, false);
    let moved = h.state().tab().doc.as_ref().unwrap().session.placed_image_marks(0).unwrap().remove(0);
    assert!(moved.rect.left - mark.rect.left >= 0.5, "the press did nothing: {}", moved.rect.left - mark.rect.left);
}
