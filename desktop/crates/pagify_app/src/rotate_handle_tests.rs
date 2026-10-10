use super::ui_tests::{click, drag, harness};
use super::*;

/// Select the first picture of `pictures.pdf` through a real click.
fn picture_selected() -> (egui_kittest::Harness<'static, PagifyApp>, PageView, pdf_core::document::Rect) {
    let mut h = harness("pictures.pdf");
    let view = h.state().tab().view_state.last_view.expect("the page was never drawn");
    h.state_mut().submit("editobject");
    h.run_steps(1);
    let image = h.state().tab().doc.as_ref().unwrap().session.images_on(0).unwrap().remove(0);
    let middle = view.to_screen(AppPoint {
        x: ((image.rect.left + image.rect.right) / 2.0) as f64,
        y: ((image.rect.top + image.rect.bottom) / 2.0) as f64,
    });
    click(&mut h, middle);
    assert!(h.state().tab().selected.is_some(), "the picture was not selected");
    (h, view, image.rect)
}

#[test]
fn the_rotate_handle_is_above_the_top_edge_and_a_press_there_is_a_turn() {
    let (h, view, rect) = picture_selected();
    let handle = PagifyApp::rotate_handle_screen_pos(&rect, view);
    let page_point = view.to_page(handle);
    assert_eq!(h.state().handle_at(page_point, view), Some(Handle::Rotate));
    // Nowhere else near it is the rotate handle: the middle of the picture, and a corner.
    let middle = AppPoint { x: ((rect.left + rect.right) / 2.0) as f64, y: ((rect.top + rect.bottom) / 2.0) as f64 };
    assert_ne!(h.state().handle_at(middle, view), Some(Handle::Rotate));
    assert_eq!(
        h.state().handle_at(AppPoint { x: rect.right as f64, y: rect.bottom as f64 }, view),
        Some(Handle::BottomRight)
    );
}

#[test]
fn dragging_the_rotate_handle_a_quarter_clockwise_turns_the_picture_and_undo_turns_it_back() {
    let (mut h, view, rect) = picture_selected();
    let (w, ht) = (rect.right - rect.left, rect.bottom - rect.top);
    let centre = view.to_screen(AppPoint {
        x: ((rect.left + rect.right) / 2.0) as f64,
        y: ((rect.top + rect.bottom) / 2.0) as f64,
    });
    let handle = PagifyApp::rotate_handle_screen_pos(&rect, view);
    // Straight above the middle to straight to its right: a quarter clockwise.
    let reach = (handle - centre).length();
    drag(&mut h, handle, centre + egui::vec2(reach, 0.0));

    let turned = h.state().tab().doc.as_ref().unwrap().session.images_on(0).unwrap().remove(0);
    let (tw, th) = (turned.rect.right - turned.rect.left, turned.rect.bottom - turned.rect.top);
    assert!(
        (tw - ht).abs() < 1.5 && (th - w).abs() < 1.5,
        "a quarter turn should swap the sides: {rect:?} then {:?}",
        turned.rect
    );
    let said = h.state().cmd.history().iter().map(|e| e.text.clone()).collect::<Vec<_>>().join("\n");
    assert!(said.contains("turned -90"), "{said}");

    h.state_mut().submit("undo");
    h.run_steps(2);
    let back = h.state().tab().doc.as_ref().unwrap().session.images_on(0).unwrap().remove(0);
    assert!(
        (back.rect.left - rect.left).abs() < 1.5 && (back.rect.right - rect.right).abs() < 1.5,
        "undo did not turn it back: {rect:?} then {:?}",
        back.rect
    );
}

/// While it is turned, nothing is applied — the page changes once, on release.
#[test]
fn while_the_handle_is_dragged_the_angle_is_shown_and_the_page_has_not_changed() {
    let (mut h, view, rect) = picture_selected();
    let centre = view.to_screen(AppPoint {
        x: ((rect.left + rect.right) / 2.0) as f64,
        y: ((rect.top + rect.bottom) / 2.0) as f64,
    });
    let handle = PagifyApp::rotate_handle_screen_pos(&rect, view);
    let from = view.to_page(handle);
    // A drag in progress, by the amount that carries the pointer to the right of the middle.
    let reach = (handle - centre).length() as f64 / view.scale as f64;
    let to_right = (
        ((rect.left + rect.right) / 2.0) as f64 + reach - from.x,
        ((rect.top + rect.bottom) / 2.0) as f64 - from.y,
    );
    h.state_mut().tab_mut().grab =
        Some(Grab { handle: Some(Handle::Rotate), from, by: (to_right.0 as f32, to_right.1 as f32) });
    h.run_steps(2);

    let shown: Vec<String> = h
        .output()
        .shapes
        .iter()
        .filter_map(|s| match &s.shape {
            egui::Shape::Text(t) => Some(t.galley.text().to_string()),
            _ => None,
        })
        .collect();
    assert!(shown.iter().any(|t| t == "-90\u{b0}"), "no angle label for a quarter turn clockwise: {shown:?}");

    let unchanged = h.state().tab().doc.as_ref().unwrap().session.images_on(0).unwrap().remove(0);
    assert_eq!(unchanged.rect, rect, "the page changed before the pointer was let go");
}

/// Shift snaps a turn to whole steps of 15 degrees.
#[test]
fn a_turn_is_kept_in_range_and_snaps_to_fifteen_degrees_with_shift() {
    let rect = pdf_core::document::Rect { left: 100.0, top: 100.0, right: 200.0, bottom: 140.0 };
    let centre = (150.0, 120.0);
    // From straight above to a point 33 degrees clockwise of it.
    let from = AppPoint { x: centre.0, y: centre.1 - 50.0 };
    let (s, c) = 33.0f64.to_radians().sin_cos();
    let to = (centre.0 + 50.0 * s, centre.1 - 50.0 * c);
    let grab = Grab { handle: Some(Handle::Rotate), from, by: ((to.0 - from.x) as f32, (to.1 - from.y) as f32) };
    let free = PagifyApp::object_turn(&rect, &grab, false);
    assert!((free - 33.0).abs() < 0.1, "{free}");
    assert_eq!(PagifyApp::object_turn(&rect, &grab, true), 30.0);
    // Swept the other way it reads as a turn the other way, not 340.
    let (s, c) = (-20.0f64).to_radians().sin_cos();
    let to = (centre.0 + 50.0 * s, centre.1 - 50.0 * c);
    let grab = Grab { handle: Some(Handle::Rotate), from, by: ((to.0 - from.x) as f32, (to.1 - from.y) as f32) };
    assert!((PagifyApp::object_turn(&rect, &grab, false) + 20.0).abs() < 0.1);
}
