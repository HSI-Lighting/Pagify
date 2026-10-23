//! Editing a page's text where it is: the page without the paragraph goes under the
//! editor once the words change (`PagifyApp::textless_for`), so the old words do not
//! show through the new ones. The engine's half is `rust/pdf_core/tests/render_hiding.rs`.

use super::wrap_hyphen_tests::open_page;
use super::*;
use crate::ui_tests::harness_from;
use eframe::App as _;

/// A three-line paragraph.
const LINES: [(f32, f32, &str); 3] = [
    (100.0, 700.0, "alpha beta gamma"),
    (100.0, 688.0, "delta epsilon zeta"),
    (100.0, 676.0, "eta theta iota"),
];

/// `app` with Edit Text in hand and the paragraph open.
fn opened(mut app: PagifyApp) -> PagifyApp {
    let run = app.tab().doc.as_ref().expect("open").session.text_runs(0).expect("runs").into_iter().find(|r| r.text.contains("delta")).expect("a line");
    app.submit("edittext");
    let at = AppPoint { x: ((run.rect.left + run.rect.right) / 2.0) as f64, y: ((run.rect.top + run.rect.bottom) / 2.0) as f64 };
    app.pick_text_run(0, at).expect("picked");
    assert!(app.tab().edit.editing_run.is_some(), "the paragraph did not open");
    app
}

fn type_a_letter(app: &mut PagifyApp) {
    app.tab_mut().edit.editing_run.as_mut().expect("an editor").buffer.push('x');
}

fn textless(app: &PagifyApp) -> Option<&crate::caches::Textless> {
    app.tab().doc.as_ref().and_then(|d| d.caches.textless.as_ref())
}

/// The app with the render worker on, which is how it runs; frames with real time between
/// them, because the worker is a real thread.
fn async_from(mut app: PagifyApp) -> egui_kittest::Harness<'static, PagifyApp> {
    app.render_state.async_render = true;
    let mut h = egui_kittest::Harness::builder()
        .with_size(egui::vec2(1400.0, 1000.0))
        .with_step_dt(1.0 / 60.0)
        .build_ui_state(
            |ui, app: &mut PagifyApp| {
                let mut frame = eframe::Frame::_new_kittest();
                app.ui(ui, &mut frame);
            },
            app,
        );
    settle(&mut h, 30);
    h
}

fn settle(h: &mut egui_kittest::Harness<'static, PagifyApp>, frames: usize) {
    for _ in 0..frames {
        std::thread::sleep(std::time::Duration::from_millis(10));
        h.run_steps(1);
    }
}

#[test]
fn an_unchanged_paragraph_asks_for_no_textless_picture() {
    let mut h = harness_from(opened(open_page("inplace-unchanged", &LINES, "")));
    h.run_steps(3);
    assert!(textless(h.state()).is_none(), "the page's own picture is what shows until the words change");
}

#[test]
fn typing_brings_the_textless_picture_and_it_hides_exactly_the_paragraph() {
    let mut app = opened(open_page("inplace-typed", &LINES, ""));
    type_a_letter(&mut app);
    let mut h = harness_from(app);
    h.run_steps(3);
    let edit = h.state().tab().edit.editing_run.as_ref().expect("still open");
    let picture = textless(h.state()).expect("a textless picture was made");
    assert_eq!(picture.objects, edit.hidden_objects());
    assert_eq!(picture.page, 0);
    assert!(picture.objects.len() >= 3, "all three lines are hidden: {:?}", picture.objects);
}

#[test]
fn the_textless_picture_hides_the_lines_and_their_twins_not_the_frozen_lines() {
    let app = opened(open_page("inplace-hidden-set", &LINES, ""));
    let mut edit = app.tab().edit.editing_run.as_ref().expect("an editor").clone();
    let rect = edit.rect;
    edit.lines = vec![(vec![4, 3], rect), (vec![9], rect), (vec![], rect)];
    edit.frozen = vec![false, true, true];
    edit.twins = vec![vec![7, 3], vec![8], vec![]];
    assert_eq!(edit.hidden_objects(), vec![3, 4, 7], "the first line, its twins, sorted and counted once");
}

#[test]
fn an_edit_elsewhere_drops_the_textless_picture() {
    let mut app = opened(open_page("inplace-stale", &LINES, ""));
    type_a_letter(&mut app);
    let mut h = harness_from(app);
    h.run_steps(3);
    assert!(textless(h.state()).is_some());
    h.state_mut().tab_mut().doc.as_mut().expect("open").rendered_is_stale();
    assert!(textless(h.state()).is_none(), "a picture of the page as it was must not outlive an edit");
    h.run_steps(3);
    let doc = h.state().tab().doc.as_ref().expect("open");
    assert!(doc.caches.textless.as_ref().map_or(true, |t| t.epoch == doc.render_epoch), "a picture from before the edit came back");
}

#[test]
fn a_heavy_page_keeps_the_cover_and_asks_for_no_textless_picture() {
    let mut shapes = String::new();
    for _ in 0..HEAVY_PAGE_OBJECTS + 100 {
        shapes.push_str("0 0 1 1 re f\n");
    }
    let mut app = open_page("inplace-heavy", &LINES, &shapes);
    assert!(crate::page_is_heavy(&app.page_weight(0).expect("counted")), "test assumption: the page is heavy");
    app.submit("edittext");
    let run = app.tab().doc.as_ref().expect("open").session.text_runs(0).expect("runs").into_iter().find(|r| r.text.contains("delta")).expect("a line");
    let at = AppPoint { x: ((run.rect.left + run.rect.right) / 2.0) as f64, y: ((run.rect.top + run.rect.bottom) / 2.0) as f64 };
    let _ = app.pick_text_run(0, at);
    if app.tab().edit.editing_run.is_none() {
        return; // a heavy page may refuse the paragraph altogether; then there is no cover to keep
    }
    type_a_letter(&mut app);
    let mut h = harness_from(app);
    h.run_steps(3);
    assert!(textless(h.state()).is_none(), "a textless render holds the one PDFium lock for seconds on a page like this");
}

#[test]
fn zooming_with_the_box_open_still_hides_the_old_words() {
    let mut app = opened(open_page("inplace-zoom", &LINES, ""));
    type_a_letter(&mut app);
    let mut h = async_from(app);
    for _ in 0..40 {
        if textless(h.state()).is_some() {
            break;
        }
        settle(&mut h, 1);
    }
    assert!(textless(h.state()).is_some(), "the picture never arrived");
    let zoom = h.state().resolved_zoom();
    h.state_mut().tab_mut().view_state.zoom = ZoomMode::Factor(zoom * 1.5);
    for frame in 0..12 {
        settle(&mut h, 1);
        assert!(textless(h.state()).is_some(), "the old words showed through on frame {frame} after the zoom");
    }
}

#[test]
fn closing_the_box_drops_the_picture() {
    let mut app = opened(open_page("inplace-closed", &LINES, ""));
    type_a_letter(&mut app);
    let mut h = harness_from(app);
    h.run_steps(3);
    assert!(textless(h.state()).is_some());
    h.state_mut().tab_mut().edit.editing_run = None;
    h.run_steps(2);
    assert!(textless(h.state()).is_none());
}
