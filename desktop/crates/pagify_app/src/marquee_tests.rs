//! The selection marquee is drawn on the page it is dragged on, and only there.

use super::ui_tests::harness;
use super::*;

/// How many marquees this frame drew: the faint violet fill (the outline is drawn
/// beside it, as a rectangle of its own).
fn marquees_drawn(h: &egui_kittest::Harness<'static, PagifyApp>) -> usize {
    let fill = theme::violet().gamma_multiply(0.08);
    h.output().shapes.iter().filter(|s| matches!(&s.shape, egui::Shape::Rect(r) if r.fill == fill)).count()
}

/// **Reported with a screenshot:** dragging a selection out on one page drew the same
/// rectangle at the same place on every page in view; nothing was selected on the
/// others. The corners are one page's coordinates and carried no page, so each
/// page's drawing pass used them with its own origin.
#[test]
fn a_marquee_is_drawn_on_its_own_page_and_not_on_the_others() {
    let mut h = harness("pages-ladder.pdf");
    h.state_mut().submit("editobject");
    // Far enough out that several pages are in view at once.
    h.state_mut().tab_mut().view_state.zoom = ZoomMode::Factor(0.3);
    h.run_steps(4);
    assert!(h.state().tab().doc.as_ref().unwrap().page_count >= 2, "test assumption: a document of several pages");

    let (a, b) = (AppPoint { x: 20.0, y: 20.0 }, AppPoint { x: 90.0, y: 70.0 });
    for page in [0, 1] {
        h.state_mut().tab_mut().selection.marquee = Some((page, a, b));
        h.run_steps(2);
        assert_eq!(marquees_drawn(&h), 1, "the marquee on page {page} was drawn on more than that page");
    }

    h.state_mut().tab_mut().selection.marquee = None;
    h.run_steps(2);
    assert_eq!(marquees_drawn(&h), 0, "a marquee was drawn with none being dragged");
}
