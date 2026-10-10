use super::*;
use eframe::App as _;
use egui_kittest::Harness;

pub(crate) fn fixture(name: &str) -> String {
    format!(
        "{}/../../../rust/pdf_core/fixtures/{name}",
        env!("CARGO_MANIFEST_DIR")
    )
}

/// The app in a window big enough to have a page in it.
pub(crate) fn harness(name: &str) -> Harness<'static, PagifyApp> {
    let app = PagifyApp::new(Some(&fixture(name)));
    assert!(app.tab().doc.is_some(), "{name} did not open");
    harness_from(app)
}

/// Same as [`harness`], but for a document outside the fixtures folder —
/// a real file on the machine it happens to be run on. `None` when that
/// file is not present, for a caller to skip rather than fail on.
pub(crate) fn harness_at(path: &str) -> Option<Harness<'static, PagifyApp>> {
    let app = PagifyApp::new(Some(path));
    if app.tab().doc.is_none() {
        return None;
    }
    Some(harness_from(app))
}

pub(crate) fn harness_from(app: PagifyApp) -> Harness<'static, PagifyApp> {
    let mut h = Harness::builder()
        .with_size(egui::vec2(1400.0, 1000.0))
        .build_ui_state(
            |ui, app: &mut PagifyApp| {
                let mut frame = eframe::Frame::_new_kittest();
                app.ui(ui, &mut frame);
            },
            app,
        );
    // A few frames: the first lays out, the rest let the page raster and
    // the strip settle.
    h.run_steps(4);
    h
}

/// **A continuous zoom gesture must not miss the texture cache on every
/// frame.** `scale` changes by a hair each frame during a pinch or a
/// scroll-wheel zoom — keying `texture_for`'s cache on that raw value,
/// as it used to, made every one of those frames a fresh render and a
/// fresh GPU upload of the whole page, reused by nothing even a moment
/// later at a functionally identical zoom. Quantising the key (see
/// `pdf_core::render::cache::quantise_zoom`, built for its own prefetch
/// cache for the same reason) means every scale within one 0.25 step
/// shares one texture. Reported from use: zooming any real, dense page
/// felt "very laggy," constantly, independent of anything else open at
/// the time.
#[test]
fn nearby_zoom_levels_during_one_gesture_share_a_texture() {
    let mut h = harness("two-column.pdf");
    let ctx = h.ctx.clone();
    let first = h.state_mut().texture_for(&ctx, 0, 1.60).expect("a page to render");
    // Close enough to land in the same quantum band as 1.60 (both round
    // up to the 1.75 step) — see `nearby_pinch_zooms_collapse_onto_one_key`
    // in `pdf_core`'s own cache tests for the exact boundary this mirrors.
    let second = h.state_mut().texture_for(&ctx, 0, 1.74).expect("a page to render");
    assert_eq!(first.id(), second.id(), "two nearby zoom levels rendered two separate textures");

    // A scale far enough away to land in a different band must still
    // get its own, sharper texture — quantising must not just always
    // reuse the first thing rendered.
    let far = h.state_mut().texture_for(&ctx, 0, 3.0).expect("a page to render");
    assert_ne!(first.id(), far.id(), "a genuinely different zoom level must not reuse the same texture");
}

/// Where the page is on screen, and a point on its first character.
fn a_character_on_screen(h: &mut Harness<'static, PagifyApp>) -> egui::Pos2 {
    let app = h.state_mut();
    let page = app.tab_mut().view_state.page;
    let chars = app.characters(page).expect("no characters").clone();
    let r = chars.line_rects(0..1).into_iter().next().expect("no character box");
    let mid = egui::pos2((r.left + r.right) / 2.0, (r.top + r.bottom) / 2.0);

    let view = app.tab_mut().view_state.last_view.expect("the page was never drawn, so nothing can be clicked");
    view.to_screen(AppPoint { x: mid.x as f64, y: mid.y as f64 })
}

pub(crate) fn drag(h: &mut Harness<'static, PagifyApp>, from: egui::Pos2, to: egui::Pos2) {
    use egui::{Event, PointerButton};
    let press = |pos, down| Event::PointerButton {
        pos,
        button: PointerButton::Primary,
        pressed: down,
        modifiers: Default::default(),
    };

    h.input_mut().events.push(Event::PointerMoved(from));
    h.run_steps(1);
    h.input_mut().events.push(press(from, true));
    h.run_steps(1);

    // Several moves, because a drag is a sequence: one jump can be read as
    // a click somewhere else.
    for i in 1..=4 {
        let t = i as f32 / 4.0;
        h.input_mut().events.push(Event::PointerMoved(from + (to - from) * t));
        h.run_steps(1);
    }
    h.input_mut().events.push(press(to, false));
    h.run_steps(2);
}

pub(crate) fn click(h: &mut Harness<'static, PagifyApp>, at: egui::Pos2) {
    use egui::{Event, PointerButton};
    h.input_mut().events.push(Event::PointerMoved(at));
    h.run_steps(1);
    for pressed in [true, false] {
        h.input_mut().events.push(Event::PointerButton {
            pos: at,
            button: PointerButton::Primary,
            pressed,
            modifiers: Default::default(),
        });
        h.run_steps(1);
    }
    h.run_steps(1);
}

/// **Reported from use: "jumping around and glitches while opening files of
/// different page dimensions".** In Fit and Width the zoom was worked out
/// from the page that fills the window, and that page was chosen from the
/// scroll position under that very zoom: scrolled to where two sizes of page
/// meet, the view flipped between zooms (1.37, 0.33, 0.65...) every frame,
/// for ever, with no input at all — and re-rendered every page at every
/// size. The zoom is now taken from a page that only the reader changes.
#[test]
fn scrolling_through_pages_of_different_sizes_neither_changes_the_zoom_nor_flips_for_ever() {
    let sizes = [(595.0, 842.0), (1190.0, 842.0), (300.0, 400.0), (2384.0, 1684.0), (595.0, 842.0), (420.0, 595.0)];
    for mode in [ZoomMode::Fit, ZoomMode::Width] {
        let mut h = harness_60fps(&fixture("single-page.pdf"), false);
        for (w, hh) in sizes {
            let doc = h.state().tab().doc.as_ref().expect("doc");
            doc.session
                .execute(pdf_core::command::Command::InsertBlankPage {
                    at: doc.page_count,
                    width_pt: w,
                    height_pt: hh,
                    fill: None,
                    ruling: 0,
                })
                .expect("page");
            h.state_mut().refresh_after_page_change();
        }
        h.state_mut().tab_mut().view_state.zoom = mode;
        h.run_steps(20);
        let first_zoom = h.state().resolved_zoom();
        h.input_mut().events.push(egui::Event::PointerMoved(egui::pos2(700.0, 500.0)));
        h.run_steps(2);
        for step in 0..60 {
            h.input_mut().events.push(egui::Event::MouseWheel {
                unit: egui::MouseWheelUnit::Point,
                delta: egui::vec2(0.0, -80.0),
                phase: egui::TouchPhase::Move,
                modifiers: Default::default(),
            });
            for _ in 0..4 {
                h.run_steps(1);
                assert_eq!(
                    h.state().resolved_zoom(),
                    first_zoom,
                    "{mode:?}: the zoom changed while scrolling (step {step}, page {})",
                    h.state().tab().view_state.page
                );
            }
        }
        // And once the scrolling stops (egui's smooth scroll takes a
        // moment to come to rest) nothing keeps moving.
        h.run_steps(60);
        let (page, offset) = (h.state().tab().view_state.page, h.state().tab().view_state.scroll_offset);
        for _ in 0..30 {
            h.run_steps(1);
            assert_eq!(
                (h.state().tab().view_state.page, h.state().tab().view_state.scroll_offset),
                (page, offset),
                "{mode:?}: the view is still moving with no input"
            );
        }
        // Fit asked for again fits the page being read, as it always did.
        h.state_mut().set_zoom(ZoomTarget::Fit);
        h.run_steps(3);
        assert_ne!(h.state().resolved_zoom(), first_zoom, "{mode:?}: Fit did not refit to the page being read");
    }
}

/// A page with one highlight and one note on it, for the Eraser tests:
/// returns the app and the page points a click on each would land at.
fn page_with_a_highlight_and_a_note() -> Harness<'static, PagifyApp> {
    let h = harness("two-column.pdf");
    let doc = h.state().tab().doc.as_ref().expect("doc");
    doc.session
        .execute(pdf_core::command::Command::AddAnnotation {
            page_index: 0,
            annotation: pdf_core::document::Annotation::Highlight {
                rects: vec![pdf_core::document::Rect { left: 100.0, top: 100.0, right: 220.0, bottom: 116.0 }],
                color: pdf_core::document::Color { r: 255, g: 224, b: 102, a: 128 },
            },
        })
        .expect("a highlight");
    doc.session
        .execute(pdf_core::command::Command::AddAnnotation {
            page_index: 0,
            annotation: pdf_core::document::Annotation::Note {
                rect: pdf_core::document::Rect { left: 300.0, top: 300.0, right: 320.0, bottom: 320.0 },
                contents: "a note".into(),
                color: pdf_core::document::Color { r: 255, g: 224, b: 102, a: 255 },
            },
        })
        .expect("a note");
    h
}

fn annotation_count(h: &Harness<'static, PagifyApp>) -> usize {
    h.state().tab().doc.as_ref().expect("doc").session.annotations(0).expect("annotations").len()
}

/// **Reported from use: "there is no option for a user to erase a
/// highlight; the eraser in the Draw tab should erase highlights too".**
/// With nothing drawn selected the Eraser now picks up a click-to-erase
/// tool; a click on a highlight takes it off the page, `undo` puts it
/// back, and the tool stays in hand for the next one.
#[test]
fn the_eraser_rubs_out_a_highlight_with_a_click_and_undo_puts_it_back() {
    let mut h = page_with_a_highlight_and_a_note();
    assert_eq!(annotation_count(&h), 2);

    h.state_mut().submit("erase");
    assert!(
        matches!(h.state().tab().tool.as_ref().map(|t| &t.kind), Some(Tool::EraseMark)),
        "the Eraser with nothing selected did not pick up the click-to-erase tool"
    );

    let on_highlight = AppPoint { x: 150.0, y: 108.0 };
    let said = h.state_mut().erase_mark_at(0, on_highlight).expect("the highlight should be erased");
    assert!(said.contains("highlight erased"), "{said}");
    assert_eq!(annotation_count(&h), 1, "the highlight is still on the page");

    h.state_mut().submit("undo");
    assert_eq!(annotation_count(&h), 2, "undo did not put the highlight back");
}

/// The Eraser leaves what it was not asked to touch: a note under it is
/// named, not destroyed, and a click on bare paper says so.
#[test]
fn the_eraser_leaves_a_note_alone_and_says_so_on_bare_paper() {
    let mut h = page_with_a_highlight_and_a_note();
    let refused = h.state_mut().erase_mark_at(0, AppPoint { x: 310.0, y: 310.0 }).expect_err("a note was erased");
    assert!(refused.contains("not a highlight"), "{refused}");
    let nothing = h.state_mut().erase_mark_at(0, AppPoint { x: 20.0, y: 20.0 }).expect_err("nothing there");
    assert!(nothing.contains("no highlight there"), "{nothing}");
    assert_eq!(annotation_count(&h), 2);
}

/// A drawn object that is selected is still erased the way it always was;
/// the click tool is only for when nothing is selected.
#[test]
fn the_eraser_still_erases_a_selected_drawing() {
    let mut app = PagifyApp::new(Some(&fixture("two-column.pdf")));
    app.submit("l 10,10 100,100");
    app.submit("all");
    app.submit("erase");
    assert!(app.tab().tool.is_none(), "a selection was erased but the click tool was armed as well");
    assert_eq!(app.tab().markup.existing(0).map(|l| l.len()).unwrap_or(0), 0, "the drawing is still there");
}

/// **The build number is in the title bar, at the far right** — with or
/// without a document open, past the Ortho / Pages checkboxes.
#[test]
fn the_title_bar_shows_the_build_number_at_the_far_right() {
    use egui_kittest::kittest::Queryable;

    let label = format!("v{}", pagify_shell::VERSION);
    let find = |h: &Harness<'static, PagifyApp>, what: &str| -> egui::Rect {
        let found: Vec<egui::Rect> = h
            .get_all_by_label(what)
            .map(|n| n.rect())
            .filter(|r| r.center().y < 70.0)
            .collect();
        assert_eq!(found.len(), 1, "expected {what:?} once in the title bar: {found:?}");
        found[0]
    };

    let empty = harness_from(PagifyApp::new(None));
    let version = find(&empty, &label);
    assert!(version.right() > 1400.0 - 40.0, "the build number is not at the far right: {version:?}");

    let h = harness("two-column.pdf");
    let version = find(&h, &label);
    let ortho = find(&h, "Ortho");
    assert!(ortho.right() <= version.left(), "the checkboxes are not left of it: {ortho:?} {version:?}");
    assert!(version.right() > 1400.0 - 40.0, "the build number is not at the far right: {version:?}");
}

/// **The title bar reads left to right, and everything in it sits on one
/// line.**
///
/// Reported from use, twice. First "the logo is not aligned with the tabs":
/// the logo, the name and the checkboxes sat higher than the document
/// tabs. Then, after an edit made without measuring anything, "why are the
/// tabs aligned to right?" — it removed the wrapper that made the tab strip
/// start at the left of the room the right-to-left checkbox group leaves,
/// and the strip was placed by that group's direction instead.
///
/// Read off the real window — where each thing landed — not reasoned about.
#[test]
fn the_title_bar_reads_left_to_right_and_sits_on_one_line() {
    use egui_kittest::kittest::Queryable;

    let mut h = harness("two-column.pdf");
    h.state_mut().open(&fixture("text-lines.pdf"));
    assert_eq!(h.state().tabs.len(), 2, "the second document did not open in a tab");
    h.run_steps(4);

    // The open document's name is also drawn on the command line, so a
    // label alone is not unique — the title bar is the one at the top.
    let title_bar = |label: &str| -> egui::Rect {
        let found: Vec<egui::Rect> = h
            .get_all_by_label(label)
            .map(|n| n.rect())
            .filter(|r| r.center().y < 70.0)
            .collect();
        assert_eq!(found.len(), 1, "expected one {label:?} in the title bar, found {found:?}");
        found[0]
    };
    let logo = title_bar("Pagify logo");
    let name = title_bar("Pagify");
    // The newest tab is the leftmost: text-lines.pdf was opened second.
    let first = title_bar("text-lines.pdf");
    let second = title_bar("two-column.pdf");
    let ortho = title_bar("Ortho");

    // Left to right, in the order they were written.
    assert!(logo.right() <= name.left(), "the name is not after the logo: {logo:?} {name:?}");
    assert!(name.right() <= first.left(), "the first tab is not after the name: {name:?} {first:?}");
    assert!(first.right() <= second.left(), "the tabs are out of order: {first:?} {second:?}");
    assert!(second.right() <= ortho.left(), "the checkboxes are not after the tabs: {second:?} {ortho:?}");

    // **Left-aligned**: the strip starts where the name stops, rather than
    // being pushed against the checkboxes at the far side.
    assert!(
        first.left() - name.right() < 40.0,
        "the tabs start {}px after the name — they are not left-aligned",
        first.left() - name.right()
    );
    // And the checkboxes did not come along with them: they are at the
    // right edge, short only of the build number that sits beyond them.
    let version = title_bar(&format!("v{}", pagify_shell::VERSION));
    assert!(
        version.right() > 1400.0 - 40.0 && version.left() - ortho.right() < 20.0,
        "the checkboxes are no longer at the right edge: {ortho:?} (build number {version:?})"
    );

    // One line. A pixel and a half of slack for rounding to whole pixels.
    let centres = [
        ("logo", logo.center().y),
        ("name", name.center().y),
        ("first tab", first.center().y),
        ("second tab", second.center().y),
        ("checkbox", ortho.center().y),
    ];
    let (lo, hi) = centres
        .iter()
        .fold((f32::MAX, f32::MIN), |(lo, hi), (_, y)| (lo.min(*y), hi.max(*y)));
    assert!(hi - lo <= 1.5, "the title bar is not on one line — vertical centres: {centres:?}");
}

/// **Rotating the pages turns the frame they are drawn in, not only what is
/// drawn in it.**
///
/// Reported from use, with screenshots, as a regression: after rotating the
/// pages "it changes the ratio of the page" — the turned page was squeezed
/// into the portrait frame it had before the turn.
#[test]
fn rotating_the_pages_turns_the_frame_they_are_drawn_in() {
    let mut h = harness("text-lines.pdf");
    let before = h.state().tab().doc.as_ref().expect("doc").strip.size_of(0).expect("size");
    assert!((before.0 - before.1).abs() > 1.0, "a square page cannot show a turn: {before:?}");

    h.state_mut().submit("rotatepages all 1");
    h.run_steps(3);

    let doc = h.state().tab().doc.as_ref().expect("doc");
    let after = doc.strip.size_of(0).expect("size");
    let raster = doc.session.render_page(0, 1.0).expect("render");
    println!(
        "frame before {before:?}; frame after {after:?}; engine's page size {:?}; raster {}x{}",
        doc.session.page_sizes().expect("sizes").first().map(|s| (s.width_pt, s.height_pt)),
        raster.width,
        raster.height
    );
    assert!(
        (after.0 - before.1).abs() < 0.5 && (after.1 - before.0).abs() < 0.5,
        "the frame did not turn with the page: {before:?} -> {after:?}"
    );
    assert!(
        (raster.width as f32 / raster.height as f32 - after.0 / after.1).abs() < 0.01,
        "the frame ({after:?}) and what is drawn in it ({}x{}) differ in proportion",
        raster.width,
        raster.height
    );
}

/// **Turning the view turns the frame the page is drawn in — it was the
/// raster alone that turned, and it was squeezed into the upright frame.**
///
/// Reported from use, with screenshots, as a regression. The invariant is
/// the one the report states: the proportions of what is drawn must be the
/// proportions of the frame it is drawn in.
#[test]
fn turning_the_view_does_not_change_the_proportions_of_the_page() {
    let mut h = harness("text-lines.pdf");
    let upright = h.state().tab().doc.as_ref().expect("doc").strip.size_of(0).expect("size");
    assert!((upright.0 - upright.1).abs() > 1.0, "a square page cannot show a turn: {upright:?}");

    h.state_mut().submit("rotate");
    h.run_steps(4);

    let doc = h.state().tab().doc.as_ref().expect("doc");
    let frame = doc.strip.frame_of(0).expect("frame");
    assert_eq!(
        doc.strip.size_of(0),
        Some(upright),
        "the page's own size must stay as the document has it"
    );
    assert!(
        (frame.0 - upright.1).abs() < 0.5 && (frame.1 - upright.0).abs() < 0.5,
        "the frame did not turn with the view: {upright:?} -> {frame:?}"
    );

    // What is drawn in it: the texture the page is painted from.
    let drawn: Vec<[usize; 2]> = doc.caches.textures.values().map(|t| t.size()).collect();
    assert!(!drawn.is_empty(), "no page was drawn, so this proves nothing");
    for size in drawn {
        let (drawn_ratio, frame_ratio) = (size[0] as f32 / size[1] as f32, frame.0 / frame.1);
        assert!(
            (drawn_ratio - frame_ratio).abs() < 0.02,
            "the page is drawn {size:?} into a frame of {frame:?} — squeezed"
        );
    }

    // And turning it back restores both.
    h.state_mut().submit("rotate 270");
    h.run_steps(4);
    let doc = h.state().tab().doc.as_ref().expect("doc");
    assert_eq!(doc.strip.frame_of(0), Some(upright), "turning it back did not restore the frame");
}

/// **The history is opened with an arrow, and an error does not open it.**
///
/// Reported from use, with screenshots: the command box opened itself over
/// the page whenever something went wrong, and the control beside the input
/// that was meant to open it — and, once open, to close it — was an empty
/// square, because the characters it was made of are in none of the
/// program's fonts. "There should be an arrow for the user to open it."
#[test]
fn an_error_leaves_the_history_shut_and_the_arrow_opens_and_folds_it() {
    use egui_kittest::kittest::Queryable;

    let mut h = harness("text-lines.pdf");
    h.state_mut().command_open = false;
    h.state_mut().say_error("something went wrong here");
    h.run_steps(2);

    assert!(!h.state().command_open, "an error opened the history");
    // It is still where the reader is looking: the line under the buttons.
    assert!(
        h.query_by_label_contains("something went wrong here").is_some(),
        "the error is not on screen, so shutting the history hid it"
    );

    let arrow = h.get_by_label("Show the history").rect();
    click(&mut h, arrow.center());
    h.run_steps(2);
    assert!(h.state().command_open, "the arrow did not open the history");

    let arrow = h.get_by_label("Hide the history").rect();
    click(&mut h, arrow.center());
    h.run_steps(2);
    assert!(!h.state().command_open, "the arrow did not fold the history away");
}

/// **The rest of the ribbon drops down as the ribbon's own tiles.**
///
/// Reported from use, with a screenshot of the first version — a plain list
/// of the held-back names under an arrow that was an empty square:
/// "instead of a drop down like this, just drop the rest of the ribbon like
/// how we had it."
#[test]
fn the_rest_of_the_ribbon_drops_down_as_the_ribbons_own_tiles() {
    use egui_kittest::kittest::Queryable;

    let mut app = PagifyApp::new(Some(&fixture("text-lines.pdf")));
    app.tab_mut().ribbon = Tab::Organize;
    let mut h = Harness::builder().with_size(egui::vec2(700.0, 800.0)).build_ui_state(
        |ui, app: &mut PagifyApp| {
            let mut frame = eframe::Frame::_new_kittest();
            app.ui(ui, &mut frame);
        },
        app,
    );
    h.run_steps(4);

    let tiles = |h: &Harness<'_, PagifyApp>, label: &str| -> Vec<egui::Rect> {
        h.query_all_by_role_and_label(egui::accesskit::Role::Button, label)
            .map(|n| n.rect())
            .collect()
    };
    let buttons = Tab::Organize.buttons();
    let held_back: Vec<&str> =
        buttons.iter().map(|b| b.1).filter(|label| tiles(&h, label).is_empty()).collect();
    assert!(!held_back.is_empty(), "the window was meant to be too narrow for the whole tab");

    let more = h.get_by_label("More tools").rect();
    click(&mut h, more.center());
    h.run_steps(3);

    for label in &held_back {
        let found = tiles(&h, label);
        assert_eq!(found.len(), 1, "{label:?} did not drop down: {found:?}");
        // A tile — the ribbon's own size — and not a line of text in a list.
        assert!(
            (found[0].width() - TOOL_WIDTH).abs() < 1.0 && (found[0].height() - TOOL_HEIGHT).abs() < 1.0,
            "{label:?} is not drawn as a ribbon tile: {:?}",
            found[0]
        );
        // And dropped *below* the ribbon, over the page.
        assert!(found[0].top() > more.bottom(), "{label:?} is not below the ribbon: {:?}", found[0]);
    }

    // Choosing one runs it and folds the panel away.
    let (_, label, command) = buttons.iter().find(|b| b.1 == held_back[0]).expect("a held-back button");
    let command = command.text();
    let tile = tiles(&h, label)[0].center();
    click(&mut h, tile);
    h.run_steps(3);
    assert!(tiles(&h, label).is_empty(), "the panel stayed open after a choice");
    assert!(
        h.state().cmd.history().iter().any(|e| e.text.trim() == command.trim()),
        "choosing {label:?} did not run `{command}`"
    );
}

/// **A thumbnail is rendered at the size it is drawn at.**
///
/// Reported from use, with a screenshot: "the quality of the preview in the
/// thumbnail is terrible… this is unacceptable". It was a seventy-pixel
/// bitmap drawn two to three times that wide.
#[test]
fn a_thumbnail_is_rendered_at_the_size_it_is_drawn() {
    use egui_kittest::kittest::Queryable;

    let mut h = harness("two-column.pdf");
    h.run_steps(3);
    let drawn = h
        .get_all_by_role(egui::accesskit::Role::Image)
        .map(|n| n.rect())
        .next()
        .expect("a thumbnail");
    let doc = h.state().tab().doc.as_ref().expect("doc");
    let bitmap = doc.caches.thumbs.get(&0).expect("the first thumbnail").size();

    assert!(
        (bitmap[0] as f32 - drawn.width()).abs() <= 8.0,
        "the bitmap is {} px across and is drawn {} px across — stretched",
        bitmap[0],
        drawn.width()
    );
    // And in the proportions of the page it shows.
    let (w, hgt) = doc.strip.size_of(0).expect("size");
    assert!(
        (bitmap[0] as f32 / bitmap[1] as f32 - w / hgt).abs() < 0.02,
        "the thumbnail's proportions are not the page's: {bitmap:?} against {w}x{hgt}"
    );
}

/// Only the thumbnails on (or near) the screen are rendered. At the size
/// they are now made, rendering every page of a long document on the first
/// frame — which is what happened — would freeze it.
#[test]
fn only_the_thumbnails_near_the_screen_are_rendered() {
    let mut h = harness("two-column.pdf");
    for _ in 0..40 {
        h.state_mut().insert_page();
    }
    h.run_steps(4);

    let doc = h.state().tab().doc.as_ref().expect("doc");
    assert!(doc.page_count >= 41, "the pages were not added");
    assert!(!doc.caches.thumbs.is_empty(), "no thumbnail was rendered at all");
    assert!(
        doc.caches.thumbs.len() < doc.page_count / 2,
        "{} of {} thumbnails were rendered with only a few in view",
        doc.caches.thumbs.len(),
        doc.page_count
    );
}

/// The Organize grid is a grid — more than one thumbnail to a row. Every
/// cell used to fill whatever was left of its row, so each was a row of its
/// own.
#[test]
fn the_organize_grid_puts_more_than_one_page_on_a_row() {
    use egui_kittest::kittest::Queryable;

    let mut h = harness("two-column.pdf");
    h.state_mut().insert_page();
    h.state_mut().insert_page();
    h.state_mut().toggle_organize_grid();
    h.run_steps(4);

    let rects: Vec<egui::Rect> =
        h.get_all_by_role(egui::accesskit::Role::Image).map(|n| n.rect()).collect();
    assert!(rects.len() >= 3, "expected the document's three pages, found {}", rects.len());
    assert!(
        (rects[0].top() - rects[1].top()).abs() < 1.0,
        "the first two pages are not on one row: {:?} {:?}",
        rects[0],
        rects[1]
    );
}

/// One frame of the Organize grid while it is open: where every
/// thumbnail is, and the panel they sit in.
struct GridFrame {
    images: Vec<egui::Rect>,
    panel: egui::Rect,
}

/// Open the Organize grid on a document of `pages` pages and record
/// `frames` frames of it, one per sixtieth of a second — the harness's
/// usual four a second would let anything that is still settling do so
/// between two frames and never be seen.
fn organize_grid_frames(
    pages: usize,
    pixels_per_point: f32,
    window: egui::Vec2,
    frames: usize,
) -> Vec<GridFrame> {
    use egui_kittest::kittest::Queryable;

    let app = PagifyApp::new(Some(&fixture("single-page.pdf")));
    let mut h = Harness::builder()
        .with_size(window)
        .with_pixels_per_point(pixels_per_point)
        .with_step_dt(1.0 / 60.0)
        .build_ui_state(
            |ui, app: &mut PagifyApp| {
                let mut frame = eframe::Frame::_new_kittest();
                app.ui(ui, &mut frame);
            },
            app,
        );
    h.run_steps(30);
    for _ in 1..pages {
        h.state_mut().insert_page();
    }
    h.run_steps(20);
    h.state_mut().toggle_organize_grid();
    let panel = egui::Id::new(ORGANIZE_GRID_PANEL);
    (0..frames)
        .map(|_| {
            h.run_steps(1);
            GridFrame {
                images: h.get_all_by_role(egui::accesskit::Role::Image).map(|n| n.rect()).collect(),
                panel: egui::PanelState::load(&h.ctx, panel).expect("the grid's panel").outer_rect,
            }
        })
        .collect()
}

/// **Reported from use: clicking Thumbnail View made the thumbnails
/// "glitch out and start zooming in and out".**
///
/// The panel is as wide as its contents, and the grid's contents were
/// always 14 points wider than the width they were laid out for: a gap
/// and a spacer after the last cell that the column arithmetic never
/// counted. So every frame the panel grew by those 14 points, the cells
/// grew with it, and when the column count finally changed the cells
/// shrank to a third of the size and it began again — a thumbnail going
/// from 135 to 191 to 95 points wide, six frames to the cycle. Found with
/// a single-page document, as in the report; nothing ever settled.
#[test]
fn the_organize_grid_holds_still_once_it_is_open() {
    for pages in [1, 2, 3, 5, 12] {
        for pixels_per_point in [1.0, 1.5] {
            for window in [egui::vec2(1400.0, 1000.0), egui::vec2(900.0, 600.0)] {
                let frames = organize_grid_frames(pages, pixels_per_point, window, 60);
                let last = frames.last().expect("frames");
                assert!(!last.images.is_empty(), "{pages} page(s): no thumbnail was drawn at all");
                for (n, frame) in frames.iter().enumerate().skip(2) {
                    assert!(
                        frame.images == last.images && frame.panel == last.panel,
                        "{pages} page(s) at {pixels_per_point}x in a {window:?} window: frame {n} is not \
                         the layout of the last frame, so the grid is still moving.\n\
                         frame {n}: panel {:?}, thumbnails {:?}\nlast:     panel {:?}, thumbnails {:?}",
                        frame.panel,
                        frame.images,
                        last.panel,
                        last.images
                    );
                }
            }
        }
    }
}

/// The grid is a grid: past the first row the thumbnails go on to a
/// second one, inside the panel — they used to carry on to the right, out
/// of it, where nothing could reach them.
#[test]
fn the_organize_grid_wraps_onto_further_rows_inside_its_panel() {
    for pages in [3, 5, 12] {
        let frames = organize_grid_frames(pages, 1.0, egui::vec2(1400.0, 1000.0), 10);
        let last = frames.last().expect("frames");
        let shown = pages.min(last.images.len());
        assert!(
            last.images.len() >= pages.min(5),
            "{pages} pages: only {} of them have a thumbnail on screen: {:?}",
            last.images.len(),
            last.images
        );
        assert!(
            (last.images[0].top() - last.images[1].top()).abs() < 1.0,
            "{pages} pages: the first two are not on one row"
        );
        assert!(
            last.images[2].top() > last.images[0].bottom(),
            "{pages} pages: the third page is not on a second row: {:?}",
            last.images
        );
        for rect in &last.images[..shown] {
            assert!(
                rect.left() >= last.panel.left() && rect.right() <= last.panel.right(),
                "{pages} pages: a thumbnail sticks out of its panel: {rect:?} in {:?}",
                last.panel
            );
        }
    }
}

/// A row of the Organize grid — its pages and the gaps between them — is
/// never wider than the width it was laid out for, at any width. The panel
/// is as wide as its contents, so a row that overshoots by anything makes
/// the panel wider and the grid never settles: it was 14 points over.
#[test]
fn a_row_of_the_organize_grid_never_outgrows_the_width_it_was_made_for() {
    // From the narrowest the panel's own size range leaves, in quarters.
    let mut width = 88.0_f32;
    while width <= 700.0 {
        let columns = PagifyApp::grid_columns(width);
        assert!(columns >= 1, "no column at {width}");
        let cell = PagifyApp::grid_cell_width(width, columns);
        let row = columns as f32 * cell + (columns as f32 - 1.0) * GRID_GAP_PT;
        assert!(row <= width, "at {width}: {columns} column(s) of {cell} make a row {row} wide");
        assert!(
            cell >= GRID_CELL_PT.min(width) - 1.0,
            "at {width}: cells of {cell} are well under the {GRID_CELL_PT} asked for"
        );
        width += 0.25;
    }
}

/// The Organize grid open on five pages that can be told apart — each a
/// different size, the first the fixture's own — two to a row at the
/// panel's default width, so the third starts a second row.
fn organize_grid_of_five_distinct_pages() -> Harness<'static, PagifyApp> {
    let mut h = harness_60fps(&fixture("single-page.pdf"), false);
    for i in 0..4 {
        let doc = h.state().tab().doc.as_ref().expect("doc");
        doc.session
            .execute(pdf_core::command::Command::InsertBlankPage {
                at: doc.page_count,
                width_pt: 200.0 + 10.0 * i as f32,
                height_pt: 300.0,
                fill: None,
                ruling: 0,
            })
            .expect("a blank page");
        h.state_mut().refresh_after_page_change();
    }
    h.state_mut().toggle_organize_grid();
    h.run_steps(6);
    h
}

fn page_sizes(h: &Harness<'static, PagifyApp>) -> Vec<(f32, f32)> {
    let doc = h.state().tab().doc.as_ref().expect("doc");
    (0..doc.page_count).map(|p| doc.strip.size_of(p).expect("a page size")).collect()
}

/// A page dragged in the Organize grid lands where it is dropped — here
/// from the first row to the end of the last, across rows.
#[test]
fn a_page_dragged_across_rows_of_the_organize_grid_is_reordered() {
    use egui_kittest::kittest::Queryable;

    let mut h = organize_grid_of_five_distinct_pages();
    let before = page_sizes(&h);
    assert_eq!(before.len(), 5, "setup: {before:?}");
    let rects: Vec<egui::Rect> =
        h.get_all_by_role(egui::accesskit::Role::Image).map(|n| n.rect()).collect();
    assert_eq!(rects.len(), 5, "all five pages should show a thumbnail: {rects:?}");

    // The right half of the last page: drop after it.
    let drop_at = rects[4].center() + egui::vec2(rects[4].width() / 4.0, 0.0);
    drag(&mut h, rects[0].center(), drop_at);
    h.run_steps(4);

    let after = page_sizes(&h);
    assert_eq!(
        after,
        vec![before[1], before[2], before[3], before[4], before[0]],
        "the first page was not moved to the end: before {before:?}, after {after:?}"
    );
}

/// A plain click on a page of the Organize grid selects it and goes to it.
#[test]
fn a_click_in_the_organize_grid_selects_the_page_and_goes_to_it() {
    use egui_kittest::kittest::Queryable;

    let mut h = organize_grid_of_five_distinct_pages();
    let rects: Vec<egui::Rect> =
        h.get_all_by_role(egui::accesskit::Role::Image).map(|n| n.rect()).collect();
    assert_eq!(rects.len(), 5, "all five pages should show a thumbnail: {rects:?}");

    click(&mut h, rects[3].center());
    h.run_steps(3);

    assert_eq!(h.state().tab().organize.organize_selected, vec![3], "the clicked page was not selected");
    assert_eq!(h.state().tab().view_state.page, 3, "the view did not go to the clicked page");
}

/// A long document scrolls in the Organize grid — down, and only down:
/// the scroll area is allowed to scroll sideways as a guard against an
/// overshooting row, and a wheel turned over the grid must not use that.
#[test]
fn the_organize_grid_scrolls_a_long_document_down_and_not_sideways() {
    use egui_kittest::kittest::Queryable;

    let mut h = harness_60fps(&fixture("single-page.pdf"), false);
    for _ in 0..40 {
        h.state_mut().insert_page();
    }
    h.state_mut().toggle_organize_grid();
    h.run_steps(6);
    let top_of_the_first = |h: &Harness<'static, PagifyApp>| -> egui::Rect {
        h.get_all_by_role(egui::accesskit::Role::Image).next().expect("a thumbnail").rect()
    };
    let before = top_of_the_first(&h);

    h.input_mut().events.push(egui::Event::PointerMoved(before.center()));
    h.run_steps(1);
    h.input_mut().events.push(egui::Event::MouseWheel {
        unit: egui::MouseWheelUnit::Point,
        delta: egui::vec2(0.0, -300.0),
        phase: egui::TouchPhase::Move,
        modifiers: Default::default(),
    });
    h.run_steps(10);
    let after = top_of_the_first(&h);

    assert!(after.top() < before.top() - 100.0, "the grid did not scroll down: {before:?} -> {after:?}");
    assert_eq!(after.left(), before.left(), "the grid moved sideways as well: {before:?} -> {after:?}");
}

/// The Organize grid is a panel of its own: opening it and closing it
/// leaves the Pages rail exactly as wide as it was. They shared one id,
/// so each inherited whatever width the other had last been left at.
#[test]
fn closing_the_organize_grid_leaves_the_pages_rail_as_wide_as_it_was() {
    let mut h = harness_60fps(&fixture("single-page.pdf"), false);
    let rail = |h: &Harness<'static, PagifyApp>| {
        egui::PanelState::load(&h.ctx, egui::Id::new("thumbs")).expect("the rail's panel").outer_rect.width()
    };
    let before = rail(&h);
    h.state_mut().toggle_organize_grid();
    h.run_steps(30);
    h.state_mut().toggle_organize_grid();
    h.run_steps(10);
    assert!(
        (rail(&h) - before).abs() < 0.5,
        "the Pages rail was {before} points wide before the grid and {} after it",
        rail(&h)
    );
}

/// The Pages rail offers a way to bring in pages from another PDF, and
/// they land after the page that is selected.
#[test]
fn pages_from_another_pdf_can_be_inserted_from_the_pages_rail() {
    use egui_kittest::kittest::Queryable;

    let mut h = harness("two-column.pdf");
    // The rail's own control — it opens a file dialog, so only its presence
    // can be asserted here, and the import itself is driven directly.
    let button = h.get_by_label("Insert pages from another PDF").rect();
    let first_thumbnail =
        h.get_all_by_role(egui::accesskit::Role::Image).map(|n| n.rect()).next().expect("a thumbnail");
    assert!(
        button.bottom() <= first_thumbnail.top() && button.right() < 220.0,
        "the button is not in the rail's header, above the thumbnails: {button:?} / {first_thumbnail:?}"
    );

    let before = h.state().tab().doc.as_ref().expect("doc").page_count;
    h.state_mut().tab_mut().organize.organize_selected = vec![0];
    let at = h.state().page_after_selection();
    assert_eq!(at, 1, "pages are not put in just after the selected one");

    h.state_mut().import_at(std::path::Path::new(&fixture("quadrants.pdf")), "all", at);
    h.run_steps(2);

    let doc = h.state().tab().doc.as_ref().expect("doc");
    assert_eq!(doc.page_count, before + 1, "the page was not added");
    assert_eq!(
        doc.strip.size_of(1),
        Some((400.0, 400.0)),
        "the imported page did not land at position 2"
    );
}

// -- rendering off the UI thread ------------------------------------------

/// The app at sixty frames a second, with pages rendered off the UI thread
/// — which the tests otherwise switch off, because a render landing on
/// whichever frame it happens to would make every test that looks at what
/// was drawn depend on timing. Sixty, because the harness's default of four
/// a second would let the zoom settle between any two frames.
pub(crate) fn async_harness(path: &str) -> Harness<'static, PagifyApp> {
    harness_60fps(path, true)
}

pub(crate) fn harness_60fps(path: &str, asynchronous: bool) -> Harness<'static, PagifyApp> {
    let mut app = PagifyApp::new(Some(path));
    assert!(app.tab().doc.is_some(), "{path} did not open");
    app.async_render = asynchronous;
    let mut h = Harness::builder()
        .with_size(egui::vec2(1400.0, 1000.0))
        .with_step_dt(1.0 / 60.0)
        .build_ui_state(
            |ui, app: &mut PagifyApp| {
                let mut frame = eframe::Frame::_new_kittest();
                app.ui(ui, &mut frame);
            },
            app,
        );
    h.run_steps(30);
    h
}

/// Step frames until the worker's answer has been put on screen, or give up.
/// Real time passes between them, because the render is on a real thread.
pub(crate) fn until_a_render_lands(h: &mut Harness<'static, PagifyApp>, already: u64) -> bool {
    for _ in 0..600 {
        std::thread::sleep(std::time::Duration::from_millis(10));
        h.run_steps(1);
        if h.state().render_stats.applied > already {
            return true;
        }
    }
    false
}

/// The same wait, for a render that is expected to come back **dropped**
/// rather than applied — see `a_render_started_before_an_edit_is_never_put_on_screen`.
///
/// **A generous budget on purpose, not a race to tighten.** `collect_renders`
/// drops a result by comparing its epoch against the document's *current*
/// one — deterministic, with no window in which a stale result could slip
/// through regardless of when the worker thread happens to finish. The only
/// thing timing affects is whether the worker gets scheduled at all inside
/// this wait: reported flaky under a full parallel suite, where every core is
/// busy with other tests' own worker threads, not under one test alone. 30
/// seconds of real wall-clock time is still a failure worth seeing, not a
/// wait anybody notices pass.
pub(crate) fn until_a_render_drops(h: &mut Harness<'static, PagifyApp>, already: u64) -> bool {
    for _ in 0..3000 {
        std::thread::sleep(std::time::Duration::from_millis(10));
        h.run_steps(1);
        if h.state().render_stats.dropped > already {
            return true;
        }
    }
    false
}

/// A page of `rects` small filled squares: heavy to render at any zoom,
/// because the cost is in how many things it draws and not how big.
fn heavy_pdf(rects: usize) -> Vec<u8> {
    let mut content = String::with_capacity(rects * 16);
    for i in 0..rects {
        content.push_str(&format!("{} {} 3 3 re f\n", 5 + i % 600, 5 + (i / 600) % 780));
    }
    let objects = [
        "<< /Type /Catalog /Pages 2 0 R >>".to_string(),
        "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_string(),
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Contents 4 0 R >>".to_string(),
        format!("<< /Length {} >>\nstream\n{content}endstream", content.len()),
    ];
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

/// **A zoom does not render on the UI thread while it is moving.**
///
/// Reported from use: "zooming in and out on a pdf page is stuttery… look
/// at logs and see whats causing it". Every new zoom step was a render in
/// the middle of a frame — 15–30 ms for an ordinary page, 80–420 ms at high
/// zoom, 120–270 ms for a drawing of sixty thousand shapes at any zoom.
#[test]
fn a_zoom_does_not_render_on_the_ui_thread_while_it_moves_and_lands_once_it_stops() {
    let mut h = async_harness(&fixture("two-column.pdf"));
    let before = h.state().render_stats;
    // The first look at a page is drawn from its thumbnail while the page
    // itself is rendered, or — with no thumbnail to show — rendered at once.
    assert!(
        before.on_ui_thread + before.requested >= 1,
        "the page was never rendered at all: {before:?}"
    );

    let centre = h.state().tab().view_state.viewport_rect.expect("the page was never drawn").center();
    h.input_mut().events.push(egui::Event::PointerMoved(centre));
    h.run_steps(1);
    for _ in 0..5 {
        h.input_mut().events.push(egui::Event::Zoom(1.3));
        h.run_steps(1);
    }

    let during = h.state().render_stats;
    assert_eq!(during.on_ui_thread, before.on_ui_thread, "a zoom step rendered on the UI thread");
    assert_eq!(during.requested, before.requested, "a render was started while the zoom was still moving");
    assert!(
        !h.state().tab().doc.as_ref().expect("doc").caches.textures.is_empty(),
        "the page has nothing to draw meanwhile"
    );

    // Stopped: after the settle window a render is asked for, off this
    // thread, and lands.
    h.run_steps(12);
    assert!(h.state().render_stats.requested > before.requested, "nothing was asked for once the zoom stopped");
    assert!(until_a_render_lands(&mut h, before.applied), "the render never came back");
    assert_eq!(h.state().render_stats.on_ui_thread, before.on_ui_thread, "it was rendered on the UI thread");
}

/// While the picture for the new size is being made, the page is drawn from
/// the one that is already held — and swapped for the right one when it
/// arrives.
#[test]
fn the_page_is_drawn_from_what_is_held_until_the_right_picture_arrives() {
    let mut h = async_harness(&fixture("two-column.pdf"));
    let ctx = h.ctx.clone();
    let held = h.state_mut().texture_for(&ctx, 0, 1.0).expect("a page");
    let before = h.state().render_stats;

    let meanwhile = h.state_mut().texture_for(&ctx, 0, 3.0).expect("a stand-in");
    assert_eq!(meanwhile.id(), held.id(), "something else was drawn while the right picture was made");
    assert_eq!(h.state().render_stats.on_ui_thread, before.on_ui_thread, "it was rendered on the UI thread");
    assert_eq!(h.state().render_stats.requested, before.requested + 1, "the right picture was not asked for");

    assert!(until_a_render_lands(&mut h, before.applied), "the render never came back");
    let arrived = h.state_mut().texture_for(&ctx, 0, 3.0).expect("the page");
    assert_ne!(arrived.id(), held.id(), "the stand-in was never replaced");
}

/// A render for a page that has since changed is dropped, never shown.
///
/// **Checks the drop, not a frozen `applied` count.** This used to also
/// assert `after.applied == before.applied` — wrong, and proven so by
/// tracing a real run: `rendered_is_stale` clears every cached texture
/// (`main.rs`'s own doc on `Self::rendered_is_stale`), so the app's
/// ordinary "something has to be on screen" redraw of the current view
/// legitimately asks for a **fresh**, non-stale picture once the old ones
/// are gone — and that one is correctly applied, same as it would be after
/// any edit. It can land in the very same `collect_renders` batch as the
/// stale one being dropped (`done.image`'s `Ok` and `Err` arms both drain
/// from one `try_recv` loop before either is processed), so no amount of
/// polling can ever observe "dropped, and nothing else has applied yet" as
/// two separate moments — they are not separate events. Reported flaky
/// under a full parallel suite because contention makes that ordinary
/// redraw more likely to land inside the test's own polling window, not
/// because the drop itself is ever in doubt — `collect_renders` drops on
/// an unconditional epoch comparison, with no timing window in which a
/// stale result could slip through regardless of when the worker finishes.
#[test]
fn a_render_started_before_an_edit_is_never_put_on_screen() {
    let mut h = async_harness(&fixture("two-column.pdf"));
    let ctx = h.ctx.clone();
    let _ = h.state_mut().texture_for(&ctx, 0, 1.0).expect("a page");
    let before = h.state().render_stats;

    let _ = h.state_mut().texture_for(&ctx, 0, 3.0);
    // The page changes while the worker has it.
    h.state_mut().tab_mut().doc.as_mut().expect("doc").rendered_is_stale();
    assert!(until_a_render_drops(&mut h, before.dropped), "the stale render never came back");
    let after = h.state().render_stats;
    assert_eq!(after.dropped, before.dropped + 1, "the stale render was not dropped: {after:?}");
    assert!(
        h.state().tab().doc.as_ref().expect("doc").caches.textures.keys().all(|(_, step, _)| *step < 12),
        "a picture of the page as it was is on screen"
    );
}

/// **A page that takes a long time to render does not stop the frames.**
///
/// The case from the report: a drawing of tens of thousands of shapes, where
/// a render is a quarter of a second at any zoom. The frames drawn while it
/// is being made must each be a small fraction of that — if anything in a
/// frame waited for the render, they would not be.
#[test]
fn a_heavy_page_does_not_stop_the_frames_while_it_renders() {
    let dir = std::env::temp_dir().join("pagify-app-tests");
    std::fs::create_dir_all(&dir).expect("scratch dir");
    let path = dir.join(format!("{:?}-heavy.pdf", std::thread::current().id()).replace(['(', ')', ' '], ""));
    // **Heavy enough to prove something.** The page is made until a render of
    // it takes a quarter of a second on this machine — a test of "the frames
    // do not wait for a render" that passes because the render was quick
    // would be worth nothing.
    let mut rects = 150_000;
    loop {
        std::fs::write(&path, heavy_pdf(rects)).expect("write the page");
        let session = Session::open(&path).expect("open");
        let started = std::time::Instant::now();
        session.render_page(0, 1.6).expect("render");
        if started.elapsed() >= std::time::Duration::from_millis(250) || rects >= 2_400_000 {
            break;
        }
        rects *= 2;
    }

    let mut h = async_harness(&path.to_string_lossy());
    let before = h.state().render_stats;

    let centre = h.state().tab().view_state.viewport_rect.expect("the page was never drawn").center();
    h.input_mut().events.push(egui::Event::PointerMoved(centre));
    h.run_steps(1);
    h.input_mut().events.push(egui::Event::Zoom(1.6));
    h.run_steps(12);
    assert!(h.state().render_stats.requested > before.requested, "no render was started");

    let mut longest = std::time::Duration::ZERO;
    let mut landed = false;
    let started = std::time::Instant::now();
    while started.elapsed() < std::time::Duration::from_secs(20) {
        let frame = std::time::Instant::now();
        h.run_steps(1);
        longest = longest.max(frame.elapsed());
        if h.state().render_stats.applied > before.applied {
            landed = true;
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    let _ = std::fs::remove_file(&path);
    assert!(landed, "the render never came back");
    let slowest = h.state().render_stats.slowest_ms;
    assert!(slowest >= 150, "the page was not heavy enough to prove anything: a render took {slowest} ms");
    assert!(
        longest < std::time::Duration::from_millis(80),
        "a frame took {longest:?} while the page took {slowest} ms to render — the UI waited for it"
    );
}

/// What is locked on a page is asked of the engine once, not on every frame
/// — a frame that calls into the engine waits for any render under way —
/// and is asked again after anything that could have changed it.
#[test]
fn what_is_locked_on_a_page_is_kept_until_the_page_changes() {
    let mut app = PagifyApp::new(Some(&fixture("two-column.pdf")));
    assert!(app.locked_items_on(0).is_empty());
    let doc = app.tab().doc.as_ref().expect("doc");
    assert!(doc.caches.locked.borrow().contains_key(&0), "the answer was not kept");

    // Planted where the engine knows nothing of it, so only a cache can
    // return it.
    let planted = pdf_core::document::LockedItem {
        id: "planted".into(),
        rect: pdf_core::document::Rect { left: 0.0, top: 0.0, right: 1.0, bottom: 1.0 },
        is_area: false,
        stale: false,
    };
    doc.caches.locked.borrow_mut().insert(0, vec![planted.clone()]);
    assert_eq!(app.locked_items_on(0), vec![planted], "the engine was asked again");

    app.tab_mut().doc.as_mut().expect("doc").rendered_is_stale();
    assert!(
        app.tab().doc.as_ref().expect("doc").caches.locked.borrow().is_empty(),
        "a page that has changed kept its old answer"
    );
}

/// Not a test: a measurement, for answering "how bad is the zoom on this
/// file" with numbers. Frame times through a zoom gesture and the settle
/// after it, with pages rendered on the UI thread (what it used to do) and
/// off it.
///
/// ```text
/// PAGIFY_PDFIUM_LIB=<pdfium> PAGIFY_BENCH_PDF=<file.pdf> \
///   cargo test -p pagify_app --release zoom_frame_times -- --ignored --nocapture
/// ```
#[test]
#[ignore]
fn zoom_frame_times_on_a_real_file() {
    let Some(path) = std::env::var_os("PAGIFY_BENCH_PDF") else { return };
    let path = path.to_string_lossy().into_owned();
    for asynchronous in [false, true] {
        let mut h = harness_60fps(&path, asynchronous);
        let centre = h.state().tab().view_state.viewport_rect.expect("the page was never drawn").center();
        h.input_mut().events.push(egui::Event::PointerMoved(centre));
        h.run_steps(1);

        let mut frames: Vec<std::time::Duration> = Vec::new();
        let mut slow: Vec<String> = Vec::new();
        let mut note = |h: &Harness<'static, PagifyApp>, n: usize, took: std::time::Duration, before: RenderStats| {
            if took > std::time::Duration::from_millis(50) {
                let now = h.state().render_stats;
                slow.push(format!(
                    "frame {n}: {took:.0?} (applied +{}, requested +{}, ui renders +{})",
                    now.applied - before.applied,
                    now.requested - before.requested,
                    now.on_ui_thread - before.on_ui_thread + now.detail_on_ui_thread - before.detail_on_ui_thread
                ));
            }
        };
        for n in 0..12 {
            h.input_mut().events.push(egui::Event::Zoom(1.2));
            let before = h.state().render_stats;
            let t = std::time::Instant::now();
            h.run_steps(1);
            frames.push(t.elapsed());
            note(&h, n, t.elapsed(), before);
        }
        for n in 12..92 {
            std::thread::sleep(std::time::Duration::from_millis(10));
            let before = h.state().render_stats;
            let t = std::time::Instant::now();
            h.run_steps(1);
            frames.push(t.elapsed());
            note(&h, n, t.elapsed(), before);
        }
        for line in &slow {
            println!("        {line}");
        }
        let longest = frames.iter().max().copied().unwrap_or_default();
        let stalls = frames.iter().filter(|f| **f > std::time::Duration::from_millis(50)).count();
        let total: std::time::Duration = frames.iter().sum();
        println!(
            "{:>5}: longest frame {longest:>9.1?}, frames over 50 ms: {stalls:>2} of {}, time in frames {total:>9.1?}, {:?}",
            if asynchronous { "async" } else { "sync" },
            frames.len(),
            h.state().render_stats
        );
    }
}

/// Which pictures of a page are thrown away: all but the nearest few.
#[test]
fn only_the_scales_nearest_the_current_one_are_kept() {
    assert_eq!(scales_to_drop(&[4, 5, 6, 7, 8, 9], 9, 3), vec![6, 5, 4]);
    assert_eq!(scales_to_drop(&[9, 4, 7, 5, 8, 6], 5, 3), vec![7, 8, 9], "the three nearest 5 are 5, 4 and 6");
    assert!(scales_to_drop(&[1, 2], 1, 3).is_empty(), "nothing to drop below the limit");
    assert!(scales_to_drop(&[], 1, 3).is_empty());
}

/// The quick conversion of a page's pixels is the general one, for every
/// pixel — opaque ones, which are nearly all of them, and any that are not.
#[test]
fn the_quick_conversion_of_a_page_gives_what_the_general_one_does() {
    let mut pixels = Vec::new();
    for i in 0..4096u32 {
        let v = (i.wrapping_mul(2654435761) >> 8) as u8;
        let alpha = if i % 7 == 0 { v } else { 255 };
        pixels.extend_from_slice(&[v, v.wrapping_mul(3), v.wrapping_add(91), alpha]);
    }
    let raster = PageRaster { width: 64, height: 64, pixels: pixels.clone(), from_cache: false };
    let quick = page_to_image(&raster);
    let general = egui::ColorImage::from_rgba_unmultiplied([64, 64], &pixels);
    assert_eq!(quick.size, general.size);
    for (i, (a, b)) in quick.pixels.iter().zip(&general.pixels).enumerate() {
        assert_eq!(a, b, "pixel {i} differs");
    }
}

/// A small solid picture, as an uploaded signature file would decode to
/// — the same helper `lock_wiring_tests` keeps its own copy of.
fn solid_rgba(width: u32, height: u32, rgb: [u8; 3]) -> Vec<u8> {
    let mut rgba = vec![0u8; (width * height * 4) as usize];
    for pixel in rgba.chunks_exact_mut(4) {
        pixel.copy_from_slice(&[rgb[0], rgb[1], rgb[2], 255]);
    }
    rgba
}

/// **The real pointer path, not just the commit function.** Every
/// signature move/resize/rotate test in `lock_wiring_tests` drives
/// `finish_signature_grab` directly — proving the arithmetic, never
/// that a real click-and-drag actually reaches it, or that placing one
/// leaves the tool out of the way afterward. This one goes through
/// `Harness` instead, where `drag` and `click` already live: a real
/// click places the signature, a second real click selects it, and a
/// real drag on its bottom-right handle resizes it — start to finish
/// the way someone actually using the app would, not the commit shape
/// either side of it already has covered.
#[test]
fn dragging_a_signature_handle_through_the_real_pointer_path_resizes_it() {
    let mut h = harness("two-column.pdf");
    let path = std::env::temp_dir()
        .join(format!("pagify-test-signatures-{}-real-drag.json", std::process::id()));
    let _ = std::fs::remove_file(&path);
    h.state_mut().signatures = Default::default();
    h.state_mut().signatures_path = Some(path.clone());
    h.state_mut()
        .save_uploaded_signature("mine", solid_rgba(4, 4, [40, 90, 200]), 4, 4)
        .expect("kept");
    let view = h.state().tab().view_state.last_view.expect("the page was never drawn");

    h.state_mut().submit("signature");
    h.run_steps(1);
    click(&mut h, view.to_screen(AppPoint { x: 100.0, y: 400.0 }));
    assert!(
        h.state().tab().tool.is_none(),
        "the signature tool stayed armed after placing one, and would have swallowed \
         the very next click instead of selecting what was just placed"
    );

    let mark = h.state().tab().doc.as_ref().unwrap().session.image_signature_marks(0).unwrap().remove(0);

    let middle = view.to_screen(AppPoint {
        x: ((mark.rect.left + mark.rect.right) / 2.0) as f64,
        y: ((mark.rect.top + mark.rect.bottom) / 2.0) as f64,
    });
    click(&mut h, middle);
    assert!(
        h.state().tab().selection.signature_selected.is_some(),
        "clicking the signature through a real pointer event did not select it"
    );

    let corner = view.to_screen(AppPoint { x: mark.rect.right as f64, y: mark.rect.bottom as f64 });
    let (w, ht) = (mark.rect.right - mark.rect.left, mark.rect.bottom - mark.rect.top);
    let target = view.to_screen(AppPoint {
        x: (mark.rect.right - w / 2.0) as f64,
        y: (mark.rect.bottom - ht / 2.0) as f64,
    });
    drag(&mut h, corner, target);

    let resized = h.state().tab().doc.as_ref().unwrap().session.image_signature_marks(0).unwrap().remove(0);
    assert!(
        (resized.rect.right - resized.rect.left) < w - 1.0,
        "dragging the bottom-right handle through a real pointer event did not resize the \
         signature: before {:?}, after {:?}",
        mark.rect,
        resized.rect
    );

    let _ = std::fs::remove_file(&path);
}

/// The rotate ring, the same way: a real click to place, a real click
/// to select, then a real drag starting exactly where the ring is
/// drawn.
#[test]
fn dragging_the_rotate_handle_through_the_real_pointer_path_turns_it() {
    let mut h = harness("two-column.pdf");
    let path = std::env::temp_dir()
        .join(format!("pagify-test-signatures-{}-real-rotate.json", std::process::id()));
    let _ = std::fs::remove_file(&path);
    h.state_mut().signatures = Default::default();
    h.state_mut().signatures_path = Some(path.clone());
    h.state_mut()
        .save_uploaded_signature("mine", solid_rgba(4, 4, [40, 90, 200]), 4, 4)
        .expect("kept");
    let view = h.state().tab().view_state.last_view.expect("the page was never drawn");

    h.state_mut().submit("signature");
    h.run_steps(1);
    click(&mut h, view.to_screen(AppPoint { x: 100.0, y: 400.0 }));
    assert!(
        h.state().tab().tool.is_none(),
        "the signature tool stayed armed after placing one, and would have swallowed \
         the very next click instead of selecting what was just placed"
    );

    let mark = h.state().tab().doc.as_ref().unwrap().session.image_signature_marks(0).unwrap().remove(0);

    let middle = view.to_screen(AppPoint {
        x: ((mark.rect.left + mark.rect.right) / 2.0) as f64,
        y: ((mark.rect.top + mark.rect.bottom) / 2.0) as f64,
    });
    click(&mut h, middle);
    assert!(
        h.state().tab().selection.signature_selected.is_some(),
        "clicking the signature did not select it"
    );

    let ring = PagifyApp::rotate_handle_screen_pos(&mark.rect, view);
    drag(&mut h, ring, ring + egui::vec2(60.0, 0.0));

    let turned = h.state().tab().doc.as_ref().unwrap().session.image_signature_marks(0).unwrap().remove(0);
    assert!(
        (turned.rotation - mark.rotation).abs() > 1.0,
        "dragging the rotate ring through a real pointer event did not turn the signature: \
         rotation stayed {}",
        turned.rotation
    );

    let _ = std::fs::remove_file(&path);
}

/// **The object tool's own resize handles, through the real pointer
/// path.** The signature tool got this exact fix
/// (`Self::signature_hover_handle`) after a corner-aimed resize was
/// silently read as a body move, because `egui` only recognises a drag
/// once the pointer has moved a few pixels — by which point it has
/// already slid off an eight-pixel handle. The generic object tool
/// shared `Handle`/`Grab` with the signature one but not the fix, so an
/// ordinary picture's resize handles had the identical bug. This proves
/// the parallel fix (`Self::object_hover_handle`) the same way the
/// signature one above is proven: a real click to select, then a real
/// drag starting exactly on the bottom-right handle.
#[test]
fn dragging_an_object_handle_through_the_real_pointer_path_resizes_it() {
    let mut h = harness("pictures.pdf");
    let view = h.state().tab().view_state.last_view.expect("the page was never drawn");

    h.state_mut().submit("editobject");
    h.run_steps(1);

    let image = h.state().tab().doc.as_ref().unwrap().session.images_on(0).unwrap().remove(0);
    let middle = view.to_screen(AppPoint {
        x: ((image.rect.left + image.rect.right) / 2.0) as f64,
        y: ((image.rect.top + image.rect.bottom) / 2.0) as f64,
    });
    click(&mut h, middle);
    assert!(
        h.state().tab().selection.selected.is_some(),
        "clicking the picture through a real pointer event did not select it"
    );

    let corner = view.to_screen(AppPoint { x: image.rect.right as f64, y: image.rect.bottom as f64 });
    let (w, ht) = (image.rect.right - image.rect.left, image.rect.bottom - image.rect.top);
    let target = view.to_screen(AppPoint {
        x: (image.rect.right - w / 2.0) as f64,
        y: (image.rect.bottom - ht / 2.0) as f64,
    });
    drag(&mut h, corner, target);

    let resized = h.state().tab().doc.as_ref().unwrap().session.images_on(0).unwrap().remove(0);
    assert!(
        (resized.rect.right - resized.rect.left) < w - 1.0,
        "dragging the bottom-right handle through a real pointer event did not resize the \
         picture: before {:?}, after {:?}",
        image.rect,
        resized.rect
    );
}

/// **Reported from use: a drawn shape could only be moved by typing
/// `move` and clicking twice — a mouse drag on it did nothing but start
/// a marquee.** Through the real pointer path this time, not the typed
/// command chain `move_moves_once_something_is_selected` already covers.
#[test]
fn dragging_a_markup_shape_through_the_real_pointer_path_moves_it() {
    let mut h = harness("pictures.pdf");
    h.state_mut().submit("l 300,300 400,300");
    h.state_mut().submit("all");
    h.run_steps(3);

    let view = h.state().tab().view_state.last_view.expect("the page was never drawn");
    let before = match &h.state().tab().markup.existing(0).unwrap().objects()[0].geom {
        cad_kernel::Geom::Line(l) => *l,
        other => panic!("{other:?}"),
    };
    let space = h.state().tab().markup.existing(0).unwrap().space();
    let start_app = space.from_kernel(cad_kernel::Vec2::new(
        (before.a.x + before.b.x) / 2.0,
        (before.a.y + before.b.y) / 2.0,
    ));
    let end_app = AppPoint { x: start_app.x + 50.0, y: start_app.y - 30.0 };
    drag(&mut h, view.to_screen(start_app), view.to_screen(end_app));

    let after = match &h.state().tab().markup.existing(0).unwrap().objects()[0].geom {
        cad_kernel::Geom::Line(l) => *l,
        other => panic!("{other:?}"),
    };
    // Not an exact delta: kittest's synthetic pointer-move sequence does
    // not always land on the last requested position by the frame a
    // release is processed — see `dragging_an_object_handle_through_the
    // _real_pointer_path_resizes_it`, which checks direction and size
    // for the same reason rather than an exact number. App space counts
    // downwards, kernel space upwards, so a drag up the screen is a
    // *positive* change in kernel y.
    let (dx, dy) = (after.a.x - before.a.x, after.a.y - before.a.y);
    assert!(dx > 20.0, "did not move right through a real drag: {before:?} then {after:?}");
    assert!(dy > 15.0, "did not move up through a real drag: {before:?} then {after:?}");
}

/// **The properties panel's own "Filled" checkbox, clicked for real** —
/// not `Layer::set_filled` called directly, which `markup::tests`
/// already covers, but the actual widget the panel puts on screen when
/// a fillable shape is selected.
#[test]
fn the_filled_checkbox_in_the_properties_panel_fills_the_selected_shape() {
    use egui_kittest::kittest::Queryable;

    let mut h = harness("pictures.pdf");
    h.state_mut().submit("circle");
    h.state_mut().submit("pick 300,300");
    h.state_mut().submit("pick 340,300");
    h.state_mut().submit("all");
    h.run_steps(3);
    assert!(
        !h.state().tab().markup.existing(0).unwrap().is_filled(0),
        "the circle started out filled, so this proves nothing"
    );

    h.get_by_label_contains("Filled").click();
    h.run_steps(2);

    assert!(
        h.state().tab().markup.existing(0).unwrap().is_filled(0),
        "clicking the panel's Filled checkbox did not fill the shape"
    );
}

/// **Clicking Add Text arms the click-and-drag box, rather than putting
/// anything in the command line** — the actual click, through the real
/// ribbon button, not just the static check
/// `add_text_buttons_submit_bare_and_arm_the_box_tool` runs against the
/// button table.
#[test]
fn clicking_add_text_arms_the_box_tool_rather_than_filling_the_command_box() {
    use egui_kittest::kittest::Queryable;

    let mut h = harness("two-column.pdf");
    h.state_mut().tab_mut().ribbon = Tab::Edit;
    h.run_steps(2);

    h.get_by_label_contains("Add Text").click();
    h.run_steps(2);

    assert!(
        h.state().cmd.input().is_empty(),
        "the button left something in the command box: {:?}",
        h.state().cmd.input()
    );
    assert!(
        matches!(h.state().tab().tool.as_ref().map(|t| &t.kind), Some(Tool::PlaceText)),
        "clicking Add Text should have armed the text-box tool"
    );
    let history: Vec<String> =
        h.state().cmd.history().iter().map(|e| e.text.clone()).collect();
    assert!(
        !history.iter().any(|s| s.contains("the words to write")),
        "it errored instead of arming the tool: {history:?}"
    );
}

/// A box smaller than a click's own jitter is refused rather than
/// opened — nothing typed into it could ever fit.
#[test]
fn a_text_box_too_small_to_type_into_is_refused() {
    let mut h = harness("two-column.pdf");
    let result = h.state_mut().begin_text_box(
        0,
        AppPoint { x: 50.0, y: 500.0 },
        AppPoint { x: 52.0, y: 501.0 },
    );
    assert!(result.is_err(), "a two-point box should have been refused");
    assert!(h.state().tab().edit.new_text_box.is_none());
}

/// A box smaller than the default font size is not refused — the font
/// size shrinks to fit the box instead, in both directions.
#[test]
fn a_text_box_smaller_than_the_default_font_shrinks_the_font_to_fit() {
    let mut h = harness("two-column.pdf");
    h.state_mut()
        .begin_text_box(0, AppPoint { x: 50.0, y: 500.0 }, AppPoint { x: 250.0, y: 508.0 })
        .expect("a box above the noise floor should open");
    assert_eq!(
        h.state().tab().edit.new_text_box.as_ref().map(|b| b.size),
        Some(8.0),
        "an 8pt-tall box should have shrunk the font to 8pt, not refused or kept the 14pt default"
    );

    h.state_mut()
        .begin_text_box(0, AppPoint { x: 50.0, y: 500.0 }, AppPoint { x: 55.0, y: 540.0 })
        .expect("a narrow box above the noise floor should open");
    assert_eq!(
        h.state().tab().edit.new_text_box.as_ref().map(|b| b.size),
        Some(5.0),
        "a 5pt-wide box should have shrunk the font to 5pt too, not just gone by height"
    );

    h.state_mut()
        .begin_text_box(0, AppPoint { x: 50.0, y: 500.0 }, AppPoint { x: 250.0, y: 540.0 })
        .expect("a normal box should still open");
    assert_eq!(
        h.state().tab().edit.new_text_box.as_ref().map(|b| b.size),
        Some(14.0),
        "a box already bigger than the default should keep the usual default size"
    );
}

/// **The click-and-drag box, end to end**: drag one out, type into it,
/// press Add to Page in the panel, and the words land on the page as a
/// real run — not the old flow where the words had to be typed into the
/// command box before anywhere to put them was known.
#[test]
fn dragging_out_a_text_box_and_inserting_writes_a_real_run() {
    use egui_kittest::kittest::Queryable;

    let mut h = harness("two-column.pdf");
    h.state_mut()
        .begin_text_box(0, AppPoint { x: 50.0, y: 500.0 }, AppPoint { x: 250.0, y: 540.0 })
        .expect("box opened");
    h.run_steps(2);
    assert!(h.state().tab().edit.new_text_box.is_some(), "the box should be open");

    h.state_mut().tab_mut().edit.new_text_box.as_mut().expect("open").buffer = "Hello there".to_string();
    h.run_steps(1);

    h.get_by_label_contains("Add to Page").click();
    h.run_steps(2);

    assert!(h.state().tab().edit.new_text_box.is_none(), "Add to Page should have closed the box");
    let text = h.state().tab().doc
        .as_ref()
        .expect("open")
        .session
        .characters(0)
        .map(|c| c.text().to_string())
        .unwrap_or_default();
    assert!(text.contains("Hello") && text.contains("there"), "the typed words are not on the page: {text:?}");
}

/// Cancel discards what was typed — the box closes and nothing is
/// written, the same as Escape.
#[test]
fn canceling_a_new_text_box_writes_nothing() {
    use egui_kittest::kittest::Queryable;

    let mut h = harness("two-column.pdf");
    h.state_mut()
        .begin_text_box(0, AppPoint { x: 50.0, y: 500.0 }, AppPoint { x: 250.0, y: 540.0 })
        .expect("box opened");
    h.state_mut().tab_mut().edit.new_text_box.as_mut().expect("open").buffer = "should not appear".to_string();
    h.run_steps(2);

    h.get_by_label_contains("Cancel").click();
    h.run_steps(1);

    assert!(h.state().tab().edit.new_text_box.is_none(), "Cancel should have closed the box");
    let text = h.state().tab().doc
        .as_ref()
        .expect("open")
        .session
        .characters(0)
        .map(|c| c.text().to_string())
        .unwrap_or_default();
    assert!(!text.contains("should not appear"), "Cancel wrote the words anyway");
}

/// **Later** just closes the prompt — no quit, no staged update, nothing
/// else touched. Asked again only on the next launch.
#[test]
fn clicking_later_dismisses_the_update_prompt_without_updating() {
    use egui_kittest::kittest::Queryable;

    let mut h = harness("two-column.pdf");
    h.state_mut().update_state.update_available = Some(("9.9.9".to_string(), std::env::temp_dir()));
    h.run_steps(2);

    h.get_by_label_contains("Later").click();
    h.run_steps(1);

    assert!(h.state().update_state.update_available.is_none(), "Later should have dismissed the prompt");
    assert!(h.state().pending_update.is_none(), "Later must never stage an update");
    assert!(h.state().tab().closing.is_none(), "Later must never start a quit");
}

/// **The one property that matters most here.** An update is a quit that
/// relaunches a newer build — so it must go through the exact same
/// unsaved-work guard a plain quit does, never straight past it. Proven
/// through the real button, not by calling the guard directly: the whole
/// point is that the *click* reaches this path, not just the function
/// underneath it.
#[test]
fn clicking_update_now_stages_it_and_asks_about_unsaved_work_first() {
    use egui_kittest::kittest::Queryable;

    let mut h = harness("two-column.pdf");
    h.state_mut().submit("l 10,10 100,100");
    assert!(h.state().unsaved().is_some(), "test assumption: the mark should count as unsaved");

    let source = std::env::temp_dir();
    h.state_mut().update_state.update_available = Some(("9.9.9".to_string(), source.clone()));
    h.run_steps(2);

    h.get_by_label_contains("Update now").click();
    h.run_steps(1);

    assert!(h.state().update_state.update_available.is_none(), "the prompt should have closed");
    assert_eq!(h.state().pending_update, Some(source), "Update now should have staged the update");
    assert_eq!(
        h.state().tab().closing,
        Some(Closing::Program),
        "an update with unsaved work should ask about it exactly like a normal quit — not \
         exit straight through"
    );
}

/// Escape while composing a new text box discards it too, the same as
/// every other tool it clears when put down.
#[test]
fn escaping_a_new_text_box_discards_it() {
    let mut h = harness("two-column.pdf");
    h.state_mut()
        .begin_text_box(0, AppPoint { x: 50.0, y: 500.0 }, AppPoint { x: 250.0, y: 540.0 })
        .expect("box opened");
    assert!(h.state().tab().edit.new_text_box.is_some());

    h.state_mut().escape();

    assert!(h.state().tab().edit.new_text_box.is_none(), "Escape should have discarded the box");
}

/// **The alignment picked in the panel actually moves where the line
/// lands** — "Right" should end up near the box's right edge, not its
/// left, and the two must differ by most of the box's own width for a
/// short word in a wide box.
#[test]
fn right_aligned_text_lands_near_the_boxs_right_edge() {
    use egui_kittest::kittest::Queryable;

    let mut h = harness("two-column.pdf");
    let (left, right) = (50.0, 400.0);
    h.state_mut()
        .begin_text_box(0, AppPoint { x: left, y: 500.0 }, AppPoint { x: right, y: 540.0 })
        .expect("box opened");
    h.run_steps(2);
    h.state_mut().tab_mut().edit.new_text_box.as_mut().expect("open").buffer = "Hi".to_string();
    h.run_steps(1);

    h.get_by_label("Align right").click();
    h.run_steps(1);
    assert_eq!(
        h.state().tab().edit.new_text_box.as_ref().map(|b| b.align),
        Some(TextAlign::Right),
        "clicking Right should have picked it"
    );

    h.get_by_label_contains("Add to Page").click();
    h.run_steps(2);

    let runs = h.state().tab().doc.as_ref().expect("open").session.text_runs(0).expect("runs");
    let mine = runs.iter().find(|r| r.text.contains("Hi")).expect("the written run");
    let midpoint = (left + right) as f32 / 2.0;
    assert!(
        mine.origin.x > midpoint,
        "a right-aligned short word should sit past the middle of the box: origin.x={}, box=[{left},{right}]",
        mine.origin.x
    );
}

/// **Dragging from inside a form's own shape starts a marquee, not a
/// grab.** `thing_at`'s `grouped` catch-all — offered so a *click* on a
/// form's shape has something to land on, since `shapes()` only ever
/// looks at depth 0 — used to also answer a drag that started there,
/// which meant a drag could never begin empty-handed enough to become a
/// marquee anywhere the page's own forms reached. A plain click there
/// still reaches the group exactly as before.
#[test]
fn dragging_from_inside_a_forms_own_shape_starts_a_marquee_not_a_grab() {
    let mut h = harness("forms.pdf");
    let view = h.state().tab().view_state.last_view.expect("the page was never drawn");
    h.state_mut().submit("editobject");
    h.run_steps(1);

    // The centre of the shape `forms.pdf` draws inside its own form —
    // found only through `grouped`, never `words`/`pictures`/`shapes`.
    let inside_group = AppPoint { x: 142.0, y: 272.0 };

    let from = view.to_screen(inside_group);
    let to = view.to_screen(AppPoint { x: inside_group.x + 60.0, y: inside_group.y + 40.0 });
    drag(&mut h, from, to);
    let grabbed_the_group =
        h.state().tab().selection.selected.as_ref().is_some_and(|s| s.what == "the group it is drawn in");
    assert!(
        !grabbed_the_group,
        "a drag starting inside the form's shape grabbed the whole group instead of \
         starting a marquee: {:?}",
        h.state().tab().selection.selected
    );

    // The same spot, clicked rather than dragged, still reaches the
    // group exactly as it always did — this is about what a *drag*
    // starts, not about taking the click away from it.
    click(&mut h, from);
    assert_eq!(
        h.state().tab().selection.selected.as_ref().map(|s| s.what),
        Some("the group it is drawn in"),
        "a plain click on the same spot should still select the group: {:?}",
        h.state().tab().selection.selected
    );
}

/// **Reported from use: dragging out a marquee across several objects
/// kept grabbing and moving whichever one the press happened to land
/// on first.** A drag starting directly on an *ordinary, unselected*
/// picture — not just the form-group catch-all the test above covers —
/// must draw a marquee too, not move it. Selecting one thing is only
/// ever a plain click now; a drag always means the marquee, unless it
/// starts on something already selected (see the test below).
#[test]
fn dragging_an_unselected_picture_starts_a_marquee_not_a_grab() {
    let mut h = harness("pictures.pdf");
    let view = h.state().tab().view_state.last_view.expect("the page was never drawn");
    h.state_mut().submit("editobject");
    h.run_steps(1);

    let image = h.state().tab().doc.as_ref().unwrap().session.images_on(0).unwrap().remove(0);
    assert!(h.state().tab().selection.selected.is_none(), "nothing should be selected yet");
    let middle = view.to_screen(AppPoint {
        x: ((image.rect.left + image.rect.right) / 2.0) as f64,
        y: ((image.rect.top + image.rect.bottom) / 2.0) as f64,
    });

    drag(&mut h, middle, middle + egui::vec2(60.0, 40.0));

    let after = h.state().tab().doc.as_ref().unwrap().session.images_on(0).unwrap().remove(0);
    assert_eq!(
        after.rect, image.rect,
        "the picture moved — a drag starting on it should have drawn a marquee instead"
    );
}

/// The other half of the fix above: a picture already selected by a
/// plain click still moves on a drag starting on its own body — moving
/// something did not become harder, it just now takes the click first.
#[test]
fn dragging_an_already_selected_pictures_body_still_moves_it() {
    let mut h = harness("pictures.pdf");
    let view = h.state().tab().view_state.last_view.expect("the page was never drawn");
    h.state_mut().submit("editobject");
    h.run_steps(1);

    let image = h.state().tab().doc.as_ref().unwrap().session.images_on(0).unwrap().remove(0);
    let middle = view.to_screen(AppPoint {
        x: ((image.rect.left + image.rect.right) / 2.0) as f64,
        y: ((image.rect.top + image.rect.bottom) / 2.0) as f64,
    });
    click(&mut h, middle);
    assert!(h.state().tab().selection.selected.is_some(), "the click should have selected the picture");

    drag(&mut h, middle, middle + egui::vec2(60.0, 40.0));

    let after = h.state().tab().doc.as_ref().unwrap().session.images_on(0).unwrap().remove(0);
    assert_ne!(
        after.rect, image.rect,
        "a drag on the already-selected picture's own body should still move it"
    );
}

/// The Home screen with nothing open — where the fonts panel lives — full
/// window, real layout, not just the state `pointer_tests` checks.
///
/// Deliberately does not click "Add font…": that dispatches
/// `outlinedfont`, which opens a real, blocking native OS dialog — see
/// `outlined_font_dialog`. Nothing in this suite clicks it, the same way
/// nothing clicks the equivalent "Open a PDF…" for `Verb::OpenDialog`.
#[test]
fn the_home_screen_renders_the_outlined_fonts_panel() {
    use egui_kittest::kittest::Queryable;

    let mut app = PagifyApp::new(None);
    assert!(app.tab_mut().doc.is_none(), "this checks the backstage view, which needs nothing open");
    // **Not the reader's own list.** This test asserts what the panel says
    // when nothing has been added, and it was reading the settings file of
    // whoever ran it — so adding a font to your own copy of the program
    // broke the suite. What is on this machine is not what is under test.
    app.outlined_fonts = Default::default();
    let mut h = Harness::builder()
        .with_size(egui::vec2(1400.0, 1000.0))
        .build_ui_state(
            |ui, app: &mut PagifyApp| {
                let mut frame = eframe::Frame::_new_kittest();
                app.ui(ui, &mut frame);
            },
            app,
        );
    h.run_steps(4);

    h.get_by_label_contains("Outlined-Text Fonts");
    h.get_by_label_contains("Add font");
    h.get_by_label_contains("No extra fonts added");
    // Nothing to remove with none configured — confirms the query above
    // is not simply matching everything on screen.
    assert!(h.query_by_label_contains("Remove").is_none());
}

/// **The command box's own placeholder says "type a command" — that has
/// to be true from the first frame.** Nothing on a fresh Home screen
/// took keyboard focus by default, so typing immediately after launch
/// went nowhere at all: not an error, not even into the box, just
/// silently discarded with no widget to route it to. Reported from use
/// as `status` "trying to open a file" — the actual mechanism was a
/// click aimed at finding somewhere to type landing on the prominent
/// "Open a PDF…" button instead, because the thin command bar gave no
/// sign it was the thing to click.
#[test]
fn the_command_box_is_focused_on_a_fresh_home_screen() {
    let mut app = PagifyApp::new(None);
    app.outlined_fonts = Default::default();
    let mut h = Harness::builder()
        .with_size(egui::vec2(1400.0, 1000.0))
        .build_ui_state(
            |ui, app: &mut PagifyApp| {
                let mut frame = eframe::Frame::_new_kittest();
                app.ui(ui, &mut frame);
            },
            app,
        );
    h.run_steps(4);
    assert!(
        h.ctx.memory(|m| m.has_focus(egui::Id::new(COMMAND_INPUT))),
        "the command box does not have focus on a fresh Home screen (ribbon: {:?})",
        h.state().tab().ribbon
    );

    // Typing immediately, with nothing clicked first, must reach the
    // box and be submittable — the whole point of focusing it early.
    for ch in "status".chars() {
        h.input_mut().events.push(egui::Event::Text(ch.to_string()));
        h.run_steps(1);
    }
    assert_eq!(h.state().cmd.input(), "status", "typing did not land in the box");
    for pressed in [true, false] {
        h.input_mut().events.push(egui::Event::Key {
            key: egui::Key::Enter,
            physical_key: None,
            pressed,
            repeat: false,
            modifiers: Default::default(),
        });
    }
    h.run_steps(2);
    let history: Vec<String> = h.state().cmd.history().iter().map(|e| e.text.clone()).collect();
    assert!(
        history.iter().any(|line| line == "status"),
        "typing \"status\" with nothing clicked first was not submitted: {history:?}; \
         box now reads {:?}; box has focus: {}",
        h.state().cmd.input(),
        h.ctx.memory(|m| m.has_focus(egui::Id::new(COMMAND_INPUT))),
    );
}

/// The same screen with a font configured — set directly on the in-memory
/// state rather than through `outlinedfont`, so this never touches the
/// real, shared settings file `pointer_tests` already exercises.
///
/// A real, existing file, deliberately: `present()` — see
/// `outlined_fonts.rs` — filters to fonts still on disk, so a made-up path
/// would silently render the *empty* state and prove nothing.
#[test]
fn a_configured_font_shows_its_name_and_a_way_to_remove_it() {
    use egui_kittest::kittest::Queryable;

    let arial = "/System/Library/Fonts/Supplemental/Arial.ttf";
    if !std::path::Path::new(arial).is_file() {
        eprintln!("skipping: {arial} not present on this machine");
        return;
    }

    let app = PagifyApp::new(None);
    let mut h = Harness::builder()
        .with_size(egui::vec2(1400.0, 1000.0))
        .build_ui_state(
            |ui, app: &mut PagifyApp| {
                let mut frame = eframe::Frame::_new_kittest();
                app.ui(ui, &mut frame);
            },
            app,
        );
    h.state_mut().outlined_fonts.paths = vec![PathBuf::from(arial)];
    h.run_steps(4);

    h.get_by_label_contains("Arial.ttf");
    h.get_by_label_contains("Remove");
    assert!(h.query_by_label_contains("No extra fonts added").is_none());
}

/// The Protect tab is where a reader actually discovers this needs doing
/// — mid-`redact` or mid-`lock`, not on the Home screen they may never
/// revisit. Checks the button is really on screen under that tab, not
/// just that `Tab::buttons()` contains the tuple.
///
/// Deliberately does not click it, for the same reason the Home tests do
/// not click "Add font…": it dispatches `outlinedfont`, which opens a
/// real, blocking native dialog.
#[test]
fn the_protect_tab_offers_the_outlined_fonts_tool() {
    use egui_kittest::kittest::Queryable;

    let mut h = harness("single-page.pdf");
    h.state_mut().tab_mut().ribbon = Tab::Protect;
    h.run_steps(4);

    h.get_by_label_contains("Outlined-Text Fonts");
    // And beside the tools it exists to support, not lost among the
    // planned stubs. Exact labels, not `_contains`: "Redact" is also a
    // substring of "Smart Redact", a few rows down on the same tab.
    h.get_by_label("Redact");
    h.get_by_label("Lock Area");
}

/// **Clicking a padlock asks for the passcode.**
///
/// Reported from use: the badge drew but pressing it did nothing. The page
/// covers every badge on it and was registering its own click handler
/// *after* theirs, so egui gave every click to the page — see the ordering
/// note where `draw_lock_badges` is called. Driven through the real window
/// rather than by calling the handler, because the bug was entirely in
/// which widget received the click.
#[test]
fn clicking_a_padlock_asks_for_the_passcode() {
    let app = PagifyApp::new(Some(&fixture("scan-300dpi.pdf")));
    assert!(app.tab().doc.is_some(), "the fixture did not open");

    let mut h = Harness::builder()
        .with_size(egui::vec2(1400.0, 1000.0))
        .build_ui_state(
            |ui, app: &mut PagifyApp| {
                let mut frame = eframe::Frame::_new_kittest();
                app.ui(ui, &mut frame);
            },
            app,
        );
    h.run_steps(4);

    // Lock the image so there is a padlock to press.
    {
        let app = h.state_mut();
        app.tab_mut().ribbon = Tab::Protect;
        let object = app.images_on(0)[0].object;
        app.lock_image(0, object, b"a good passcode").expect("lock");
    }
    h.run_steps(4);

    let (badge, view) = {
        let app = h.state_mut();
        let items = app.locked_items_on(0);
        assert_eq!(items.len(), 1, "nothing was locked, so there is no badge");
        (items[0].rect, app.tab_mut().view_state.last_view.expect("the page was never drawn"))
    };

    // The badge sits at the middle of what it stands for.
    let middle = view.to_screen(AppPoint {
        x: ((badge.left + badge.right) / 2.0) as f64,
        y: ((badge.top + badge.bottom) / 2.0) as f64,
    });
    click(&mut h, middle);

    assert!(
        matches!(h.state().tab().secure_state.awaiting_password, Some(Awaiting::UnlockItem(_))),
        "clicking the padlock did not ask for a passcode"
    );
}

/// **The same passcode locks the text and the pictures alike**, and a
/// second lock is not asked to choose one again.
///
/// The vault holds one key, wrapped under one passcode, so everything
/// locked in a document shares it. The window has to say so: asking a
/// person to invent a fresh passcode for the image on a page they have
/// already locked would be asking for something that cannot exist.
#[test]
fn a_second_lock_uses_the_passcode_the_document_already_has() {
    use egui_kittest::kittest::Queryable;

    let strong = "Correct-Horse-99-Battery";
    let mut app = PagifyApp::new(Some(&fixture("two-column.pdf")));
    app.lock_pages(&[0], strong.as_bytes()).expect("lock the page");

    let mut h = Harness::builder()
        .with_size(egui::vec2(1200.0, 900.0))
        .build_ui_state(
            |ui, app: &mut PagifyApp| {
                let mut frame = eframe::Frame::_new_kittest();
                app.ui(ui, &mut frame);
            },
            app,
        );
    h.run_steps(2);

    // Unlocking asks for what the document has, not for a new choice —
    // said in the window's own words, which nothing else on screen uses.
    h.state_mut().submit("unlock");
    h.run_steps(2);
    h.get_by_label_contains("The passcode this document was locked with");

    // And it takes the passcode straight through — no rule, no second
    // typing, because nothing is being chosen.
    h.state_mut().tab_mut().secure_state.password_typed = String::from(strong).into();
    h.run();
    h.get_by_label_contains("Unlock document").click();
    h.run_steps(3);

    // Asserted on the words coming back rather than on the vault
    // emptying: unlocking puts the pages back and leaves the sealed copy
    // where it is, so `locked_pages` is not the question.
    let back = h
        .state_mut()
        .characters(0)
        .map(|c| c.text())
        .unwrap_or_default();
    assert!(
        back.contains("luminaire"),
        "the passcode that locked it did not bring the page back: {back:?}"
    );
}

/// **Choosing a passcode: a window, a rule, and typing it twice.**
///
/// The rule matters because a locked document carries its own original
/// inside itself — the passcode is the only thing between somebody with
/// the file and everything taken off its pages, and they can guess offline
/// for as long as they like.
#[test]
fn locking_asks_in_a_window_and_holds_out_for_a_strong_passcode() {
    use egui_kittest::kittest::Queryable;

    let app = PagifyApp::new(Some(&fixture("two-column.pdf")));
    let mut h = Harness::builder()
        .with_size(egui::vec2(1200.0, 900.0))
        .build_ui_state(
            |ui, app: &mut PagifyApp| {
                let mut frame = eframe::Frame::_new_kittest();
                app.ui(ui, &mut frame);
            },
            app,
        );
    h.run_steps(2);

    // Arm a whole-page lock, which is the simplest thing that asks.
    h.state_mut().submit("lock all");
    h.run_steps(2);
    h.get_by_label_contains("Choose a passcode");

    let strong = "Correct-Horse-99-Battery";
    assert!(pagify_shell::passphrase::is_strong_enough(strong));

    // **A weak one does nothing.** The button is disabled rather than
    // absent — a disabled widget is still in the accessibility tree — so
    // the property to check is that pressing it locks nothing and leaves
    // the window up.
    h.state_mut().tab_mut().secure_state.password_typed = String::from("short").into();
    h.run();
    h.get_by_label_contains("Lock document").click();
    h.run();
    assert!(
        h.state().tab().doc.as_ref().expect("doc").session.locked_pages().is_empty(),
        "a weak passcode locked something"
    );
    h.get_by_label_contains("Choose a passcode");

    // The rest goes through the method the window's button calls, rather
    // than through the button: a password field reports itself to the
    // accessibility tree as a password, not as text, and there is no
    // reading back what was typed into one. What matters is that the
    // decision is the same one either way.
    h.state_mut().answer_passcode(strong);
    h.run();
    h.get_by_label_contains("Type it again");

    // Mistyped, and nothing is locked.
    h.state_mut().answer_passcode("something else 9A!bcdefgh");
    h.run();
    assert!(
        h.state().tab().doc.as_ref().expect("doc").session.locked_pages().is_empty(),
        "a mistyped confirmation locked the document anyway"
    );

    // Typed the same twice, and it locks.
    h.state_mut().submit("lock all");
    h.state_mut().answer_passcode(strong);
    h.state_mut().answer_passcode(strong);
    h.run();
    assert!(
        !h.state().tab().doc.as_ref().expect("doc").session.locked_pages().is_empty(),
        "the passcode was right twice and nothing locked"
    );
}

/// **The password window is really on screen, and really opens it.**
///
/// The test beside this one exercises the shared path; this one renders the
/// window, types into it, and presses Open — because a dialog nothing
/// drives is a dialog nobody has checked, which is how drawn ink stayed
/// visible on a locked page through a suite that passed.
#[test]
fn a_document_that_wants_a_password_asks_for_one_in_a_window() {
    use egui_kittest::kittest::Queryable;

    let mut app = PagifyApp::new(None);
    app.submit(&format!("open \"{}\"", fixture("encrypted.pdf")));
    assert!(app.tab_mut().doc.is_none(), "it opened without a password");

    let mut h = Harness::builder()
        .with_size(egui::vec2(1200.0, 900.0))
        .build_ui_state(
            |ui, app: &mut PagifyApp| {
                let mut frame = eframe::Frame::_new_kittest();
                app.ui(ui, &mut frame);
            },
            app,
        );
    h.run_steps(3);

    // The window is there, and says what it wants.
    h.get_by_label_contains("This document needs a password");

    // Type into it and press Open, as a person would.
    h.state_mut().tab_mut().secure_state.password_typed = String::from("pagify").into();
    h.run_steps(1);
    // Exact, because the ribbon has an "Open..." of its own.
    h.get_by_label("Open").click();
    h.run_steps(3);

    assert!(h.state().tab().doc.is_some(), "the window did not open the document");
    assert!(
        h.state().tab().secure_state.awaiting_password.is_none(),
        "the window is still asking after it opened"
    );
}

/// **Nothing locked may still be on screen anywhere.**
///
/// Reported from use: a locked page went on showing its contents in the
/// thumbnail strip. Thirteen edit sites cleared the page textures and two
/// cleared the thumbnails, so almost every edit left a stale one — and for
/// a lock that is not a cosmetic lag, it is the hidden content still
/// visible.
///
/// Driven through the real window, because the bug was entirely in which
/// cache an edit remembered to clear.
#[test]
fn locking_a_page_leaves_no_stale_thumbnail_of_it() {
    let app = PagifyApp::new(Some(&fixture("two-column.pdf")));
    assert!(app.tab().doc.is_some(), "the fixture did not open");

    let mut h = Harness::builder()
        .with_size(egui::vec2(1400.0, 1000.0))
        .build_ui_state(
            |ui, app: &mut PagifyApp| {
                let mut frame = eframe::Frame::_new_kittest();
                app.ui(ui, &mut frame);
            },
            app,
        );
    h.run_steps(6);

    // The strip has drawn, so a thumbnail of the unlocked page is cached.
    assert!(
        !h.state().tab().doc.as_ref().expect("doc").caches.thumbs.is_empty(),
        "no thumbnail was cached, so this proves nothing"
    );

    h.state_mut()
        .lock_pages(&[0], b"a good passcode")
        .expect("lock the page");

    assert!(
        h.state().tab().doc.as_ref().expect("doc").caches.thumbs.is_empty(),
        "the thumbnail of the locked page is still cached"
    );
    assert!(
        h.state().tab().doc.as_ref().expect("doc").caches.textures.is_empty(),
        "the rendered page is still cached"
    );
}

/// The two files this was actually failing on.
///
/// Not committed — they are 80 MB and 500 kB of someone's real work — so
/// these skip when the files are not there. They are here because a
/// one-page fixture hid the defect completely: `self.tab_mut().view_state.page` never followed
/// the scroll, so on any document long enough to scroll, the pointer talked
/// to page 1 while the reader was somewhere else entirely.
fn real_file(name: &str) -> Option<String> {
    let path = format!("{}/Downloads/{name}", std::env::var("HOME").ok()?);
    std::path::Path::new(&path).exists().then_some(path)
}

fn open_real(name: &str) -> Option<Harness<'static, PagifyApp>> {
    let path = real_file(name)?;
    let app = PagifyApp::new(Some(&path));
    // Skipped rather than failed when it will not open. These tests read a
    // document from the person's own Downloads folder, which they are
    // entitled to change — and one of them acquired a password, at which
    // point six tests started failing about scrolling and selection. A
    // test that depends on somebody's file has to tolerate that file.
    if app.tab().doc.is_none() {
        eprintln!("skipping: {name} is present but would not open (a password, perhaps)");
        return None;
    }
    let mut h = Harness::builder()
        .with_size(egui::vec2(1400.0, 1000.0))
        .build_ui_state(
            |ui, app: &mut PagifyApp| {
                let mut frame = eframe::Frame::_new_kittest();
                app.ui(ui, &mut frame);
            },
            app,
        );
    h.run_steps(6);
    Some(h)
}

/// Scroll the strip the way a wheel would, then let it settle.
fn wheel(h: &mut Harness<'static, PagifyApp>, by: f32) {
    // A scroll area only takes the wheel while the pointer is over it,
    // which is exactly right and exactly the sort of thing a test that
    // pokes methods would never notice.
    h.input_mut().events.push(egui::Event::PointerMoved(egui::pos2(700.0, 600.0)));
    h.input_mut().events.push(egui::Event::MouseWheel {
        unit: egui::MouseWheelUnit::Point,
        delta: egui::vec2(0.0, -by),
        phase: egui::TouchPhase::Move,
        modifiers: Default::default(),
    });
    h.run_steps(4);
}

#[test]
fn selection_works_after_scrolling_into_a_long_document() {
    let Some(mut h) = open_real("HSI CATALOG 2026.pdf") else {
        eprintln!("skipping: catalogue not in ~/Downloads");
        return;
    };
    assert_eq!(h.state().tab().view_state.page, 0);

    // Far enough in that the current page must have moved with it.
    for _ in 0..6 {
        wheel(&mut h, 900.0);
    }
    let landed = h.state().tab().view_state.page;
    assert!(landed > 0, "scrolling six screens did not change the current page");

    let start = a_character_on_screen(&mut h);
    drag(&mut h, start, start + egui::vec2(160.0, 0.0));

    let app = h.state();
    assert!(
        app.tab().selection.text_selection.is_some(),
        "nothing selected on page {} of the catalogue.\npointer: {:?}  tool armed: {:?}",
        landed + 1,
        app.tab().pointer,
        app.tab().tool.is_some()
    );
}

#[test]
fn selection_works_on_the_test_report() {
    let Some(mut h) = open_real("8_144498923727031132.pdf")
        .or_else(|| {
            let p = format!("{}/Desktop/8_144498923727031132.pdf", std::env::var("HOME").unwrap());
            std::path::Path::new(&p).exists().then(|| {
                let app = PagifyApp::new(Some(&p));
                let mut h = Harness::builder()
                    .with_size(egui::vec2(1400.0, 1000.0))
                    .build_ui_state(
                        |ui, app: &mut PagifyApp| {
                            let mut frame = eframe::Frame::_new_kittest();
                            app.ui(ui, &mut frame);
                        },
                        app,
                    );
                h.run_steps(6);
                h
            })
        })
    else {
        eprintln!("skipping: the report is not in ~/Downloads or ~/Desktop");
        return;
    };

    // Page 2 carries the monospace evidence blocks, which are real text.
    h.state_mut().submit("page 2");
    h.run_steps(4);

    let start = a_character_on_screen(&mut h);
    drag(&mut h, start, start + egui::vec2(120.0, 0.0));

    let app = h.state();
    assert!(
        app.tab().selection.text_selection.is_some(),
        "nothing selected on page {} of the report",
        app.tab().view_state.page + 1
    );
}

/// What actually reached the clipboard this frame.
///
/// Asserted rather than trusting that `copy_text` was called: the whole
/// point of copying is that something lands outside the program, and a
/// test that only checks the call was made would pass on an empty string.
fn copied(h: &Harness<'static, PagifyApp>) -> Option<String> {
    // From the frame's own output, not from the context: egui hands the
    // commands to the integration and clears them, so by the next step
    // there is nothing left to read.
    h.output().platform_output.commands.iter().find_map(|c| match c {
        egui::OutputCommand::CopyText(t) => Some(t.clone()),
        _ => None,
    })
}

/// ⌘C, the way almost everyone will copy.
///
/// Asserted on what the app reports rather than on the clipboard command,
/// and not for want of trying: `step()` drains every queued event in one
/// call, so the frame that carries `CopyText` has already been replaced by
/// the time the test can look. The payload is checked through the command
/// box instead, in `what_is_copied_is_what_was_selected`, where the frame
/// is observable — between them the key, the path and the text are all
/// covered.
#[test]
fn selected_text_can_be_copied_with_the_keyboard() {
    let mut h = harness("text-lines.pdf");
    let start = a_character_on_screen(&mut h);
    drag(&mut h, start, start + egui::vec2(160.0, 0.0));
    assert!(h.state().tab().selection.text_selection.is_some(), "nothing was selected to copy");

    // `Event::Copy` directly, not a raw `Key::C` press: that is what a
    // real ⌘C actually produces (egui-winit intercepts it and never also
    // emits a `Key` press — see `Keys.copy`'s own doc comment in main.rs),
    // so this is what the harness needs to inject for the test to mean
    // what it says.
    h.event(egui::Event::Copy);
    h.run_steps(3);

    let said = h
        .state()
        .cmd
        .history()
        .iter()
        .map(|e| e.text.as_str())
        .collect::<Vec<_>>()
        .join("\n");
    assert!(
        said.contains("characters copied"),
        "\u{2318}C copied nothing.\nselection: {:?}\n{said}",
        h.state().tab().selection.text_selection
    );
}

/// **Reported from use, twice**: a page selected in the plain, always-
/// visible "Pages" rail could not be copied with ⌘C — Organize's own
/// copy/paste/delete used to require `organize_open`, the wider grid's
/// own mode, even though selecting a page is something the rail itself
/// has always been able to do. Nobody should have to switch into a
/// separate tool just to reorganize pages while another tool is armed.
/// This pins the fix at the keyboard-dispatch level, independent of any
/// click coordinates: a page selected with `organize_open` false still
/// copies.
#[test]
fn a_page_selected_in_the_plain_rail_copies_without_opening_organize() {
    let mut h = harness("text-lines.pdf");
    h.state_mut().insert_page();
    assert!(!h.state().organize_open, "the wider Organize grid was never opened");

    h.state_mut().tab_mut().organize.organize_selected = vec![0];
    h.event(egui::Event::Copy);
    h.run_steps(3);

    assert!(
        h.state().page_clipboard.is_some(),
        "⌘C on a page selected outside Organize should have filled the page clipboard"
    );
}

/// **Reported from use**: dragging pages to reorder them, repeatedly,
/// then pressing ⌘C copied nothing — and said nothing either, not even
/// "nothing selected.", which every other empty-copy path does say. The
/// test above proves ⌘C works when `organize_selected` is set directly;
/// this drives the *real* click-and-drag path through real pointer
/// events, the way the report actually happened, to find what a direct
/// assignment can't catch.
#[test]
fn copying_still_works_after_several_real_drag_reorders() {
    use egui_kittest::kittest::Queryable;

    let mut h = harness("text-lines.pdf");
    h.state_mut().insert_page();
    h.run_steps(3);

    let rects: Vec<egui::Rect> =
        h.get_all_by_role(egui::accesskit::Role::Image).map(|n| n.rect()).collect();
    assert!(rects.len() >= 2, "expected at least two page thumbnails, found {}", rects.len());
    let (a, b) = (rects[0].center(), rects[1].center());

    for _ in 0..3 {
        drag(&mut h, a, b);
        drag(&mut h, a, b);
    }
    h.run_steps(2);

    assert!(
        !h.state().tab().organize.organize_selected.is_empty(),
        "a page should still be selected after dragging it"
    );

    h.event(egui::Event::Copy);
    h.run_steps(3);

    let said: Vec<&str> = h.state().cmd.history().iter().map(|e| e.text.as_str()).collect();
    assert!(
        h.state().page_clipboard.is_some(),
        "⌘C after several real drag-reorders should still copy — said: {said:?}"
    );
}

/// Same report, the other half of it: a plain click (not a drag) on a
/// thumbnail, through the real UI, then ⌘C.
#[test]
fn copying_works_after_a_real_plain_click_on_a_thumbnail() {
    use egui_kittest::kittest::Queryable;

    let mut h = harness("text-lines.pdf");
    h.state_mut().insert_page();
    h.run_steps(3);

    let rect = h.get_all_by_role(egui::accesskit::Role::Image).next().expect("a thumbnail").rect();
    click(&mut h, rect.center());
    h.run_steps(2);

    assert!(
        !h.state().tab().organize.organize_selected.is_empty(),
        "a plain click should have selected the page it landed on"
    );

    h.event(egui::Event::Copy);
    h.run_steps(3);

    let said: Vec<&str> = h.state().cmd.history().iter().map(|e| e.text.as_str()).collect();
    assert!(h.state().page_clipboard.is_some(), "⌘C after a plain click should copy — said: {said:?}");
}

/// The typed command, not the keyboard shortcut — §7 says they must
/// agree, and until now only ⌘C knew to try a page selection first.
#[test]
fn the_typed_copy_command_also_copies_a_selected_page() {
    let mut h = harness("text-lines.pdf");
    h.state_mut().insert_page();
    h.state_mut().tab_mut().organize.organize_selected = vec![0];

    h.state_mut().submit("copy");
    h.run_steps(2);

    assert!(
        h.state().page_clipboard.is_some(),
        "typing `copy` with a page selected should copy it, same as ⌘C"
    );
}

/// **Reported from use, with a screenshot**: `copy`'s own success
/// message says "`paste` puts it in." — typing exactly that answered
/// "unknown command 'paste'." There was no such verb at all; `Verb::
/// Paste` is now what the parser resolves it to, same as `Verb::
/// CopyText` for `copy`.
#[test]
fn the_typed_paste_command_exists_and_pastes_a_copied_page() {
    let mut h = harness("text-lines.pdf");
    let before = h.state().tab().doc.as_ref().unwrap().page_count;

    h.state_mut().insert_page();
    h.state_mut().tab_mut().organize.organize_selected = vec![0];
    h.state_mut().submit("copy");
    h.run_steps(1);
    assert!(h.state().page_clipboard.is_some(), "the copy half of this didn't take");

    h.state_mut().submit("paste");
    h.run_steps(1);

    let said: Vec<&str> = h.state().cmd.history().iter().map(|e| e.text.as_str()).collect();
    assert!(
        !said.iter().any(|s| s.contains("unknown command")),
        "`paste` should be a real command — said: {said:?}"
    );
    let after = h.state().tab().doc.as_ref().unwrap().page_count;
    assert_eq!(after, before + 2, "insert_page then paste should both have added a page");
}

/// The other half of the report: ⌘V after a real ⌘C, both through real
/// pointer and key events, not direct state assignment.
#[test]
fn pasting_after_a_real_copy_adds_a_page() {
    use egui_kittest::kittest::Queryable;

    let mut h = harness("text-lines.pdf");
    h.state_mut().insert_page();
    h.run_steps(3);
    let before = h.state().tab().doc.as_ref().unwrap().page_count;

    let rect = h.get_all_by_role(egui::accesskit::Role::Image).next().expect("a thumbnail").rect();
    click(&mut h, rect.center());
    h.run_steps(2);

    h.event(egui::Event::Copy);
    h.run_steps(3);
    assert!(h.state().page_clipboard.is_some(), "the copy half of this didn't take");

    h.event(egui::Event::Paste("placeholder".to_string()));
    h.run_steps(3);

    let after = h.state().tab().doc.as_ref().unwrap().page_count;
    let said: Vec<&str> = h.state().cmd.history().iter().map(|e| e.text.as_str()).collect();
    assert_eq!(after, before + 1, "⌘V should have pasted the copied page — said: {said:?}");
}

/// **Reported from use**: typing `copy`/`paste` into the command box
/// worked, but the raw ⌘C/⌘V shortcut did not. The command box
/// re-focuses itself right after every typed command runs (see
/// `response.request_focus()` right after `cmd.submit(...)` in the
/// titlebar's own drawing code) and stays focused until something else
/// explicitly takes it away — e.g. clicking a thumbnail, which is what
/// every *other* ⌘C test above does first. Nobody reaching for the raw
/// shortcut right after typing a command (or, on a fresh document, right
/// after nothing at all — the box can start focused) clicks away first,
/// so the strict guard that used to gate ⌘C/⌘V — "nothing at all may
/// have focus" — silently routed the shortcut into the "nothing
/// selected" fallback instead of the page clipboard the identical typed
/// command reaches.
#[test]
fn the_shortcut_still_copies_and_pastes_while_the_command_box_has_focus() {
    let mut h = harness("text-lines.pdf");
    h.state_mut().insert_page();
    h.run_steps(1);

    // The exact state the command box is left in after any typed
    // command runs.
    h.ctx.memory_mut(|m| m.request_focus(egui::Id::new(COMMAND_INPUT)));
    h.run_steps(1);
    assert!(
        h.ctx.memory(|m| m.has_focus(egui::Id::new(COMMAND_INPUT))),
        "test setup: the command box should have focus"
    );

    h.state_mut().tab_mut().organize.organize_selected = vec![0];
    h.event(egui::Event::Copy);
    h.run_steps(3);
    assert!(
        h.state().page_clipboard.is_some(),
        "\u{2318}C should still copy the selected page while the command box has focus"
    );

    h.state_mut().tab_mut().organize.organize_selected.clear();
    let before = h.state().tab().doc.as_ref().unwrap().page_count;
    h.event(egui::Event::Paste("placeholder".to_string()));
    h.run_steps(3);
    let after = h.state().tab().doc.as_ref().unwrap().page_count;
    assert_eq!(
        after,
        before + 1,
        "\u{2318}V should still paste while the command box has focus"
    );
}

/// Locks against `theme::TEST_LOCK` — the same global flag
/// `theme::tests` exercises directly, shared across every test in this
/// binary so the two sets of tests can't flip it out from under each
/// other.
#[test]
fn the_appearance_command_toggles_the_theme_and_says_which() {
    let _guard = theme::TEST_LOCK.lock().unwrap();
    theme::set_mode(theme::Mode::Dark);
    let mut h = harness("text-lines.pdf");

    h.state_mut().submit("appearance");
    assert_eq!(theme::mode(), theme::Mode::Light);
    assert!(
        h.state().cmd.history().iter().any(|e| e.text.contains("light")),
        "should have said it switched to the light theme"
    );

    h.state_mut().submit("appearance");
    assert_eq!(theme::mode(), theme::Mode::Dark);

    theme::set_mode(theme::Mode::Dark);
}

/// §7: the box can do anything the interface can. `copy` did not exist, so
/// the one thing a reader most wants to do with a selection could not be
/// scripted or recorded.
#[test]
fn copy_works_from_the_command_box_too() {
    let mut h = harness("text-lines.pdf");
    let start = a_character_on_screen(&mut h);
    drag(&mut h, start, start + egui::vec2(160.0, 0.0));

    h.state_mut().submit("copy");
    h.run_steps(1);

    let text = copied(&h).expect("`copy` put nothing on the clipboard");
    assert!(!text.trim().is_empty(), "an empty string was copied");
}

#[test]
fn copying_with_nothing_selected_says_so_rather_than_copying_nothing() {
    let mut h = harness("text-lines.pdf");
    h.state_mut().submit("copy");
    h.run_steps(2);

    assert!(copied(&h).is_none(), "an empty selection reached the clipboard");
    let said = h
        .state()
        .cmd
        .history()
        .iter()
        .map(|e| e.text.as_str())
        .collect::<Vec<_>>()
        .join("\n");
    assert!(said.contains("nothing selected"), "no explanation:\n{said}");
}

/// The copied text must be the text that was highlighted, not the page.
#[test]
fn what_is_copied_is_what_was_selected() {
    let mut h = harness("text-lines.pdf");
    let start = a_character_on_screen(&mut h);
    drag(&mut h, start, start + egui::vec2(160.0, 0.0));

    let expected = {
        let app = h.state_mut();
        let page = app.tab_mut().view_state.page;
        let range = app.tab_mut().selection.text_selection.clone().expect("no selection");
        app.characters(page).expect("characters").text_of(range)
    };

    h.state_mut().submit("copy");
    h.run_steps(1);

    assert_eq!(copied(&h).as_deref(), Some(expected.as_str()));
}

fn drag_with(
    h: &mut Harness<'static, PagifyApp>,
    button: egui::PointerButton,
    from: egui::Pos2,
    to: egui::Pos2,
) {
    use egui::Event;
    let press = |pos, down| Event::PointerButton {
        pos,
        button,
        pressed: down,
        modifiers: Default::default(),
    };
    h.input_mut().events.push(Event::PointerMoved(from));
    h.run_steps(1);
    h.input_mut().events.push(press(from, true));
    h.run_steps(1);
    for i in 1..=4 {
        let t = i as f32 / 4.0;
        h.input_mut().events.push(Event::PointerMoved(from + (to - from) * t));
        h.run_steps(1);
    }
    h.input_mut().events.push(press(to, false));
    h.run_steps(3);
}

/// Zooming on a real, long document — where the page under the pointer is
/// very often not the current page.
///
/// A one-page fixture cannot show this: `last_view` is recorded for the
/// current page, so anchoring with it put the fixed point on the wrong page
/// entirely and the view slid.
#[test]
fn zooming_holds_the_point_on_a_scrolling_document() {
    let Some(mut h) = open_real("HSI CATALOG 2026.pdf") else {
        eprintln!("skipping: catalogue not in ~/Downloads");
        return;
    };
    h.state_mut().tab_mut().view_state.zoom = ZoomMode::Factor(2.5);
    h.run_steps(3);
    for _ in 0..4 {
        wheel(&mut h, 900.0);
    }

    let at = egui::pos2(700.0, 620.0);
    h.input_mut().events.push(egui::Event::PointerMoved(at));
    h.run_steps(2);
    // Measured in **strip** coordinates. A `PageView` maps to points within
    // its own page, so on a scrolling strip the point under the cursor can
    // belong to a different page before and after — comparing those two
    // directly measures the gap between pages, not the anchor.
    let strip_point = |h: &Harness<'static, PagifyApp>, at: egui::Pos2| {
        let app = h.state();
        let (page, view) = app.tab().view_state.hover_view.expect("the pointer was over no page");
        let top = app.tab().doc.as_ref().expect("open").strip.top_of(page).unwrap_or(0.0) as f64;
        let on_page = view.to_page(at);
        (on_page.x, on_page.y + top)
    };

    let target = strip_point(&h, at);
    pinch_at(&mut h, at, 1.04, 6);

    let app = h.state();
    let now = strip_point(&h, at);
    let drift = ((now.0 - target.0).powi(2) + (now.1 - target.1).powi(2)).sqrt();
    assert!(
        drift < 8.0,
        "the page slid {drift:.1}pt out from under the cursor while zooming \
         ({target:?} -> {now:?}) at scale {:.2}",
        app.resolved_zoom()
    );
}

/// A selection has to survive the wheel.
///
/// The current page follows the scroll now, and the selection used to be
/// dropped whenever the current page changed — so nudging the wheel after
/// highlighting something threw it away, and ⌘C then had nothing to copy.
#[test]
fn a_selection_survives_scrolling() {
    let Some(mut h) = open_real("HSI CATALOG 2026.pdf") else {
        eprintln!("skipping: catalogue not in ~/Downloads");
        return;
    };
    for _ in 0..3 {
        wheel(&mut h, 900.0);
    }

    let start = a_character_on_screen(&mut h);
    drag(&mut h, start, start + egui::vec2(150.0, 0.0));
    let selected = h.state().tab().selection.text_selection.clone();
    assert!(selected.is_some(), "nothing was selected to begin with");
    let on_page = h.state().tab().organize.selection_page;

    // A nudge, not a jump to another page.
    wheel(&mut h, 60.0);

    assert_eq!(h.state().tab().selection.text_selection, selected, "the wheel threw the selection away");
    assert_eq!(h.state().tab().organize.selection_page, on_page, "the selection changed page");

    h.state_mut().submit("copy");
    h.run_steps(1);
    assert!(copied(&h).is_some(), "the surviving selection copied nothing");
}

/// Clicking a thumbnail must actually go there. The programmatic scroll
/// takes a frame or two to arrive, and follow-the-scroll would read the old
/// position in the meantime and put the page straight back.
#[test]
fn a_jump_to_a_page_is_not_undone_by_the_scroll_catching_up() {
    let Some(mut h) = open_real("HSI CATALOG 2026.pdf") else {
        eprintln!("skipping: catalogue not in ~/Downloads");
        return;
    };
    assert_eq!(h.state().tab().view_state.page, 0);

    h.state_mut().act(Verb::Page(PageTarget::Number(12)));
    h.run_steps(6);

    assert_eq!(
        h.state().tab().view_state.page,
        11,
        "the jump was undone; the view slid back to page {}",
        h.state().tab().view_state.page + 1
    );
}

/// **The page must never vanish.**
///
/// The engine refuses a render wider than 16,384 px or larger than 64 M
/// pixels, and the draw path turned a refusal into `None` — so past a
/// certain zoom nothing was drawn at all and the page came back only when
/// you zoomed out again.
#[test]
fn the_page_is_still_drawn_at_extreme_zoom() {
    let mut h = harness("text-lines.pdf");

    for scale in [4.0f32, 8.0, 12.0, 16.0] {
        h.state_mut().tab_mut().view_state.zoom = ZoomMode::Factor(scale);
        h.run_steps(3);
        assert!(
            h.state().tab().view_state.last_view.is_some(),
            "nothing was drawn at {scale}x — the page disappeared"
        );
    }
}

/// **Reported from use**: comparing Pagify to another viewer on the same
/// CAD-exported PDF, the same drawing detail could not be reached at a
/// comparable zoom. Every interactive way to zoom in — the toolbar
/// buttons, `+`/`-`, pinch/Ctrl-scroll — was capped at a logical 16.0
/// (1600%), four times below the 6400% professional PDF/CAD viewers
/// commonly offer, even though the renderer underneath was never the
/// bottleneck (`render_page_region`'s own budget scales with zoom).
#[test]
fn zooming_in_can_go_well_past_the_old_sixteen_hundred_percent_ceiling() {
    let mut h = harness("text-lines.pdf");

    h.state_mut().set_zoom(ZoomTarget::Factor(50.0));
    assert!(
        (h.state().resolved_zoom() - 50.0).abs() < 0.01,
        "a direct 5000% zoom request should not be clamped down to the old 1600% ceiling, got {}%",
        h.state().resolved_zoom() * 100.0
    );

    // The interactive path too, not just a direct factor: pressing Zoom
    // In repeatedly from just under the old ceiling must be able to
    // cross it, the way a reader actually reaches high zoom.
    h.state_mut().set_zoom(ZoomTarget::Factor(15.0));
    for _ in 0..10 {
        h.state_mut().set_zoom(ZoomTarget::In);
    }
    assert!(
        h.state().resolved_zoom() > 16.0,
        "Zoom In repeated from near the old ceiling should be able to cross it, got {}%",
        h.state().resolved_zoom() * 100.0
    );
}

/// And it must still be *usable* there: a drawn page nobody can point at is
/// only half back.
#[test]
fn the_page_can_still_be_pointed_at_when_zoomed_right_in() {
    let mut h = harness("text-lines.pdf");
    h.state_mut().tab_mut().view_state.zoom = ZoomMode::Factor(14.0);
    h.run_steps(3);

    let view = h.state().tab().view_state.last_view.expect("nothing drawn");
    let at = egui::pos2(700.0, 600.0);
    let on_page = view.to_page(at);
    assert!(
        on_page.x.is_finite() && on_page.y.is_finite(),
        "the page mapping went bad at 14x: {on_page:?}"
    );
}

/// A page smaller than the window belongs in the middle of it, not against
/// the left edge with all the empty space on one side.
#[test]
fn a_page_smaller_than_the_window_is_centred() {
    let mut h = harness("single-page.pdf");
    h.state_mut().tab_mut().view_state.zoom = ZoomMode::Factor(0.5);
    h.run_steps(3);

    let (view, page_w) = {
        let app = h.state();
        let view = app.tab().view_state.last_view.expect("nothing drawn");
        let (w, _) = app.tab().doc.as_ref().unwrap().strip.size_of(app.tab().view_state.page).unwrap();
        (view, w)
    };

    let viewport = h.state().tab().view_state.viewport_rect.expect("no viewport");
    let left = view.origin.x;
    let right = left + page_w * view.scale;
    let before = left - viewport.left();
    let after = viewport.right() - right;

    assert!(
        (before - after).abs() < 4.0,
        "the page is not centred: {before:.1}pt of space on the left, {after:.1}pt on \
         the right"
    );
}

/// Once the page is larger than the window there is nothing to centre, and
/// forcing it would fight the scroll.
#[test]
fn a_page_larger_than_the_window_is_not_centred() {
    let mut h = harness("single-page.pdf");
    h.state_mut().tab_mut().view_state.zoom = ZoomMode::Factor(8.0);
    h.run_steps(3);

    let viewport = h.state().tab().view_state.viewport_rect.expect("no viewport");
    let view = h.state().tab().view_state.last_view.expect("nothing drawn");
    assert!(
        view.origin.x <= viewport.left() + 14.0,
        "an oversized page was padded away from the edge ({} vs {})",
        view.origin.x,
        viewport.left()
    );
}

/// Facing pages have to be drawn side by side, not merely laid out that
/// way — the placement is the strip's, and the draw has to ask it.
#[test]
fn facing_draws_two_pages_side_by_side() {
    let mut h = harness("pages-ladder.pdf");
    h.state_mut().submit("viewfacing");
    h.run_steps(4);

    let (a, b) = {
        let app = h.state();
        let strip = &app.tab().doc.as_ref().unwrap().strip;
        (strip.left_of(0).unwrap(), strip.left_of(1).unwrap())
    };
    assert!(b > a, "the second page is not to the right of the first");

    let same_row = {
        let strip = &h.state().tab().doc.as_ref().unwrap().strip;
        strip.top_of(0) == strip.top_of(1)
    };
    assert!(same_row, "the two pages are not on the same row");
}

/// Adding or removing a page must not quietly put a spread back to one-up.
#[test]
fn a_page_operation_keeps_the_chosen_layout() {
    use pagify_shell::reader::Layout;
    let mut h = harness("pages-ladder.pdf");
    h.state_mut().submit("viewfacing");
    h.run_steps(3);

    h.state_mut().submit("reversepages");
    h.run_steps(3);

    assert_eq!(
        h.state().tab().doc.as_ref().unwrap().strip.layout(),
        Layout::Facing,
        "reversing the document reset the layout"
    );
}

/// Whatever the layout, the page under the pointer is the page you
/// interact with — which on a spread means the one you are actually over,
/// not whichever shares its row.
#[test]
fn the_right_page_of_a_spread_is_its_own_page() {
    let mut h = harness("pages-ladder.pdf");
    h.state_mut().submit("viewfacing");
    h.run_steps(4);

    let strip = h.state().tab().doc.as_ref().unwrap().strip.clone();
    assert_eq!(strip.top_of(0), strip.top_of(1));
    assert_ne!(
        strip.left_of(0),
        strip.left_of(1),
        "both pages of the spread are in the same place"
    );
}

/// A button that is not built yet must **say so where the user is looking**.
///
/// The reply went into the history panel, which is folded away by default —
/// so every unbuilt button looked broken rather than unbuilt, and a tab of
/// them read as a tab that does nothing.
#[test]
fn an_unbuilt_button_says_so_without_opening_the_history() {
    let mut h = harness("text-lines.pdf");
    h.state_mut().command_open = false;
    h.state_mut().submit("add3d");
    h.run_steps(2);

    let last = h
        .state()
        .cmd
        .history()
        .last()
        .map(|e| e.text.clone())
        .unwrap_or_default();
    assert!(
        last.contains("editing phase"),
        "the reply did not explain itself: {last:?}"
    );

    // And it has to be on screen, not merely in the log.
    let shown = h.state().command_open;
    assert!(!shown, "the test is not exercising the collapsed bar");
    assert!(
        h.state().tab().tool.is_none(),
        "a planned verb armed something, so the bar would show that instead"
    );
}

/// Editing the words that are already on a page: arm Edit Text, click a
/// run, and retyping it changes it.
#[test]
fn clicking_a_run_offers_its_words_and_retyping_replaces_them() {
    let mut h = harness("text-lines.pdf");
    h.state_mut().submit("edittext");
    h.run_steps(1);
    let word = a_character_on_screen(&mut h);
    click(&mut h, word);
    h.run_steps(2);
    assert!(h.state().tab().edit.editing_run.is_some(), "clicking a run did not open it");

    // The run's own words go into the box, ready to be edited — a run is
    // usually a sentence, and retyping one from scratch is not editing.
    // The editor opens **on the page**, holding the run's own words —
    // editing a word is a thing you do to the word.
    let run = h.state().tab().edit.editing_run.as_ref().cloned().expect("no run was picked");
    assert!(!run.buffer.is_empty(), "the editor opened empty");
    assert_eq!(
        run.buffer.trim(),
        run.original.trim(),
        "the editor is not showing the run's words"
    );
    let original = run.original;

    h.state_mut().submit("REPLACED");
    h.run_steps(2);

    let app = h.state_mut();
    app.tab_mut().doc.as_mut().unwrap().caches.text = None;
    let page = app.characters(0).map(|c| c.text()).unwrap_or_default();
    assert!(page.contains("REPLACED"), "the words did not change:\n{page}");
    assert!(!page.contains(original.trim()), "the old words are still there");
}

/// **Typing past the run's own width starts a new line instead of
/// growing the box sideways.** Requested directly: once a replacement
/// no longer fits the words it replaced, whatever comes next should
/// read as the next line, not as an ever-widening box that stops
/// matching anything else on the page.
#[test]
fn typing_past_the_runs_own_width_starts_a_new_line() {
    let mut h = harness("text-lines.pdf");
    h.state_mut().submit("edittext");
    h.run_steps(1);
    let word = a_character_on_screen(&mut h);
    click(&mut h, word);
    h.run_steps(2);
    assert!(h.state().tab().edit.editing_run.is_some(), "clicking a run did not open it");

    // Long enough that no run's own on-screen width could hold it on
    // one line, whatever fixture or font this runs against — the point
    // is proving overflow wraps, not measuring any one run's own width.
    let long_text = "wrap ".repeat(40);
    for ch in long_text.chars() {
        h.input_mut().events.push(egui::Event::Text(ch.to_string()));
        h.run_steps(1);
    }

    let buffer = h.state().tab().edit.editing_run.as_ref().expect("still editing").buffer.clone();
    assert!(
        buffer.contains('\n'),
        "typing past the box's own width did not start a new line:\n{buffer}"
    );
}

/// **Reported from use, on the real CAMINO file: editing a multi-line
/// paragraph, then deleting text from the end, corrupted the page** — a
/// word cut off mid-way with a stray hyphen, a different font, and a
/// wrong-coloured duplicate of a word left behind. Root-caused to the
/// auto-wrap feature `typing_past_the_runs_own_width_starts_a_new_line`
/// proves above: it always measures and wraps *whatever is after the
/// buffer's own last `\n`*, with no idea that a multi-line paragraph's
/// own lines are matched to this paragraph's own objects *by position*
/// (see `apply_paragraph_edit`'s own doc). Deleting text from the end
/// collapses the buffer until an edited, interior line becomes the new
/// last segment; if auto-wrap then fires on it, the inserted `\n` shifts
/// every line after it by one, misrouting each remaining line onto the
/// wrong object. Fixed by confining auto-wrap to a single run
/// (`edit.lines.len() <= 1`), where there is no earlier structural line
/// for a wrap to misroute onto — this proves it from the real paragraph
/// shape that corrupted the page, through the real on-page editor, not
/// a direct call that would skip the drawing code the bug lives in.
#[test]
fn typing_long_then_deleting_in_a_real_paragraph_does_not_corrupt_it() {
    let path = r"C:\Users\hsili\Downloads\CAMINO elitee-plus 3.0.pdf";
    let Some(mut h) = harness_at(path) else {
        eprintln!("skipping: CAMINO not present on this machine");
        return;
    };

    let runs = h.state_mut().tab_mut().doc.as_ref().unwrap().session.text_runs(0).expect("runs");
    let target = runs
        .iter()
        .find(|r| r.text.contains("manufacturers"))
        .cloned()
        .expect("the 'COB' paragraph's own run was not found — has the fixture changed?");
    let view = h.state().tab().view_state.last_view.expect("the page was never drawn");
    let at = view.to_screen(AppPoint {
        x: ((target.rect.left + target.rect.right) / 2.0) as f64,
        y: ((target.rect.top + target.rect.bottom) / 2.0) as f64,
    });

    h.state_mut().submit("edittext");
    h.run_steps(1);
    click(&mut h, at);
    h.run_steps(2);
    let line_count =
        h.state().tab().edit.editing_run.as_ref().map(|e| e.lines.len()).unwrap_or(0);
    assert!(line_count > 1, "clicking the COB paragraph did not open a multi-line paragraph");

    let wanted: std::collections::HashSet<usize> = h
        .state()
        .tab()
        .edit.editing_run
        .as_ref()
        .unwrap()
        .lines
        .iter()
        .flat_map(|(objs, _)| objs.iter().copied())
        .collect();
    let before: std::collections::HashMap<usize, pdf_core::document::TextRun> = h
        .state_mut()
        .tab_mut()
        .doc
        .as_ref()
        .unwrap()
        .session
        .text_runs_some(0, &wanted)
        .expect("runs")
        .into_iter()
        .map(|r| (r.object, r))
        .collect();

    // Type real characters, through the real editor, long enough to
    // overflow several times over — the exact shape auto-wrap used to
    // react to.
    for ch in "extra words to push this line well past its own edge, more than once over"
        .chars()
    {
        h.input_mut().events.push(egui::Event::Text(ch.to_string()));
        h.run_steps(1);
    }
    // Then delete most of it back off again, the same shape reported
    // from use ("all i did was remove some text from the end") —
    // collapsing the buffer until an *edited* line, not a pristine
    // trailing one, becomes its new last segment.
    for _ in 0..70 {
        for pressed in [true, false] {
            h.input_mut().events.push(egui::Event::Key {
                key: egui::Key::Backspace,
                physical_key: None,
                pressed,
                repeat: false,
                modifiers: Default::default(),
            });
        }
        h.run_steps(1);
    }

    // **No extra line landed in the buffer.** `edit.lines.len() - 1` is
    // exactly as many `\n` as this paragraph's own structure already
    // has; any more would be exactly the misrouting auto-wrap used to
    // cause here.
    let buffer = h.state().tab().edit.editing_run.as_ref().expect("still editing").buffer.clone();
    let newline_count = buffer.matches('\n').count();
    assert!(
        newline_count <= line_count - 1,
        "typing and deleting in this paragraph's own editor inserted an extra line \
         (expected at most {}, found {newline_count}) — the exact misrouting reported \
         from use:\n{buffer:?}",
        line_count - 1
    );

    // Click away to apply, then check every one of this paragraph's own
    // objects precisely — not a substring search over the whole page,
    // which could pass or fail for reasons unrelated to whether *this*
    // paragraph's own objects landed correctly.
    //
    // **The PDF page's own bottom-right corner, not a harness-window
    // coordinate — and the view re-read now, not the one from before
    // any typing.** `fn interact`'s click-to-apply only ever sees a
    // click that lands inside the page's own on-screen rect — the
    // harness window itself (1400x1000, see `harness_from`) can be
    // larger than that rect, with the page centred inside it, so a
    // fixed window coordinate is not guaranteed to land on the page at
    // all. And over 200-odd frames of typing and backspacing, the page
    // may have scrolled to keep the caret in view, which would make the
    // transform captured before any of that stale.
    let page_size = h.state_mut().tab_mut().doc.as_ref().unwrap().session.page_size(0).expect("page size");
    let view = h.state().tab().view_state.last_view.expect("the page was never drawn");
    let far_corner = view.to_screen(AppPoint {
        x: (page_size.width_pt - 5.0) as f64,
        y: (page_size.height_pt - 5.0) as f64,
    });
    click(&mut h, far_corner);
    h.run_steps(2);
    assert!(h.state().tab().edit.editing_run.is_none(), "the editor should have applied and closed");

    let after: std::collections::HashMap<usize, pdf_core::document::TextRun> = h
        .state_mut()
        .tab_mut()
        .doc
        .as_ref()
        .unwrap()
        .session
        .text_runs_some(0, &wanted)
        .expect("runs should still be readable")
        .into_iter()
        .map(|r| (r.object, r))
        .collect();
    assert_eq!(
        after.len(),
        before.len(),
        "some objects in the edited paragraph are no longer readable as text runs at all"
    );
    // Every object this edit did not touch (every line beyond the one
    // actually typed into) must still report its own original font
    // size — a stray `\n` shifting lines would have sent some other
    // line's words (sized for a different run) onto it instead.
    for (object, was) in &before {
        let now = after.get(object).expect("checked above: present");
        assert!(
            (now.size - was.size).abs() < 0.5 || now.text.trim() != was.text.trim(),
            "object {object}'s own size changed from {} to {} without its text changing — \
             a sign another line's styling landed on it instead of its own",
            was.size,
            now.size
        );
    }
}

/// **Clicking away from the editor applies it, rather than losing it or
/// doing nothing.** Reported from use, of the previous ("nothing
/// happens") behaviour: "once i select a text to edit it and if i click
/// somewhere else the cursor is gone and i cant bring it back. once i
/// edit then i cant apply the change." The fix is not "let the editor
/// keep the caret" — it is that a click elsewhere no longer needs to:
/// it commits what was typed immediately, the same as clicking out of
/// any ordinary text field.
#[test]
fn clicking_away_from_the_editor_applies_it() {
    let mut h = harness("text-lines.pdf");
    h.state_mut().submit("edittext");
    h.run_steps(1);

    let word = a_character_on_screen(&mut h);
    click(&mut h, word);
    h.run_steps(2);

    let run = h.state().tab().edit.editing_run.clone().expect("no run was picked");
    h.state_mut().tab_mut().edit.editing_run.as_mut().expect("no run was picked").buffer =
        "REPLACED".to_string();

    // Somewhere else on the page entirely, well clear of the editor's
    // own box.
    let view = h.state().tab().view_state.last_view.expect("the page was never drawn");
    let box_right = view.to_screen(AppPoint {
        x: run.rect.left.max(run.rect.right) as f64,
        y: run.rect.top.max(run.rect.bottom) as f64,
    });
    click(&mut h, egui::pos2(box_right.x + 240.0, box_right.y + 180.0));
    h.run_steps(2);

    assert!(
        h.state().tab().edit.editing_run.is_none(),
        "the editor should have closed once its change was applied"
    );
    let app = h.state_mut();
    app.tab_mut().doc.as_mut().unwrap().caches.text = None;
    let page = app.characters(0).map(|c| c.text()).unwrap_or_default();
    assert!(page.contains("REPLACED"), "clicking away did not apply the change:\n{page}");
}

/// **Clicking on other text while an editor is open applies it and opens
/// the text clicked, in one gesture — from a fresh reading of the page.**
/// The apply moves the document under the click that caused it, so the
/// paragraph the second click means has to be found on the page as the
/// apply left it, not on the reading made for the first click: the log
/// says the page was read again, the new editor holds the *other*
/// column's lines, and the first edit is on the page.
#[test]
fn clicking_on_other_text_applies_the_open_edit_and_picks_from_the_edited_page() {
    let mut h = harness("two-column.pdf");
    let dir = std::env::temp_dir().join(format!("pagify-test-chain-click-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    h.state_mut().session_log = pagify_shell::session_log::SessionLog::start_in(dir.clone());
    let log_path = h.state().session_log.path().expect("the scratch folder is writable").to_path_buf();

    let runs = h.state_mut().tab_mut().doc.as_ref().unwrap().session.text_runs(0).expect("runs");
    let left: std::collections::BTreeSet<usize> =
        runs.iter().filter(|r| r.rect.left < 300.0).map(|r| r.object).collect();
    let right: std::collections::BTreeSet<usize> =
        runs.iter().filter(|r| r.rect.left >= 300.0).map(|r| r.object).collect();
    assert!(left.len() >= 2 && right.len() >= 2, "setup: the fixture has two columns");
    let inside = |object: usize| {
        let run = runs.iter().find(|r| r.object == object).expect("a run");
        AppPoint { x: ((run.rect.left + run.rect.right) / 2.0) as f64, y: ((run.rect.top + run.rect.bottom) / 2.0) as f64 }
    };
    let screen = |h: &mut Harness<'static, PagifyApp>, at: AppPoint| {
        h.state().tab().view_state.last_view.expect("the page was never drawn").to_screen(at)
    };

    // The left column opens...
    h.state_mut().submit("edittext");
    h.run_steps(1);
    let at = screen(&mut h, inside(*left.iter().nth(left.len() / 2).unwrap()));
    click(&mut h, at);
    h.run_steps(2);
    let opened: std::collections::BTreeSet<usize> = h
        .state()
        .tab()
        .edit.editing_run
        .as_ref()
        .expect("the first click opened nothing")
        .lines
        .iter()
        .flat_map(|(objects, _)| objects.iter().copied())
        .collect();
    assert_eq!(opened, left, "setup: the first click should open the left column");

    // ...is retyped...
    let retyped = h.state().tab().edit.editing_run.as_ref().unwrap().buffer.replacen('e', "E", 1);
    h.state_mut().tab_mut().edit.editing_run.as_mut().unwrap().buffer = retyped.clone();

    // ...and a click on the right column applies that and opens this.
    let at = screen(&mut h, inside(*right.iter().next().unwrap()));
    click(&mut h, at);
    h.run_steps(2);

    let now: std::collections::BTreeSet<usize> = h
        .state()
        .tab()
        .edit.editing_run
        .as_ref()
        .expect("the click on other text opened nothing")
        .lines
        .iter()
        .flat_map(|(objects, _)| objects.iter().copied())
        .collect();
    assert_eq!(now, right, "the second editor should hold the right column's lines, and only those");
    let page = h.state_mut().characters(0).map(|c| c.text()).unwrap_or_default();
    assert!(
        retyped.lines().next().map_or(true, |first| page.contains(first.trim())),
        "the first edit did not land on the page:\n{page}"
    );

    let picks: Vec<String> = std::fs::read_to_string(&log_path)
        .expect("the log file exists")
        .lines()
        .map(|l| serde_json::from_str::<serde_json::Value>(l).expect("valid json"))
        .filter(|l| l["kind"] == "pick")
        .map(|l| l["text"].as_str().unwrap_or_default().to_string())
        .collect();
    assert_eq!(picks.len(), 2, "one line per click: {picks:?}");
    // **Not `cache=miss` any more, and why that is still not a stale read.** With Edit Text in hand,
    // moving the pointer over the page reads it for the paragraph boxes — the same read, kept under
    // the same key, that a click uses — so a click finds it already made. After the apply moved the
    // page the key no longer matches, so what the second click finds was read from the edited page:
    // which the assertions above (the right column's own lines, the first edit on the page) check.
    assert!(picks[0].contains("path=block"), "{}", picks[0]);
    assert!(picks[1].contains("path=block"), "{}", picks[1]);
    let _ = std::fs::remove_dir_all(&dir);
}

/// **The editor draws over a paragraph that has lines drawn as shapes in
/// it.** The datasheet's 13-line paragraph has two lines with a drawn word
/// in them, and the block beside it has three lines that are nothing but
/// shapes, each standing in the box as a placeholder: laying out the buffer
/// (justified, line by line, in the document's own face) over them for a
/// few frames must not panic, and the editor must still be there, whole.
#[test]
fn the_editor_draws_over_a_paragraph_with_lines_drawn_as_shapes() {
    let path = r"C:\Users\hsili\Desktop\Datasheets - Editors market - Marina mall.pdf";
    let _turn = tests_support::one_at_a_time();
    let Some(mut h) = harness_at(path) else {
        eprintln!("skipping: the Marina datasheet is not on this machine");
        return;
    };
    let runs = h.state_mut().tab_mut().doc.as_ref().unwrap().session.text_runs(0).expect("runs");
    let centre = |object: usize| {
        let run = runs.iter().find(|r| r.object == object).expect("a run");
        AppPoint { x: ((run.rect.left + run.rect.right) / 2.0) as f64, y: ((run.rect.top + run.rect.bottom) / 2.0) as f64 }
    };
    // (a word of the paragraph, its lines, how many are frozen)
    let (page_blocks, _) = h.state_mut().page_blocks(0).expect("the page's blocks");
    // **Selected by which lines are drawn, not by which have no text object**
    // (changed with the reason): a hyphen glyph is a member of its line now, so
    // one of the three drawn lines has a text object of its own.
    let long = page_blocks
        .blocks
        .iter()
        .find(|b| b.lines.len() == 19 && b.lines.iter().filter(|l| !l.outlined.is_empty()).count() == 3)
        .expect("the 19-line block");
    for (word, lines, frozen) in [(986usize, 13usize, 2usize), (long.lines[0].objects[0], 19, 3)] {
        h.state_mut().tab_mut().edit.editing_run = None;
        h.state_mut().pick_text_run(0, centre(word)).expect("picked");
        h.run_steps(4);
        let edit = h.state().tab().edit.editing_run.as_ref().expect("the editor is still open after drawing");
        assert_eq!(edit.lines.len(), lines);
        assert_eq!(edit.frozen.iter().filter(|f| **f).count(), frozen);
        assert_eq!(edit.buffer.split('\n').count(), lines, "drawing changed the buffer");
    }
}

/// **The whole-page-editing revert, checked against the actual dense
/// document that motivated it.** Arming Edit Text must be instant again
/// — no more reading every paragraph's font and rendering the page up
/// front before anything is even clicked — a click must open exactly
/// one paragraph (structurally guaranteed now that `editing_run` is an
/// `Option`, not a `Vec`, but worth seeing hold on a real, busy page),
/// and clicking elsewhere on the page must apply the typed change with
/// no separate Apply press.
#[test]
fn arming_and_click_to_apply_work_on_the_real_camino_page() {
    let path = r"C:\Users\hsili\Downloads\CAMINO elitee-plus 3.0.pdf";
    let Some(mut h) = harness_at(path) else {
        eprintln!("skipping: CAMINO not present on this machine");
        return;
    };

    let started = std::time::Instant::now();
    h.state_mut().submit("edittext");
    let armed = started.elapsed();
    eprintln!("timing: arming Edit Text on the CAMINO page took {armed:?}");
    assert!(
        armed.as_millis() < 50,
        "arming Edit Text took {armed:?} on a dense page — should be instant again"
    );
    h.run_steps(1);

    let word = a_character_on_screen(&mut h);
    click(&mut h, word);
    h.run_steps(2);

    let run = h
        .state()
        .tab()
        .edit.editing_run
        .clone()
        .expect("clicking a word did not open an editor on the real page");
    h.state_mut().tab_mut().edit.editing_run.as_mut().unwrap().buffer = "REPLACED TEXT".into();

    // Somewhere else on the page entirely, well clear of the editor's
    // own box — same technique as `clicking_away_from_the_editor_applies_it`.
    let view = h.state().tab().view_state.last_view.expect("the page was never drawn");
    let box_right = view.to_screen(AppPoint {
        x: run.rect.left.max(run.rect.right) as f64,
        y: run.rect.top.max(run.rect.bottom) as f64,
    });
    click(&mut h, egui::pos2(box_right.x + 240.0, box_right.y + 180.0));
    h.run_steps(2);

    // **Not `editing_run.is_none()`.** On a page this dense, 240px away
    // is not guaranteed blank paper — clicking away now also opens
    // whatever text is actually there, applying the old edit and
    // chaining straight into editing the new spot in the same click
    // (requested from use: picking a field, then the next, used to take
    // a fresh `edittext` for every single one). Whether a new editor
    // opened here depends on this real page's own dense layout; that the
    // old edit actually landed does not.
    let app = h.state_mut();
    app.tab_mut().doc.as_mut().unwrap().caches.text = None;
    let page = app.characters(0).map(|c| c.text()).unwrap_or_default();
    assert!(
        page.contains("REPLACED TEXT"),
        "click-away did not apply the change on the real page:\n{page}"
    );
}

/// **Opening the font picker takes the caret away from the run's own
/// text box.** Reported from use: typing to filter the font list moved
/// the page — because the filter field never asked for focus, so the
/// keystrokes kept landing in the run editor behind it, rewriting the
/// words being edited instead of searching anything.
#[test]
fn opening_the_font_picker_takes_focus_off_the_run_editor() {
    let mut h = harness("text-lines.pdf");
    // `pick_text_run` directly, not `edittext` plus a click: same effect,
    // fewer frames to settle before the rest of this test needs the
    // editor already open.
    let at = {
        let app = h.state_mut();
        let chars = app.characters(0).expect("no characters").clone();
        let r = chars.line_rects(0..1).into_iter().next().expect("no character box");
        AppPoint { x: ((r.left + r.right) / 2.0) as f64, y: ((r.top + r.bottom) / 2.0) as f64 }
    };
    h.state_mut().pick_text_run(0, at).expect("a run was here");
    h.run_steps(2);

    let run = h.state().tab().edit.editing_run.as_ref().cloned().expect("no run was picked");
    let editor_id = egui::Id::new(("run-editor", run.page, run.object));

    // Confirms the setup as much as it does anything: the editor
    // already auto-focused on open, and clicking back into it here
    // should simply leave it that way, before the rest of this test
    // proves the font picker takes the caret back off it.
    let view = h.state().tab().view_state.last_view.expect("the page was drawn");
    let middle = view.to_screen(AppPoint {
        x: ((run.rect.left + run.rect.right) / 2.0) as f64,
        y: ((run.rect.top + run.rect.bottom) / 2.0) as f64,
    });
    click(&mut h, middle);
    h.run_steps(2);
    assert!(h.ctx.memory(|m| m.has_focus(editor_id)), "setup: the editor should hold the caret");

    h.state_mut().faces_state.font_picker_open = true;
    h.run_steps(2);

    assert!(
        !h.ctx.memory(|m| m.has_focus(editor_id)),
        "the run editor kept the caret once the font picker opened over it"
    );
}

/// **Typing in the editor and pressing Enter applies the change.**
///
/// **Reported from use: a change went live the moment Enter was pressed,
/// before the Apply button anybody could see was ever touched.** A
/// paragraph's own multiline box already had to treat Enter as "add a
/// line" rather than "submit"; a single line used to be the odd one out.
/// Now neither submits on Enter — only Apply does, for both, the same
/// way `the_run_editor_apply_button_can_be_pressed` proves it for this
/// exact scenario. Enter still does something, though — it adds the line
/// `a_single_run_can_grow_a_second_line_in_its_own_style` proves gets
/// written out correctly once Apply *is* pressed.
#[test]
fn pressing_enter_in_the_run_editor_does_not_apply_it() {
    let mut h = harness("text-lines.pdf");
    h.state_mut().submit("edittext");
    h.run_steps(1);
    let word = a_character_on_screen(&mut h);
    click(&mut h, word);
    h.run_steps(2);

    let original = h.state().tab().edit.editing_run.as_ref().cloned().expect("no run").original;

    // Clear what is there and type over it, through the field itself.
    h.input_mut().events.push(egui::Event::Key {
        key: egui::Key::A,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers: egui::Modifiers::COMMAND,
    });
    h.run_steps(1);
    h.input_mut().events.push(egui::Event::Text("TYPED".into()));
    h.run_steps(1);
    for pressed in [true, false] {
        h.input_mut().events.push(egui::Event::Key {
            key: egui::Key::Enter,
            physical_key: None,
            pressed,
            repeat: false,
            modifiers: Default::default(),
        });
    }
    h.run_steps(2);

    assert!(
        h.state().tab().edit.editing_run.is_some(),
        "Enter closed the editor — it should take a click on Apply, not a keystroke"
    );
    // A newline, not nothing — Enter now adds a line here the same way
    // it already did in a paragraph's own box — but still no submit.
    assert_eq!(
        h.state().tab().edit.editing_run.as_ref().unwrap().buffer,
        "TYPED\n",
        "Enter should have added a line, and nothing more, still unapplied"
    );
    let app = h.state_mut();
    app.tab_mut().doc.as_mut().unwrap().caches.text = None;
    let page = app.characters(0).map(|c| c.text()).unwrap_or_default();
    assert!(
        !page.contains("TYPED"),
        "Enter changed the page before Apply was ever pressed:\n{page}\nwas: {original}"
    );
}

/// And the Apply button beside it can be pressed, for the same reason.
#[test]
fn the_run_editor_apply_button_can_be_pressed() {
    use egui_kittest::kittest::Queryable;

    let mut h = harness("text-lines.pdf");
    h.state_mut().submit("edittext");
    h.run_steps(1);
    let word = a_character_on_screen(&mut h);
    click(&mut h, word);
    h.run_steps(2);

    let run = h.state().tab().edit.editing_run.as_ref().cloned().expect("no run was picked");
    h.state_mut().tab_mut().edit.editing_run.as_mut().expect("editing").buffer = "REPLACED".into();
    h.run_steps(1);

    let apply = h.get_by_label("Apply");
    apply.click();
    h.run_steps(2);

    assert!(
        h.state().tab().edit.editing_run.is_none(),
        "Apply did not finish the edit"
    );
    let app = h.state_mut();
    app.tab_mut().doc.as_mut().unwrap().caches.text = None;
    let page = app.characters(0).map(|c| c.text()).unwrap_or_default();
    assert!(
        page.contains("REPLACED"),
        "Apply changed nothing on the page:\n{page}\nwas: {}",
        run.original
    );
}

/// **A tool part-way through shows what it is making.**
///
/// Reported from use: "in draw i need to click the start and end points for
/// a drawn object to show up — when i make the 1st click it should start
/// drawing." Until the last click there was nothing on screen to say where
/// the first point had landed or what shape was coming.
#[test]
fn a_half_placed_tool_previews_what_it_would_make() {
    let mut h = harness("pages-ladder.pdf");
    h.state_mut().submit("line");
    h.run_steps(2);
    assert!(h.state().tab().tool.is_some(), "the tool was not armed");

    // Nothing to preview before the first click.
    let view = h.state().tab().view_state.last_view.expect("the page was drawn");
    let start = view.to_screen(AppPoint { x: 100.0, y: 100.0 });
    click(&mut h, start);
    h.run_steps(2);

    let armed = h.state().tab().tool.as_ref().expect("still collecting");
    assert_eq!(armed.points.len(), 1, "the first click did not land");

    // With one point placed and the pointer somewhere else, the preview has
    // something to draw. Drawing is painting, so what is checked is that it
    // runs over a real pointer position without panicking and that the tool
    // is still waiting for its second click.
    h.input_mut()
        .events
        .push(egui::Event::PointerMoved(egui::pos2(start.x + 40.0, start.y + 25.0)));
    h.run_steps(2);
    assert!(
        h.state().tab().tool.as_ref().is_some_and(|t| t.points.len() == 1),
        "moving the pointer finished the line by itself"
    );

    // And the second click completes it — after which the tool arms itself
    // again for the next one, which is what makes drawing several bearable.
    click(&mut h, egui::pos2(start.x + 40.0, start.y + 25.0));
    h.run_steps(2);
    let said = h
        .state()
        .cmd
        .history()
        .iter()
        .map(|e| e.text.as_str())
        .collect::<Vec<_>>()
        .join("\n");
    assert!(said.contains("line added"), "the second click did not finish it:\n{said}");
    assert!(
        h.state().tab().tool.as_ref().is_some_and(|t| t.points.is_empty()),
        "it did not come back ready for the next line"
    );
}

/// **A tool armed on one page works on the page you click.**
///
/// A part-way pick belongs to its own page — one end of a line on page 40
/// and the other on page 41 is coordinates that mean nothing on either. But
/// the *first* click of a one-click tool was bound the same way, so arming
/// while one page was current and clicking another visible one did nothing
/// at all: no mark, no message, nothing to read.
///
/// **Bare `fillsign`, not `edittext`** — an arbitrary stand-in for "a
/// one-click tool armed on one page": both pick a single run by its own
/// click, through the same `Tool::PickText` this test means to exercise.
#[test]
fn a_tool_that_has_collected_nothing_answers_a_click_on_another_page() {
    let mut h = harness("pages-ladder.pdf");
    // Small enough that several pages share the window.
    h.state_mut().submit("zoom 25");
    h.run_steps(6);

    h.state_mut().submit("fillsign");
    h.run_steps(2);
    let armed_for = h.state().tab().tool.as_ref().map(|t| t.page).expect("armed");

    // Down the strip, past the page the tool was armed on.
    // Where the next page actually sits, worked out from the page the
    // view is anchored on and the strip's own layout.
    let target = {
        let app = h.state();
        let doc = app.tab().doc.as_ref().expect("open");
        let view = app.tab().view_state.last_view.expect("the page was drawn");
        let here = doc.strip.top_of(armed_for).expect("a top");
        let next = armed_for + 1;
        let (Some(top), Some((w, height))) =
            (doc.strip.top_of(next), doc.strip.size_of(next))
        else {
            return;
        };
        let left = doc.strip.left_of(next).unwrap_or(0.0)
            - doc.strip.left_of(armed_for).unwrap_or(0.0);
        egui::pos2(
            view.origin.x + (left + w / 2.0) * view.scale,
            view.origin.y + (top - here + height / 2.0) * view.scale,
        )
    };

    let before = h.state().cmd.history().len();
    click(&mut h, target);
    h.run_steps(3);
    let answered = h.state().tab().tool.as_ref().is_some_and(|t| t.page != armed_for)
        || h.state().cmd.history().len() > before
        || h.state().tab().edit.editing_run.is_some();

    assert!(
        answered,
        "clicking a page the tool was not armed on did nothing at all — \
         no pick, no message, nothing to read"
    );
}

/// **The pointer must not be pulled about when no tool is placing points.**
///
/// Snapping, ortho and the grid exist to put a line exactly on the end of
/// another line. Applied to ordinary clicking they drag the pointer away
/// from whatever the user was aiming at — a word, a run to edit, somebody
/// else's highlight — and the page feels like it is fighting them.
#[test]
fn clicking_is_not_pulled_to_nearby_geometry() {
    let mut h = harness("text-lines.pdf");

    // A grid coarse enough that any snapping would be unmistakable, and a
    // mark on the page so the snap engine has something to pull towards.
    h.state_mut().grid_pt = 72.0;
    h.state_mut().submit("l 20,20 300,300");
    h.run_steps(2);

    let start = a_character_on_screen(&mut h);
    drag(&mut h, start, start + egui::vec2(150.0, 0.0));

    assert!(
        h.state().tab().selection.text_selection.is_some(),
        "the drag was snapped off the text and selected nothing"
    );
    assert!(h.state().tab().selection.last_snap.is_none(), "a snap was applied with no tool in hand");
}

/// And it must still snap when a tool *is* placing points — that is what it
/// is for.
/// And it must still snap when a tool *is* placing points — that is what it
/// is for.
#[test]
fn a_drawing_tool_still_snaps_to_what_is_there() {
    let mut h = harness("single-page.pdf");

    // Drawn with clicks, so the line and the later hover are in the same
    // space. A typed `l 40,40 200,40` is kernel space, y up from the
    // bottom — the same numbers and a different place.
    // Points inside the page, worked out from where it was actually drawn
    // — a guessed screen position lands off a 200pt-wide sheet.
    let (a, b) = {
        let view = h.state().tab().view_state.last_view.expect("page never drawn");
        (
            view.to_screen(AppPoint { x: 40.0, y: 60.0 }),
            view.to_screen(AppPoint { x: 150.0, y: 60.0 }),
        )
    };
    h.state_mut().submit("line");
    h.run_steps(1);
    click(&mut h, a);
    click(&mut h, b);
    h.run_steps(2);
    assert_eq!(
        h.state().tab().markup.existing(0).map(|l| l.len()).unwrap_or(0),
        1,
        "the line was not drawn, so there is nothing to snap to"
    );

    h.state_mut().submit("line");
    h.run_steps(1);
    // Near the end of that line, but not on it.
    h.input_mut().events.push(egui::Event::PointerMoved(b + egui::vec2(3.0, 3.0)));
    h.run_steps(2);

    assert!(
        h.state().tab().selection.last_snap.is_some(),
        "the line tool did not snap to the end of the line beside it"
    );
}

/// **The white-text bug, from the outside.**
///
/// Editing white text on a dark banner brought it back black — which is to
/// say the words disappeared. An edit changes what it was asked to change
/// and nothing else.
#[test]
fn editing_words_does_not_change_how_they_look() {
    let mut h = harness("text-lines.pdf");
    h.state_mut().submit("edittext");
    h.run_steps(1);
    let at = a_character_on_screen(&mut h);
    click(&mut h, at);
    h.run_steps(2);

    let before = {
        let app = h.state();
        let edit = app.tab().edit.editing_run.as_ref().expect("no run");
        app.tab().doc
            .as_ref()
            .unwrap()
            .session
            .text_runs(0)
            .expect("runs")
            .into_iter()
            .find(|r| r.object == edit.object)
            .expect("the run")
    };

    h.state_mut().submit("REPLACED");
    h.run_steps(2);

    let after = h.state().tab().doc
        .as_ref()
        .unwrap()
        .session
        .text_runs(0)
        .expect("runs")
        .into_iter()
        .find(|r| r.object == before.object)
        .expect("the run");

    assert_eq!(after.text.trim(), "REPLACED", "the words did not change");
    assert_eq!(after.color, before.color, "the colour changed on its own");
    assert!(
        (after.size - before.size).abs() < 0.5,
        "the size changed on its own: {} then {}",
        before.size,
        after.size
    );
}

/// The properties are editable, which is the other half of the ask.
#[test]
fn a_run_can_be_recoloured_from_the_editor() {
    let mut h = harness("text-lines.pdf");
    h.state_mut().submit("edittext");
    h.run_steps(1);
    let at = a_character_on_screen(&mut h);
    click(&mut h, at);
    h.run_steps(2);

    let object = {
        let app = h.state_mut();
        let edit = app.tab_mut().edit.editing_run.as_mut().expect("no run");
        edit.style.color =
            Some(pdf_core::document::Color { r: 250, g: 40, b: 40, a: 255 });
        edit.style.size = Some(22.0);
        edit.object
    };
    // Applying with the words untouched: only the look changes.
    let words = h.state().tab().edit.editing_run.as_ref().unwrap().original.clone();
    h.state_mut().submit(&words);
    h.run_steps(2);

    let run = h.state().tab().doc
        .as_ref()
        .unwrap()
        .session
        .text_runs(0)
        .expect("runs")
        .into_iter()
        .find(|r| r.object == object)
        .expect("the run");
    assert_eq!(run.color.r, 250, "the colour was not applied");
    assert!((run.size - 22.0).abs() < 0.5, "the size was not applied: {}", run.size);
}

/// **"i can't increase the font sizes."** `apply_one_edit` used to
/// send `edit.style` whole, and every field on it is seeded with the
/// run's own *current* colour and position from the moment the editor
/// opens — so a size-only change also asserted an unchanged colour and
/// position right back at their own values, which reads to `pdf_core`
/// as three things changing rather than one and sends it down the path
/// that regenerates the whole page. Proven here by changing only the
/// size and confirming the colour underneath never had a reason to move
/// at all.
#[test]
fn a_runs_size_can_be_changed_alone_without_touching_its_colour() {
    let mut h = harness("text-lines.pdf");
    h.state_mut().submit("edittext");
    h.run_steps(1);
    let at = a_character_on_screen(&mut h);
    click(&mut h, at);
    h.run_steps(2);

    let (object, before_color, new_size) = {
        let app = h.state_mut();
        let edit = app.tab_mut().edit.editing_run.as_mut().expect("no run");
        let before_color = edit.style.color;
        let new_size = edit.style.size.unwrap_or(12.0) + 8.0;
        edit.style.size = Some(new_size);
        (edit.object, before_color, new_size)
    };
    let words = h.state().tab().edit.editing_run.as_ref().unwrap().original.clone();
    h.state_mut().submit(&words);
    h.run_steps(2);

    let run = h.state().tab().doc
        .as_ref()
        .unwrap()
        .session
        .text_runs(0)
        .expect("runs")
        .into_iter()
        .find(|r| r.object == object)
        .expect("the run");
    assert!((run.size - new_size).abs() < 0.5, "the size was not applied: {}", run.size);
    assert_eq!(Some(run.color), before_color, "the colour moved even though it was never touched");
}

#[test]
fn an_edited_run_can_be_undone() {
    let mut h = harness("text-lines.pdf");
    h.state_mut().submit("edittext");
    h.run_steps(1);
    let at = a_character_on_screen(&mut h);
    click(&mut h, at);
    h.run_steps(2);
    let original = h.state().tab().edit.editing_run.as_ref().cloned().expect("no run").original;

    h.state_mut().submit("REPLACED");
    h.run_steps(2);
    h.state_mut().submit("undo");
    h.run_steps(2);

    let app = h.state_mut();
    app.tab_mut().doc.as_mut().unwrap().caches.text = None;
    let page = app.characters(0).map(|c| c.text()).unwrap_or_default();
    assert!(page.contains(original.trim()), "undo did not restore the words:\n{page}");
    assert!(!page.contains("REPLACED"), "the replacement is still there");
}

/// Escape leaves the words alone — the safe answer, and the same key that
/// stops everything else.
#[test]
fn escape_leaves_the_words_as_they_were() {
    let mut h = harness("text-lines.pdf");
    h.state_mut().submit("edittext");
    h.run_steps(1);
    let at = a_character_on_screen(&mut h);
    click(&mut h, at);
    h.run_steps(2);
    let original = h.state().tab().edit.editing_run.as_ref().cloned().expect("no run").original;

    h.state_mut().escape();
    h.run_steps(1);
    assert!(h.state().tab().edit.editing_run.is_none(), "escape did not stop the edit");

    let app = h.state_mut();
    app.tab_mut().doc.as_mut().unwrap().caches.text = None;
    let page = app.characters(0).map(|c| c.text()).unwrap_or_default();
    assert!(page.contains(original.trim()), "escape changed the words anyway");
}

/// Bare `fillsign`, not `edittext` — an arbitrary stand-in for a
/// click-driven pick, same as the sibling test above.
#[test]
fn clicking_bare_paper_says_there_is_no_text_there() {
    let mut h = harness("text-lines.pdf");
    h.state_mut().submit("fillsign");
    h.run_steps(1);

    // Below every run there is, worked out from the runs themselves — a
    // guessed offset lands inside a paragraph as often as not, and then the
    // test passes for the wrong reason.
    let below = {
        let app = h.state();
        let runs = app.tab().doc.as_ref().unwrap().session.text_runs(0).expect("runs");
        let lowest = runs
            .iter()
            .map(|r| r.rect.top.max(r.rect.bottom))
            .fold(0.0f32, f32::max);
        let view = app.tab().view_state.last_view.expect("page never drawn");
        view.to_screen(AppPoint { x: 40.0, y: (lowest + 30.0) as f64 })
    };
    click(&mut h, below);
    h.run_steps(2);

    let said = h
        .state()
        .cmd
        .history()
        .iter()
        .map(|e| e.text.as_str())
        .collect::<Vec<_>>()
        .join("\n");
    assert!(said.contains("no text there"), "no explanation:\n{said}");
    assert!(h.state().tab().edit.editing_run.is_none());
}

/// Words written onto a page are **real text objects**, not a picture of
/// text — they select, search and copy like anything else on the page.
#[test]
fn writing_words_puts_selectable_text_on_the_page() {
    let mut h = harness("text-lines.pdf");
    let before = {
        let app = h.state_mut();
        app.characters(0).map(|c| c.len()).unwrap_or(0)
    };

    h.state_mut().submit("addtext DRAFT");
    h.run_steps(1);
    assert!(h.state().tab().tool.is_some(), "addtext did not ask where");
    // A stale snapshot, from before the words landed — the state a
    // person looking at the page already put it in.
    let _ = h.state_mut().foreign_marks(0);

    let at = a_character_on_screen(&mut h) + egui::vec2(0.0, 90.0);
    click(&mut h, at);
    h.run_steps(2);

    // **Reported from use: a freshly written run drew from a stale page
    // texture, and stayed invisible to hit-testing until the page was
    // left and returned to.** `write_text_at` has to invalidate both
    // caches the moment the words land, the same way placing a picture
    // or a signature already does.
    assert!(
        h.state().tab().doc.as_ref().unwrap().caches.foreign.is_none(),
        "the foreign-marks cache was not invalidated"
    );

    let app = h.state_mut();
    app.tab_mut().doc.as_mut().unwrap().caches.text = None;
    let after = app.characters(0).map(|c| c.text()).unwrap_or_default();
    assert!(
        after.contains("DRAFT"),
        "the words are not in the page's text (was {before} characters):\n{after}"
    );
}

/// It is an edit, so it comes back off.
#[test]
fn written_words_can_be_undone() {
    let mut h = harness("text-lines.pdf");
    h.state_mut().submit("addtext DRAFT");
    h.run_steps(1);
    let at = a_character_on_screen(&mut h) + egui::vec2(0.0, 90.0);
    click(&mut h, at);
    h.run_steps(2);

    h.state_mut().submit("undo");
    h.run_steps(2);
    let app = h.state_mut();
    app.tab_mut().doc.as_mut().unwrap().caches.text = None;
    let after = app.characters(0).map(|c| c.text()).unwrap_or_default();
    assert!(!after.contains("DRAFT"), "undo left the words on the page");
}

/// Bare `addtext` — no words typed — arms the click-and-drag box rather
/// than refusing outright: see `begin_text_box` and the ribbon buttons,
/// which now run exactly this. `addtext <words>` still wants a real
/// string, since `Write` has nowhere else to get one from.
#[test]
fn bare_addtext_arms_the_box_tool_not_an_empty_mark() {
    let mut h = harness("text-lines.pdf");
    h.state_mut().submit("addtext");
    h.run_steps(1);
    assert!(
        matches!(h.state().tab().tool.as_ref().map(|t| &t.kind), Some(Tool::PlaceText)),
        "an empty string should arm the text-box tool, not refuse or place an empty mark"
    );
}

/// A highlighter is a thing you pick up and then use.
///
/// Requiring the selection first means the tool can only ever be applied
/// once per selection, and a button that answers "select some text first"
/// reads as a scolding rather than a tool.
#[test]
fn a_markup_tool_can_be_picked_up_and_then_used() {
    let mut h = harness("text-lines.pdf");
    h.state_mut().submit("highlight");
    h.run_steps(1);
    assert!(h.state().tab().tool.is_some(), "the tool was not picked up");
    assert_eq!(
        h.state().tab().doc.as_ref().unwrap().session.annotations(0).unwrap().len(),
        0,
        "picking the tool up marked something"
    );

    let start = a_character_on_screen(&mut h);
    drag(&mut h, start, start + egui::vec2(150.0, 0.0));

    let marks = h.state().tab().doc.as_ref().unwrap().session.annotations(0).expect("read");
    assert_eq!(marks.len(), 1, "dragging with the tool in hand marked nothing");
}

/// It stays in hand: a highlighter is not put down after one sentence.
#[test]
fn the_tool_stays_in_hand_for_a_second_passage() {
    let mut h = harness("text-lines.pdf");
    h.state_mut().submit("highlight");
    h.run_steps(1);

    // Both drags along the same line: a drag that begins on blank paper
    // box-selects marks instead of text, correctly, and would prove nothing
    // about the tool staying in hand.
    let start = a_character_on_screen(&mut h);
    drag(&mut h, start, start + egui::vec2(150.0, 0.0));
    drag(&mut h, start + egui::vec2(20.0, 0.0), start + egui::vec2(170.0, 0.0));

    let marks = h.state().tab().doc.as_ref().unwrap().session.annotations(0).expect("read");
    assert_eq!(marks.len(), 2, "the second passage was not marked");
}

#[test]
fn escape_puts_the_tool_down() {
    let mut h = harness("text-lines.pdf");
    h.state_mut().submit("underline");
    h.run_steps(1);
    assert!(h.state().tab().tool.is_some());

    h.state_mut().escape();
    h.run_steps(1);
    assert!(h.state().tab().tool.is_none(), "escape did not put it down");

    let start = a_character_on_screen(&mut h);
    drag(&mut h, start, start + egui::vec2(150.0, 0.0));
    let marks = h.state().tab().doc.as_ref().unwrap().session.annotations(0).expect("read");
    assert!(marks.is_empty(), "it marked something after being put down");
}

/// The old way still works — text already selected, then the tool.
#[test]
fn a_tool_pressed_with_text_already_selected_marks_it_at_once() {
    let mut h = harness("text-lines.pdf");
    let start = a_character_on_screen(&mut h);
    drag(&mut h, start, start + egui::vec2(150.0, 0.0));
    assert!(h.state().tab().selection.text_selection.is_some());

    h.state_mut().submit("highlight");
    h.run_steps(1);

    let marks = h.state().tab().doc.as_ref().unwrap().session.annotations(0).expect("read");
    assert_eq!(marks.len(), 1, "an existing selection was not marked");
    assert!(h.state().tab().tool.is_none(), "it should not also be left in hand");
}

/// Zoom a few steps at a screen position, the way a pinch arrives.
fn pinch_at(h: &mut Harness<'static, PagifyApp>, at: egui::Pos2, factor: f32, steps: usize) {
    for _ in 0..steps {
        h.input_mut().events.push(egui::Event::PointerMoved(at));
        h.input_mut().events.push(egui::Event::Zoom(factor));
        h.run_steps(1);
    }
    h.run_steps(2);
}

/// **The property zooming is judged by.** Whatever is under the cursor when
/// you start must still be under it when you stop — that is the point on
/// the page you were asking about.
#[test]
fn zooming_holds_the_point_under_the_cursor() {
    let mut h = harness("text-lines.pdf");
    // Started zoomed in, because anchoring is only possible on an axis that
    // can actually scroll. While the whole page fits in the window there is
    // no offset to move, so it can only grow — correctly, and with nothing
    // to hold still.
    h.state_mut().tab_mut().view_state.zoom = ZoomMode::Factor(3.0);
    h.run_steps(3);

    let at = egui::pos2(700.0, 600.0);
    let before = h.state().tab().view_state.last_view.expect("page never drawn").to_page(at);

    pinch_at(&mut h, at, 1.06, 8);

    let app = h.state();
    assert!(
        app.resolved_zoom() > 1.2,
        "the pinch did not zoom (scale {:.2})",
        app.resolved_zoom()
    );

    let after = app.tab().view_state.last_view.expect("page never drawn").to_page(at);
    let drift = ((after.x - before.x).powi(2) + (after.y - before.y).powi(2)).sqrt();
    assert!(
        drift < 6.0,
        "the page slid {drift:.1}pt out from under the cursor while zooming \
         ({before:?} -> {after:?})"
    );
}

/// And zooming out must hold it too, or the anchor is only half right.
#[test]
fn zooming_out_holds_the_point_too() {
    let mut h = harness("text-lines.pdf");
    // High enough that the page still overflows the window at the end of
    // the gesture, so there is an offset to hold all the way through.
    // Far enough in, and parked well away from both ends, that the offset
    // the anchor asks for is actually reachable throughout.
    //
    // Anchoring is only possible while there is scroll range to spend. Near
    // either end of the document — or at a zoom where the page barely
    // overflows the window — holding a point would need an offset outside
    // the scrollable range, and it gets clamped. That is the document
    // running out, not the anchor being wrong, and a test that ignores the
    // difference measures the clamp instead of the thing it is named after.
    h.state_mut().tab_mut().view_state.zoom = ZoomMode::Factor(10.0);
    h.run_steps(3);
    h.state_mut().tab_mut().view_state.anchor_offset = Some(egui::vec2(1200.0, 2500.0));
    h.run_steps(3);

    let at = egui::pos2(700.0, 600.0);
    let before = h.state().tab().view_state.last_view.expect("page never drawn").to_page(at);

    pinch_at(&mut h, at, 0.985, 6);

    let app = h.state();
    assert!(
        app.resolved_zoom() < 9.6,
        "the pinch did not zoom out (scale {:.2})",
        app.resolved_zoom()
    );

    let after = app.tab().view_state.last_view.expect("page never drawn").to_page(at);
    let drift = ((after.x - before.x).powi(2) + (after.y - before.y).powi(2)).sqrt();
    assert!(
        drift < 6.0,
        "the page slid {drift:.1}pt while zooming out (dx {:.1}, dy {:.1}) \
         zoom {:.2} offset {:?}",
        after.x - before.x,
        after.y - before.y,
        app.resolved_zoom(),
        app.tab().view_state.scroll_offset
    );
}

/// **Reported from use**: the same file, the same zoom, looked smaller in
/// Pagify than in another reader. "Actual size" used to mean one PDF
/// point (1/72 inch) drawn as one on-screen unit — literally correct,
/// and 25% smaller than what "100%" means in a browser, Acrobat, or
/// anything else that follows the 96-DPI convention Windows itself has
/// used since its own default scaling was defined. `last_view.scale` is
/// what every click, drag and render actually uses to go from a page
/// point to a screen one, so it is the one place that has to carry this,
/// not just the number painted in a "100%" label.
#[test]
fn actual_size_matches_the_96_dpi_convention_other_readers_use() {
    let mut h = harness("text-lines.pdf");
    h.state_mut().tab_mut().view_state.zoom = ZoomMode::Factor(1.0);
    h.run_steps(3);
    let scale = h.state().tab().view_state.last_view.expect("page never drawn").scale;
    assert!(
        (scale - 96.0 / 72.0).abs() < 0.01,
        "1.0 (\"100%\") should put 96/72 screen units on one PDF point, got {scale}"
    );
}

/// The middle button pans whatever tool is held — the CAD convention, and a
/// button nothing else here uses.
#[test]
fn the_middle_button_pans() {
    let mut h = harness("pages-ladder.pdf");
    // Zoomed in, so there is somewhere to pan *to*. At Fit the whole strip
    // is inside the window and the offset cannot move — which is correct,
    // and would have made this test pass for the wrong reason.
    h.state_mut().tab_mut().view_state.zoom = ZoomMode::Factor(4.0);
    h.run_steps(3);
    let before = h.state().tab().view_state.scroll_offset;

    // Comfortably inside the page's own on-screen rect, not just the
    // viewport's — `DISPLAY_DPI_SCALE` (§ its own doc) widened a page at
    // a given zoom factor by a third, which moved a page's own left
    // edge far enough right that a coordinate calibrated against the
    // old geometry started landing in the gutter beside it instead.
    let from = egui::pos2(1000.0, 500.0);
    drag_with(&mut h, egui::PointerButton::Middle, from, from - egui::vec2(0.0, 200.0));

    let after = h.state().tab().view_state.scroll_offset;
    assert!(
        (after.y - before.y).abs() > 20.0,
        "the middle button did not move the view ({before:?} -> {after:?})"
    );
}

/// It must not disturb a tool that is part-way through, which is most of
/// why it is worth having: you can move the paper without putting the tool
/// down.
#[test]
fn panning_with_the_middle_button_leaves_an_armed_tool_alone() {
    let mut h = harness("pages-ladder.pdf");
    h.state_mut().tab_mut().view_state.zoom = ZoomMode::Factor(4.0);
    h.state_mut().submit("line");
    h.run_steps(3);

    let from = egui::pos2(1000.0, 500.0);
    drag_with(&mut h, egui::PointerButton::Middle, from, from - egui::vec2(0.0, 150.0));

    let app = h.state();
    assert!(app.tab().tool.is_some(), "panning disarmed the line tool");
    assert!(
        app.tab().tool.as_ref().unwrap().points.is_empty(),
        "panning fed a point into the tool"
    );
}

/// Hand mode wrote to a field nothing reads, so dragging with it moved
/// nothing.
#[test]
fn hand_mode_actually_moves_the_page() {
    let mut h = harness("pages-ladder.pdf");
    h.state_mut().tab_mut().view_state.zoom = ZoomMode::Factor(4.0);
    h.state_mut().submit("hand");
    h.run_steps(3);
    let before = h.state().tab().view_state.scroll_offset;

    let from = egui::pos2(1000.0, 500.0);
    drag(&mut h, from, from - egui::vec2(0.0, 200.0));

    let after = h.state().tab().view_state.scroll_offset;
    assert!(
        (after.y - before.y).abs() > 20.0,
        "Hand did not move the page ({before:?} -> {after:?})"
    );
}

#[test]
fn dragging_over_text_selects_it() {
    let mut h = harness("text-lines.pdf");
    let start = a_character_on_screen(&mut h);
    let end = start + egui::vec2(140.0, 0.0);

    drag(&mut h, start, end);

    let app = h.state();
    assert!(
        app.tab().selection.text_selection.is_some(),
        "a drag across text selected nothing.\npointer mode: {:?}\nfrom {start:?} to {end:?}\n{}",
        app.tab().pointer,
        app.cmd.history().iter().map(|e| e.text.as_str()).collect::<Vec<_>>().join("\n")
    );
}

/// The first of the two states that stop selection, and the one that is
/// easiest to end up in by accident: Hand is the leftmost button on every
/// tab.
#[test]
fn hand_mode_stops_selection_and_select_gives_it_back() {
    let mut h = harness("text-lines.pdf");
    let start = a_character_on_screen(&mut h);
    let end = start + egui::vec2(140.0, 0.0);

    h.state_mut().submit("hand");
    h.run_steps(2);
    drag(&mut h, start, end);
    assert!(
        h.state().tab().selection.text_selection.is_none(),
        "Hand mode selected text, so a drag both scrolls and selects"
    );

    h.state_mut().submit("selecttool");
    h.run_steps(2);
    drag(&mut h, start, end);
    assert!(
        h.state().tab().selection.text_selection.is_some(),
        "Select did not give selection back, which would make Hand a one-way trip"
    );
}

/// The second state: a tool waiting for clicks takes them. That is correct
/// — but it means "selection stopped working" and "a tool is armed" look
/// identical unless something says so, which is why the armed tool lights
/// up in the ribbon and Escape gets out.
#[test]
fn an_armed_tool_takes_the_click_and_escape_gives_it_back() {
    let mut h = harness("text-lines.pdf");
    let start = a_character_on_screen(&mut h);
    let end = start + egui::vec2(140.0, 0.0);

    h.state_mut().submit("line");
    h.run_steps(2);
    assert!(h.state().tab().tool.is_some());

    drag(&mut h, start, end);
    assert!(
        h.state().tab().selection.text_selection.is_none(),
        "an armed tool let the drag select as well, so a click would do two things"
    );

    h.state_mut().escape();
    h.run_steps(2);
    assert!(h.state().tab().tool.is_none(), "escape did not disarm the tool");

    drag(&mut h, start, end);
    assert!(
        h.state().tab().selection.text_selection.is_some(),
        "selection did not come back after escape"
    );
}

/// Mice move. egui calls a press-move-release a drag rather than a click
/// even when the movement is a pixel, and a tool that throws those away
/// registers most clicks and loses some, which is indistinguishable from
/// being broken.
#[test]
fn a_click_that_wandered_a_pixel_still_counts() {
    let mut h = harness("text-lines.pdf");
    h.state_mut().submit("line");
    h.run_steps(2);

    let base = a_character_on_screen(&mut h) + egui::vec2(0.0, 120.0);
    drag(&mut h, base, base + egui::vec2(1.5, 1.0));
    drag(&mut h, base + egui::vec2(150.0, 60.0), base + egui::vec2(151.0, 61.0));
    h.run_steps(2);

    let app = h.state();
    let marks = app.tab().markup.existing(app.tab().view_state.page).map(|l| l.len()).unwrap_or(0);
    assert_eq!(
        marks, 1,
        "two clicks that each moved a pixel drew nothing.\n{}",
        app.cmd.history().iter().map(|e| e.text.as_str()).collect::<Vec<_>>().join("\n")
    );
}

#[test]
fn the_line_tool_draws_where_it_is_clicked() {
    let mut h = harness("text-lines.pdf");
    h.state_mut().submit("line");
    h.run_steps(2);

    let first = a_character_on_screen(&mut h);
    click(&mut h, first + egui::vec2(0.0, 120.0));
    click(&mut h, first + egui::vec2(180.0, 200.0));
    h.run_steps(2);

    let app = h.state();
    let marks = app.tab().markup.existing(app.tab().view_state.page).map(|l| l.len()).unwrap_or(0);
    assert_eq!(
        marks, 1,
        "two clicks on the page drew nothing.\n{}",
        app.cmd.history().iter().map(|e| e.text.as_str()).collect::<Vec<_>>().join("\n")
    );
}

/// **Reported from use: after `addimage`, Edit Object could neither select nor
/// move the picture.** A placed picture is an annotation, not page content, so
/// the object tool's own hit-testing — which walks the page's content objects —
/// never saw it, and the annotation interaction that does was only reachable
/// with *no* tool in hand. The log read `addimage`, "picture placed",
/// `editobject`, then a marquee ("7 things selected") that could not include
/// it.
#[test]
fn a_placed_picture_is_selected_and_moved_by_the_object_tool() {
    let mut h = harness("two-column.pdf");
    let rgba: Vec<u8> = [200u8, 60, 60, 255].repeat(16);
    h.state_mut().place_image_at(0, AppPoint { x: 150.0, y: 300.0 }, rgba, 4, 4).expect("placed");
    h.state_mut().submit("editobject");
    h.run_steps(3);
    assert!(h.state_mut().tab_mut().tool_state.object_tool.is_some(), "setup: the object tool should be in hand");

    let marks = |h: &mut Harness<'static, PagifyApp>| {
        h.state_mut().tab_mut().doc.as_ref().expect("open").session.placed_image_marks(0).expect("marks")
    };
    let before = marks(&mut h).remove(0);
    let view = h.state_mut().tab_mut().view_state.last_view.expect("the page was never drawn");
    let centre = view.to_screen(AppPoint {
        x: ((before.rect.left + before.rect.right) / 2.0) as f64,
        y: ((before.rect.top + before.rect.bottom) / 2.0) as f64,
    });

    click(&mut h, centre);
    assert!(
        h.state_mut().tab_mut().selection.placed_image_selected.is_some(),
        "clicking the picture with the object tool in hand did not select it"
    );

    drag(&mut h, centre, centre + egui::vec2(80.0, 0.0));
    let after = marks(&mut h).remove(0);
    assert!(
        after.rect.left > before.rect.left + 20.0,
        "the picture did not move: left was {} and is {}",
        before.rect.left,
        after.rect.left
    );
}

/// [`click`] with a modifier held down (Ctrl; Cmd on a Mac).
fn click_holding(h: &mut Harness<'static, PagifyApp>, at: egui::Pos2, modifiers: egui::Modifiers) {
    use egui::{Event, PointerButton};
    h.input_mut().events.push(Event::ModifiersChanged(modifiers));
    h.input_mut().events.push(Event::PointerMoved(at));
    h.run_steps(1);
    for pressed in [true, false] {
        h.input_mut().events.push(Event::PointerButton {
            pos: at,
            button: PointerButton::Primary,
            pressed,
            modifiers,
        });
        h.run_steps(1);
    }
    h.run_steps(1);
    h.input_mut().events.push(Event::ModifiersChanged(Default::default()));
    h.run_steps(1);
}

fn screen_centre(h: &mut Harness<'static, PagifyApp>, r: pdf_core::document::Rect) -> egui::Pos2 {
    let view = h.state_mut().tab_mut().view_state.last_view.expect("the page was never drawn");
    view.to_screen(AppPoint {
        x: ((r.left + r.right) / 2.0) as f64,
        y: ((r.top + r.bottom) / 2.0) as f64,
    })
}

/// **Hold Ctrl and click to pick several things.** Reported from use: only
/// Shift extended a selection, so a user reaching for the usual Ctrl-click got
/// a lone thing selected each time.
#[test]
fn ctrl_clicking_adds_to_and_removes_from_the_selection_through_the_real_pointer() {
    let mut h = harness("pictures.pdf");
    h.state_mut().submit("editobject");
    h.run_steps(3);
    let pictures = h.state_mut().tab_mut().doc.as_ref().expect("open").session.images_on(0).expect("images");
    let (first, second) = (pictures[0].clone(), pictures[1].clone());
    let (a, b) = (screen_centre(&mut h, first.rect), screen_centre(&mut h, second.rect));

    click(&mut h, a);
    assert_eq!(h.state_mut().tab_mut().selection.selected.as_ref().map(|s| s.object), Some(first.object), "setup");

    click_holding(&mut h, b, egui::Modifiers::COMMAND);
    let objects: Vec<usize> = h.state_mut().tab_mut().selection.group.iter().map(|s| s.object).collect();
    assert!(
        objects.contains(&first.object) && objects.contains(&second.object),
        "Ctrl-click did not add the second picture: group {objects:?}"
    );

    click_holding(&mut h, a, egui::Modifiers::COMMAND);
    assert_eq!(
        h.state_mut().tab_mut().selection.selected.as_ref().map(|s| s.object),
        Some(second.object),
        "Ctrl-clicking a selected picture did not take it out"
    );
}

/// **A group that was moved comes back in one `undo`.** Reported from use:
/// moving a paragraph then pressing undo walked it back a piece at a time,
/// and two such moves used up the whole undo history.
#[test]
fn moving_a_group_is_one_undo_step() {
    let mut h = harness("two-column.pdf");
    h.state_mut().submit("editobject");
    h.run_steps(3);
    let runs = h.state_mut().tab_mut().doc.as_ref().expect("open").session.text_runs(0).expect("runs");
    let mine: Vec<_> = runs.iter().take(4).cloned().collect();
    let area = pdf_core::document::Rect {
        left: mine.iter().map(|r| r.rect.left).fold(f32::MAX, f32::min) - 1.0,
        top: mine.iter().map(|r| r.rect.top).fold(f32::MAX, f32::min) - 1.0,
        right: mine.iter().map(|r| r.rect.right).fold(f32::MIN, f32::max) + 1.0,
        bottom: mine.iter().map(|r| r.rect.bottom).fold(f32::MIN, f32::max) + 1.0,
    };
    h.state_mut().select_group_in(
        0,
        AppPoint { x: area.left as f64, y: area.top as f64 },
        AppPoint { x: area.right as f64, y: area.bottom as f64 },
        false,
    );
    let members = h.state_mut().tab_mut().selection.group.len();
    assert!(members >= 2, "setup: the marquee should have selected several runs, got {members}");

    let where_are = |h: &mut Harness<'static, PagifyApp>| -> Vec<(usize, f32, f32)> {
        let runs = h.state_mut().tab_mut().doc.as_ref().expect("open").session.text_runs(0).expect("runs");
        runs.iter().map(|r| (r.object, r.rect.left, r.rect.top)).collect()
    };
    let before = where_are(&mut h);

    let bounds = h.state_mut().group_bounds(0).expect("a group has bounds");
    let from = screen_centre(&mut h, bounds);
    drag(&mut h, from, from + egui::vec2(60.0, 30.0));
    let moved = where_are(&mut h);
    assert_ne!(before, moved, "setup: dragging the group should have moved it");

    h.state_mut().submit("undo");
    h.run_steps(2);
    let after = where_are(&mut h);
    assert_eq!(after.len(), before.len());
    for (was, now) in before.iter().zip(&after) {
        assert!(
            (was.1 - now.1).abs() < 0.5 && (was.2 - now.2).abs() < 0.5,
            "one undo left run {} at ({}, {}) instead of ({}, {})",
            was.0, now.1, now.2, was.1, was.2
        );
    }
}

/// **A group move that cannot be completed moves nothing.**
#[test]
fn a_group_move_that_one_member_refuses_moves_none_of_them() {
    let mut h = harness("two-column.pdf");
    h.state_mut().submit("editobject");
    h.run_steps(3);
    let runs = h.state_mut().tab_mut().doc.as_ref().expect("open").session.text_runs(0).expect("runs");
    let (good, other) = (runs[0].clone(), runs[1].clone());
    h.state_mut().tab_mut().selection.group = vec![
        Selected { page: 0, object: good.object, rect: good.rect, what: "the text" },
        Selected { page: 0, object: other.object, rect: other.rect, what: "the text" },
        Selected { page: 0, object: 999_999, rect: other.rect, what: "the text" },
    ];
    h.state_mut().finish_group_grab(Grab { handle: None, from: AppPoint { x: 0.0, y: 0.0 }, by: (40.0, 0.0) }, 1.0);

    let now = h.state_mut().tab_mut().doc.as_ref().expect("open").session.text_runs(0).expect("runs");
    let find = |object: usize| now.iter().find(|r| r.object == object).expect("run").rect;
    assert!((find(good.object).left - good.rect.left).abs() < 0.5, "a run moved although the move failed");
    assert!((find(other.object).left - other.rect.left).abs() < 0.5, "a run moved although the move failed");
}

/// **The reported paragraph, end to end.** On the user's datasheet a paragraph
/// is 29 pieces; moving it, then pressing undo once, must put every piece back
/// — and moving it a second time afterwards must still work (it used to be
/// refused once earlier moves had left their markers in the page).
#[test]
fn a_whole_paragraph_moves_undoes_and_moves_again_on_the_real_datasheet() {
    let Some(mut h) = harness_at(r"D:\Dropbox\Datasheets\Data sheet 2024\RING 600.pdf") else { return };
    h.state_mut().submit("editobject");
    h.run_steps(3);
    h.state_mut().select_group_in(
        0,
        AppPoint { x: 24.0, y: 384.0 },
        AppPoint { x: 146.0, y: 409.0 },
        false,
    );
    let members = h.state_mut().tab_mut().selection.group.len();
    assert!(members >= 20, "setup: expected the paragraph's pieces to be selected, got {members}");

    let where_are = |h: &mut Harness<'static, PagifyApp>| -> Vec<(usize, f32, f32)> {
        let runs = h.state_mut().tab_mut().doc.as_ref().expect("open").session.text_runs(0).expect("runs");
        runs.iter().map(|r| (r.object, r.rect.left, r.rect.top)).collect()
    };
    let before = where_are(&mut h);
    let members_before: Vec<usize> = h.state_mut().tab_mut().selection.group.iter().map(|m| m.object).collect();
    let grab = |dx, dy| Grab { handle: None, from: AppPoint { x: 0.0, y: 0.0 }, by: (dx, dy) };

    let started = std::time::Instant::now();
    h.state_mut().finish_group_grab(grab(30.0, 12.0), 1.0);
    eprintln!("moving {members} pieces took {:?}", started.elapsed());
    let moved = where_are(&mut h);
    assert_ne!(before, moved, "the paragraph did not move");
    let group_objects: Vec<usize> = members_before.clone();
    for (was, now) in before.iter().zip(&moved) {
        let member = group_objects.contains(&was.0);
        let (dx, dy) = (now.1 - was.1, now.2 - was.2);
        let wanted = if member { (30.0, 12.0) } else { (0.0, 0.0) };
        if (dx - wanted.0).abs() > 0.5 || (dy - wanted.1).abs() > 0.5 {
            eprintln!("FORWARD {} run {} moved by ({dx:.2}, {dy:.2})", if member { "member" } else { "NON-member" }, was.0);
        }
    }

    h.state_mut().submit("undo");
    let back = where_are(&mut h);
    for (was, now) in before.iter().zip(&back) {
        if (now.1 - was.1).abs() > 0.5 || (now.2 - was.2).abs() > 0.5 {
            eprintln!("UNDO run {} is off by ({:.2}, {:.2})", was.0, now.1 - was.1, now.2 - was.2);
        }
    }
    for (was, now) in before.iter().zip(&back) {
        assert!(
            (was.1 - now.1).abs() < 0.5 && (was.2 - now.2).abs() < 0.5,
            "after one undo run {} is at ({}, {}) instead of ({}, {})",
            was.0, now.1, now.2, was.1, was.2
        );
    }

    // And it can be moved again, as often as the reader likes.
    for _ in 0..3 {
        h.state_mut().finish_group_grab(grab(20.0, 0.0), 1.0);
        h.state_mut().submit("undo");
    }
    let end = where_are(&mut h);
    for (was, now) in before.iter().zip(&end) {
        assert!((was.1 - now.1).abs() < 0.5 && (was.2 - now.2).abs() < 0.5, "run {} drifted", was.0);
    }
}
