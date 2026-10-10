use super::ui_tests::{fixture, harness};
use super::*;
use egui_kittest::kittest::Queryable;
use egui_kittest::Harness;

const MANY: [&str; 10] = [
    "secret-in-chained-form.pdf",
    "secret-in-form-twice.pdf",
    "secret-in-lzw-form.pdf",
    "secret-in-matrix-form.pdf",
    "secret-in-nested-form.pdf",
    "secret-in-pixels-form.pdf",
    "secret-in-predicted-form.pdf",
    "pages-ladder.pdf",
    "quadrants.pdf",
    "mixed-sizes.pdf",
];

fn many_tabs() -> Harness<'static, PagifyApp> {
    let mut h = harness(MANY[0]);
    for name in &MANY[1..] {
        h.state_mut().open(&fixture(name));
    }
    assert_eq!(h.state().tabs.len(), MANY.len(), "the documents did not all open");
    h.run_steps(6);
    h
}

/// The tabs drawn in the title bar, left to right: their names and where they are.
fn drawn(h: &Harness<'static, PagifyApp>) -> Vec<(&'static str, egui::Rect)> {
    let mut found: Vec<(&'static str, egui::Rect)> = MANY
        .iter()
        .filter_map(|name| {
            h.query_all_by_label(name)
                .map(|n| n.rect())
                .find(|r| r.center().y < 70.0)
                .map(|r| (*name, r))
        })
        .collect();
    found.sort_by(|a, b| a.1.left().total_cmp(&b.1.left()));
    found
}

#[test]
fn how_many_tabs_fit_never_wraps_and_never_leaves_the_strip_empty() {
    let widths = [100.0; 5];
    assert_eq!(tabs_that_fit(&widths, 4.0, 520.0, 26.0), 5, "all of them, with no menu");
    // Too many: as many as fit beside the menu button.
    assert_eq!(tabs_that_fit(&widths, 4.0, 300.0, 26.0), 2);
    assert_eq!(tabs_that_fit(&widths, 4.0, 10.0, 26.0), 1, "never fewer than one");
    assert_eq!(tabs_that_fit(&[], 4.0, 300.0, 26.0), 0);
    // The room for exactly all of them does not call for a menu.
    assert_eq!(tabs_that_fit(&widths, 4.0, 516.0, 26.0), 5);
}

#[test]
fn many_tabs_are_one_row_with_the_newest_on_the_left_and_the_page_keeps_its_space() {
    let mut one = harness(MANY[0]);
    one.run_steps(4);
    let page_top_with_one = one.state().tab().view_state.viewport_rect.expect("drawn").top();

    let h = many_tabs();
    let shown = drawn(&h);
    assert!(shown.len() >= 2 && shown.len() < MANY.len(), "{} of {} tabs drawn", shown.len(), MANY.len());

    // One row: the same line, whatever the number.
    let (lo, hi) = shown
        .iter()
        .fold((f32::MAX, f32::MIN), |(lo, hi), (_, r)| (lo.min(r.center().y), hi.max(r.center().y)));
    assert!(hi - lo <= 1.5, "the tabs are on more than one row: {shown:?}");

    // Newest first: the last document opened is the leftmost, then the one before.
    let order: Vec<&str> = shown.iter().map(|(n, _)| *n).collect();
    let expected: Vec<&str> = MANY.iter().rev().take(shown.len()).copied().collect();
    assert_eq!(order, expected, "the tabs are not newest-first");

    // The page starts where it did with one tab: the strip did not grow.
    let page_top_with_many = h.state().tab().view_state.viewport_rect.expect("drawn").top();
    assert!(
        (page_top_with_many - page_top_with_one).abs() < 1.0,
        "the page area moved from {page_top_with_one} to {page_top_with_many}: the tab strip is taller"
    );
    h.get_by_label("More tabs");
}

#[test]
fn a_tab_from_the_menu_is_shown_and_takes_the_last_place_in_the_strip() {
    let mut h = many_tabs();
    let before = drawn(&h);
    let hidden = MANY[0];
    assert!(!before.iter().any(|(n, _)| *n == hidden), "setup: the oldest tab should be behind the menu");

    h.get_by_label("More tabs").click();
    h.run_steps(3);
    h.get_by_label(hidden).click();
    h.run_steps(5);

    let name_showing = h
        .state()
        .tab()
        .doc
        .as_ref()
        .and_then(|d| d.session.path().file_name().map(|n| n.to_string_lossy().into_owned()));
    assert_eq!(name_showing.as_deref(), Some(hidden), "the chosen tab is not the one showing");
    let after = drawn(&h);
    assert!(after.iter().any(|(n, _)| *n == hidden), "the chosen tab is not in the strip: {after:?}");
    assert_eq!(after.len(), before.len(), "the strip changed size");
    // The one it replaced is now the one behind the menu.
    let (left_out, _) = before.last().unwrap();
    assert!(!after.iter().any(|(n, _)| n == left_out), "{left_out} is still drawn");
    assert_eq!(after.last().map(|(n, _)| *n), Some(hidden), "it did not take the last place");
}

#[test]
fn the_tab_showing_is_always_one_of_the_tabs_drawn() {
    let mut h = many_tabs();
    // Left showing by something other than a click: the last, oldest tab.
    h.state_mut().active_tab = MANY.len() - 1;
    h.run_steps(4);
    let showing = h.state().tab().doc.as_ref().unwrap().session.path().file_name().unwrap().to_string_lossy().into_owned();
    assert!(
        drawn(&h).iter().any(|(n, _)| *n == showing),
        "{showing} is showing but is not in the strip"
    );
}
