//! Direct, no-window unit tests against `Tool`'s own methods — the
//! DESIGN_REVIEW.md Phase 2 task 5 ask ("write transition tests in
//! `tools.rs` covering the rules currently encoded in comments"), for the
//! two of its five named rules that are genuinely pure properties of a
//! bare `Tool` value rather than something only a running app can answer.
//! The other three are real interaction — a click, a drag, an Escape with
//! a window behind it — and already have dedicated coverage; this file
//! cites exactly where rather than duplicating a harness-driven test here.
//!
//! The sixth task in that list — a property test that `on_cancel` returns
//! to "nothing armed" for every variant — is below
//! (`on_cancel_always_leaves_nothing_armed_and_no_ribbon_button_lit`).
//! Read loosely rather than literally: there is no `Tool::None` variant
//! (`tool: Option<ArmedTool>` already says "nothing armed" without one —
//! see `Pagify-Phase2-BigTasks.md` §2.3 step 6 for why one was not added),
//! so this tests `tool.is_none()` and `id()` rather than a variant that
//! does not exist.

use super::*;

/// **Rule: placing a signature, picture, or text box disarms the tool.**
/// `repeats() == false` is the whole mechanism — `resolve_tool`'s tail only
/// re-arms when `repeats()` says to, so these three simply don't.
/// `Calibrate` is the fourth non-repeating kind (not named in the
/// mentor's comment, which predates it) — included here for the same
/// reason: it answers a question and leaves you to look at the result,
/// not stamp another.
#[test]
fn placing_a_signature_picture_or_text_box_disarms_the_tool() {
    assert!(!Tool::Signature.repeats(), "a placed signature should not stay in hand");
    assert!(
        !Tool::PlaceImage { rgba: Vec::new(), width: 0, height: 0 }.repeats(),
        "a placed picture should not stay in hand"
    );
    assert!(!Tool::PlaceText.repeats(), "a drawn text box should not stay in hand");
    assert!(
        !Tool::Calibrate { distance: 1.0, unit: "m".into() }.repeats(),
        "a calibration should not stay in hand"
    );
}

/// **Where rule 1 (click-away from an open editor applies it, and does not
/// double-handle the same click) is actually tested**: this needs a real
/// editor open and a real click, not a bare `Tool` value —
/// `g4_edit_error_tests::the_click_that_closes_an_editor_picks_once_not_
/// twice` drives exactly this, including the "two identical errors"
/// regression the mentor's own comment names.
///
/// **Where rule 3 (a failed pick re-arms quietly) is actually tested**:
/// `g4_edit_error_tests::a_click_on_bare_paper_stays_on_screen_beside_the_
/// prompt_while_the_tool_is_armed` drives a real miss through `PickText`
/// and checks both halves of "quietly" — the tool is still armed, *and*
/// the error (not a re-stated prompt) is the last thing on screen.
///
/// **Where rule 4 (Escape cancels without applying) is actually tested**:
/// every `_wiring_tests.rs` file has at least one of these; the plainest
/// is `ui_tests::escape_puts_the_tool_down`.
///
/// This test exists only to hold these three citations somewhere a future
/// reader of *this* file will actually see them, so "transition tests
/// live in tools.rs" doesn't read as "the other three rules were never
/// written" — they were, just where the interaction they test already
/// lived.
#[test]
fn rules_1_3_and_4_are_interaction_level_and_tested_elsewhere() {}

/// **Rule: `Draw(Polyline)`, `Draw(Spline)`, and `Measure(Area)` end on
/// Enter and re-arm per `repeats`** — and, per the mentor's own research
/// (`Pagify-Phase2-Handoff.md` §2b), these are *exactly* the three, not
/// "at least" these three: `ends_on_enter()` is derived from
/// `wants_points() == usize::MAX`, so this also pins down that no other
/// kind has (or should gain, without updating this test) an unbounded
/// point count.
#[test]
fn polyline_spline_and_area_end_on_enter_and_repeat() {
    for tool in [Tool::Draw(DrawKind::Polyline), Tool::Draw(DrawKind::Spline), Tool::Measure(MeasureKind::Area)] {
        assert!(tool.ends_on_enter(), "should end on Enter");
        assert!(tool.repeats(), "should stay armed for the next one");
    }

    // A representative sample of fixed-point-count kinds, each a
    // different shape (no points/objects only, points only, objects only,
    // a fixed count already at the top of its own range) — not the full
    // 20-odd variants, which would just be re-deriving wants_points()'s
    // own match by hand.
    for tool in [
        Tool::Draw(DrawKind::Line),
        Tool::Draw(DrawKind::Rectangle),
        Tool::Measure(MeasureKind::Distance),
        Tool::Lock,
        Tool::PickText,
        Tool::Signature,
    ] {
        assert!(!tool.ends_on_enter(), "a fixed point count should not end on Enter");
    }
}

/// **`Tool::id` lights the exact ribbon button its own kind should** — for
/// every kind that lights a button at all. Checked against the live
/// ribbon tables (`Tab::leading`/`Tab::buttons`) rather than a second,
/// hand-written copy of the same mapping that could silently drift from
/// them. `.trim()` accounts for `Calibrate`'s own button text ending in a
/// trailing space (see `ribbon.rs`'s own doc on `Command`) — a rendering
/// quirk, not a different identity.
#[test]
fn every_tool_ids_ribbon_command_matches_what_the_ribbon_table_expects() {
    let literal_for = |id: ToolId| -> &'static str {
        Tab::ALL
            .iter()
            .flat_map(|t| t.leading().iter().chain(t.buttons()))
            .find_map(|(_, _, command)| match command {
                crate::ribbon::Command::Tool(found, text) if *found == id => Some(text.trim()),
                _ => None,
            })
            .unwrap_or_else(|| panic!("{id:?} lights no ribbon button"))
    };

    let cases: &[(Tool, &str)] = &[
        (Tool::Signature, "signature"),
        (Tool::PlaceText, "addtext"),
        (Tool::Write(String::new()), "addtext"),
        (Tool::Calibrate { distance: 1.0, unit: "m".into() }, "calibrate"),
        (Tool::Redact, "redact"),
        (Tool::Whiteout, "whiteout"),
        (Tool::SignRectangle, "signrectangle"),
        (Tool::SignLine, "signline"),
        (Tool::Draw(DrawKind::Line), "line"),
        (Tool::Draw(DrawKind::Circle), "circle"),
        (Tool::Draw(DrawKind::Polyline), "pline"),
        (Tool::Draw(DrawKind::Arrow), "arrow"),
        (Tool::Draw(DrawKind::Spline), "spline"),
        (Tool::EraseMark, "erase"),
        (Tool::Measure(MeasureKind::Distance), "measure distance"),
        (Tool::Measure(MeasureKind::Area), "measure area"),
        (Tool::Lock, "lock"),
        (Tool::ArticleBox, "articlebox"),
        (Tool::PickText, "edittext"),
        (Tool::Markup(pagify_shell::verbs::Markup::Highlight), "highlight"),
        (Tool::Markup(pagify_shell::verbs::Markup::Underline), "underline"),
        (Tool::Markup(pagify_shell::verbs::Markup::StrikeOut), "strikeout"),
        (Tool::Markup(pagify_shell::verbs::Markup::Squiggly), "squiggly"),
        (Tool::Link, "weblinks"),
        (Tool::MatchProperties { sample: None }, "matchproperties"),
    ];
    for (tool, expected) in cases {
        let id = tool.id().unwrap_or_else(|| panic!("{tool:?} should light a ribbon button"));
        assert_eq!(literal_for(id), *expected, "{tool:?}");
    }

    // The kinds that never light a button at all — a file name, a mark
    // argument, and a rectangle outline shared visually with other shapes
    // that do light their own.
    for tool in [
        Tool::PlaceImage { rgba: Vec::new(), width: 0, height: 0 },
        Tool::Fill(pdf_core::document::FillMark::Tick),
        Tool::Draw(DrawKind::Rectangle),
    ] {
        assert!(tool.id().is_none(), "{tool:?} should not light a ribbon button");
    }
}

/// **Task 6's property test: `on_cancel` leaves nothing armed and no
/// ribbon button lit, for every variant.** Read loosely, not literally —
/// see this file's own module doc for why there is no `Tool::None` to
/// assert a return value against.
///
/// What is actually checked here, since a bare `Tool` has no app or
/// window behind it to ask "is the ribbon button still lit": every
/// variant's `on_cancel` says `Cancelled`, never one of the re-arming
/// effects (`Rearm`/`RearmQuietly`) or a dialog hand-off
/// (`OpenPasscodePrompt`/`OpenArticleBoxPrompt`) — the ones that would
/// leave something behind for `escape()` to act on beyond putting the
/// tool down. "Nothing armed" itself is structural, not something
/// `on_cancel`'s return decides: `escape()` (`main.rs`) calls
/// `self.tab_mut().tool.take()` *before* calling `on_cancel`, so `tool`
/// is already `None` by the time any variant's own answer is read — this
/// test exists to pin that no variant's answer undoes that by asking to
/// re-arm. "No ribbon button lit" follows from the same `take()`, and
/// from `Tool::id()`'s own exhaustive match requiring every new variant
/// to say which button (if any) it lights — both already covered at the
/// interaction level by `ui_tests::escape_puts_the_tool_down`, cited above.
#[test]
fn on_cancel_always_leaves_nothing_armed_and_no_ribbon_button_lit() {
    let every_kind = [
        Tool::Signature,
        Tool::PlaceImage { rgba: Vec::new(), width: 0, height: 0 },
        Tool::PlaceText,
        Tool::Calibrate { distance: 1.0, unit: "m".into() },
        Tool::Fill(pdf_core::document::FillMark::Tick),
        Tool::Redact,
        Tool::Whiteout,
        Tool::SignRectangle,
        Tool::SignLine,
        Tool::Write(String::new()),
        Tool::Draw(DrawKind::Line),
        Tool::Draw(DrawKind::Circle),
        Tool::Draw(DrawKind::Rectangle),
        Tool::Draw(DrawKind::Polyline),
        Tool::Draw(DrawKind::Arrow),
        Tool::Draw(DrawKind::Spline),
        Tool::EraseMark,
        Tool::Measure(MeasureKind::Distance),
        Tool::Measure(MeasureKind::Area),
        Tool::Modify(pagify_shell::tools::Pick {
            op: pagify_shell::tools::Op::Move,
            objects: 1,
            points: 1,
            needs_selection: false,
            repeating: false,
        }),
        Tool::Lock,
        Tool::ArticleBox,
        Tool::PickText,
        Tool::Markup(pagify_shell::verbs::Markup::Highlight),
        Tool::Link,
        Tool::MatchProperties { sample: None },
    ];
    for tool in every_kind {
        let described = format!("{tool:?}");
        assert!(
            matches!(tool.on_cancel(), ToolEffect::Cancelled),
            "{described} should just say \"cancelled.\", not re-arm or open a dialog"
        );
    }
}
