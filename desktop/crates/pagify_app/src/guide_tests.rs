use super::ui_tests::harness;
use super::*;

const GREEN: egui::Color32 = egui::Color32::from_rgb(0, 230, 0);

/// Two top-level objects with different left edges, and the first one as a selection.
fn two_objects(app: &mut PagifyApp) -> (Selected, pdf_core::document::Rect) {
    let layers: Vec<_> = app.layers_on(0).to_vec();
    let page = app.tab().doc.as_ref().unwrap().strip.size_of(0).unwrap();
    let things: Vec<_> = layers
        .iter()
        .filter(|o| o.depth == 0)
        .filter(|o| (o.rect.right - o.rect.left) > 5.0 && (o.rect.bottom - o.rect.top) > 5.0)
        .filter(|o| (o.rect.right - o.rect.left) < page.0 * 0.9)
        .collect();
    let a = things[0];
    let b = things.iter().find(|o| (o.rect.left - a.rect.left).abs() > 40.0).expect("a second object elsewhere");
    let sel = Selected { page: 0, object: a.object, rect: a.rect, what: "the shape" };
    (sel, b.rect)
}

#[test]
fn dragging_close_to_another_things_edge_lands_exactly_on_it() {
    let mut app = PagifyApp::new(Some(&ui_fixture("covered.pdf")));
    let (sel, other) = two_objects(&mut app);
    app.tab_mut().selection.selected = Some(sel.clone());
    // Where the drag would put its left edge: 3 points past the other thing's.
    let dx = other.left + 3.0 - sel.rect.left;
    app.tab_mut().selection.grab = Some(Grab { handle: None, from: AppPoint { x: 0.0, y: 0.0 }, by: (dx, 0.0) });

    app.snap_the_move(0, 1.0, true);

    let by = app.tab().selection.grab.as_ref().unwrap().by;
    let targets: Vec<f32> = {
        let t = app.guide_targets(0, &[sel.object]);
        t.iter().flat_map(|r| [r.left, (r.left + r.right) / 2.0, r.right]).chain([0.0]).collect()
    };
    let moved = PagifyApp::shifted(sel.rect, by);
    let sides = [moved.left, (moved.left + moved.right) / 2.0, moved.right];
    assert!(
        sides.iter().any(|s| targets.iter().any(|t| (s - t).abs() < 0.01)),
        "the thing was not pulled onto a line: sides {sides:?}, drag by {by:?}"
    );
    assert!((by.0 - dx).abs() <= 6.0 + 0.01, "pulled further than the snap allows: {} for {}", by.0, dx);
}

#[test]
fn holding_alt_shows_the_lines_but_pulls_nothing() {
    let mut app = PagifyApp::new(Some(&ui_fixture("covered.pdf")));
    let (sel, other) = two_objects(&mut app);
    app.tab_mut().selection.selected = Some(sel.clone());
    let dx = other.left + 3.0 - sel.rect.left;
    app.tab_mut().selection.grab = Some(Grab { handle: None, from: AppPoint { x: 0.0, y: 0.0 }, by: (dx, 0.0) });
    app.snap_the_move(0, 1.0, false);
    assert_eq!(app.tab().selection.grab.as_ref().unwrap().by, (dx, 0.0));
}

#[test]
fn a_resize_is_not_pulled_onto_anything() {
    let mut app = PagifyApp::new(Some(&ui_fixture("covered.pdf")));
    let (sel, other) = two_objects(&mut app);
    app.tab_mut().selection.selected = Some(sel.clone());
    let dx = other.left + 3.0 - sel.rect.left;
    app.tab_mut().selection.grab = Some(Grab { handle: Some(Handle::Right), from: AppPoint { x: 0.0, y: 0.0 }, by: (dx, 0.0) });
    app.snap_the_move(0, 1.0, true);
    assert_eq!(app.tab().selection.grab.as_ref().unwrap().by, (dx, 0.0));
}

/// The lines of this frame, by colour: how many green ones were drawn.
fn lines_drawn(h: &egui_kittest::Harness<'static, PagifyApp>) -> (usize, usize) {
    let (mut grey, mut green) = (0, 0);
    for s in &h.output().shapes {
        if let egui::Shape::LineSegment { points, stroke } = &s.shape {
            // A reference line runs most of the page; ignore every short stroke.
            if (points[0] - points[1]).length() < 150.0 {
                continue;
            }
            if stroke.color == GREEN {
                green += 1;
            } else if stroke.color == egui::Color32::from_rgba_unmultiplied(120, 130, 150, 190) {
                grey += 1;
            }
        }
    }
    (grey, green)
}

#[test]
fn the_lines_are_drawn_only_while_something_is_being_moved() {
    let mut h = harness("covered.pdf");
    let (sel, other) = {
        let app = h.state_mut();
        two_objects(app)
    };
    h.state_mut().submit("editobject");
    h.state_mut().tab_mut().selection.selected = Some(sel.clone());
    h.run_steps(3);
    assert_eq!(lines_drawn(&h), (0, 0), "lines were drawn for a selection that is not being moved");

    // Dragged so its left edge is on the other thing's: a green line.
    let dx = other.left - sel.rect.left;
    h.state_mut().tab_mut().selection.grab = Some(Grab { handle: None, from: AppPoint { x: 0.0, y: 0.0 }, by: (dx, 0.0) });
    h.run_steps(2);
    let (_, green) = lines_drawn(&h);
    assert!(green >= 1, "no green line while a thing sits on another's edge");

    // Let go: gone again.
    h.state_mut().tab_mut().selection.grab = None;
    h.run_steps(2);
    assert_eq!(lines_drawn(&h), (0, 0), "the lines stayed after the move");

    // And a resize shows none.
    h.state_mut().tab_mut().selection.grab = Some(Grab { handle: Some(Handle::Right), from: AppPoint { x: 0.0, y: 0.0 }, by: (dx, 0.0) });
    h.run_steps(2);
    assert_eq!(lines_drawn(&h), (0, 0), "lines were drawn for a resize");
}

fn ui_fixture(name: &str) -> String {
    super::ui_tests::fixture(name)
}
