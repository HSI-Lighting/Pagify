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

/// Frames until `ready` holds, or ten seconds of real time. The render worker is a real thread
/// and takes the one PDFium lock that every other test running at the same time shares, so how
/// long a picture takes depends on the load; what is asserted is that it arrives.
fn wait_until(h: &mut egui_kittest::Harness<'static, PagifyApp>, ready: impl Fn(&PagifyApp) -> bool) -> bool {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    while std::time::Instant::now() < deadline {
        if ready(h.state()) {
            return true;
        }
        settle(h, 1);
    }
    ready(h.state())
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
    assert!(wait_until(&mut h, |app| textless(app).is_some()), "the picture never arrived");
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

/// Where the page's own letters of `text` end, in screen px: the box of the last one. **The page's
/// letters, not the text object's box**, which can run well past them (a table cell's padding).
fn page_ink_right_px(h: &mut egui_kittest::Harness<'static, PagifyApp>, text: &str) -> f32 {
    let view = h.state().tab().view_state.last_view.expect("the page was drawn");
    let chars = h.state_mut().characters(0).expect("characters").clone();
    let range = chars.find(text.trim()).first().cloned().unwrap_or_else(|| panic!("{text:?} is not in the page's letters"));
    let last = range.end - 1;
    let right = chars.line_rects(last..last + 1).first().map_or(0.0, |r| r.right);
    view.to_screen(AppPoint { x: right as f64, y: 0.0 }).x
}

/// The last letter (not a space) of row `row` of the editor's text, its right edge on screen.
fn last_letter_right(galley: &egui::Galley, at: egui::Pos2, row: usize) -> f32 {
    let placed = &galley.rows[row];
    let glyph = placed.row.glyphs.iter().rev().find(|g| !g.chr.is_whitespace()).expect("a letter");
    at.x + placed.pos.x + glyph.pos.x + glyph.advance_width
}

fn each_row_matches_the_page(app: PagifyApp) {
    let mut h = harness_from(opened(app));
    h.run_steps(3);
    let view = h.state().tab().view_state.last_view.expect("the page was drawn");
    let (galley, at, lines, buffer) = {
        let edit = h.state().tab().edit.editing_run.as_ref().expect("an editor");
        let (galley, at) = edit.shown.clone().expect("drawn");
        (galley, at, edit.lines.clone(), edit.buffer.clone())
    };
    for (row, ((_, line), text)) in lines.iter().zip(buffer.split('\n')).enumerate() {
        let first = galley.rows[row].row.glyphs.first().expect("a letter");
        let left = at.x + galley.rows[row].pos.x + first.pos.x;
        let page_left = view.to_screen(AppPoint { x: line.left as f64, y: 0.0 }).x;
        assert!((left - page_left).abs() < 1.0, "the line starts at {left}, the page's at {page_left}");
        let (right, page_right) = (last_letter_right(&galley, at, row), page_ink_right_px(&mut h, text));
        assert!((right - page_right).abs() < 1.0, "the line's last letter ends at {right}, the page's at {page_right}");
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
    assert!(wait_until(&mut h, |app| textless(app).is_some()), "the picture never arrived");
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

/// A page's drawn (frozen) line stays on the page as shapes. Once anything is typed the whole
/// box is visible, and a frozen line's row would be painted in the editor's font over those
/// shapes: its row must stay invisible, and only its row.
#[test]
fn a_frozen_line_is_not_drawn_over_the_pages_shapes_once_typed() {
    let mut app = opened(open_page("inplace-frozen", &LINES, ""));
    app.tab_mut().edit.editing_run.as_mut().expect("an editor").frozen = vec![false, true, false];
    type_a_letter(&mut app);
    let mut h = harness_from(app);
    h.run_steps(4);
    let galley = shown(h.state());
    let visible = |row: usize| galley.rows[row].row.visuals.mesh.vertices.iter().any(|v| v.color.a() > 0);
    assert!(visible(0) && visible(2), "the lines the box owns are drawn");
    assert!(!visible(1), "the frozen line is painted over the page's own shapes");
}

/// An undo, or an edit elsewhere, closes the open box because the page moved on: its typed words
/// were never applied, so nothing of them may be left up while the page is drawn again.
#[test]
fn a_box_closed_because_the_page_changed_leaves_no_look_behind() {
    let mut h = typed_and_settled();
    let zoom = canvas_zoom(&h);
    h.state_mut().tab_mut().doc.as_mut().expect("open").rendered_is_stale();
    h.run_steps(1);
    assert!(h.state().tab().edit.editing_run.is_none(), "test assumption: the stale box was closed");
    assert!(h.state().tab().edit.settling.is_none(), "unapplied words were kept up as if applied");
    assert!(!settling_now(&mut h, zoom, false));
}

/// Three lines whose text boxes run 50 pt past their letters, as a table cell's padding does on the
/// datasheet: a gap, then a trailing space, after the words; the box includes both and the letters neither.
const PADDED: &str = "BT /F1 10 Tf 100 700 Td [(alpha beta gamma) -5000 ( )] TJ ET\n\
                      BT /F1 10 Tf 100 688 Td [(delta epsilon zeta) -5000 ( )] TJ ET\n\
                      BT /F1 10 Tf 100 676 Td [(eta theta iota) -5000 ( )] TJ ET\n";

/// The row must end where the page's *letters* end. Spread to the text object's box instead, the
/// caret sits past the last letter and a click or a drag lands on letters a few to the left of the
/// ones pointed at: here, where the datasheet's box ran 24 px past its words.
#[test]
fn each_row_ends_where_the_pages_letters_end_not_where_their_box_does() {
    let mut h = harness_from(opened(open_page("inplace-padded", &[], PADDED)));
    h.run_steps(3);
    let view = h.state().tab().view_state.last_view.expect("the page was drawn");
    let (galley, at, lines, buffer) = {
        let edit = h.state().tab().edit.editing_run.as_ref().expect("an editor");
        let (galley, at) = edit.shown.clone().expect("drawn");
        (galley, at, edit.lines.clone(), edit.buffer.clone())
    };
    for (row, ((_, line), text)) in lines.iter().zip(buffer.split('\n')).enumerate() {
        let (right, ink) = (last_letter_right(&galley, at, row), page_ink_right_px(&mut h, text));
        let boxed = view.to_screen(AppPoint { x: line.right as f64, y: 0.0 }).x;
        assert!(boxed - ink > 20.0, "test assumption: the box runs past the letters ({boxed} against {ink})");
        assert!((right - ink).abs() < 1.0, "the row's last letter ends at {right}; the page's letters end at {ink} (the box at {boxed})");
    }
}

/// Whether `shape` is text with a letter in `area` that can be seen. The selection's own highlight
/// (`highlight`) is part of the same mesh and is meant to show; a letter in any other colour is not.
fn shows_text_inside(shape: &egui::epaint::Shape, area: egui::Rect, highlight: egui::Color32) -> bool {
    match shape {
        egui::epaint::Shape::Text(text) => {
            area.contains(text.pos)
                && text.galley.rows.iter().any(|r| r.row.visuals.mesh.vertices.iter().any(|v| v.color.a() > 0 && v.color != highlight))
        }
        egui::epaint::Shape::Vec(shapes) => shapes.iter().any(|s| shows_text_inside(s, area, highlight)),
        _ => false,
    }
}

/// egui paints selected letters in the selection's text colour, whatever colour they were given. The
/// letters of a box that is still the page's own words are given none, so a selection must not bring
/// them back: they would be drawn over the page's own, a little aside and a little tight.
#[test]
fn selecting_in_an_unchanged_box_does_not_show_its_invisible_letters() {
    let mut h = harness_from(opened(open_page("inplace-selection", &LINES, "")));
    h.run_steps(3);
    let (id, letters, area) = {
        let edit = h.state().tab().edit.editing_run.as_ref().expect("an editor");
        let (galley, at) = edit.shown.clone().expect("drawn");
        (egui::Id::new(("run-editor", edit.page, edit.object)), edit.buffer.chars().count(), egui::Rect::from_min_size(at, galley.rect.size()).expand(6.0))
    };
    let ctx = h.ctx.clone();
    let mut state = egui::text_edit::TextEditState::load(&ctx, id).unwrap_or_default();
    let everything = egui::text::CCursorRange::two(egui::text::CCursor::new(0), egui::text::CCursor::new(letters));
    state.cursor.set_char_range(Some(everything));
    state.store(&ctx, id);
    h.run_steps(2);
    let selected = egui::text_edit::TextEditState::load(&ctx, id).and_then(|s| s.cursor.char_range()).is_some_and(|r| !r.is_empty());
    assert!(selected, "test assumption: the letters are selected");
    let highlight = crate::theme::violet().gamma_multiply(0.35);
    assert!(!h.output().shapes.iter().any(|clipped| shows_text_inside(&clipped.shape, area, highlight)), "the selected letters are painted over the page's own");
}

/// One line as a letter-spacing producer writes it: `-0.15 Tc` and a `TJ` number after every glyph
/// that cancels it, so the page draws the letters at their own widths.
fn tracked_line(x: f32, y: f32, words: &str) -> String {
    let glyphs: Vec<String> = words.chars().map(|c| format!("({c})")).collect();
    format!("BT /F1 1 Tf 10 0 0 10 {x} {y} Tm -0.15 Tc [{}] TJ ET\n", glyphs.join("-150"))
}

/// Once typed in, the box's words are drawn by the box. It used to draw them with the page's
/// `Tc` alone, which the producer's numbers cancel, so they came out squeezed together.
#[test]
fn retyped_text_keeps_the_pages_letter_spacing() {
    let mut app = opened(open_page("inplace-tracked", &[], &tracked_line(100.0, 700.0, "delta epsilon zeta")));
    // Typed at the end, a one-line run past its edge starts a new line; taking a letter off changes it in place.
    app.tab_mut().edit.editing_run.as_mut().expect("an editor").buffer.pop();
    let mut h = harness_from(app);
    h.run_steps(4);
    let view = h.state().tab().view_state.last_view.expect("the page was drawn");
    let (galley, at, left) = {
        let edit = h.state().tab().edit.editing_run.as_ref().expect("an editor");
        let (galley, at) = edit.shown.clone().expect("drawn");
        (galley, at, edit.lines[0].1.left)
    };
    let left_px = view.to_screen(AppPoint { x: left as f64, y: 0.0 }).x;
    let chars = h.state_mut().characters(0).expect("characters").clone();
    let range = chars.find("delta epsilon zeta").first().cloned().expect("the line in the page's letters");
    let row = &galley.rows[0];
    let mut worst = 0.0f32;
    for (k, i) in range.enumerate() {
        let Some(glyph) = row.row.glyphs.get(k) else { break };
        if glyph.chr.is_whitespace() {
            continue;
        }
        let page = view.to_screen(AppPoint { x: chars.line_rects(i..i + 1).first().map_or(0.0, |r| r.left) as f64, y: 0.0 }).x - left_px;
        worst = worst.max((at.x + row.pos.x + glyph.pos.x - left_px - page).abs());
    }
    assert!(worst < 3.0, "a typed-in letter is {worst:.1} px from where the page has it");
}

#[test]
fn the_box_hugs_a_paragraphs_lines() {
    let mut h = harness_from(opened(open_page("inplace-hug", &LINES, "")));
    h.run_steps(3);
    let view = h.state().tab().view_state.last_view.expect("the page was drawn");
    let edit = h.state().tab().edit.editing_run.as_ref().expect("an editor");
    let frame = edit.frame.expect("the box's frame");
    let (first, last) = (edit.lines[0].1, edit.lines[edit.lines.len() - 1].1);
    let top = view.to_screen(AppPoint { x: 0.0, y: first.top as f64 }).y;
    let bottom = view.to_screen(AppPoint { x: 0.0, y: last.bottom as f64 }).y;
    assert!((frame.top() - top).abs() < 3.0, "the frame starts at {}, the first line at {top}", frame.top());
    assert!((frame.bottom() - bottom).abs() < 4.0, "the frame ends at {}, the last line at {bottom}", frame.bottom());
}

/// A one-line box's frame, and the handles on it, sit on its letters: not on the end of the text
/// box, which can run far past them (a table cell's padding).
#[test]
fn a_one_line_boxs_frame_sits_on_its_letters_not_its_padding() {
    let padded = "BT /F1 10 Tf 100 700 Td [(delta epsilon zeta) -5000 ( )] TJ ET\n";
    let mut h = harness_from(opened(open_page("inplace-hug-one", &[], padded)));
    h.run_steps(3);
    let view = h.state().tab().view_state.last_view.expect("the page was drawn");
    let (frame, boxed, lines) = {
        let edit = h.state().tab().edit.editing_run.as_ref().expect("an editor");
        (edit.frame.expect("the box's frame"), view.to_screen(AppPoint { x: edit.lines[0].1.right as f64, y: 0.0 }).x, edit.lines.len())
    };
    assert_eq!(lines, 1, "test assumption: a one-line box");
    let ink = page_ink_right_px(&mut h, "delta epsilon zeta");
    assert!(boxed - ink > 20.0, "test assumption: the text box runs past its letters");
    assert!(frame.right() - ink < 8.0, "the frame ends at {}, the letters at {ink}", frame.right());
}
