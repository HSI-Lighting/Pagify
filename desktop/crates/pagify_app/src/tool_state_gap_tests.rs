//! Characterization tests for Phase 2 (the `Tool` state machine): four
//! places today's scattered tool-arming state behaves asymmetrically, found
//! by audit while mapping it ahead of that refactor — not reported from use,
//! and not fixed here. **Behaviour-preserving**: these pin down what the app
//! does *today*, on purpose, so a migration that quietly makes one of them
//! symmetric (which a unified `Tool` enum would do very easily, in either
//! direction, without anyone noticing) shows up as a failing test instead of
//! a silent behaviour change.
//!
//! Each test's assertion message says which asymmetry it is pinning down and
//! why, so a future change that deliberately resolves one of them can delete
//! the right test with a clear conscience instead of wondering why it broke.

use super::ui_tests::harness;
use super::*;

fn fixture(name: &str) -> String {
    format!(
        "{}/../../../rust/pdf_core/fixtures/{name}",
        env!("CARGO_MANIFEST_DIR")
    )
}

fn app(name: &str) -> PagifyApp {
    let app = PagifyApp::new(Some(&fixture(name)));
    assert!(app.tab().doc.is_some(), "{name} did not open");
    app
}

fn said(app: &PagifyApp) -> String {
    app.cmd.history().iter().map(|e| e.text.as_str()).collect::<Vec<_>>().join("\n")
}

/// **Arming a markup tool does not put down the object tool already in
/// hand.** `mark_selection`'s own "nothing selected, pick the tool up"
/// branch clears `link_armed`/`match_properties_armed`/`match_properties_
/// sample` and calls `put_down_page_editors` — never `object_tool` or
/// anything it owns. The only thing keeping this harmless today is that
/// `interact` checks `object_tool.is_some()` first and returns before ever
/// reaching the markup-selection drag handling: the armed highlighter sits
/// inert, not disarmed, for as long as the object tool stays in hand.
///
/// The other direction is not symmetric: `take_up_object_tool` *does* clear
/// `markup_armed` (and `link_armed`, `match_properties_*`) when the object
/// tool is taken up second — this test's order (object tool first, markup
/// second) is the one direction where both end up armed at once.
#[test]
fn arming_a_markup_tool_does_not_put_down_the_object_tool_already_in_hand() {
    let mut app = app("two-column.pdf");
    app.submit("editobject");
    assert!(app.tab_mut().object_tool.is_some(), "setup: the object tool should have armed");

    app.submit("highlight");
    assert!(app.tab_mut().markup_armed.is_some(), "setup: the markup tool should have armed");
    assert!(
        app.tab_mut().object_tool.is_some(),
        "today, arming a markup tool after the object tool leaves both armed \
         at once — mark_selection never touches object_tool, unlike \
         take_up_object_tool, which does clear markup_armed the other way round"
    );
}

/// **The central `Escape` handler never touches `pending_link`.** `escape()`
/// walks a long, explicit list of what to put down — `paste_ghost`,
/// `editing_run`, `new_text_box`, `object_tool`, `signature_selected`,
/// `placed_image_selected`, `markup_armed`, `link_armed`,
/// `match_properties_*`, `pending` — and `pending_link` (and
/// `pending_article_box`) are not on it. The Link prompt closes on Escape
/// anyway, but only because `draw_link_prompt` reads `ctx.input(|i|
/// i.key_pressed(Key::Escape))` itself, independently, every frame it is
/// open — a second, separate reader of the same keypress, not a consequence
/// of the central handler. (`key_pressed` is non-consuming, so this causes
/// no conflict today; it is two code paths implementing what reads as one
/// rule, which a unified `Tool::on_cancel` would naturally merge into one —
/// deliberately, not by this test's silent assumption.)
#[test]
fn the_central_escape_does_not_close_the_link_prompt() {
    let mut app = app("two-column.pdf");
    let range = app.characters(0).expect("chars").find("the").first().cloned().expect("a match");
    app.tab_mut().text_selection = Some(range);
    app.tab_mut().selection_page = 0;
    app.submit("weblinks");
    assert!(app.tab_mut().pending_link.is_some(), "setup: the link prompt should have opened");

    app.escape();

    assert!(
        app.tab_mut().pending_link.is_some(),
        "today, the central Escape handler never looks at pending_link at all — \
         only the link-prompt panel's own separate ctx.input(Escape) check \
         (not exercised by this test, which calls escape() directly with no \
         egui frame) closes it"
    );
}

/// **Enter and the typed `done` command answer "not enough points yet"
/// differently for the same situation.** Both read the identical condition
/// (`kind.ends_on_enter() && points.len() >= 2`, now against `tool` since
/// `Draw`/`pline` moved there — the shape of the check didn't change) —
/// Enter in `main.rs`'s own per-frame key handler, `done` as `Verb::Finish`
/// in `dispatch.rs` — but Enter's `if closeable { self.resolve_tool(); }`
/// has no `else`, so with too few points it does nothing and says nothing;
/// `done`'s `else if ... { say_error("not enough points yet.") }` always
/// says something. A reader who presses Enter on a one-point polyline learns
/// nothing happened only by watching the page not finish; a reader who types
/// `done` is told why.
#[test]
fn enter_with_too_few_points_says_nothing_but_done_says_so() {
    let mut h = harness("single-page.pdf");
    h.state_mut().submit("pline");
    h.state_mut().take_pick(AppPoint { x: 10.0, y: 10.0 });
    assert_eq!(
        h.state_mut().tab_mut().tool.as_ref().expect("setup: still armed").points.len(),
        1,
        "setup: exactly one point should be down"
    );

    // Nothing must have focus, or `allows_document_keys` refuses the key
    // before it ever reaches the pending-finish check this test is about.
    h.ctx.memory_mut(|m| m.surrender_focus(egui::Id::new(COMMAND_INPUT)));
    let before = said(h.state());
    h.key_press(egui::Key::Enter);
    h.run_steps(2);

    assert!(
        h.state().tab().tool.is_some(),
        "a one-point polyline must not finish on Enter — there are not enough points"
    );
    assert_eq!(
        said(h.state()),
        before,
        "today, Enter with too few points says nothing new at all (no `else` \
         branch in main.rs's own handler) — if this now says something, `done` \
         below should too, and the asymmetry this test exists to pin down is gone"
    );

    h.state_mut().submit("done");
    assert!(
        said(h.state()).to_lowercase().contains("not enough points"),
        "the typed `done` command, unlike Enter, reports the same situation:\n{}",
        said(h.state())
    );
}

/// **An object-pick miss leaves `pending` completely untouched — quieter
/// even than `resolve()`'s own quiet re-arm.** `take_pick`'s `wants_object()`
/// branch, on a miss, calls `say_info("nothing there…")` and returns
/// immediately (`picking.rs`): it never pushes anything onto `objects`,
/// never calls `resolve()`, never touches `pending` at all. The existing
/// coverage (`pointer_tests.rs`'s `picking_empty_paper_for_an_object_says_so`)
/// only checks that the message was said, not that the state was left alone
/// — this test pins the state invariant too, so a `Tool` migration that
/// routes a miss through even a trivial no-op transition (which would be the
/// natural thing to write) shows up as a failing test instead of an
/// unnoticed behaviour change.
#[test]
fn an_object_pick_that_misses_leaves_pending_completely_untouched() {
    let mut app = app("single-page.pdf");
    app.submit("l 10,10 100,100");
    app.submit("fillet 20");
    assert_eq!(
        app.tab_mut().pending.as_ref().expect("setup: fillet should have armed").objects.len(),
        0,
        "setup: no objects collected yet"
    );

    app.take_pick(AppPoint { x: 500.0, y: 500.0 }); // far from the drawn line

    assert!(
        said(&app).contains("nothing there"),
        "setup: the miss should have said so:\n{}",
        said(&app)
    );
    let after = app.tab_mut().pending.as_ref().expect("the tool should still be armed");
    assert_eq!(
        after.objects.len(),
        0,
        "today, a missed object pick does not even record a failed attempt — \
         take_pick's None arm returns before resolve() or any mutation at all"
    );
    assert_eq!(after.points.len(), 0, "a miss on an object pick must not fall through to the points branch either");
}
