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

fn shown(app: &PagifyApp) -> std::sync::Arc<egui::Galley> {
    app.tab().edit.editing_run.as_ref().and_then(|e| e.shown.as_ref()).map(|(galley, _)| galley.clone()).expect("the editor was drawn")
}

#[test]
fn an_unchanged_box_draws_no_paper_and_no_ink() {
    let mut h = harness_from(opened(open_page("inplace-invisible", &LINES, "")));
    h.run_steps(3);
    assert_eq!(h.state().editor_cover(0), None, "paper over the page's own words");
    let galley = shown(h.state());
    let vertices: Vec<_> = galley.rows.iter().flat_map(|r| r.row.visuals.mesh.vertices.iter()).collect();
    assert!(!vertices.is_empty(), "nothing was laid out, so this proves nothing");
    assert!(vertices.iter().all(|v| v.color.a() == 0), "the editor draws letters over the page's own");
}

#[test]
fn typed_words_are_drawn_once_the_textless_picture_is_there() {
    let mut app = opened(open_page("inplace-typed-ink", &LINES, ""));
    type_a_letter(&mut app);
    let mut h = harness_from(app);
    h.run_steps(4);
    assert!(textless(h.state()).is_some());
    assert_eq!(h.state().editor_cover(0), None);
    let galley = shown(h.state());
    assert!(galley.rows.iter().flat_map(|r| r.row.visuals.mesh.vertices.iter()).any(|v| v.color.a() > 0), "typed words are not drawn");
}

#[test]
fn until_the_picture_is_there_the_paper_still_covers() {
    let mut h = async_from(opened(open_page("inplace-cover", &LINES, "")));
    type_a_letter(h.state_mut());
    h.run_steps(1);
    assert!(textless(h.state()).is_none(), "test assumption: the worker has not answered yet");
    let background = h.state().tab().edit.editing_run.as_ref().expect("open").background;
    assert_eq!(h.state().editor_cover(0), Some(egui::Color32::from_rgb(background.r, background.g, background.b)));
}

#[test]
fn baselines_lie_inside_their_lines() {
    let app = opened(open_page("inplace-baselines", &LINES, ""));
    let edit = app.tab().edit.editing_run.as_ref().expect("an editor");
    assert_eq!(edit.baselines.len(), edit.lines.len());
    for ((_, rect), baseline) in edit.lines.iter().zip(&edit.baselines) {
        assert!(*baseline > rect.top && *baseline <= rect.bottom, "baseline {baseline} outside {rect:?}");
    }
    // The page draws them 12 pt apart.
    assert!((edit.baselines[1] - edit.baselines[0] - 12.0).abs() < 0.1, "{:?}", edit.baselines);
}

#[test]
fn each_row_sits_on_its_lines_baseline() {
    let mut h = harness_from(opened(open_page("inplace-rows", &LINES, "")));
    h.run_steps(3);
    let view = h.state().tab().view_state.last_view.expect("the page was drawn");
    let edit = h.state().tab().edit.editing_run.as_ref().expect("an editor");
    let (galley, at) = edit.shown.clone().expect("drawn");
    assert_eq!(galley.rows.len(), 3);
    for (row, baseline) in galley.rows.iter().zip(&edit.baselines) {
        let drawn = at.y + row.pos.y + row.row.glyphs[0].pos.y;
        let wanted = view.to_screen(AppPoint { x: 0.0, y: *baseline as f64 }).y;
        assert!((drawn - wanted).abs() < 1.0, "baseline drawn at {drawn}, the page's at {wanted}");
    }
}

#[test]
fn a_click_lands_on_the_letter_the_page_shows_there() {
    let mut h = harness_from(opened(open_page("inplace-caret", &LINES, "")));
    h.run_steps(3);
    let view = h.state().tab().view_state.last_view.expect("the page was drawn");
    let edit = h.state().tab().edit.editing_run.as_ref().expect("an editor");
    let (galley, at) = edit.shown.clone().expect("drawn");
    // The page's own line "delta epsilon zeta": a click at the left edge of "zeta" (letter 14).
    let line = &edit.lines[1].1;
    let letters = 18.0;
    let x = view.to_screen(AppPoint { x: (line.left + (line.right - line.left) * 14.0 / letters) as f64, y: 0.0 }).x;
    let y = view.to_screen(AppPoint { x: 0.0, y: ((line.top + line.bottom) / 2.0) as f64 }).y;
    let cursor = galley.cursor_from_pos(egui::pos2(x, y) - at);
    let index = cursor.index.0;
    let expected = "alpha beta gamma
".chars().count() + 14;
    assert!(index.abs_diff(expected) <= 2, "clicked letter 14 of line 2 (index {expected}), the caret went to {index}");
}

/// Three lines the page draws with 3 pt between the letters, which the editor's own font
/// has no way to match without being told the width.
const SPACED: &str = "BT /F1 10 Tf 3 Tc 100 700 Td (alpha beta gamma) Tj ET
                      BT /F1 10 Tf 3 Tc 100 688 Td (delta epsilon zeta) Tj ET
                      BT /F1 10 Tf 3 Tc 100 676 Td (eta theta iota) Tj ET
";

#[test]
fn each_row_spans_the_width_of_the_pages_line() {
    each_row_matches_the_page(open_page("inplace-widths", &LINES, ""));
}

#[test]
fn each_row_spans_a_line_the_page_draws_with_spaced_letters() {
    each_row_matches_the_page(open_page("inplace-widths-spaced", &[], SPACED));
}

fn each_row_matches_the_page(app: PagifyApp) {
    let mut h = harness_from(opened(app));
    h.run_steps(3);
    let view = h.state().tab().view_state.last_view.expect("the page was drawn");
    let edit = h.state().tab().edit.editing_run.as_ref().expect("an editor");
    let (galley, at) = edit.shown.clone().expect("drawn");
    for (row, (_, line)) in galley.rows.iter().zip(&edit.lines) {
        let (first, last) = (row.row.glyphs.first().expect("a letter"), row.row.glyphs.last().expect("a letter"));
        let (left, right) = (at.x + row.pos.x + first.pos.x, at.x + row.pos.x + last.pos.x + last.advance_width);
        let page_left = view.to_screen(AppPoint { x: line.left as f64, y: 0.0 }).x;
        let page_right = view.to_screen(AppPoint { x: line.right as f64, y: 0.0 }).x;
        assert!((left - page_left).abs() < 1.0, "the line starts at {left}, the page's at {page_left}");
        assert!((right - page_right).abs() < 1.0, "the line ends at {right}, the page's at {page_right}");
    }
}

/// The zoom the canvas drew the page at, which is what a settling look is held against.
fn canvas_zoom(h: &egui_kittest::Harness<'static, PagifyApp>) -> f32 {
    h.state().tab().view_state.last_view.expect("the page was drawn").scale
}

/// The box typed in, with its page-without-the-words under it and its look remembered.
fn typed_and_settled() -> egui_kittest::Harness<'static, PagifyApp> {
    let mut app = opened(open_page("inplace-settling", &LINES, ""));
    type_a_letter(&mut app);
    let mut h = async_from(app);
    for _ in 0..60 {
        if textless(h.state()).is_some() {
            break;
        }
        settle(&mut h, 1);
    }
    assert!(textless(h.state()).is_some(), "the picture never arrived");
    settle(&mut h, 3);
    h
}

fn settling_now(h: &mut egui_kittest::Harness<'static, PagifyApp>, zoom: f32, has_picture: bool) -> bool {
    let ctx = h.ctx.clone();
    h.state_mut().settling_for(&ctx, 0, zoom, has_picture).is_some()
}

#[test]
fn after_apply_the_typed_look_stays_until_the_page_is_drawn_again() {
    let mut h = typed_and_settled();
    let zoom = canvas_zoom(&h);
    h.state_mut().apply_editing_page();
    assert!(h.state().tab().edit.editing_run.is_none(), "test assumption: the box closed");
    assert!(settling_now(&mut h, zoom, false), "the typed words must stay up while the page has no new picture");
    // And the frames that follow draw it, until the page's own new picture lands - or, on a
    // machine too slow for that, until the held look times out (3 s of frames at 60 a second).
    for _ in 0..250 {
        if h.state().tab().edit.settling.is_none() {
            return;
        }
        settle(&mut h, 1);
    }
    panic!("the typed look was never let go of");
}

#[test]
fn a_picture_of_the_page_ends_it() {
    let mut h = typed_and_settled();
    let zoom = canvas_zoom(&h);
    h.state_mut().apply_editing_page();
    assert!(!settling_now(&mut h, zoom, true), "the page has its own picture again");
    assert!(h.state().tab().edit.settling.is_none());
}

#[test]
fn a_zoom_ends_it() {
    let mut h = typed_and_settled();
    let zoom = canvas_zoom(&h);
    h.state_mut().apply_editing_page();
    assert!(!settling_now(&mut h, zoom * 1.5, false), "rows drawn for another zoom would be misplaced");
    assert!(h.state().tab().edit.settling.is_none());
}

#[test]
fn a_box_put_away_with_no_edit_leaves_nothing_behind() {
    let mut h = typed_and_settled();
    let zoom = canvas_zoom(&h);
    h.state_mut().tab_mut().edit.editing_run = None;
    assert!(!settling_now(&mut h, zoom, false), "nothing was applied, so the page is as it was");
}
