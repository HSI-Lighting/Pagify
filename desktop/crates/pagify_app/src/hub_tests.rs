//! Tests of `hub`: where a release lands, what leaving means, and what a tab
//! carries with it when it moves. Nothing here needs a second operating-system
//! window — the geometry is handed in — and the real thing is exercised by
//! running the program (see the report that came with this change).

use super::*;
use crate::ui_tests::harness_from;
use crate::{Tab, ZoomMode};
use egui::{pos2, vec2};

fn fixture(name: &str) -> String {
    format!("{}/../../../rust/pdf_core/fixtures/{name}", env!("CARGO_MANIFEST_DIR"))
}

/// An app with these documents open, one tab each, in the order given, the last
/// one showing. A document opened is put on the left of the strip, so they are
/// opened last to first.
fn app_with(names: &[&str]) -> PagifyApp {
    let last = names.len() - 1;
    let mut app = PagifyApp::new(Some(&fixture(names[last])));
    for name in names[..last].iter().rev() {
        app.open(&fixture(name));
    }
    app.active_tab = last;
    assert_eq!(app.tabs.len(), names.len(), "the documents did not all open");
    app
}

fn pages_of(tab: &DocTab) -> usize {
    tab.doc.as_ref().expect("a document").session.page_count().expect("page count")
}

fn name_of(tab: &DocTab) -> String {
    Hub::name_of(tab)
}

fn marks(tab: &DocTab) -> usize {
    tab.markup.existing(tab.page).map(|l| l.len()).unwrap_or(0)
}

fn rect(l: f32, t: f32, r: f32, b: f32) -> Rect {
    Rect::from_min_max(pos2(l, t), pos2(r, b))
}

/// `n` tabs, each 100 wide with a 10 gap, in a row 30 high.
fn row(n: usize) -> Vec<Rect> {
    (0..n).map(|i| rect(10.0 + 110.0 * i as f32, 10.0, 110.0 + 110.0 * i as f32, 40.0)).collect()
}

/// A window at `outer`, whose strip runs from 30 to 110 below its top and whose
/// tabs are `row(n)` shifted to sit in it (y 50 to 80 below the top).
fn window(name: &str, outer: Rect, tabs: usize) -> Geom {
    let strip = Some(rect(outer.left(), outer.top() + 30.0, outer.right(), outer.top() + 110.0));
    let tabs = row(tabs).into_iter().map(|r| r.translate(vec2(outer.left(), outer.top() + 40.0))).collect();
    Geom { id: ViewportId::from_hash_of(name), outer, new_size: outer.size(), strip, tabs }
}

// -- where a release lands ------------------------------------------------

#[test]
fn a_place_in_a_strip_is_the_number_of_tabs_before_the_point() {
    let tabs = row(3);
    assert_eq!(slot(pos2(0.0, 25.0), &tabs, None), 0, "left of every tab");
    assert_eq!(slot(pos2(50.0, 25.0), &tabs, None), 0, "over the first tab, left of its middle");
    assert_eq!(slot(pos2(70.0, 25.0), &tabs, None), 1, "right of the first tab's middle");
    assert_eq!(slot(pos2(200.0, 25.0), &tabs, None), 2);
    assert_eq!(slot(pos2(5000.0, 25.0), &tabs, None), 3, "past the last tab");
}

#[test]
fn the_tab_being_carried_is_not_in_the_way_of_itself() {
    let tabs = row(3);
    // Carrying the first tab to just right of the second: it becomes the second.
    assert_eq!(slot(pos2(200.0, 25.0), &tabs, Some(0)), 1);
    // Carrying the last tab to the far left: it becomes the first.
    assert_eq!(slot(pos2(0.0, 25.0), &tabs, Some(2)), 0);
    // And over where it already is: it stays where it is.
    assert_eq!(slot(pos2(tabs[1].center().x, 25.0), &tabs, Some(1)), 1);
}

#[test]
fn the_margin_of_the_title_bar_is_on_the_row_of_tabs() {
    let tabs = row(3);
    for y in [0.0, 5.0, 45.0, 49.0] {
        assert_eq!(slot(pos2(200.0, y), &tabs, None), 2, "y {y} is level with the tabs");
    }
}

#[test]
fn a_strip_that_wraps_is_read_row_by_row() {
    // Two rows of two: the second row is lower.
    let tabs = vec![
        rect(10.0, 10.0, 110.0, 40.0),
        rect(120.0, 10.0, 220.0, 40.0),
        rect(10.0, 50.0, 110.0, 80.0),
        rect(120.0, 50.0, 220.0, 80.0),
    ];
    assert_eq!(slot(pos2(5000.0, 25.0), &tabs, None), 2, "past the end of the first row");
    assert_eq!(slot(pos2(0.0, 65.0), &tabs, None), 2, "start of the second row");
    assert_eq!(slot(pos2(5000.0, 65.0), &tabs, None), 4);
}

#[test]
fn a_slot_in_an_empty_strip_is_the_first() {
    assert_eq!(slot(pos2(50.0, 20.0), &[], None), 0);
}

#[test]
fn letting_go_inside_the_window_it_came_from_moves_nothing_out() {
    let source = window("a", rect(0.0, 0.0, 1000.0, 800.0), 3);
    let other = window("b", rect(1100.0, 0.0, 2100.0, 800.0), 2);
    // Over the page: back where it was.
    assert_eq!(decide(pos2(500.0, 400.0), 1, &source, &[other.clone()]), Drop::Nothing);
    // Over the window's own title bar frame, above the strip: still inside.
    assert_eq!(decide(pos2(500.0, 5.0), 1, &source, &[other]), Drop::Nothing);
}

#[test]
fn letting_go_in_its_own_strip_reorders() {
    let source = window("a", rect(0.0, 0.0, 1000.0, 800.0), 3);
    let y = 65.0;
    assert_eq!(decide(pos2(5.0, y), 2, &source, &[]), Drop::Reorder { to: 0 });
    assert_eq!(decide(pos2(500.0, y), 0, &source, &[]), Drop::Reorder { to: 2 });
    // Over where it already was: nothing to do.
    assert_eq!(decide(pos2(source.tabs[1].center().x, y), 1, &source, &[]), Drop::Nothing);
}

#[test]
fn letting_go_over_another_window_puts_it_there() {
    let source = window("a", rect(0.0, 0.0, 1000.0, 800.0), 3);
    let other = window("b", rect(1100.0, 0.0, 2100.0, 800.0), 2);
    // Over its strip, between its two tabs: at that place.
    let between = pos2(1100.0 + 10.0 + 100.0 + 5.0, 65.0);
    assert_eq!(decide(between, 0, &source, &[other.clone()]), Drop::Merge { into: other.id, at: 1 });
    // Over its strip, left of everything.
    assert_eq!(decide(pos2(1101.0, 65.0), 0, &source, &[other.clone()]), Drop::Merge { into: other.id, at: 0 });
    // Over its page: at the end.
    assert_eq!(decide(pos2(1500.0, 500.0), 0, &source, &[other.clone()]), Drop::Merge { into: other.id, at: 2 });
    // Over its title bar's own frame, above the strip: at the end too.
    assert_eq!(decide(pos2(1500.0, 2.0), 0, &source, &[other.clone()]), Drop::Merge { into: other.id, at: 2 });
}

#[test]
fn of_two_windows_over_each_other_the_one_used_last_is_on_top() {
    let source = window("a", rect(0.0, 0.0, 1000.0, 800.0), 2);
    let recent = window("b", rect(1100.0, 0.0, 2100.0, 800.0), 1);
    let older = window("c", rect(1500.0, 100.0, 2500.0, 900.0), 1);
    let p = pos2(1800.0, 500.0);
    assert_eq!(decide(p, 0, &source, &[recent.clone(), older.clone()]), Drop::Merge { into: recent.id, at: 1 });
    assert_eq!(decide(p, 0, &source, &[older.clone(), recent]), Drop::Merge { into: older.id, at: 1 });
}

#[test]
fn the_window_it_came_from_is_on_top_of_a_window_under_it() {
    // They overlap; the point is in both. The source was just clicked, so it
    // is the one on top: back where it was, not into the other.
    let source = window("a", rect(0.0, 0.0, 1000.0, 800.0), 2);
    let under = window("b", rect(800.0, 0.0, 1800.0, 800.0), 2);
    assert_eq!(decide(pos2(900.0, 500.0), 0, &source, &[under]), Drop::Nothing);
}

#[test]
fn a_little_outside_the_window_is_not_a_tear_off_but_well_outside_is() {
    let source = window("a", rect(100.0, 100.0, 1100.0, 900.0), 2);
    // Just off the edges: a click that slid, not a tear-off.
    assert_eq!(decide(pos2(1100.0 + DEAD_ZONE - 1.0, 400.0), 0, &source, &[]), Drop::Nothing);
    assert_eq!(decide(pos2(100.0 - DEAD_ZONE + 1.0, 400.0), 0, &source, &[]), Drop::Nothing);
    assert_eq!(decide(pos2(500.0, 100.0 - DEAD_ZONE + 1.0), 0, &source, &[]), Drop::Nothing);
    // Past the dead zone: a window of its own, where it was let go.
    let far = pos2(1100.0 + DEAD_ZONE + 1.0, 400.0);
    assert_eq!(decide(far, 0, &source, &[]), Drop::TearOff { at: far });
    let above = pos2(500.0, 100.0 - DEAD_ZONE - 1.0);
    assert_eq!(decide(above, 1, &source, &[]), Drop::TearOff { at: above });
}

#[test]
fn another_window_in_the_dead_zone_is_still_a_window_to_drop_on() {
    let source = window("a", rect(0.0, 0.0, 1000.0, 800.0), 2);
    let beside = window("b", rect(1010.0, 0.0, 2010.0, 800.0), 1);
    assert_eq!(decide(pos2(1020.0, 400.0), 0, &source, &[beside.clone()]), Drop::Merge { into: beside.id, at: 1 });
}

#[test]
fn a_window_made_like_another_is_as_big_but_not_bigger_than_the_screen_holds() {
    let monitor = Some(vec2(2000.0, 1000.0));
    assert_eq!(size_for_a_new_window(vec2(1240.0, 860.0), None), vec2(1240.0, 860.0));
    assert_eq!(size_for_a_new_window(vec2(1240.0, 700.0), monitor), vec2(1240.0, 700.0));
    // A maximised window is as big as the monitor: the new one is most of it.
    assert_eq!(size_for_a_new_window(vec2(2000.0, 1000.0), monitor), vec2(1600.0, 800.0));
    // And never smaller than a window can be.
    assert_eq!(size_for_a_new_window(vec2(100.0, 100.0), monitor), vec2(640.0, 480.0));
}

// -- what happens when windows leave ----------------------------------------

fn status(leaving: Option<Leaving>) -> Status {
    Status { leaving, asking: false, cancelled: false }
}

#[test]
fn a_window_that_closes_while_another_is_open_is_only_that() {
    let p = plan(&[status(Some(Leaving::Window)), status(None)], false);
    assert_eq!(p, Plan { close: vec![0], ..Plan::default() });
}

#[test]
fn the_program_ends_when_the_last_window_goes() {
    assert!(plan(&[status(Some(Leaving::Window))], false).exit);
    assert!(plan(&[status(Some(Leaving::Window)), status(Some(Leaving::Window))], false).exit);
    assert!(!plan(&[status(Some(Leaving::Window)), status(None)], false).exit);
    assert!(!plan(&[status(None)], false).exit, "nothing said it was leaving");
}

#[test]
fn a_quit_asks_every_window_in_turn_and_ends_when_all_have_agreed() {
    // One window has agreed; the other has not been asked.
    let p = plan(&[status(Some(Leaving::Program)), status(None)], false);
    assert!(!p.exit && p.quitting);
    assert_eq!(p.ask, vec![1]);
    // The other is asking its question: not asked again, and not the end.
    let asking = Status { leaving: None, asking: true, cancelled: false };
    let p = plan(&[status(Some(Leaving::Program)), asking], true);
    assert!(!p.exit && p.quitting && p.ask.is_empty());
    // Both have agreed.
    let p = plan(&[status(Some(Leaving::Program)), status(Some(Leaving::Program))], true);
    assert!(p.exit);
}

#[test]
fn a_window_closed_by_hand_during_a_quit_counts_as_agreeing() {
    let p = plan(&[status(Some(Leaving::Program)), status(Some(Leaving::Window))], true);
    assert!(p.exit);
}

#[test]
fn a_cancel_stops_a_quit_and_the_windows_that_agreed_stay_open() {
    let cancelled = Status { leaving: None, asking: false, cancelled: true };
    let p = plan(&[status(Some(Leaving::Program)), cancelled], true);
    assert!(!p.exit && !p.quitting);
    assert_eq!(p.forget, vec![0]);
    assert!(p.close.is_empty(), "a window that agreed to a quit that is off must not close");
}

#[test]
fn a_cancel_with_no_quit_going_changes_nothing() {
    let cancelled = Status { leaving: None, asking: false, cancelled: true };
    assert_eq!(plan(&[status(None), cancelled], false), Plan::default());
}

#[test]
fn quit_with_force_asks_nothing() {
    let p = plan(&[status(Some(Leaving::Now)), Status { leaving: None, asking: true, cancelled: false }], false);
    assert!(p.exit);
}

/// **The rule that protects unsaved work, over every combination.** Whatever
/// the windows say, the program does not end and no window closes unless it has
/// said it is leaving — and a window with a question up is never asked another
/// or closed under it.
#[test]
fn nothing_closes_that_did_not_say_it_was_leaving() {
    let states = [None, Some(Leaving::Window), Some(Leaving::Program), Some(Leaving::Now)];
    let per_window = states.len() * 4;
    let mut cases = 0;
    for n in 1..=3usize {
        for combo in 0..per_window.pow(n as u32) {
            let mut windows = Vec::new();
            let mut rest = combo;
            for _ in 0..n {
                let k = rest % per_window;
                rest /= per_window;
                windows.push(Status { leaving: states[k % 4], asking: (k / 4) % 2 == 1, cancelled: (k / 8) % 2 == 1 });
            }
            for quitting in [false, true] {
                cases += 1;
                let p = plan(&windows, quitting);
                for &i in &p.close {
                    assert_eq!(windows[i].leaving, Some(Leaving::Window), "closed a window that did not ask to be: {windows:?}");
                }
                for &i in &p.ask {
                    assert!(
                        windows[i].leaving.is_none() && !windows[i].asking,
                        "asked a window that cannot be asked: {windows:?}"
                    );
                }
                for &i in &p.forget {
                    assert_eq!(windows[i].leaving, Some(Leaving::Program));
                }
                if p.exit {
                    assert!(
                        windows.iter().any(|w| w.leaving == Some(Leaving::Now))
                            || windows.iter().all(|w| w.leaving.is_some()),
                        "ended the program with a window that had not said it was leaving: {windows:?}"
                    );
                }
                assert!(p.close.iter().all(|i| !p.ask.contains(i)), "a window both closes and is asked: {windows:?}");
            }
        }
    }
    assert!(cases > 4000, "the enumeration shrank: {cases}");
}

// -- what a tab carries with it ---------------------------------------------

/// **The point of the whole thing.** A tab with unsaved work in it moves, and in
/// the window it arrives in the work is still there and still undoes — neither
/// saved nor reopened.
#[test]
fn a_moved_tab_keeps_its_unsaved_marks_and_its_undo_history() {
    let mut from = app_with(&["single-page.pdf", "pages-ladder.pdf"]);
    // On the second tab: a mark (the markup layer's own history), and a change
    // to the document itself (the session's own history).
    from.submit("l 10,10 100,100");
    from.submit("l 10,50 100,150");
    from.submit("deletepage 2");
    assert_eq!(pages_of(&from.tabs[1]), 4);
    assert_eq!(marks(&from.tabs[1]), 2);
    assert!(from.tab_with_unsaved_work().is_some(), "test assumption: that is unsaved work");

    let moving = from.give_tab(1).expect("the tab would not leave");
    assert_eq!(from.tabs.len(), 1, "the tab is in both windows, or neither");
    let mut to = PagifyApp::build(None, true);
    to.take_in_tab(moving, None);

    assert_eq!(to.tabs.len(), 1);
    // Each window's command line names the document it is for.
    assert_eq!(to.cmd.prompt().document.as_deref(), Some("pages-ladder.pdf"), "the new window names no document");
    assert_eq!(from.cmd.prompt().document.as_deref(), Some("single-page.pdf"), "the old window names the one that left");
    assert_eq!(pages_of(to.tab()), 4, "the document edit did not travel");
    assert_eq!(marks(to.tab()), 2, "the marks did not travel");
    assert!(to.tab_with_unsaved_work().is_some(), "the unsaved work was lost in the move — it would close without asking");

    // Undo, in the new window, takes back the last thing done first — the
    // deleted page — and then the marks, one at a time: both histories travelled.
    to.submit("undo");
    assert_eq!(pages_of(to.tab()), 5, "undo did not take back the deleted page in the new window");
    assert_eq!(marks(to.tab()), 2);
    to.submit("undo");
    assert_eq!(marks(to.tab()), 1, "undo did not take back a mark in the new window");
    to.submit("undo");
    assert_eq!(marks(to.tab()), 0);
    // And the window it left still has what it had.
    assert_eq!(name_of(&from.tabs[0]), "single-page.pdf");
}

#[test]
fn a_moved_tab_keeps_its_page_zoom_and_view() {
    let mut from = app_with(&["pages-ladder.pdf"]);
    from.submit("page 3");
    from.tab_mut().zoom = ZoomMode::Factor(2.0);
    from.tab_mut().scroll_offset = vec2(0.0, 1234.0);
    from.tab_mut().ribbon = Tab::Edit;
    from.tab_mut().text_selection = Some(3..9);
    let (page, zoom) = (from.tab().page, from.tab().zoom);
    assert!(page > 0, "test assumption: the tab is not on its first page");

    // A second tab, so the first can leave.
    from.open(&fixture("single-page.pdf"));
    // The new one is on the left; the tab that leaves is the one that was there.
    let moving = from.give_tab(1).unwrap();
    let mut to = PagifyApp::build(None, true);
    to.take_in_tab(moving, None);

    let tab = to.tab();
    assert_eq!(tab.page, page, "the page was lost");
    assert_eq!(tab.zoom, zoom, "the zoom was lost");
    assert_eq!(tab.ribbon, Tab::Edit, "the ribbon tab was lost");
    assert_eq!(tab.text_selection, Some(3..9), "the selection was lost");
    // The window it arrives in has never held this scroll position: it is asked
    // for on the first frame there.
    assert_eq!(tab.anchor_offset, Some(vec2(0.0, 1234.0)), "the place on the page is not asked for again");
    assert!(tab.viewport_rect.is_none(), "the old window's page area was carried into the new one");
}

/// The scroll position is held by the scroll area of the window the tab is in —
/// a new window's has never held it — so a tab that moves has to ask for it
/// again, and be shown where it was left, not at the top.
#[test]
fn a_moved_tab_comes_up_in_the_new_window_where_it_was_left_on_the_page() {
    let mut app = app_with(&["pages-ladder.pdf"]);
    app.tab_mut().zoom = ZoomMode::Factor(1.5);
    app.tab_mut().anchor_offset = Some(vec2(0.0, 600.0));
    let mut h = harness_from(app);
    h.run_steps(6);
    let left_at = h.state().tab().scroll_offset;
    assert!(left_at.y > 300.0, "test assumption: the first window did not scroll: {left_at:?}");

    let mut from = h.into_state();
    let moving = from.give_tab(0).unwrap();
    let mut to = PagifyApp::build(None, true);
    to.take_in_tab(moving, None);
    let mut h = harness_from(to);
    h.run_steps(6);
    let now = h.state().tab().scroll_offset;
    assert!(
        (now.y - left_at.y).abs() < 2.0,
        "the tab was left at {left_at:?} and came up at {now:?} in the other window"
    );
}

#[test]
fn the_empty_tab_a_new_window_starts_with_is_replaced_not_kept() {
    let mut from = app_with(&["single-page.pdf", "two-column.pdf"]);
    let moving = from.give_tab(1).unwrap();
    let mut to = PagifyApp::build(None, true);
    assert_eq!(to.tabs.len(), 1);
    assert!(to.tabs[0].doc.is_none());
    to.take_in_tab(moving, Some(0));
    assert_eq!(to.tabs.len(), 1, "an empty tab was left beside the document");
    assert_eq!(name_of(to.tab()), "two-column.pdf");
}

#[test]
fn a_tab_arrives_at_the_place_asked_for_and_is_the_one_showing() {
    let mut to = app_with(&["single-page.pdf", "two-column.pdf", "pages-ladder.pdf"]);
    let mut from = app_with(&["mixed-sizes.pdf", "quadrants.pdf"]);
    let moving = from.give_tab(1).unwrap();
    let at = to.take_in_tab(moving, Some(1));
    assert_eq!(at, 1);
    assert_eq!(to.tabs.len(), 4);
    assert_eq!(name_of(&to.tabs[1]), "quadrants.pdf");
    assert_eq!(to.active_tab, 1, "the tab that arrived is not the one showing");
    // Past the end is the end.
    let moving = from.give_tab(0).unwrap();
    assert_eq!(to.take_in_tab(moving, Some(99)), 4);
    assert_eq!(name_of(&to.tabs[4]), "mixed-sizes.pdf");
}

#[test]
fn a_window_with_a_question_up_is_neither_emptied_nor_filled_in_the_middle() {
    // A question names its tab by position, and moving a tab moves every
    // position after it.
    let mut asking = app_with(&["single-page.pdf", "two-column.pdf"]);
    asking.submit("l 10,10 100,100");
    asking.submit("close");
    assert!(asking.tab().closing.is_some(), "test assumption: a question is up");
    assert!(asking.give_tab(0).is_none(), "a tab left a window with a question up");
    assert_eq!(asking.tabs.len(), 2);

    let mut from = app_with(&["mixed-sizes.pdf", "quadrants.pdf"]);
    let moving = from.give_tab(1).unwrap();
    let asking_at = asking.active_tab;
    let at = asking.take_in_tab(moving, Some(0));
    assert_eq!(at, 2, "a tab was put in front of the one being asked about");
    assert_eq!(asking.active_tab, asking_at, "the arriving tab covered the question");
    assert!(asking.tabs[asking_at].closing.is_some(), "the question was lost");
}

#[test]
fn reordering_keeps_the_same_tab_showing() {
    for from in 0..4 {
        for to in 0..4 {
            for active in 0..4 {
                let mut app = app_with(&["single-page.pdf", "two-column.pdf", "pages-ladder.pdf", "quadrants.pdf"]);
                app.active_tab = active;
                let showing = name_of(app.tab());
                app.reorder_tab(from, to);
                assert_eq!(name_of(app.tab()), showing, "moving {from} to {to} changed the tab showing (was {active})");
                assert_eq!(app.tabs.len(), 4);
            }
        }
    }
    let mut app = app_with(&["single-page.pdf", "two-column.pdf", "pages-ladder.pdf"]);
    app.reorder_tab(0, 2);
    let names: Vec<String> = app.tabs.iter().map(name_of).collect();
    assert_eq!(names, ["two-column.pdf", "pages-ladder.pdf", "single-page.pdf"]);
}

#[test]
fn a_window_left_with_no_tab_says_it_is_leaving() {
    let mut hub = Hub::new(app_with(&["single-page.pdf"]), Handover::default());
    let moving = hub.windows[0].app.give_tab(0).unwrap();
    hub.after_giving(0);
    assert_eq!(hub.windows[0].app.win.leaving, Some(Leaving::Window));
    drop(moving);
}

// -- windows, as the hub keeps them -------------------------------------------

fn hub_of(names: &[&str]) -> Hub {
    Hub::new(app_with(names), Handover::default())
}

#[test]
fn tearing_a_tab_off_makes_a_window_of_its_own_with_that_very_tab() {
    let ctx = egui::Context::default();
    let mut hub = hub_of(&["single-page.pdf", "pages-ladder.pdf"]);
    hub.windows[0].app.submit("l 10,10 100,100");
    let doc = hub.windows[0].app.tabs[1].doc.as_ref().unwrap().id;

    hub.tear_off(&ctx, 0, 1, pos2(900.0, 300.0), vec2(1240.0, 860.0));

    assert_eq!(hub.windows.len(), 2);
    assert_eq!(hub.windows[0].app.tabs.len(), 1, "the tab is in both windows");
    assert_eq!(name_of(hub.windows[0].app.tab()), "single-page.pdf");
    let new = &hub.windows[1];
    assert_eq!(new.app.tabs.len(), 1);
    assert_eq!(new.app.tab().doc.as_ref().unwrap().id, doc, "it is another document, not the same live tab");
    assert_eq!(marks(new.app.tab()), 1, "the unsaved mark did not travel");
    assert_eq!(new.app.win.serial, 1, "the new window shares the first one's command box id");
    assert_ne!(new.vp, ViewportId::ROOT);
    assert_eq!(hub.mru[0], new.vp, "the new window is the one used last");
    assert_eq!(hub.focus_next, Some(new.vp), "the new window is not asked to take the focus");
    // Where it was let go is where it opens, with the tab strip under the pointer.
    let at = new.place.at.expect("a place");
    assert!(at.x < 900.0 && at.y < 300.0, "the window opens down and right of the pointer: {at:?}");
    assert_eq!(new.place.size, vec2(1240.0, 860.0));
}

#[test]
fn tearing_off_the_last_tab_of_a_window_is_a_moved_window() {
    let ctx = egui::Context::default();
    let mut hub = hub_of(&["single-page.pdf"]);
    hub.tear_off(&ctx, 0, 0, pos2(900.0, 300.0), vec2(1240.0, 860.0));
    assert_eq!(hub.windows.len(), 2);
    assert_eq!(hub.windows[0].app.win.leaving, Some(Leaving::Window), "the emptied window stays open");
    assert!(!hub.settle(&ctx), "the program ended with a window still open");
    assert_eq!(hub.windows.len(), 1, "the emptied window did not close");
    assert_eq!(name_of(hub.windows[0].app.tab()), "single-page.pdf");
    assert_ne!(hub.windows[0].vp, ViewportId::ROOT, "the first window is gone and what is left is the new one");
}

#[test]
fn a_tab_with_a_question_up_cannot_be_torn_off() {
    let ctx = egui::Context::default();
    let mut hub = hub_of(&["single-page.pdf", "two-column.pdf"]);
    hub.windows[0].app.submit("l 10,10 100,100");
    hub.windows[0].app.submit("close");
    hub.tear_off(&ctx, 0, 1, pos2(900.0, 300.0), vec2(1240.0, 860.0));
    assert_eq!(hub.windows.len(), 1, "a window was made from a tab that was being asked about");
    assert_eq!(hub.windows[0].app.tabs.len(), 2);
}

#[test]
fn merging_moves_the_tab_and_the_emptied_window_goes() {
    let ctx = egui::Context::default();
    let mut hub = hub_of(&["single-page.pdf", "two-column.pdf"]);
    hub.tear_off(&ctx, 0, 1, pos2(900.0, 300.0), vec2(1240.0, 860.0));
    let second = hub.windows[1].vp;
    // The first window's last tab joins the new window, in front of the other.
    hub.merge(&ctx, 0, 0, second, 0);

    assert_eq!(hub.windows[1].app.tabs.len(), 2);
    assert_eq!(name_of(&hub.windows[1].app.tabs[0]), "single-page.pdf");
    assert_eq!(name_of(&hub.windows[1].app.tabs[1]), "two-column.pdf");
    assert_eq!(hub.windows[1].app.active_tab, 0, "the tab that arrived is not showing");
    assert_eq!(hub.windows[0].app.win.leaving, Some(Leaving::Window));
    assert!(!hub.settle(&ctx));
    assert_eq!(hub.windows.len(), 1);
    assert_eq!(hub.windows[0].vp, second);
    assert_eq!(hub.mru, vec![second], "a window that is gone is still remembered as the one used last");
}

#[test]
fn closing_one_of_two_windows_leaves_the_program_running_and_the_last_ends_it() {
    let ctx = egui::Context::default();
    let mut hub = hub_of(&["single-page.pdf", "two-column.pdf"]);
    hub.tear_off(&ctx, 0, 1, pos2(900.0, 300.0), vec2(1240.0, 860.0));
    hub.windows[1].app.win.leaving = Some(Leaving::Window);
    assert!(!hub.settle(&ctx), "the program ended with a window still open");
    assert_eq!(hub.windows.len(), 1);
    hub.windows[0].app.win.leaving = Some(Leaving::Window);
    assert!(hub.settle(&ctx), "the last window closed and the program did not end");
}

#[test]
fn a_quit_asks_about_the_unsaved_work_in_every_window_and_a_cancel_anywhere_stops_it() {
    let ctx = egui::Context::default();
    let mut hub = hub_of(&["single-page.pdf", "two-column.pdf"]);
    hub.tear_off(&ctx, 0, 1, pos2(900.0, 300.0), vec2(1240.0, 860.0));
    hub.windows[0].app.submit("l 10,10 100,100");
    hub.windows[1].app.submit("l 20,20 120,120");

    // `quit` in the first window: it asks about its own tab.
    hub.windows[0].app.submit("quit");
    assert_eq!(hub.windows[0].app.tab().closing, Some(crate::Closing::Program));
    assert!(!hub.settle(&ctx));
    assert!(hub.windows[1].app.win.leaving.is_none() && hub.windows[1].app.tab().closing.is_none(), "asked too soon");

    // It is answered (Discard): that window agrees, and the next is asked.
    hub.windows[0].app.tab_mut().closing = None;
    hub.windows[0].app.finish_closing(crate::Closing::Program, &ctx);
    assert_eq!(hub.windows[0].app.win.leaving, Some(Leaving::Program));
    assert!(!hub.settle(&ctx));
    assert_eq!(hub.windows[1].app.tab().closing, Some(crate::Closing::Program), "the other window was not asked");
    assert!(hub.quitting);

    // Cancel there: the quit is off, and the first window is not left half gone.
    hub.windows[1].app.tab_mut().closing = None;
    hub.windows[1].app.win.quit_cancelled = true;
    assert!(!hub.settle(&ctx));
    assert!(!hub.quitting);
    assert!(hub.windows[0].app.win.leaving.is_none(), "a window that agreed to a quit that was cancelled still wants to go");
    assert_eq!(hub.windows.len(), 2);
    assert!(hub.windows[1].app.tab_with_unsaved_work().is_some(), "the unsaved work is still there");

    // Ask again and answer both: now it ends.
    hub.windows[0].app.submit("quit");
    hub.windows[0].app.tab_mut().closing = None;
    hub.windows[0].app.finish_closing(crate::Closing::Program, &ctx);
    assert!(!hub.settle(&ctx));
    hub.windows[1].app.tab_mut().closing = None;
    hub.windows[1].app.finish_closing(crate::Closing::Program, &ctx);
    assert!(hub.settle(&ctx), "every window agreed and the program did not end");
}

#[test]
fn an_update_staged_in_one_window_is_carried_out_by_the_program() {
    let ctx = egui::Context::default();
    let mut hub = hub_of(&["single-page.pdf", "two-column.pdf"]);
    hub.tear_off(&ctx, 0, 1, pos2(900.0, 300.0), vec2(1240.0, 860.0));
    let source = std::env::temp_dir();
    hub.windows[1].app.pending_update = Some(source.clone());
    hub.windows[1].app.submit("quit");
    assert!(!hub.settle(&ctx), "the other window has not been asked yet");
    assert_eq!(hub.pending_update, Some(source.clone()), "the staged update was forgotten before the program could exit");
    // A cancel anywhere takes the update with it.
    hub.windows[0].app.win.quit_cancelled = true;
    assert!(!hub.settle(&ctx));
    assert!(hub.pending_update.is_none(), "an update survived a cancelled quit");
    assert!(hub.windows[1].app.pending_update.is_none());
}

#[test]
fn closing_the_last_tab_closes_the_window_but_not_the_other_windows() {
    let ctx = egui::Context::default();
    let mut hub = hub_of(&["single-page.pdf", "two-column.pdf"]);
    hub.tear_off(&ctx, 0, 1, pos2(900.0, 300.0), vec2(1240.0, 860.0));
    // The × on the only tab of the new window.
    hub.windows[1].app.close_tab(0);
    assert_eq!(hub.windows[1].app.win.leaving, Some(Leaving::Window));
    assert!(!hub.settle(&ctx));
    assert_eq!(hub.windows.len(), 1);
    assert_eq!(name_of(hub.windows[0].app.tab()), "single-page.pdf");
}

#[test]
fn a_window_with_unsaved_work_asks_before_it_closes_and_only_about_its_own_tabs() {
    let ctx = egui::Context::default();
    let mut hub = hub_of(&["single-page.pdf", "two-column.pdf"]);
    hub.tear_off(&ctx, 0, 1, pos2(900.0, 300.0), vec2(1240.0, 860.0));
    hub.windows[0].app.submit("l 10,10 100,100");
    // The second window's close button, with nothing unsaved in it, closes it.
    hub.windows[1].app.ask_to_leave(Leaving::Window);
    assert_eq!(hub.windows[1].app.win.leaving, Some(Leaving::Window));
    // The first window's, with a mark in it, asks — and the other window is not
    // part of the question.
    hub.windows[0].app.ask_to_leave(Leaving::Window);
    assert!(hub.windows[0].app.win.leaving.is_none(), "a window with unsaved work said it was leaving without asking");
    assert_eq!(hub.windows[0].app.tab().closing, Some(crate::Closing::Program));
    assert!(!hub.settle(&ctx));
    assert_eq!(hub.windows.len(), 1, "the clean window did not close, or the dirty one did");
    assert!(hub.windows[0].app.tab_with_unsaved_work().is_some());
    // Cancel: it stays, with its work.
    hub.windows[0].app.tab_mut().closing = None;
    assert_eq!(marks(hub.windows[0].app.tab()), 1);
    // Discard: it goes — and that is the last window, so the program ends.
    hub.windows[0].app.ask_to_leave(Leaving::Window);
    hub.windows[0].app.tab_mut().closing = None;
    hub.windows[0].app.finish_closing(crate::Closing::Program, &ctx);
    assert_eq!(hub.windows[0].app.win.leaving, Some(Leaving::Window));
    assert!(hub.settle(&ctx));
}

#[test]
fn what_is_the_programs_is_one_copy_whichever_window_is_working() {
    let ctx = egui::Context::default();
    let mut hub = hub_of(&["single-page.pdf", "two-column.pdf"]);
    hub.tear_off(&ctx, 0, 1, pos2(900.0, 300.0), vec2(1240.0, 860.0));
    // Something copied in one window is there to paste in the other.
    hub.with_window(0, |a| a.object_clipboard = Some(crate::ObjectClipboard::Shapes(Vec::new())));
    assert!(hub.with_window(1, |b| b.object_clipboard.is_some()), "a copy made in one window cannot be pasted in the other");
    // A document opened in one window is in the recent list of the other.
    hub.with_window(0, |a| a.recent.record(std::path::Path::new("some-recent.pdf"), 3, 1));
    let seen = hub.with_window(1, |b| b.recent.entries.len());
    assert_eq!(seen, hub.with_window(0, |a| a.recent.entries.len()), "two windows, two recent lists");
    assert!(seen >= 1);
    // And outside a frame a window holds none of it: it is on loan.
    assert!(hub.windows[1].app.recent.entries.is_empty() && hub.windows[1].app.object_clipboard.is_none());
}

// -- documents handed over from outside, with several windows ------------------

#[test]
fn a_document_handed_over_goes_to_the_window_used_last() {
    let ctx = egui::Context::default();
    let mut hub = hub_of(&["single-page.pdf", "two-column.pdf"]);
    hub.tear_off(&ctx, 0, 1, pos2(900.0, 300.0), vec2(1240.0, 860.0));
    // The new window was used last; then the first.
    assert_eq!(hub.target(), 1);
    hub.touch(ViewportId::ROOT);
    assert_eq!(hub.target(), 0);

    let request = Request { files: vec![PathBuf::from(fixture("quadrants.pdf"))], commands: Vec::new() };
    hub.accept(request, &ctx);
    assert_eq!(hub.windows[0].app.tabs.len(), 2, "it did not become a tab of the window used last");
    assert_eq!(name_of(hub.windows[0].app.tab()), "quadrants.pdf");
    assert_eq!(hub.windows[1].app.tabs.len(), 1, "it went to the other window");
}

#[test]
fn a_document_open_in_another_window_is_shown_there_not_opened_again() {
    let ctx = egui::Context::default();
    let mut hub = hub_of(&["single-page.pdf", "two-column.pdf"]);
    hub.tear_off(&ctx, 0, 1, pos2(900.0, 300.0), vec2(1240.0, 860.0));
    hub.touch(ViewportId::ROOT);
    // two-column.pdf is open in the second window; the first was used last.
    let request = Request { files: vec![PathBuf::from(fixture("two-column.pdf"))], commands: Vec::new() };
    hub.accept(request, &ctx);
    assert_eq!(hub.windows[0].app.tabs.len(), 1, "a second copy of an open document was opened");
    assert_eq!(hub.windows[1].app.tabs.len(), 1);
    assert_eq!(hub.target(), 1, "the window that has it is not the one used last now");
}

#[test]
fn showing_a_document_that_is_open_does_not_cover_a_question_that_is_up() {
    let ctx = egui::Context::default();
    let mut hub = hub_of(&["single-page.pdf", "two-column.pdf"]);
    hub.windows[0].app.active_tab = 0;
    hub.windows[0].app.submit("l 10,10 100,100");
    hub.windows[0].app.submit("close");
    assert!(hub.windows[0].app.tab().closing.is_some(), "test assumption: a question is up");
    let request = Request { files: vec![PathBuf::from(fixture("two-column.pdf"))], commands: Vec::new() };
    hub.accept(request, &ctx);
    assert_eq!(hub.windows[0].app.active_tab, 0, "the question was covered by showing another tab");
    assert!(hub.windows[0].app.tabs[0].closing.is_some());
    assert_eq!(hub.windows[0].app.tabs.len(), 2);
}

// -- the gesture, in a real strip --------------------------------------------

/// A window as eframe reports it: where it is on the screen.
fn placed(h: &mut egui_kittest::Harness<'static, PagifyApp>, at: Pos2) {
    let info = h.input_mut().viewports.get_mut(&ViewportId::ROOT).unwrap();
    info.inner_rect = Some(Rect::from_min_size(at, vec2(1400.0, 1000.0)));
    info.outer_rect = Some(Rect::from_min_size(at - vec2(8.0, 31.0), vec2(1416.0, 1039.0)));
}

/// Where tab `name` is drawn, found the way a person finds it.
fn tab_centre(h: &egui_kittest::Harness<'static, PagifyApp>, name: &str) -> Pos2 {
    use egui_kittest::kittest::Queryable;
    h.get_all_by_label(name)
        .map(|n| n.rect())
        .find(|r| r.center().y < 70.0)
        .unwrap_or_else(|| panic!("no tab called {name} in the title bar"))
        .center()
}

fn press_and_carry(h: &mut egui_kittest::Harness<'static, PagifyApp>, from: Pos2, via: &[Pos2]) {
    h.hover_at(from);
    h.drag_at(from);
    for p in via {
        h.hover_at(*p);
        h.run_steps(1);
    }
}

#[test]
fn carrying_a_tab_lifts_it_and_letting_go_reports_where() {
    let app = app_with(&["single-page.pdf", "two-column.pdf"]);
    let mut h = harness_from(app);
    placed(&mut h, pos2(300.0, 200.0));
    h.run_steps(2);
    let tab = tab_centre(&h, "two-column.pdf");

    press_and_carry(&mut h, tab, &[tab + vec2(30.0, 5.0), pos2(1700.0, 400.0)]);
    assert!(h.state().dragging_tab(1), "the tab was not picked up");
    assert!(h.state().win.out.is_none(), "it was let go before it was");

    h.drop_at(pos2(1700.0, 400.0));
    h.run_steps(1);
    let out = h.state_mut().win.out.take().expect("letting go did not report");
    assert_eq!(out.tab, 1);
    // In the window's own points, plus where the window is on the screen.
    assert_eq!(out.screen, Some(pos2(300.0 + 1700.0, 200.0 + 400.0)));
    assert!(!h.state().dragging_tab(1), "the tab is still lifted");
    assert_eq!(h.state().tabs.len(), 2, "the window took the tab out by itself: that is the hub's to decide");
}

/// Every filled rectangle of the last frame, with its colour.
fn filled_rects(h: &egui_kittest::Harness<'static, PagifyApp>) -> Vec<(Rect, egui::Color32)> {
    fn flat(shape: &egui::Shape, out: &mut Vec<(Rect, egui::Color32)>) {
        match shape {
            egui::Shape::Vec(v) => v.iter().for_each(|s| flat(s, out)),
            egui::Shape::Rect(r) => out.push((r.rect, r.fill)),
            _ => {}
        }
    }
    let mut out = Vec::new();
    for s in &h.output().shapes {
        flat(&s.shape, &mut out);
    }
    out
}

#[test]
fn a_tab_being_carried_is_drawn_under_the_pointer_and_its_place_is_marked() {
    let app = app_with(&["single-page.pdf", "two-column.pdf", "pages-ladder.pdf"]);
    let mut h = harness_from(app);
    placed(&mut h, pos2(300.0, 200.0));
    h.run_steps(2);
    let tab = tab_centre(&h, "single-page.pdf");
    let last = tab_centre(&h, "pages-ladder.pdf");
    let carried = egui::Color32::from(theme::violet().gamma_multiply(0.9));
    let marker = theme::violet_bright();
    assert!(!filled_rects(&h).iter().any(|(_, c)| *c == carried), "a copy of a tab is drawn with none carried");

    // Carried along the strip, to just right of the last tab.
    let to = pos2(last.x + 100.0, last.y);
    press_and_carry(&mut h, tab, &[tab + vec2(25.0, 0.0), to]);
    let rects = filled_rects(&h);
    let copy = rects.iter().find(|(_, c)| *c == carried).expect("the copy of the tab under the pointer").0;
    assert!(copy.contains(to), "the copy is not under the pointer: {copy:?} for the pointer at {to:?}");
    // And where it would land: a bar after the last of the other tabs.
    let bar = rects.iter().find(|(r, c)| *c == marker && r.width() <= 4.0).expect("the place it would take").0;
    let others_right = tab_centre(&h, "pages-ladder.pdf").x;
    assert!(bar.center().x > others_right, "the marker is not after the last tab: {bar:?}");

    h.drop_at(to);
    h.run_steps(2);
    assert!(
        !filled_rects(&h).iter().any(|(_, c)| *c == carried),
        "the copy of the tab stayed on screen after it was let go"
    );
}

#[test]
fn a_tab_carried_over_this_window_from_another_marks_its_place_or_outlines_the_window() {
    let marker = theme::violet_bright();
    let bars = |h: &egui_kittest::Harness<'static, PagifyApp>| {
        filled_rects(h).iter().filter(|(r, c)| *c == marker && r.width() <= 4.0).count()
    };
    let app = app_with(&["single-page.pdf", "two-column.pdf"]);
    let mut h = harness_from(app);
    h.run_steps(2);
    assert_eq!(bars(&h), 0);

    // Over the strip, between the two tabs: a bar there.
    h.state_mut().win.hint = Some(DropHint { over_strip: true, at: 1 });
    h.run_steps(1);
    assert_eq!(bars(&h), 1, "no marker for the place a tab would take");

    // Over the page: the whole window is outlined instead, and there is no bar.
    h.state_mut().win.hint = Some(DropHint { over_strip: false, at: 2 });
    h.run_steps(1);
    assert_eq!(bars(&h), 0);
    let stroked = h.output().shapes.iter().any(|s| matches!(&s.shape, egui::Shape::Rect(r) if r.stroke.color == marker && r.stroke.width >= 3.0));
    assert!(stroked, "the window was not outlined");

    h.state_mut().win.hint = None;
    h.run_steps(1);
    assert_eq!(bars(&h), 0);
}

#[test]
fn escape_while_carrying_puts_the_tab_back_and_is_not_also_an_escape_elsewhere() {
    let app = app_with(&["single-page.pdf", "two-column.pdf"]);
    let mut h = harness_from(app);
    placed(&mut h, pos2(300.0, 200.0));
    h.run_steps(2);
    let tab = tab_centre(&h, "two-column.pdf");
    // A tool in hand: an Escape that reached the rest of the window would put it down.
    h.state_mut().submit("l");
    assert!(h.state_mut().tab_mut().pending.is_some(), "test assumption: a tool is armed");

    press_and_carry(&mut h, tab, &[tab + vec2(40.0, 10.0), pos2(1700.0, 400.0)]);
    assert!(h.state().dragging_tab(1));
    h.key_press(egui::Key::Escape);
    h.run_steps(1);
    // (egui ends the drag itself on Escape, in the same frame, so the tab is
    // put down without being reported — what matters is that nothing came of it.)
    assert!(h.state().win.drag.is_none(), "the tab is still lifted after Escape");
    assert!(h.state().win.out.is_none(), "Escape was reported as a release");
    assert!(h.state_mut().tab_mut().pending.is_some(), "the Escape also put the armed tool down");

    h.drop_at(pos2(1700.0, 400.0));
    h.run_steps(2);
    assert!(h.state().win.out.is_none(), "a cancelled drag still reported a release");
    assert!(h.state().win.drag.is_none());
    assert_eq!(h.state().tabs.len(), 2);
}

/// The window's own half of the cancel: Escape pressed while the drag is still
/// going on, as far as the window is concerned, ends it unreported even where
/// egui would let the drag run on.
#[test]
fn a_cancelled_drag_is_never_reported_when_it_is_let_go() {
    let app = app_with(&["single-page.pdf", "two-column.pdf"]);
    let mut h = harness_from(app);
    placed(&mut h, pos2(300.0, 200.0));
    h.run_steps(2);
    let tab = tab_centre(&h, "two-column.pdf");
    press_and_carry(&mut h, tab, &[tab + vec2(40.0, 10.0), pos2(1700.0, 400.0)]);
    h.state_mut().win.drag.as_mut().expect("lifted").cancelled = true;
    h.drop_at(pos2(1700.0, 400.0));
    h.run_steps(2);
    assert!(h.state().win.out.is_none(), "a cancelled drag was reported when let go");
    assert!(h.state().win.drag.is_none());
}

#[test]
fn a_press_on_the_close_button_does_not_pick_the_tab_up() {
    let app = app_with(&["single-page.pdf", "two-column.pdf"]);
    let mut h = harness_from(app);
    placed(&mut h, pos2(300.0, 200.0));
    h.run_steps(2);
    let tab = tab_centre(&h, "two-column.pdf");
    // The × is the right-hand end of the tab's pill.
    let close = {
        use egui_kittest::kittest::Queryable;
        let r = h.get_all_by_label("two-column.pdf").map(|n| n.rect()).find(|r| r.center().y < 70.0).unwrap();
        pos2(r.right() - 20.0, tab.y)
    };
    press_and_carry(&mut h, close, &[close + vec2(40.0, 10.0), close + vec2(300.0, 80.0)]);
    assert!(h.state().win.drag.is_none(), "a drag that began on the × picked the tab up");
}

#[test]
fn a_click_on_a_tab_still_selects_it() {
    let app = app_with(&["single-page.pdf", "two-column.pdf"]);
    let mut h = harness_from(app);
    placed(&mut h, pos2(300.0, 200.0));
    h.run_steps(2);
    assert_eq!(h.state().active_tab, 1);
    let first = tab_centre(&h, "single-page.pdf");
    h.hover_at(first);
    h.drag_at(first);
    h.run_steps(1);
    h.drop_at(first);
    h.run_steps(2);
    assert_eq!(h.state().active_tab, 0, "a click on a tab no longer selects it");
    assert!(h.state().win.out.is_none() && h.state().win.drag.is_none());
}

#[test]
fn a_drag_with_no_window_geometry_reports_nothing_rather_than_guess() {
    // No inner_rect: the window cannot say where on the screen it is.
    let app = app_with(&["single-page.pdf", "two-column.pdf"]);
    let mut h = harness_from(app);
    h.run_steps(2);
    let tab = tab_centre(&h, "two-column.pdf");
    press_and_carry(&mut h, tab, &[tab + vec2(30.0, 5.0), pos2(1700.0, 400.0)]);
    h.drop_at(pos2(1700.0, 400.0));
    h.run_steps(1);
    let out = h.state_mut().win.out.take().expect("a release was not reported");
    assert_eq!(out.screen, None);
    // Which the hub does nothing with.
    let ctx = h.ctx.clone();
    let mut hub = Hub::new(app_with(&["single-page.pdf", "two-column.pdf"]), Handover::default());
    hub.drop_tab(&ctx, 0, out);
    assert_eq!(hub.windows.len(), 1);
    assert_eq!(hub.windows[0].app.tabs.len(), 2);
}

#[test]
fn the_cancel_button_of_the_question_ends_a_quit_that_was_walking_the_windows() {
    use egui_kittest::kittest::Queryable;
    let mut app = app_with(&["single-page.pdf"]);
    app.submit("l 10,10 100,100");
    app.ask_to_leave(Leaving::Program);
    assert_eq!(app.tab().closing, Some(crate::Closing::Program));
    let mut h = harness_from(app);
    h.run_steps(2);
    h.get_by_label("Cancel").click();
    h.run_steps(2);
    assert!(h.state().win.quit_cancelled, "Cancel did not say so — a quit would ask the next window anyway");
    assert!(h.state().tab().closing.is_none());
    assert!(h.state().win.leaving.is_none());
}

#[test]
fn the_question_says_window_when_there_is_another_window() {
    use egui_kittest::kittest::Queryable;
    let mut app = app_with(&["single-page.pdf"]);
    app.submit("l 10,10 100,100");
    app.win.others = 1;
    app.ask_to_leave(Leaving::Window);
    let mut h = harness_from(app);
    h.run_steps(2);
    let _ = h.get_by_label("Close this window?");
    // On its own, closing the window is quitting.
    let mut alone = app_with(&["single-page.pdf"]);
    alone.submit("l 10,10 100,100");
    alone.ask_to_leave(Leaving::Window);
    let mut h = harness_from(alone);
    h.run_steps(2);
    let _ = h.get_by_label("Quit Pagify?");
}

#[test]
fn the_first_window_keeps_its_command_box_and_the_others_have_their_own() {
    let mut a = PagifyApp::new(None);
    let mut b = PagifyApp::build(None, true);
    b.win.serial = 1;
    a.win.serial = 0;
    assert_eq!(a.command_id(), egui::Id::new(crate::COMMAND_INPUT));
    assert_ne!(a.command_id(), b.command_id(), "two windows' command boxes share one text state");
}

/// The close button of a window, as eframe reports it: a close event on the
/// viewport the window is drawn in.
fn press_the_close_button(h: &mut egui_kittest::Harness<'static, PagifyApp>) {
    h.input_mut().viewports.get_mut(&ViewportId::ROOT).unwrap().events.push(egui::ViewportEvent::Close);
    h.run_steps(1);
}

#[test]
fn the_close_button_of_a_window_with_nothing_unsaved_says_the_window_is_leaving() {
    let mut h = harness_from(app_with(&["single-page.pdf", "two-column.pdf"]));
    press_the_close_button(&mut h);
    assert_eq!(h.state().win.leaving, Some(Leaving::Window));
    assert_eq!(h.state().tabs.len(), 2, "the window took its own tabs down: that is the hub's to do");
}

#[test]
fn the_close_button_of_a_window_with_unsaved_work_asks_instead_of_leaving() {
    let mut app = app_with(&["single-page.pdf", "two-column.pdf"]);
    // The mark is on the first tab, and the second is the one showing.
    app.active_tab = 0;
    app.submit("l 10,10 100,100");
    app.active_tab = 1;
    let mut h = harness_from(app);
    press_the_close_button(&mut h);
    assert!(h.state().win.leaving.is_none(), "a window with unsaved work said it was leaving");
    assert_eq!(h.state().active_tab, 0, "the question is not about the tab with the work in it");
    assert_eq!(h.state().tab().closing, Some(crate::Closing::Program));
    assert_eq!(h.state().win.closing_leaves, Leaving::Window, "the button of one window is not a quit");
}

// -- the hub as eframe runs it ---------------------------------------------------

fn hub_harness(hub: Hub) -> egui_kittest::Harness<'static, Hub> {
    let mut h = egui_kittest::Harness::builder().with_size(vec2(1400.0, 1000.0)).build_ui_state(
        |ui, hub: &mut Hub| {
            let mut frame = eframe::Frame::_new_kittest();
            eframe::App::ui(hub, ui, &mut frame);
        },
        hub,
    );
    h.run_steps(4);
    h
}

#[test]
fn a_hub_with_one_window_draws_it_as_the_app_alone_does() {
    use egui_kittest::kittest::Queryable;
    let h = hub_harness(hub_of(&["single-page.pdf", "two-column.pdf"]));
    let titles: Vec<egui::Rect> = ["single-page.pdf", "two-column.pdf"]
        .iter()
        .map(|n| h.get_all_by_label(n).map(|x| x.rect()).find(|r| r.center().y < 70.0).expect("tab in the title bar"))
        .collect();
    assert!(titles[0].right() <= titles[1].left(), "the tabs are out of order: {titles:?}");
    assert_eq!(h.state().windows.len(), 1);
    assert_eq!(h.state().windows[0].vp, ViewportId::ROOT);
}

/// Letting go of a tab far outside its window, in the real frame loop: the
/// strip reports, the hub decides, and there is a second window.
#[test]
fn a_tab_let_go_far_outside_its_window_becomes_a_window_in_the_frame_loop() {
    let mut h = hub_harness(hub_of(&["single-page.pdf", "two-column.pdf"]));
    {
        let info = h.input_mut().viewports.get_mut(&ViewportId::ROOT).unwrap();
        info.inner_rect = Some(Rect::from_min_size(pos2(300.0, 200.0), vec2(1400.0, 1000.0)));
        info.outer_rect = Some(Rect::from_min_size(pos2(292.0, 169.0), vec2(1416.0, 1039.0)));
    }
    h.run_steps(2);
    let tab = {
        use egui_kittest::kittest::Queryable;
        h.get_all_by_label("two-column.pdf").map(|n| n.rect()).find(|r| r.center().y < 70.0).unwrap().center()
    };
    h.hover_at(tab);
    h.drag_at(tab);
    for p in [tab + vec2(30.0, 5.0), pos2(1500.0, 500.0), pos2(2100.0, 500.0)] {
        h.hover_at(p);
        h.run_steps(1);
    }
    h.drop_at(pos2(2100.0, 500.0));
    h.run_steps(3);

    let hub = h.state();
    assert_eq!(hub.windows.len(), 2, "no window was made");
    assert_eq!(hub.windows[0].app.tabs.len(), 1);
    assert_eq!(name_of(hub.windows[0].app.tab()), "single-page.pdf");
    assert_eq!(name_of(hub.windows[1].app.tab()), "two-column.pdf");
    // It opens where it was let go (300 + 2100, 200 + 500), a little up and
    // left so the pointer is over its tab strip.
    let at = hub.windows[1].place.at.unwrap();
    assert!(at.x > 2000.0 && at.x < 2400.0 && at.y > 500.0 && at.y < 700.0, "opened somewhere else: {at:?}");
}

#[test]
fn a_tab_let_go_in_its_own_strip_in_the_frame_loop_changes_its_place() {
    let mut h = hub_harness(hub_of(&["single-page.pdf", "two-column.pdf", "pages-ladder.pdf"]));
    {
        let info = h.input_mut().viewports.get_mut(&ViewportId::ROOT).unwrap();
        info.inner_rect = Some(Rect::from_min_size(pos2(300.0, 200.0), vec2(1400.0, 1000.0)));
        info.outer_rect = Some(Rect::from_min_size(pos2(292.0, 169.0), vec2(1416.0, 1039.0)));
    }
    h.run_steps(2);
    let (first, last) = {
        use egui_kittest::kittest::Queryable;
        let find = |name: &str| h.get_all_by_label(name).map(|n| n.rect()).find(|r| r.center().y < 70.0).unwrap();
        (find("single-page.pdf"), find("pages-ladder.pdf"))
    };
    // The first tab is carried to the right of the last, along the strip.
    let from = first.center();
    let to = pos2(last.right() + 40.0, last.center().y);
    h.hover_at(from);
    h.drag_at(from);
    for p in [from + vec2(25.0, 0.0), pos2((from.x + to.x) / 2.0, to.y), to] {
        h.hover_at(p);
        h.run_steps(1);
    }
    h.drop_at(to);
    h.run_steps(3);
    let names: Vec<String> = h.state().windows[0].app.tabs.iter().map(name_of).collect();
    assert_eq!(names, ["two-column.pdf", "pages-ladder.pdf", "single-page.pdf"]);
    assert_eq!(h.state().windows.len(), 1, "a reorder made a window");
}

/// What each kind of release does, through the hub's own geometry, with two real
/// windows' worth of strips.
#[test]
fn what_the_hub_does_with_each_kind_of_release() {
    let ctx = egui::Context::default();
    let mut hub = hub_of(&["single-page.pdf", "two-column.pdf", "pages-ladder.pdf"]);
    hub.tear_off(&ctx, 0, 2, pos2(900.0, 300.0), vec2(1240.0, 860.0));
    let other = hub.windows[1].vp;

    // Where each window is, and what its strip looks like (own points, strip
    // 50 high at the top, tabs on one row), the way a frame would have said.
    let a = Rect::from_min_size(pos2(0.0, 0.0), vec2(1000.0, 800.0));
    let b = Rect::from_min_size(pos2(1100.0, 0.0), vec2(1000.0, 800.0));
    let mut raw = egui::RawInput::default();
    for (vp, outer) in [(ViewportId::ROOT, a), (other, b)] {
        let info = raw.viewports.entry(vp).or_default();
        info.outer_rect = Some(outer);
        info.inner_rect = Some(outer);
    }
    let _ = ctx.run_ui(raw, |_| {});
    for w in &mut hub.windows {
        let n = w.app.tabs.len();
        w.app.win.strip = Some(StripGeom {
            panel: rect(0.0, 0.0, 1000.0, 50.0),
            tabs: (0..n).map(|i| rect(10.0 + 110.0 * i as f32, 10.0, 110.0 + 110.0 * i as f32, 40.0)).collect(),
        });
    }
    let names = |hub: &Hub, w: usize| -> Vec<String> { hub.windows[w].app.tabs.iter().map(name_of).collect() };
    assert_eq!(names(&hub, 0), ["single-page.pdf", "two-column.pdf"]);
    assert_eq!(names(&hub, 1), ["pages-ladder.pdf"]);

    // Nothing: let go over the page.
    hub.drop_tab(&ctx, 0, TabOut { tab: 1, screen: Some(pos2(500.0, 500.0)) });
    assert_eq!(names(&hub, 0), ["single-page.pdf", "two-column.pdf"]);
    // Reorder: let go left of the first tab, in the strip.
    hub.drop_tab(&ctx, 0, TabOut { tab: 1, screen: Some(pos2(2.0, 25.0)) });
    assert_eq!(names(&hub, 0), ["two-column.pdf", "single-page.pdf"]);
    // Merge: let go on the other window's strip, in front of its tab.
    hub.drop_tab(&ctx, 0, TabOut { tab: 0, screen: Some(pos2(1100.0 + 5.0, 25.0)) });
    assert_eq!(names(&hub, 0), ["single-page.pdf"]);
    assert_eq!(names(&hub, 1), ["two-column.pdf", "pages-ladder.pdf"]);
    assert_eq!(hub.windows[1].app.active_tab, 0, "the tab that arrived is not showing");
    assert_eq!(hub.mru[0], other, "the window it joined is not the one used last");
    // Tear-off: let go well outside both windows.
    hub.drop_tab(&ctx, 1, TabOut { tab: 1, screen: Some(pos2(1500.0, 1500.0)) });
    assert_eq!(hub.windows.len(), 3);
    assert_eq!(names(&hub, 1), ["two-column.pdf"]);
    assert_eq!(names(&hub, 2), ["pages-ladder.pdf"]);
    // And a release the window could not place is nothing.
    hub.drop_tab(&ctx, 1, TabOut { tab: 0, screen: None });
    assert_eq!(hub.windows.len(), 3);
}

/// Several windows drawn in one frame, without a panic and without sharing a
/// command box or a scroll position: the second one here is drawn embedded, as
/// egui does where there is no second operating-system window.
#[test]
fn two_windows_are_drawn_in_one_frame_each_with_its_own_tabs() {
    use egui_kittest::kittest::Queryable;
    let ctx = egui::Context::default();
    let mut hub = hub_of(&["single-page.pdf", "two-column.pdf"]);
    hub.tear_off(&ctx, 0, 1, pos2(900.0, 300.0), vec2(1240.0, 860.0));
    let mut h = hub_harness(hub);
    h.run_steps(4);
    // One window shows single-page.pdf's tab strip, the other two-column.pdf's.
    assert_eq!(h.get_all_by_label("single-page.pdf").count() >= 1, true);
    assert_eq!(h.get_all_by_label("two-column.pdf").count() >= 1, true);
    assert_eq!(h.state().windows[0].app.tabs.len(), 1);
    assert_eq!(h.state().windows[1].app.tabs.len(), 1);
    assert_eq!(h.state().windows[0].app.win.others, 1);
    // What is lent is back where it belongs after the frame.
    assert!(h.state().windows.iter().all(|w| w.app.recent.entries.is_empty()));
}
