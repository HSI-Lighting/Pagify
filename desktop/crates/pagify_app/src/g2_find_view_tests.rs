use super::ui_tests::{fixture, harness, harness_from};
use super::*;
use egui_kittest::kittest::Queryable;
use egui_kittest::Harness;

/// One page: width, height, and its lines as (x, y from the top, text).
type Page = (f32, f32, Vec<(f32, f32, &'static str)>);

/// A PDF whose text is where the test says it is — the fixtures on disk
/// have their words in the top half of one page, which is exactly where a
/// view that only ever shows the top of the page cannot be caught out.
fn text_pdf(pages: &[Page]) -> Vec<u8> {
    let kids: Vec<String> = (0..pages.len()).map(|i| format!("{} 0 R", 4 + 2 * i)).collect();
    let mut objects = vec![
        "<< /Type /Catalog /Pages 2 0 R >>".to_string(),
        format!("<< /Type /Pages /Kids [{}] /Count {} >>", kids.join(" "), pages.len()),
        "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>".to_string(),
    ];
    for (i, (width, height, lines)) in pages.iter().enumerate() {
        let mut content = String::new();
        for (x, y, text) in lines {
            content.push_str(&format!("BT /F1 12 Tf 1 0 0 1 {x} {} Tm ({text}) Tj ET\n", height - y));
        }
        objects.push(format!(
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 {width} {height}] \
             /Resources << /Font << /F1 3 0 R >> >> /Contents {} 0 R >>",
            5 + 2 * i
        ));
        objects.push(format!("<< /Length {} >>\nstream\n{content}endstream", content.len()));
    }
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
    out.extend_from_slice(
        format!("trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref_at}\n%%EOF", objects.len() + 1)
            .as_bytes(),
    );
    out
}

fn written(name: &str, pages: &[Page]) -> String {
    let dir = std::env::temp_dir().join("pagify-g2-tests");
    std::fs::create_dir_all(&dir).expect("scratch dir");
    let path = dir.join(format!("{}-{name}.pdf", std::process::id()));
    std::fs::write(&path, text_pdf(pages)).expect("write the fixture");
    path.to_string_lossy().into_owned()
}

fn open_text_pdf(name: &str, pages: &[Page]) -> Harness<'static, PagifyApp> {
    let app = PagifyApp::new(Some(&written(name, pages)));
    assert!(app.tab().doc.is_some(), "{name} did not open");
    harness_from(app)
}

/// Where the current match was painted this frame: the amber wash at its
/// brighter alpha, which only `draw_text_highlights` draws.
fn painted_current_match(h: &Harness<'static, PagifyApp>) -> Vec<egui::Rect> {
    let current = egui::Color32::from_rgba_unmultiplied(0xFF, 0xC1, 0x07, 150);
    h.output()
        .shapes
        .iter()
        .filter_map(|s| match &s.shape {
            egui::Shape::Rect(r) if r.fill == current => Some(r.rect),
            _ => None,
        })
        .collect()
}

fn assert_match_in_view(h: &Harness<'static, PagifyApp>, what: &str) {
    let view = h.state().tab().view_state.viewport_rect.expect("the page was never drawn");
    let painted = painted_current_match(h);
    assert_eq!(
        painted.len(),
        1,
        "{what}: the current match was painted {} times (window {view:?}, scrolled to {:?}, page {})",
        painted.len(),
        h.state().tab().view_state.scroll_offset,
        h.state().tab().view_state.page,
    );
    assert!(
        view.contains_rect(painted[0]),
        "{what}: the match is painted at {:?}, outside the window {view:?} (scrolled to {:?})",
        painted[0],
        h.state().tab().view_state.scroll_offset,
    );
}

/// A page tall enough that its lower part is below the fold at the zoom
/// the test sets, so a view that stays where it is cannot pass.
fn tall_page_with_a_match_low_down() -> Vec<Page> {
    vec![(
        612.0,
        792.0,
        vec![(72.0, 100.0, "Lorem ipsum dolor sit amet"), (72.0, 720.0, "the zebra is here")],
    )]
}

fn assert_below_the_fold(h: &Harness<'static, PagifyApp>, down_to: f32, zoom: f32) {
    let view = h.state().tab().view_state.viewport_rect.expect("the page was never drawn");
    let at = down_to * zoom * PagifyApp::DISPLAY_DPI_SCALE;
    assert!(
        view.height() < at,
        "the fixture is not tall enough to prove anything: window {view:?}, the match is {at}px down"
    );
}

// -- 1. Find must show the match ------------------------------------------

/// **Reported from use: "it jumps to the page, but the highlighted match is
/// not in view — you have to scroll to spot it."** The page was already
/// the current one, so nothing moved at all, and the match was below the
/// bottom of the window.
#[test]
fn find_brings_a_match_low_on_the_page_into_view() {
    let mut h = open_text_pdf("low", &tall_page_with_a_match_low_down());
    h.state_mut().tab_mut().view_state.zoom = ZoomMode::Factor(1.5);
    h.run_steps(4);
    assert_below_the_fold(&h, 720.0, 1.5);

    h.state_mut().submit("find zebra");
    h.run_steps(8);

    assert_eq!(h.state().tab().panels.find_hits.len(), 1);
    assert_match_in_view(&h, "Find on a zoomed page");
}

/// The same, from the button the testers actually used: the Find button in
/// the Search & Replace window.
#[test]
fn the_find_button_brings_the_match_into_view_too() {
    let mut h = open_text_pdf("button", &tall_page_with_a_match_low_down());
    h.state_mut().tab_mut().view_state.zoom = ZoomMode::Factor(1.5);
    h.state_mut().submit("replace");
    h.run_steps(2);
    h.state_mut().tab_mut().panels.find_replace.as_mut().expect("the panel").find = "zebra".into();
    h.run_steps(2);

    h.get_by_role_and_label(egui::accesskit::Role::Button, "Find").click();
    h.run_steps(8);

    assert_match_in_view(&h, "the Find button");
}

/// Three pages at Width zoom: each is taller than the window, and the
/// match on the last is near its bottom — going to that page is not
/// enough.
fn three_pages_with_a_match_at_each_end() -> Vec<Page> {
    let page = |lines: Vec<(f32, f32, &'static str)>| (612.0, 792.0, lines);
    vec![
        page(vec![(72.0, 100.0, "the zebra opens the document")]),
        page(vec![(72.0, 400.0, "nothing to see on this one")]),
        page(vec![(72.0, 700.0, "and the zebra closes it")]),
    ]
}

#[test]
fn find_next_on_another_page_reveals_the_match_not_just_the_page() {
    let mut h = open_text_pdf("pages", &three_pages_with_a_match_at_each_end());
    h.state_mut().tab_mut().view_state.zoom = ZoomMode::Width;
    h.run_steps(4);

    h.state_mut().submit("find zebra");
    h.run_steps(8);
    assert_eq!(h.state().tab().panels.find_at, 0);
    assert_match_in_view(&h, "the first match");

    h.state_mut().submit("findnext");
    h.run_steps(8);
    assert_eq!(h.state().tab().panels.find_at, 1);
    assert_eq!(h.state().tab().view_state.page, 2, "the match is on the third page");
    assert_match_in_view(&h, "the match on the third page");
}

#[test]
fn stepping_back_and_wrapping_reveal_the_match_too() {
    let mut h = open_text_pdf("wrap", &three_pages_with_a_match_at_each_end());
    h.state_mut().tab_mut().view_state.zoom = ZoomMode::Width;
    h.run_steps(4);
    h.state_mut().submit("find zebra");
    h.run_steps(6);
    h.state_mut().submit("findnext");
    h.run_steps(6);
    assert_match_in_view(&h, "the last match");

    // Forward from the last one wraps to the first, at the top.
    h.state_mut().submit("findnext");
    h.run_steps(8);
    assert_eq!(h.state().tab().panels.find_at, 0, "stepping on from the last did not wrap");
    assert_match_in_view(&h, "wrapping round to the first match");

    // And back from the first wraps to the last, at the bottom.
    h.state_mut().submit("findprev");
    h.run_steps(8);
    assert_eq!(h.state().tab().panels.find_at, 1, "stepping back from the first did not wrap");
    assert_match_in_view(&h, "wrapping back to the last match");
}

/// A match in the middle of a page that is much taller than the window is
/// put in the middle of the window, not at the top of its page.
#[test]
fn a_match_in_the_middle_of_a_tall_page_is_centred() {
    let mut h = open_text_pdf(
        "middle",
        &[(612.0, 3000.0, vec![(72.0, 100.0, "the start"), (72.0, 1500.0, "the zebra")])],
    );
    h.state_mut().tab_mut().view_state.zoom = ZoomMode::Factor(1.0);
    h.run_steps(4);
    // A zoom now keeps the middle of the page in the middle of the window,
    // which for this page is the zebra itself. The match has to start out of
    // sight for finding it to have anything to do.
    h.state_mut().tab_mut().view_state.scroll_to_pt = Some(0.0);
    h.run_steps(3);

    h.state_mut().submit("find zebra");
    h.run_steps(8);

    assert_match_in_view(&h, "a match in the middle of a tall page");
    let view = h.state().tab().view_state.viewport_rect.expect("drawn");
    let wash = painted_current_match(&h)[0];
    assert!(
        (wash.center().y - view.center().y).abs() < 2.0,
        "the match is not centred: it is at {:?}, the window's middle is {}",
        wash.center().y,
        view.center().y
    );
}

// ---- the reader keeps their place when the page area changes ------------

/// Where in the strip, in page points, the middle of the window is.
fn middle_of_the_window_pt(h: &Harness<'static, PagifyApp>) -> f32 {
    let view = h.state().tab().view_state.viewport_rect.expect("the page was never drawn");
    let zoom = h.state().resolved_zoom() * PagifyApp::DISPLAY_DPI_SCALE;
    (h.state().tab().view_state.scroll_offset.y + view.height() / 2.0 - STRIP_PAD_PX) / zoom
}

fn thirty_pages() -> Vec<Page> {
    (0..30).map(|_| (612.0, 792.0, vec![(72.0, 400.0, "a line in the middle")])).collect()
}

/// **Reported from use: "the page shifts after editing", and the same when
/// changing tools.** Anything that changes the width of the page area — the
/// Edit Text panel docking, the Pages rail, Organize — changes the zoom in
/// Fit and Width, and the reader's place was only a pixel offset, so on a
/// long document the same pixels were a different part of it.
#[test]
fn narrowing_the_page_area_does_not_move_the_reader_in_width_zoom() {
    for mode in [ZoomMode::Width, ZoomMode::Fit] {
        let mut h = open_text_pdf("narrowing", &thirty_pages());
        h.state_mut().tab_mut().view_state.zoom = mode;
        h.run_steps(4);
        h.state_mut().act(Verb::Page(PageTarget::Number(15)));
        h.run_steps(4);
        // Part of the way down the page, not at its top.
        h.input_mut().events.push(egui::Event::PointerMoved(egui::pos2(700.0, 500.0)));
        h.run_steps(2);
        h.input_mut().events.push(egui::Event::MouseWheel {
            unit: egui::MouseWheelUnit::Point,
            delta: egui::vec2(0.0, -150.0),
            phase: egui::TouchPhase::Move,
            modifiers: Default::default(),
        });
        h.run_steps(4);
        let (page, before) = (h.state().tab().view_state.page, middle_of_the_window_pt(&h));
        assert_eq!(page, 14, "{mode:?}: setup: the reader should be on page 15");

        let rail = h.state().ui_state.show_thumbs;
        h.state_mut().ui_state.show_thumbs = !rail;
        h.run_steps(6);
        let after = middle_of_the_window_pt(&h);
        assert_eq!(h.state().tab().view_state.page, page, "{mode:?}: the reader was moved to another page");
        assert!(
            (after - before).abs() < 2.0,
            "{mode:?}: the middle of the window moved from {before} to {after} (page points)"
        );

        // And putting it back puts them back.
        h.state_mut().ui_state.show_thumbs = rail;
        h.run_steps(6);
        let again = middle_of_the_window_pt(&h);
        assert!((again - before).abs() < 2.0, "{mode:?}: {again} is not where it started, {before}");
    }
}

/// The restore must only answer a change, never a scroll.
#[test]
fn an_ordinary_scroll_is_not_undone_by_the_view_restore() {
    let mut h = open_text_pdf("scrolling", &thirty_pages());
    h.state_mut().tab_mut().view_state.zoom = ZoomMode::Width;
    h.run_steps(4);
    let start = h.state().tab().view_state.scroll_offset.y;
    h.input_mut().events.push(egui::Event::PointerMoved(egui::pos2(700.0, 500.0)));
    h.run_steps(2);
    for _ in 0..4 {
        h.input_mut().events.push(egui::Event::MouseWheel {
            unit: egui::MouseWheelUnit::Point,
            delta: egui::vec2(0.0, -200.0),
            phase: egui::TouchPhase::Move,
            modifiers: Default::default(),
        });
        h.run_steps(2);
    }
    h.run_steps(6);
    let moved = h.state().tab().view_state.scroll_offset.y - start;
    assert!(moved > 400.0, "the scroll went only {moved} px");
}

/// Each document keeps its own place: another tab's offset must not be
/// carried under this one's pages.
#[test]
fn each_document_has_its_own_scroll_state() {
    let mut h = open_text_pdf("own-state", &thirty_pages());
    h.state_mut().tab_mut().view_state.zoom = ZoomMode::Width;
    h.run_steps(3);
    h.state_mut().act(Verb::Page(PageTarget::Number(20)));
    h.run_steps(4);
    let deep = h.state().tab().view_state.scroll_offset.y;
    assert!(deep > 5_000.0, "setup: not far enough down ({deep})");

    // A second document opened in a new tab starts at its own top.
    h.state_mut().submit(&format!("open \"{}\"", fixture("single-page.pdf")));
    h.run_steps(6);
    assert!(
        h.state().tab().view_state.scroll_offset.y < 100.0,
        "the new document inherited the other's offset: {}",
        h.state().tab().view_state.scroll_offset.y
    );
}

/// **A match that is already comfortably on screen does not move the
/// view**, so stepping through the matches on one screenful reads like
/// reading, not like the page lurching on every press.
#[test]
fn a_match_already_in_view_does_not_move_the_page() {
    let mut h = open_text_pdf(
        "stay",
        &[(
            612.0,
            3000.0,
            vec![(72.0, 100.0, "the start"), (72.0, 1400.0, "the zebra one"), (72.0, 1500.0, "the zebra two")],
        )],
    );
    h.state_mut().tab_mut().view_state.zoom = ZoomMode::Factor(1.0);
    h.run_steps(4);

    h.state_mut().submit("find zebra");
    h.run_steps(8);
    assert_match_in_view(&h, "the first of two");
    let after_find = h.state().tab().view_state.scroll_offset;

    h.state_mut().submit("findnext");
    h.run_steps(8);
    assert_eq!(h.state().tab().panels.find_at, 1);
    assert_match_in_view(&h, "the second of two");
    assert_eq!(
        h.state().tab().view_state.scroll_offset,
        after_find,
        "the second match was already on screen, but the view moved"
    );
}

/// A match in the very corner of the page: the view goes to the corner, not
/// past it.
#[test]
fn a_match_at_the_top_left_corner_is_in_view_with_the_view_at_the_corner() {
    let mut h = open_text_pdf("corner", &[(612.0, 792.0, vec![(2.0, 12.0, "zebra")])]);
    h.state_mut().tab_mut().view_state.zoom = ZoomMode::Factor(6.0);
    h.run_steps(4);

    h.state_mut().submit("find zebra");
    h.run_steps(8);

    assert_match_in_view(&h, "a match in the page's corner");
    assert_eq!(h.state().tab().view_state.scroll_offset, egui::Vec2::ZERO, "the view went past the corner");
}

/// Sideways too: on a page zoomed past the window's width a match off to the
/// right is off screen, and the page-top jump never touched the horizontal
/// offset.
#[test]
fn a_match_off_to_the_side_of_a_zoomed_in_page_is_found() {
    let mut h = open_text_pdf("side", &[(612.0, 792.0, vec![(72.0, 100.0, "the start"), (480.0, 150.0, "zebra")])]);
    h.state_mut().tab_mut().view_state.zoom = ZoomMode::Factor(4.0);
    h.run_steps(4);
    let view = h.state().tab().view_state.viewport_rect.expect("drawn");
    assert!(
        view.width() < 480.0 * 4.0 * PagifyApp::DISPLAY_DPI_SCALE,
        "the fixture is not wide enough to prove anything: {view:?}"
    );

    h.state_mut().submit("find zebra");
    h.run_steps(8);

    assert_match_in_view(&h, "a match off to the right");
}

/// **Facing pages overflow sideways even at Width**, because Width is sized
/// from one page and the strip is two wide: a match on the right-hand page
/// is off screen at the zoom the reader chose to fit the page.
#[test]
fn a_match_on_the_right_hand_page_of_a_facing_view_is_found() {
    let mut h = open_text_pdf(
        "facing",
        &[
            (612.0, 792.0, vec![(72.0, 100.0, "the left page")]),
            (612.0, 792.0, vec![(72.0, 100.0, "the zebra on the right")]),
        ],
    );
    h.state_mut().submit("viewfacing");
    h.state_mut().tab_mut().view_state.zoom = ZoomMode::Width;
    h.run_steps(6);

    h.state_mut().submit("find zebra");
    h.run_steps(8);

    assert_match_in_view(&h, "a match on the right-hand page");
}

/// **A word in a page's footer, at the default Fit zoom, shows the page as
/// it fits — it does not half scroll it away.** At Fit the page's bottom is
/// exactly the strip's padding above the window's, so a rule asking for more
/// clear space than that under the word sends every page number and footer
/// to "centre it", with the page top off the window.
#[test]
fn a_footer_at_fit_keeps_the_page_top_where_going_to_the_page_put_it() {
    // The middle page of three: on the last one the view cannot go past the
    // end of the strip, so "centre it" and "the page top" are the same place.
    let page = |footer: &'static str| (612.0, 792.0, vec![(72.0, 100.0, "a page"), (72.0, 772.0, footer)]);
    let mut h = open_text_pdf("footer", &[page("one"), page("zebra"), page("three")]);
    assert!(matches!(h.state().tab().view_state.zoom, ZoomMode::Fit), "Fit is the default zoom");

    h.state_mut().submit("find zebra");
    h.run_steps(8);

    assert_eq!(h.state().tab().view_state.page, 1);
    assert_match_in_view(&h, "a footer at Fit");
    let scale = h.state().tab().view_state.last_view.expect("drawn").scale;
    let page_top = h.state().tab().doc.as_ref().expect("doc").strip.scroll_to(1) * scale;
    assert!(
        (h.state().tab().view_state.scroll_offset.y - page_top).abs() < 1.0,
        "the view is at {}, not at the top of the page ({page_top})",
        h.state().tab().view_state.scroll_offset.y
    );
}

/// A turned view has no usable rectangle for a word — the page-space box
/// is not where the word is on screen, the same reason the sharp detail
/// overlay stands down there — so Find does there what it always did: goes
/// to the page, and on the page it is already on does not scroll.
#[test]
fn in_a_turned_view_find_does_not_scroll_to_a_box_that_is_not_where_the_word_is() {
    let mut h = open_text_pdf("turned-one", &tall_page_with_a_match_low_down());
    h.state_mut().tab_mut().view_state.zoom = ZoomMode::Factor(1.5);
    h.state_mut().submit("rotate 90");
    h.run_steps(4);
    assert!(!matches!(h.state().tab().view_state.rotation, Rotation::None), "the view did not turn");

    h.state_mut().submit("find zebra");
    h.run_steps(6);

    assert_eq!(h.state().tab().panels.find_hits.len(), 1);
    assert_eq!(h.state().tab().view_state.scroll_offset, egui::Vec2::ZERO, "the view went somewhere for a box that is not there");
}

#[test]
fn in_a_turned_view_find_still_goes_to_the_page() {
    let mut h = open_text_pdf("turned", &three_pages_with_a_match_at_each_end());
    h.state_mut().submit("rotate 90");
    h.run_steps(4);
    assert!(!matches!(h.state().tab().view_state.rotation, Rotation::None), "the view did not turn");

    h.state_mut().submit("find zebra");
    h.state_mut().submit("findnext");
    h.run_steps(6);

    assert_eq!(h.state().tab().panels.find_at, 1);
    assert_eq!(h.state().tab().view_state.page, 2, "Find did not go to the page the match is on");
    assert!(h.state().tab().view_state.reveal.is_none());
    let scale = h.state().tab().view_state.last_view.expect("drawn").scale;
    let page_top = h.state().tab().doc.as_ref().expect("doc").strip.scroll_to(2) * scale;
    assert!(
        (h.state().tab().view_state.scroll_offset.y - page_top).abs() < 1.0,
        "the view is at {}, not at the top of the page ({page_top})",
        h.state().tab().view_state.scroll_offset.y
    );
}

/// The arithmetic on its own, over a grid of windows and words: the answer
/// is always somewhere the view can be, anything that fits is wholly
/// shown, a word already comfortably in view is left alone, and asking
/// again from where it ended up changes nothing.
#[test]
fn reveal_axis_stays_in_range_shows_what_fits_and_leaves_alone_what_is_in_view() {
    let (len, room) = (700.0_f32, 2000.0_f32);
    let content = len + room;
    let mut cases = 0;
    for offset in [0.0, 150.0, 640.0, 1999.0, 2000.0] {
        for lo in (12..2688).step_by(41).map(|v| v as f32) {
            for width in [4.0, 60.0, 300.0, 640.0, 700.0, 950.0] {
                let hi = (lo + width).min(content - STRIP_PAD_PX);
                for page_top in [None, Some(0.0), Some(lo - 30.0), Some(lo - 600.0)] {
                    let to = reveal_axis(offset, len, room, lo, hi, page_top);
                    let what = format!("offset {offset}, word {lo}..{hi}, page top {page_top:?} -> {to}");
                    assert!((0.0..=room).contains(&to), "out of range: {what}");
                    if hi - lo <= len {
                        assert!(lo >= to && hi <= to + len, "a word that fits is not wholly shown: {what}");
                    }
                    if lo - REVEAL_AIR_PX >= offset && hi + REVEAL_AIR_PX <= offset + len {
                        assert_eq!(to, offset, "a word comfortably in view moved the view: {what}");
                    }
                    assert_eq!(reveal_axis(to, len, room, lo, hi, page_top), to, "not stable: {what}");
                    cases += 1;
                }
            }
        }
    }
    assert!(cases > 4000, "the grid shrank: {cases}");
}

// -- 2. Replace and the Find button keep their place ----------------------

fn open_two_column() -> PagifyApp {
    PagifyApp::new(Some(&fixture("two-column.pdf")))
}

fn run_texts(app: &PagifyApp, page: usize) -> Vec<String> {
    let session = &app.tab().doc.as_ref().expect("doc").session;
    session.text_runs(page).expect("runs").into_iter().map(|r| r.text).collect()
}

/// **After Replace the view goes on to the next match, not back to the
/// first.** `replace_current` searched again from the start, so replacing
/// the third match put the view on the first.
#[test]
fn replace_goes_on_to_the_match_after_the_one_it_replaced() {
    let mut app = open_two_column();
    app.find("the");
    app.find_step(true);
    app.find_step(true);
    assert_eq!(app.tab().panels.find_at, 2);
    let (page, was) = app.tab().panels.find_hits[2].clone();
    let before = app.tab().panels.find_hits.len();

    app.replace_current("the", "XYZ").expect("replace failed");

    assert_eq!(app.tab().panels.find_hits.len(), before - 1);
    let at = app.tab().panels.find_at;
    let (now_page, now) = app.tab().panels.find_hits[at].clone();
    assert!(
        (now_page, now.start) >= (page, was.start),
        "after replacing match 3 the view went back to match {} (page {now_page}, character {})",
        at + 1,
        now.start
    );
    assert_eq!(at, 2, "the next match is the one that was fourth");
    // And it is shown: the selection is on it, ready to be replaced in turn.
    assert_eq!(app.tab().selection.text_selection, Some(now));
}

/// A replacement that still contains the word searched for — "the" to
/// "other" — leaves a match on the very spot just replaced. Going on must
/// step past it, or Replace can never get beyond the first match.
#[test]
fn a_replacement_containing_the_search_term_is_stepped_past() {
    let mut app = open_two_column();
    app.find("the");
    let (page, was) = app.tab().panels.find_hits[0].clone();

    app.replace_current("the", "other").expect("replace failed");

    let (now_page, now) = app.tab().panels.find_hits[app.tab().panels.find_at].clone();
    assert!(
        (now_page, now.start) >= (page, was.start + "other".chars().count()),
        "the view stayed on the word just replaced (character {} of page {now_page})",
        now.start
    );
}

/// **Find with the same word steps on**, like Find Next, instead of going
/// back to the first match every time it is pressed.
#[test]
fn pressing_find_again_with_the_same_word_steps_to_the_next_match() {
    let mut h = harness("two-column.pdf");
    h.state_mut().submit("replace");
    h.run_steps(2);
    h.state_mut().tab_mut().panels.find_replace.as_mut().expect("the panel").find = "the".into();
    h.run_steps(2);

    h.get_by_role_and_label(egui::accesskit::Role::Button, "Find").click();
    h.run_steps(3);
    assert_eq!(h.state().tab().panels.find_at, 0);
    h.get_by_role_and_label(egui::accesskit::Role::Button, "Find").click();
    h.run_steps(3);
    assert_eq!(h.state().tab().panels.find_at, 1, "the second Find went back to the first match");
    h.get_by_role_and_label(egui::accesskit::Role::Button, "Find").click();
    h.run_steps(3);
    assert_eq!(h.state().tab().panels.find_at, 2);
}

/// A different word is a new search, from the first match.
#[test]
fn find_with_a_different_word_starts_again_from_the_first_match() {
    let mut h = harness("two-column.pdf");
    h.state_mut().submit("find the");
    h.state_mut().submit("findnext");
    h.state_mut().submit("findnext");
    h.state_mut().submit("replace");
    h.run_steps(2);
    h.state_mut().tab_mut().panels.find_replace.as_mut().expect("the panel").find = "and".into();
    h.run_steps(2);

    h.get_by_role_and_label(egui::accesskit::Role::Button, "Find").click();
    h.run_steps(3);

    assert_eq!(h.state().tab().panels.find_needle, "and");
    assert_eq!(h.state().tab().panels.find_at, 0);
}

/// **A match is shown before it is replaced.** Replace with a word that had
/// not been searched for yet replaced the first match without the reader
/// ever having seen it; now the first press shows it and the next one
/// replaces what is on screen.
#[test]
fn replace_shows_the_match_before_replacing_it() {
    let mut app = PagifyApp::new(Some(&fixture("text-lines.pdf")));
    let before = run_texts(&app, 0);

    let said = app.replace_current("fox", "wolf").expect("it should show the match");
    assert!(said.contains("replace"), "it did not say what to do next: {said}");
    assert_eq!(run_texts(&app, 0), before, "the first press replaced a match nobody had seen");
    assert_eq!(app.tab().panels.find_hits.len(), 1);
    assert!(app.tab().selection.text_selection.is_some(), "the match was not shown");

    let said = app.replace_current("fox", "wolf").expect("replace failed");
    assert!(said.contains("none left"), "{said}");
    assert_ne!(run_texts(&app, 0), before, "the second press did not replace it");
}

/// **Replace changes the match you are on, not the first one in the line.**
/// The run held "the cat and the dog"; stepping to the second "the" and
/// pressing Replace changed the first.
#[test]
fn replace_changes_the_occurrence_you_are_on_not_the_first_in_its_line() {
    let path = written("twice", &[(612.0, 792.0, vec![(72.0, 100.0, "the cat and the dog")])]);
    let mut app = PagifyApp::new(Some(&path));
    app.find("the");
    assert_eq!(app.tab().panels.find_hits.len(), 2);
    app.find_step(true);
    assert_eq!(app.tab().panels.find_at, 1);

    app.replace_current("the", "a").expect("replace failed");

    assert_eq!(run_texts(&app, 0), vec!["the cat and a dog".to_string()]);
}

/// **Enter in the field is Find, and goes on being Find.** A single-line
/// field lets go of the keyboard on Enter, so the second Enter used to
/// reach nothing at all.
#[test]
fn enter_in_the_find_field_steps_on_every_time() {
    let mut h = harness("two-column.pdf");
    h.state_mut().submit("replace");
    h.run_steps(2);
    h.state_mut().tab_mut().panels.find_replace.as_mut().expect("the panel").find = "the".into();
    h.run_steps(2);
    // The panel's field, not the command box's: it is the one holding "the".
    h.get_all_by_role(egui::accesskit::Role::TextInput)
        .find(|n| n.value().as_deref() == Some("the"))
        .expect("the Find field")
        .focus();
    h.run_steps(2);

    for expected in 0..3 {
        h.key_press(egui::Key::Enter);
        h.run_steps(3);
        assert_eq!(h.state().tab().panels.find_at, expected, "Enter number {} did not step on", expected + 1);
    }
}

// -- 3. Drawing a highlight does not read the page -----------------------

fn three_text_pages() -> Vec<Page> {
    let page = |text: &'static str| (612.0, 792.0, vec![(72.0, 100.0, text)]);
    vec![page("the zebra on the first page"), page("nothing on the second"), page("or the third")]
}

fn pages_in_view(h: &Harness<'static, PagifyApp>, zoom: f32) -> usize {
    let view = h.state().tab().view_state.viewport_rect.expect("drawn");
    let strip = &h.state().tab().doc.as_ref().expect("doc").strip;
    strip.visible(0.0, view.height() / (zoom * PagifyApp::DISPLAY_DPI_SCALE)).len()
}

/// **Nothing is asked of the engine for the text of a page that has
/// nothing to highlight.** The highlight pass fetched every visible page's
/// characters on every frame, through the one lock the render worker also
/// holds, even when there was no search and no selection.
#[test]
fn nothing_asks_for_page_text_when_there_is_nothing_to_highlight() {
    let mut h = open_text_pdf("quiet", &three_text_pages());
    h.state_mut().tab_mut().view_state.zoom = ZoomMode::Factor(0.3);
    h.run_steps(4);
    assert!(pages_in_view(&h, 0.3) >= 2, "the test needs more than one page on screen");

    h.run_steps(3);

    assert!(
        h.state().tab().doc.as_ref().unwrap().caches.text.is_none(),
        "a frame with nothing to highlight extracted the text of page {:?}",
        h.state().tab().doc.as_ref().unwrap().caches.text.as_ref().map(|(p, _)| *p)
    );
}

/// With a hit on one of several visible pages, only that page is asked for
/// its text: the others used to be asked too, each evicting the last
/// one's entry from the one-page cache.
#[test]
fn only_a_page_with_something_to_highlight_is_asked_for_its_text() {
    let mut h = open_text_pdf("one-hit", &three_text_pages());
    h.state_mut().tab_mut().view_state.zoom = ZoomMode::Factor(0.3);
    h.run_steps(4);
    assert!(pages_in_view(&h, 0.3) >= 2, "the test needs more than one page on screen");

    h.state_mut().submit("find zebra");
    h.run_steps(4);

    assert_eq!(h.state().tab().panels.find_hits.len(), 1);
    assert_eq!(
        h.state().tab().doc.as_ref().unwrap().caches.text.as_ref().map(|(p, _)| *p),
        Some(0),
        "a page with no hit and no selection was asked for its text"
    );
}

// -- 4. The File tab does not reset the document view -------------------

/// **Visiting File and coming back put the page back at the top.** The
/// File tab's scroll area and the page strip's are made on the same panel
/// with the same default id, so they shared one stored scroll position, and
/// a frame of File stored a zero over where the reader was.
#[test]
fn visiting_the_file_tab_does_not_put_the_view_back_at_the_top() {
    let mut h = harness("pages-ladder.pdf");
    h.state_mut().tab_mut().view_state.zoom = ZoomMode::Factor(2.0);
    h.run_steps(3);
    h.state_mut().pan(egui::vec2(0.0, -700.0));
    h.run_steps(4);
    let scrolled = h.state().tab().view_state.scroll_offset;
    assert!(scrolled.y > 300.0, "the document did not scroll: {scrolled:?}");

    h.state_mut().tab_mut().ribbon = Tab::File;
    h.run_steps(3);
    h.state_mut().tab_mut().ribbon = Tab::Home;
    h.run_steps(3);

    assert_eq!(
        h.state().tab().view_state.scroll_offset,
        scrolled,
        "going to the File tab and back moved the document"
    );
}
