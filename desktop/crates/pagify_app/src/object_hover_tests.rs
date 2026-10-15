//! Edit Object boxes the one thing a click would pick, and a double click on words in it
//! opens them for retyping, the way Edit Text does.

use super::ui_tests::harness_60fps;
use super::wrap_hyphen_tests::open_page;
use super::*;
use crate::overlay::PageView;

const WORDS: &[(f32, f32, &str)] = &[(100.0, 700.0, "first line of the words"), (100.0, 688.0, "second line of the words")];
/// A blue square well away from the words.
const SQUARE: &str = "0.2 0.4 0.8 rg 300 300 100 50 re f\n";

fn view() -> PageView {
    PageView { origin: egui::Pos2::new(0.0, 0.0), scale: 1.0 }
}

fn centre(r: &pdf_core::document::Rect) -> AppPoint {
    AppPoint { x: ((r.left + r.right) / 2.0) as f64, y: ((r.top + r.bottom) / 2.0) as f64 }
}

/// The first line's words, and the square, where the page has them.
fn words_and_square(app: &PagifyApp) -> (pdf_core::document::Rect, pdf_core::document::Rect) {
    let session = &app.tab().doc.as_ref().expect("open").session;
    let words = session.text_run_rects(0).expect("runs").into_iter().map(|(_, r)| r).next().expect("a run");
    let square = session
        .drawn_objects(0)
        .expect("drawn")
        .into_iter()
        .find(|d| d.kind == pdf_core::document::DrawnKind::Shape)
        .expect("the square")
        .rect;
    (words, square)
}

/// **With Edit Object in hand the thing under the pointer is boxed**, so what a click
/// will pick is something to see rather than to find out. The box is the very rectangle
/// `thing_at` gives a click, so the two cannot disagree.
#[test]
fn edit_object_boxes_the_thing_under_the_pointer_and_nothing_else() {
    let mut app = open_page("hover-box", WORDS, SQUARE);
    let (words, square) = words_and_square(&app);
    let screen = |at: AppPoint| view().to_screen(at);

    // Not until Edit Object is in hand.
    assert!(app.object_hover_box(0, view(), screen(centre(&words))).is_none(), "a box with no tool armed");

    app.submit("editobject");
    let over_words = app.object_hover_box(0, view(), screen(centre(&words))).expect("words under the pointer are boxed");
    assert!((over_words.left() - words.left).abs() < 0.5 && (over_words.right() - words.right).abs() < 0.5, "{over_words:?} vs {words:?}");
    let over_square = app.object_hover_box(0, view(), screen(centre(&square))).expect("a shape under the pointer is boxed");
    assert!((over_square.left() - square.left).abs() < 0.5 && (over_square.bottom() - square.bottom).abs() < 0.5, "{over_square:?} vs {square:?}");

    // Bare paper: nothing.
    let paper = AppPoint { x: 500.0, y: 100.0 };
    assert!(app.thing_at(0, paper, true).is_none(), "test assumption: nothing is drawn there");
    assert!(app.object_hover_box(0, view(), screen(paper)).is_none(), "bare paper was boxed");

    // Edit Text has boxes of its own; Edit Object's are not drawn over them.
    app.submit("edittext");
    assert!(app.object_hover_box(0, view(), screen(centre(&words))).is_none(), "boxed while Edit Text was in hand");
}

/// While something is being dragged the pointer is the drag's, not a hover.
#[test]
fn nothing_is_boxed_while_a_thing_is_being_dragged() {
    let mut app = open_page("hover-drag", WORDS, SQUARE);
    let (_, square) = words_and_square(&app);
    app.submit("editobject");
    let at = view().to_screen(centre(&square));
    assert!(app.object_hover_box(0, view(), at).is_some(), "test assumption: boxed when still");

    app.tab_mut().selection.grab = Some(Grab { handle: None, from: centre(&square), by: (0.0, 0.0) });
    assert!(app.object_hover_box(0, view(), at).is_none(), "boxed during a move");
    app.tab_mut().selection.grab = None;
    app.tab_mut().selection.marquee = Some((0, centre(&square), centre(&square)));
    assert!(app.object_hover_box(0, view(), at).is_none(), "boxed during a marquee");
}

/// A page too heavy to read is not boxed, as with Edit Text: finding the thing under the
/// pointer reads every shape on it, and that on every frame would stand the window still.
#[test]
fn a_page_too_heavy_to_read_is_not_boxed() {
    let mut shapes = String::from(SQUARE);
    for _ in 0..HEAVY_PAGE_OBJECTS + 100 {
        shapes.push_str("0 0 1 1 re f\n");
    }
    let mut app = open_page("hover-heavy", WORDS, &shapes);
    assert!(crate::page_is_heavy(&app.page_weight(0).expect("counted")), "test assumption: the page is heavy");
    let (words, _) = words_and_square(&app);
    app.submit("editobject");
    assert!(app.object_hover_box(0, view(), view().to_screen(centre(&words))).is_none(), "a heavy page was boxed");
}

/// **What a page can pick is read once for the page as it is, not on every frame the
/// pointer moves.** Each read takes the one lock over PDFium; measured on real pages a box
/// cost 39 ms (VIVO) and 85 ms (the datasheet) a frame before this was kept.
#[test]
fn the_page_is_read_once_while_the_pointer_moves_over_it() {
    let mut app = open_page("hover-read-once", WORDS, SQUARE);
    let (words, square) = words_and_square(&app);
    app.submit("editobject");
    let held = |app: &PagifyApp| {
        app.tab().doc.as_ref().unwrap().caches.hits.borrow().as_ref().map(|(_, hits)| std::rc::Rc::as_ptr(hits))
    };
    app.object_hover_box(0, view(), view().to_screen(centre(&words)));
    let first = held(&app);
    assert!(first.is_some(), "nothing was kept");
    app.object_hover_box(0, view(), view().to_screen(centre(&square)));
    assert_eq!(held(&app), first, "the page was read again");
}

/// **A thing that moved is boxed where it went**: what was kept is for the page as it was.
#[test]
fn the_box_follows_a_thing_that_was_moved() {
    let mut app = open_page("hover-moved", WORDS, SQUARE);
    let (_, square) = words_and_square(&app);
    let object = app.tab().doc.as_ref().unwrap().session.drawn_objects(0).unwrap()
        .into_iter().find(|d| d.kind == pdf_core::document::DrawnKind::Shape).unwrap().object;
    app.submit("editobject");
    let was = centre(&square);
    assert!(app.object_hover_box(0, view(), view().to_screen(was)).is_some(), "test assumption: boxed where it is");

    app.move_object_by(0, object, "the shape", (100.0, 0.0)).expect("moved");
    let now = AppPoint { x: was.x + 100.0, y: was.y };
    assert!(app.object_hover_box(0, view(), view().to_screen(was)).is_none(), "boxed where it was");
    let boxed = app.object_hover_box(0, view(), view().to_screen(now)).expect("not boxed where it went");
    assert!((boxed.left() - (square.left + 100.0)).abs() < 0.5, "{boxed:?}");
}

/// Two clicks a frame or two apart, as a hand makes them.
fn double_click(h: &mut egui_kittest::Harness<'static, PagifyApp>, at: egui::Pos2) {
    use egui::{Event, PointerButton};
    h.input_mut().events.push(Event::PointerMoved(at));
    h.run_steps(2);
    for _ in 0..2 {
        for pressed in [true, false] {
            h.input_mut().events.push(Event::PointerButton { pos: at, button: PointerButton::Primary, pressed, modifiers: Default::default() });
            h.run_steps(1);
        }
    }
    h.run_steps(2);
}

/// The words the editor opened with, or None when none is open.
fn editor_words(h: &egui_kittest::Harness<'static, PagifyApp>) -> Option<String> {
    h.state().tab().edit.editing_run.as_ref().map(|e| e.buffer.clone())
}

fn screen_of(h: &egui_kittest::Harness<'static, PagifyApp>, at: AppPoint) -> egui::Pos2 {
    h.state().tab().view_state.last_view.expect("the page was never drawn").to_screen(at)
}

fn path_of(name: &str) -> String {
    std::env::temp_dir().join("pagify-app-tests").join(format!("wrap-hyphen-{name}.pdf")).to_string_lossy().into_owned()
}

/// **A double click on words in Edit Object opens the editor Edit Text would open there**
/// — the same paragraph, not a letter of it (the click that began the double click has
/// already split the run into letters), and Edit Text is then the tool in hand.
#[test]
fn a_double_click_on_words_in_edit_object_opens_what_edit_text_opens() {
    // What a click in Edit Text opens on this page.
    let mut plain = open_page("hover-dbl-plain", WORDS, SQUARE);
    let (words, _) = words_and_square(&plain);
    plain.submit("edittext");
    plain.pick_text_run(0, centre(&words)).expect("Edit Text opens the words");
    let expected = plain.tab().edit.editing_run.as_ref().expect("an editor").buffer.clone();
    assert!(expected.contains("second line"), "test assumption: the page's two lines are one paragraph: {expected:?}");

    drop(open_page("hover-dbl", WORDS, SQUARE));
    let mut h = harness_60fps(&path_of("hover-dbl"), false);
    h.state_mut().submit("editobject");
    h.run_steps(2);
    let at = screen_of(&h, centre(&words));
    double_click(&mut h, at);

    assert_eq!(editor_words(&h).as_deref(), Some(expected.as_str()), "the editor is not the one Edit Text opens");
    assert!(h.state().tab().tool_state.object_tool.is_none(), "Edit Object is still in hand");

    // Letting go of the editor by a click away lands in Edit Text, as after any Edit Text pick.
    let away = screen_of(&h, AppPoint { x: 500.0, y: 100.0 });
    super::ui_tests::click(&mut h, away);
    assert!(editor_words(&h).is_none(), "the editor is still open after a click away");
    assert!(h.state().tab().tool.as_ref().is_some_and(|t| matches!(t.kind, Tool::PickText)), "Edit Text is not the tool in hand");
    assert!(h.state().tab().tool_state.object_tool.is_none(), "Edit Object came back");
}

/// A double click on a shape opens nothing, and clicks on words one at a time only select them.
#[test]
fn a_double_click_on_a_shape_or_clicks_on_words_one_at_a_time_open_no_editor() {
    drop(open_page("hover-dbl-no", WORDS, SQUARE));
    let mut h = harness_60fps(&path_of("hover-dbl-no"), false);
    h.state_mut().submit("editobject");
    h.run_steps(2);
    let (words, square) = words_and_square(h.state());

    let at = screen_of(&h, centre(&square));
    double_click(&mut h, at);
    assert!(h.state().tab().selection.selected.is_some(), "test assumption: the shape was selected");
    assert!(editor_words(&h).is_none(), "a double click on a shape opened an editor");
    assert!(h.state().tab().tool_state.object_tool.is_some(), "Edit Object was put down");

    // egui counts a double only within a few pixels of the click before, so this is one click.
    let at = screen_of(&h, centre(&words));
    super::ui_tests::click(&mut h, at);
    assert!(h.state().tab().selection.selected.as_ref().is_some_and(|s| s.is_text()), "test assumption: the click selected the words");
    assert!(editor_words(&h).is_none(), "one click opened an editor");

    // Once more, after longer than egui's 0.3 s for a double: the words are selected
    // already, and this click only selects them again.
    h.run_steps(30);
    super::ui_tests::click(&mut h, at);
    assert!(editor_words(&h).is_none(), "a click on words already selected opened an editor");
}
